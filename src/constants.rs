use std::time::Duration;

use serde::Serialize;

use crate::tui::state::{ConfigKey, MainItem, SubscriptionAction};

pub const APP_NAME: &str = "V2RayDAR";
pub const APP_DATA_DIR_NAME: &str = "v2raydar_data";
pub const CACHE_DIR_NAME: &str = "cache";
pub const CACHE_METADATA_FILE_NAME: &str = "metadata.json";
pub const CONFIG_FILE_NAME: &str = "configs.yaml";
pub const FIREWALL_STATE_FILE_NAME: &str = ".v2raydar-firewall.json";
pub const LEGACY_APP_MARKER_FILE_NAME: &str = ".v2raydar";
pub const LEGACY_CACHE_MARKER_FILE_NAME: &str = ".v2raydar-cache";
pub const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../configs.example.yaml");

pub const DEFAULT_BIND: &str = "127.0.0.1:27141";
pub const DEFAULT_TOP_N: usize = 10;
pub const DEFAULT_REFRESH_SECONDS: u64 = 300;
pub const DEFAULT_ENCODED_SUBSCRIPTION: bool = true;
pub const DEFAULT_PRIORITIZE_STABILITY: bool = true;
pub const DEFAULT_SCAN_ALL_CONFIGS: bool = false;
pub const DEFAULT_SHARING_ENABLED: bool = false;
pub const DEFAULT_REQUIRE_TOKEN: bool = false;
pub const DEFAULT_SHARING_TOKEN: &str = "";
pub const DEFAULT_FETCH_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_FETCH_CONCURRENCY: usize = 8;
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
pub const DEFAULT_DOWNLOAD_BYTES_LIMIT: usize = 1_048_576;

pub const MAX_TUI_LOGS: usize = 512;
pub const STABLE_WORKING_APPEARANCES: u32 = 2;

pub const DEFAULT_LOG_FILTER_PLAIN: &str = "v2raydar=warn,tower_http=warn";
pub const DEFAULT_LOG_FILTER_VERBOSE: &str = "v2raydar=info,tower_http=warn";
pub const DEFAULT_LOG_FILTER_TUI: &str = "v2raydar=off,tower_http=warn";
pub const CONFIG_WATCH_INTERVAL: Duration = Duration::from_secs(1);
pub const LOCALHOST_IP: &str = "127.0.0.1";
pub const ROUTE_PROBE_ADDR: &str = "8.8.8.8:80";
pub const INTERFACE_CACHE_TTL: Duration = Duration::from_secs(5);

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
pub const SING_BOX_DOWNLOAD_URL: &str = "https://github.com/SagerNet/sing-box/releases/latest";

pub const HTTP_EXCHANGE_OVERHEAD_BYTES: u64 = 1024;
pub const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
pub const FNV_PRIME: u64 = 0x100000001b3;
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

pub const TUI_FRAME_INTERVAL: Duration = Duration::from_millis(100);
pub const TUI_INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);
pub const TUI_MAX_EVENTS_PER_FRAME: usize = 64;
pub const TUI_MAX_VISIBLE_RANKED: usize = 64;
pub const TUI_SETUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
pub const TUI_CONFIG_PANEL_HEIGHT: u16 = 12;
pub const TUI_CONFIG_PANEL_ENDPOINT_HEIGHT: u16 = 14;
pub const TUI_CONFIG_GROUP_HEIGHT: u16 = 10;
pub const TUI_CONFIG_KEY_WIDTH: usize = 14;
pub const TUI_ANSI_UNDERLINE_ENABLE: &str = "\x1b[4m";
pub const TUI_ANSI_UNDERLINE_DISABLE: &str = "\x1b[24m";
pub const TUI_OSC8_LINK_PREFIX: &str = "\x1b]8;;";
pub const TUI_OSC8_LINK_SEPARATOR: &str = "\x1b\\";
pub const TUI_OSC8_LINK_SUFFIX: &str = "\x1b]8;;\x1b\\";
pub const BYTE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
pub const FIREWALL_RULE_NAME: &str = "V2RayDAR Subscription Sharing";
pub const SUBSCRIPTION_READY_WAIT: Duration = Duration::from_secs(20);
pub const SUBSCRIPTION_READY_POLL: Duration = Duration::from_millis(100);
#[cfg(target_os = "windows")]
pub const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(test)]
pub const TEST_REALITY_PUBLIC_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub const MAIN_ITEMS: [MainItem; 6] = [
    MainItem::OpenConfig,
    MainItem::Sharing,
    MainItem::Subscriptions,
    MainItem::CleanCache,
    MainItem::Configurations,
    MainItem::Logs,
];
pub const SUBSCRIPTION_ACTIONS: [SubscriptionAction; 6] = [
    SubscriptionAction::EditName,
    SubscriptionAction::EditUrl,
    SubscriptionAction::EditPriority,
    SubscriptionAction::Toggle,
    SubscriptionAction::Delete,
    SubscriptionAction::Back,
];
pub const CONFIG_KEYS: [ConfigKey; 26] = [
    ConfigKey::Bind,
    ConfigKey::TopN,
    ConfigKey::RefreshSeconds,
    ConfigKey::EncodedSubscription,
    ConfigKey::PrioritizeStability,
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
    ConfigKey::TokenRequired,
    ConfigKey::Token,
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
        help: "false favors any quick working config. true promotes configs seen working in 2+ refreshes.",
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
];
