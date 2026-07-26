use std::collections::HashMap;
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
pub static IDLE_CONTAINERS: OnceLock<IdleContainerPool> = OnceLock::new();
