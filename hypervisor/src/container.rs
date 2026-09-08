//! Container lifecycle: the `select_tarball -> inject -> run -> kill` API
//! from arch.md, built on the engine backend in `engine.rs`.
//!
//! Performance model: a pool of idle containers is started ahead of time,
//! so `inject` is just a tar upload into an already-running container and
//! `run` is an exec — the hot path never waits for a container cold start.
//! The pool is topped back up slowly in the background whenever a slot is
//! consumed.

use std::{
    collections::VecDeque, fmt, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration,
};

use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify},
    task::JoinHandle,
};

use crate::engine::{Engine, EngineError};

pub type Result<T> = std::result::Result<T, ContainerError>;

#[derive(Debug)]
pub enum ContainerError {
    EmptyCommand,
    Package(String),
    Io(io::Error),
    IdlePoolExhausted,
    Engine(EngineError),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCommand => write!(f, "a container needs at least one argument to run"),
            Self::Package(err) => write!(f, "failed to load package: {err}"),
            Self::Io(err) => write!(f, "network I/O failed: {err}"),
            Self::IdlePoolExhausted => write!(f, "no idle containers are available"),
            Self::Engine(err) => write!(f, "container engine request failed: {err}"),
        }
    }
}

impl std::error::Error for ContainerError {}

impl From<io::Error> for ContainerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<EngineError> for ContainerError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

/// A published host port relaying to `127.0.0.1:container_port`.
///
/// This gives Docker-style `HOST_PORT:CONTAINER_PORT` behaviour: the host
/// port is selected and reserved by `port.rs`, while the application can
/// listen on any distinct container port. Containers currently share the
/// host network namespace, so the relay targets loopback; a future isolated
/// namespace backend must route the relay to that namespace instead.
#[derive(Debug)]
pub struct PortMapping {
    host_port: u16,
    container_port: u16,
    relay: JoinHandle<()>,
}

impl PortMapping {
    pub async fn publish(container_port: u16) -> Result<Self> {
        let reservation = crate::port::reserve_free_port().map_err(ContainerError::Io)?;
        let host_port = reservation.port();
        eprintln!(
            "[container] port: publishing host_port={} -> container_port={}",
            host_port, container_port
        );
        let listener = reservation.into_listener();
        listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(listener)?;
        let target = SocketAddr::from(([127, 0, 0, 1], container_port));
        let relay = tokio::spawn(async move {
            loop {
                let Ok((mut inbound, addr)) = listener.accept().await else {
                    break;
                };
                eprintln!(
                    "[container] port: relay accepted connection from {}:{}",
                    addr.ip(),
                    addr.port()
                );
                let target = target;
                tokio::spawn(async move {
                    let Ok(mut outbound) = async {
                        for _ in 0..30 {
                            match TcpStream::connect(target).await {
                                Ok(stream) => return Ok(stream),
                                Err(_) => {
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                }
                            }
                        }
                        Err(std::io::Error::new(
                            std::io::ErrorKind::ConnectionRefused,
                            "target not ready after retries",
                        ))
                    }
                    .await
                    else {
                        return;
                    };
                    let _ = copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });
        Ok(Self {
            host_port,
            container_port,
            relay,
        })
    }

    pub fn host_port(&self) -> u16 {
        self.host_port
    }
    pub fn container_port(&self) -> u16 {
        self.container_port
    }
}

impl Drop for PortMapping {
    fn drop(&mut self) {
        self.relay.abort();
    }
}

pub struct SelectedTarball(PathBuf);

pub struct InjectedContainer {
    id: String,
    argv: Vec<String>,
}

impl InjectedContainer {
    pub fn id(&self) -> &str {
        &self.id
    }
}

pub struct RunningContainer {
    id: String,
    port: PortMapping,
    log_task: JoinHandle<()>,
}

impl RunningContainer {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn host_port(&self) -> u16 {
        self.port.host_port()
    }
    pub fn container_port(&self) -> u16 {
        self.port.container_port()
    }
}

#[derive(Clone)]
pub struct ContainerApi {
    engine: Arc<Engine>,
    idle: Arc<IdleContainerPool>,
}

impl ContainerApi {
    pub fn new(engine: Arc<Engine>, idle: Arc<IdleContainerPool>) -> Self {
        Self { engine, idle }
    }

    pub async fn idle_ids(&self) -> Vec<String> {
        self.idle.list_ids().await
    }

    pub async fn select_tarball(&self, path: impl Into<PathBuf>) -> Result<SelectedTarball> {
        let path = path.into();
        eprintln!("[container] select_tarball: path={}", path.display());
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ContainerError::Package(e.to_string()))?;
        if !metadata.is_file() {
            return Err(ContainerError::Package("tarball path is not a file".into()));
        }
        eprintln!("[container] select_tarball: ok size={}", metadata.len());
        Ok(SelectedTarball(path))
    }

    pub async fn inject(
        &self,
        tarball: SelectedTarball,
        argv: Vec<String>,
    ) -> Result<InjectedContainer> {
        if argv.is_empty() {
            return Err(ContainerError::EmptyCommand);
        }
        eprintln!(
            "[container] inject: tarball={} argv={:?}",
            tarball.0.display(),
            argv
        );
        let tar = tokio::fs::read(&tarball.0)
            .await
            .map_err(|e| ContainerError::Package(e.to_string()))?;
        let id = self
            .idle
            .acquire()
            .await
            .ok_or(ContainerError::IdlePoolExhausted)?;
        eprintln!("[container] inject: id={id} uploading package into container...");
        if let Err(error) = self.engine.upload_package(&id, tar.into()).await {
            // The container may hold a partial extraction; destroy it rather
            // than returning a dirty slot to the pool. The pool maintainer
            // replaces it in the background.
            eprintln!("[container] inject: id={id} upload failed: {error}");
            let _ = self.engine.remove_container(&id).await;
            return Err(error.into());
        }
        eprintln!("[container] inject: id={id} done");
        Ok(InjectedContainer { id, argv })
    }

    pub async fn run(
        &self,
        injected: InjectedContainer,
        container_port: u16,
    ) -> Result<RunningContainer> {
        eprintln!(
            "[container] run: id={} container_port={}",
            injected.id, container_port
        );
        let port = PortMapping::publish(container_port).await?;
        eprintln!(
            "[container] run: id={} host_port={}",
            injected.id,
            port.host_port()
        );
        let output = match self.engine.exec(&injected.id, injected.argv, "/").await {
            Ok(output) => output,
            Err(error) => {
                eprintln!("[container] run: id={} exec failed: {error}", injected.id);
                let _ = self.engine.remove_container(&injected.id).await;
                return Err(error.into());
            }
        };
        let log_task = crate::log::forward(injected.id.clone(), output);
        eprintln!("[container] run: id={} started", injected.id);
        Ok(RunningContainer {
            id: injected.id,
            port,
            log_task,
        })
    }

    pub async fn shutdown(&self, running: RunningContainer) -> Result<()> {
        eprintln!("[container] shutdown: id={}", running.id);
        let result = self.engine.shutdown_container(&running.id).await;
        running.log_task.abort();
        if let Err(ref error) = result {
            eprintln!(
                "[container] shutdown: id={} remove failed: {error}",
                running.id
            );
        } else {
            eprintln!("[container] shutdown: id={} done", running.id);
        }
        // `running.port` drops here, closing the published host port.
        result.map_err(Into::into)
    }

    pub async fn kill(&self, running: RunningContainer) -> Result<()> {
        eprintln!("[container] kill: id={}", running.id);
        running.log_task.abort();
        let result = self.engine.remove_container(&running.id).await;
        if let Err(ref error) = result {
            eprintln!("[container] kill: id={} remove failed: {error}", running.id);
        } else {
            eprintln!("[container] kill: id={} done", running.id);
        }
        // `running.port` drops here, closing the published host port.
        result.map_err(Into::into)
    }
}

/// Pool of pre-started containers waiting for a package.
///
/// The initial fill happens synchronously at startup; afterwards a
/// background maintainer tops the pool back up to `target` one container at
/// a time, pausing between creations so replenishment never stresses the
/// system (arch.md: "dont stress the system to reach this, do it slowly").
pub struct IdleContainerPool {
    engine: Arc<Engine>,
    target: usize,
    available: Mutex<VecDeque<String>>,
    wake: Notify,
}

const REPLENISH_PAUSE: Duration = Duration::from_millis(500);
const REPLENISH_RETRY: Duration = Duration::from_secs(5);

impl IdleContainerPool {
    pub async fn new(engine: Arc<Engine>, target: u32) -> Result<Arc<Self>> {
        let target = target as usize;
        let mut available = VecDeque::with_capacity(target);
        for n in 0..target {
            eprintln!("[idle-pool] preparing container {}/{target}...", n + 1);
            available.push_back(engine.create_idle_container().await?);
        }
        let pool = Arc::new(Self {
            engine,
            target,
            available: Mutex::new(available),
            wake: Notify::new(),
        });
        tokio::spawn(Arc::clone(&pool).maintain());
        Ok(pool)
    }

    /// Takes an idle container out of the pool and wakes the maintainer to
    /// replace it.
    pub async fn acquire(&self) -> Option<String> {
        let id = self.available.lock().await.pop_front();
        if id.is_some() {
            self.wake.notify_one();
        }
        id
    }

    pub async fn list_ids(&self) -> Vec<String> {
        self.available.lock().await.iter().cloned().collect()
    }

    async fn maintain(self: Arc<Self>) {
        loop {
            self.wake.notified().await;
            loop {
                if self.available.lock().await.len() >= self.target {
                    break;
                }
                match self.engine.create_idle_container().await {
                    Ok(id) => {
                        eprintln!("[idle-pool] replenished container {id}");
                        self.available.lock().await.push_back(id);
                        tokio::time::sleep(REPLENISH_PAUSE).await;
                    }
                    Err(error) => {
                        eprintln!(
                            "[idle-pool] replenish failed: {error}; retrying in {}s",
                            REPLENISH_RETRY.as_secs()
                        );
                        tokio::time::sleep(REPLENISH_RETRY).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn published_host_port_relays_to_a_different_container_port() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let container_port = backend.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = backend.accept().await.unwrap();
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await.unwrap();
            stream.write_all(&buf).await.unwrap();
        });

        let mapping = PortMapping::publish(container_port).await.unwrap();
        assert_ne!(mapping.host_port(), mapping.container_port());

        let mut client = TcpStream::connect(("127.0.0.1", mapping.host_port()))
            .await
            .unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"ping");
    }
}
