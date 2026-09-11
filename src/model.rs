use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    time::Instant,
};

use serde::Serialize;

use crate::config::should_include_token_in_url;

#[derive(Debug, Clone, Serialize, Eq, PartialEq, Hash)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    pub dedup_key: String,
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
    pub stability_count: u32,
    pub id: String,
    pub dedup_key: String,
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
    pub download_bytes: Option<usize>,
    pub error: Option<String>,
    pub country_code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct RuntimeState {
    pub last_refresh: Option<String>,
    pub last_error: Option<String>,
    pub logs: Vec<String>,
    pub live_logs: Vec<String>,
    pub refresh_started_at: Option<String>,
    pub refresh_finished_at: Option<String>,
    #[serde(skip)]
    pub refresh_started_instant: Option<Instant>,
    #[serde(skip)]
    pub refresh_finished_instant: Option<Instant>,
    /// Explicit deadline for the next automatic refresh.
    /// Set when the refresh loop schedules its sleep, so the TUI countdown
    /// matches the actual timer instead of recomputing from `finished_at`
    /// (which drifts by proxy-switch/health-check time after `refresh_once`).
    #[serde(skip)]
    pub next_refresh_instant: Option<Instant>,
    /// Explicit deadline for the next ping cycle (same idea, ping loop).
    #[serde(skip)]
    pub next_ping_instant: Option<Instant>,
    /// Start of the current (or most recent) ping cycle.
    #[serde(skip)]
    pub last_ping_instant: Option<Instant>,
    /// Wall-time twin of the most recent ping-deadline (re)arm, RFC3339 UTC.
    /// The monotonic `next_ping_instant` never crosses serialization, so the
    /// web summary ships this instead: ping countdown = anchor + interval.
    /// Stamped everywhere the deadline is (re)armed; carried over wholesale
    /// by refresh rebuilds like every other ping field.
    pub last_ping_at: Option<String>,
    pub refresh_duration_ms: Option<u128>,
    pub refreshing: bool,
    pub pinging: bool,
    pub total_candidates: usize,
    pub tested_candidates: usize,
    pub reachable_candidates: usize,
    pub fetch_bytes: u64,
    pub speedtest_bytes: u64,
    pub fetch_errors: Vec<String>,
    pub ranked: Vec<RankedConfig>,
    pub stable_working_counts: HashMap<String, u32>,
    pub proxy_active_config: Option<String>,
    pub proxy_active_uri: Option<String>,
    pub proxy_running: bool,
    pub proxy_port: Option<u16>,
    pub proxy_discoverable: bool,
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    LiveLog(String),
    ProbeDelta {
        tested: usize,
        working: usize,
        /// Observed HTTP exchange bytes (Sub Usage accounting).
        bytes: u64,
    },
    RankedSnapshot(Vec<RankedConfig>),
    WorkingConfigsFound {
        configs: Vec<RankedConfig>,
        top_n: usize,
    },
    FetchedDelta(usize),
}

#[derive(Debug, Clone)]
pub struct ProbeStopPolicy {
    pub scan_all_configs: bool,
    pub top_n: usize,
    pub prioritize_stability: bool,
    pub return_configs_asap: bool,
    pub previous_working_keys: HashSet<String>,
    /// Working configs verified by a preempted ping cycle just before this
    /// probe started. They count toward `top_n` so a refresh that interrupts
    /// a ping only gathers the shortfall instead of re-probing everything.
    pub prefound_working: usize,
    /// Of those, the ones that were also in `previous_working_keys`: they
    /// count toward the stability quorum like freshly verified ones.
    pub prefound_previous_working: usize,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct RuntimeConfig {
    pub bind: SocketAddr,
    pub top_n: usize,
    pub refresh_seconds: u64,
    pub ping_seconds: u64,
    pub encoded_subscription: bool,
    pub prioritize_stability: bool,
    pub return_configs_asap: bool,
    pub scan_all_configs: bool,
    pub fetch_timeout_ms: u64,
    pub fetch_concurrency: usize,
    pub max_subscription_bytes: usize,
    pub sharing_enabled: bool,
    pub require_token: bool,
    pub token: String,
    pub probe_mode: String,
    pub speedtest_enabled: bool,
    pub probe_concurrency: usize,
    pub probe_batch_size: Option<usize>,
    pub active_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub test_url: String,
    pub accepted_statuses: Vec<u16>,
    pub download_bytes_limit: usize,
    pub subscription_count: usize,
    pub enabled_subscription_count: usize,
    pub proxy_enabled: bool,
    pub proxy_port: u16,
    pub proxy_discoverable: bool,
}

impl RuntimeConfig {
    pub fn subscription_url(&self, host: &str, raw: bool) -> String {
        let endpoint = if raw {
            "subscription.txt"
        } else {
            "subscription"
        };
        let mut url = format!(
            "http://{}:{}/{}",
            format_url_host(host),
            self.bind.port(),
            endpoint
        );

        if should_include_token_in_url(&self.token) {
            url.push_str("?token=");
            url.push_str(&self.token);
        }

        url
    }
}

fn format_url_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}
