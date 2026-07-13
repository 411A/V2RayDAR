use std::net::SocketAddr;

use anyhow::{Result, anyhow};

use crate::{
    config::{ProbeMode, normalize_sharing_token},
    sing_box,
};

use super::state::ConfigKey;

pub const fn label(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::Bind => "bind",
        ConfigKey::TopN => "top_n",
        ConfigKey::RefreshSeconds => "refresh_seconds",
        ConfigKey::EncodedSubscription => "encoded_subscription",
        ConfigKey::PrioritizeStability => "prioritize_stability",
        ConfigKey::ReturnConfigsAsap => "return_configs_asap",
        ConfigKey::ScanAllConfigs => "scan_all_configs",
        ConfigKey::FetchTimeout => "fetch_timeout_ms",
        ConfigKey::FetchConcurrency => "fetch_concurrency",
        ConfigKey::MaxSubscriptionBytes => "max_subscription_bytes",
        ConfigKey::UseCacheOnly => "use_cache_only",
        ConfigKey::EmergencyConfig => "emergency_config",
        ConfigKey::ProbeMode => "probe.mode",
        ConfigKey::SingBoxPath => "probe.sing_box_path",
        ConfigKey::ConnectTimeout => "probe.connect_timeout_ms",
        ConfigKey::ActiveTimeout => "probe.active_timeout_ms",
        ConfigKey::StartupTimeout => "probe.startup_timeout_ms",
        ConfigKey::ProbeConcurrency => "probe.concurrency",
        ConfigKey::ProbeBatchSize => "probe.batch_size",
        ConfigKey::ProbeProcessConcurrency => "probe.process_concurrency",
        ConfigKey::TestUrl => "probe.test_url",
        ConfigKey::AcceptedStatuses => "probe.accepted_statuses",
        ConfigKey::DownloadUrl => "probe.download_url",
        ConfigKey::DownloadLimit => "probe.download_bytes_limit",
        ConfigKey::CleanOfflineDays => "clean_offlines_after_days",
        ConfigKey::TokenRequired => "sharing.require_token",
        ConfigKey::Token => "sharing.token",
        ConfigKey::ProxyEnabled => "proxy.enabled",
        ConfigKey::ProxyPort => "proxy.port",
        ConfigKey::ProxyDiscoverable => "proxy.discoverable",
        ConfigKey::ProxyHealthCheckUrl => "proxy.health_check_url",
        ConfigKey::ProxyHealthCheckInterval => "proxy.health_check_interval_seconds",
        ConfigKey::ResetDefaults => "reset to defaults",
    }
}

pub const fn guide(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::Bind => "host:port, e.g. 0.0.0.0:27141",
        ConfigKey::TopN => "positive number, e.g. 10",
        ConfigKey::RefreshSeconds => "seconds between refreshes",
        ConfigKey::EncodedSubscription => "true/false for base64 feed",
        ConfigKey::PrioritizeStability => {
            "true favors repeat working configs; false favors short wins"
        }
        ConfigKey::ReturnConfigsAsap => "true publishes working configs immediately",
        ConfigKey::ScanAllConfigs => {
            "true scans every config; false stops after enough working configs"
        }
        ConfigKey::FetchTimeout => "fetch timeout in ms",
        ConfigKey::FetchConcurrency => "parallel fetch count",
        ConfigKey::MaxSubscriptionBytes => "max bytes per subscription",
        ConfigKey::UseCacheOnly => "true uses cached subscriptions only",
        ConfigKey::EmergencyConfig => "null or direct share link",
        ConfigKey::ProbeMode => "active or tcp",
        ConfigKey::SingBoxPath => "full path to sing-box executable",
        ConfigKey::ConnectTimeout => "connect timeout in ms",
        ConfigKey::ActiveTimeout => "active probe timeout in ms",
        ConfigKey::StartupTimeout => "sing-box startup timeout ms",
        ConfigKey::ProbeConcurrency => "parallel probe count",
        ConfigKey::ProbeBatchSize => "configs per sing-box process; auto/null",
        ConfigKey::ProbeProcessConcurrency => "parallel sing-box processes; auto/null",
        ConfigKey::TestUrl => "URL used for active probe",
        ConfigKey::AcceptedStatuses => "HTTP codes, e.g. 204,200",
        ConfigKey::DownloadUrl => "speedtest URL or off/null",
        ConfigKey::DownloadLimit => "speedtest byte limit",
        ConfigKey::CleanOfflineDays => "days before offline configs are removed",
        ConfigKey::TokenRequired => "true/false for URL token",
        ConfigKey::Token => "token text, empty allowed",
        ConfigKey::ProxyEnabled => "true/false to enable persistent proxy",
        ConfigKey::ProxyPort => "mixed SOCKS5/HTTP proxy port",
        ConfigKey::ProxyDiscoverable => "true binds 0.0.0.0 + firewall for LAN",
        ConfigKey::ProxyHealthCheckUrl => "URL tested through the proxy",
        ConfigKey::ProxyHealthCheckInterval => "seconds between health checks",
        ConfigKey::ResetDefaults => "type shown code to reset",
    }
}

pub fn value(config: &crate::config::AppConfig, key: ConfigKey) -> String {
    match key {
        ConfigKey::Bind => config.bind.to_string(),
        ConfigKey::TopN => config.top_n.to_string(),
        ConfigKey::RefreshSeconds => config.refresh_seconds.to_string(),
        ConfigKey::EncodedSubscription => config.encoded_subscription.to_string(),
        ConfigKey::PrioritizeStability => config.prioritize_stability.to_string(),
        ConfigKey::ReturnConfigsAsap => config.return_configs_asap.to_string(),
        ConfigKey::ScanAllConfigs => config.scan_all_configs.to_string(),
        ConfigKey::FetchTimeout => config.fetch_timeout_ms.to_string(),
        ConfigKey::FetchConcurrency => config.fetch_concurrency.to_string(),
        ConfigKey::MaxSubscriptionBytes => config.max_subscription_bytes.to_string(),
        ConfigKey::UseCacheOnly => config.use_cache_only.to_string(),
        ConfigKey::EmergencyConfig => config.emergency_config.clone().unwrap_or_default(),
        ConfigKey::ProbeMode => format!("{:?}", config.probe.mode).to_ascii_lowercase(),
        ConfigKey::SingBoxPath => config.probe.sing_box_path.clone(),
        ConfigKey::ConnectTimeout => config.probe.connect_timeout_ms.to_string(),
        ConfigKey::ActiveTimeout => config.probe.active_timeout_ms.to_string(),
        ConfigKey::StartupTimeout => config.probe.startup_timeout_ms.to_string(),
        ConfigKey::ProbeConcurrency => config.probe.concurrency.to_string(),
        ConfigKey::ProbeBatchSize => config
            .probe
            .batch_size
            .map_or_else(|| "auto".to_string(), |value| value.to_string()),
        ConfigKey::ProbeProcessConcurrency => config
            .probe
            .process_concurrency
            .map_or_else(|| "auto".to_string(), |value| value.to_string()),
        ConfigKey::TestUrl => config.probe.test_url.clone(),
        ConfigKey::AcceptedStatuses => config
            .probe
            .accepted_statuses
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(","),
        ConfigKey::DownloadUrl => config
            .probe
            .download_url
            .clone()
            .unwrap_or_else(|| "off".into()),
        ConfigKey::DownloadLimit => config.probe.download_bytes_limit.to_string(),
        ConfigKey::CleanOfflineDays => config.clean_offlines_after_days.to_string(),
        ConfigKey::TokenRequired => config.sharing.require_token.to_string(),
        ConfigKey::Token => config.sharing.token.clone(),
        ConfigKey::ProxyEnabled => config.proxy.enabled.to_string(),
        ConfigKey::ProxyPort => config.proxy.port.to_string(),
        ConfigKey::ProxyDiscoverable => config.proxy.discoverable.to_string(),
        ConfigKey::ProxyHealthCheckUrl => config.proxy.health_check_url.clone(),
        ConfigKey::ProxyHealthCheckInterval => {
            config.proxy.health_check_interval_seconds.to_string()
        }
        ConfigKey::ResetDefaults => "keeps subscriptions".to_string(),
    }
}

pub fn apply(config: &mut crate::config::AppConfig, key: ConfigKey, raw: &str) -> Result<()> {
    let value = raw.trim();
    match key {
        ConfigKey::Bind => config.bind = value.parse::<SocketAddr>()?,
        ConfigKey::TopN => config.top_n = positive(value, "top_n")?,
        ConfigKey::RefreshSeconds => config.refresh_seconds = value.parse()?,
        ConfigKey::EncodedSubscription => config.encoded_subscription = bool_value(value)?,
        ConfigKey::PrioritizeStability => config.prioritize_stability = bool_value(value)?,
        ConfigKey::ReturnConfigsAsap => config.return_configs_asap = bool_value(value)?,
        ConfigKey::ScanAllConfigs => config.scan_all_configs = bool_value(value)?,
        ConfigKey::FetchTimeout => config.fetch_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::FetchConcurrency => config.fetch_concurrency = positive(value, label(key))?,
        ConfigKey::MaxSubscriptionBytes => {
            config.max_subscription_bytes = positive(value, label(key))?;
        }
        ConfigKey::UseCacheOnly => config.use_cache_only = bool_value(value)?,
        ConfigKey::EmergencyConfig => config.emergency_config = optional(value),
        ConfigKey::ProbeMode => config.probe.mode = probe_mode(value)?,
        ConfigKey::SingBoxPath => {
            config.probe.sing_box_path = optional_string(value);
            config.probe.sing_box_path_auto = false;
        }
        ConfigKey::ConnectTimeout => config.probe.connect_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::ActiveTimeout => config.probe.active_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::StartupTimeout => config.probe.startup_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::ProbeConcurrency => config.probe.concurrency = positive(value, label(key))?,
        ConfigKey::ProbeBatchSize => {
            config.probe.batch_size = optional_positive(value, label(key))?;
        }
        ConfigKey::ProbeProcessConcurrency => {
            config.probe.process_concurrency = optional_positive(value, label(key))?;
        }
        ConfigKey::TestUrl => config.probe.test_url = required(value, label(key))?,
        ConfigKey::AcceptedStatuses => config.probe.accepted_statuses = statuses(value)?,
        ConfigKey::DownloadUrl => config.probe.download_url = optional(value),
        ConfigKey::DownloadLimit => {
            config.probe.download_bytes_limit = positive(value, label(key))?;
        }
        ConfigKey::CleanOfflineDays => {
            config.clean_offlines_after_days = positive(value, label(key))?;
        }
        ConfigKey::TokenRequired => config.sharing.require_token = bool_value(value)?,
        ConfigKey::Token => config.sharing.token = normalize_sharing_token(value),
        ConfigKey::ProxyEnabled => config.proxy.enabled = bool_value(value)?,
        ConfigKey::ProxyPort => config.proxy.port = positive(value, label(key))?,
        ConfigKey::ProxyDiscoverable => config.proxy.discoverable = bool_value(value)?,
        ConfigKey::ProxyHealthCheckUrl => {
            config.proxy.health_check_url = required(value, label(key))?;
        }
        ConfigKey::ProxyHealthCheckInterval => {
            config.proxy.health_check_interval_seconds = nonzero(value, label(key))?;
        }
        ConfigKey::ResetDefaults => {}
    }
    Ok(())
}

fn bool_value(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(anyhow!("expected true/false")),
    }
}

fn positive<T>(value: &str, label: &str) -> Result<T>
where
    T: std::str::FromStr + PartialOrd + From<u8>,
{
    let parsed = value
        .parse::<T>()
        .map_err(|_| anyhow!("{label} must be a number"))?;
    if parsed > T::from(0) {
        Ok(parsed)
    } else {
        Err(anyhow!("{label} must be greater than 0"))
    }
}

fn nonzero(value: &str, label: &str) -> Result<u64> {
    positive(value, label)
}

fn probe_mode(value: &str) -> Result<ProbeMode> {
    match value.to_ascii_lowercase().as_str() {
        "active" => Ok(ProbeMode::Active),
        "tcp" => Ok(ProbeMode::Tcp),
        _ => Err(anyhow!("probe.mode must be active or tcp")),
    }
}

fn required(value: &str, label: &str) -> Result<String> {
    if value.is_empty() {
        Err(anyhow!("{label} cannot be empty"))
    } else {
        Ok(value.to_string())
    }
}

fn statuses(value: &str) -> Result<Vec<u16>> {
    let parsed = value
        .split(',')
        .map(|part| part.trim().parse::<u16>())
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.iter().all(|status| (100..=599).contains(status)) {
        Ok(parsed)
    } else {
        Err(anyhow!("accepted_statuses must be HTTP codes 100..599"))
    }
}

fn optional(value: &str) -> Option<String> {
    let value = optional_string(value);
    match value.to_ascii_lowercase().as_str() {
        "" | "off" | "none" => None,
        _ => Some(value),
    }
}

fn optional_string(value: &str) -> String {
    let value = sing_box::normalize_path(value);
    if value.eq_ignore_ascii_case("null") {
        String::new()
    } else {
        value
    }
}

fn optional_positive(value: &str, label: &str) -> Result<Option<usize>> {
    match value.to_ascii_lowercase().as_str() {
        "" | "auto" | "off" | "none" | "null" => Ok(None),
        _ => positive(value, label).map(Some),
    }
}
