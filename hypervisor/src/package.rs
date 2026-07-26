use std::io::SeekFrom;
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
