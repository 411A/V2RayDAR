use std::time::Instant;

use crate::{
    constants::TUI_MAX_VISIBLE_RANKED,
    geoip::format_display_name,
    model::{RuntimeConfig, RuntimeState},
};

#[derive(Debug, Clone, Default)]
pub struct RuntimeView {
    pub refresh_duration_ms: Option<u128>,
    pub refreshing: bool,
    pub refresh_started_instant: Option<Instant>,
    pub refresh_finished_instant: Option<Instant>,
    pub total_candidates: usize,
    pub tested_candidates: usize,
    pub reachable_candidates: usize,
    pub fetch_bytes: u64,
    pub speedtest_bytes: u64,
    pub logs: Vec<String>,
    pub live_logs: Vec<String>,
    pub ranked: Vec<RankedView>,
}

#[derive(Debug, Clone)]
pub struct RankedView {
    pub rank: usize,
    pub stability_count: u32,
    pub source: String,
    pub protocol: String,
    pub display_name: String,
    pub endpoint: String,
    pub latency_ms: Option<u128>,
}

impl RuntimeView {
    pub fn from_state(runtime: &RuntimeState, config: &RuntimeConfig) -> Self {
        Self {
            refresh_duration_ms: runtime.refresh_duration_ms,
            refreshing: runtime.refreshing,
            refresh_started_instant: runtime.refresh_started_instant,
            refresh_finished_instant: runtime.refresh_finished_instant,
            total_candidates: runtime.total_candidates,
            tested_candidates: runtime.tested_candidates,
            reachable_candidates: runtime.reachable_candidates,
            fetch_bytes: runtime.fetch_bytes,
            speedtest_bytes: runtime.speedtest_bytes,
            logs: runtime.logs.clone(),
            live_logs: runtime.live_logs.clone(),
            ranked: runtime
                .ranked
                .iter()
                .filter(|item| item.reachable)
                .take(config.top_n.min(TUI_MAX_VISIBLE_RANKED))
                .map(|item| {
                    let display_name =
                        format_display_name(item.country_code.as_deref(), &item.name);
                    RankedView {
                        rank: item.rank,
                        stability_count: item.stability_count,
                        source: item.source.clone(),
                        protocol: item.protocol.clone(),
                        display_name,
                        endpoint: format!("{}:{}", item.endpoint.host, item.endpoint.port),
                        latency_ms: item.latency_ms,
                    }
                })
                .collect(),
        }
    }
}
