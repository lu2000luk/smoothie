use redis::AsyncCommands;
use rocket::http::Status;
use rocket::serde::{Deserialize, Serialize};
use rocket::{State, get, post};

use crate::auth::{SessionToken, require_user};
use crate::packages::Package;
use crate::services::{ApiError, ApiJson, Service, api_json};
use crate::state::{ApiState, conn, json_error};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "rocket::serde")]
pub struct Deployment {
    pub id: String,
    pub service_id: String,
    pub package_id: String,
    pub app_id: String,
    pub owner: u64,
    pub argv: Vec<String>,
    pub port: u16,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
struct RouterDeployRequest<'a> {
    service_id: &'a str,
    package_id: &'a str,
    argv: &'a [String],
    port: u16,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(crate = "rocket::serde")]
struct RouterDeployment {
    status: String,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    host_port: Option<u16>,
    #[serde(default)]
    container_port: Option<u16>,
    #[serde(default)]
    container_id: Option<String>,
}

pub fn key_service_deployments(owner: u64, app_id: &str, service_id: &str) -> String {
    format!("user:{owner}:app:{app_id}:service:{service_id}:deployments")
}

fn router_deployment_url(state: &ApiState, deployment_id: &str) -> String {
    format!(
        "{}/deployments/{deployment_id}",
        crate::config::router_base_url(&state.config.router_address)
    )
}

async fn save_deployment(state: &ApiState, deployment: &Deployment) -> Result<(), ApiError> {
    let raw = serde_json::to_string(deployment).map_err(|e| {
        json_error(
            Status::InternalServerError,
            format!("encode deployment failed: {e}"),
        )
    })?;
    let mut redis = conn(state).await?;
    redis
        .hset::<_, _, _, ()>(
            key_service_deployments(deployment.owner, &deployment.app_id, &deployment.service_id),
            &deployment.id,
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

pub(crate) async fn load_deployment(
    state: &ApiState,
    service: &Service,
    deployment_id: &str,
) -> Result<Deployment, ApiError> {
    let mut redis = conn(state).await?;
    let raw: Option<String> = redis
        .hget(
            key_service_deployments(service.owner, &service.app_id, &service.id),
            deployment_id,
        )
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })?;
    let raw = raw.ok_or_else(|| json_error(Status::NotFound, "deployment not found"))?;
    let deployment: Deployment = serde_json::from_str(&raw)
        .map_err(|_| json_error(Status::InternalServerError, "stored deployment is corrupt"))?;
    if deployment.owner != service.owner
        || deployment.app_id != service.app_id
        || deployment.service_id != service.id
    {
        return Err(json_error(Status::NotFound, "deployment not found"));
    }
    Ok(deployment)
}

pub(crate) async fn load_active_deployment(
    state: &ApiState,
    service: &Service,
) -> Result<Option<Deployment>, ApiError> {
    match service.active_deployment_id.as_deref() {
        Some(deployment_id) => load_deployment(state, service, deployment_id)
            .await
            .map(Some),
        None => Ok(None),
    }
}

pub fn sort_deployments_newest_first(deployments: &mut [Deployment]) {
    deployments.sort_by(|a, b| b.created_at.cmp(&a.created_at));
}

pub fn runtime_argv(service_argv: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(service_argv.len() + 1);
    argv.push("./main".into());
    argv.extend(service_argv.iter().cloned());
    argv
}

fn apply_router_deployment(deployment: &mut Deployment, router: RouterDeployment) {
    deployment.status = router.status;
    deployment.host = router.host;
    deployment.host_port = router.host_port;
    deployment.container_port = router.container_port;
    deployment.container_id = router.container_id;
    deployment.error = None;
    deployment.updated_at = chrono::Utc::now().to_rfc3339();
}

async fn router_deploy(
    state: &ApiState,
    deployment: &Deployment,
) -> Result<RouterDeployment, String> {
    let response = state
        .http
        .post(router_deployment_url(state, &deployment.id))
        .json(&RouterDeployRequest {
            service_id: &deployment.service_id,
            package_id: &deployment.package_id,
            argv: &deployment.argv,
            port: deployment.port,
        })
        .send()
        .await
        .map_err(|e| format!("router request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!("router returned {status}: {detail}"));
    }
    response
        .json::<RouterDeployment>()
        .await
        .map_err(|e| format!("invalid router deployment response: {e}"))
}

async fn run_deployment(
    state: &ApiState,
    deployment: &mut Deployment,
) -> Result<Option<String>, ApiError> {
    match router_deploy(state, deployment).await {
        Ok(router) => apply_router_deployment(deployment, router),
        Err(error) => {
            deployment.status = "failed".into();
            deployment.error = Some(error.clone());
            deployment.updated_at = chrono::Utc::now().to_rfc3339();
            save_deployment(state, deployment).await?;
            return Ok(Some(error));
        }
    }
    save_deployment(state, deployment).await?;
    Ok(None)
}

pub(crate) async fn stop_runtime(
    state: &ApiState,
    service: &mut Service,
    deployment_id: &str,
) -> Result<Deployment, ApiError> {
    let mut deployment = load_deployment(state, service, deployment_id).await?;
    let response = state
        .http
        .delete(router_deployment_url(state, deployment_id))
        .send()
        .await
        .map_err(|e| {
            json_error(
                Status::BadGateway,
                format!("router stop request failed: {e}"),
            )
        })?;
    if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(json_error(
            Status::BadGateway,
            format!("router stop returned {status}: {detail}"),
        ));
    }

    deployment.status = "stopped".into();
    deployment.error = None;
    deployment.updated_at = chrono::Utc::now().to_rfc3339();
    if service.active_deployment_id.as_deref() == Some(deployment_id) {
        service.active_deployment_id = None;
        service.updated_at = chrono::Utc::now().to_rfc3339();
    }
    save_deployment(state, &deployment).await?;
    crate::services::save_service(state, service).await?;
    Ok(deployment)
}

pub(crate) async fn activate_package(
    state: &ApiState,
    service: &mut Service,
    package: &Package,
) -> Result<(Deployment, Option<String>), ApiError> {
    if let Some(active_id) = service.active_deployment_id.clone() {
        stop_runtime(state, service, &active_id).await?;
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut deployment = Deployment {
        id: uuid::Uuid::new_v4().to_string(),
        service_id: service.id.clone(),
        package_id: package.id.clone(),
        app_id: service.app_id.clone(),
        owner: service.owner,
        argv: runtime_argv(&service.argv),
        port: service.container_port,
        status: "deploying".into(),
        host: None,
        host_port: None,
        container_port: None,
        container_id: None,
        error: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    service.active_package_id = Some(package.id.clone());
    service.active_deployment_id = Some(deployment.id.clone());
    service.updated_at = now;
    save_deployment(state, &deployment).await?;
    crate::services::save_service(state, service).await?;
    let error = run_deployment(state, &mut deployment).await?;
    Ok((deployment, error))
}

fn action_response(
    status: Status,
    service: Service,
    deployment: Deployment,
    error: Option<String>,
) -> (Status, ApiJson) {
    let mut value = serde_json::json!({ "service": service, "deployment": deployment });
    if let Some(error) = error {
        value["error"] = serde_json::Value::String(error);
    }
    (status, api_json(value))
}

#[get("/apps/<app_id>/services/<service_id>/deployments")]
pub async fn list_service_deployments(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let mut redis = conn(state).await?;
    let map: std::collections::HashMap<String, String> = redis
        .hgetall(key_service_deployments(
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
    let mut deployments = Vec::with_capacity(map.len());
    for raw in map.into_values() {
        let deployment: Deployment = serde_json::from_str(&raw)
            .map_err(|_| json_error(Status::InternalServerError, "stored deployment is corrupt"))?;
        if deployment.owner == service.owner
            && deployment.app_id == service.app_id
            && deployment.service_id == service.id
        {
            deployments.push(deployment);
        }
    }
    sort_deployments_newest_first(&mut deployments);
    Ok(api_json(serde_json::json!({ "deployments": deployments })))
}

#[get("/apps/<app_id>/services/<service_id>/deployments/<deployment_id>")]
pub async fn get_deployment_status(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    deployment_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let mut deployment = load_deployment(state, &service, deployment_id).await?;
    let response = state
        .http
        .get(router_deployment_url(state, deployment_id))
        .send()
        .await
        .map_err(|e| {
            json_error(
                Status::BadGateway,
                format!("router status request failed: {e}"),
            )
        })?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        deployment.status = "not_found".into();
        deployment.updated_at = chrono::Utc::now().to_rfc3339();
    } else if response.status().is_success() {
        let router: RouterDeployment = response.json().await.map_err(|e| {
            json_error(
                Status::BadGateway,
                format!("invalid router status response: {e}"),
            )
        })?;
        apply_router_deployment(&mut deployment, router);
    } else {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(json_error(
            Status::BadGateway,
            format!("router status returned {status}: {detail}"),
        ));
    }
    save_deployment(state, &deployment).await?;
    Ok(api_json(serde_json::json!({ "deployment": deployment })))
}

#[post("/apps/<app_id>/services/<service_id>/deployments/start")]
pub async fn start_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<(Status, ApiJson), ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let package_id = service
        .active_package_id
        .clone()
        .ok_or_else(|| json_error(Status::Conflict, "service has no active package"))?;
    let package = crate::packages::load_package(state, &service, &package_id).await?;
    if package.blob_status != crate::packages::BlobStatus::Available {
        return Err(json_error(
            Status::Conflict,
            "active package blob is not available",
        ));
    }
    let (deployment, error) = activate_package(state, &mut service, &package).await?;
    Ok(action_response(Status::Created, service, deployment, error))
}

#[post("/apps/<app_id>/services/<service_id>/deployments/<deployment_id>/retry")]
pub async fn retry_deployment(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
    deployment_id: &str,
) -> Result<(Status, ApiJson), ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let mut deployment = load_deployment(state, &service, deployment_id).await?;
    let package = crate::packages::load_package(state, &service, &deployment.package_id).await?;
    if package.blob_status != crate::packages::BlobStatus::Available {
        return Err(json_error(
            Status::Conflict,
            "deployment package blob is not available",
        ));
    }
    if let Some(active_id) = service.active_deployment_id.clone() {
        if active_id != deployment_id {
            stop_runtime(state, &mut service, &active_id).await?;
        }
    }
    if deployment.argv.first().map(String::as_str) != Some("./main") {
        deployment.argv.insert(0, "./main".into());
    }
    deployment.status = "deploying".into();
    deployment.error = None;
    deployment.updated_at = chrono::Utc::now().to_rfc3339();
    service.active_package_id = Some(deployment.package_id.clone());
    service.active_deployment_id = Some(deployment.id.clone());
    service.updated_at = chrono::Utc::now().to_rfc3339();
    save_deployment(state, &deployment).await?;
    crate::services::save_service(state, &service).await?;
    let error = run_deployment(state, &mut deployment).await?;
    let status = if error.is_some() {
        Status::BadGateway
    } else {
        Status::Ok
    };
    Ok(action_response(status, service, deployment, error))
}

#[post("/apps/<app_id>/services/<service_id>/deployments/stop")]
pub async fn stop_service(
    state: &State<ApiState>,
    token: SessionToken,
    app_id: &str,
    service_id: &str,
) -> Result<ApiJson, ApiError> {
    let user = require_user(state, &token).await?;
    crate::services::ensure_owned_app(state, user.github_id, app_id).await?;
    let _guard = state.lock_service(user.github_id, app_id, service_id).await;
    let mut service =
        crate::services::load_owned_service(state, user.github_id, app_id, service_id).await?;
    let deployment_id = service
        .active_deployment_id
        .clone()
        .ok_or_else(|| json_error(Status::Conflict, "service has no active deployment"))?;
    let deployment = stop_runtime(state, &mut service, &deployment_id).await?;
    Ok(api_json(serde_json::json!({
        "service": service,
        "deployment": deployment,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_keys_are_scoped_to_the_owner_and_service() {
        assert_eq!(
            key_service_deployments(7, "a", "s"),
            "user:7:app:a:service:s:deployments"
        );
    }

    #[test]
    fn deployment_lists_are_newest_first() {
        let make = |id: &str, created_at: &str| Deployment {
            id: id.into(),
            service_id: "s".into(),
            package_id: "p".into(),
            app_id: "a".into(),
            owner: 1,
            argv: vec!["./main".into()],
            port: 3000,
            status: "running".into(),
            host: None,
            host_port: None,
            container_port: None,
            container_id: None,
            error: None,
            created_at: created_at.into(),
            updated_at: created_at.into(),
        };
        let mut deployments = vec![make("old", "2024-01-01"), make("new", "2024-01-02")];
        sort_deployments_newest_first(&mut deployments);
        assert_eq!(deployments[0].id, "new");
    }

    #[test]
    fn router_request_matches_contract() {
        let argv = runtime_argv(&["--listen".into()]);
        let value = serde_json::to_value(RouterDeployRequest {
            service_id: "s",
            package_id: "p",
            argv: &argv,
            port: 8080,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "service_id": "s",
                "package_id": "p",
                "argv": ["./main", "--listen"],
                "port": 8080
            })
        );
    }

    #[test]
    fn runtime_argv_always_starts_with_root_main() {
        assert_eq!(runtime_argv(&[]), vec!["./main"]);
        assert_eq!(
            runtime_argv(&["--port".into(), "8080".into()]),
            vec!["./main", "--port", "8080"]
        );
    }
}
