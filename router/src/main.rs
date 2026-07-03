use actix_web::{App, HttpResponse, HttpServer, Responder, get};
use serde::{Deserialize, Serialize};

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Smoothie Router")
}

#[derive(Serialize, Deserialize)]
struct ServerConfig {
    id: String,
    address: String,
    tunnel: bool,
    power: u32,
}

#[derive(Serialize, Deserialize)]
struct Config {
    servers: Vec<ServerConfig>,
    redis: String,
    port: Option<u16>,
    host: Option<String>,
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

    let port = config.port.unwrap_or(8080);
    println!("Starting server: http://localhost:{}", port);

    HttpServer::new(|| App::new().service(hello))
        .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
        .run()
        .await
}
