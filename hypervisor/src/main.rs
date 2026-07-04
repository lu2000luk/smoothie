use std::sync::OnceLock;

use actix_web::{App, HttpServer};
use serde::{Deserialize, Serialize};

static REDIS_CLIENT: OnceLock<redis::Client> = OnceLock::new();

#[derive(Serialize, Deserialize)]
struct S3Config {
    access_key: String,
    secret_key: String,
    bucket: String,
    region: String,
}

#[derive(Serialize, Deserialize)]
struct Config {
    redis: String,
    port: Option<u16>,
    host: Option<String>,
    s3: S3Config,
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

    println!("Redis URL: {}", config.redis);

    let redis_client = match redis::Client::open(config.redis.clone()) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to create Redis client: {}", e);
            std::process::exit(1);
        }
    };
    REDIS_CLIENT.set(redis_client).expect("Failed to set Redis client");

    let port = config.port.unwrap_or(8080);
    println!("Starting server: http://localhost:{}", port);

    HttpServer::new(|| App::new())
        .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
        .run()
        .await
}
