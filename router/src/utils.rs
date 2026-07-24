use rand::{
    distr::{Distribution, weighted::WeightedIndex},
    seq::index,
};

use crate::ServerConfig;

// server ranker system (to be done)
// based on scores
// each server has a score of 100
// each request the server served decrease score by 1
// if the server served a request from this host in last 24hrs boost score by 500 (means app is cached in this server)
// pick highest score server to serve the request

pub fn get_server(servers: &Vec<ServerConfig>) -> Option<ServerConfig> {
    if servers.is_empty() {
        return None;
    }

    let ids = servers
        .iter()
        .map(|s| s.id.clone())
        .collect::<Vec<String>>();
    let powers = servers.iter().map(|s| s.power).collect::<Vec<u32>>();

    let dist = WeightedIndex::new(&powers).ok()?;
    let mut rng = rand::rng();
    let index = dist.sample(&mut rng);

    let server = servers.get(index).cloned();

    server
}
