use redis::AsyncCommands;
use rocket::http::Status;
use rocket::serde::{Deserialize, Serialize};
use rocket::{State, delete, get, patch, post};

use crate::auth::{SessionToken, require_user};
use crate::state::{ApiState, conn, json_error};

pub fn key_service(service_id: &str) -> String {
    format!("service:{service_id}")
}

pub fn key_app_services(app_id: &str) -> String {
    format!("app:{app_id}:services")
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "rocket::serde")]
pub struct Service {
    pub id: String,
    pub app_id: String,
    pub owner_id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, Debug)]
#[serde(crate = "rocket::serde")]
pub struct CreateServiceBody {
    pub name: Option<String>,
    pub image: Option<String>,
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

pub fn validate_image(image: &Option<String>) -> Result<Option<String>, String> {
    match image {
        None => Ok(None),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.len() > 256 {
                return Err("image must be at most 256 characters".into());
            }
            Ok(Some(trimmed.to_owned()))
        }
    }
}

async fn load_owned_service(
    state: &State<ApiState>,
    github_id: u64,
    app_id: &str,
    service_id: &str,
) -> Result<Service, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    if service_id.trim().is_empty() || service_id.len() > 128 {
        return Err(json_error(Status::BadRequest, "invalid service id"));
    }
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .get(key_service(service_id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "service not found"))?;
    let svc: Service = serde_json::from_str(&raw)
        .map_err(|_| json_error(Status::InternalServerError, "stored service is corrupt"))?;
    if svc.owner_id != github_id || svc.app_id != app_id {
        return Err(json_error(Status::NotFound, "service not found"));
    }
    Ok(svc)
}

async fn ensure_owned_app(
    state: &State<ApiState>,
    github_id: u64,
    app_id: &str,
) -> Result<(), (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .get(crate::apps::key_app(app_id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "app not found"))?;
    let app: crate::apps::App = serde_json::from_str(&raw)
        .map_err(|_| json_error(Status::InternalServerError, "stored app is corrupt"))?;
    if app.owner_id != github_id {
        return Err(json_error(Status::NotFound, "app not found"));
    }
    Ok(())
}

#[get("/apps/<app_id>/services")]
pub async fn list_services(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let mut redis = conn(state).await?;
    let ids: Vec<String> = redis
        .smembers(key_app_services(app_id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let mut services: Vec<Service> = Vec::with_capacity(ids.len());
    for id in ids {
        let raw: Option<String> = redis
            .get(key_service(&id))
            .await
            .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
        if let Some(raw) = raw {
            if let Ok(svc) = serde_json::from_str::<Service>(&raw) {
                if svc.owner_id == user.github_id && svc.app_id == app_id {
                    services.push(svc);
                }
            }
        }
    }
    services.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(rocket::serde::json::Json(serde_json::json!({ "services": services })))
}

#[post("/apps/<app_id>/services", data = "<body>")]
pub async fn create_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<(Status, rocket::serde::json::Json<serde_json::Value>), (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    crate::apps::reject_id_change(&body)?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let parsed: CreateServiceBody = serde_json::from_value(body.into_inner())
        .map_err(|e| json_error(Status::BadRequest, format!("invalid body: {e}")))?;
    let name = validate_service_name(parsed.name.as_deref().unwrap_or(""))
        .map_err(|e| json_error(Status::BadRequest, e))?;
    let image =
        validate_image(&parsed.image).map_err(|e| json_error(Status::BadRequest, e))?;

    let now = chrono::Utc::now().to_rfc3339();
    let svc = Service {
        id: uuid::Uuid::new_v4().to_string(),
        app_id: app_id.to_owned(),
        owner_id: user.github_id,
        name,
        image,
        created_at: now.clone(),
        updated_at: now,
    };
    let raw = serde_json::to_string(&svc)
        .map_err(|e| json_error(Status::InternalServerError, format!("encode service failed: {e}")))?;
    let mut redis = conn(state).await?;
    redis
        .set::<_, _, ()>(key_service(&svc.id), raw)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    redis
        .sadd::<_, _, ()>(key_app_services(app_id), svc.id.clone())
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok((
        Status::Created,
        rocket::serde::json::Json(serde_json::json!({ "service": svc })),
    ))
}

#[patch("/apps/<app_id>/services/<service_id>", data = "<body>")]
pub async fn update_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    crate::apps::reject_id_change(&body)?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let mut svc = load_owned_service(state, user.github_id, app_id, service_id).await?;
    let obj = body.into_inner();

    let name = obj.get("name").and_then(|v| v.as_str());
    let image_present = obj.get("image").is_some();
    if name.is_none() && !image_present {
        return Err(json_error(Status::BadRequest, "nothing to update (want name, image)"));
    }
    if let Some(name) = name {
        svc.name = validate_service_name(name).map_err(|e| json_error(Status::BadRequest, e))?;
    }
    if image_present {
        let image_value = obj.get("image");
        let parsed: Option<String> = match image_value {
            Some(v) if v.is_null() => None,
            Some(v) => match v.as_str() {
                Some(s) => Some(s.to_owned()),
                None => return Err(json_error(Status::BadRequest, "image must be a string or null")),
            },
            None => None,
        };
        let as_opt = |s: Option<String>| -> Option<String> { s };
        let _ = as_opt;
        // parsed=None means explicit null or missing; empty string also clears via validate_image
        let input: Option<String> = parsed;
        svc.image = validate_image(&input).map_err(|e| json_error(Status::BadRequest, e))?;
    }
    svc.updated_at = chrono::Utc::now().to_rfc3339();
    let raw = serde_json::to_string(&svc)
        .map_err(|e| json_error(Status::InternalServerError, format!("encode service failed: {e}")))?;
    let mut redis = conn(state).await?;
    redis
        .set::<_, _, ()>(key_service(&svc.id), raw)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok(rocket::serde::json::Json(serde_json::json!({ "service": svc })))
}

#[delete("/apps/<app_id>/services/<service_id>")]
pub async fn delete_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    ensure_owned_app(state, user.github_id, app_id).await?;
    let svc = load_owned_service(state, user.github_id, app_id, service_id).await?;
    let mut redis = conn(state).await?;
    let _: () = redis
        .del(key_service(&svc.id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let _: () = redis
        .srem(key_app_services(app_id), svc.id.clone())
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok(rocket::serde::json::Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_keys_are_namespaced() {
        assert_eq!(key_service("s1"), "service:s1");
        assert_eq!(key_app_services("a1"), "app:a1:services");
    }

    #[test]
    fn validates_service_fields() {
        assert!(validate_service_name("").is_err());
        assert!(validate_service_name("api").is_ok());
        assert!(validate_image(&None).unwrap().is_none());
        assert!(validate_image(&Some("nginx:latest".into())).unwrap().is_some());
        assert!(validate_image(&Some("x".repeat(300))).is_err());
    }
}
