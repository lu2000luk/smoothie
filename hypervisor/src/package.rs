use std::collections::HashSet;
use std::fmt;
use std::io::{self, SeekFrom};
use std::path::{Path, PathBuf};
use std::println;
use std::sync::Arc;
use std::time::SystemTime;

use futures::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::globals;

pub type PkgResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const CHUNK_SIZE: u64 = 8 * 1024 * 1024;
const PRESIGN_EXPIRY: std::time::Duration = std::time::Duration::from_secs(3600);

pub const MAX_PACKAGE_ARCHIVE_SIZE: u64 = 256 * 1024 * 1024;
pub const MAX_PACKAGE_ENTRIES: usize = 4_096;
pub const MAX_PACKAGE_EXTRACTED_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug)]
pub struct PackageValidationError(String);

impl PackageValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PackageValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackageValidationError {}

/// Validates the exact tar bytes that will be sent to the container engine.
///
/// The engine extracts uploads at `/`, so packages are restricted to canonical
/// POSIX-relative paths and regular files/directories. This duplicates upstream
/// API validation intentionally: the hypervisor is the final trust boundary
/// before extraction into a runtime container.
pub fn validate_archive(bytes: &[u8]) -> Result<(), PackageValidationError> {
    if bytes.len() as u64 > MAX_PACKAGE_ARCHIVE_SIZE {
        return Err(PackageValidationError::new(format!(
            "archive is {} bytes; maximum is {MAX_PACKAGE_ARCHIVE_SIZE} bytes",
            bytes.len()
        )));
    }

    validate_archive_with_limits(bytes, MAX_PACKAGE_ENTRIES, MAX_PACKAGE_EXTRACTED_SIZE)
}

fn validate_archive_with_limits(
    bytes: &[u8],
    max_entries: usize,
    max_extracted_size: u64,
) -> Result<(), PackageValidationError> {
    let mut archive = tar::Archive::new(io::Cursor::new(bytes));
    let entries = archive
        .entries()
        .map_err(|error| PackageValidationError::new(format!("invalid tar archive: {error}")))?;
    let mut paths = HashSet::new();
    let mut entry_count = 0usize;
    let mut extracted_size = 0u64;
    let mut has_main = false;

    for entry in entries {
        let mut entry = entry.map_err(|error| {
            PackageValidationError::new(format!("invalid tar archive entry: {error}"))
        })?;
        entry_count = entry_count
            .checked_add(1)
            .ok_or_else(|| PackageValidationError::new("archive entry count overflowed"))?;
        if entry_count > max_entries {
            return Err(PackageValidationError::new(format!(
                "archive has more than {max_entries} entries"
            )));
        }

        let raw_path = entry.path_bytes();
        let path = validate_path(raw_path.as_ref())?;
        let display_path = String::from_utf8_lossy(&path);
        if !paths.insert(path.clone()) {
            return Err(PackageValidationError::new(format!(
                "archive contains duplicate path {display_path:?}"
            )));
        }

        if let Some(extensions) = entry.pax_extensions().map_err(|error| {
            PackageValidationError::new(format!(
                "could not read archive metadata for {display_path:?}: {error}"
            ))
        })? {
            for extension in extensions {
                let extension = extension.map_err(|error| {
                    PackageValidationError::new(format!(
                        "invalid archive metadata for {display_path:?}: {error}"
                    ))
                })?;
                if extension.key_bytes().starts_with(b"GNU.sparse.") {
                    return Err(PackageValidationError::new(format!(
                        "archive entry {display_path:?} uses unsupported sparse metadata"
                    )));
                }
            }
        }

        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(PackageValidationError::new(format!(
                "archive entry {display_path:?} is not a regular file or directory"
            )));
        }

        extracted_size = extracted_size
            .checked_add(entry.size())
            .ok_or_else(|| PackageValidationError::new("archive extracted size overflowed"))?;
        if extracted_size > max_extracted_size {
            return Err(PackageValidationError::new(format!(
                "archive extracts to more than {max_extracted_size} bytes"
            )));
        }

        if path == b"main" {
            if !entry_type.is_file() {
                return Err(PackageValidationError::new(
                    "root main entry is not a regular file",
                ));
            }
            let mode = entry.header().mode().map_err(|error| {
                PackageValidationError::new(format!("root main has an invalid mode: {error}"))
            })?;
            if mode & 0o111 == 0 {
                return Err(PackageValidationError::new(
                    "root main entry is not executable",
                ));
            }
            has_main = true;
        }

        io::copy(&mut entry, &mut io::sink()).map_err(|error| {
            PackageValidationError::new(format!(
                "could not read archive entry {display_path:?}: {error}"
            ))
        })?;
    }

    if !has_main {
        return Err(PackageValidationError::new(
            "archive must contain an executable regular file named main at its root",
        ));
    }

    Ok(())
}

fn validate_path(raw_path: &[u8]) -> Result<Vec<u8>, PackageValidationError> {
    let display_path = String::from_utf8_lossy(raw_path);
    if raw_path.is_empty() {
        return Err(PackageValidationError::new(
            "archive entry has an empty path",
        ));
    }
    if raw_path.starts_with(b"/") {
        return Err(PackageValidationError::new(format!(
            "archive entry path {display_path:?} is absolute"
        )));
    }
    if raw_path.contains(&0) {
        return Err(PackageValidationError::new(format!(
            "archive entry path {display_path:?} contains a NUL byte"
        )));
    }
    if raw_path.contains(&b'\\') {
        return Err(PackageValidationError::new(format!(
            "archive entry path {display_path:?} contains a non-POSIX path separator"
        )));
    }

    let mut normalized = Vec::with_capacity(raw_path.len());
    for component in raw_path.split(|byte| *byte == b'/') {
        match component {
            b"" | b"." => continue,
            b".." => {
                return Err(PackageValidationError::new(format!(
                    "archive entry path {display_path:?} contains .."
                )));
            }
            component => {
                if !normalized.is_empty() {
                    normalized.push(b'/');
                }
                normalized.extend_from_slice(component);
            }
        }
    }

    if normalized.is_empty() {
        return Err(PackageValidationError::new(format!(
            "archive entry path {display_path:?} does not name a file or directory"
        )));
    }

    Ok(normalized)
}

fn cache_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("cannot determine executable path");
    exe.parent()
        .expect("executable has no parent directory")
        .join("hdata")
        .join("package_cache")
}

fn tar_path(id: &str) -> PathBuf {
    cache_dir().join(format!("{id}.tar"))
}

fn tmp_path(id: &str) -> PathBuf {
    cache_dir().join(format!("{id}.tar.tmp"))
}

pub fn ensure_dirs() {
    let dir = cache_dir();
    println!("Package Cache: {dir:?}");

    let mut check = dir.as_path();
    let mut to_remove = Vec::new();
    while let Some(parent) = check.parent() {
        if check.exists() && !check.is_dir() {
            to_remove.push(check.to_path_buf());
        }
        check = parent;
    }
    for p in to_remove {
        eprintln!("removing file blocking path: {p:?}");
        std::fs::remove_file(&p).expect("failed to remove file blocking directory path");
    }

    std::fs::create_dir_all(&dir).expect("failed to create package cache directory");
}

pub fn cleanup_stale_temps() {
    let dir = cache_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub async fn prepare_package(id: &str) -> PkgResult<()> {
    let path = tar_path(id);

    if path.exists() {
        touch(&path);
        return Ok(());
    }

    let lock = {
        let locks = globals::DOWNLOAD_LOCKS
            .get()
            .expect("globals not initialised");
        let mut map = locks.lock().await;
        map.entry(id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };

    let _guard = lock.lock().await;

    if path.exists() {
        touch(&path);
        return Ok(());
    }

    download(id).await?;
    enforce_quota(id).await;

    Ok(())
}

pub async fn get_package(id: &str) -> PkgResult<PathBuf> {
    prepare_package(id).await?;
    Ok(tar_path(id))
}

async fn download(id: &str) -> PkgResult<()> {
    let tmp = tmp_path(id);
    let dest = tar_path(id);

    match download_inner(id, &tmp).await {
        Ok(()) => {
            tokio::fs::rename(&tmp, &dest).await?;
            Ok(())
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}

async fn download_inner(id: &str, tmp: &Path) -> PkgResult<()> {
    let client = globals::S3_CLIENT.get().expect("S3_CLIENT not set");
    let bucket = globals::S3_BUCKET.get().expect("S3_BUCKET not set");
    let supports_range = *globals::SUPPORTS_RANGE
        .get()
        .expect("SUPPORTS_RANGE not set");

    let key = format!("{id}.tar");

    let presign_cfg = aws_sdk_s3::presigning::PresigningConfig::expires_in(PRESIGN_EXPIRY)?;
    let presigned = client
        .get_object()
        .bucket(bucket)
        .key(&key)
        .presigned(presign_cfg)
        .await?;

    let url = presigned.uri().to_string();

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    if supports_range {
        if let Some(total) = head_content_length(&http, &url).await {
            if total > CHUNK_SIZE {
                download_ranged(&http, &url, total, tmp).await?;
                return Ok(());
            }
        }
    }

    download_single(&http, &url, tmp).await
}

async fn head_content_length(http: &reqwest::Client, url: &str) -> Option<u64> {
    let resp = http.head(url).send().await.ok()?;
    resp.headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}

async fn download_single(http: &reqwest::Client, url: &str, dest: &Path) -> PkgResult<()> {
    let resp = http.get(url).send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(dest).await?;

    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

async fn download_ranged(
    http: &reqwest::Client,
    url: &str,
    total: u64,
    dest: &Path,
) -> PkgResult<()> {
    let ranges: Vec<(u64, u64)> = {
        let mut v = Vec::new();
        let mut start = 0u64;
        while start < total {
            let end = std::cmp::min(start + CHUNK_SIZE - 1, total - 1);
            v.push((start, end));
            start = end + 1;
        }
        v
    };

    let tasks: Vec<_> = ranges
        .into_iter()
        .map(|(start, end)| {
            let http = http.clone();
            let url = url.to_string();
            tokio::spawn(async move {
                let resp = http
                    .get(&url)
                    .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                    .send()
                    .await?
                    .error_for_status()?;
                let data = resp.bytes().await?;
                Ok::<_, reqwest::Error>((start, data))
            })
        })
        .collect();

    let mut chunks: Vec<(u64, bytes::Bytes)> = Vec::with_capacity(tasks.len());
    for task in tasks {
        chunks.push(task.await??);
    }
    chunks.sort_by_key(|(off, _)| *off);

    let mut file = tokio::fs::File::create(dest).await?;
    file.set_len(total).await?;
    for (offset, data) in chunks {
        file.seek(SeekFrom::Start(offset)).await?;
        file.write_all(&data).await?;
    }
    file.flush().await?;
    Ok(())
}

fn touch(path: &Path) {
    let now = filetime::FileTime::now();
    let _ = filetime::set_file_mtime(path, now);
}

async fn enforce_quota(exclude_id: &str) {
    let quota = *globals::CACHE_QUOTA.get().expect("CACHE_QUOTA not set");
    let dir = cache_dir();

    let mut entries: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    let mut total: u64 = 0;

    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("tar") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let size = meta.len();
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                total += size;
                entries.push((path, size, mtime));
            }
        }
    }

    if total <= quota {
        return;
    }

    entries.sort_by_key(|(_, _, t)| *t);

    let keep = tar_path(exclude_id);
    for (path, size, _) in &entries {
        if total <= quota {
            break;
        }
        if *path == keep {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tar::{Builder, Header};

    fn append_entry(
        builder: &mut Builder<Vec<u8>>,
        path: &[u8],
        type_flag: u8,
        mode: u32,
        contents: &[u8],
    ) {
        assert!(path.len() <= 100);
        let mut header = Header::new_gnu();
        header.set_mode(mode);
        header.set_size(contents.len() as u64);
        {
            let bytes = header.as_mut_bytes();
            bytes[..100].fill(0);
            bytes[..path.len()].copy_from_slice(path);
            bytes[156] = type_flag;
        }
        header.set_cksum();
        builder.append(&header, contents).unwrap();
    }

    fn build_archive(entries: &[(&[u8], u8, u32, &[u8])]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());
        for &(path, type_flag, mode, contents) in entries {
            append_entry(&mut builder, path, type_flag, mode, contents);
        }
        builder.into_inner().unwrap()
    }

    fn valid_main() -> (&'static [u8], u8, u32, &'static [u8]) {
        (b"main", b'0', 0o755, b"#!/bin/sh\n")
    }

    #[test]
    fn accepts_safe_archive_with_executable_root_main() {
        let archive = build_archive(&[
            valid_main(),
            (b"assets/", b'5', 0o755, b""),
            (b"assets/message.txt", b'0', 0o644, b"hello"),
        ]);

        validate_archive(&archive).unwrap();
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        for path in [b"/main".as_slice(), b"bin/../main".as_slice()] {
            let archive = build_archive(&[(path, b'0', 0o755, b"main")]);
            let error = validate_archive(&archive).unwrap_err().to_string();
            assert!(
                error.contains("absolute") || error.contains("contains .."),
                "unexpected error for {path:?}: {error}"
            );
        }
    }

    #[test]
    fn rejects_links_and_special_entries() {
        for (type_flag, description) in [
            (b'1', "hard link"),
            (b'2', "symbolic link"),
            (b'3', "character device"),
            (b'4', "block device"),
            (b'6', "fifo"),
        ] {
            let archive = build_archive(&[valid_main(), (b"unsafe", type_flag, 0o644, b"")]);
            let error = validate_archive(&archive).unwrap_err().to_string();
            assert!(
                error.contains("not a regular file or directory"),
                "{description} produced unexpected error: {error}"
            );
        }
    }

    #[test]
    fn requires_root_main_to_be_regular_and_executable() {
        let missing = build_archive(&[(b"bin/main", b'0', 0o755, b"main")]);
        assert!(
            validate_archive(&missing)
                .unwrap_err()
                .to_string()
                .contains("root")
        );

        let directory = build_archive(&[(b"main", b'5', 0o755, b"")]);
        assert!(
            validate_archive(&directory)
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );

        let not_executable = build_archive(&[(b"main", b'0', 0o644, b"main")]);
        assert!(
            validate_archive(&not_executable)
                .unwrap_err()
                .to_string()
                .contains("not executable")
        );
    }

    #[test]
    fn enforces_entry_and_extracted_size_limits() {
        let two_entries = build_archive(&[valid_main(), (b"asset", b'0', 0o644, b"x")]);
        assert!(
            validate_archive_with_limits(&two_entries, 1, u64::MAX)
                .unwrap_err()
                .to_string()
                .contains("more than 1 entries")
        );

        let oversized = build_archive(&[valid_main(), (b"asset", b'0', 0o644, b"12345")]);
        assert!(
            validate_archive_with_limits(&oversized, usize::MAX, 12)
                .unwrap_err()
                .to_string()
                .contains("extracts to more than 12 bytes")
        );
    }

    #[test]
    fn rejects_duplicate_normalized_paths() {
        let archive = build_archive(&[valid_main(), (b"./main", b'0', 0o755, b"other")]);
        assert!(
            validate_archive(&archive)
                .unwrap_err()
                .to_string()
                .contains("duplicate path")
        );
    }

    #[test]
    fn rejects_sparse_pax_metadata() {
        let mut builder = Builder::new(Vec::new());
        builder
            .append_pax_extensions([("GNU.sparse.realsize", b"536870912".as_slice())])
            .unwrap();
        append_entry(&mut builder, b"main", b'0', 0o755, b"main");
        let archive = builder.into_inner().unwrap();

        assert!(
            validate_archive(&archive)
                .unwrap_err()
                .to_string()
                .contains("sparse metadata")
        );
    }
}
