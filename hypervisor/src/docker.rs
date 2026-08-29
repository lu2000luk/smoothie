use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, KillContainerOptions, LogsOptions,
    RemoveContainerOptions, StartContainerOptions,
};
use bollard::image::{CreateImageOptions, RemoveImageOptions};
use bollard::models::{
    DeviceRequest, HostConfig, HostConfigLogConfig, Mount, MountTypeEnum, Resources,
};
use bollard::Docker;
use bytes::Bytes;
use futures::{Stream, TryStreamExt};

use crate::container::{
    Architecture, ContainerDefinition, ContainerError, ContainerRequest, NetworkMode,
    ProxyConfig, ResourceLimits, Result,
};

const BASE_IMAGE_TAG: &str = "smoothie/base:latest";
const SMOOTHIE_LABEL: &str = "io.smoothie.managed";

const DOCKER_CANDIDATE_SOCKETS: &[&str] = &[
    "/var/run/docker.sock",
    "/run/docker.sock",
    "/run/podman/podman.sock",
    "/var/run/podman/podman.sock",
];

/// Discover the default Docker / Podman socket path.
///
/// Honours the `DOCKER_HOST` and `CONTAINER_HOST` environment variables when they
/// point at a Unix socket, then falls back to the well-known socket locations.
pub fn default_docker_socket() -> Option<PathBuf> {
    for env in ["DOCKER_HOST", "CONTAINER_HOST"] {
        if let Ok(value) = std::env::var(env) {
            if let Some(path) = parse_unix_socket(&value) {
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }
    for candidate in DOCKER_CANDIDATE_SOCKETS {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn parse_unix_socket(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix("unix://") {
        Some(PathBuf::from(rest))
    } else if value.starts_with('/') {
        Some(PathBuf::from(value))
    } else {
        None
    }
}

#[derive(Debug)]
pub struct DockerRuntime {
    client: Docker,
    base_image: String,
    base_rootfs: PathBuf,
    runtime_root: PathBuf,
    /// Bollard image builds (e.g. `create_image`) cannot be invoked concurrently
    /// against the same daemon by a single client. Serialize them.
    import_lock: Arc<tokio::sync::Mutex<()>>,
}

impl DockerRuntime {
    /// Connect to the Docker / Podman daemon, verify reachability, and import the
    /// base image. Fails fast if any of those steps fail.
    pub async fn connect(
        socket_path: PathBuf,
        base_rootfs: PathBuf,
        runtime_root: PathBuf,
    ) -> Result<Self> {
        eprintln!("[docker] connecting to {}", socket_path.display());
        let client = Docker::connect_with_socket(
            &socket_path.to_string_lossy(),
            30,
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(|err| ContainerError::Docker {
            operation: "connect",
            message: format!("failed to create Docker client: {err}"),
        })?;
        client.ping().await.map_err(|err| ContainerError::Docker {
            operation: "ping",
            message: format!(
                "cannot reach Docker daemon at {}: {err}",
                socket_path.display()
            ),
        })?;
        let version = client.version().await.map_err(|err| ContainerError::Docker {
            operation: "version",
            message: format!("failed to query daemon version: {err}"),
        })?;
        eprintln!(
            "[docker] connected: {} {}",
            version.version.unwrap_or_default(),
            version.api_version.unwrap_or_default()
        );

        let runtime = Self {
            client,
            base_image: BASE_IMAGE_TAG.to_string(),
            base_rootfs,
            runtime_root,
            import_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        runtime.ensure_base_image().await?;
        Ok(runtime)
    }

    pub fn client(&self) -> &Docker {
        &self.client
    }

    pub fn base_image(&self) -> &str {
        &self.base_image
    }

    pub fn base_rootfs(&self) -> &Path {
        &self.base_rootfs
    }

    pub fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    /// Build the base image from the on-disk rootfs if it is not already present.
    async fn ensure_base_image(&self) -> Result<()> {
        if self.image_exists(&self.base_image).await? {
            eprintln!("[docker] base image {} already present", self.base_image);
            return Ok(());
        }
        eprintln!(
            "[docker] importing base rootfs from {} as {}",
            self.base_rootfs.display(),
            self.base_image
        );
        let _guard = self.import_lock.lock().await;
        if self.image_exists(&self.base_image).await? {
            return Ok(());
        }
        let tar = tar_dir(&self.base_rootfs).await?;
        self.import_tar(&tar, Some(BASE_IMAGE_TAG)).await?;
        eprintln!("[docker] base image {} imported", self.base_image);
        let _ = tokio::fs::remove_file(&tar).await;
        Ok(())
    }

    /// Build a per-container image by importing the merged rootfs as a single
    /// layer. The image is tagged with `smoothie/{id}:latest`.
    async fn build_container_image(&self, container_id: &str, rootfs: &Path) -> Result<String> {
        let tag = format!("smoothie/{}:latest", container_id);
        eprintln!(
            "[docker] building image for {container_id} from {} -> {tag}",
            rootfs.display()
        );
        let _guard = self.import_lock.lock().await;
        let tar = tar_dir(rootfs).await?;
        self.import_tar(&tar, Some(&tag)).await?;
        let _ = tokio::fs::remove_file(&tar).await;
        Ok(tag)
    }

    /// Translate a high-level `ContainerRequest` into a `ContainerDefinition`,
    /// building the per-container image on demand.
    pub async fn prepare(&self, request: ContainerRequest) -> Result<ContainerDefinition> {
        validate_request(&request)?;
        let image = self
            .build_container_image(&request.id, &request.rootfs)
            .await?;
        let mut env: Vec<String> = request
            .env
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        append_proxy_env(&mut env, &request.proxy);
        let mut argv = request.argv;
        if let Architecture::Qemu { emulator, guest } = &request.architecture {
            env.push(format!("SMOOTHIE_GUEST_ARCH={guest}"));
            let mut emulated = vec![emulator.display().to_string(), "--".into()];
            emulated.append(&mut argv);
            argv = emulated;
        }
        Ok(ContainerDefinition {
            id: request.id,
            image,
            argv,
            env,
            cwd: request.cwd,
            network: request.network,
            limits: request.limits,
        })
    }

    /// Create the container with the daemon and start it. Errors after the
    /// container has been created are followed by a best-effort cleanup.
    pub async fn start(&self, definition: &ContainerDefinition) -> Result<()> {
        let cwd = definition.cwd.to_string_lossy().to_string();
        self.create_and_start(
            &definition.id,
            &definition.image,
            &definition.argv,
            &definition.env,
            &cwd,
            definition.network.docker_value(),
            &definition.limits,
        )
        .await
    }

    /// Send SIGKILL to the container. `NotFound` is treated as success so callers
    /// can use this as an idempotent stop hook.
    pub async fn kill(&self, definition: &ContainerDefinition) -> Result<()> {
        eprintln!("[docker] killing container {}", definition.id);
        let opts: Option<KillContainerOptions<String>> = Some(KillContainerOptions {
            signal: "SIGKILL".to_string(),
        });
        if let Err(err) = self.client.kill_container(&definition.id, opts).await {
            if !is_not_found(&err) {
                return Err(ContainerError::Docker {
                    operation: "kill_container",
                    message: format!("failed to kill container {}: {err}", definition.id),
                });
            }
        }
        Ok(())
    }

    /// Remove both the container and its scratch image. `NotFound` is treated as
    /// success on each step so the call remains idempotent.
    pub async fn delete(&self, definition: &ContainerDefinition) -> Result<()> {
        eprintln!("[docker] removing container {}", definition.id);
        let opts: Option<RemoveContainerOptions> = Some(RemoveContainerOptions {
            force: true,
            v: true,
            link: false,
        });
        if let Err(err) = self
            .client
            .remove_container(&definition.id, opts)
            .await
        {
            if !is_not_found(&err) {
                return Err(ContainerError::Docker {
                    operation: "remove_container",
                    message: format!("failed to remove container {}: {err}", definition.id),
                });
            }
        }
        eprintln!("[docker] removing image {}", definition.image);
        let opts: Option<RemoveImageOptions> = Some(RemoveImageOptions {
            force: true,
            noprune: false,
        });
        if let Err(err) = self
            .client
            .remove_image(&definition.image, opts, None)
            .await
        {
            if !is_not_found(&err) {
                return Err(ContainerError::Docker {
                    operation: "remove_image",
                    message: format!("failed to remove image {}: {err}", definition.image),
                });
            }
        }
        Ok(())
    }

    /// Stream container logs as decoded frames.
    pub async fn logs(
        &self,
        container_id: &str,
    ) -> Result<Box<dyn Stream<Item = Result<LogFrame>> + Unpin + Send>> {
        let opts: LogsOptions<String> = LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            since: 0,
            until: 0,
            timestamps: false,
            tail: "all".to_string(),
        };
        let stream = self
            .client
            .logs(container_id, Some(opts))
            .map_err(|err| ContainerError::Docker {
                operation: "logs",
                message: format!("failed to stream logs for {container_id}: {err}"),
            });
        Ok(Box::new(FrameStream {
            inner: Box::pin(stream),
        }))
    }

    async fn image_exists(&self, name: &str) -> Result<bool> {
        match self.client.inspect_image(name).await {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            }) => Ok(false),
            Err(err) => Err(ContainerError::Docker {
                operation: "inspect_image",
                message: format!("failed to inspect {name}: {err}"),
            }),
        }
    }

    /// Import a rootfs tar into the daemon, optionally tagged with `tag`.
    ///
    /// Uses `POST /images/create?fromSrc=-` (`create_image`) which accepts a
    /// tarball as the request body. The tar must contain a rootfs (no manifest).
    async fn import_tar(&self, tar_path: &Path, tag: Option<&str>) -> Result<()> {
        let bytes = tokio::fs::read(tar_path).await.map_err(|err| ContainerError::Image {
            operation: "read_tar",
            message: format!("failed to read {}: {err}", tar_path.display()),
        })?;
        let opts: CreateImageOptions<'_, String> = CreateImageOptions {
            from_image: String::new(),
            from_src: "-".to_string(),
            repo: tag.unwrap_or("").to_string(),
            tag: "latest".to_string(),
            platform: String::new(),
            changes: Vec::new(),
        };

        let body: Option<Bytes> = Some(Bytes::from(bytes));
        let mut stream = self.client.create_image(Some(opts), body, None);
        while let Some(item) = stream.try_next().await.map_err(|err| {
            ContainerError::Docker {
                operation: "create_image",
                message: format!("image import failed: {err}"),
            }
        })? {
            if let Some(error) = item.error {
                return Err(ContainerError::Docker {
                    operation: "create_image",
                    message: error,
                });
            }
            if let Some(stream) = item.stream {
                eprintln!("[docker] import: {stream}");
            }
        }
        Ok(())
    }

    async fn create_and_start(
        &self,
        container_id: &str,
        image: &str,
        cmd: &[String],
        env: &[String],
        working_dir: &str,
        network_mode: &str,
        limits: &ResourceLimits,
    ) -> Result<()> {
        let host_config =
            build_host_config(network_mode, limits, &self.runtime_root, container_id);
        let labels: std::collections::HashMap<String, String> = [
            (SMOOTHIE_LABEL.to_string(), "true".to_string()),
        ]
        .into_iter()
        .collect();
        let config = ContainerConfig::<String> {
            image: Some(image.to_string()),
            cmd: Some(cmd.to_vec()),
            env: Some(env.to_vec()),
            working_dir: Some(working_dir.to_string()),
            host_config: Some(host_config),
            labels: Some(labels),
            attach_stdout: Some(false),
            attach_stderr: Some(false),
            tty: Some(false),
            ..Default::default()
        };
        let opts: CreateContainerOptions<String> = CreateContainerOptions {
            name: container_id.to_string(),
            platform: None,
        };

        eprintln!("[docker] creating container {container_id} from image {image}");
        self.client
            .create_container(Some(opts), config)
            .await
            .map_err(|err| ContainerError::Docker {
                operation: "create_container",
                message: format!("failed to create container {container_id}: {err}"),
            })?;

        eprintln!("[docker] starting container {container_id}");
        let start_opts: Option<StartContainerOptions<String>> = Some(StartContainerOptions {
            detach_keys: String::new(),
        });
        if let Err(err) = self
            .client
            .start_container(container_id, start_opts)
            .await
        {
            // Try to clean up the half-created container before reporting.
            let cleanup_opts: Option<RemoveContainerOptions> = Some(RemoveContainerOptions {
                force: true,
                v: true,
                link: false,
            });
            let _ = self
                .client
                .remove_container(container_id, cleanup_opts)
                .await;
            return Err(ContainerError::Docker {
                operation: "start_container",
                message: format!("failed to start container {container_id}: {err}"),
            });
        }
        Ok(())
    }
}

fn is_not_found(err: &bollard::errors::Error) -> bool {
    matches!(
        err,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

fn build_host_config(
    network_mode: &str,
    limits: &ResourceLimits,
    runtime_root: &Path,
    container_id: &str,
) -> HostConfig {
    let resources = Resources {
        memory: limits.memory_bytes.map(|v| v as i64),
        memory_swap: limits.memory_swap_bytes.map(|v| v as i64),
        cpu_period: limits.cpu_period_us.map(|v| v as i64),
        cpu_quota: limits.cpu_quota_us.map(|v| v as i64),
        cpu_shares: limits.cpu_weight.map(|v| v as i64),
        pids_limit: limits.pids.map(|v| v as i64),
        ..Default::default()
    };

    let log_driver = HostConfigLogConfig {
        typ: Some("json-file".to_string()),
        config: Some(std::collections::HashMap::from([(
            "max-size".to_string(),
            "10m".to_string(),
        )])),
    };

    let logs_dir = runtime_root.join("logs").join(container_id);
    let mut binds = Vec::new();
    if let Err(err) = std::fs::create_dir_all(&logs_dir) {
        eprintln!(
            "[docker] warning: failed to create {}: {err}",
            logs_dir.display()
        );
    } else {
        binds.push(Mount {
            target: Some(logs_dir.to_string_lossy().to_string()),
            source: Some(logs_dir.to_string_lossy().to_string()),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        });
    }

    HostConfig {
        network_mode: Some(network_mode.to_string()),
        resources: Some(resources),
        log_config: Some(log_driver),
        mounts: Some(binds),
        device_requests: Some(vec![DeviceRequest {
            driver: Some("".to_string()),
            count: Some(-1),
            device_ids: Some(vec![]),
            capabilities: Some(vec![vec!["CAP_SYS_ADMIN".to_string()]]),
            options: Some(std::collections::HashMap::new()),
        }]),
        ..Default::default()
    }
}

fn validate_request(request: &ContainerRequest) -> Result<()> {
    if request.id.is_empty()
        || !request
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(ContainerError::InvalidId);
    }
    if request.argv.is_empty() {
        return Err(ContainerError::EmptyCommand);
    }
    Ok(())
}

fn append_proxy_env(env: &mut Vec<String>, proxy: &ProxyConfig) {
    for (name, value) in [
        ("HTTP_PROXY", &proxy.http),
        ("HTTPS_PROXY", &proxy.https),
        ("ALL_PROXY", &proxy.all),
    ] {
        if let Some(value) = value {
            env.push(format!("{name}={value}"));
        }
    }
    if !proxy.no_proxy.is_empty() {
        env.push(format!("NO_PROXY={}", proxy.no_proxy.join(",")));
    }
}

#[derive(Debug, Clone)]
pub struct LogFrame {
    pub stream: LogStream,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
    #[allow(dead_code)]
    Stdin,
}

struct FrameStream {
    inner: Pin<
        Box<
            dyn Stream<
                    Item = std::result::Result<
                        bollard::container::LogOutput,
                        bollard::errors::Error,
                    >,
                > + Send,
        >,
    >,
}

impl Stream for FrameStream {
    type Item = Result<LogFrame>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(ContainerError::Docker {
                operation: "logs",
                message: err.to_string(),
            }))),
            Poll::Ready(Some(Ok(output))) => {
                let stream = match output {
                    bollard::container::LogOutput::StdOut { .. } => LogStream::Stdout,
                    bollard::container::LogOutput::StdErr { .. } => LogStream::Stderr,
                    bollard::container::LogOutput::StdIn { .. } => LogStream::Stdin,
                    bollard::container::LogOutput::Console { .. } => LogStream::Stdout,
                };
                let payload = output.into_bytes().to_vec();
                Poll::Ready(Some(Ok(LogFrame { stream, payload })))
            }
        }
    }
}

/// Build a deterministic tar archive of `root` and return the path to the
/// resulting file. The path lives under `runtime_root/imports` so that callers
/// can clean it up together with the rest of the runtime state.
async fn tar_dir(root: &Path) -> Result<PathBuf> {
    let imports_dir = root
        .parent()
        .map(|p| p.join("runtime").join("imports"))
        .unwrap_or_else(|| PathBuf::from("./imports"));
    std::fs::create_dir_all(&imports_dir).map_err(|err| ContainerError::Image {
        operation: "create_dir",
        message: format!(
            "failed to create imports dir {}: {err}",
            imports_dir.display()
        ),
    })?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let target = imports_dir.join(format!(
        "{}-{}.tar",
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("rootfs"),
        stamp
    ));

    let root = root.to_path_buf();
    let target_clone = target.clone();
    tokio::task::spawn_blocking(move || -> std::result::Result<(), std::io::Error> {
        let file = std::fs::File::create(&target_clone)?;
        let mut builder = tar::Builder::new(file);
        builder.follow_symlinks(false);
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            let name = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            builder.append_dir_all(name, &path)?;
        }
        builder.into_inner()?.sync_all()?;
        Ok(())
    })
    .await
    .map_err(|err| ContainerError::Image {
        operation: "join_tar",
        message: format!("tar build join error: {err}"),
    })?
    .map_err(|err| ContainerError::Image {
        operation: "build_tar",
        message: format!("failed to build tar: {err}"),
    })?;
    Ok(target)
}

#[allow(dead_code)]
fn _ensure_network_in_path() {
    // Reserve a use of the import to silence the "unused import" warning when
    // no other module references `NetworkMode`.
    let _ = std::mem::size_of::<NetworkMode>();
}
