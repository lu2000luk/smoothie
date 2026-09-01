//! Container engine backend.
//!
//! Talks to a Docker- or Podman-compatible daemon over its unix socket
//! (Podman exposes the Docker-compatible API on its socket, so both work
//! unchanged). This is the single place that knows about the engine API;
//! `container.rs` builds the hypervisor's container lifecycle on top of it,
//! so a future backend (e.g. raw crun, process mode) only has to replace
//! this module.
//!
//! Every container created here carries the [`MANAGED_LABEL`] plus an
//! [`INSTANCE_LABEL`] identifying this hypervisor run. Cleanup relies on
//! those labels: on startup we remove anything with the managed label
//! (leftovers from a crashed or SIGKILLed run), and on graceful shutdown we
//! remove everything from our own instance.

use std::collections::HashMap;

use bollard::models::{ContainerCreateBody, ExecConfig, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
    RemoveContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use bollard::{API_DEFAULT_VERSION, Docker, body_full};
use futures::{Stream, StreamExt};
use uuid::Uuid;

pub use bollard::container::LogOutput;
pub use bollard::errors::Error as EngineError;

pub type Result<T> = std::result::Result<T, EngineError>;

/// Combined stdout/stderr stream of a process exec'd inside a container.
pub type ExecOutput = std::pin::Pin<Box<dyn Stream<Item = Result<LogOutput>> + Send>>;

/// Label present on every container the hypervisor creates, ever.
pub const MANAGED_LABEL: &str = "io.smoothie.hypervisor";
pub const MANAGED_LABEL_VALUE: &str = "1";
/// Label scoping a container to one hypervisor run.
pub const INSTANCE_LABEL: &str = "io.smoothie.instance";

const CONNECT_TIMEOUT_SECS: u64 = 30;

/// Per-container resource limits, applied when the container is created.
#[derive(Clone, Debug, Default)]
pub struct ResourceLimits {
    pub memory_bytes: Option<i64>,
    pub memory_swap_bytes: Option<i64>,
    pub cpu_period_us: Option<i64>,
    pub cpu_quota_us: Option<i64>,
    pub pids: Option<i64>,
}

#[derive(Clone)]
pub struct Engine {
    docker: Docker,
    image: String,
    limits: ResourceLimits,
    instance: String,
}

impl Engine {
    /// Connects to the engine socket and verifies it responds to a ping.
    /// The hypervisor must not run without this succeeding.
    pub async fn connect(socket: &str, image: String, limits: ResourceLimits) -> Result<Self> {
        let docker = Docker::connect_with_unix(socket, CONNECT_TIMEOUT_SECS, API_DEFAULT_VERSION)?;
        docker.ping().await?;
        Ok(Self {
            docker,
            image,
            limits,
            instance: Uuid::new_v4().to_string(),
        })
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    /// Pulls the base image if it is not already present locally.
    pub async fn ensure_image(&self) -> Result<()> {
        if self.docker.inspect_image(&self.image).await.is_ok() {
            return Ok(());
        }
        eprintln!("[engine] pulling image {}...", self.image);
        let options = CreateImageOptionsBuilder::new()
            .from_image(&self.image)
            .build();
        let mut pull = self.docker.create_image(Some(options), None, None);
        while let Some(progress) = pull.next().await {
            progress?;
        }
        eprintln!("[engine] image {} ready", self.image);
        Ok(())
    }

    /// Creates and starts an idle container: the base image parked on
    /// `sleep infinity`, with this run's labels and the configured resource
    /// limits. Injection later uploads a package into it and `run` execs the
    /// entrypoint, so the hot path never pays a container cold start.
    pub async fn create_idle_container(&self) -> Result<String> {
        let name = format!("smoothie-{}", Uuid::new_v4());
        let body = ContainerCreateBody {
            image: Some(self.image.clone()),
            cmd: Some(vec!["sleep".into(), "infinity".into()]),
            labels: Some(self.labels()),
            host_config: Some(HostConfig {
                // Host networking keeps the Docker-style HOST:CONTAINER port
                // behaviour documented in arch.md: the app listens on its
                // container port on loopback and port.rs relays a reserved
                // host port to it. A future isolated-network backend must
                // route the relay into that namespace instead.
                network_mode: Some("host".into()),
                memory: self.limits.memory_bytes,
                memory_swap: self.limits.memory_swap_bytes,
                cpu_period: self.limits.cpu_period_us,
                cpu_quota: self.limits.cpu_quota_us,
                pids_limit: self.limits.pids,
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new().name(&name).build();
        self.docker.create_container(Some(options), body).await?;
        if let Err(error) = self.docker.start_container(&name, None).await {
            let _ = self.remove_container(&name).await;
            return Err(error);
        }
        Ok(name)
    }

    /// Uploads a package tarball into a running container; the engine
    /// extracts it at `/`.
    pub async fn upload_package(&self, id: &str, tar: bytes::Bytes) -> Result<()> {
        let options = UploadToContainerOptionsBuilder::new().path("/").build();
        self.docker
            .upload_to_container(id, Some(options), body_full(tar))
            .await
    }

    /// Starts `argv` inside a running container and returns its attached
    /// output stream.
    pub async fn exec(&self, id: &str, argv: Vec<String>, cwd: &str) -> Result<ExecOutput> {
        let exec = self
            .docker
            .create_exec(
                id,
                ExecConfig {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(argv),
                    working_dir: Some(cwd.into()),
                    ..Default::default()
                },
            )
            .await?;
        match self.docker.start_exec(&exec.id, None).await? {
            bollard::exec::StartExecResults::Attached { output, .. } => Ok(output),
            bollard::exec::StartExecResults::Detached => {
                unreachable!("start_exec without detach always attaches")
            }
        }
    }

    /// Force-removes a container (kills whatever is running inside it).
    /// A container that is already gone is not an error.
    pub async fn remove_container(&self, id: &str) -> Result<()> {
        let options = RemoveContainerOptionsBuilder::new().force(true).build();
        match self.docker.remove_container(id, Some(options)).await {
            Err(EngineError::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(()),
            result => result,
        }
    }

    /// Removes every container labeled as hypervisor-managed, regardless of
    /// which run created it. Called on startup so a crashed or killed run
    /// never leaks containers past the next boot.
    pub async fn cleanup_leftovers(&self) -> Result<usize> {
        self.remove_labeled(format!("{MANAGED_LABEL}={MANAGED_LABEL_VALUE}"))
            .await
    }

    /// Removes every container created by this run (idle, injected and
    /// running alike). Called on graceful shutdown.
    pub async fn cleanup_instance(&self) -> Result<usize> {
        self.remove_labeled(format!("{INSTANCE_LABEL}={}", self.instance))
            .await
    }

    async fn remove_labeled(&self, label: String) -> Result<usize> {
        let filters = HashMap::from([("label".to_string(), vec![label])]);
        let options = ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build();
        let containers = self.docker.list_containers(Some(options)).await?;
        let mut removed = 0;
        for container in containers {
            let Some(id) = container.id else { continue };
            match self.remove_container(&id).await {
                Ok(()) => removed += 1,
                Err(error) => eprintln!("[engine] cleanup: failed to remove {id}: {error}"),
            }
        }
        Ok(removed)
    }

    fn labels(&self) -> HashMap<String, String> {
        HashMap::from([
            (MANAGED_LABEL.into(), MANAGED_LABEL_VALUE.into()),
            (INSTANCE_LABEL.into(), self.instance.clone()),
        ])
    }
}
