use std::{
    collections::{BTreeMap, VecDeque},
    ffi::CString,
    fmt,
    path::{Path, PathBuf},
};

use tokio::sync::Mutex;

use crun_sys::{
    crun_error_release, libcrun_container_delete, libcrun_container_free, libcrun_container_kill,
    libcrun_container_load_from_memory, libcrun_container_run, libcrun_context_s, libcrun_error_t,
};
use oci_spec::runtime::{
    Linux, LinuxCpu, LinuxMemory, LinuxNamespaceType, LinuxPids, LinuxResources, Process, Root,
    Spec,
};

pub type Result<T> = std::result::Result<T, ContainerError>;

#[derive(Debug)]
pub enum ContainerError {
    InvalidId,
    EmptyCommand,
    InteriorNul(&'static str),
    NonUtf8Path(&'static str),
    LimitTooLarge(&'static str),
    Serialize(serde_json::Error),
    Oci(String),
    Package(String),
    Image(String),
    IdlePoolExhausted,
    Crun { operation: &'static str, code: i32 },
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
            Self::LimitTooLarge(field) => write!(f, "{field} exceeds OCI's signed cgroup limit")?,
            Self::Serialize(err) => write!(f, "failed to serialize OCI config: {err}")?,
            Self::Oci(err) => write!(f, "failed to write OCI config: {err}")?,
            Self::Package(err) => write!(f, "failed to load package: {err}")?,
            Self::Image(err) => write!(f, "failed to prepare Alpine image: {err}")?,
            Self::IdlePoolExhausted => write!(f, "no idle containers are available")?,
            Self::Crun { operation, code } => {
                write!(f, "libcrun {operation} failed with status {code}")?
            }
        }
        Ok(())
    }
}

impl std::error::Error for ContainerError {}

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
        return Self::Isolated;
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
}

#[derive(Debug)]
pub struct IdleContainerPool {
    available: Mutex<VecDeque<IdleContainer>>,
    base_archive: PathBuf,
}

impl IdleContainerPool {
    pub async fn new(
        root: impl Into<PathBuf>,
        base_archive: impl Into<PathBuf>,
        count: u32,
    ) -> Result<Self> {
        let root = root.into();
        let base_archive = base_archive.into();
        let mut available = VecDeque::with_capacity(count as usize);
        for index in 0..count {
            let slot = IdleContainer {
                id: format!("idle-{index}"),
                bundle: root.join(format!("idle-{index}")),
                rootfs: root.join(format!("idle-{index}")).join("rootfs"),
            };
            unpack(&base_archive, &slot.rootfs, true).await?;
            available.push_back(slot);
        }
        Ok(Self {
            available: Mutex::new(available),
            base_archive,
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
        if let Err(error) = unpack(&package_tarball, &slot.rootfs, false).await {
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
            bundle: request.bundle,
            rootfs: request.rootfs,
        };
        if unpack(&self.base_archive, &slot.rootfs, true).await.is_ok() {
            self.available.lock().await.push_back(slot);
        }
    }

    pub async fn available(&self) -> usize {
        self.available.lock().await.len()
    }
}

async fn unpack(archive: &Path, rootfs: &Path, gzip: bool) -> Result<()> {
    if rootfs.exists() {
        tokio::fs::remove_dir_all(rootfs)
            .await
            .map_err(|error| ContainerError::Image(error.to_string()))?;
    }
    tokio::fs::create_dir_all(rootfs)
        .await
        .map_err(|error| ContainerError::Image(error.to_string()))?;
    let status = tokio::process::Command::new("tar")
        .arg(if gzip { "-xzf" } else { "-xf" })
        .arg(archive)
        .arg("-C")
        .arg(rootfs)
        .status()
        .await
        .map_err(|error| ContainerError::Image(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(ContainerError::Image(format!("tar exited with {status}")))
    }
}

#[derive(Clone, Debug)]
pub struct ContainerDefinition {
    pub id: String,
    pub bundle: PathBuf,
    pub spec: Spec,
}

impl ContainerDefinition {
    pub fn oci_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.spec).map_err(ContainerError::Serialize)
    }

    pub fn save(&self) -> Result<()> {
        self.spec
            .save(self.bundle.join("config.json"))
            .map_err(|err| ContainerError::Oci(err.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct CrunRuntime {
    state_root: PathBuf,
}

impl CrunRuntime {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn prepare(&self, request: ContainerRequest) -> Result<ContainerDefinition> {
        validate_request(&request)?;
        let mut process = Process::default();
        let mut argv = request.argv;
        let mut env = process.env().clone().unwrap_or_default();
        if let Architecture::Qemu { emulator, guest } = &request.architecture {
            let mut emulated = vec![emulator.display().to_string(), "--".into()];
            emulated.append(&mut argv);
            argv = emulated;
            env.push(format!("SMOOTHIE_GUEST_ARCH={guest}"));
        }
        process.set_args(Some(argv));
        process.set_cwd(request.cwd);
        env.extend(request.env.into_iter().map(|(k, v)| format!("{k}={v}")));
        append_proxy_env(&mut env, &request.proxy);
        process.set_env(Some(env));

        let mut linux = Linux::default();
        linux.set_resources(Some(to_oci_limits(&request.limits)?));
        apply_network(&mut linux, &request.network);

        let mut spec = Spec::default();
        spec.set_root(Some(Root::default()));
        spec.root_mut()
            .as_mut()
            .expect("root set")
            .set_path(request.rootfs);
        spec.root_mut()
            .as_mut()
            .expect("root set")
            .set_readonly(Some(request.readonly_rootfs));
        spec.set_process(Some(process));
        spec.set_linux(Some(linux));
        spec.set_hostname(request.hostname);
        spec.set_annotations(Some(request.annotations.into_iter().collect()));
        if let DnsConfig::ResolvConf(path) = request.dns {
            add_resolv_conf_mount(&mut spec, path);
        }
        Ok(ContainerDefinition {
            id: request.id,
            bundle: request.bundle,
            spec,
        })
    }

    pub fn run(&self, definition: &ContainerDefinition) -> Result<()> {
        definition.save()?;
        let json = definition.oci_json()?;
        let json = CString::new(json).map_err(|_| ContainerError::InteriorNul("OCI JSON"))?;
        let state_root = cstring_path(&self.state_root, "state root")?;
        let id = cstring(&definition.id, "container id")?;
        let bundle = cstring_path(&definition.bundle, "bundle path")?;
        unsafe {
            let mut err: libcrun_error_t = std::ptr::null_mut();
            let container = libcrun_container_load_from_memory(json.as_ptr(), &mut err);
            if container.is_null() {
                return Err(crun_error("load", -1, &mut err));
            }
            let mut ctx: libcrun_context_s = std::mem::zeroed();
            ctx.state_root = state_root.as_ptr();
            ctx.id = id.as_ptr();
            ctx.bundle = bundle.as_ptr();
            let status = libcrun_container_run(&mut ctx, container, 0, &mut err);
            libcrun_container_free(container);
            if status != 0 {
                return Err(crun_error("run", status, &mut err));
            }
            release_error(&mut err);
        }
        Ok(())
    }

    pub fn kill(&self, id: &str, signal: &str) -> Result<()> {
        let state_root = cstring_path(&self.state_root, "state root")?;
        let id = cstring(id, "container id")?;
        let signal = cstring(signal, "signal")?;
        unsafe {
            let mut err: libcrun_error_t = std::ptr::null_mut();
            let mut ctx: libcrun_context_s = std::mem::zeroed();
            ctx.state_root = state_root.as_ptr();
            let status = libcrun_container_kill(&mut ctx, id.as_ptr(), signal.as_ptr(), &mut err);
            if status != 0 {
                return Err(crun_error("kill", status, &mut err));
            }
            release_error(&mut err);
        }
        Ok(())
    }

    pub fn delete(&self, definition: &ContainerDefinition, force: bool) -> Result<()> {
        let json = definition.oci_json()?;
        let json = CString::new(json).map_err(|_| ContainerError::InteriorNul("OCI JSON"))?;
        let id = cstring(&definition.id, "container id")?;
        let state_root = cstring_path(&self.state_root, "state root")?;
        unsafe {
            let mut err: libcrun_error_t = std::ptr::null_mut();
            let container = libcrun_container_load_from_memory(json.as_ptr(), &mut err);
            if container.is_null() {
                return Err(crun_error("load for delete", -1, &mut err));
            }
            let mut ctx: libcrun_context_s = std::mem::zeroed();
            ctx.state_root = state_root.as_ptr();
            let status = libcrun_container_delete(
                (&mut ctx as *mut libcrun_context_s).cast(),
                container.cast(),
                id.as_ptr(),
                force,
                &mut err,
            );
            libcrun_container_free(container);
            if status != 0 {
                return Err(crun_error("delete", status, &mut err));
            }
            release_error(&mut err);
        }
        Ok(())
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

fn to_oci_limits(limits: &ResourceLimits) -> Result<LinuxResources> {
    let memory = LinuxMemory::default();
    let mut memory = memory;
    memory.set_limit(limits.memory_bytes.map(as_i64).transpose()?);
    memory.set_swap(limits.memory_swap_bytes.map(as_i64).transpose()?);
    let mut cpu = LinuxCpu::default();
    cpu.set_period(limits.cpu_period_us);
    cpu.set_quota(limits.cpu_quota_us.map(as_i64).transpose()?);
    cpu.set_shares(limits.cpu_weight);
    let mut resources = LinuxResources::default();
    resources.set_memory(Some(memory));
    resources.set_cpu(Some(cpu));
    if let Some(limit) = limits.pids {
        let mut pids = LinuxPids::default();
        pids.set_limit(as_i64(limit)?);
        resources.set_pids(Some(pids));
    }
    if !limits.unified.is_empty() {
        resources.set_unified(Some(limits.unified.clone().into_iter().collect()));
    }
    Ok(resources)
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| ContainerError::LimitTooLarge("resource limit"))
}

fn apply_network(linux: &mut Linux, network: &NetworkMode) {
    let namespaces = linux
        .namespaces_mut()
        .as_mut()
        .expect("OCI defaults namespaces");
    match network {
        NetworkMode::Isolated => {}
        NetworkMode::Host => {
            namespaces.retain(|namespace| namespace.typ() != LinuxNamespaceType::Network)
        }
        NetworkMode::ExistingNamespace(path) => {
            for namespace in namespaces {
                if namespace.typ() == LinuxNamespaceType::Network {
                    namespace.set_path(Some(path.clone()));
                }
            }
        }
    }
}

fn add_resolv_conf_mount(spec: &mut Spec, source: PathBuf) {
    let mounts = spec.mounts_mut().as_mut().expect("OCI defaults mounts");
    mounts.retain(|mount| mount.destination() != Path::new("/etc/resolv.conf"));
    let mount = oci_spec::runtime::MountBuilder::default()
        .destination("/etc/resolv.conf")
        .typ("bind")
        .source(source)
        .options(vec!["rbind".into(), "ro".into()])
        .build()
        .expect("valid resolv.conf mount");
    mounts.push(mount);
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

fn cstring(value: impl AsRef<str>, field: &'static str) -> Result<CString> {
    CString::new(value.as_ref()).map_err(|_| ContainerError::InteriorNul(field))
}
fn cstring_path(value: &Path, field: &'static str) -> Result<CString> {
    cstring(
        value.to_str().ok_or(ContainerError::NonUtf8Path(field))?,
        field,
    )
}
fn crun_error(operation: &'static str, code: i32, err: &mut libcrun_error_t) -> ContainerError {
    release_error(err);
    ContainerError::Crun { operation, code }
}
fn release_error(err: &mut libcrun_error_t) {
    unsafe {
        if !err.is_null() {
            crun_error_release(err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_generates_cgroup_and_network_oci() {
        let mut request = ContainerRequest::new(
            "api-1",
            "/bundles/api-1",
            "/images/alpine",
            vec!["/bin/echo".into(), "ok".into()],
        );
        request.limits.memory_bytes = Some(26_214_400);
        request.limits.cpu_period_us = Some(50_000);
        request.limits.cpu_quota_us = Some(12_500);
        request.network = NetworkMode::Host;
        let definition = CrunRuntime::new("/run/smoothie").prepare(request).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&definition.oci_json().unwrap()).unwrap();
        assert_eq!(value["linux"]["resources"]["memory"]["limit"], 26_214_400);
        assert_eq!(value["linux"]["resources"]["cpu"]["quota"], 12_500);
        assert!(
            !value["linux"]["namespaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["type"] == "network")
        );
    }
}
