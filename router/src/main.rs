mod utils;

use actix_web::{App, HttpResponse, HttpServer, Responder, delete, get, post, web};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DeploymentStatus {
    Running,
    Deleted,
}

#[derive(Clone)]
struct DeploymentBinding {
    service_id: String,
    package_id: String,
    argv: Vec<String>,
    container: ContainerTarget,
    hypervisor_id: String,
    host: String,
    host_port: u16,
    container_port: u16,
    status: DeploymentStatus,
}

#[derive(Clone, Deserialize)]
struct DeploymentRequest {
    service_id: String,
    package_id: String,
    argv: Vec<String>,
    port: u16,
}

#[derive(Debug, PartialEq, Serialize)]
struct DeploymentResponse {
    deployment_id: String,
    status: DeploymentStatus,
    host: Option<String>,
    host_port: Option<u16>,
    container_port: Option<u16>,
    container_id: Option<String>,
}

static BINDINGS: LazyLock<Arc<DashMap<String, Bind>>> = LazyLock::new(|| Arc::new(DashMap::new()));
static ROUTE_LOCKS: LazyLock<DashMap<String, Arc<tokio::sync::Mutex<()>>>> =
    LazyLock::new(DashMap::new);
static DEPLOYMENTS: LazyLock<DashMap<String, DeploymentBinding>> = LazyLock::new(DashMap::new);
static DEPLOYMENT_LOCKS: LazyLock<DashMap<String, Arc<tokio::sync::Mutex<()>>>> =
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
    container_port: u16,
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
    argv: &[String],
    container_port: u16,
) -> Result<StartedContainer, String> {
    let base_url = hypervisor_base_url(&server.address);
    let inject_response = HTTP_CLIENT
        .post(format!("{base_url}/container/inject/{package_id}"))
        .json(&serde_json::json!({ "argv": argv }))
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
        .query(&[("port", container_port)])
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
            container_port,
        })
        .map_err(|error| format!("invalid run response: {error}"))
}

async fn shutdown_container(target: &ContainerTarget) -> Result<(), String> {
    let response = HTTP_CLIENT
        .post(format!(
            "{}/container/shutdown/{}",
            target.hypervisor_url, target.id
        ))
        .timeout(Duration::from_secs(40))
        .send()
        .await
        .map_err(|error| format!("shutdown request failed: {error}"))?;

    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("shutdown returned {status}: {body}"))
}

async fn terminate_container(target: ContainerTarget) {
    match shutdown_container(&target).await {
        Ok(()) => println!("Stopped idle container {}", target.id),
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

fn validate_deployment_request(
    deployment_id: &str,
    request: &DeploymentRequest,
) -> Result<(), &'static str> {
    if deployment_id.trim().is_empty() {
        return Err("deployment_id must not be empty");
    }
    if request.service_id.trim().is_empty() {
        return Err("service_id must not be empty");
    }
    if request.package_id.trim().is_empty() {
        return Err("package_id must not be empty");
    }
    if request.argv.is_empty() {
        return Err("argv must not be empty");
    }
    if request.port == 0 {
        return Err("port must not be zero");
    }
    Ok(())
}

fn find_running_deployment(
    deployments: &DashMap<String, DeploymentBinding>,
    deployment_id: &str,
) -> Option<DeploymentBinding> {
    deployments
        .get(deployment_id)
        .filter(|binding| binding.status == DeploymentStatus::Running)
        .map(|binding| binding.clone())
}

fn deployment_matches_request(binding: &DeploymentBinding, request: &DeploymentRequest) -> bool {
    binding.service_id == request.service_id
        && binding.package_id == request.package_id
        && binding.argv == request.argv
        && binding.container_port == request.port
}

fn remove_deployment(
    deployments: &DashMap<String, DeploymentBinding>,
    deployment_id: &str,
) -> Option<DeploymentBinding> {
    deployments
        .remove(deployment_id)
        .map(|(_, binding)| binding)
}

fn deployment_response(deployment_id: &str, binding: &DeploymentBinding) -> DeploymentResponse {
    DeploymentResponse {
        deployment_id: deployment_id.to_owned(),
        status: binding.status,
        host: Some(binding.host.clone()),
        host_port: Some(binding.host_port),
        container_port: Some(binding.container_port),
        container_id: Some(binding.container.id.clone()),
    }
}

fn deleted_deployment_response(deployment_id: &str) -> DeploymentResponse {
    DeploymentResponse {
        deployment_id: deployment_id.to_owned(),
        status: DeploymentStatus::Deleted,
        host: None,
        host_port: None,
        container_port: None,
        container_id: None,
    }
}

#[post("/deployments/{deployment_id}")]
async fn create_deployment(
    deployment_id: web::Path<String>,
    request: web::Json<DeploymentRequest>,
) -> impl Responder {
    let deployment_id = deployment_id.into_inner();
    let request = request.into_inner();
    if let Err(error) = validate_deployment_request(&deployment_id, &request) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": error }));
    }

    let deployment_lock = DEPLOYMENT_LOCKS
        .entry(deployment_id.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _deployment_guard = deployment_lock.lock().await;

    if let Some(binding) = find_running_deployment(&DEPLOYMENTS, &deployment_id) {
        if deployment_matches_request(&binding, &request) {
            return HttpResponse::Ok().json(deployment_response(&deployment_id, &binding));
        }
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "deployment_id is already running with different configuration"
        }));
    }

    let Some(servers) = SERVERS.get() else {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": "SERVERS not initialized" }));
    };
    if servers.is_empty() {
        return HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({ "error": "No servers configured" }));
    }

    let mut failures = Vec::new();
    for server in utils::rank_servers(servers, &request.service_id) {
        let Some(host) = server_dial_host(&server) else {
            failures.push(format!("{}: invalid address", server.id));
            continue;
        };

        match start_container(&server, &request.package_id, &request.argv, request.port).await {
            Ok(container) => {
                let binding = DeploymentBinding {
                    service_id: request.service_id.clone(),
                    package_id: request.package_id.clone(),
                    argv: request.argv.clone(),
                    container: ContainerTarget {
                        id: container.id,
                        hypervisor_url: hypervisor_base_url(&server.address),
                    },
                    hypervisor_id: server.id.clone(),
                    host,
                    host_port: container.host_port,
                    container_port: container.container_port,
                    status: DeploymentStatus::Running,
                };
                utils::record_server_request(&server.id, &request.service_id);
                let response = deployment_response(&deployment_id, &binding);
                DEPLOYMENTS.insert(deployment_id.clone(), binding);
                return HttpResponse::Ok().json(response);
            }
            Err(error) => failures.push(format!("{}: {error}", server.id)),
        }
    }

    eprintln!(
        "Failed to start deployment {deployment_id} for package {}: {}",
        request.package_id,
        failures.join("; ")
    );
    HttpResponse::BadGateway().json(serde_json::json!({ "error": "All hypervisors failed" }))
}

#[get("/deployments/{deployment_id}")]
async fn get_deployment(deployment_id: web::Path<String>) -> impl Responder {
    let deployment_id = deployment_id.into_inner();
    if deployment_id.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "deployment_id must not be empty" }));
    }

    match find_running_deployment(&DEPLOYMENTS, &deployment_id) {
        Some(binding) => HttpResponse::Ok().json(deployment_response(&deployment_id, &binding)),
        None => {
            HttpResponse::NotFound().json(serde_json::json!({ "error": "deployment not found" }))
        }
    }
}

#[delete("/deployments/{deployment_id}")]
async fn delete_deployment(deployment_id: web::Path<String>) -> impl Responder {
    let deployment_id = deployment_id.into_inner();
    if deployment_id.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "deployment_id must not be empty" }));
    }

    let deployment_lock = DEPLOYMENT_LOCKS
        .entry(deployment_id.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _deployment_guard = deployment_lock.lock().await;

    let Some(binding) = find_running_deployment(&DEPLOYMENTS, &deployment_id) else {
        return HttpResponse::Ok().json(deleted_deployment_response(&deployment_id));
    };

    if let Err(error) = shutdown_container(&binding.container).await {
        return HttpResponse::BadGateway().json(serde_json::json!({ "error": error }));
    }

    remove_deployment(&DEPLOYMENTS, &deployment_id);
    println!(
        "Stopped deployment {deployment_id} for service {} package {} on hypervisor {}",
        binding.service_id, binding.package_id, binding.hypervisor_id
    );
    HttpResponse::Ok().json(deleted_deployment_response(&deployment_id))
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

            match start_container(&server, &package_id, &["./main".into()], 8080).await {
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

    HttpServer::new(|| {
        App::new()
            .service(hello)
            .service(allow)
            .service(route)
            .service(create_deployment)
            .service(get_deployment)
            .service(delete_deployment)
    })
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

    fn deployment_binding() -> DeploymentBinding {
        DeploymentBinding {
            service_id: "service-1".into(),
            package_id: "package-1".into(),
            argv: vec!["./server".into()],
            container: ContainerTarget {
                id: "container-1".into(),
                hypervisor_url: "http://127.0.0.1:3100".into(),
            },
            hypervisor_id: "hypervisor-1".into(),
            host: "127.0.0.1".into(),
            host_port: 3000,
            container_port: 8080,
            status: DeploymentStatus::Running,
        }
    }

    fn deployment_request() -> DeploymentRequest {
        DeploymentRequest {
            service_id: "service-1".into(),
            package_id: "package-1".into(),
            argv: vec!["./server".into()],
            port: 8080,
        }
    }

    #[test]
    fn validates_explicit_deployment_requests() {
        let mut request = deployment_request();
        assert_eq!(
            validate_deployment_request("deployment-1", &request),
            Ok(())
        );

        assert_eq!(
            validate_deployment_request("  ", &request),
            Err("deployment_id must not be empty")
        );

        request.service_id = " ".into();
        assert_eq!(
            validate_deployment_request("deployment-1", &request),
            Err("service_id must not be empty")
        );
        request = deployment_request();
        request.package_id.clear();
        assert_eq!(
            validate_deployment_request("deployment-1", &request),
            Err("package_id must not be empty")
        );
        request = deployment_request();
        request.argv.clear();
        assert_eq!(
            validate_deployment_request("deployment-1", &request),
            Err("argv must not be empty")
        );
        request = deployment_request();
        request.port = 0;
        assert_eq!(
            validate_deployment_request("deployment-1", &request),
            Err("port must not be zero")
        );
    }

    #[test]
    fn deployment_request_compatibility_uses_full_launch_config() {
        let binding = deployment_binding();
        let request = deployment_request();
        assert!(deployment_matches_request(&binding, &request));

        let mut changed = request.clone();
        changed.service_id = "service-2".into();
        assert!(!deployment_matches_request(&binding, &changed));

        let mut changed = request.clone();
        changed.package_id = "package-2".into();
        assert!(!deployment_matches_request(&binding, &changed));

        let mut changed = request.clone();
        changed.argv.push("--production".into());
        assert!(!deployment_matches_request(&binding, &changed));

        let mut changed = request;
        changed.port = 9090;
        assert!(!deployment_matches_request(&binding, &changed));
    }

    #[test]
    fn deployment_registry_lookup_and_removal_are_idempotent() {
        let deployments = DashMap::new();
        deployments.insert("deployment-1".into(), deployment_binding());

        let first = find_running_deployment(&deployments, "deployment-1").unwrap();
        let second = find_running_deployment(&deployments, "deployment-1").unwrap();
        assert_eq!(first.container.id, "container-1");
        assert_eq!(second.container.id, "container-1");
        assert_eq!(deployments.len(), 1);

        assert!(remove_deployment(&deployments, "deployment-1").is_some());
        assert!(remove_deployment(&deployments, "deployment-1").is_none());
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
