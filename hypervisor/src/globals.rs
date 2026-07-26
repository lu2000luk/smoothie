use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex;

use crate::container::{CrunRuntime, IdleContainerPool};

pub static REDIS_CLIENT: OnceLock<redis::Client> = OnceLock::new();
pub static S3_CLIENT: OnceLock<aws_sdk_s3::Client> = OnceLock::new();
pub static S3_BUCKET: OnceLock<String> = OnceLock::new();
pub static CACHE_QUOTA: OnceLock<u64> = OnceLock::new();
pub static SUPPORTS_RANGE: OnceLock<bool> = OnceLock::new();
pub static DOWNLOAD_LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();
pub static RUNTIME: OnceLock<CrunRuntime> = OnceLock::new();
pub static IDLE_CONTAINERS: OnceLock<Arc<IdleContainerPool>> = OnceLock::new();
pub static SOCKETS_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn console_socket_path(container_id: &str) -> PathBuf {
    SOCKETS_DIR
        .get()
        .expect("SOCKETS_DIR not set")
        .join(format!("{container_id}.sock"))
}
