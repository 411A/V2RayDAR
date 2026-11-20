use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use serde::Serialize;

use crate::config::should_include_token_in_url;
use crate::constants::{BULK_MIN_BYTES, BULK_MIN_MBPS};
use crate::logarchive::LogArchive;

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
    /// `Some(false)` when the upload probe proved this config cannot carry a
    /// request body; `None` when it has not been measured.
    pub upload_ok: Option<bool>,
    pub error: Option<String>,
    pub country_code: Option<String>,
}

impl RankedConfig {
    /// Throughput class, lower is better:
    /// - `0`: measured, carried the bulk minimum, and its tail was fast
    ///   enough for real streaming/downloads;
    /// - `1`: never measured (unknown, not proven bad);
    /// - `2`: measured and either cut short or too slow, or the upload probe
    ///   proved it cannot carry a request body. A download-only shaper is
    ///   useless for forms and file uploads even when it streams well.
    ///
    /// The class comes before latency because a 40 ms handshake on a link
    /// that dies at 12 KB is worthless, while the reverse is merely slower.
    #[must_use]
    pub fn throughput_class(&self) -> u8 {
        if self.upload_ok == Some(false) {
            return 2;
        }
        match (self.download_bytes, self.download_mbps) {
            (Some(bytes), Some(mbps)) if bytes >= BULK_MIN_BYTES && mbps >= BULK_MIN_MBPS => 0,
            (None, _) => 1,
            _ => 2,
        }
    }

    /// Bulk-capable: proven to carry a real body at a usable rate.
    #[must_use]
    pub fn bulk_capable(&self) -> bool {
        self.throughput_class() == 0
    }
}

/// One ranking order for every surface: live probe results, stable
/// re-ranking, cached reloads and the proxy selector.
///
/// Order: reachable → throughput class → measured speed → stability →
/// latency → source priority. Latency stays a tie-breaker on purpose: the
/// previous releases ranked by stability and latency because that was the
/// only data they had. Once a body has been timed, the class decides first.
#[must_use]
pub fn compare_ranked(left: &RankedConfig, right: &RankedConfig) -> Ordering {
    right
        .reachable
        .cmp(&left.reachable)
        .then_with(|| left.throughput_class().cmp(&right.throughput_class()))
        .then_with(|| {
            right
                .download_mbps
                .partial_cmp(&left.download_mbps)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| right.stability_count.cmp(&left.stability_count))
        .then_with(|| {
            left.latency_ms
                .unwrap_or(u128::MAX)
                .cmp(&right.latency_ms.unwrap_or(u128::MAX))
        })
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| left.protocol.cmp(&right.protocol))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.uri.cmp(&right.uri))
}

#[derive(Debug, Clone, Default, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct RuntimeState {
    pub last_refresh: Option<String>,
    pub last_error: Option<String>,
    pub logs: Vec<String>,
    pub live_logs: Vec<String>,
    /// Monotonic count of lines ever pushed to `live_logs`. The bounded ring
    /// stops growing at its cap, so a length comparison alone would make the
    /// SSE feed stall forever once the ring is full; this counter always
    /// advances and tells the feed how many newest lines are fresh.
    pub log_seq: u64,
    /// Complete session history in compressed chunks (the live ring above is
    /// duplicated there). Skipped by serialization and cheap to clone: every
    /// `RuntimeState` snapshot shares the same archive.
    #[serde(skip)]
    pub log_archive: Arc<Mutex<LogArchive>>,
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
    /// How many of the configs tested in this cycle came from the database
    /// cache instead of a live fetch.
    ///
    /// The two counters answer different questions: `total_candidates` is
    /// what the network delivered, `cached_candidates` is what the app fell
    /// back to. Without the split the dashboard could only say "408 failed
    /// of 0 fetched", which reads like a bug instead of "it is checking the
    /// cache because the sources are down".
    pub cached_candidates: usize,
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
    /// A manual pin is stored but suspended because it failed or crawled.
    pub proxy_pin_suspended: bool,
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    LiveLog(String),
    ProbeDelta {
        tested: usize,
        working: usize,
        /// Observed HTTP exchange bytes (traffic usage accounting).
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
    /// Working configs verified before this probe started: the ones a
    /// preempted ping cycle confirmed, plus whatever this cycle's own
    /// earlier probe already verified. They count toward `top_n` so a
    /// follow-up probe (the database fallback) only gathers the shortfall
    /// instead of re-verifying the same target.
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
    pub use_cache_only: bool,
    /// Fallback share link when every subscription fails (`None` = unset).
    pub emergency_config: Option<String>,
    pub clean_offlines_after_days: u32,
    /// Custom `GeoIP` database path (`None` = built-in; restart to apply).
    pub geoip_db_path: Option<String>,
    pub sharing_enabled: bool,
    pub require_token: bool,
    pub token: String,
    pub probe_mode: String,
    pub sing_box_path: String,
    pub connect_timeout_ms: u64,
    pub speedtest_enabled: bool,
    pub probe_concurrency: usize,
    pub probe_batch_size: Option<usize>,
    pub probe_process_concurrency: Option<usize>,
    pub active_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub test_url: String,
    pub accepted_statuses: Vec<u16>,
    pub download_bytes_limit: usize,
    /// Speedtest download link (`None` = speedtest off, shown as `off`).
    pub download_url: Option<String>,
    /// 16 KB upload probe URL (`None` = upload probe off).
    pub upload_url: Option<String>,
    pub speedtest_top_n: usize,
    pub speedtest_interval_seconds: u64,
    pub require_transport_security: bool,
    pub subscription_count: usize,
    pub enabled_subscription_count: usize,
    pub proxy_enabled: bool,
    pub proxy_port: u16,
    pub proxy_discoverable: bool,
    pub rotating_proxy: bool,
    pub health_check_url: String,
    pub health_check_interval_seconds: u64,
    pub proxy_slow_traffic_kbps: u64,
    pub proxy_bulk_check_interval_seconds: u64,
    /// Pinned config URI (`None` = auto-select). Carried so the TUI can
    /// adopt pins made from the dashboard (which live-pushes without
    /// touching the file the TUI edits from).
    pub proxy_manual_uri: Option<String>,
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

    /// Full Mihomo endpoint for a LAN host, tokenized like [`Self::subscription_url`].
    #[must_use]
    pub fn mihomo_url(&self, host: &str) -> String {
        let mut url = format!(
            "http://{}:{}/mihomo.yaml",
            format_url_host(host),
            self.bind.port()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(
        name: &str,
        latency_ms: u128,
        download_mbps: Option<f64>,
        download_bytes: Option<usize>,
    ) -> RankedConfig {
        RankedConfig {
            rank: 0,
            stability_count: 0,
            id: name.to_string(),
            dedup_key: format!("{name}-key"),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: name.to_string(),
            endpoint: Endpoint {
                host: "example.com".to_string(),
                port: 443,
            },
            uri: format!("vless://{name}@example.com:443"),
            reachable: true,
            validation: "active_http".to_string(),
            latency_ms: Some(latency_ms),
            http_status: Some(200),
            download_mbps,
            download_bytes,
            upload_ok: None,
            error: None,
            country_code: None,
        }
    }

    #[test]
    fn upload_dead_config_is_demoted_but_still_selectable() {
        let mut dead = ranked("dead", 10, Some(9.0), Some(BULK_MIN_BYTES));
        dead.upload_ok = Some(false);
        let good = ranked("good", 500, Some(2.1), Some(BULK_MIN_BYTES));

        assert_eq!(dead.throughput_class(), 2);
        assert!(!dead.bulk_capable());
        assert_eq!(compare_ranked(&good, &dead), Ordering::Less);
        // It is still reachable and eligible when nothing better exists.
        assert!(dead.reachable);
    }

    #[test]
    fn bulk_throughput_beats_low_latency() {
        let fast_handshake = ranked("fast-handshake", 40, None, None);
        let bulk = ranked("bulk", 900, Some(3.6), Some(BULK_MIN_BYTES));
        assert_eq!(bulk.throughput_class(), 0);
        assert_eq!(fast_handshake.throughput_class(), 1);
        assert_eq!(compare_ranked(&bulk, &fast_handshake), Ordering::Less);
    }

    #[test]
    fn truncated_body_ranks_below_unmeasured_and_bulk() {
        // A 12 KB cut-off with a burst-fast tail is still broken: it never
        // carried a bulk body, so it must not outrank an unmeasured config.
        let truncated = ranked("truncated", 30, Some(100.0), Some(12 * 1024));
        let unmeasured = ranked("unmeasured", 400, None, None);
        let bulk = ranked("bulk", 900, Some(3.6), Some(BULK_MIN_BYTES));
        assert_eq!(truncated.throughput_class(), 2);
        assert_eq!(compare_ranked(&truncated, &unmeasured), Ordering::Greater);
        assert_eq!(compare_ranked(&truncated, &bulk), Ordering::Greater);
    }

    #[test]
    fn stability_breaks_ties_inside_one_throughput_class() {
        let mut steady = ranked("steady", 500, Some(3.0), Some(BULK_MIN_BYTES));
        steady.stability_count = 9;
        let mut fresh = ranked("fresh", 100, Some(3.0), Some(BULK_MIN_BYTES));
        fresh.stability_count = 1;
        assert_eq!(compare_ranked(&steady, &fresh), Ordering::Less);
    }
}
