use actix_web::{App, HttpResponse, HttpServer, Responder, get, http::header::ALLOW, web};
use moka::future::Cache;
use r2d2::{Pool, PooledConnection};
use redis::Commands;
use serde::{Deserialize, Serialize};
use std::{
    sync::{LazyLock, OnceLock},
    time::{Duration, Instant},
};
use tokio::time::error::Elapsed;

static ALLOW_CACHE: LazyLock<Cache<String, bool>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(2000)
        .time_to_live(Duration::from_secs(900)) // 15 minutes
        .build()
});

static REDIS_POOL: OnceLock<Pool<redis::Client>> = OnceLock::new();

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Smoothie Router")
}

#[derive(Deserialize)]
struct AllowQuery {
    domain: String,
}

#[derive(Deserialize)]
struct RouteQuery {
    host: String,
    scheme: String,
    id: String,
    ip: String,
}

#[get("/allow")]
async fn allow(query: web::Query<AllowQuery>) -> impl Responder {
    let now = Instant::now();

    if query.domain == "localhost" {
        let elapsed = now.elapsed();
        return HttpResponse::Ok().body(elapsed.as_nanos().to_string());
    }

    let cached = ALLOW_CACHE.get(&query.domain).await;

    if cached == Some(true) {
        let elapsed = now.elapsed();
        return HttpResponse::Ok()
            .append_header(("X-Cache", "HIT"))
            .body(elapsed.as_nanos().to_string());
    } else if cached == Some(false) {
        let elapsed = now.elapsed();
        return HttpResponse::Forbidden()
            .append_header(("X-Cache", "HIT"))
            .body(elapsed.as_nanos().to_string());
    }

    let mut conn: PooledConnection<redis::Client> = match REDIS_POOL.get() {
        Some(pool) => match pool.get() {
            Ok(conn) => conn,
            Err(_) => {
                let elapsed = now.elapsed();
                return HttpResponse::InternalServerError()
                    .append_header(("X-Error", "Failed to get Redis connection (Err)"))
                    .body(elapsed.as_nanos().to_string());
            }
        },
        None => {
            let elapsed = now.elapsed();
            return HttpResponse::InternalServerError()
                .append_header(("X-Error", "Failed to get Redis connection (None)"))
                .body(elapsed.as_nanos().to_string());
        }
    };

    let resp: redis::RedisResult<String> = conn.get("domains:".to_string() + &query.domain);

    match resp {
        Ok(value) => {
            let elapsed = now.elapsed();
            if value != "0" {
                ALLOW_CACHE.insert(query.domain.clone(), true).await;
                return HttpResponse::Ok()
                    .append_header(("X-Cache", "MISS"))
                    .append_header(("X-Removed", "true"))
                    .body(elapsed.as_nanos().to_string());
            } else {
                ALLOW_CACHE.insert(query.domain.clone(), false).await;
                return HttpResponse::Forbidden()
                    .append_header(("X-Cache", "MISS"))
                    .body(elapsed.as_nanos().to_string());
            }
        }
        Err(_) => {
            ALLOW_CACHE.insert(query.domain.clone(), false).await;
            let elapsed = now.elapsed();
            return HttpResponse::Forbidden()
                .append_header(("X-Cache", "MISS"))
                .body(elapsed.as_nanos().to_string());
        }
    }
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

    let redis_client = match redis::Client::open(config.redis.clone()) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to create Redis client: {}", e);
            std::process::exit(1);
        }
    };

    let pool = match r2d2::Pool::builder().build(redis_client) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create Redis connection pool: {}", e);
            std::process::exit(1);
        }
    };
    REDIS_POOL.set(pool).expect("Failed to set Redis pool");

    let port = config.port.unwrap_or(8080);
    println!("Starting server: http://localhost:{}", port);

    HttpServer::new(|| App::new().service(hello).service(allow))
        .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
        .run()
        .await
}
