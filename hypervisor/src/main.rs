mod container;
mod globals;
mod package;

use std::collections::HashMap;

use actix_web::{App, HttpServer, web};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize)]
struct S3Config {
    access_key: String,
    secret_key: String,
    bucket: String,
    region: String,
    endpoint: Option<String>,
    supports_range: Option<bool>,
    force_path_style: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct ResourceLimits {
    #[serde(default = "defaults::ram")]
    ram: u64,
    #[serde(default = "defaults::cpu_p")]
    cpu_p: u64,
    #[serde(default = "defaults::cpu_q")]
    cpu_q: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            ram: defaults::ram(),
            cpu_p: defaults::cpu_p(),
            cpu_q: defaults::cpu_q(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Config {
    redis: String,
    port: Option<u16>,
    host: Option<String>,
    s3: S3Config,
    #[serde(default = "defaults::idle_containers")]
    idle_containers: u32,
    #[serde(default = "defaults::max_hybernated_containers")]
    max_hybernated_containers: u32,
    #[serde(default = "defaults::snapshots_storage_quota")]
    snapshots_storage_quota: u64,
    #[serde(default = "defaults::package_cache_quota")]
    package_cache_quota: u64,
    #[serde(default)]
    resource_limits: ResourceLimits,
}

mod defaults {
    pub fn idle_containers() -> u32 {
        5
    }
    pub fn max_hybernated_containers() -> u32 {
        20
    }
    pub fn snapshots_storage_quota() -> u64 {
        209_715_200
    }
    pub fn package_cache_quota() -> u64 {
        524_288_000
    }
    pub fn ram() -> u64 {
        26_214_400
    }
    pub fn cpu_p() -> u64 {
        50_000
    }
    pub fn cpu_q() -> u64 {
        12_500
    }
}

#[actix_web::get("/package/prepare/{id}")]
async fn prepare_package_route(id: web::Path<String>) -> actix_web::HttpResponse {
    match package::prepare_package(&id).await {
        Ok(_) => actix_web::HttpResponse::Ok().json(serde_json::json!({"status": "ok"})),
        Err(e) => actix_web::HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": e.to_string()})),
    }
}

#[actix_web::get("/package/getlocal/{id}")]
async fn get_package_route(id: web::Path<String>) -> actix_web::HttpResponse {
    match package::get_package(&id).await {
        Ok(path) => {
            actix_web::HttpResponse::Ok().json(serde_json::json!({"path": path.to_string_lossy()}))
        }
        Err(e) => actix_web::HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": e.to_string()})),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config_path = "config.json";
    let config = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => {
            eprintln!("Failed to read configuration file: {}", config_path);
            std::process::exit(1);
        }
    };

    let config: Config = match serde_json::from_str(&config) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to parse configuration file: {}", e);
            std::process::exit(1);
        }
    };

    let redis_client = match redis::Client::open(config.redis.clone()) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to create Redis client: {}", e);
            std::process::exit(1);
        }
    };
    globals::REDIS_CLIENT
        .set(redis_client)
        .expect("Failed to set Redis client");

    let creds = aws_credential_types::Credentials::new(
        &config.s3.access_key,
        &config.s3.secret_key,
        None,
        None,
        "smoothie-config",
    );

    let mut s3_builder = aws_sdk_s3::Config::builder()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new(config.s3.region.clone()))
        .credentials_provider(creds)
        .force_path_style(config.s3.force_path_style.unwrap_or(true));

    if let Some(ref endpoint) = config.s3.endpoint {
        s3_builder = s3_builder.endpoint_url(endpoint);
    }

    let s3_client = aws_sdk_s3::Client::from_conf(s3_builder.build());

    globals::S3_CLIENT
        .set(s3_client)
        .expect("Failed to set S3 client");
    globals::S3_BUCKET
        .set(config.s3.bucket.clone())
        .expect("Failed to set S3 bucket");
    globals::CACHE_QUOTA
        .set(config.package_cache_quota)
        .expect("Failed to set cache quota");
    globals::SUPPORTS_RANGE
        .set(config.s3.supports_range.unwrap_or(false))
        .expect("Failed to set supports_range");
    globals::DOWNLOAD_LOCKS
        .set(Mutex::new(HashMap::new()))
        .expect("Failed to set download locks");

    package::ensure_dirs();
    package::cleanup_stale_temps();

    let port = config.port.unwrap_or(8080);
    println!("Starting server: http://localhost:{}", port);

    HttpServer::new(|| {
        App::new()
            .service(prepare_package_route)
            .service(get_package_route)
    })
    .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
    .run()
    .await
}
