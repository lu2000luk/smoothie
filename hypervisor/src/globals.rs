use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex;

use crate::container::IdleContainerPool;

pub static REDIS_CLIENT: OnceLock<redis::Client> = OnceLock::new();
pub static S3_CLIENT: OnceLock<aws_sdk_s3::Client> = OnceLock::new();
pub static S3_BUCKET: OnceLock<String> = OnceLock::new();
pub static CACHE_QUOTA: OnceLock<u64> = OnceLock::new();
pub static SUPPORTS_RANGE: OnceLock<bool> = OnceLock::new();
pub static DOWNLOAD_LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();
pub static DOCKER_SOCKET: OnceLock<PathBuf> = OnceLock::new();
pub static DOCKER_RUNTIME: OnceLock<Arc<crate::docker::DockerRuntime>> = OnceLock::new();
pub static IDLE_CONTAINERS: OnceLock<Arc<IdleContainerPool>> = OnceLock::new();
pub static SOCKETS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Path of the per-container console socket. Retained for backwards
/// compatibility with any consumer of the global state. The Docker backend
/// does not use this; it streams logs through the Docker API instead.
pub fn console_socket_path(container_id: &str) -> PathBuf {
    SOCKETS_DIR
        .get()
        .expect("SOCKETS_DIR not set")
        .join(format!("{container_id}.sock"))
}
