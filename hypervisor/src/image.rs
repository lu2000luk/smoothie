use std::path::PathBuf;

use flate2::read::GzDecoder;
use tar::Archive as TarArchive;

const ALPINE_VERSION: &str = "3.24.1";

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn data_root() -> PathBuf {
    std::env::current_exe()
        .expect("cannot determine executable path")
        .parent()
        .expect("executable has no parent directory")
        .join("hdata")
}

pub fn images_dir() -> PathBuf {
    data_root().join("images")
}

pub fn alpine_archive_path() -> PathBuf {
    images_dir().join(format!(
        "alpine-minirootfs-{ALPINE_VERSION}-{}.tar.gz",
        alpine_arch()
    ))
}

pub fn base_rootfs_path() -> PathBuf {
    data_root().join("base_rootfs")
}

pub async fn ensure_alpine() -> Result<PathBuf> {
    let destination = alpine_archive_path();
    if destination.is_file() {
        return Ok(destination);
    }
    tokio::fs::create_dir_all(images_dir()).await?;
    let arch = alpine_arch();
    let url = format!(
        "https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/{arch}/alpine-minirootfs-{ALPINE_VERSION}-{arch}.tar.gz"
    );
    let temporary = destination.with_extension("tar.gz.part");
    let result = async {
        let response = reqwest::get(url).await?.error_for_status()?;
        let bytes = response.bytes().await?;
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, &destination).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result?;
    Ok(destination)
}

pub fn alpine_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => panic!("unsupported Alpine architecture: {other}"),
    }
}

pub fn ensure_base_rootfs(archive: &PathBuf) -> Result<PathBuf> {
    let dest = base_rootfs_path();
    if dest.join("etc").exists() {
        return Ok(dest);
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::create_dir_all(&dest)?;
    eprintln!(
        "[image] extracting alpine base rootfs to {}...",
        dest.display()
    );
    let file = std::fs::File::open(archive)?;
    let gz = GzDecoder::new(file);
    let mut ar = TarArchive::new(gz);
    ar.unpack(&dest)?;
    eprintln!("[image] base rootfs ready at {}", dest.display());
    Ok(dest)
}
