mod container;
mod globals;
mod image;
mod package;
mod port;

use std::{collections::HashMap, sync::Arc};

use actix_web::{App, HttpServer, web};
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
        Err(e) => {
            return actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": e.to_string()}));
        }
    };

    let injected = match state.api.inject(tarball, argv).await {
        Ok(i) => i,
        Err(e) => {
            return actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": e.to_string()}));
        }
    };

    let id = injected.id().to_string();
    state.injected.lock().await.insert(id.clone(), InjectedEntry {
        package_id: package_id.clone(),
        container: injected,
    });

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
        Err(e) => {
            return actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": e.to_string()}));
        }
    };

    let host_port = running.host_port();
    let container_port = running.container_port();
    let rid = running.id().to_string();
    state.running.lock().await.insert(rid.clone(), RunningEntry {
        package_id: entry.package_id,
        container: running,
    });

    actix_web::HttpResponse::Ok()
        .json(serde_json::json!({"id": rid, "host_port": host_port, "container_port": container_port}))
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

    match state.api.kill(entry.container) {
        Ok(()) => actix_web::HttpResponse::Ok().json(serde_json::json!({"status": "ok"})),
        Err(e) => actix_web::HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": e.to_string()})),
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

    if let Some(idle_pool) = globals::IDLE_CONTAINERS.get() {
        let idle_ids = idle_pool.list_ids().await;
        for id in idle_ids {
            let is_injected = state.injected.lock().await.contains_key(&id);
            let is_running = state.running.lock().await.contains_key(&id);
            if !is_injected && !is_running {
                containers.push(ContainerInfo {
                    id,
                    package: None,
                    idle: Some(true),
                    c_port: None,
                    h_port: None,
                });
            }
        }
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

#[actix_web::get("/image/ensure/alpine")]
async fn ensure_alpine_route() -> actix_web::HttpResponse {
    match image::ensure_alpine().await {
        Ok(path) => actix_web::HttpResponse::Ok()
            .json(serde_json::json!({"status": "ok", "path": path.to_string_lossy()})),
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

    println!("Preparing package...");

    package::ensure_dirs();
    package::cleanup_stale_temps();

    println!("Ensuring Alpine image is available...");

    let alpine_archive = image::ensure_alpine()
        .await
        .map_err(std::io::Error::other)?;
    let data_root = alpine_archive
        .parent()
        .expect("Alpine archive has no parent")
        .parent()
        .expect("images directory has no parent");

    println!("Initializing container runtime...");

    let runtime_root = data_root.join("runtime");
    let idle_root = data_root.join("containers").join("idle");
    std::fs::create_dir_all(&runtime_root)?;
    std::fs::create_dir_all(&idle_root)?;
    globals::RUNTIME
        .set(container::CrunRuntime::new(runtime_root))
        .expect("Failed to initialize container runtime");

    println!("Initializing idle containers...");

    globals::IDLE_CONTAINERS
        .set(Arc::new(
            container::IdleContainerPool::new(idle_root, alpine_archive, config.idle_containers)
                .await
                .map_err(std::io::Error::other)?,
        ))
        .expect("Failed to initialize idle containers");

    println!("Starting server...");

    let port = config.port.unwrap_or(8080);

    let api = container::ContainerApi::new(
        globals::RUNTIME.get().expect("RUNTIME not set").clone(),
        globals::IDLE_CONTAINERS
            .get()
            .expect("IDLE_CONTAINERS not set")
            .clone(),
    );

    let app_state = web::Data::new(AppState {
        api,
        injected: Mutex::new(HashMap::new()),
        running: Mutex::new(HashMap::new()),
    });

    println!("Started server: http://localhost:{}", port);

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(prepare_package_route)
            .service(get_package_route)
            .service(inject_container)
            .service(run_container)
            .service(kill_container)
            .service(get_container_port)
            .service(list_containers)
            .service(ensure_alpine_route)
    })
    .bind((config.host.unwrap_or_else(|| "0.0.0.0".into()), port))?
    .run()
    .await
}
