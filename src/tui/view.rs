use chrono::{DateTime, Utc};

use crate::{
    constants::TUI_MAX_VISIBLE_RANKED,
    model::{RuntimeConfig, RuntimeState},
};

#[derive(Debug, Clone, Default)]
pub struct RuntimeView {
    pub refresh_started_at: Option<DateTime<Utc>>,
    pub refresh_finished_at: Option<DateTime<Utc>>,
    pub refresh_duration_ms: Option<u128>,
    pub refreshing: bool,
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
    pub name: String,
    pub endpoint: String,
    pub latency_ms: Option<u128>,
}

impl RuntimeView {
    pub fn from_state(runtime: &RuntimeState, config: &RuntimeConfig) -> Self {
        Self {
            refresh_started_at: parse_time(runtime.refresh_started_at.as_deref()),
            refresh_finished_at: parse_time(runtime.refresh_finished_at.as_deref()),
            refresh_duration_ms: runtime.refresh_duration_ms,
            refreshing: runtime.refreshing,
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
                .map(|item| RankedView {
                    rank: item.rank,
                    stability_count: item.stability_count,
                    source: item.source.clone(),
                    protocol: item.protocol.clone(),
                    name: item.name.clone(),
                    endpoint: format!("{}:{}", item.endpoint.host, item.endpoint.port),
                    latency_ms: item.latency_ms,
                })
                .collect(),
        }
    }
}

fn parse_time(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
