use std::{collections::HashMap, io, sync::Arc, time::Duration};

use actix_web::{HttpRequest, HttpResponse, web};
use bytes::Bytes;
use futures::StreamExt;
use serde::Serialize;
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, broadcast, watch},
};

use crate::engine::{Engine, EngineError, ExecInput, LogOutput};

const BOX_LIFETIME: Duration = Duration::from_secs(4 * 60 * 60);
const OUTPUT_BUFFER_CHUNKS: usize = 256;

pub struct BoxSession {
    id: String,
    input: Mutex<ExecInput>,
    output: broadcast::Sender<Bytes>,
    closed: watch::Receiver<bool>,
}

impl BoxSession {
    async fn send_input(&self, bytes: &[u8]) -> io::Result<()> {
        let mut input = self.input.lock().await;
        input.write_all(bytes).await?;
        input.flush().await
    }

    fn subscribe_output(&self) -> broadcast::Receiver<Bytes> {
        self.output.subscribe()
    }

    fn subscribe_closed(&self) -> watch::Receiver<bool> {
        self.closed.clone()
    }
}

#[derive(Clone)]
pub struct BoxRegistry {
    engine: Arc<Engine>,
    sessions: Arc<Mutex<HashMap<String, Arc<BoxSession>>>>,
}

impl BoxRegistry {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create(&self) -> Result<Arc<BoxSession>, EngineError> {
        // Boxes are deliberately not taken from the package idle pool: every
        // request gets a newly created Alpine container.
        let id = self.engine.create_idle_container().await?;
        let interactive = match self
            .engine
            .exec_interactive(&id, vec!["/bin/sh".into()], "/")
            .await
        {
            Ok(interactive) => interactive,
            Err(error) => {
                let _ = self.engine.remove_container(&id).await;
                return Err(error);
            }
        };

        let (output, _) = broadcast::channel(OUTPUT_BUFFER_CHUNKS);
        let (closed_tx, closed) = watch::channel(false);
        let session = Arc::new(BoxSession {
            id: id.clone(),
            input: Mutex::new(interactive.input),
            output,
            closed,
        });
        self.sessions
            .lock()
            .await
            .insert(id.clone(), Arc::clone(&session));

        let registry = self.clone();
        let session_for_task = Arc::clone(&session);
        tokio::spawn(async move {
            let mut process_output = interactive.output;
            let deadline = tokio::time::sleep(BOX_LIFETIME);
            tokio::pin!(deadline);

            loop {
                tokio::select! {
                    _ = &mut deadline => {
                        eprintln!("[box] id={id} reached the four-hour limit");
                        break;
                    }
                    chunk = process_output.next() => {
                        match chunk {
                            Some(Ok(LogOutput::StdOut { message }))
                            | Some(Ok(LogOutput::StdErr { message }))
                            | Some(Ok(LogOutput::Console { message })) => {
                                let _ = session_for_task.output.send(message);
                            }
                            Some(Ok(_)) => {}
                            Some(Err(error)) => {
                                eprintln!("[box] id={id} shell output failed: {error}");
                                break;
                            }
                            None => {
                                eprintln!("[box] id={id} shell exited");
                                break;
                            }
                        }
                    }
                }
            }

            if let Err(error) = registry.engine.remove_container(&id).await {
                eprintln!("[box] id={id} cleanup failed: {error}");
            }
            registry.sessions.lock().await.remove(&id);
            let _ = closed_tx.send(true);
        });

        Ok(session)
    }

    pub async fn get(&self, id: &str) -> Option<Arc<BoxSession>> {
        self.sessions.lock().await.get(id).cloned()
    }
}

#[derive(Serialize)]
pub struct CreateBoxResponse {
    id: String,
    ws_endpoint: String,
    expires_in_seconds: u64,
}

#[actix_web::post("/box")]
pub async fn create_box(request: HttpRequest, registry: web::Data<BoxRegistry>) -> HttpResponse {
    match registry.create().await {
        Ok(session) => {
            let connection = request.connection_info();
            let ws_scheme = if connection.scheme() == "https" {
                "wss"
            } else {
                "ws"
            };
            let ws_endpoint = format!("{ws_scheme}://{}/box/{}/ws", connection.host(), session.id);
            HttpResponse::Created().json(CreateBoxResponse {
                id: session.id.clone(),
                ws_endpoint,
                expires_in_seconds: BOX_LIFETIME.as_secs(),
            })
        }
        Err(error) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": error.to_string()})),
    }
}

#[actix_web::get("/box/{id}/ws")]
pub async fn connect_box(
    request: HttpRequest,
    payload: web::Payload,
    id: web::Path<String>,
    registry: web::Data<BoxRegistry>,
) -> Result<HttpResponse, actix_web::Error> {
    let Some(box_session) = registry.get(&id).await else {
        return Ok(HttpResponse::NotFound()
            .json(serde_json::json!({"error": "box not found or shell has exited"})));
    };

    let (response, mut websocket, mut incoming) = actix_ws::handle(&request, payload)?;
    actix_web::rt::spawn(async move {
        let mut output = box_session.subscribe_output();
        let mut closed = box_session.subscribe_closed();

        loop {
            tokio::select! {
                message = incoming.next() => {
                    match message {
                        Some(Ok(actix_ws::Message::Text(text))) => {
                            if box_session.send_input(text.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Binary(bytes))) => {
                            if box_session.send_input(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Ping(bytes))) => {
                            if websocket.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Close(reason))) => {
                            let _ = websocket.close(reason).await;
                            return;
                        }
                        Some(Ok(actix_ws::Message::Continuation(_))) => {
                            let _ = websocket
                                .close(Some(actix_ws::CloseReason {
                                    code: actix_ws::CloseCode::Unsupported,
                                    description: Some("fragmented messages are not supported".into()),
                                }))
                                .await;
                            return;
                        }
                        Some(Ok(actix_ws::Message::Pong(_) | actix_ws::Message::Nop)) => {}
                        Some(Err(_)) | None => return,
                    }
                }
                chunk = output.recv() => {
                    match chunk {
                        Ok(bytes) => {
                            if websocket.binary(bytes).await.is_err() {
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let _ = websocket
                                .close(Some(actix_ws::CloseReason {
                                    code: actix_ws::CloseCode::Error,
                                    description: Some("client could not keep up with shell output".into()),
                                }))
                                .await;
                            return;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                changed = closed.changed() => {
                    if changed.is_err() || *closed.borrow() {
                        break;
                    }
                }
            }
        }

        let _ = websocket
            .close(Some(actix_ws::CloseReason {
                code: actix_ws::CloseCode::Normal,
                description: Some("shell terminated".into()),
            }))
            .await;
    });

    Ok(response)
}
