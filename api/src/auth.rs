use redis::AsyncCommands;
use rocket::http::{Cookie, SameSite, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket::serde::{Deserialize, Serialize};
use rocket::{Request, State, get, post};

use crate::config::GithubConfig;
use crate::state::{ApiState, conn, json_error};

pub const SESSION_COOKIE: &str = "smoothie_session";
pub const SESSION_TTL_SECS: u64 = 30 * 24 * 60 * 60;
pub const OAUTH_STATE_TTL_SECS: u64 = 600;

pub struct SessionToken(pub Option<String>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SessionToken {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        if let Some(header) = req.headers().get_one("Authorization") {
            let header = header.trim();
            let token = header
                .strip_prefix("Bearer ")
                .or_else(|| header.strip_prefix("bearer "));
            if let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) {
                return Outcome::Success(SessionToken(Some(token.to_owned())));
            }
        }
        if let Some(cookie) = req.cookies().get(SESSION_COOKIE) {
            let value = cookie.value().trim();
            if !value.is_empty() {
                return Outcome::Success(SessionToken(Some(value.to_owned())));
            }
        }
        Outcome::Success(SessionToken(None))
    }
}

#[derive(Deserialize, Debug)]
struct GithubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GithubUser {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GithubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "rocket::serde")]
pub struct StoredUser {
    pub github_id: u64,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub email: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn key_user(github_id: u64) -> String {
    format!("user:github:{github_id}")
}

pub fn key_user_login(login: &str) -> String {
    format!("user:login:{}", login.to_lowercase())
}

pub fn key_session(token: &str) -> String {
    format!("session:{token}")
}

pub fn key_user_sessions(github_id: u64) -> String {
    format!("user:github:{github_id}:sessions")
}

pub fn key_oauth_state(state: &str) -> String {
    format!("oauth:state:{state}")
}

pub fn github_authorize_url(cfg: &GithubConfig, state: &str) -> String {
    let scope = urlencoding::encode("read:user user:email");
    format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope={}&state={}",
        urlencoding::encode(&cfg.client_id),
        urlencoding::encode(&cfg.redirect_uri),
        scope,
        urlencoding::encode(state),
    )
}

pub fn frontend_url(cfg: &GithubConfig) -> String {
    cfg.frontend_url.trim_end_matches('/').to_owned()
}

async fn exchange_code(
    http: &reqwest::Client,
    cfg: &GithubConfig,
    code: &str,
) -> Result<String, String> {
    let res = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": cfg.client_id,
            "client_secret": cfg.client_secret,
            "code": code,
            "redirect_uri": cfg.redirect_uri,
        }))
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("token request returned {}", res.status()));
    }
    let body: GithubTokenResponse = res
        .json()
        .await
        .map_err(|e| format!("token response parse failed: {e}"))?;
    body.access_token.ok_or_else(|| {
        body.error_description
            .or(body.error)
            .unwrap_or_else(|| "missing access token".into())
    })
}

async fn fetch_github_user(http: &reqwest::Client, token: &str) -> Result<GithubUser, String> {
    let mut user: GithubUser = http
        .get("https://api.github.com/user")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "smoothie")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("user request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("user response parse failed: {e}"))?;
    if user.email.is_none() {
        let emails: Vec<GithubEmail> = http
            .get("https://api.github.com/user/emails")
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "smoothie")
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("emails request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("emails response parse failed: {e}"))?;
        user.email = emails
            .iter()
            .find(|e| e.primary && e.verified)
            .or_else(|| emails.iter().find(|e| e.verified))
            .map(|e| e.email.clone());
    }
    Ok(user)
}

pub async fn require_user(
    state: &State<ApiState>,
    token: &SessionToken,
) -> Result<StoredUser, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let token = token.0.clone().ok_or_else(|| {
        json_error(
            Status::Unauthorized,
            "missing session (Bearer token or smoothie_session cookie)",
        )
    })?;
    let mut redis = conn(state).await?;
    let github_id: Option<String> = redis
        .get(key_session(&token))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let github_id = github_id.ok_or_else(|| json_error(Status::Unauthorized, "invalid or expired session"))?;
    let github_id: u64 = github_id
        .parse()
        .map_err(|_| json_error(Status::Unauthorized, "invalid session"))?;
    let raw: Option<String> = redis
        .get(key_user(github_id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let raw = raw.ok_or_else(|| json_error(Status::Unauthorized, "user not found"))?;
    serde_json::from_str(&raw).map_err(|_| json_error(Status::InternalServerError, "stored user is corrupt"))
}

#[get("/auth/github")]
pub async fn github_login(
    state: &State<ApiState>,
) -> Result<Redirect, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    if crate::config::github_configured(&state.config.github) == false {
        return Err(json_error(
            Status::ServiceUnavailable,
            "github auth is not configured (set github.client_id / github.client_secret in config.json)",
        ));
    }
    let session = uuid::Uuid::new_v4().simple().to_string();
    let mut redis = conn(state).await?;
    redis
        .set_ex::<_, _, ()>(key_oauth_state(&session), "1", OAUTH_STATE_TTL_SECS)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok(Redirect::temporary(github_authorize_url(
        &state.config.github,
        &session,
    )))
}

#[get("/auth/github/url")]
pub async fn github_login_url(
    state: &State<ApiState>,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)>
{
    if !crate::config::github_configured(&state.config.github) {
        return Err(json_error(
            Status::ServiceUnavailable,
            "github auth is not configured (set github.client_id / github.client_secret in config.json)",
        ));
    }
    let session = uuid::Uuid::new_v4().simple().to_string();
    let mut redis = conn(state).await?;
    redis
        .set_ex::<_, _, ()>(key_oauth_state(&session), "1", OAUTH_STATE_TTL_SECS)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    Ok(rocket::serde::json::Json(serde_json::json!({
        "url": github_authorize_url(&state.config.github, &session),
    })))
}

#[get("/auth/github/callback?<code>&<state>")]
pub async fn github_callback(
    app: &State<ApiState>,
    cookies: &rocket::http::CookieJar<'_>,
    code: Option<String>,
    state: Option<String>,
) -> Result<Redirect, (Status, rocket::serde::json::Json<serde_json::Value>)> {
    let code = code
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| json_error(Status::BadRequest, "missing ?code="))?;
    let oauth_state = state
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| json_error(Status::BadRequest, "missing ?state="))?;
    if !crate::config::github_configured(&app.config.github) {
        return Err(json_error(
            Status::ServiceUnavailable,
            "github auth is not configured",
        ));
    }

    let mut redis = conn(app).await?;
    let stored: Option<String> = redis
        .get(key_oauth_state(&oauth_state))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    if stored.is_none() {
        return Err(json_error(Status::Unauthorized, "invalid or expired ?state="));
    }
    let _: () = redis
        .del(key_oauth_state(&oauth_state))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;

    let access_token = exchange_code(&app.http, &app.config.github, &code)
        .await
        .map_err(|e| json_error(Status::BadGateway, format!("github oauth failed: {e}")))?;
    let gh = fetch_github_user(&app.http, &access_token)
        .await
        .map_err(|e| json_error(Status::BadGateway, format!("github user lookup failed: {e}")))?;

    let now = chrono::Utc::now().to_rfc3339();
    let existing: Option<String> = redis
        .get(key_user(gh.id))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let created_at = existing
        .and_then(|raw| serde_json::from_str::<StoredUser>(&raw).ok())
        .map(|u| u.created_at)
        .unwrap_or_else(|| now.clone());
    let user = StoredUser {
        github_id: gh.id,
        login: gh.login.clone(),
        name: gh.name.clone(),
        avatar_url: gh.avatar_url.clone(),
        email: gh.email.clone(),
        created_at,
        updated_at: now,
    };
    let raw = serde_json::to_string(&user)
        .map_err(|e| json_error(Status::InternalServerError, format!("encode user failed: {e}")))?;
    redis
        .set::<_, _, ()>(key_user(gh.id), raw)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    redis
        .set::<_, _, ()>(key_user_login(&gh.login), gh.id.to_string())
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;

    let token = uuid::Uuid::new_v4().simple().to_string();
    redis
        .set_ex::<_, _, ()>(key_session(&token), gh.id.to_string(), SESSION_TTL_SECS)
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    redis
        .sadd::<_, _, ()>(key_user_sessions(gh.id), token.clone())
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;

    cookies.add(
        Cookie::build((SESSION_COOKIE, token.clone()))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .build(),
    );
    Ok(Redirect::temporary(format!(
        "{}/auth/callback?session={}",
        frontend_url(&app.config.github),
        urlencoding::encode(&token),
    )))
}

#[get("/auth/me")]
pub async fn me(
    state: &State<ApiState>,
    token: SessionToken,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)>
{
    let user = require_user(state, &token).await?;
    Ok(rocket::serde::json::Json(serde_json::json!({ "user": user })))
}

#[post("/auth/logout")]
pub async fn logout(
    state: &State<ApiState>,
    cookies: &rocket::http::CookieJar<'_>,
    token: SessionToken,
) -> Result<rocket::serde::json::Json<serde_json::Value>, (Status, rocket::serde::json::Json<serde_json::Value>)>
{
    let token = token
        .0
        .ok_or_else(|| json_error(Status::Unauthorized, "missing session"))?;
    let mut redis = conn(state).await?;
    let github_id: Option<String> = redis
        .get(key_session(&token))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    let _: () = redis
        .del(key_session(&token))
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    if let Some(id) = github_id.and_then(|id| id.parse::<u64>().ok()) {
        let _: () = redis
            .srem(key_user_sessions(id), token.clone())
            .await
            .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))?;
    }
    cookies.remove(Cookie::build(SESSION_COOKIE).path("/").build());
    Ok(rocket::serde::json::Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_points_at_github_with_state() {
        let cfg = GithubConfig {
            client_id: "Iv1.test".into(),
            client_secret: "secret".into(),
            redirect_uri: "http://localhost:3400/auth/github/callback".into(),
            frontend_url: "http://localhost:5173".into(),
        };
        let url = github_authorize_url(&cfg, "abc123");
        assert!(url.starts_with("https://github.com/login/oauth/authorize?"));
        assert!(url.contains("client_id=Iv1.test"));
        assert!(url.contains("state=abc123"));
        assert!(url.contains("redirect_uri="));
    }

    #[test]
    fn redis_keys_are_namespaced_by_github_id() {
        assert_eq!(key_user(123), "user:github:123");
        assert_eq!(key_user_login("OctoCat"), "user:login:octocat");
        assert_eq!(key_session("tok"), "session:tok");
        assert_eq!(key_user_sessions(123), "user:github:123:sessions");
        assert_eq!(key_oauth_state("s"), "oauth:state:s");
    }
}
