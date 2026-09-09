mod apps;
mod auth;
mod config;
mod deployments;
mod packages;
mod services;
mod state;

use rocket::State;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::Status;
use rocket::{get, options, routes};

use config::{
    defaults, github_configured, load_config, parse_host, router_base_url, s3_force_path_style,
};
use state::{ApiState, Cors};

#[get("/")]
fn index() -> &'static str {
    "Smoothie API"
}

#[get("/health")]
fn health(state: &State<ApiState>) -> rocket::serde::json::Json<serde_json::Value> {
    rocket::serde::json::Json(serde_json::json!({
        "status": "ok",
        "router": router_base_url(&state.config.router_address),
        "github_configured": github_configured(&state.config.github),
    }))
}

#[options("/<_path..>")]
fn preflight(_path: std::path::PathBuf) -> Status {
    Status::Ok
}

#[rocket::launch]
fn rocket() -> _ {
    println!("Loading config...");
    let config = load_config();

    println!("Redis URL: {}", config.redis);
    let redis_client = match redis::Client::open(config.redis.clone()) {
        Ok(client) => client,
        Err(_) => {
            eprintln!("Failed to create Redis client for {}", config.redis);
            std::process::exit(1);
        }
    };

    if github_configured(&config.github) {
        println!(
            "GitHub auth: enabled (redirect_uri={})",
            config.github.redirect_uri
        );
    } else {
        println!(
            "GitHub auth: DISABLED - set github.client_id / github.client_secret in config.json"
        );
    }

    let port = config.port.unwrap_or_else(defaults::port);
    let address = parse_host(config.host.clone());
    let router_url = router_base_url(&config.router_address);
    println!("Router address: {router_url}");
    println!("S3 bucket: {} ({})", config.s3.bucket, config.s3.region);
    println!("Starting server: http://localhost:{port}");

    let limits = Limits::default()
        .limit("file", 200.mebibytes())
        .limit("data-form", 201.mebibytes());
    let figment = rocket::Config::figment()
        .merge(("address", address))
        .merge(("port", port))
        .merge(("limits", limits));
    let frontend_url = config.github.frontend_url.clone();
    let credentials = aws_sdk_s3::config::Credentials::new(
        config.s3.access_key.clone(),
        config.s3.secret_key.clone(),
        None,
        None,
        "smoothie-config",
    );
    let mut s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new(config.s3.region.clone()))
        .credentials_provider(credentials)
        .force_path_style(s3_force_path_style(config.s3.force_path_style));
    if let Some(endpoint) = config.s3.endpoint.as_deref() {
        s3_config = s3_config.endpoint_url(endpoint);
    }
    let state = ApiState {
        config,
        redis: redis_client,
        http: reqwest::Client::new(),
        s3: aws_sdk_s3::Client::from_conf(s3_config.build()),
        service_locks: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    };

    rocket::custom(figment)
        .manage(state)
        .attach(Cors { frontend_url })
        .mount(
            "/",
            routes![
                index,
                health,
                preflight,
                auth::github_login,
                auth::github_login_url,
                auth::github_callback,
                auth::me,
                auth::logout,
                apps::list_apps,
                apps::create_app,
                apps::get_app,
                apps::update_app,
                apps::delete_app,
                services::list_services,
                services::get_service,
                services::create_service,
                services::update_service,
                services::update_service_position,
                services::delete_service,
                packages::list_service_packages,
                packages::upload_service_package,
                packages::delete_service_package,
                packages::activate_service_package,
                deployments::list_service_deployments,
                deployments::get_deployment_status,
                deployments::start_service,
                deployments::retry_deployment,
                deployments::stop_service,
            ],
        )
}
