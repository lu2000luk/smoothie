use std::io::Read;
use std::path::{Component, Path};

use redis::AsyncCommands;
use rocket::form::{Form, FromForm};
use rocket::fs::TempFile;
use rocket::http::Status;
use rocket::serde::{Deserialize, Serialize};
use rocket::{State, delete, get, post};
use sha2::{Digest, Sha256};

use crate::auth::{SessionToken, require_user};
use crate::services::{ApiError, ApiJson, Service, api_json};
use crate::state::{ApiState, conn, json_error};

pub const SERVICE_BLOB_CAP_BYTES: u64 = 209_715_200;
pub const MAX_TAR_ENTRIES: usize = 10_000;
pub const MAX_TAR_CONTENT_BYTES: u64 = SERVICE_BLOB_CAP_BYTES;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(crate = "rocket::serde", rename_all = "snake_case")]
pub enum BlobStatus {
    Available,
    Pruned,
    Deleted,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "rocket::serde")]
pub struct Package {
    pub id: String,
    pub service_id: String,
    pub app_id: String,
    pub owner: u64,
    #[serde(default)]
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub blob_status: BlobStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruned_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub sha256: String,
    pub size_bytes: u64,
    pub entries: usize,
}

#[derive(FromForm)]
pub struct PackageUpload<'r> {
    pub package: TempFile<'r>,
}

pub fn key_service_packages(owner: u64, app_id: &str, service_id: &str) -> String {
    format!("user:{owner}:app:{app_id}:service:{service_id}:packages")
}

pub fn blob_key(package_id: &str) -> String {
    format!("{package_id}.tar")
}

pub fn safe_package_filename(raw: Option<&str>) -> String {
    let basename = raw
        .unwrap_or("package.tar")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("package.tar")
        .trim();
    let mut safe: String = basename
        .chars()
        .filter(|character| !character.is_control())
        .take(255)
        .collect();
    if safe.is_empty() || safe == "." || safe == ".." {
        safe = "package.tar".into();
    }
    safe
}

fn validate_archive_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("tar entries must use non-empty relative paths".into());
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe tar path: {}", path.display()));
    }
    Ok(())
}

pub fn validate_archive(path: &Path) -> Result<ArchiveInfo, String> {
    let size_bytes = std::fs::metadata(path)
        .map_err(|e| format!("read archive metadata failed: {e}"))?
        .len();
    if size_bytes == 0 {
        return Err("package archive is empty".into());
    }
    if size_bytes > SERVICE_BLOB_CAP_BYTES {
        return Err(format!(
            "package exceeds the {SERVICE_BLOB_CAP_BYTES} byte service blob cap"
        ));
    }

    let mut hash_file =
        std::fs::File::open(path).map_err(|e| format!("open archive for hashing failed: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = hash_file
            .read(&mut buffer)
            .map_err(|e| format!("hash archive failed: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let sha256 = format!("{:x}", hasher.finalize());

    let file = std::fs::File::open(path)
        .map_err(|e| format!("open archive for validation failed: {e}"))?;
    let mut archive = tar::Archive::new(file);
    let entries = archive
        .entries()
        .map_err(|e| format!("invalid tar archive: {e}"))?;
    let mut entry_count = 0_usize;
    let mut content_bytes = 0_u64;
    let mut has_root_main = false;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("invalid tar entry: {e}"))?;
        entry_count += 1;
        if entry_count > MAX_TAR_ENTRIES {
            return Err(format!("tar contains more than {MAX_TAR_ENTRIES} entries"));
        }

        let entry_path = entry
            .path()
            .map_err(|e| format!("invalid tar path: {e}"))?
            .into_owned();
        validate_archive_path(&entry_path)?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(format!(
                "tar links, devices, and special entries are not allowed: {}",
                entry_path.display()
            ));
        }

        let entry_size = entry
            .header()
            .size()
            .map_err(|e| format!("invalid tar entry size: {e}"))?;
        content_bytes = content_bytes
            .checked_add(entry_size)
            .ok_or_else(|| "tar content size overflow".to_string())?;
        if content_bytes > MAX_TAR_CONTENT_BYTES {
            return Err(format!(
                "tar content exceeds the {MAX_TAR_CONTENT_BYTES} byte limit"
            ));
        }

        if entry_path == Path::new("main") {
            if !entry_type.is_file() {
                return Err("root main must be a regular file".into());
            }
            let mode = entry
                .header()
                .mode()
                .map_err(|e| format!("invalid main mode: {e}"))?;
            if mode & 0o111 == 0 {
                return Err("root main must be executable".into());
            }
            has_root_main = true;
        }

        std::io::copy(&mut entry, &mut std::io::sink())
            .map_err(|e| format!("truncated or unreadable tar entry: {e}"))?;
    }

    if !has_root_main {
        return Err("tar must contain an executable file named main at its root".into());
    }

    Ok(ArchiveInfo {
        sha256,
        size_bytes,
        entries: entry_count,
    })
}

async fn put_blob(state: &ApiState, package_id: &str, path: &Path) -> Result<(), ApiError> {
    let body = aws_sdk_s3::primitives::ByteStream::from_path(path)
        .await
        .map_err(|e| {
            json_error(
                Status::InternalServerError,
                format!("read package failed: {e}"),
            )
        })?;
    state
        .s3
        .put_object()
        .bucket(&state.config.s3.bucket)
        .key(blob_key(package_id))
        .body(body)
        .content_type("application/x-tar")
        .send()
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("S3 upload failed: {e}")))?;
    Ok(())
}

async fn delete_blob(state: &ApiState, package_id: &str) -> Result<(), ApiError> {
    state
        .s3
        .delete_object()
        .bucket(&state.config.s3.bucket)
        .key(blob_key(package_id))
        .send()
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("S3 delete failed: {e}")))?;
    Ok(())
}

async fn save_package(state: &ApiState, package: &Package) -> Result<(), ApiError> {
    let raw = serde_json::to_string(package).map_err(|e| {
        json_error(
            Status::InternalServerError,
            format!("encode package failed: {e}"),
        )
    })?;
    let mut redis = conn(state).await?;
    redis
        .hset::<_, _, _, ()>(
            key_service_packages(package.owner, &package.app_id, &package.service_id),
            &package.id,
            raw,
        )
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })
}

pub(crate) async fn load_package(
    state: &ApiState,
    service: &Service,
    package_id: &str,
) -> Result<Package, ApiError> {
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .hget(
            key_service_packages(service.owner, &service.app_id, &service.id),
            package_id,
        )
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "package not found"))?;
    let package: Package = serde_json::from_str(&raw)
        .map_err(|_| json_error(Status::InternalServerError, "stored package is corrupt"))?;
    if package.owner != service.owner
        || package.app_id != service.app_id
        || package.service_id != service.id
    {
        return Err(json_error(Status::NotFound, "package not found"));
    }
    Ok(package)
}

async fn load_packages(state: &ApiState, service: &Service) -> Result<Vec<Package>, ApiError> {
    let mut redis = conn(state).await?;
    let map: std::collections::HashMap<String, String> = redis
        .hgetall(key_service_packages(
            service.owner,
            &service.app_id,
            &service.id,
        ))
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let mut by_id = std::collections::HashMap::with_capacity(map.len());
    for raw in map.into_values() {
        let package: Package = serde_json::from_str(&raw)
            .map_err(|_| json_error(Status::InternalServerError, "stored package is corrupt"))?;
        if package.owner == service.owner
            && package.app_id == service.app_id
            && package.service_id == service.id
        {
            by_id.insert(package.id.clone(), package);
        }
    }
    let mut packages = Vec::with_capacity(by_id.len());
    for id in &service.package_ids {
        if let Some(package) = by_id.remove(id) {
            packages.push(package);
        }
    }
    let mut unindexed: Vec<_> = by_id.into_values().collect();
    unindexed.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    packages.extend(unindexed);
    Ok(packages)
}

pub fn packages_newest_first(mut packages: Vec<Package>) -> Vec<Package> {
    packages.reverse();
    packages
}

pub fn packages_to_prune(packages: &[Package], active_package_id: &str) -> Vec<String> {
    let mut available_bytes: u64 = packages
        .iter()
        .filter(|package| package.blob_status == BlobStatus::Available)
        .map(|package| package.size_bytes)
        .sum();
    let mut result = Vec::new();
    for package in packages {
        if available_bytes <= SERVICE_BLOB_CAP_BYTES {
            break;
        }
        if package.blob_status == BlobStatus::Available && package.id != active_package_id {
            available_bytes = available_bytes.saturating_sub(package.size_bytes);
            result.push(package.id.clone());
        }
    }
    result
}

async fn enforce_blob_cap(state: &ApiState, service: &Service) -> Result<Vec<String>, ApiError> {
    let Some(active_package_id) = service.active_package_id.as_deref() else {
        return Ok(Vec::new());
    };
    let mut packages = load_packages(state, service).await?;
    let prune_ids = packages_to_prune(&packages, active_package_id);
    let now = chrono::Utc::now().to_rfc3339();
    for package_id in &prune_ids {
        let package = packages
            .iter_mut()
            .find(|package| &package.id == package_id)
            .expect("prune IDs come from loaded packages");
        delete_blob(state, &package.id).await?;
        package.blob_status = BlobStatus::Pruned;
        package.pruned_at = Some(now.clone());
        save_package(state, package).await?;
    }
    Ok(prune_ids)
}

pub(crate) async fn delete_available_blobs(
    state: &ApiState,
    service: &Service,
) -> Result<(), ApiError> {
    for package in load_packages(state, service).await? {
        if package.blob_status == BlobStatus::Available {
            delete_blob(state, &package.id).await?;
        }
    }
    Ok(())
}

fn deployment_response(
    status: Status,
    service: Service,
    package: Package,
    deployment: crate::deployments::Deployment,
    pruned_package_ids: Vec<String>,
    router_error: Option<String>,
) -> (Status, ApiJson) {
    let mut value = serde_json::json!({
        "service": service,
        "package": package,
        "deployment": deployment,
        "pruned_package_ids": pruned_package_ids,
    });
    if let Some(error) = router_error {
        value["error"] = serde_json::Value::String(error);
    }
    (status, api_json(value))
}

#[get("/apps/<app_id>/services/<service_id>/packages")]
pub async fn list_service_packages(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let packages = packages_newest_first(load_packages(state, &service).await?);
    Ok(api_json(serde_json::json!({ "packages": packages })))
}

#[post("/apps/<app_id>/services/<service_id>/packages", data = "<upload>")]
pub async fn upload_service_package(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    upload: Form<PackageUpload<'_>>,
) -> Result<(Status, ApiJson), ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;

    let mut upload = upload.into_inner();
    let filename = safe_package_filename(upload.package.name());
    let temp_dir = tempfile::tempdir().map_err(|e| {
        json_error(
            Status::InternalServerError,
            format!("create upload directory failed: {e}"),
        )
    })?;
    let path = temp_dir.path().join("package.tar");
    upload.package.persist_to(&path).await.map_err(|e| {
        json_error(
            Status::BadRequest,
            format!("read multipart package failed: {e}"),
        )
    })?;
    let validation_path = path.clone();
    let archive = tokio::task::spawn_blocking(move || validate_archive(&validation_path))
        .await
        .map_err(|e| {
            json_error(
                Status::InternalServerError,
                format!("archive validation task failed: {e}"),
            )
        })?
        .map_err(|e| json_error(Status::BadRequest, e))?;

    let package_id = uuid::Uuid::new_v4().to_string();
    put_blob(state, &package_id, &path).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let package = Package {
        id: package_id.clone(),
        service_id: service.id.clone(),
        app_id: service.app_id.clone(),
        owner: service.owner,
        filename,
        sha256: archive.sha256,
        size_bytes: archive.size_bytes,
        blob_status: BlobStatus::Available,
        created_at: now,
        pruned_at: None,
        deleted_at: None,
    };
    if let Err(error) = save_package(state, &package).await {
        let _ = delete_blob(state, &package.id).await;
        return Err(error);
    }
    if !service.package_ids.iter().any(|id| id == &package.id) {
        service.package_ids.push(package.id.clone());
    }
    crate::services::save_service(state, &service).await?;

    let (deployment, router_error) =
        crate::deployments::activate_package(state, &mut service, &package).await?;
    let pruned_package_ids = enforce_blob_cap(state, &service).await?;
    Ok(deployment_response(
        Status::Created,
        service,
        package,
        deployment,
        pruned_package_ids,
        router_error,
    ))
}

#[delete("/apps/<app_id>/services/<service_id>/packages/<package_id>")]
pub async fn delete_service_package(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    package_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    if service.active_package_id.as_deref() == Some(package_id) {
        return Err(json_error(
            Status::Conflict,
            "the active package cannot be deleted; activate another package first",
        ));
    }
    let mut package = load_package(state, &service, package_id).await?;
    if package.blob_status == BlobStatus::Available {
        delete_blob(state, &package.id).await?;
        package.blob_status = BlobStatus::Deleted;
        package.deleted_at = Some(chrono::Utc::now().to_rfc3339());
        save_package(state, &package).await?;
    }
    Ok(api_json(
        serde_json::json!({ "ok": true, "package": package }),
    ))
}

#[post("/apps/<app_id>/services/<service_id>/packages/<package_id>/activate")]
pub async fn activate_service_package(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    package_id: &str,
) -> Result<(Status, ApiJson), ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let package = load_package(state, &service, package_id).await?;
    if package.blob_status != BlobStatus::Available {
        return Err(json_error(
            Status::Conflict,
            "package blob is not available",
        ));
    }
    let (deployment, router_error) =
        crate::deployments::activate_package(state, &mut service, &package).await?;
    let pruned_package_ids = enforce_blob_cap(state, &service).await?;
    Ok(deployment_response(
        Status::Ok,
        service,
        package,
        deployment,
        pruned_package_ids,
        router_error,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(id: &str, size_bytes: u64, status: BlobStatus) -> Package {
        Package {
            id: id.into(),
            service_id: "s".into(),
            app_id: "a".into(),
            owner: 1,
            filename: format!("{id}.tar"),
            sha256: "hash".into(),
            size_bytes,
            blob_status: status,
            created_at: id.into(),
            pruned_at: None,
            deleted_at: None,
        }
    }

    fn write_tar(entries: &[(&str, &[u8], u32)]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut builder = tar::Builder::new(file.as_file_mut());
            for (path, bytes, mode) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(*mode);
                header.set_cksum();
                builder.append_data(&mut header, path, *bytes).unwrap();
            }
            builder.finish().unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn sanitizes_original_upload_filenames() {
        assert_eq!(
            safe_package_filename(Some(r#"C:\\builds\\smoothie app.tar"#)),
            "smoothie app.tar"
        );
        assert_eq!(
            safe_package_filename(Some("../../release.tar")),
            "release.tar"
        );
        assert_eq!(safe_package_filename(Some("..")), "package.tar");
        assert_eq!(safe_package_filename(None), "package.tar");
    }

    #[test]
    fn old_package_records_default_the_filename() {
        let package: Package = serde_json::from_value(serde_json::json!({
            "id": "p",
            "service_id": "s",
            "app_id": "a",
            "owner": 1,
            "sha256": "hash",
            "size_bytes": 10,
            "blob_status": "available",
            "created_at": "2024-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(package.filename, "");
    }

    #[test]
    fn validates_executable_root_main_and_hashes_archive() {
        let file = write_tar(&[("main", b"#!/bin/sh\n", 0o755), ("assets/a", b"a", 0o644)]);
        let info = validate_archive(file.path()).unwrap();
        assert_eq!(info.entries, 2);
        assert_eq!(info.sha256.len(), 64);
        assert!(info.size_bytes > 0);
    }

    #[test]
    fn rejects_missing_or_non_executable_main() {
        let missing = write_tar(&[("bin/main", b"x", 0o755)]);
        assert!(validate_archive(missing.path()).is_err());
        let not_executable = write_tar(&[("main", b"x", 0o644)]);
        assert!(validate_archive(not_executable.path()).is_err());
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(validate_archive_path(Path::new("../main")).is_err());
        assert!(validate_archive_path(Path::new("/main")).is_err());
        assert!(validate_archive_path(Path::new("./main")).is_err());
        assert!(validate_archive_path(Path::new("main")).is_ok());
    }

    #[test]
    fn package_lists_are_newest_first() {
        let packages = vec![
            package("old", 1, BlobStatus::Available),
            package("new", 1, BlobStatus::Available),
        ];
        let ids: Vec<_> = packages_newest_first(packages)
            .into_iter()
            .map(|package| package.id)
            .collect();
        assert_eq!(ids, vec!["new", "old"]);
    }

    #[test]
    fn pruning_is_oldest_first_and_never_prunes_active() {
        let packages = vec![
            package("old", 100_000_000, BlobStatus::Available),
            package("active", 150_000_000, BlobStatus::Available),
            package("already-pruned", 100_000_000, BlobStatus::Pruned),
        ];
        assert_eq!(packages_to_prune(&packages, "active"), vec!["old"]);
    }

    #[test]
    fn cap_is_exactly_two_hundred_mebibytes() {
        assert_eq!(SERVICE_BLOB_CAP_BYTES, 209_715_200);
        let packages = vec![
            package("old", 1, BlobStatus::Available),
            package("active", SERVICE_BLOB_CAP_BYTES, BlobStatus::Available),
        ];
        assert_eq!(packages_to_prune(&packages, "active"), vec!["old"]);
    }
}
