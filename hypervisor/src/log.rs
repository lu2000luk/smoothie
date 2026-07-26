use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags, UnixAddr};
use tokio::io::unix::AsyncFd;
use tokio::task::JoinHandle;

use crate::globals;

pub struct LogForwarder {
    task: JoinHandle<()>,
    socket_path: PathBuf,
}

impl LogForwarder {
    pub async fn start(container_id: String, _bundle: &Path) -> Result<Self, io::Error> {
        let socket_path = crate::globals::console_socket_path(&container_id);
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(&socket_path);
        eprintln!("[log] starting log forwarder for {container_id}");

        let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;

        let id = container_id.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = forward_logs(listener, id.clone()).await {
                eprintln!("log forwarder error for {}: {}", id, e);
            }
        });

        Ok(Self { task, socket_path })
    }

    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Drop for LogForwarder {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn forward_logs(
    listener: std::os::unix::net::UnixListener,
    container_id: String,
) -> Result<(), io::Error> {
    let async_listener = AsyncFd::new(listener)?;

    let stream = tokio::time::timeout(Duration::from_secs(30), accept_connection(&async_listener))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timeout waiting for console"))?
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let master_fd = receive_fd(stream)?;

    fcntl(master_fd.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let async_fd = AsyncFd::new(master_fd)?;

    let client = globals::REDIS_CLIENT
        .get()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Redis client not set"))?;
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut buf = [0u8; 4096];
    let mut line_buf = String::new();

    loop {
        let n = read_pty(&async_fd, &mut buf).await?;
        if n == 0 {
            break;
        }

        line_buf.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(pos) = line_buf.find('\n') {
            let line = line_buf[..pos].to_string();
            line_buf.drain(..=pos);
            if !line.is_empty() {
                let payload = format!("{}$info$stdout: {}", timestamp(), line);
                let _: Result<(), _> = redis::cmd("XADD")
                    .arg(format!("logs:{}", container_id))
                    .arg("*")
                    .arg("msg")
                    .arg(&payload)
                    .query_async(&mut conn)
                    .await;
            }
        }
    }

    if !line_buf.is_empty() {
        let payload = format!("{}$info$stdout: {}", timestamp(), line_buf);
        let _: Result<(), _> = redis::cmd("XADD")
            .arg(format!("logs:{}", container_id))
            .arg("*")
            .arg("msg")
            .arg(&payload)
            .query_async(&mut conn)
            .await;
    }

    Ok(())
}

async fn accept_connection(
    listener: &AsyncFd<std::os::unix::net::UnixListener>,
) -> Result<std::os::unix::net::UnixStream, io::Error> {
    loop {
        let mut guard = listener
            .readable()
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        match guard.try_io(|inner| {
            let (stream, _) = inner.get_ref().accept()?;
            Ok(stream)
        }) {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => return Err(e),
            Err(_WouldBlock) => continue,
        }
    }
}

fn receive_fd(mut stream: std::os::unix::net::UnixStream) -> Result<OwnedFd, io::Error> {
    stream.set_nonblocking(false)?;

    let mut buf = [0u8; 1];
    let mut iov = [std::io::IoSliceMut::new(&mut buf)];
    let mut cmsg_buffer = Vec::with_capacity(128);

    let fd = stream.as_raw_fd();
    let msg = recvmsg::<UnixAddr>(
        fd,
        &mut iov,
        Some(&mut cmsg_buffer),
        MsgFlags::MSG_CMSG_CLOEXEC,
    )
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let cmsgs = msg.cmsgs().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    for cmsg in cmsgs {
        if let ControlMessageOwned::ScmRights(fds) = cmsg {
            if let Some(&raw_fd) = fds.first() {
                return Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) });
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no fd received via SCM_RIGHTS",
    ))
}

async fn read_pty(fd: &AsyncFd<OwnedFd>, buf: &mut [u8]) -> Result<usize, io::Error> {
    loop {
        let mut guard = fd.readable().await?;
        match guard.try_io(|inner| {
            let n = unsafe {
                libc::read(
                    inner.get_ref().as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len() as libc::size_t,
                )
            };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(n as usize)
        }) {
            Ok(Ok(n)) => return Ok(n),
            Ok(Err(e)) => return Err(e),
            Err(_WouldBlock) => continue,
        }
    }
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
