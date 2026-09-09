use std::collections::HashMap;
use std::sync::{Arc, Weak};

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::http::Status;
use rocket::{Request, Response};

use crate::config::Config;

type ServiceLockKey = (u64, String, String);

pub struct ApiState {
    pub config: Config,
    pub redis: redis::Client,
    pub http: reqwest::Client,
    pub s3: aws_sdk_s3::Client,
    pub(crate) service_locks:
        tokio::sync::Mutex<HashMap<ServiceLockKey, Weak<tokio::sync::Mutex<()>>>>,
}

async fn acquire_service_lock(
    registry: &tokio::sync::Mutex<HashMap<ServiceLockKey, Weak<tokio::sync::Mutex<()>>>>,
    owner: u64,
    app_id: &str,
    service_id: &str,
) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut registry = registry.lock().await;
        registry.retain(|_, lock| lock.strong_count() > 0);
        let key = (owner, app_id.to_owned(), service_id.to_owned());
        if let Some(lock) = registry.get(&key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            registry.insert(key, Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}

impl ApiState {
    pub async fn lock_service(
        &self,
        owner: u64,
        app_id: &str,
        service_id: &str,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        acquire_service_lock(&self.service_locks, owner, app_id, service_id).await
    }
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
    state: &ApiState,
) -> Result<redis::aio::MultiplexedConnection, (Status, rocket::serde::json::Json<serde_json::Value>)>
{
    state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| {
            json_error(
                Status::ServiceUnavailable,
                format!("redis unavailable: {e}"),
            )
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn service_lock_serializes_the_same_owner_app_and_service() {
        let registry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let first = acquire_service_lock(&registry, 1, "app", "service").await;
        let second_registry = Arc::clone(&registry);
        let (acquired_tx, mut acquired_rx) = tokio::sync::mpsc::channel(1);
        let second = tokio::spawn(async move {
            let _guard = acquire_service_lock(&second_registry, 1, "app", "service").await;
            acquired_tx.send(()).await.unwrap();
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(20), acquired_rx.recv())
                .await
                .is_err()
        );
        drop(first);
        assert_eq!(acquired_rx.recv().await, Some(()));
        second.await.unwrap();
    }
}
