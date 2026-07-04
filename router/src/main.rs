mod ports;
mod utils;

use actix_web::{App, HttpResponse, HttpServer, Responder, get, web};
use dashmap::DashMap;
use moka::future::Cache;
use r2d2::{Pool, PooledConnection};
use redis::Commands;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, LazyLock, OnceLock}, time::{Duration, Instant},
};

use crate::ports::generate_ports;

static ALLOW_CACHE: LazyLock<Cache<String, bool>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(2000)
        .time_to_live(Duration::from_secs(900)) // 15 minutes
        .build()
});

struct Bind {
    port: i16,
    host: String,
    last: Instant,
}

static BINDINGS: LazyLock<Arc<DashMap<String, Bind>>> =
    LazyLock::new(|| Arc::new(DashMap::new()));

static REDIS_POOL: OnceLock<Pool<redis::Client>> = OnceLock::new();

static SERVERS: OnceLock<Vec<ServerConfig>> = OnceLock::new();

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

#[get("/route")]
async fn route(query: web::Query<RouteQuery>) -> impl Responder {
    // Response format: [{"dial":"host:port"}]
    // Before responding: choose server and port, if tunnel enable port, store the binding so that its not needed again, inform the supervisor to bind port to container 
    // After responding: log timings in db
    let now = Instant::now();

    let host = query.host.clone();
    let scheme = query.scheme.clone();
    let ip = query.ip.clone();
    let timestamp = now.elapsed().as_nanos().to_string();
    let id = query.id.clone();

    let host_spawn = host.clone();
    let scheme_spawn = scheme.clone();
    let ip_spawn = ip.clone();
    let id_spawn = id.clone();
    let timestamp_spawn = timestamp.clone();

    let mut already_bound = false;
    let mut is_tunnel = false;

    let mut solved_port = None;
    let mut solved_host = None;

    if let Some(bind) = BINDINGS.get(&host) {
        if bind.last.elapsed() < Duration::from_mins(10) {
            already_bound = true;
            BINDINGS.insert(host.clone(), Bind {
                port: bind.port,
                host: bind.host.clone(),
                last: Instant::now(),
            });
            solved_host = Some(bind.host.clone());
            solved_port = Some(bind.port);
        } else {
            BINDINGS.remove(&host);
        }
    }

    if solved_host.is_none() || solved_port.is_none() {
        if let Some(servers) = SERVERS.get() {
            if let Some(choosen_server) = utils::get_server(&servers) {
                solved_host = Some(choosen_server.address.clone().split(":").nth(0).unwrap_or("").to_string());
                
                // TODO: supervisor/bindPort
                
                if choosen_server.tunnel_address.is_some() {
                    is_tunnel = true;
                    // TODO: tunnel/createConnection
                }
                
                // TODO: supervisor/initContainer
                BINDINGS.insert(host.clone(), Bind {
                    port: solved_port.unwrap_or(0),
                    host: solved_host.clone().unwrap_or("".to_string()),
                    last: Instant::now(),
                });
            } else {
                return HttpResponse::InternalServerError()
                    .append_header(("X-Error", "No server available"))
                    .body(now.elapsed().as_nanos().to_string());
            }
        } else {
            return HttpResponse::InternalServerError()
                .append_header(("X-Error", "SERVERS not initialized"))
                .body(now.elapsed().as_nanos().to_string());
        }
        let port = ports::consume_port().await.unwrap_or(0);
        solved_port = Some(port);
    }

    let solved_host_spawn = solved_host.clone();
    let solved_port_spawn = solved_port.clone();

    tokio::spawn(async move {
        let mut conn: PooledConnection<redis::Client> = match REDIS_POOL.get() {
            Some(pool) => match pool.get() {
                Ok(conn) => conn,
                Err(_) => {
                    return;
                }
            },
            None => {
                return;
            }
        };

        let _ : redis::RedisResult<String> = conn.xadd("req:".to_owned() + &id_spawn, "info", &[
            ("host", host_spawn.as_str()),
            ("scheme", scheme_spawn.as_str()),
            ("ip", ip_spawn.as_str()),
            ("timestamp", timestamp_spawn.as_str()),
        ]);

        let _ : redis::RedisResult<String> = conn.xadd("req:".to_owned() + &id_spawn, "router", &[
            ("already_bound", already_bound.to_string().as_str()),
            ("solved_host", solved_host_spawn.as_deref().unwrap_or("")),
            ("solved_port", solved_port_spawn.map(|p| p.to_string()).as_deref().unwrap_or("")),
            ("is_tunnel", is_tunnel.to_string().as_str())
        ]);
    });

    if solved_host.is_none() || solved_port.is_none() {
        let elapsed = now.elapsed();
        return HttpResponse::InternalServerError()
            .append_header(("X-Error", "Failed to solve host and port"))
            .body(elapsed.as_nanos().to_string());
    }

    HttpResponse::Ok().body(format!(
        "[{{\"dial\":\"{}:{}\"}}]",
        solved_host.unwrap_or_else(|| host.clone()),
        solved_port.unwrap_or(0)
    ))
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

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ServerConfig {
    id: String,
    address: String,
    tunnel: bool,
    tunnel_address: Option<String>,
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

    generate_ports().await;

    SERVERS.set(config.servers.clone()).expect("Failed to set servers");

    HttpServer::new(|| App::new().service(hello).service(allow).service(route))
        .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
        .run()
        .await
}
