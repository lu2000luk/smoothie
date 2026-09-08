mod utils;

use actix_web::{App, HttpResponse, HttpServer, Responder, get, web};
use dashmap::{DashMap, mapref::entry::Entry};
use moka::future::Cache;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, LazyLock, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static ALLOW_CACHE: LazyLock<Cache<String, bool>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(2000)
        .time_to_live(Duration::from_secs(900)) // 15 minutes
        .build()
});

const CONTAINER_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const IDLE_REAPER_INTERVAL: Duration = Duration::from_secs(60);

struct Bind {
    port: u16,
    host: String,
    container: ContainerTarget,
    last: Instant,
}

#[derive(Clone)]
struct ContainerTarget {
    id: String,
    hypervisor_url: String,
}

static BINDINGS: LazyLock<Arc<DashMap<String, Bind>>> = LazyLock::new(|| Arc::new(DashMap::new()));
static ROUTE_LOCKS: LazyLock<DashMap<String, Arc<tokio::sync::Mutex<()>>>> =
    LazyLock::new(DashMap::new);

static REDIS_CLIENT: OnceLock<redis::Client> = OnceLock::new();

static SERVERS: OnceLock<Vec<ServerConfig>> = OnceLock::new();
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

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

#[derive(Deserialize)]
struct InjectResponse {
    id: String,
}

#[derive(Deserialize)]
struct RunResponse {
    id: String,
    host_port: u16,
}

struct StartedContainer {
    id: String,
    host_port: u16,
}

async fn resolve_package_id(host: &str) -> Result<Option<String>, redis::RedisError> {
    let Some(client) = REDIS_CLIENT.get() else {
        return Ok(None);
    };
    let mut conn = client.get_multiplexed_async_connection().await?;
    let domain_key = format!("domains:{host}");

    let app_id: Option<String> = match conn.hget(&domain_key, "bind").await {
        Ok(value) => value,
        Err(_) => {
            // Compatibility with early installations where the domain key
            // directly contained a package ID.
            let package_id: Option<String> = conn.get(&domain_key).await?;
            return Ok(package_id.filter(|value| value != "0"));
        }
    };
    let Some(app_id) = app_id else {
        return Ok(None);
    };

    let build_id: Option<String> = conn.hget(format!("apps:{app_id}"), "build").await?;
    let Some(build_id) = build_id else {
        return Ok(None);
    };

    conn.hget(format!("builds:{build_id}"), "package").await
}

fn hypervisor_base_url(address: &str) -> String {
    let address = address.trim_end_matches('/');
    if address.starts_with("http://") || address.starts_with("https://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    }
}

fn server_dial_host(server: &ServerConfig) -> Option<String> {
    reqwest::Url::parse(&hypervisor_base_url(&server.address))
        .ok()?
        .host_str()
        .map(str::to_owned)
}

async fn start_container(
    server: &ServerConfig,
    package_id: &str,
) -> Result<StartedContainer, String> {
    let base_url = hypervisor_base_url(&server.address);
    let inject_response = HTTP_CLIENT
        .post(format!("{base_url}/container/inject/{package_id}"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|error| format!("inject request failed: {error}"))?;

    if !inject_response.status().is_success() {
        let status = inject_response.status();
        let body = inject_response.text().await.unwrap_or_default();
        return Err(format!("inject returned {status}: {body}"));
    }

    let injected: InjectResponse = inject_response
        .json()
        .await
        .map_err(|error| format!("invalid inject response: {error}"))?;
    let run_response = HTTP_CLIENT
        .post(format!("{base_url}/container/run/{}", injected.id))
        .query(&[("port", 8080_u16)])
        .send()
        .await
        .map_err(|error| format!("run request failed: {error}"))?;

    if !run_response.status().is_success() {
        let status = run_response.status();
        let body = run_response.text().await.unwrap_or_default();
        return Err(format!("run returned {status}: {body}"));
    }

    run_response
        .json::<RunResponse>()
        .await
        .map(|response| StartedContainer {
            id: response.id,
            host_port: response.host_port,
        })
        .map_err(|error| format!("invalid run response: {error}"))
}

async fn terminate_container(target: ContainerTarget) {
    let response = HTTP_CLIENT
        .post(format!(
            "{}/container/shutdown/{}",
            target.hypervisor_url, target.id
        ))
        .timeout(Duration::from_secs(40))
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => {
            println!("Stopped idle container {}", target.id);
        }
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!(
                "Failed to stop idle container {}: hypervisor returned {status}: {body}",
                target.id
            );
        }
        Err(error) => eprintln!("Failed to stop idle container {}: {error}", target.id),
    }
}

fn drain_idle_bindings(
    bindings: &DashMap<String, Bind>,
    idle_timeout: Duration,
) -> Vec<ContainerTarget> {
    let mut targets = Vec::new();
    bindings.retain(|_, binding| {
        if binding.last.elapsed() >= idle_timeout {
            targets.push(binding.container.clone());
            false
        } else {
            true
        }
    });
    targets
}

async fn reap_idle_bindings(bindings: Arc<DashMap<String, Bind>>) {
    loop {
        tokio::time::sleep(IDLE_REAPER_INTERVAL).await;
        for target in drain_idle_bindings(&bindings, CONTAINER_IDLE_TIMEOUT) {
            tokio::spawn(terminate_container(target));
        }
    }
}

#[get("/route")]
async fn route(query: web::Query<RouteQuery>) -> impl Responder {
    // Response format: [{"dial":"host:port"}]
    // Before responding: choose server and port, if tunnel enable port, store the binding so that its not needed again, inform the supervisor to bind port to container
    // After responding: log timings in db
    let now = Instant::now();

    let host = query.host.clone();
    let route_lock = ROUTE_LOCKS
        .entry(host.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _route_guard = route_lock.lock().await;

    let scheme = query.scheme.clone();
    let ip = query.ip.clone();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();
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

    if let Entry::Occupied(mut entry) = BINDINGS.entry(host.clone()) {
        if entry.get().last.elapsed() < CONTAINER_IDLE_TIMEOUT {
            already_bound = true;
            let binding = entry.get_mut();
            binding.last = Instant::now();
            solved_host = Some(binding.host.clone());
            solved_port = Some(binding.port);
        } else {
            let stale = entry.remove();
            tokio::spawn(terminate_container(stale.container));
        }
    }

    if solved_host.is_none() || solved_port.is_none() {
        let package_id = match resolve_package_id(&host).await {
            Ok(Some(package_id)) => package_id,
            Ok(None) => {
                return HttpResponse::NotFound()
                    .append_header(("X-Error", "No package is bound to this host"))
                    .body(now.elapsed().as_nanos().to_string());
            }
            Err(error) => {
                return HttpResponse::InternalServerError()
                    .append_header(("X-Error", format!("Redis lookup failed: {error}")))
                    .body(now.elapsed().as_nanos().to_string());
            }
        };

        let Some(servers) = SERVERS.get() else {
            return HttpResponse::InternalServerError()
                .append_header(("X-Error", "SERVERS not initialized"))
                .body(now.elapsed().as_nanos().to_string());
        };
        if servers.is_empty() {
            return HttpResponse::ServiceUnavailable()
                .append_header(("X-Error", "No servers configured"))
                .body(now.elapsed().as_nanos().to_string());
        }

        let mut failures = Vec::new();
        for server in utils::rank_servers(servers, &host) {
            let Some(dial_host) = server_dial_host(&server) else {
                failures.push(format!("{}: invalid address", server.id));
                continue;
            };

            match start_container(&server, &package_id).await {
                Ok(container) => {
                    solved_host = Some(dial_host.clone());
                    solved_port = Some(container.host_port);
                    is_tunnel = server.tunnel || server.tunnel_address.is_some();
                    utils::record_server_request(&server.id, &host);
                    BINDINGS.insert(
                        host.clone(),
                        Bind {
                            port: container.host_port,
                            host: dial_host,
                            container: ContainerTarget {
                                id: container.id,
                                hypervisor_url: hypervisor_base_url(&server.address),
                            },
                            last: Instant::now(),
                        },
                    );
                    break;
                }
                Err(error) => failures.push(format!("{}: {error}", server.id)),
            }
        }

        if solved_host.is_none() || solved_port.is_none() {
            eprintln!(
                "Failed to start package {package_id}: {}",
                failures.join("; ")
            );
            return HttpResponse::BadGateway()
                .append_header(("X-Error", "All hypervisors failed"))
                .body(now.elapsed().as_nanos().to_string());
        }
    }

    let solved_host_spawn = solved_host.clone();
    let solved_port_spawn = solved_port;

    tokio::spawn(async move {
        let client = match REDIS_CLIENT.get() {
            Some(client) => client,
            None => {
                return;
            }
        };

        let mut conn = match client.get_multiplexed_async_connection().await {
            Ok(conn) => conn,
            Err(_) => {
                return;
            }
        };

        let _: redis::RedisResult<String> = conn
            .xadd(
                "req:".to_owned() + &id_spawn,
                "info",
                &[
                    ("host", host_spawn.as_str()),
                    ("scheme", scheme_spawn.as_str()),
                    ("ip", ip_spawn.as_str()),
                    ("timestamp", timestamp_spawn.as_str()),
                ],
            )
            .await;

        let _: redis::RedisResult<String> = conn
            .xadd(
                "req:".to_owned() + &id_spawn,
                "router",
                &[
                    ("already_bound", already_bound.to_string().as_str()),
                    ("solved_host", solved_host_spawn.as_deref().unwrap_or("")),
                    (
                        "solved_port",
                        solved_port_spawn
                            .map(|p| p.to_string())
                            .as_deref()
                            .unwrap_or(""),
                    ),
                    ("is_tunnel", is_tunnel.to_string().as_str()),
                ],
            )
            .await;

        println!("[IN] {}", id_spawn);
    });

    if solved_host.is_none() || solved_port.is_none() {
        let elapsed = now.elapsed();
        return HttpResponse::InternalServerError()
            .append_header(("X-Error", "Failed to solve host and port"))
            .body(elapsed.as_nanos().to_string());
    }

    HttpResponse::Ok()
        .append_header(("X-Time", now.elapsed().as_nanos().to_string()))
        .body(format!(
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

    match resolve_package_id(&query.domain).await {
        Ok(Some(_)) => {
            ALLOW_CACHE.insert(query.domain.clone(), true).await;
            HttpResponse::Ok()
                .append_header(("X-Cache", "MISS"))
                .body(now.elapsed().as_nanos().to_string())
        }
        Ok(None) => {
            ALLOW_CACHE.insert(query.domain.clone(), false).await;
            HttpResponse::Forbidden()
                .append_header(("X-Cache", "MISS"))
                .body(now.elapsed().as_nanos().to_string())
        }
        Err(error) => HttpResponse::InternalServerError()
            .append_header(("X-Error", format!("Redis lookup failed: {error}")))
            .body(now.elapsed().as_nanos().to_string()),
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
    REDIS_CLIENT
        .set(redis_client)
        .expect("Failed to set Redis client");

    let port = config.port.unwrap_or(8080);
    println!("Starting server: http://localhost:{}", port);

    SERVERS
        .set(config.servers.clone())
        .expect("Failed to set servers");

    tokio::spawn(reap_idle_bindings(BINDINGS.clone()));

    HttpServer::new(|| App::new().service(hello).service(allow).service(route))
        .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
        .run()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(id: &str, last: Instant) -> Bind {
        Bind {
            port: 3000,
            host: "127.0.0.1".into(),
            container: ContainerTarget {
                id: id.into(),
                hypervisor_url: "http://127.0.0.1:3100".into(),
            },
            last,
        }
    }

    #[test]
    fn drains_only_bindings_idle_for_thirty_minutes() {
        let bindings = DashMap::new();
        bindings.insert(
            "idle.example".into(),
            binding("idle", Instant::now() - CONTAINER_IDLE_TIMEOUT),
        );
        bindings.insert("active.example".into(), binding("active", Instant::now()));

        let targets = drain_idle_bindings(&bindings, CONTAINER_IDLE_TIMEOUT);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "idle");
        assert!(!bindings.contains_key("idle.example"));
        assert!(bindings.contains_key("active.example"));
    }
}
