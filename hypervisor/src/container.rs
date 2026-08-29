use std::{
    collections::{BTreeMap, VecDeque},
    fmt, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use flate2::read::GzDecoder;
use futures::StreamExt;
use tar::Archive;

use uuid::Uuid;

use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    sync::Mutex,
    task::JoinHandle,
};

pub type Result<T> = std::result::Result<T, ContainerError>;

#[derive(Debug)]
pub enum ContainerError {
    InvalidId,
    EmptyCommand,
    InteriorNul(&'static str),
    NonUtf8Path(&'static str),
    LimitTooLarge(&'static str),
    Package(String),
    Image(String),
    Io(io::Error),
    IdlePoolExhausted,
    Docker { operation: &'static str, message: String },
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId => write!(
                f,
                "container IDs may contain only alphanumerics, '.', '_' and '-'"
            )?,
            Self::EmptyCommand => write!(f, "an OCI process needs at least one argument")?,
            Self::InteriorNul(field) => write!(f, "{field} contains an interior NUL byte")?,
            Self::NonUtf8Path(field) => write!(f, "{field} is not valid UTF-8")?,
            Self::LimitTooLarge(field) => write!(f, "{field} exceeds the resource limit range")?,
            Self::Package(err) => write!(f, "failed to load package: {err}")?,
            Self::Image(err) => write!(f, "failed to prepare image: {err}")?,
            Self::Io(err) => write!(f, "network I/O failed: {err}")?,
            Self::IdlePoolExhausted => write!(f, "no idle containers are available")?,
            Self::Docker { operation, message } => {
                write!(f, "docker {operation} failed: {message}")?
            }
        }
        Ok(())
    }
}

impl std::error::Error for ContainerError {}

impl From<io::Error> for ContainerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceLimits {
    pub memory_bytes: Option<u64>,
    pub memory_swap_bytes: Option<u64>,
    pub cpu_period_us: Option<u64>,
    pub cpu_quota_us: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub pids: Option<u64>,
    pub unified: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkMode {
    Isolated,
    Host,
    ExistingNamespace(PathBuf),
}

impl Default for NetworkMode {
    fn default() -> Self {
        Self::Isolated
    }
}

impl NetworkMode {
    pub fn docker_value(&self) -> &'static str {
        match self {
            Self::Isolated => "bridge",
            Self::Host | Self::ExistingNamespace(_) => "host",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsConfig {
    Inherit,
    ResolvConf(PathBuf),
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self::Inherit
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    pub http: Option<String>,
    pub https: Option<String>,
    pub all: Option<String>,
    pub no_proxy: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Architecture {
    Native,
    Qemu { emulator: PathBuf, guest: String },
}

impl Default for Architecture {
    fn default() -> Self {
        Self::Native
    }
}

#[derive(Clone, Debug)]
pub struct ContainerRequest {
    pub id: String,
    pub bundle: PathBuf,
    pub rootfs: PathBuf,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub hostname: Option<String>,
    pub readonly_rootfs: bool,
    pub limits: ResourceLimits,
    pub network: NetworkMode,
    pub dns: DnsConfig,
    pub proxy: ProxyConfig,
    pub architecture: Architecture,
    pub annotations: BTreeMap<String, String>,
    pub package_tarball: Option<PathBuf>,
}

impl ContainerRequest {
    pub fn new(
        id: impl Into<String>,
        bundle: impl Into<PathBuf>,
        rootfs: impl Into<PathBuf>,
        argv: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            bundle: bundle.into(),
            rootfs: rootfs.into(),
            argv,
            env: BTreeMap::new(),
            cwd: PathBuf::from("/"),
            hostname: None,
            readonly_rootfs: true,
            limits: ResourceLimits::default(),
            network: NetworkMode::default(),
            dns: DnsConfig::default(),
            proxy: ProxyConfig::default(),
            architecture: Architecture::default(),
            annotations: BTreeMap::new(),
            package_tarball: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IdleContainer {
    pub id: String,
    pub bundle: PathBuf,
    pub rootfs: PathBuf,
    pub upper: PathBuf,
    pub work: PathBuf,
}

#[derive(Debug)]
pub struct IdleContainerPool {
    available: Mutex<VecDeque<IdleContainer>>,
    base_rootfs: PathBuf,
}

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

#[derive(Clone)]
pub struct ContainerApi {
    runtime: Arc<crate::docker::DockerRuntime>,
    idle: Arc<IdleContainerPool>,
}

pub struct SelectedTarball(PathBuf);

pub struct InjectedContainer(ContainerRequest);

impl InjectedContainer {
    pub fn id(&self) -> &str {
        &self.0.id
    }
    pub fn package_tarball(&self) -> Option<&Path> {
        self.0.package_tarball.as_deref()
    }
}

pub struct RunningContainer {
    pub(crate) definition: ContainerDefinition,
    port: PortMapping,
    task: JoinHandle<Result<()>>,
    pub(crate) log_forwarder: crate::log::LogForwarder,
}

impl RunningContainer {
    pub fn id(&self) -> &str {
        &self.definition.id
    }
    pub fn host_port(&self) -> u16 {
        self.port.host_port()
    }
    pub fn container_port(&self) -> u16 {
        self.port.container_port()
    }
    pub async fn wait(self) -> Result<()> {
        self.task
            .await
            .map_err(|e| ContainerError::Image(e.to_string()))?
    }
}

impl ContainerApi {
    pub fn new(runtime: Arc<crate::docker::DockerRuntime>, idle: Arc<IdleContainerPool>) -> Self {
        Self { runtime, idle }
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
        eprintln!(
            "[container] inject: tarball={} argv={:?}",
            tarball.0.display(),
            argv
        );
        let slot = self
            .idle
            .available
            .lock()
            .await
            .pop_front()
            .ok_or(ContainerError::IdlePoolExhausted)?;
        eprintln!(
            "[container] inject: got idle slot id={} extracting package into upper layer...",
            slot.id
        );
        if let Err(error) = extract_tar(&tarball.0, &slot.upper, false).await {
            eprintln!(
                "[container] inject: id={} extract failed: {}",
                slot.id, error
            );
            self.idle.available.lock().await.push_front(slot);
            return Err(error);
        }
        eprintln!("[container] inject: id={} extract done", slot.id);
        let mut request = ContainerRequest::new(slot.id, slot.bundle, slot.rootfs, argv);
        request.network = NetworkMode::Host;
        request.package_tarball = Some(tarball.0);
        Ok(InjectedContainer(request))
    }

    pub async fn run(
        &self,
        injected: InjectedContainer,
        container_port: u16,
    ) -> Result<RunningContainer> {
        eprintln!(
            "[container] run: id={} container_port={}",
            injected.0.id, container_port
        );
        let port = PortMapping::publish(container_port).await?;
        eprintln!(
            "[container] run: id={} host_port={}",
            injected.0.id,
            port.host_port()
        );

        let definition = self.runtime.prepare(injected.0).await?;
        eprintln!("[container] run: id={} docker spec prepared", definition.id);

        let log_forwarder = crate::log::LogForwarder::start(definition.id.clone())
            .await
            .map_err(ContainerError::Io)?;
        eprintln!(
            "[container] run: id={} log forwarder started",
            definition.id
        );

        let runtime = Arc::clone(&self.runtime);
        let run_definition = definition.clone();
        let task = tokio::spawn(async move {
            runtime.start(&run_definition).await
        });

        eprintln!("[container] run: id={} spawned", definition.id);
        Ok(RunningContainer {
            definition,
            port,
            task,
            log_forwarder,
        })
    }

    pub async fn kill(&self, running: RunningContainer) -> Result<()> {
        eprintln!("[container] kill: id={}", running.definition.id);
        let RunningContainer {
            definition,
            port: _,
            task,
            log_forwarder,
        } = running;
        log_forwarder.abort();
        task.abort();
        eprintln!(
            "[container] kill: id={} sending SIGKILL via Docker",
            definition.id
        );
        if let Err(e) = self.runtime.kill(&definition).await {
            eprintln!(
                "[container] kill: id={} docker kill error: {}",
                definition.id, e
            );
        }
        eprintln!("[container] kill: id={} deleting container", definition.id);
        let result = self.runtime.delete(&definition).await;
        if let Err(ref e) = result {
            eprintln!(
                "[container] kill: id={} delete failed: {}",
                definition.id, e
            );
        } else {
            eprintln!("[container] kill: id={} done", definition.id);
        }
        result
    }
}

impl IdleContainerPool {
    pub async fn new(
        root: impl Into<PathBuf>,
        base_rootfs: impl Into<PathBuf>,
        count: u32,
    ) -> Result<Self> {
        let root = root.into();
        let base_rootfs = base_rootfs.into();
        let mut available = VecDeque::with_capacity(count as usize);
        for x in 0..count {
            eprintln!("[idle-pool] preparing container {x}/{count}...");
            let id = Uuid::new_v4().to_string();
            let slot_root = root.join(&id);
            let upper = slot_root.join("upper");
            let work = slot_root.join("work");
            let rootfs = slot_root.join("rootfs");
            tokio::fs::create_dir_all(&upper)
                .await
                .map_err(|e| ContainerError::Image(e.to_string()))?;
            tokio::fs::create_dir_all(&work)
                .await
                .map_err(|e| ContainerError::Image(e.to_string()))?;
            tokio::fs::create_dir_all(&rootfs)
                .await
                .map_err(|e| ContainerError::Image(e.to_string()))?;
            mount_overlay(&base_rootfs, &upper, &work, &rootfs)?;
            let slot = IdleContainer {
                id: id.clone(),
                bundle: root.join(&id),
                rootfs,
                upper,
                work,
            };
            available.push_back(slot);
        }
        Ok(Self {
            available: Mutex::new(available),
            base_rootfs,
        })
    }

    pub async fn initialize_request(
        &self,
        package_id: &str,
        argv: Vec<String>,
    ) -> Result<ContainerRequest> {
        let package_tarball = crate::package::get_package(package_id)
            .await
            .map_err(|error| ContainerError::Package(error.to_string()))?;
        let slot = self
            .available
            .lock()
            .await
            .pop_front()
            .ok_or(ContainerError::IdlePoolExhausted)?;
        if let Err(error) = extract_tar(&package_tarball, &slot.upper, false).await {
            self.available.lock().await.push_front(slot);
            return Err(error);
        }
        let mut request = ContainerRequest::new(slot.id, slot.bundle, slot.rootfs, argv);
        request.package_tarball = Some(package_tarball);
        request.annotations.insert(
            "io.smoothie.package.tarball".into(),
            request
                .package_tarball
                .as_ref()
                .expect("set")
                .display()
                .to_string(),
        );
        Ok(request)
    }

    pub async fn release(&self, request: ContainerRequest) {
        let slot = IdleContainer {
            id: request.id,
            bundle: request.bundle.clone(),
            rootfs: request.rootfs,
            upper: request.bundle.join("upper"),
            work: request.bundle.join("work"),
        };
        let _ = umount_overlay(&slot.rootfs);
        let _ = tokio::fs::remove_dir_all(&slot.upper).await;
        let _ = tokio::fs::remove_dir_all(&slot.work).await;
        if tokio::fs::create_dir_all(&slot.upper).await.is_err()
            || tokio::fs::create_dir_all(&slot.work).await.is_err()
            || tokio::fs::create_dir_all(&slot.rootfs).await.is_err()
            || mount_overlay(&self.base_rootfs, &slot.upper, &slot.work, &slot.rootfs).is_err()
        {
            return;
        }
        self.available.lock().await.push_back(slot);
    }

    pub async fn list_ids(&self) -> Vec<String> {
        self.available
            .lock()
            .await
            .iter()
            .map(|c| c.id.clone())
            .collect()
    }

    pub async fn available(&self) -> usize {
        self.available.lock().await.len()
    }
}

async fn extract_tar(archive: &Path, dest: &Path, _gzip: bool) -> Result<()> {
    eprintln!(
        "[container] extract_tar: archive={} dest={}",
        archive.display(),
        dest.display()
    );
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive)
            .map_err(|e| ContainerError::Image(format!("failed to open archive: {e}")))?;
        if archive.extension().and_then(|e| e.to_str()) == Some("gz") {
            let gz = GzDecoder::new(file);
            let mut ar = Archive::new(gz);
            ar.unpack(&dest)
                .map_err(|e| ContainerError::Image(format!("tar extract failed: {e}")))?;
        } else {
            let mut ar = Archive::new(file);
            ar.unpack(&dest)
                .map_err(|e| ContainerError::Image(format!("tar extract failed: {e}")))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ContainerError::Image(e.to_string()))?
}

#[cfg(unix)]
fn mount_overlay(lower: &Path, upper: &Path, work: &Path, merged: &Path) -> Result<()> {
    eprintln!(
        "[container] overlayfs: lower={} upper={} work={} merged={}",
        lower.display(),
        upper.display(),
        work.display(),
        merged.display()
    );
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        lower.display(),
        upper.display(),
        work.display()
    );
    let c_opts = match std::ffi::CString::new(opts) {
        Ok(v) => v,
        Err(_) => return Err(ContainerError::InteriorNul("overlayfs opts")),
    };
    let c_merged = cstring_path(merged, "overlayfs merged")?;
    let c_type = match std::ffi::CString::new("overlay") {
        Ok(v) => v,
        Err(_) => return Err(ContainerError::InteriorNul("overlayfs type")),
    };
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            c_merged.as_ptr().cast(),
            c_type.as_ptr().cast(),
            0,
            c_opts.as_ptr().cast(),
        )
    };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        return Err(ContainerError::Image(format!(
            "overlayfs mount failed: {err}"
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn mount_overlay(_lower: &Path, _upper: &Path, _work: &Path, _merged: &Path) -> Result<()> {
    Err(ContainerError::Image(
        "overlayfs is only supported on Linux".into(),
    ))
}

#[cfg(unix)]
fn umount_overlay(merged: &Path) -> Result<()> {
    eprintln!("[container] umount: merged={}", merged.display());
    let c_merged = cstring_path(merged, "umount path")?;
    let ret = unsafe { libc::umount2(c_merged.as_ptr(), libc::MNT_DETACH) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        return Err(ContainerError::Image(format!("umount failed: {err}")));
    }
    Ok(())
}

#[cfg(not(unix))]
fn umount_overlay(_merged: &Path) -> Result<()> {
    Err(ContainerError::Image(
        "umount is only supported on Linux".into(),
    ))
}

#[derive(Clone, Debug)]
pub struct ContainerDefinition {
    pub id: String,
    pub image: String,
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub cwd: PathBuf,
    pub network: NetworkMode,
    pub limits: ResourceLimits,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn resource_limits_validate_quantities() {
        let mut request = ContainerRequest::new(
            "api-1",
            "/bundles/api-1",
            "/images/alpine",
            vec!["/bin/echo".into(), "ok".into()],
        );
        request.limits.memory_bytes = Some(26_214_400);
        request.limits.cpu_period_us = Some(50_000);
        request.limits.cpu_quota_us = Some(12_500);
        assert_eq!(request.limits.memory_bytes, Some(26_214_400));
        assert_eq!(request.limits.cpu_quota_us, Some(12_500));
    }

    #[test]
    fn network_mode_maps_to_docker_value() {
        assert_eq!(NetworkMode::Host.docker_value(), "host");
        assert_eq!(NetworkMode::Isolated.docker_value(), "bridge");
        assert_eq!(
            NetworkMode::ExistingNamespace(PathBuf::from("/run/netns/foo")).docker_value(),
            "host"
        );
    }

    #[tokio::test]
    async fn published_host_port_relays_to_a_different_container_port() {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_port = target_listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = target_listener.accept().await.unwrap();
            let mut request = [0; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let mapping = PortMapping::publish(target_port).await.unwrap();
        assert_ne!(mapping.host_port(), mapping.container_port());
        let mut client = TcpStream::connect(("127.0.0.1", mapping.host_port()))
            .await
            .unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut response = [0; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        server.await.unwrap();
    }
}
