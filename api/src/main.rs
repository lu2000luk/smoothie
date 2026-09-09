use std::net::IpAddr;
use std::str::FromStr;

use rocket::serde::{Deserialize, Serialize};
use rocket::{State, get, routes};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
struct S3Config {
    access_key: String,
    secret_key: String,
    bucket: String,
    region: String,
    endpoint: Option<String>,
    supports_range: Option<bool>,
    force_path_style: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
struct Config {
    redis: String,
    port: Option<u16>,
    host: Option<String>,
    s3: S3Config,
    #[serde(alias = "router", alias = "router_url")]
    router_address: String,
}

mod defaults {
    pub fn port() -> u16 {
        3400
    }
    pub fn host() -> String {
        "0.0.0.0".into()
    }
    #[allow(dead_code)]
    pub fn redis() -> String {
        "redis://localhost:6379".into()
    }
    #[allow(dead_code)]
    pub fn router_address() -> String {
        "127.0.0.1:3300".into()
    }
    #[allow(dead_code)]
    pub fn s3() -> super::S3Config {
        super::S3Config {
            access_key: "minioadmin".into(),
            secret_key: "minioadmin".into(),
            bucket: "packages".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://localhost:9000".into()),
            supports_range: Some(true),
            force_path_style: Some(true),
        }
    }
}

#[allow(dead_code)]
fn default_config() -> Config {
    Config {
        redis: defaults::redis(),
        port: Some(defaults::port()),
        host: Some(defaults::host()),
        s3: defaults::s3(),
        router_address: defaults::router_address(),
    }
}

fn load_config() -> Config {
    let config_path = "config.json";
    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => {
            eprintln!("Failed to read configuration file: {}", config_path);
            std::process::exit(1);
        }
    };
    match serde_json::from_str::<Config>(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to parse configuration file: {}", e);
            std::process::exit(1);
        }
    }
}

fn router_base_url(address: &str) -> String {
    let address = address.trim_end_matches('/');
    if address.starts_with("http://") || address.starts_with("https://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    }
}

fn parse_host(host: Option<String>) -> IpAddr {
    let raw = host.unwrap_or_else(defaults::host);
    if raw.eq_ignore_ascii_case("localhost") {
        return IpAddr::from_str("127.0.0.1").expect("loopback parses");
    }
    match IpAddr::from_str(&raw) {
        Ok(ip) => ip,
        Err(_) => {
            eprintln!("Invalid host in config.json: {raw} (want an IP like 0.0.0.0)");
            std::process::exit(1);
        }
    }
}

struct ApiState {
    config: Config,
}

#[get("/")]
fn index() -> &'static str {
    "Smoothie API"
}

#[get("/health")]
fn health(state: &State<ApiState>) -> rocket::serde::json::Json<serde_json::Value> {
    rocket::serde::json::Json(serde_json::json!({
        "status": "ok",
        "router": router_base_url(&state.config.router_address),
    }))
}

#[rocket::launch]
fn rocket() -> _ {
    println!("Loading config...");
    let config = load_config();

    println!("Redis URL: {}", config.redis);
    if redis::Client::open(config.redis.clone()).is_err() {
        eprintln!("Failed to create Redis client for {}", config.redis);
        std::process::exit(1);
    }

    let port = config.port.unwrap_or_else(defaults::port);
    let address = parse_host(config.host.clone());
    let router_url = router_base_url(&config.router_address);
    println!("Router address: {router_url}");
    println!(
        "S3 bucket: {} ({})",
        config.s3.bucket, config.s3.region
    );
    println!("Starting server: http://localhost:{port}");

    let figment = rocket::Config::figment()
        .merge(("address", address))
        .merge(("port", port));
    let state = ApiState { config };

    rocket::custom(figment)
        .manage(state)
        .mount("/", routes![index, health])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_box_defaults_interoperate() {
        let cfg = default_config();
        assert_eq!(cfg.port, Some(3400));
        assert_eq!(cfg.host.as_deref(), Some("0.0.0.0"));
        assert_eq!(cfg.redis, "redis://localhost:6379");
        assert_eq!(cfg.router_address, "127.0.0.1:3300");
        assert_eq!(router_base_url(&cfg.router_address), "http://127.0.0.1:3300");
        assert_eq!(cfg.s3.bucket, "packages");
        assert_eq!(cfg.s3.region, "us-east-1");
    }

    #[test]
    fn parses_minimal_router_style_s3() {
        let cfg: Config = serde_json::from_str(
            r#"{
                "redis": "redis://localhost:6379",
                "port": 3400,
                "host": "0.0.0.0",
                "s3": {
                    "access_key": "minioadmin",
                    "secret_key": "minioadmin",
                    "bucket": "packages",
                    "region": "us-east-1"
                },
                "router_address": "127.0.0.1:3300"
            }"#,
        )
        .expect("minimal config parses");
        assert_eq!(cfg.s3.endpoint, None);
    }

    #[test]
    fn accepts_router_alias_for_router_address() {
        let cfg: Config = serde_json::from_str(
            r#"{
                "redis": "redis://localhost:6379",
                "s3": {
                    "access_key": "a",
                    "secret_key": "b",
                    "bucket": "c",
                    "region": "d"
                },
                "router": "127.0.0.1:3300"
            }"#,
        )
        .expect("router alias parses");
        assert_eq!(cfg.router_address, "127.0.0.1:3300");
        assert_eq!(cfg.port, None);
    }
}
