use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use serde::Serialize;

use crate::tui::state::{ConfigKey, MainItem, SubscriptionAction};

pub const APP_NAME: &str = "V2RayDAR";
pub const APP_DATA_DIR_NAME: &str = "v2raydar_data";
pub const DB_FILE_NAME: &str = "data.db";
/// `SQLite` write-ahead log sidecar, created next to the database while the
/// database is open in WAL mode.
pub const DB_WAL_FILE_NAME: &str = "data.db-wal";
/// `SQLite` shared-memory sidecar for the write-ahead log.
pub const DB_SHM_FILE_NAME: &str = "data.db-shm";
pub const CACHE_DIR_NAME: &str = "cache";
pub const GEOIP_DIR_NAME: &str = "geoip";
pub const GEOIP_MMDB_FILE_NAME: &str = "GeoLite2-Country.mmdb";
/// Upper bound for one `GeoIP` hostname resolution. DNS runs async (never on a
/// worker), but a hanging resolver must not stall a config's result: on
/// timeout the country stays `None` and the next cycle retries.
pub const GEOIP_DNS_TIMEOUT: Duration = Duration::from_secs(5);
pub const CONFIG_FILE_NAME: &str = "configs.yaml";
/// One-line note replacing a migrated `configs.yaml`: settings and
/// subscriptions now live in `data.db`, so the file only asks to be deleted.
/// Startup treats a file containing exactly this line as "already migrated".
pub const CONFIG_MIGRATION_NOTE: &str =
    "# V2RayDAR settings and subscriptions now live in data.db — you can safely delete this file.";
pub const FIREWALL_STATE_FILE_NAME: &str = ".v2raydar-firewall.json";
pub const LEGACY_APP_MARKER_FILE_NAME: &str = ".v2raydar";
pub const LEGACY_CACHE_MARKER_FILE_NAME: &str = ".v2raydar-cache";
pub const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../configs.example.yaml");

pub const DEFAULT_BIND: &str = "127.0.0.1:27141";
pub const DEFAULT_TOP_N: usize = 10;
pub const DEFAULT_REFRESH_SECONDS: u64 = 900;
pub const DEFAULT_PING_SECONDS: u64 = 300;
pub const DEFAULT_ENCODED_SUBSCRIPTION: bool = true;
pub const DEFAULT_PRIORITIZE_STABILITY: bool = true;
pub const DEFAULT_RETURN_CONFIGS_ASAP: bool = false;
pub const DEFAULT_SCAN_ALL_CONFIGS: bool = false;
pub const DEFAULT_SHARING_ENABLED: bool = false;
pub const DEFAULT_REQUIRE_TOKEN: bool = false;
pub const DEFAULT_SHARING_TOKEN: &str = "";
pub const DEFAULT_FETCH_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_FETCH_CONCURRENCY: usize = 8;
/// Total HTTP attempts per subscription fetch, first try included.
/// Only transient failures (connect resets, timeouts, 429/5xx) retry.
pub const FETCH_RETRY_ATTEMPTS: usize = 3;
/// Base delay before a fetch retry; doubles per attempt (1000ms, then 2s).
pub const FETCH_RETRY_BASE_DELAY_MS: u64 = 1000;
pub const DEFAULT_MAX_SUBSCRIPTION_BYTES: usize = 32 * 1024 * 1024;
pub const DEFAULT_USE_CACHE_ONLY: bool = false;
pub const DEFAULT_SUBSCRIPTION_PRIORITY: u32 = 100;
pub const DEFAULT_SUBSCRIPTION_ENABLED: bool = true;
pub const DEFAULT_SING_BOX_PATH: &str = "";
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_ACTIVE_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_STARTUP_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_PROBE_CONCURRENCY: usize = 16;
pub const DEFAULT_PROBE_BATCH_SIZE: Option<usize> = Some(20);
pub const DEFAULT_PROBE_PROCESS_CONCURRENCY: Option<usize> = None;
pub const DEFAULT_TEST_URL: &str = "https://www.gstatic.com/generate_204";
pub const DEFAULT_ACCEPTED_STATUSES: &[u16] = &[204, 200];
/// Speed-test object large enough to get past a shaper's burst allowance.
/// A 1 MiB probe lands exactly on the boundary where many ISPs turn a fast
/// link into a 2 KB/s crawl, so configs were certified by a body they could
/// never finish. The default target carries 10 MiB; we read this much.
pub const DEFAULT_DOWNLOAD_BYTES_LIMIT: usize = 4 * 1024 * 1024;
/// How many of the top reachable configs get a speedtest in one run. Lower
/// values save data; the rest keep their previous measurement or stay
/// unmeasured (class 1, behind proven bulk nodes).
pub const DEFAULT_SPEEDTEST_TOP_N: usize = 2;
/// Re-use a successful speedtest for this long before downloading the body
/// again. 0 re-tests on every cycle.
pub const DEFAULT_SPEEDTEST_INTERVAL_SECONDS: u64 = 900;
/// Off by default: other countries keep every transport that works for them.
pub const DEFAULT_REQUIRE_TRANSPORT_SECURITY: bool = false;
/// Public static objects (not a speed-test API) used when the user has not
/// chosen a speed-test URL. Both support HTTP Range and return a real body.
pub const DEFAULT_DOWNLOAD_URL: &str = "https://download.thinkbroadband.com/10MB.zip";
pub const BULK_CHECK_FALLBACK_URL: &str = "https://proof.ovh.net/files/10Mb.dat";
/// Throughput classes use the same bar in the prober, the database and the
/// live proxy: a config only counts as bulk-capable when it carried this
/// much without dying and its tail was at least this fast (Mbit/s).
pub const BULK_MIN_BYTES: usize = 4 * 1024 * 1024;
pub const BULK_MIN_MBPS: f64 = 2.0;
/// Speed is scored over the end of the transfer, not the whole-body average:
/// a burst-then-crawl link has a great average and a useless tail.
pub const DOWNLOAD_TAIL_WINDOW: Duration = Duration::from_secs(2);
/// One speed-test attempt, body included. Without this a stalled download
/// could hang a cycle forever (the client itself had no timeout).
pub const SPEEDTEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Active-proxy deep check: ten megabytes, the size that exposes a
/// TLS-inspecting middlebox that truncates around 12 KB. The check passes as
/// soon as it has carried the bulk minimum at the bulk rate; otherwise it
/// keeps reading to the cap and judges the tail.
pub const PROXY_BULK_CHECK_BYTES: usize = 10 * 1024 * 1024;
pub const PROXY_BULK_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
pub const PROXY_BULK_CHECK_FALLBACK_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_CLEAN_OFFLINES_AFTER_DAYS: u32 = 7;
pub const DEFAULT_PROXY_ENABLED: bool = false;
pub const DEFAULT_PROXY_PORT: u16 = 27910;
pub const DEFAULT_PROXY_DISCOVERABLE: bool = false;
pub const DEFAULT_ROTATING_PROXY: bool = true;
pub const DEFAULT_PROXY_HEALTH_CHECK_URL: &str = "https://cp.cloudflare.com";
pub const DEFAULT_PROXY_HEALTH_CHECK_INTERVAL: u64 = 60;
/// Live-traffic floor for an automatic speed switch. 0 turns the slow-link
/// switch off; starvation and health failover still run.
pub const DEFAULT_PROXY_SLOW_TRAFFIC_KBPS: u64 = PROXY_STREAM_SLOW_BPS / 1024;
/// Minimum gap between deep 10 MB checks on the active proxy. 0 turns the
/// deep check off (probe measurements can still justify a switch).
pub const DEFAULT_PROXY_BULK_CHECK_INTERVAL_SECONDS: u64 = PROXY_BULK_CHECK_INTERVAL.as_secs();
pub const PROXY_MAX_CONSECUTIVE_FAILURES: u32 = 3;
pub const PROXY_FAILOVER_COOLDOWN: Duration = Duration::from_secs(10);
/// Minimum dwell on a freshly switched config: post-cycle updates must not
/// flap straight onto the next-best config when rankings jitter. Failovers
/// (proven failures) and manual pins always bypass the dwell.
pub const PROXY_MIN_SWITCH_INTERVAL: Duration = Duration::from_mins(1);
/// Idle-path verification uses one URL on a short budget instead of the full
/// gauntlet: a dead proxy with nobody connected must still be caught.
pub const PROXY_HEALTH_LIGHT_TIMEOUT: Duration = Duration::from_secs(5);
/// Minimum effective speed for a completed small body: a fast tiny page
/// proves data flows (switching proxies cannot fix upstream content, and the
/// starvation detector covers real user traffic), while a slow one cannot
/// serve interactive apps — fail over instead of waiting for starvation.
/// 204-empty and capped bodies always pass; incomplete bodies always fail.
/// Adaptive: full speed while the pool is healthy, absolute minimum once
/// every reachable config has failed (nothing works — accept anything that
/// moves at all, never lower).
pub const PROXY_HEALTH_FULL_BPS: u64 = 10 * 1024;
pub const PROXY_HEALTH_MIN_BPS: u64 = 1024;
/// A passing check slower than this logs a warning: the config works but is
/// degrading, so the next failure reads in context instead of out of nowhere.
pub const PROXY_SLOW_CHECK_WARN: Duration = Duration::from_secs(10);
pub const PROXY_MAX_RECENTLY_FAILED_KEYS: usize = 50;
pub const PROXY_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
pub const PROXY_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(15);
pub const PROXY_PORT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// A health check only passes when the response body actually transfers:
/// complete, or at least this much received (short-circuits huge bodies).
/// Headers alone never pass — false-positive configs handshake then stall.
pub const PROXY_HEALTH_CHECK_BODY_BYTES: u64 = 1_048_576;
/// Timeout for one Clash API snapshot read (well under the poll interval).
pub const PROXY_CLASH_API_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the starvation detector polls sing-box `/connections`.
pub const PROXY_STARVATION_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// A connection counts as starved only after trying this long with zero
/// bytes back: long enough that slow-but-working links never trip it.
pub const PROXY_STARVATION_MIN_AGE: Duration = Duration::from_secs(20);
/// ... and after uploading at least this much (a real request's headers, not
/// a bare connect). TCP acknowledgements alone put any working connection's
/// download above zero, so `download == 0` at this volume after the age gate
/// is pathological, not slow. The old 32 KB bar let a stuck 1 KB request sit
/// forever because a browser uploads only headers before it waits.
pub const PROXY_STARVATION_MIN_UPLOAD_BYTES: u64 = 256;
/// Live-traffic window: connections that keep moving this little over a
/// whole window are slow enough that a working alternative is worth a switch.
/// A 2 KB/s crawl trips it; ordinary browsing (a page or two per window)
/// mostly does not because it ends before the window closes.
pub const PROXY_STREAM_WINDOW: Duration = Duration::from_secs(10);
pub const PROXY_STREAM_MIN_AGE: Duration = Duration::from_secs(10);
pub const PROXY_STREAM_MIN_WINDOW_BYTES: u64 = 8 * 1024;
pub const PROXY_STREAM_SLOW_BPS: u64 = 64 * 1024;
pub const PROXY_STREAM_SLOW_WINDOWS: u8 = 2;
/// A candidate must be at least this much faster than live traffic before its
/// measured speed alone justifies switching (avoids flapping on noise).
pub const PROXY_STREAM_UPGRADE_FACTOR: f64 = 2.0;
/// Deep bulk checks are expensive; one per active config per interval is
/// enough to catch a middlebox that starts truncating later.
pub const PROXY_BULK_CHECK_INTERVAL: Duration = Duration::from_mins(5);
/// Minimum gap between performance-driven switches, independent of the
/// ordinary ranking dwell and the failure cooldown.
pub const PROXY_PERFORMANCE_SWITCH_COOLDOWN: Duration = Duration::from_secs(10);
/// Controller ports probed above the proxy port for the Clash API listener.
pub const PROXY_CLASH_CONTROLLER_PORT_ATTEMPTS: u16 = 32;
/// Unreachable-controller polls to skip before retrying (same-port
/// restarts re-arm the detector instead of staying dark).
pub const PROXY_CLASH_UNREACHABLE_RETRY_POLLS: u8 = 12;
pub const PROXY_DNS_PRIMARY: &str = "8.8.8.8";
pub const PROXY_DNS_FALLBACK: &str = "1.1.1.1";
pub const PROXY_SING_BOX_TAG_OUTBOUND: &str = "proxy-0";
pub const PROXY_SING_BOX_TAG_DIRECT: &str = "direct-out";
pub const PROXY_SING_BOX_TAG_INBOUND: &str = "proxy-in";
pub const PROXY_SING_BOX_TAG_DNS_DIRECT: &str = "dns-direct";
pub const PROXY_SING_BOX_TAG_DNS_FALLBACK: &str = "dns-fallback";
pub const PROXY_SING_BOX_TAG_DNS_PROXY: &str = "dns-proxy";
pub const FIREWALL_PROXY_RULE_NAME: &str = "V2RayDAR Proxy";
pub const PROXY_EMOJI: &str = "🚪";

pub const MAX_TUI_LOGS: usize = 512;

pub const DEFAULT_LOG_FILTER_PLAIN: &str = "v2raydar=warn,tower_http=warn";
pub const DEFAULT_LOG_FILTER_VERBOSE: &str = "v2raydar=info,tower_http=warn";
pub const DEFAULT_LOG_FILTER_TUI: &str = "v2raydar=off,tower_http=warn";
pub const LOCALHOST_IP: &str = "127.0.0.1";
pub const ROUTE_PROBE_ADDR: &str = "8.8.8.8:80";
pub const ROUTE_PROBE_ADDR_FALLBACK: &str = "1.1.1.1:80";
pub const INTERFACE_CACHE_TTL: Duration = Duration::from_secs(5);
pub const GATEWAY_CHECK_TTL: Duration = Duration::from_mins(1);
pub const DHCP_CONFLICT_RANGE: u8 = 10;
pub const DHCP_CONFLICT_CACHE_TTL: Duration = Duration::from_secs(30);

pub const ACTIVE_PROBE_BATCH_MIN_SIZE: usize = 32;
pub const ACTIVE_PROBE_BATCH_MAX_SIZE: usize = 128;
pub const ACTIVE_PROBE_BATCH_CONCURRENCY_MULTIPLIER: usize = 16;
pub const ACTIVE_PROBE_HTTP_MAX_CONCURRENCY: usize = 128;
pub const ACTIVE_PROBE_PROCESS_MAX_CONCURRENCY: usize = 4;
pub const LOCAL_PROXY_WAIT_INTERVAL: Duration = Duration::from_millis(25);
pub const LOCAL_PROXY_CONNECT_TIMEOUT: Duration = Duration::from_millis(5);
pub const SING_BOX_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
pub const SING_BOX_CONFIG_FILE_PREFIX: &str = "v2raydar-sing-box";
pub const SING_BOX_INBOUND_TAG_PREFIX: &str = "mixed-in";
pub const SING_BOX_OUTBOUND_TAG_PREFIX: &str = "proxy";
pub const SING_BOX_VERSION: &str = "1.13.13";
pub const SING_BOX_RELEASE_URL_PREFIX: &str = "https://github.com/SagerNet/sing-box/releases/tag/v";

pub fn sing_box_download_url() -> String {
    format!("{SING_BOX_RELEASE_URL_PREFIX}{SING_BOX_VERSION}")
}

pub const HTTP_EXCHANGE_OVERHEAD_BYTES: u64 = 1024;
pub const BITS_PER_BYTE: f64 = 8.0;
pub const BITS_PER_MEGABIT: f64 = 1_000_000.0;
pub const SUPPORTED_URI_SCHEMES: &[&str] = &[
    "vmess://",
    "vless://",
    "trojan://",
    "ss://",
    "ssr://",
    "hysteria2://",
    "hy2://",
    "tuic://",
];

/// Public suffixes that appear in real Reality cover hostnames. The ingest
/// hardener rejects a Reality SNI whose last label is not here: a fantasy
/// cover (`example.invalidtld`, a bare word, an IP) cannot survive a real
/// TLS handshake and only fills the pool with configs that will never work.
/// This is deliberately a static allow-list, not a full PSL: it is the
/// escape hatch for a new TLD, not a correctness oracle.
pub const KNOWN_PUBLIC_SUFFIXES: &[&str] = &[
    "com",
    "net",
    "org",
    "edu",
    "gov",
    "mil",
    "int",
    "io",
    "dev",
    "app",
    "xyz",
    "info",
    "biz",
    "me",
    "tv",
    "cc",
    "co",
    "site",
    "online",
    "store",
    "tech",
    "cloud",
    "live",
    "link",
    "click",
    "top",
    "pro",
    "vip",
    "fun",
    "space",
    "world",
    "today",
    "life",
    "news",
    "blog",
    "zone",
    "network",
    "agency",
    "digital",
    "media",
    "services",
    "solutions",
    "systems",
    "email",
    "social",
    "design",
    "studio",
    "software",
    "host",
    "website",
    "shop",
    "club",
    "art",
    "one",
    "page",
    "run",
    "sh",
    "ly",
    "to",
    "in",
    "ir",
    "de",
    "nl",
    "uk",
    "fr",
    "se",
    "fi",
    "no",
    "dk",
    "es",
    "it",
    "ch",
    "at",
    "be",
    "pl",
    "ru",
    "ua",
    "tr",
    "ae",
    "sa",
    "qa",
    "il",
    "eg",
    "iq",
    "af",
    "pk",
    "bd",
    "lk",
    "np",
    "az",
    "am",
    "ge",
    "kz",
    "uz",
    "tm",
    "tj",
    "kg",
    "cn",
    "hk",
    "tw",
    "jp",
    "kr",
    "sg",
    "my",
    "id",
    "th",
    "vn",
    "ph",
    "au",
    "nz",
    "ca",
    "us",
    "mx",
    "br",
    "ar",
    "cl",
    "pe",
    "za",
    "ng",
    "ke",
    "ma",
    "eu",
];

/// Shadowsocks cipher methods supported by sing-box and Mihomo.
/// Includes AEAD, 2022, and legacy stream ciphers that Mihomo supports.
pub const SUPPORTED_SS_METHODS: &[&str] = &[
    // AEAD
    "aes-128-gcm",
    "aes-256-gcm",
    "chacha20-ietf-poly1305",
    "xchacha20-ietf-poly1305",
    // AEAD 2022
    "2022-blake3-aes-128-gcm",
    "2022-blake3-aes-256-gcm",
    "2022-blake3-chacha20-poly1305",
    // Legacy stream ciphers (Mihomo/sing-box compat)
    "aes-128-cfb",
    "aes-128-ctr",
    "aes-128-ofb",
    "aes-256-cfb",
    "aes-256-ctr",
    "aes-256-ofb",
    "bf-cfb",
    "cast5-cfb",
    "chacha20",
    "chacha20-ietf",
    "rc4-md5",
    "seed-cfb",
];

pub const TUI_FRAME_INTERVAL: Duration = Duration::from_millis(100);
pub const TUI_INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);
pub const TUI_MAX_EVENTS_PER_FRAME: usize = 64;
pub const TUI_MAX_VISIBLE_RANKED: usize = 64;
pub const TUI_SETUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
pub const TUI_CONFIG_PANEL_HEIGHT: u16 = 10;
pub const TUI_CONFIG_PANEL_ENDPOINT_HEIGHT: u16 = 12;

pub const TUI_ANSI_UNDERLINE_ENABLE: &str = "\x1b[4m";
pub const TUI_ANSI_UNDERLINE_DISABLE: &str = "\x1b[24m";
pub const TUI_OSC8_LINK_PREFIX: &str = "\x1b]8;;";
pub const TUI_OSC8_LINK_SEPARATOR: &str = "\x1b\\";
pub const TUI_OSC8_LINK_SUFFIX: &str = "\x1b]8;;\x1b\\";
pub const BYTE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
pub const BYTES_PER_UNIT: f64 = 1024.0;
pub const FIREWALL_RULE_NAME: &str = "V2RayDAR Subscription Sharing";
pub const SUBSCRIPTION_READY_WAIT: Duration = Duration::from_secs(20);
pub const SUBSCRIPTION_READY_POLL: Duration = Duration::from_millis(100);
#[cfg(target_os = "windows")]
pub const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
pub const TEST_REALITY_PUBLIC_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub const MAIN_ITEMS: [MainItem; 8] = [
    MainItem::Sharing,
    MainItem::Proxy,
    MainItem::Subscriptions,
    MainItem::CleanCache,
    MainItem::Configurations,
    MainItem::Logs,
    MainItem::DashboardPassword,
    MainItem::QrCodes,
];

/// Menu rows the user can actually reach: the QR sheet item needs a desktop
/// image viewer, so Termux/Android hides it while everything else stays put.
#[must_use]
pub fn visible_main_items() -> &'static [MainItem] {
    debug_assert!(matches!(MAIN_ITEMS.last(), Some(MainItem::QrCodes)));
    #[cfg(target_os = "android")]
    {
        &MAIN_ITEMS[..MAIN_ITEMS.len() - 1]
    }
    #[cfg(not(target_os = "android"))]
    {
        &MAIN_ITEMS
    }
}
pub const SUBSCRIPTION_ACTIONS: [SubscriptionAction; 6] = [
    SubscriptionAction::EditName,
    SubscriptionAction::EditUrl,
    SubscriptionAction::EditPriority,
    SubscriptionAction::Toggle,
    SubscriptionAction::Delete,
    SubscriptionAction::Back,
];
pub const CONFIG_KEYS: [ConfigKey; 40] = [
    ConfigKey::Bind,
    ConfigKey::TopN,
    ConfigKey::RefreshSeconds,
    ConfigKey::PingSeconds,
    ConfigKey::EncodedSubscription,
    ConfigKey::PrioritizeStability,
    ConfigKey::ReturnConfigsAsap,
    ConfigKey::ScanAllConfigs,
    ConfigKey::FetchTimeout,
    ConfigKey::FetchConcurrency,
    ConfigKey::MaxSubscriptionBytes,
    ConfigKey::UseCacheOnly,
    ConfigKey::EmergencyConfig,
    ConfigKey::ProbeMode,
    ConfigKey::SingBoxPath,
    ConfigKey::ConnectTimeout,
    ConfigKey::ActiveTimeout,
    ConfigKey::StartupTimeout,
    ConfigKey::ProbeConcurrency,
    ConfigKey::ProbeBatchSize,
    ConfigKey::ProbeProcessConcurrency,
    ConfigKey::TestUrl,
    ConfigKey::AcceptedStatuses,
    ConfigKey::DownloadUrl,
    ConfigKey::DownloadLimit,
    ConfigKey::SpeedtestTopN,
    ConfigKey::SpeedtestInterval,
    ConfigKey::RequireTransportSecurity,
    ConfigKey::CleanOfflineDays,
    ConfigKey::TokenRequired,
    ConfigKey::Token,
    ConfigKey::ProxyEnabled,
    ConfigKey::ProxyPort,
    ConfigKey::ProxyDiscoverable,
    ConfigKey::RotatingProxy,
    ConfigKey::ProxyHealthCheckUrl,
    ConfigKey::ProxyHealthCheckInterval,
    ConfigKey::SlowTrafficKbps,
    ConfigKey::BulkCheckInterval,
    ConfigKey::ResetDefaults,
];

#[derive(Debug, Clone, Serialize)]
pub struct SettingGuide {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

pub const SETTING_GUIDES: &[SettingGuide] = &[
    SettingGuide {
        key: "bind",
        label: "Listen address",
        help: "127.0.0.1 stays private. Use the device LAN IP for sharing; 0.0.0.0 listens on all interfaces.",
    },
    SettingGuide {
        key: "sharing.enabled",
        label: "LAN sharing",
        help: "Shows the subscription on your local network. Keep off on untrusted Wi-Fi.",
    },
    SettingGuide {
        key: "sharing.require_token",
        label: "URL token",
        help: "Adds ?token=... so casual LAN visitors cannot read the subscription.",
    },
    SettingGuide {
        key: "sharing.token",
        label: "Token value",
        help: "null disables URL tokens. Set true to generate one, or provide a string.",
    },
    SettingGuide {
        key: "encoded_subscription",
        label: "Encoded feed",
        help: "Use base64 for v2rayN/v2rayNG. Use .txt for a raw link list.",
    },
    SettingGuide {
        key: "prioritize_stability",
        label: "Stable ranking",
        help: "true keeps the previous run's top-N ahead inside the same throughput class (re-pinged first). Measured bulk throughput always decides first; false drops the stability carry-over.",
    },
    SettingGuide {
        key: "probe.speedtest_top_n",
        label: "Speedtest count",
        help: "How many top reachable configs get a real-body speedtest per run. Lower uses less data; 0 turns speedtests off.",
    },
    SettingGuide {
        key: "probe.require_transport_security",
        label: "Require TLS",
        help: "true drops Shadowsocks/SSR, no-TLS VMess/VLESS, and Reality with an impossible SNI before they take a probe slot. Off by default.",
    },
    SettingGuide {
        key: "proxy.slow_traffic_kbps",
        label: "Slow link floor",
        help: "Live traffic below this KB/s for two windows triggers a fast switch. 0 disables the slow-link switch; health and starvation failover still run.",
    },
    SettingGuide {
        key: "return_configs_asap",
        label: "ASAP results",
        help: "true publishes working configs as they are found; early configs may not have the lowest ping or best stability.",
    },
    SettingGuide {
        key: "scan_all_configs",
        label: "Full scan",
        help: "true checks every loaded config. false stops once enough sing-box-validated configs are found.",
    },
    SettingGuide {
        key: "probe.mode",
        label: "Validation mode",
        help: "Active uses sing-box for real checks. TCP is diagnostic only.",
    },
    SettingGuide {
        key: "probe.sing_box_path",
        label: "sing-box path",
        help: "Use sing-box on Linux, Termux, or macOS; use sing-box.exe on Windows.",
    },
    SettingGuide {
        key: "clean_offlines_after_days",
        label: "Offline cleanup",
        help: "Configs not seen online for this many days are removed from the database.",
    },
];

// ── URI / YAML encoding sets ────────────────────────────────────────────────

/// Percent-encoding set for URI query parameters (keeps `-_.~/` unencoded).
pub const URI_QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

/// Percent-encoding set for SS SIP002 plugin query values (keeps `-_.~;` unencoded).
pub const URI_PLUGIN: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b';');

/// Characters that require quoting in generated YAML proxy names.
pub const YAML_NEEDS_QUOTE: &str = ":#{}[],&*!|><%@`\\\"\n\r\t";
