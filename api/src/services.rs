use redis::AsyncCommands;
use rocket::http::Status;
use rocket::serde::{Deserialize, Serialize};
use rocket::{State, delete, get, patch, post};

use crate::auth::{SessionToken, require_user};
use crate::state::{ApiState, conn, json_error};

pub type ApiError = (Status, rocket::serde::json::Json<serde_json::Value>);
pub type ApiJson = rocket::serde::json::Json<serde_json::Value>;

pub fn api_json(value: serde_json::Value) -> ApiJson {
    rocket::serde::json::Json(value)
}

pub const DEFAULT_CONTAINER_PORT: u16 = 3000;
pub const MAX_ARGV_ITEMS: usize = 64;
pub const MAX_ARG_LENGTH: usize = 4096;

pub fn key_user_app_services(github_id: u64, app_id: &str) -> String {
    format!("user:{github_id}:app:{app_id}:services")
}

fn default_container_port() -> u16 {
    DEFAULT_CONTAINER_PORT
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "rocket::serde")]
pub struct Service {
    pub id: String,
    pub app_id: String,
    #[serde(alias = "owner_id")]
    pub owner: u64,
    pub name: String,
    #[serde(default)]
    pub position_x: i32,
    #[serde(default)]
    pub position_y: i32,
    #[serde(default = "default_container_port")]
    pub container_port: u16,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_deployment_id: Option<String>,
    #[serde(default)]
    pub package_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, Debug)]
#[serde(crate = "rocket::serde")]
pub struct CreateServiceBody {
    pub name: Option<String>,
    #[serde(default)]
    pub position_x: i32,
    #[serde(default)]
    pub position_y: i32,
    #[serde(default = "default_container_port")]
    pub container_port: u16,
    #[serde(default)]
    pub argv: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(crate = "rocket::serde")]
struct PositionBody {
    position_x: i32,
    position_y: i32,
}

pub fn validate_service_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("service name is required".into());
    }
    if trimmed.chars().count() > 64 {
        return Err("service name must be at most 64 characters".into());
    }
    Ok(trimmed.to_owned())
}

pub fn validate_container_port(port: u16) -> Result<u16, String> {
    if port == 0 {
        Err("container_port must be between 1 and 65535".into())
    } else {
        Ok(port)
    }
}

pub fn validate_argv(argv: Vec<String>) -> Result<Vec<String>, String> {
    if argv.len() > MAX_ARGV_ITEMS {
        return Err(format!("argv must contain at most {MAX_ARGV_ITEMS} items"));
    }
    if argv.iter().any(|item| item.as_bytes().contains(&0)) {
        return Err("argv may not contain NUL bytes".into());
    }
    if argv.iter().any(|item| item.len() > MAX_ARG_LENGTH) {
        return Err(format!(
            "each argv item must be at most {MAX_ARG_LENGTH} bytes"
        ));
    }
    Ok(argv)
}

pub(crate) async fn ensure_owned_app(
    state: &ApiState,
    github_id: u64,
    app_id: &str,
) -> Result<(), ApiError> {
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .hget(crate::apps::key_user_apps(github_id), app_id)
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "app not found"))?;
    let app: crate::apps::App = serde_json::from_str(&raw)
        .map_err(|_| json_error(Status::InternalServerError, "stored app is corrupt"))?;
    if app.owner_id != github_id {
        return Err(json_error(Status::NotFound, "app not found"));
    }
    Ok(())
}

pub(crate) async fn load_owned_service(
    state: &ApiState,
    github_id: u64,
    app_id: &str,
    service_id: &str,
) -> Result<Service, ApiError> {
    if service_id.trim().is_empty() || service_id.len() > 128 {
        return Err(json_error(Status::BadRequest, "invalid service id"));
    }
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .hget(key_user_app_services(github_id, app_id), service_id)
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "service not found"))?;
    let service: Service = serde_json::from_str(&raw)
        .map_err(|_| json_error(Status::InternalServerError, "stored service is corrupt"))?;
    if service.owner != github_id || service.app_id != app_id {
        return Err(json_error(Status::NotFound, "service not found"));
    }
    Ok(service)
}

pub(crate) async fn service_response_value(
    state: &ApiState,
    service: &Service,
) -> Result<serde_json::Value, ApiError> {
    let active_deployment = crate::deployments::load_active_deployment(state, service).await?;
    let mut value = serde_json::to_value(service).map_err(|e| {
        json_error(
            Status::InternalServerError,
            format!("encode service response failed: {e}"),
        )
    })?;
    value
        .as_object_mut()
        .expect("serialized Service is an object")
        .insert(
            "active_deployment".into(),
            serde_json::to_value(active_deployment).expect("Deployment serializes"),
        );
    Ok(value)
}

pub(crate) async fn save_service(state: &ApiState, service: &Service) -> Result<(), ApiError> {
    let raw = serde_json::to_string(service).map_err(|e| {
        json_error(
            Status::InternalServerError,
            format!("encode service failed: {e}"),
        )
    })?;
    let mut redis = conn(state).await?;
    redis
        .hset::<_, _, _, ()>(
            key_user_app_services(service.owner, &service.app_id),
            service.id.clone(),
            raw,
        )
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })
}

pub(crate) async fn cleanup_service(
    state: &ApiState,
    mut service: Service,
) -> Result<(), ApiError> {
    if let Some(deployment_id) = service.active_deployment_id.clone() {
        crate::deployments::stop_runtime(state, &mut service, &deployment_id).await?;
    }

    crate::packages::delete_available_blobs(state, &service).await?;

    let mut redis = conn(state).await?;
    let _: () = redis
        .del(crate::packages::key_service_packages(
            service.owner,
            &service.app_id,
            &service.id,
        ))
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let _: () = redis
        .del(crate::deployments::key_service_deployments(
            service.owner,
            &service.app_id,
            &service.id,
        ))
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let _: () = redis
        .hdel(
            key_user_app_services(service.owner, &service.app_id),
            &service.id,
        )
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    Ok(())
}

#[get("/apps/<app_id>/services")]
pub async fn list_services(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let mut redis = conn(state).await?;
    let map: std::collections::HashMap<String, String> = redis
        .hgetall(key_user_app_services(user.github_id, app_id))
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let mut services = Vec::with_capacity(map.len());
    for raw in map.into_values() {
        if let Ok(service) = serde_json::from_str::<Service>(&raw) {
            if service.owner == user.github_id && service.app_id == app_id {
                services.push(service);
            }
        }
    }
    services.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let mut response_services = Vec::with_capacity(services.len());
    for service in services {
        response_services.push(service_response_value(state, &service).await?);
    }
    Ok(api_json(
        serde_json::json!({ "services": response_services }),
    ))
}

#[get("/apps/<app_id>/services/<service_id>")]
pub async fn get_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let service = load_owned_service(state, user.github_id, app_id, service_id).await?;
    let service = service_response_value(state, &service).await?;
    Ok(api_json(serde_json::json!({ "service": service })))
}

#[post("/apps/<app_id>/services", data = "<body>")]
pub async fn create_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<(Status, ApiJson), ApiError> {
    let user = require_user(state, &token).await?;
    crate::apps::reject_id_change(&body)?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let parsed: CreateServiceBody = serde_json::from_value(body.into_inner())
        .map_err(|e| json_error(Status::BadRequest, format!("invalid body: {e}")))?;
    let name = validate_service_name(parsed.name.as_deref().unwrap_or(""))
        .map_err(|e| json_error(Status::BadRequest, e))?;
    let container_port = validate_container_port(parsed.container_port)
        .map_err(|e| json_error(Status::BadRequest, e))?;
    let argv = validate_argv(parsed.argv).map_err(|e| json_error(Status::BadRequest, e))?;

    let now = chrono::Utc::now().to_rfc3339();
    let service = Service {
        id: uuid::Uuid::new_v4().to_string(),
        app_id: app_id.to_owned(),
        owner: user.github_id,
        name,
        position_x: parsed.position_x,
        position_y: parsed.position_y,
        container_port,
        argv,
        active_package_id: None,
        active_deployment_id: None,
        package_ids: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    };
    save_service(state, &service).await?;
    Ok((
        Status::Created,
        api_json(serde_json::json!({ "service": service })),
    ))
}

#[patch("/apps/<app_id>/services/<service_id>", data = "<body>")]
pub async fn update_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::apps::reject_id_change(&body)?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let object = body
        .into_inner()
        .as_object()
        .cloned()
        .ok_or_else(|| json_error(Status::BadRequest, "body must be an object"))?;

    if !object.contains_key("name")
        && !object.contains_key("container_port")
        && !object.contains_key("argv")
        && !object.contains_key("active_package_id")
    {
        return Err(json_error(
            Status::BadRequest,
            "nothing to update (want name, container_port, argv, active_package_id)",
        ));
    }

    let name = object
        .get("name")
        .map(|value| {
            let name = value
                .as_str()
                .ok_or_else(|| json_error(Status::BadRequest, "name must be a string"))?;
            validate_service_name(name).map_err(|e| json_error(Status::BadRequest, e))
        })
        .transpose()?;
    let container_port = object
        .get("container_port")
        .map(|value| {
            let port: u16 = serde_json::from_value(value.clone()).map_err(|_| {
                json_error(
                    Status::BadRequest,
                    "container_port must be between 1 and 65535",
                )
            })?;
            validate_container_port(port).map_err(|e| json_error(Status::BadRequest, e))
        })
        .transpose()?;
    let argv = object
        .get("argv")
        .map(|value| {
            let argv: Vec<String> = serde_json::from_value(value.clone())
                .map_err(|_| json_error(Status::BadRequest, "argv must be an array of strings"))?;
            validate_argv(argv).map_err(|e| json_error(Status::BadRequest, e))
        })
        .transpose()?;
    let clear_active_package = match object.get("active_package_id") {
        Some(value) if value.is_null() => true,
        Some(_) => {
            return Err(json_error(
                Status::BadRequest,
                "active_package_id can only be cleared with null; use the package activation endpoint to select a package",
            ));
        }
        None => false,
    };

    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service = load_owned_service(state, user.github_id, app_id, service_id).await?;
    if clear_active_package {
        if let Some(deployment_id) = service.active_deployment_id.clone() {
            crate::deployments::stop_runtime(state, &mut service, &deployment_id).await?;
        }
        service.active_package_id = None;
    }
    if let Some(name) = name {
        service.name = name;
    }
    if let Some(container_port) = container_port {
        service.container_port = container_port;
    }
    if let Some(argv) = argv {
        service.argv = argv;
    }
    service.updated_at = chrono::Utc::now().to_rfc3339();
    save_service(state, &service).await?;
    Ok(api_json(serde_json::json!({ "service": service })))
}

#[patch("/apps/<app_id>/services/<service_id>/position", data = "<body>")]
pub async fn update_service_position(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::apps::reject_id_change(&body)?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let position: PositionBody = serde_json::from_value(body.into_inner())
        .map_err(|e| json_error(Status::BadRequest, format!("invalid position: {e}")))?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service = load_owned_service(state, user.github_id, app_id, service_id).await?;
    service.position_x = position.position_x;
    service.position_y = position.position_y;
    service.updated_at = chrono::Utc::now().to_rfc3339();
    save_service(state, &service).await?;
    Ok(api_json(serde_json::json!({ "service": service })))
}

#[delete("/apps/<app_id>/services/<service_id>")]
pub async fn delete_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let service = load_owned_service(state, user.github_id, app_id, service_id).await?;
    cleanup_service(state, service).await?;
    Ok(api_json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_keys_are_namespaced() {
        assert_eq!(key_user_app_services(7, "a1"), "user:7:app:a1:services");
    }

    #[test]
    fn old_service_records_receive_runtime_defaults() {
        let service: Service = serde_json::from_value(serde_json::json!({
            "id": "s1",
            "app_id": "a1",
            "owner_id": 7,
            "name": "api",
            "image": "ignored:legacy",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(service.owner, 7);
        assert_eq!(service.position_x, 0);
        assert_eq!(service.position_y, 0);
        assert_eq!(service.container_port, DEFAULT_CONTAINER_PORT);
        assert!(service.argv.is_empty());
        assert!(service.package_ids.is_empty());
        assert!(service.active_package_id.is_none());
    }

    #[test]
    fn validates_runtime_fields() {
        assert!(validate_service_name("").is_err());
        assert_eq!(validate_service_name(" api ").unwrap(), "api");
        assert!(validate_container_port(0).is_err());
        assert_eq!(validate_container_port(8080).unwrap(), 8080);
        assert!(validate_argv(vec!["x".repeat(MAX_ARG_LENGTH + 1)]).is_err());
        assert!(validate_argv(vec!["--verbose".into()]).is_ok());
    }
}
