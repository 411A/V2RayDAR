use std::{fs, net::SocketAddr, path::Path};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml::Value;

use crate::constants::{
    DEFAULT_ACCEPTED_STATUSES, DEFAULT_ACTIVE_TIMEOUT_MS, DEFAULT_BIND,
    DEFAULT_CLEAN_OFFLINES_AFTER_DAYS, DEFAULT_CONFIG_TEMPLATE, DEFAULT_CONNECT_TIMEOUT_MS,
    DEFAULT_DOWNLOAD_BYTES_LIMIT, DEFAULT_ENCODED_SUBSCRIPTION, DEFAULT_FETCH_CONCURRENCY,
    DEFAULT_FETCH_TIMEOUT_MS, DEFAULT_MAX_SUBSCRIPTION_BYTES, DEFAULT_PRIORITIZE_STABILITY,
    DEFAULT_PROBE_BATCH_SIZE, DEFAULT_PROBE_CONCURRENCY, DEFAULT_PROBE_PROCESS_CONCURRENCY,
    DEFAULT_PROXY_DISCOVERABLE, DEFAULT_PROXY_ENABLED, DEFAULT_PROXY_HEALTH_CHECK_INTERVAL,
    DEFAULT_PROXY_HEALTH_CHECK_URL, DEFAULT_PROXY_PORT, DEFAULT_REFRESH_SECONDS,
    DEFAULT_REQUIRE_TOKEN, DEFAULT_RETURN_CONFIGS_ASAP, DEFAULT_SCAN_ALL_CONFIGS,
    DEFAULT_SHARING_ENABLED, DEFAULT_SHARING_TOKEN, DEFAULT_SING_BOX_PATH,
    DEFAULT_STARTUP_TIMEOUT_MS, DEFAULT_SUBSCRIPTION_ENABLED, DEFAULT_SUBSCRIPTION_PRIORITY,
    DEFAULT_TEST_URL, DEFAULT_TOP_N, DEFAULT_USE_CACHE_ONLY,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct AppConfig {
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    #[serde(default = "default_refresh_seconds")]
    pub refresh_seconds: u64,
    #[serde(default = "default_encoded_subscription")]
    pub encoded_subscription: bool,
    #[serde(default = "default_prioritize_stability")]
    pub prioritize_stability: bool,
    #[serde(default = "default_return_configs_asap")]
    pub return_configs_asap: bool,
    #[serde(default = "default_scan_all_configs")]
    pub scan_all_configs: bool,
    #[serde(default = "default_fetch_timeout_ms")]
    pub fetch_timeout_ms: u64,
    #[serde(default = "default_fetch_concurrency")]
    pub fetch_concurrency: usize,
    #[serde(default = "default_max_subscription_bytes")]
    pub max_subscription_bytes: usize,
    #[serde(default = "default_use_cache_only")]
    pub use_cache_only: bool,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub emergency_config: Option<String>,
    #[serde(default = "default_clean_offlines_after_days")]
    pub clean_offlines_after_days: u32,
    #[serde(default)]
    pub probe: ProbeConfig,
    #[serde(default)]
    pub sharing: SharingConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub geoip_db_path: Option<String>,
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct SubscriptionSource {
    pub name: String,
    pub url: String,
    #[serde(default = "default_subscription_enabled")]
    pub enabled: bool,
    #[serde(default = "default_priority")]
    pub priority: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct ProbeConfig {
    #[serde(default = "default_probe_mode")]
    pub mode: ProbeMode,
    #[serde(
        default = "default_sing_box_path",
        deserialize_with = "deserialize_string_or_null_as_default"
    )]
    pub sing_box_path: String,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_active_timeout_ms")]
    pub active_timeout_ms: u64,
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    #[serde(default = "default_probe_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_probe_batch_size")]
    pub batch_size: Option<usize>,
    #[serde(default = "default_probe_process_concurrency")]
    pub process_concurrency: Option<usize>,
    #[serde(default = "default_test_url")]
    pub test_url: String,
    #[serde(default = "default_accepted_statuses")]
    pub accepted_statuses: Vec<u16>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub download_url: Option<String>,
    #[serde(default = "default_download_bytes_limit")]
    pub download_bytes_limit: usize,
    #[serde(skip)]
    pub sing_box_path_auto: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeMode {
    Active,
    Tcp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SharingConfig {
    #[serde(default = "default_sharing_enabled")]
    pub enabled: bool,
    #[serde(default = "default_require_token")]
    pub require_token: bool,
    #[serde(
        default = "default_sharing_token",
        deserialize_with = "deserialize_sharing_token"
    )]
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProxyConfig {
    #[serde(default = "default_proxy_enabled")]
    pub enabled: bool,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default = "default_proxy_discoverable")]
    pub discoverable: bool,
    #[serde(default = "default_proxy_health_check_url")]
    pub health_check_url: String,
    #[serde(default = "default_proxy_health_check_interval")]
    pub health_check_interval_seconds: u64,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            mode: default_probe_mode(),
            sing_box_path: default_sing_box_path(),
            connect_timeout_ms: default_connect_timeout_ms(),
            active_timeout_ms: default_active_timeout_ms(),
            startup_timeout_ms: default_startup_timeout_ms(),
            concurrency: default_probe_concurrency(),
            batch_size: default_probe_batch_size(),
            process_concurrency: default_probe_process_concurrency(),
            test_url: default_test_url(),
            accepted_statuses: default_accepted_statuses(),
            download_url: None,
            download_bytes_limit: default_download_bytes_limit(),
            sing_box_path_auto: false,
        }
    }
}

impl Default for SharingConfig {
    fn default() -> Self {
        Self {
            enabled: default_sharing_enabled(),
            require_token: default_require_token(),
            token: default_sharing_token(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: default_proxy_enabled(),
            port: default_proxy_port(),
            discoverable: default_proxy_discoverable(),
            health_check_url: default_proxy_health_check_url(),
            health_check_interval_seconds: default_proxy_health_check_interval(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self::load_with_generated_token_flag(path)?.0)
    }

    pub fn load_with_generated_token_flag(path: &Path) -> Result<(Self, bool)> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("unable to read {}", path.display()))?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let config = match extension.as_str() {
            "json" => serde_json::from_str(&content).context("invalid JSON config")?,
            "yaml" | "yml" | "" => serde_yaml::from_str(&content).context("invalid YAML config")?,
            other => {
                return Err(anyhow!(
                    "unsupported config extension '.{other}'; use .yaml, .yml, or .json"
                ));
            }
        };

        let generated_token_requested = sharing_token_requests_generation(&content);
        Ok((validate(config)?, generated_token_requested))
    }

    pub fn default_for_first_run() -> Self {
        let config = serde_yaml::from_str::<Self>(DEFAULT_CONFIG_TEMPLATE)
            .expect("default config template is valid");
        validate(config).expect("default config template passes validation")
    }

    pub fn write_default(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("unable to create {}", parent.display()))?;
        }

        let config = Self::default_for_first_run();
        validate(config).context("default config template failed validation")?;
        fs::write(path, DEFAULT_CONFIG_TEMPLATE)
            .with_context(|| format!("unable to write default config to {}", path.display()))
    }

    pub fn subscription_url(&self, host: &str, raw: bool) -> String {
        let endpoint = if raw {
            "subscription.txt"
        } else {
            "subscription"
        };
        let mut url = format!("http://{}:{}/{}", host, self.bind.port(), endpoint);

        if should_include_token_in_url(&self.sharing.token) {
            url.push_str("?token=");
            url.push_str(&self.sharing.token);
        }

        url
    }
}

fn validate(mut config: AppConfig) -> Result<AppConfig> {
    config.probe.sing_box_path = normalize_string_or_null(&config.probe.sing_box_path);
    config.probe.download_url = normalize_optional_string(config.probe.download_url.as_deref());
    config.emergency_config = normalize_optional_string(config.emergency_config.as_deref());
    config.sharing.token = normalize_sharing_token(&config.sharing.token);

    if config.top_n == 0 {
        return Err(anyhow!("top_n must be greater than 0"));
    }

    if config.fetch_concurrency == 0 {
        return Err(anyhow!("fetch_concurrency must be greater than 0"));
    }

    if config.max_subscription_bytes == 0 {
        return Err(anyhow!("max_subscription_bytes must be greater than 0"));
    }

    if config.probe.concurrency == 0 {
        return Err(anyhow!("probe.concurrency must be greater than 0"));
    }

    if config.probe.batch_size == Some(0) {
        return Err(anyhow!("probe.batch_size must be null or greater than 0"));
    }

    if config.probe.process_concurrency == Some(0) {
        return Err(anyhow!(
            "probe.process_concurrency must be null or greater than 0"
        ));
    }

    if config.probe.connect_timeout_ms == 0 {
        return Err(anyhow!("probe.connect_timeout_ms must be greater than 0"));
    }

    if config.probe.active_timeout_ms == 0 {
        return Err(anyhow!("probe.active_timeout_ms must be greater than 0"));
    }

    if config.probe.startup_timeout_ms == 0 {
        return Err(anyhow!("probe.startup_timeout_ms must be greater than 0"));
    }

    if config.probe.mode == ProbeMode::Active && config.probe.test_url.trim().is_empty() {
        return Err(anyhow!(
            "probe.test_url cannot be empty when probe.mode is active"
        ));
    }

    if config.probe.mode == ProbeMode::Active && config.probe.accepted_statuses.is_empty() {
        return Err(anyhow!(
            "probe.accepted_statuses cannot be empty when probe.mode is active"
        ));
    }

    if config
        .probe
        .accepted_statuses
        .iter()
        .any(|status| !(100..=599).contains(status))
    {
        return Err(anyhow!(
            "probe.accepted_statuses must contain valid HTTP status codes from 100 to 599"
        ));
    }

    if config.probe.download_bytes_limit == 0 {
        return Err(anyhow!("probe.download_bytes_limit must be greater than 0"));
    }

    if config.clean_offlines_after_days == 0 {
        return Err(anyhow!("clean_offlines_after_days must be greater than 0"));
    }

    for subscription in &config.subscriptions {
        if subscription.name.trim().is_empty() {
            return Err(anyhow!("subscription name cannot be empty"));
        }

        if subscription.url.trim().is_empty() {
            return Err(anyhow!(
                "subscription '{}' has an empty url",
                subscription.name
            ));
        }
    }

    if config.sharing.require_token && config.sharing.token.is_empty() {
        return Err(anyhow!(
            "sharing.token must be a string or true when sharing.require_token is true"
        ));
    }

    if config.proxy.enabled {
        if config.proxy.port == 0 {
            return Err(anyhow!("proxy.port must be greater than 0"));
        }

        if config.proxy.port == config.bind.port() {
            return Err(anyhow!(
                "proxy.port ({}) must not equal bind port ({})",
                config.proxy.port,
                config.bind.port()
            ));
        }

        if config.proxy.health_check_interval_seconds == 0 {
            return Err(anyhow!(
                "proxy.health_check_interval_seconds must be greater than 0"
            ));
        }
    }

    Ok(config)
}

fn deserialize_string_or_null_as_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(normalize_string_or_null(
        Option::<String>::deserialize(deserializer)?
            .as_deref()
            .unwrap_or_default(),
    ))
}

fn deserialize_sharing_token<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Value>::deserialize(deserializer)? {
        Some(Value::Bool(true)) => generate_token(),
        Some(Value::Bool(false) | Value::Null) | None => String::new(),
        Some(Value::String(value)) => normalize_sharing_token(&value),
        Some(_) => {
            return Err(serde::de::Error::custom(
                "sharing.token must be null, true, false, or a string",
            ));
        }
    })
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(normalize_optional_string(
        Option::<String>::deserialize(deserializer)?.as_deref(),
    ))
}

fn normalize_string_or_null(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_sharing_token(value: &str) -> String {
    let value = normalize_string_or_null(value);
    if value.eq_ignore_ascii_case("true") {
        generate_token()
    } else {
        value
    }
}

pub fn should_include_token_in_url(token: &str) -> bool {
    !token.trim().is_empty()
}

fn sharing_token_requests_generation(content: &str) -> bool {
    let Ok(document) = serde_yaml::from_str::<Value>(content) else {
        return false;
    };
    let Some(sharing) = mapping_value(&document, "sharing") else {
        return false;
    };
    let Some(token) = mapping_value(sharing, "token") else {
        return false;
    };

    match token {
        Value::Bool(true) => true,
        Value::String(value) => value.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn mapping_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Mapping(mapping) = value else {
        return None;
    };

    mapping.get(Value::String(key.to_string()))
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    let value = normalize_string_or_null(value.unwrap_or_default());
    match value.to_ascii_lowercase().as_str() {
        "" | "off" | "none" => None,
        _ => Some(value),
    }
}

fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    if getrandom::fill(&mut bytes).is_ok() {
        return URL_SAFE_NO_PAD.encode(bytes);
    }

    let fallback = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    URL_SAFE_NO_PAD.encode(fallback)
}

fn default_bind() -> SocketAddr {
    DEFAULT_BIND.parse().expect("default bind address is valid")
}

const fn default_top_n() -> usize {
    DEFAULT_TOP_N
}

const fn default_refresh_seconds() -> u64 {
    DEFAULT_REFRESH_SECONDS
}

const fn default_encoded_subscription() -> bool {
    DEFAULT_ENCODED_SUBSCRIPTION
}

const fn default_prioritize_stability() -> bool {
    DEFAULT_PRIORITIZE_STABILITY
}

const fn default_return_configs_asap() -> bool {
    DEFAULT_RETURN_CONFIGS_ASAP
}

const fn default_scan_all_configs() -> bool {
    DEFAULT_SCAN_ALL_CONFIGS
}

const fn default_sharing_enabled() -> bool {
    DEFAULT_SHARING_ENABLED
}

const fn default_require_token() -> bool {
    DEFAULT_REQUIRE_TOKEN
}

fn default_sharing_token() -> String {
    DEFAULT_SHARING_TOKEN.to_string()
}

const fn default_fetch_timeout_ms() -> u64 {
    DEFAULT_FETCH_TIMEOUT_MS
}

const fn default_fetch_concurrency() -> usize {
    DEFAULT_FETCH_CONCURRENCY
}

const fn default_max_subscription_bytes() -> usize {
    DEFAULT_MAX_SUBSCRIPTION_BYTES
}

const fn default_use_cache_only() -> bool {
    DEFAULT_USE_CACHE_ONLY
}

const fn default_priority() -> u32 {
    DEFAULT_SUBSCRIPTION_PRIORITY
}

const fn default_subscription_enabled() -> bool {
    DEFAULT_SUBSCRIPTION_ENABLED
}

const fn default_probe_mode() -> ProbeMode {
    ProbeMode::Active
}

fn default_sing_box_path() -> String {
    DEFAULT_SING_BOX_PATH.to_string()
}

const fn default_connect_timeout_ms() -> u64 {
    DEFAULT_CONNECT_TIMEOUT_MS
}

const fn default_active_timeout_ms() -> u64 {
    DEFAULT_ACTIVE_TIMEOUT_MS
}

const fn default_startup_timeout_ms() -> u64 {
    DEFAULT_STARTUP_TIMEOUT_MS
}

const fn default_probe_concurrency() -> usize {
    DEFAULT_PROBE_CONCURRENCY
}

const fn default_probe_batch_size() -> Option<usize> {
    DEFAULT_PROBE_BATCH_SIZE
}

const fn default_probe_process_concurrency() -> Option<usize> {
    DEFAULT_PROBE_PROCESS_CONCURRENCY
}

fn default_test_url() -> String {
    DEFAULT_TEST_URL.to_string()
}

fn default_accepted_statuses() -> Vec<u16> {
    DEFAULT_ACCEPTED_STATUSES.to_vec()
}

const fn default_download_bytes_limit() -> usize {
    DEFAULT_DOWNLOAD_BYTES_LIMIT
}

const fn default_clean_offlines_after_days() -> u32 {
    DEFAULT_CLEAN_OFFLINES_AFTER_DAYS
}

const fn default_proxy_enabled() -> bool {
    DEFAULT_PROXY_ENABLED
}

const fn default_proxy_port() -> u16 {
    DEFAULT_PROXY_PORT
}

const fn default_proxy_discoverable() -> bool {
    DEFAULT_PROXY_DISCOVERABLE
}

fn default_proxy_health_check_url() -> String {
    DEFAULT_PROXY_HEALTH_CHECK_URL.to_string()
}

const fn default_proxy_health_check_interval() -> u64 {
    DEFAULT_PROXY_HEALTH_CHECK_INTERVAL
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::AppConfig;

    fn write_temp_config(extension: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-{}.{}",
            std::process::id(),
            extension
        ));
        fs::write(
            &path,
            r"
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        )
        .expect("temp config can be written");
        path
    }

    #[test]
    fn loads_yml_config() {
        let path = write_temp_config("yml");
        let config = AppConfig::load(&path).expect("yml config loads");
        fs::remove_file(&path).ok();

        assert_eq!(config.subscriptions.len(), 1);
        assert_eq!(config.subscriptions[0].name, "local");
    }

    #[test]
    fn rejects_unknown_config_extension() {
        let path = write_temp_config("toml");
        let error = AppConfig::load(&path).expect_err("unsupported extension should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("unsupported config extension"));
    }

    #[test]
    fn rejects_zero_probe_timeout() {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-zero-timeout-{}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            r"
probe:
    active_timeout_ms: 0
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        )
        .expect("temp config can be written");
        let error = AppConfig::load(&path).expect_err("zero timeout should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("active_timeout_ms"));
    }

    #[test]
    fn rejects_zero_probe_batch_size() {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-zero-batch-{}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            r"
probe:
    batch_size: 0
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        )
        .expect("temp config can be written");
        let error = AppConfig::load(&path).expect_err("zero batch size should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("batch_size"));
    }

    #[test]
    fn rejects_invalid_http_status() {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-status-{}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            r"
probe:
    accepted_statuses: [99]
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        )
        .expect("temp config can be written");
        let error = AppConfig::load(&path).expect_err("invalid HTTP status should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("valid HTTP status codes"));
    }

    #[test]
    fn accepts_null_probe_and_sharing_strings() {
        let config = load_inline_config(
            "null-values",
            r"
sharing:
    token: null
probe:
    sing_box_path: null
    download_url: null
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        );

        assert_eq!(config.sharing.token, "");
        assert_eq!(config.probe.sing_box_path, "");
        assert_eq!(config.probe.download_url, None);
    }

    #[test]
    fn accepts_empty_probe_and_sharing_strings() {
        let config = load_inline_config(
            "empty-values",
            r#"
sharing:
    token: ""
probe:
    sing_box_path: ""
    download_url: ""
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
"#,
        );

        assert_eq!(config.sharing.token, "");
        assert_eq!(config.probe.sing_box_path, "");
        assert_eq!(config.probe.download_url, None);
    }

    #[test]
    fn normalizes_legacy_literal_null_strings() {
        let config = load_inline_config(
            "literal-null-values",
            r#"
sharing:
    token: "null"
probe:
    sing_box_path: "null"
    download_url: "null"
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
"#,
        );

        assert_eq!(config.sharing.token, "");
        assert_eq!(config.probe.sing_box_path, "");
        assert_eq!(config.probe.download_url, None);
    }

    #[test]
    fn default_config_keeps_token_null() {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-default-token-{}.yaml",
            std::process::id()
        ));
        AppConfig::write_default(&path).expect("default config writes");
        let saved = fs::read_to_string(&path).expect("default config can be read");
        let config = AppConfig::load(&path).expect("default config loads");
        fs::remove_file(&path).ok();

        assert!(saved.contains("  token: null"));
        assert_eq!(config.sharing.token, "");
        assert_eq!(
            config.subscription_url("127.0.0.1", false),
            "http://127.0.0.1:27141/subscription"
        );
    }

    #[test]
    fn token_true_generates_token_and_requests_persistence() {
        let (config, generated) = load_inline_config_with_generation_flag(
            "token-true",
            r"
sharing:
    token: true
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        );

        assert!(generated);
        assert!(!config.sharing.token.is_empty());
        assert!(
            config
                .subscription_url("127.0.0.1", false)
                .contains("?token=")
        );
    }

    #[test]
    fn string_token_is_used_in_subscription_url() {
        let (config, generated) = load_inline_config_with_generation_flag(
            "token-string",
            r"
sharing:
    token: user-token
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        );

        assert!(!generated);
        assert_eq!(config.sharing.token, "user-token");
        assert_eq!(
            config.subscription_url("127.0.0.1", false),
            "http://127.0.0.1:27141/subscription?token=user-token"
        );
    }

    #[test]
    fn require_token_rejects_null_token() {
        let path = write_inline_config(
            "require-token-null",
            r"
sharing:
    require_token: true
    token: null
subscriptions:
    - name: local
      url: data:,vless://uuid@example.com:443%23demo
",
        );

        let error = AppConfig::load(&path).expect_err("missing required token should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("sharing.token"));
    }

    fn load_inline_config(name: &str, content: &str) -> AppConfig {
        let path = write_inline_config(name, content);
        let config = AppConfig::load(&path).expect("config loads");
        fs::remove_file(&path).ok();
        config
    }

    fn load_inline_config_with_generation_flag(name: &str, content: &str) -> (AppConfig, bool) {
        let path = write_inline_config(name, content);
        let result = AppConfig::load_with_generated_token_flag(&path).expect("config loads");
        fs::remove_file(&path).ok();
        result
    }

    fn write_inline_config(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-{name}-{}.yaml",
            std::process::id()
        ));
        fs::write(&path, content).expect("temp config can be written");
        path
    }
}
