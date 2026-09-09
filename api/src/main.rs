mod apps;
mod auth;
mod config;
mod services;
mod state;

use rocket::{get, options, routes};
use rocket::http::Status;
use rocket::State;

use config::{load_config, parse_host, router_base_url, defaults, github_configured};
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
        println!("GitHub auth: enabled (redirect_uri={})", config.github.redirect_uri);
    } else {
        println!("GitHub auth: DISABLED - set github.client_id / github.client_secret in config.json");
    }

    let port = config.port.unwrap_or_else(defaults::port);
    let address = parse_host(config.host.clone());
    let router_url = router_base_url(&config.router_address);
    println!("Router address: {router_url}");
    println!("S3 bucket: {} ({})", config.s3.bucket, config.s3.region);
    println!("Starting server: http://localhost:{port}");

    let figment = rocket::Config::figment()
        .merge(("address", address))
        .merge(("port", port));
    let frontend_url = config.github.frontend_url.clone();
    let state = ApiState {
        config,
        redis: redis_client,
        http: reqwest::Client::new(),
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
                services::create_service,
                services::update_service,
                services::delete_service,
            ],
        )
}
