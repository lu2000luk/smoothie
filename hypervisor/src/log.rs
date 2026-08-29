use std::time::Duration;

use futures::StreamExt;
use tokio::task::JoinHandle;

use crate::docker::{LogFrame, LogStream};
use crate::globals;

pub struct LogForwarder {
    task: JoinHandle<()>,
    container_id: String,
}

impl LogForwarder {
    /// Subscribe to the container's logs through the Docker daemon and forward
    /// each decoded frame into the per-container Redis stream.
    pub async fn start(container_id: String) -> std::io::Result<Self> {
        eprintln!("[log] starting log forwarder for {container_id}");
        let runtime = globals::DOCKER_RUNTIME
            .get()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "docker runtime not set"))?
            .clone();
        let id = container_id.clone();
        let task = tokio::spawn(async move {
            forward_logs(runtime, id).await;
        });
        Ok(Self {
            task,
            container_id,
        })
    }

    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Drop for LogForwarder {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward_logs(runtime: std::sync::Arc<crate::docker::DockerRuntime>, container_id: String) {
    // Give Docker a moment to attach to the container's logs. Repeatedly retry
    // because the log endpoint can race the container's start.
    let mut stream = match connect_logs(&runtime, &container_id).await {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("log forwarder connect failed for {container_id}: {err}");
            return;
        }
    };

    let client = match globals::REDIS_CLIENT.get() {
        Some(client) => client.clone(),
        None => {
            eprintln!("log forwarder: redis client not set");
            return;
        }
    };
    let mut conn = match client.get_multiplexed_async_connection().await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("log forwarder: redis connect failed: {err}");
            return;
        }
    };

    let mut line_buf = String::new();
    while let Some(item) = stream.next().await {
        let LogFrame { stream, payload } = match item {
            Ok(frame) => frame,
            Err(err) => {
                eprintln!("log forwarder: stream error: {err}");
                break;
            }
        };
        let chunk = String::from_utf8_lossy(&payload);
        line_buf.push_str(&chunk);
        while let Some(pos) = line_buf.find('\n') {
            let line = line_buf[..pos].to_string();
            line_buf.drain(..=pos);
            if !line.is_empty() {
                forward_line(&mut conn, &container_id, stream, &line).await;
            }
        }
    }
    if !line_buf.is_empty() {
        forward_line(&mut conn, &container_id, LogStream::Stdout, &line_buf).await;
        line_buf.clear();
    }
}

async fn connect_logs(
    runtime: &crate::docker::DockerRuntime,
    container_id: &str,
) -> Result<
    std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<LogFrame, crate::container::ContainerError>> + Send>,
    >,
    crate::container::ContainerError,
> {
    let mut last_err: Option<crate::container::ContainerError> = None;
    for _ in 0..30 {
        match runtime.logs(container_id).await {
            Ok(stream) => return Ok(Box::pin(stream)),
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| crate::container::ContainerError::Docker {
        operation: "logs",
        message: "failed to attach to container logs".into(),
    }))
}

async fn forward_line(
    conn: &mut redis::aio::MultiplexedConnection,
    container_id: &str,
    stream: LogStream,
    line: &str,
) {
    let prefix = match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
        LogStream::Stdin => "stdin",
    };
    let payload = format!("{}$info${prefix}: {}", timestamp(), line);
    let _: Result<(), _> = redis::cmd("XADD")
        .arg(format!("logs:{container_id}"))
        .arg("*")
        .arg("msg")
        .arg(&payload)
        .query_async(conn)
        .await;
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
