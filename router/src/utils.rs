use rand::{distr::{Distribution, weighted::WeightedIndex}, seq::index};

use crate::ServerConfig;

pub fn get_server(servers: &Vec<ServerConfig>) -> Option<ServerConfig> {
    if servers.is_empty() {
        return None;
    }

     let ids = servers.iter().map(|s| s.id.clone()).collect::<Vec<String>>();
     let powers = servers.iter().map(|s| s.power).collect::<Vec<u32>>();

    let dist = WeightedIndex::new(&powers).ok()?;
    let mut rng = rand::rng();
    let index = dist.sample(&mut rng);

    let server = servers.get(index).cloned();
    
server
}