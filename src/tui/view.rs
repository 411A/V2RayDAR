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
    pub pinging: bool,
    pub refresh_started_instant: Option<Instant>,
    pub refresh_finished_instant: Option<Instant>,
    pub next_refresh_instant: Option<Instant>,
    pub next_ping_instant: Option<Instant>,
    pub last_ping_instant: Option<Instant>,
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
    pub is_proxy: bool,
    pub uri: String,
}

/// Compare proxy URIs ignoring the `#remark` fragment.
///
/// Subscription sources rename remarks daily while the pre-`#` base
/// (scheme, credentials, host, port, query) identifies the same server.
/// Without this, every remark rename makes the 🚪 door vanish even though
/// the proxy is still running that server. Pure string split, no I/O.
#[must_use]
pub fn same_proxy_uri(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    uri_base(a) == uri_base(b)
}

fn uri_base(uri: &str) -> &str {
    uri.split('#').next().unwrap_or(uri)
}

impl RuntimeView {
    pub fn from_state(runtime: &RuntimeState, config: &RuntimeConfig) -> Self {
        let proxy_uri = runtime.proxy_active_uri.clone();
        Self {
            refresh_duration_ms: runtime.refresh_duration_ms,
            refreshing: runtime.refreshing,
            pinging: runtime.pinging,
            refresh_started_instant: runtime.refresh_started_instant,
            refresh_finished_instant: runtime.refresh_finished_instant,
            next_refresh_instant: runtime.next_refresh_instant,
            next_ping_instant: runtime.next_ping_instant,
            last_ping_instant: runtime.last_ping_instant,
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
                    let is_proxy = proxy_uri
                        .as_deref()
                        .is_some_and(|active| same_proxy_uri(active, &item.uri));
                    RankedView {
                        rank: item.rank,
                        stability_count: item.stability_count,
                        source: item.source.clone(),
                        protocol: item.protocol.clone(),
                        display_name,
                        endpoint: format!("{}:{}", item.endpoint.host, item.endpoint.port),
                        latency_ms: item.latency_ms,
                        is_proxy,
                        uri: item.uri.clone(),
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::same_proxy_uri;

    #[test]
    fn same_proxy_ignores_remark_rename() {
        let a = "vless://uuid@example.com:443?security=tls#Old%20Name";
        let b = "vless://uuid@example.com:443?security=tls#New%20Name";
        assert!(same_proxy_uri(a, b));
    }

    #[test]
    fn same_proxy_rejects_different_server() {
        let a = "vless://uuid@one.example.com:443?security=tls#Name";
        let b = "vless://uuid@two.example.com:443?security=tls#Name";
        assert!(!same_proxy_uri(a, b));
    }
}
