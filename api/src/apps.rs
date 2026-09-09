use redis::AsyncCommands;
use rocket::http::Status;
use rocket::serde::{Deserialize, Serialize};
use rocket::{State, delete, get, patch, post};

use crate::auth::{SessionToken, require_user};
use crate::state::{ApiState, conn, json_error};

pub fn key_app(app_id: &str) -> String {
    format!("app:{app_id}")
}

pub fn key_user_apps(github_id: u64) -> String {
    format!("user:{github_id}:apps")
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "rocket::serde")]
pub struct App {
    pub id: String,
    pub owner_id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_kind: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, Debug)]
#[serde(crate = "rocket::serde")]
pub struct CreateAppBody {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_kind: Option<String>,
}

pub fn validate_app_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name is required".into());
    }
    if trimmed.chars().count() > 64 {
        return Err("name must be at most 64 characters".into());
    }
    Ok(trimmed.to_owned())
}

pub fn validate_icon(icon: &Option<String>, icon_kind: &Option<String>) -> Result<(Option<String>, Option<String>), String> {
    let kind = icon_kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());
    let value = icon
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());

    match (&kind, &value) {
        (None, None) => Ok((None, None)),
        (None, Some(_)) => Err("icon_kind is required when icon is set (emoji | icon | upload)".into()),
        (Some(_), None) => Err("icon is required when icon_kind is set".into()),
        (Some(k), Some(v)) => {
            match k.as_str() {
                "emoji" => {
                    if v.chars().count() > 16 {
                        return Err("emoji icon is too long".into());
                    }
                    Ok((Some(v.clone()), Some(k.clone())))
                }
                "icon" => {
                    if v.len() > 64 {
                        return Err("icon name is too long".into());
                    }
                    if !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                        return Err("icon name may only contain letters, numbers, - and _".into());
                    }
                    Ok((Some(v.clone()), Some(k.clone())))
                }
                "upload" => {
                    if v.len() > 1_000_000 {
                        return Err("uploaded icon is too large".into());
                    }
                    Ok((Some(v.clone()), Some(k.clone())))
                }
                _ => Err("icon_kind must be one of: emoji, icon, upload".into()),
            }
        }
    }
}

pub fn reject_id_change(raw: &serde_json::Value) -> Result<(), (Status, rocket::serde::json::Json<serde_json::Value>)> {
    if let Some(obj) = raw.as_object() {
        for forbidden in ["id", "uuid", "owner_id", "ownerId", "app_id", "appId"] {
            if obj.contains_key(forbidden) {
                return Err(json_error(
                    Status::BadRequest,
                    "changing id is not allowed",
                ));
            }
        }
    }
    Ok(())
}

async fn load_owned_app(
    state: &State<ApiState>,
    github_id: u64,
    app_id: &str,
) -> Result<App, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    if app_id.trim().is_empty() || app_id.len() > 128 {
        return Err(json_error(Status::BadRequest, "invalid app id"));
    }
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .get(key_app(app_id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "app not found"))?;
    let app: App =
        serde_json::from_str(&raw).map_err(|_| json_error(Status::InternalServerError, "stored app is corrupt"))?;
    if app.owner_id != github_id {
        return Err(json_error(Status::NotFound, "app not found"));
    }
    Ok(app)
}

#[get("/apps")]
pub async fn list_apps(
    state: &State<ApiState>,
    token: SessionToken,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    let mut redis = conn(state).await?;
    let ids: Vec<String> = redis
        .smembers(key_user_apps(user.github_id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let mut apps: Vec<App> = Vec::with_capacity(ids.len());
    for id in ids {
        let raw: Option<String> = redis
            .get(key_app(&id))
            .await
            .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
        if let Some(raw) = raw {
            if let Ok(app) = serde_json::from_str::<App>(&raw) {
                if app.owner_id == user.github_id {
                    apps.push(app);
                }
            }
        }
    }
    apps.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(rocket::serde::json::Json(serde_json::json!({ "apps": apps })))
}

#[post("/apps", data = "<body>")]
pub async fn create_app(
    state: &State<ApiState>,
    token: SessionToken,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<(Status, rocket::serde::json::Json<serde_json::Value>), (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    reject_id_change(&body)?;
    let parsed: CreateAppBody = serde_json::from_value(body.into_inner())
        .map_err(|e| json_error(Status::BadRequest, format!("invalid body: {e}")))?;
    let name = validate_app_name(parsed.name.as_deref().unwrap_or(""))
        .map_err(|e| json_error(Status::BadRequest, e))?;
    let (icon, icon_kind) = validate_icon(&parsed.icon, &parsed.icon_kind)
        .map_err(|e| json_error(Status::BadRequest, e))?;

    let now = chrono::Utc::now().to_rfc3339();
    let app = App {
        id: uuid::Uuid::new_v4().to_string(),
        owner_id: user.github_id,
        name,
        icon,
        icon_kind,
        created_at: now.clone(),
        updated_at: now,
    };
    let raw = serde_json::to_string(&app)
        .map_err(|e| json_error(Status::InternalServerError, format!("encode app failed: {e}")))?;
    let mut redis = conn(state).await?;
    redis
        .set::<_, _, ()>(key_app(&app.id), raw)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    redis
        .sadd::<_, _, ()>(key_user_apps(user.github_id), app.id.clone())
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok((Status::Created, rocket::serde::json::Json(serde_json::json!({ "app": app }))))
}

#[get("/apps/<id>")]
pub async fn get_app(
    state: &State<ApiState>,
    token: SessionToken,
    id: &str,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    let app = load_owned_app(state, user.github_id, id).await?;
    Ok(rocket::serde::json::Json(serde_json::json!({ "app": app })))
}

#[patch("/apps/<id>", data = "<body>")]
pub async fn update_app(
    state: &State<ApiState>,
    token: SessionToken,
    id: &str,
    body: rocket::serde::json::Json<serde_json::Value>,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    reject_id_change(&body)?;
    let mut app = load_owned_app(state, user.github_id, id).await?;

    let obj = body.into_inner();
    let name = obj.get("name").and_then(|v| v.as_str());
    let icon_present = obj.get("icon").is_some();
    let icon_kind_present = obj.get("icon_kind").is_some();

    if name.is_none() && !icon_present && !icon_kind_present {
        return Err(json_error(Status::BadRequest, "nothing to update (want name, icon, icon_kind)"));
    }

    if let Some(name) = name {
        app.name = validate_app_name(name).map_err(|e| json_error(Status::BadRequest, e))?;
    }

    if icon_present || icon_kind_present {
        let icon = obj.get("icon").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_owned())
            }
        });
        if obj.get("icon").is_some_and(|v| !v.is_null() && v.as_str().is_none()) {
            return Err(json_error(Status::BadRequest, "icon must be a string or null"));
        }
        let icon_kind = obj.get("icon_kind").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_owned())
            }
        });
        if obj.get("icon_kind").is_some_and(|v| !v.is_null() && v.as_str().is_none()) {
            return Err(json_error(Status::BadRequest, "icon_kind must be a string or null"));
        }
        let (icon, icon_kind) =
            validate_icon(&icon, &icon_kind).map_err(|e| json_error(Status::BadRequest, e))?;
        app.icon = icon;
        app.icon_kind = icon_kind;
    }

    app.updated_at = chrono::Utc::now().to_rfc3339();
    let raw = serde_json::to_string(&app)
        .map_err(|e| json_error(Status::InternalServerError, format!("encode app failed: {e}")))?;
    let mut redis = conn(state).await?;
    redis
        .set::<_, _, ()>(key_app(&app.id), raw)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok(rocket::serde::json::Json(serde_json::json!({ "app": app })))
}

#[delete("/apps/<id>")]
pub async fn delete_app(
    state: &State<ApiState>,
    token: SessionToken,
    id: &str,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let user = require_user(state, &token).await?;
    let app = load_owned_app(state, user.github_id, id).await?;
    let mut redis = conn(state).await?;

    let service_ids: Vec<String> = redis
        .smembers(crate::services::key_app_services(&app.id))
        .await
        .unwrap_or_default();
    for sid in service_ids {
        let _: () = redis.del(crate::services::key_service(&sid)).await.unwrap_or(());
    }
    let _: () = redis
        .del(crate::services::key_app_services(&app.id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let _: () = redis
        .del(key_app(&app.id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let _: () = redis
        .srem(key_user_apps(user.github_id), app.id.clone())
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok(rocket::serde::json::Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_keys_are_namespaced() {
        assert_eq!(key_app("abc"), "app:abc");
        assert_eq!(key_user_apps(7), "user:7:apps");
    }

    #[test]
    fn rejects_blank_and_long_names() {
        assert!(validate_app_name("").is_err());
        assert!(validate_app_name("   ").is_err());
        assert!(validate_app_name("My app").unwrap() == "My app");
        assert!(validate_app_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn validates_icons_by_kind() {
        assert!(validate_icon(&None, &None).unwrap() == (None, None));
        assert!(validate_icon(&Some("😀".into()), &None).is_err());
        assert!(validate_icon(&None, &Some("emoji".into())).is_err());
        assert!(validate_icon(&Some("😀".into()), &Some("emoji".into())).is_ok());
        assert!(validate_icon(&Some("box".into()), &Some("icon".into())).is_ok());
        assert!(validate_icon(&Some("not an icon!!".into()), &Some("icon".into())).is_err());
        assert!(validate_icon(&Some("x".into()), &Some("nope".into())).is_err());
    }

    #[test]
    fn forbids_changing_id_uuid_owner() {
        for key in ["id", "uuid", "owner_id", "app_id"] {
            let v = serde_json::json!({ key: "evil" });
            assert!(reject_id_change(&v).is_err(), "{key} should be rejected");
        }
        let ok = serde_json::json!({ "name": "new name" });
        assert!(reject_id_change(&ok).is_ok());
    }
}
