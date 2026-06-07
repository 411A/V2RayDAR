use std::{fs, net::SocketAddr, path::Path};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    #[serde(default = "default_refresh_seconds")]
    pub refresh_seconds: u64,
    #[serde(default = "default_encoded_subscription")]
    pub encoded_subscription: bool,
    #[serde(default = "default_fetch_timeout_ms")]
    pub fetch_timeout_ms: u64,
    #[serde(default = "default_fetch_concurrency")]
    pub fetch_concurrency: usize,
    #[serde(default = "default_max_subscription_bytes")]
    pub max_subscription_bytes: usize,
    #[serde(default)]
    pub probe: ProbeConfig,
    #[serde(default)]
    pub sharing: SharingConfig,
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
    #[serde(default = "default_sing_box_path")]
    pub sing_box_path: String,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_active_timeout_ms")]
    pub active_timeout_ms: u64,
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    #[serde(default = "default_probe_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_test_url")]
    pub test_url: String,
    #[serde(default = "default_accepted_statuses")]
    pub accepted_statuses: Vec<u16>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default = "default_download_bytes_limit")]
    pub download_bytes_limit: usize,
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
    #[serde(default = "default_sharing_token")]
    pub token: String,
}

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
        help: "127.0.0.1 stays private. 0.0.0.0 or a LAN IP allows nearby devices.",
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
        help: "Auto-generated on first run. Regenerate if the URL was shared too widely.",
    },
    SettingGuide {
        key: "encoded_subscription",
        label: "Encoded feed",
        help: "Use base64 for v2rayN/v2rayNG. Use .txt for a raw link list.",
    },
    SettingGuide {
        key: "probe.mode",
        label: "Validation mode",
        help: "Active uses sing-box for real checks. TCP is diagnostic only.",
    },
    SettingGuide {
        key: "probe.sing_box_path",
        label: "sing-box path",
        help: "Use a full path if sing-box is not available in your terminal PATH.",
    },
];

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            mode: default_probe_mode(),
            sing_box_path: default_sing_box_path(),
            connect_timeout_ms: default_connect_timeout_ms(),
            active_timeout_ms: default_active_timeout_ms(),
            startup_timeout_ms: default_startup_timeout_ms(),
            concurrency: default_probe_concurrency(),
            test_url: default_test_url(),
            accepted_statuses: default_accepted_statuses(),
            download_url: None,
            download_bytes_limit: default_download_bytes_limit(),
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

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
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
                    "unsupported config extension '.{}'; use .yaml, .yml, or .json",
                    other
                ));
            }
        };

        validate(config)
    }

    pub fn default_for_first_run() -> Self {
        Self {
            bind: default_bind(),
            top_n: default_top_n(),
            refresh_seconds: default_refresh_seconds(),
            encoded_subscription: default_encoded_subscription(),
            fetch_timeout_ms: default_fetch_timeout_ms(),
            fetch_concurrency: default_fetch_concurrency(),
            max_subscription_bytes: default_max_subscription_bytes(),
            probe: ProbeConfig::default(),
            sharing: SharingConfig {
                token: generate_token(),
                ..SharingConfig::default()
            },
            subscriptions: Vec::new(),
        }
    }

    pub fn write_default(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("unable to create {}", parent.display()))?;
        }

        let content = serde_yaml::to_string(&Self::default_for_first_run())
            .context("unable to serialize default config")?;
        fs::write(path, content)
            .with_context(|| format!("unable to write default config to {}", path.display()))
    }

    pub fn subscription_url(&self, host: &str, raw: bool) -> String {
        let endpoint = if raw {
            "subscription.txt"
        } else {
            "subscription"
        };
        let mut url = format!("http://{}:{}/{}", host, self.bind.port(), endpoint);

        if self.sharing.require_token {
            url.push_str("?token=");
            url.push_str(&self.sharing.token);
        }

        url
    }
}

fn validate(config: AppConfig) -> Result<AppConfig> {
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

    Ok(config)
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
    "127.0.0.1:14127"
        .parse()
        .expect("default bind address is valid")
}

fn default_top_n() -> usize {
    10
}

fn default_refresh_seconds() -> u64 {
    300
}

fn default_encoded_subscription() -> bool {
    true
}

fn default_sharing_enabled() -> bool {
    false
}

fn default_require_token() -> bool {
    false
}

fn default_sharing_token() -> String {
    String::new()
}

fn default_fetch_timeout_ms() -> u64 {
    15_000
}

fn default_fetch_concurrency() -> usize {
    16
}

fn default_max_subscription_bytes() -> usize {
    16 * 1024 * 1024
}

fn default_priority() -> u32 {
    100
}

fn default_subscription_enabled() -> bool {
    true
}

fn default_probe_mode() -> ProbeMode {
    ProbeMode::Active
}

fn default_sing_box_path() -> String {
    String::new()
}

fn default_connect_timeout_ms() -> u64 {
    1_500
}

fn default_active_timeout_ms() -> u64 {
    10_000
}

fn default_startup_timeout_ms() -> u64 {
    2_000
}

fn default_probe_concurrency() -> usize {
    16
}

fn default_test_url() -> String {
    "https://www.gstatic.com/generate_204".to_string()
}

fn default_accepted_statuses() -> Vec<u16> {
    vec![204, 200]
}

fn default_download_bytes_limit() -> usize {
    1_048_576
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
            r#"
subscriptions:
  - name: local
    url: data:,vless://uuid@example.com:443%23demo
"#,
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
            r#"
probe:
  active_timeout_ms: 0
subscriptions:
  - name: local
    url: data:,vless://uuid@example.com:443%23demo
"#,
        )
        .expect("temp config can be written");
        let error = AppConfig::load(&path).expect_err("zero timeout should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("active_timeout_ms"));
    }

    #[test]
    fn rejects_invalid_http_status() {
        let path = std::env::temp_dir().join(format!(
            "v2raydar-config-test-status-{}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"
probe:
  accepted_statuses: [99]
subscriptions:
  - name: local
    url: data:,vless://uuid@example.com:443%23demo
"#,
        )
        .expect("temp config can be written");
        let error = AppConfig::load(&path).expect_err("invalid HTTP status should fail");
        fs::remove_file(&path).ok();

        assert!(error.to_string().contains("valid HTTP status codes"));
    }
}
