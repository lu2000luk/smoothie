use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Status;
use rocket::{Request, Response, State};
use rocket::http::Header;

use crate::config::Config;

pub struct ApiState {
    pub config: Config,
    pub redis: redis::Client,
    pub http: reqwest::Client,
}

pub fn json_error(
    status: Status,
    message: impl Into<String>,
) -> (Status, rocket::serde::json::Json<serde_json::Value>) {
    (
        status,
        rocket::serde::json::Json(serde_json::json!({ "error": message.into() })),
    )
}

pub async fn conn(
    state: &State<ApiState>,
) -> Result<redis::aio::MultiplexedConnection, (Status, rocket::serde::json::Json<serde_json::Value>)>
{
    state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| json_error(Status::ServiceUnavailable, format!("redis unavailable: {e}")))
}

pub struct Cors {
    pub frontend_url: String,
}

#[rocket::async_trait]
impl Fairing for Cors {
    fn info(&self) -> Info {
        Info {
            name: "CORS for frontend",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, _req: &'r Request<'_>, res: &mut Response<'r>) {
        res.set_header(Header::new(
            "Access-Control-Allow-Origin",
            self.frontend_url.clone(),
        ));
        res.set_header(Header::new("Vary", "Origin"));
        res.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
        res.set_header(Header::new(
            "Access-Control-Allow-Methods",
            "GET, POST, PUT, PATCH, DELETE, OPTIONS",
        ));
        res.set_header(Header::new(
            "Access-Control-Allow-Headers",
            "Authorization, Content-Type",
        ));
    }
}
