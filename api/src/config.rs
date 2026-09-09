use rocket::serde::{Deserialize, Serialize};

fn default_github_redirect_uri() -> String {
    "http://localhost:3400/auth/github/callback".into()
}

fn default_frontend_url() -> String {
    "http://localhost:5173".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
pub struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub supports_range: Option<bool>,
    pub force_path_style: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
pub struct GithubConfig {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default = "default_github_redirect_uri")]
    pub redirect_uri: String,
    #[serde(default = "default_frontend_url")]
    pub frontend_url: String,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: default_github_redirect_uri(),
            frontend_url: default_frontend_url(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    pub redis: String,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub s3: S3Config,
    #[serde(alias = "router", alias = "router_url")]
    pub router_address: String,
    #[serde(default)]
    pub github: GithubConfig,
}

pub mod defaults {
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
pub fn default_config() -> Config {
    Config {
        redis: defaults::redis(),
        port: Some(defaults::port()),
        host: Some(defaults::host()),
        s3: defaults::s3(),
        router_address: defaults::router_address(),
        github: GithubConfig::default(),
    }
}

pub fn load_config() -> Config {
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

pub fn router_base_url(address: &str) -> String {
    let address = address.trim_end_matches('/');
    if address.starts_with("http://") || address.starts_with("https://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    }
}

pub fn parse_host(host: Option<String>) -> std::net::IpAddr {
    use std::str::FromStr;
    let raw = host.unwrap_or_else(defaults::host);
    if raw.eq_ignore_ascii_case("localhost") {
        return std::net::IpAddr::from_str("127.0.0.1").expect("loopback parses");
    }
    match std::net::IpAddr::from_str(&raw) {
        Ok(ip) => ip,
        Err(_) => {
            eprintln!("Invalid host in config.json: {raw} (want an IP like 0.0.0.0)");
            std::process::exit(1);
        }
    }
}

pub fn github_configured(cfg: &GithubConfig) -> bool {
    !cfg.client_id.is_empty() && !cfg.client_secret.is_empty()
}

pub fn s3_force_path_style(configured: Option<bool>) -> bool {
    configured.unwrap_or(true)
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
        assert_eq!(
            router_base_url(&cfg.router_address),
            "http://127.0.0.1:3300"
        );
        assert_eq!(cfg.s3.bucket, "packages");
        assert_eq!(cfg.s3.region, "us-east-1");
        assert!(s3_force_path_style(cfg.s3.force_path_style));
        assert_eq!(
            cfg.github.redirect_uri,
            "http://localhost:3400/auth/github/callback"
        );
        assert_eq!(cfg.github.frontend_url, "http://localhost:5173");
        assert!(!github_configured(&cfg.github));
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
        assert!(s3_force_path_style(cfg.s3.force_path_style));
        assert!(!github_configured(&cfg.github));
    }

    #[test]
    fn explicit_virtual_host_s3_style_is_preserved() {
        assert!(!s3_force_path_style(Some(false)));
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

    #[test]
    fn parses_github_auth_config() {
        let cfg: Config = serde_json::from_str(
            r#"{
                "redis": "redis://localhost:6379",
                "s3": {
                    "access_key": "a",
                    "secret_key": "b",
                    "bucket": "c",
                    "region": "d"
                },
                "router_address": "127.0.0.1:3300",
                "github": {
                    "client_id": "Iv1.test",
                    "client_secret": "secret",
                    "redirect_uri": "http://localhost:3400/auth/github/callback",
                    "frontend_url": "http://localhost:5173"
                }
            }"#,
        )
        .expect("github config parses");
        assert!(github_configured(&cfg.github));
        assert_eq!(cfg.github.client_id, "Iv1.test");
    }
}
