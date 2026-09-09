mod container;
mod engine;
mod globals;
mod instant_box;
mod log;
mod package;
mod port;

use std::{collections::HashMap, sync::Arc};

use actix_web::{App, HttpServer, http::StatusCode, web};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

struct InjectedEntry {
    package_id: String,
    container: container::InjectedContainer,
}

struct RunningEntry {
    package_id: String,
    container: container::RunningContainer,
}

struct AppState {
    engine: Arc<engine::Engine>,
    api: container::ContainerApi,
    injected: Mutex<HashMap<String, InjectedEntry>>,
    running: Mutex<HashMap<String, RunningEntry>>,
}

#[derive(Deserialize, Default)]
struct InjectBody {
    argv: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RunQuery {
    port: Option<u16>,
}

#[derive(Serialize)]
struct ContainerInfo {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    c_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    h_port: Option<u16>,
}

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
struct EngineConfig {
    /// Path to a Docker- or Podman-compatible API socket,
    /// e.g. "/var/run/docker.sock" or "/run/podman/podman.sock".
    /// Required: the hypervisor refuses to start without it.
    socket: String,
    /// Base image idle containers are started from.
    #[serde(default = "defaults::image")]
    image: String,
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
    engine: EngineConfig,
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
    pub fn image() -> String {
        // Fully qualified so Podman resolves it without unqualified-search
        // registry configuration.
        "docker.io/library/alpine:3.24".into()
    }
}

impl ResourceLimits {
    fn to_engine(&self) -> engine::ResourceLimits {
        let as_limit = |value: u64| i64::try_from(value).ok();
        engine::ResourceLimits {
            memory_bytes: as_limit(self.ram),
            // No extra swap beyond the RAM limit.
            memory_swap_bytes: as_limit(self.ram),
            cpu_period_us: as_limit(self.cpu_p),
            cpu_quota_us: as_limit(self.cpu_q),
            pids: None,
        }
    }
}

fn container_error_status(error: &container::ContainerError) -> StatusCode {
    match error {
        container::ContainerError::EmptyCommand | container::ContainerError::Package(_) => {
            StatusCode::BAD_REQUEST
        }
        container::ContainerError::IdlePoolExhausted => StatusCode::SERVICE_UNAVAILABLE,
        container::ContainerError::Io(_) | container::ContainerError::Engine(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn container_error_response(error: container::ContainerError) -> actix_web::HttpResponse {
    actix_web::HttpResponse::build(container_error_status(&error))
        .json(serde_json::json!({"error": error.to_string()}))
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

#[actix_web::post("/container/inject/{package_id}")]
async fn inject_container(
    state: web::Data<AppState>,
    package_id: web::Path<String>,
    body: web::Json<InjectBody>,
) -> actix_web::HttpResponse {
    let argv = body.argv.clone().unwrap_or_else(|| vec!["./main".into()]);

    let tar_path = match package::get_package(&package_id).await {
        Ok(p) => p,
        Err(e) => {
            return actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": e.to_string()}));
        }
    };

    let tarball = match state.api.select_tarball(tar_path).await {
        Ok(t) => t,
        Err(e) => return container_error_response(e),
    };

    let injected = match state.api.inject(tarball, argv).await {
        Ok(i) => i,
        Err(e) => return container_error_response(e),
    };

    let id = injected.id().to_string();
    state.injected.lock().await.insert(
        id.clone(),
        InjectedEntry {
            package_id: package_id.clone(),
            container: injected,
        },
    );

    actix_web::HttpResponse::Ok().json(serde_json::json!({"id": id}))
}

#[actix_web::post("/container/run/{id}")]
async fn run_container(
    state: web::Data<AppState>,
    id: web::Path<String>,
    query: web::Query<RunQuery>,
) -> actix_web::HttpResponse {
    let cport = query.port.unwrap_or(8080);

    let entry = match state.injected.lock().await.remove(&*id) {
        Some(e) => e,
        None => {
            return actix_web::HttpResponse::NotFound()
                .json(serde_json::json!({"error": "injected container not found"}));
        }
    };

    let running = match state.api.run(entry.container, cport).await {
        Ok(r) => r,
        Err(e) => return container_error_response(e),
    };

    let host_port = running.host_port();
    let container_port = running.container_port();
    let rid = running.id().to_string();
    state.running.lock().await.insert(
        rid.clone(),
        RunningEntry {
            package_id: entry.package_id,
            container: running,
        },
    );

    actix_web::HttpResponse::Ok().json(
        serde_json::json!({"id": rid, "host_port": host_port, "container_port": container_port}),
    )
}

#[actix_web::post("/container/shutdown/{id}")]
async fn shutdown_container(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> actix_web::HttpResponse {
    let entry = match state.running.lock().await.remove(&*id) {
        Some(e) => e,
        None => {
            return actix_web::HttpResponse::NotFound()
                .json(serde_json::json!({"error": "running container not found"}));
        }
    };

    match state.api.shutdown(entry.container).await {
        Ok(()) => actix_web::HttpResponse::Ok().json(serde_json::json!({"status": "ok"})),
        Err(e) => container_error_response(e),
    }
}

#[actix_web::post("/container/kill/{id}")]
async fn kill_container(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> actix_web::HttpResponse {
    let entry = match state.running.lock().await.remove(&*id) {
        Some(e) => e,
        None => {
            return actix_web::HttpResponse::NotFound()
                .json(serde_json::json!({"error": "running container not found"}));
        }
    };

    match state.api.kill(entry.container).await {
        Ok(()) => actix_web::HttpResponse::Ok().json(serde_json::json!({"status": "ok"})),
        Err(e) => container_error_response(e),
    }
}

#[actix_web::get("/container/port/{id}")]
async fn get_container_port(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> actix_web::HttpResponse {
    let running = state.running.lock().await;
    match running.get(&*id) {
        Some(entry) => actix_web::HttpResponse::Ok().json(
            serde_json::json!({"host_port": entry.container.host_port(), "container_port": entry.container.container_port()}),
        ),
        None => actix_web::HttpResponse::NotFound()
            .json(serde_json::json!({"error": "running container not found"})),
    }
}

#[actix_web::get("/container/list")]
async fn list_containers(state: web::Data<AppState>) -> actix_web::HttpResponse {
    let mut containers: Vec<ContainerInfo> = Vec::new();

    for id in state.api.idle_ids().await {
        containers.push(ContainerInfo {
            id,
            package: None,
            idle: Some(true),
            c_port: None,
            h_port: None,
        });
    }

    for (id, entry) in state.injected.lock().await.iter() {
        containers.push(ContainerInfo {
            id: id.clone(),
            package: Some(entry.package_id.clone()),
            idle: None,
            c_port: None,
            h_port: None,
        });
    }

    for (id, entry) in state.running.lock().await.iter() {
        containers.push(ContainerInfo {
            id: id.clone(),
            package: Some(entry.package_id.clone()),
            idle: None,
            c_port: Some(entry.container.container_port()),
            h_port: Some(entry.container.host_port()),
        });
    }

    actix_web::HttpResponse::Ok().json(containers)
}

#[actix_web::get("/image/ensure")]
async fn ensure_image_route(state: web::Data<AppState>) -> actix_web::HttpResponse {
    match state.engine.ensure_image().await {
        Ok(()) => actix_web::HttpResponse::Ok()
            .json(serde_json::json!({"status": "ok", "image": state.engine.image()})),
        Err(e) => actix_web::HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": e.to_string()})),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Loading config...");
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
            eprintln!(
                "Note: the hypervisor requires an \"engine\" section pointing at a \
Docker/Podman API socket, e.g. {{\"engine\": {{\"socket\": \"/var/run/docker.sock\"}}}}"
            );
            std::process::exit(1);
        }
    };

    println!("Connecting to Redis...");

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

    println!("Connecting to S3...");

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

    println!("Preparing package cache...");

    package::ensure_dirs();
    package::cleanup_stale_temps();

    println!(
        "Connecting to container engine at {}...",
        config.engine.socket
    );

    let engine = match engine::Engine::connect(
        &config.engine.socket,
        config.engine.image.clone(),
        config.resource_limits.to_engine(),
    )
    .await
    {
        Ok(engine) => Arc::new(engine),
        Err(e) => {
            eprintln!(
                "Failed to reach the container engine socket {}: {}",
                config.engine.socket, e
            );
            eprintln!("The hypervisor does not run without a working Docker/Podman socket.");
            std::process::exit(1);
        }
    };

    println!("Cleaning up containers left over from previous runs...");

    match engine.cleanup_leftovers().await {
        Ok(0) => {}
        Ok(removed) => println!("Removed {removed} leftover container(s)"),
        Err(e) => {
            eprintln!("Failed to clean up leftover containers: {}", e);
            std::process::exit(1);
        }
    }

    println!(
        "Ensuring base image {} is available...",
        config.engine.image
    );

    engine.ensure_image().await.map_err(std::io::Error::other)?;

    println!("Initializing idle containers...");

    let idle_pool = container::IdleContainerPool::new(engine.clone(), config.idle_containers)
        .await
        .map_err(std::io::Error::other)?;

    println!("Starting server...");

    let port = config.port.unwrap_or(8080);

    let api = container::ContainerApi::new(engine.clone(), idle_pool);

    let app_state = web::Data::new(AppState {
        engine: engine.clone(),
        api,
        injected: Mutex::new(HashMap::new()),
        running: Mutex::new(HashMap::new()),
    });
    let box_registry = web::Data::new(instant_box::BoxRegistry::new(engine.clone()));

    println!("Started server: http://localhost:{}", port);

    let result = HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .app_data(box_registry.clone())
            .service(prepare_package_route)
            .service(get_package_route)
            .service(inject_container)
            .service(run_container)
            .service(shutdown_container)
            .service(kill_container)
            .service(get_container_port)
            .service(list_containers)
            .service(ensure_image_route)
            .service(instant_box::create_box)
            .service(instant_box::connect_box)
    })
    .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
    .run()
    .await;

    // Graceful shutdown (SIGINT/SIGTERM): remove every container this run
    // created. A SIGKILLed run is covered by cleanup_leftovers on next boot.
    println!("Shutting down: removing managed containers...");
    match engine.cleanup_instance().await {
        Ok(removed) => println!("Removed {removed} container(s)"),
        Err(e) => eprintln!(
            "Failed to remove managed containers: {}; they are labeled {}={} and will be \
removed on next startup",
            e,
            engine::MANAGED_LABEL,
            engine::MANAGED_LABEL_VALUE
        ),
    }

    result
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn maps_container_errors_to_specific_http_statuses() {
        assert_eq!(
            container_error_status(&container::ContainerError::Package("invalid tar".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            container_error_status(&container::ContainerError::EmptyCommand),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            container_error_status(&container::ContainerError::IdlePoolExhausted),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            container_error_status(&container::ContainerError::Io(std::io::Error::other(
                "internal"
            ))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
