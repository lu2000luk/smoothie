use std::{
    cmp::Reverse,
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use crate::ServerConfig;

const BASE_SCORE: i64 = 100;
const CACHE_AFFINITY_BOOST: i64 = 500;
const CACHE_AFFINITY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
struct ServerRanker {
    served: HashMap<String, u64>,
    host_affinity: HashMap<(String, String), Instant>,
}

impl ServerRanker {
    fn rank(&mut self, servers: &[ServerConfig], host: &str) -> Vec<ServerConfig> {
        let now = Instant::now();
        self.host_affinity
            .retain(|_, last_served| now.duration_since(*last_served) < CACHE_AFFINITY_TTL);

        let mut ranked = servers.to_vec();
        ranked.sort_by_key(|server| {
            let served = self.served.get(&server.id).copied().unwrap_or_default();
            let served_penalty = i64::try_from(served).unwrap_or(i64::MAX);
            let is_cached = self
                .host_affinity
                .contains_key(&(host.to_owned(), server.id.clone()));
            let score = BASE_SCORE.saturating_sub(served_penalty)
                + if is_cached { CACHE_AFFINITY_BOOST } else { 0 };

            // Prefer higher-capacity servers and then stable IDs when scores tie.
            (Reverse(score), Reverse(server.power), server.id.clone())
        });
        ranked
    }

    fn record_request(&mut self, server_id: &str, host: &str) {
        *self.served.entry(server_id.to_owned()).or_default() += 1;
        self.host_affinity
            .insert((host.to_owned(), server_id.to_owned()), Instant::now());
    }
}

static SERVER_RANKER: LazyLock<Mutex<ServerRanker>> =
    LazyLock::new(|| Mutex::new(ServerRanker::default()));

/// Returns servers from highest to lowest score.
///
/// Every server starts at 100 points, loses one point per successful request,
/// and receives a 500-point boost when it served this host in the last 24 hours.
pub fn rank_servers(servers: &[ServerConfig], host: &str) -> Vec<ServerConfig> {
    SERVER_RANKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .rank(servers, host)
}

pub fn record_server_request(server_id: &str, host: &str) {
    SERVER_RANKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .record_request(server_id, host);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str, power: u32) -> ServerConfig {
        ServerConfig {
            id: id.into(),
            address: format!("127.0.0.1:{power}"),
            tunnel: false,
            tunnel_address: None,
            power,
        }
    }

    #[test]
    fn prefers_host_affinity_over_an_unscored_server() {
        let servers = vec![server("a", 1), server("b", 1)];
        let mut ranker = ServerRanker::default();

        ranker.record_request("b", "example.com");

        assert_eq!(ranker.rank(&servers, "example.com")[0].id, "b");
        assert_eq!(ranker.rank(&servers, "other.example")[0].id, "a");
    }

    #[test]
    fn decreases_score_for_each_served_request() {
        let servers = vec![server("a", 1), server("b", 1)];
        let mut ranker = ServerRanker::default();

        ranker.record_request("a", "first.example");

        assert_eq!(ranker.rank(&servers, "other.example")[0].id, "b");
    }

    #[test]
    fn uses_power_to_break_equal_scores() {
        let servers = vec![server("small", 1), server("large", 4)];
        let mut ranker = ServerRanker::default();

        assert_eq!(ranker.rank(&servers, "example.com")[0].id, "large");
    }
}
