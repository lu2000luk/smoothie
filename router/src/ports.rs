use std::sync::OnceLock;
use tokio::{net::TcpListener, sync::Mutex};

const PREPARE_NEXT_PORTS: i16 = 50;
const PORT_RANGE_START: i16 = 3000;
const PORT_RANGE_END: i16 = 16000;

static PORTS: std::sync::LazyLock<Mutex<Vec<i16>>> = std::sync::LazyLock::new(|| {
    Mutex::new(Vec::new())
});

async fn is_port_free(port: i16) -> bool {
    TcpListener::bind(format!("127.0.0.1:{}", port)).await.is_ok()
}

pub async fn generate_ports() {
    let mut ports = Vec::with_capacity(PREPARE_NEXT_PORTS as usize);
    let mut attempts = 0;
    let max_attempts = 200;
    print!("Ports: ");
    while ports.len() < PREPARE_NEXT_PORTS as usize && attempts < max_attempts {
        let port = rand::random_range(PORT_RANGE_START..=PORT_RANGE_END);
        if is_port_free(port).await {
            print!("{} ", port);
            ports.push(port);
        }
        attempts += 1;
    }

    print!("\n");
   let mut guard = PORTS.lock().await;
    *guard = ports;
}

pub async fn consume_port() -> Option<i16> {
    let mut guard = PORTS.lock().await;
    if !guard.is_empty() {
        return Some(guard.remove(0));
    }
    None
}

