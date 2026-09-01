//! Forwards a container's exec output to the app's Redis log stream
//! (`logs:{container_id}`, payload `{timestamp}${level}${stream}: {line}`
//! per db.schema's `[datetime]$[level]$[message]` format).

use futures::StreamExt;
use tokio::task::JoinHandle;

use crate::engine::{ExecOutput, LogOutput};
use crate::globals;

/// Spawns a task that reads the exec output stream until it ends (the
/// process exited) and forwards complete lines to Redis. Abort the handle
/// to stop forwarding when a container is killed.
pub fn forward(container_id: String, mut output: ExecOutput) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(client) = globals::REDIS_CLIENT.get() else {
            eprintln!("[log] Redis client not set; dropping logs for {container_id}");
            return;
        };
        let mut conn = match client.get_multiplexed_async_connection().await {
            Ok(conn) => conn,
            Err(error) => {
                eprintln!("[log] Redis connection failed for {container_id}: {error}");
                return;
            }
        };

        let mut stdout = LineBuffer::new("info", "stdout");
        let mut stderr = LineBuffer::new("error", "stderr");

        while let Some(chunk) = output.next().await {
            match chunk {
                Ok(LogOutput::StdOut { message }) | Ok(LogOutput::Console { message }) => {
                    stdout.push(&message, &container_id, &mut conn).await;
                }
                Ok(LogOutput::StdErr { message }) => {
                    stderr.push(&message, &container_id, &mut conn).await;
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[log] output stream error for {container_id}: {error}");
                    break;
                }
            }
        }

        stdout.flush(&container_id, &mut conn).await;
        stderr.flush(&container_id, &mut conn).await;
    })
}

struct LineBuffer {
    level: &'static str,
    stream: &'static str,
    pending: String,
}

impl LineBuffer {
    fn new(level: &'static str, stream: &'static str) -> Self {
        Self {
            level,
            stream,
            pending: String::new(),
        }
    }

    async fn push(
        &mut self,
        bytes: &[u8],
        container_id: &str,
        conn: &mut redis::aio::MultiplexedConnection,
    ) {
        self.pending.push_str(&String::from_utf8_lossy(bytes));
        while let Some(pos) = self.pending.find('\n') {
            let line = self.pending[..pos].trim_end_matches('\r').to_string();
            self.pending.drain(..=pos);
            if !line.is_empty() {
                self.emit(&line, container_id, conn).await;
            }
        }
    }

    async fn flush(
        &mut self,
        container_id: &str,
        conn: &mut redis::aio::MultiplexedConnection,
    ) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.emit(&line, container_id, conn).await;
        }
    }

    async fn emit(
        &self,
        line: &str,
        container_id: &str,
        conn: &mut redis::aio::MultiplexedConnection,
    ) {
        let payload = format!(
            "{}${}${}: {}",
            timestamp(),
            self.level,
            self.stream,
            line
        );
        let _: Result<(), _> = redis::cmd("XADD")
            .arg(format!("logs:{container_id}"))
            .arg("*")
            .arg("msg")
            .arg(&payload)
            .query_async(conn)
            .await;
    }
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
