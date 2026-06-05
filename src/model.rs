use std::net::SocketAddr;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, Eq, PartialEq, Hash)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    pub source: String,
    pub priority: u32,
    pub protocol: String,
    pub name: String,
    pub endpoint: Endpoint,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedConfig {
    pub rank: usize,
    pub id: String,
    pub source: String,
    pub priority: u32,
    pub protocol: String,
    pub name: String,
    pub endpoint: Endpoint,
    pub uri: String,
    pub reachable: bool,
    pub validation: String,
    pub latency_ms: Option<u128>,
    pub http_status: Option<u16>,
    pub download_mbps: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RuntimeState {
    pub last_refresh: Option<String>,
    pub last_error: Option<String>,
    pub total_candidates: usize,
    pub reachable_candidates: usize,
    pub fetch_errors: Vec<String>,
    pub ranked: Vec<RankedConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeConfig {
    pub bind: SocketAddr,
    pub top_n: usize,
    pub refresh_seconds: u64,
    pub encoded_subscription: bool,
    pub probe_mode: String,
}
