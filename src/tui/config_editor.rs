use std::net::SocketAddr;

use anyhow::{Result, anyhow};

use crate::{config::ProbeMode, sing_box};

use super::state::ConfigKey;

pub fn label(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::Bind => "bind",
        ConfigKey::TopN => "top_n",
        ConfigKey::RefreshSeconds => "refresh_seconds",
        ConfigKey::EncodedSubscription => "encoded_subscription",
        ConfigKey::PrioritizeStability => "prioritize_stability",
        ConfigKey::FetchTimeout => "fetch_timeout_ms",
        ConfigKey::FetchConcurrency => "fetch_concurrency",
        ConfigKey::MaxSubscriptionBytes => "max_subscription_bytes",
        ConfigKey::ProbeMode => "probe.mode",
        ConfigKey::SingBoxPath => "probe.sing_box_path",
        ConfigKey::ConnectTimeout => "probe.connect_timeout_ms",
        ConfigKey::ActiveTimeout => "probe.active_timeout_ms",
        ConfigKey::StartupTimeout => "probe.startup_timeout_ms",
        ConfigKey::ProbeConcurrency => "probe.concurrency",
        ConfigKey::ProbeBatchSize => "probe.batch_size",
        ConfigKey::TestUrl => "probe.test_url",
        ConfigKey::AcceptedStatuses => "probe.accepted_statuses",
        ConfigKey::DownloadUrl => "probe.download_url",
        ConfigKey::DownloadLimit => "probe.download_bytes_limit",
        ConfigKey::TokenRequired => "sharing.require_token",
        ConfigKey::Token => "sharing.token",
        ConfigKey::ResetDefaults => "reset to defaults",
    }
}

pub fn guide(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::Bind => "host:port, e.g. 0.0.0.0:27141",
        ConfigKey::TopN => "positive number, e.g. 10",
        ConfigKey::RefreshSeconds => "seconds between refreshes",
        ConfigKey::EncodedSubscription => "true/false for base64 feed",
        ConfigKey::PrioritizeStability => {
            "true favors repeat working configs; false favors short wins"
        }
        ConfigKey::FetchTimeout => "fetch timeout in ms",
        ConfigKey::FetchConcurrency => "parallel fetch count",
        ConfigKey::MaxSubscriptionBytes => "max bytes per subscription",
        ConfigKey::ProbeMode => "active or tcp",
        ConfigKey::SingBoxPath => "full path to sing-box executable",
        ConfigKey::ConnectTimeout => "connect timeout in ms",
        ConfigKey::ActiveTimeout => "active probe timeout in ms",
        ConfigKey::StartupTimeout => "sing-box startup timeout ms",
        ConfigKey::ProbeConcurrency => "parallel probe count",
        ConfigKey::ProbeBatchSize => "configs per sing-box process; auto/null",
        ConfigKey::TestUrl => "URL used for active probe",
        ConfigKey::AcceptedStatuses => "HTTP codes, e.g. 204,200",
        ConfigKey::DownloadUrl => "speedtest URL or off/null",
        ConfigKey::DownloadLimit => "speedtest byte limit",
        ConfigKey::TokenRequired => "true/false for URL token",
        ConfigKey::Token => "token text, empty allowed",
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
        ConfigKey::FetchTimeout => config.fetch_timeout_ms.to_string(),
        ConfigKey::FetchConcurrency => config.fetch_concurrency.to_string(),
        ConfigKey::MaxSubscriptionBytes => config.max_subscription_bytes.to_string(),
        ConfigKey::ProbeMode => format!("{:?}", config.probe.mode).to_ascii_lowercase(),
        ConfigKey::SingBoxPath => config.probe.sing_box_path.clone(),
        ConfigKey::ConnectTimeout => config.probe.connect_timeout_ms.to_string(),
        ConfigKey::ActiveTimeout => config.probe.active_timeout_ms.to_string(),
        ConfigKey::StartupTimeout => config.probe.startup_timeout_ms.to_string(),
        ConfigKey::ProbeConcurrency => config.probe.concurrency.to_string(),
        ConfigKey::ProbeBatchSize => config
            .probe
            .batch_size
            .map(|value| value.to_string())
            .unwrap_or_else(|| "auto".to_string()),
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
        ConfigKey::TokenRequired => config.sharing.require_token.to_string(),
        ConfigKey::Token => config.sharing.token.clone(),
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
        ConfigKey::FetchTimeout => config.fetch_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::FetchConcurrency => config.fetch_concurrency = positive(value, label(key))?,
        ConfigKey::MaxSubscriptionBytes => {
            config.max_subscription_bytes = positive(value, label(key))?
        }
        ConfigKey::ProbeMode => config.probe.mode = probe_mode(value)?,
        ConfigKey::SingBoxPath => config.probe.sing_box_path = sing_box::normalize_path(value),
        ConfigKey::ConnectTimeout => config.probe.connect_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::ActiveTimeout => config.probe.active_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::StartupTimeout => config.probe.startup_timeout_ms = nonzero(value, label(key))?,
        ConfigKey::ProbeConcurrency => config.probe.concurrency = positive(value, label(key))?,
        ConfigKey::ProbeBatchSize => {
            config.probe.batch_size = optional_positive(value, label(key))?
        }
        ConfigKey::TestUrl => config.probe.test_url = required(value, label(key))?,
        ConfigKey::AcceptedStatuses => config.probe.accepted_statuses = statuses(value)?,
        ConfigKey::DownloadUrl => config.probe.download_url = optional(value),
        ConfigKey::DownloadLimit => {
            config.probe.download_bytes_limit = positive(value, label(key))?
        }
        ConfigKey::TokenRequired => config.sharing.require_token = bool_value(value)?,
        ConfigKey::Token => config.sharing.token = value.to_string(),
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
    match value.to_ascii_lowercase().as_str() {
        "" | "off" | "none" | "null" => None,
        _ => Some(value.to_string()),
    }
}

fn optional_positive(value: &str, label: &str) -> Result<Option<usize>> {
    match value.to_ascii_lowercase().as_str() {
        "" | "auto" | "off" | "none" | "null" => Ok(None),
        _ => positive(value, label).map(Some),
    }
}
