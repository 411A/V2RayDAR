use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result};

use crate::{
    config::{AppConfig, ProbeMode, SubscriptionSource},
    constants::{BYTE_UNITS, BYTES_PER_UNIT},
};

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension == "json" {
        save_json_config(path, config)
    } else {
        save_yaml_config(path, config)
    }
}

/// Load the on-disk config, merge with TUI edits, and save.
pub fn save_merged(path: &Path, startup: &AppConfig, tui: &AppConfig) -> Result<()> {
    let merged = AppConfig::load(path)
        .map_or_else(|_| tui.clone(), |disk| merge_for_save(startup, tui, &disk));
    save_config(path, &merged)
}

/// Merge in-memory (TUI) edits with on-disk values.
///
/// For each field: if the TUI changed it (differs from startup), keep the TUI value.
/// Otherwise pick up the on-disk value so manual file edits are never overwritten.
pub fn merge_for_save(startup: &AppConfig, tui: &AppConfig, disk: &AppConfig) -> AppConfig {
    let mut merged = disk.clone();
    merge_top_level(&mut merged, startup, tui);
    merge_section_sharing(&mut merged, startup, tui);
    merge_section_proxy(&mut merged, startup, tui);
    merge_section_probe(&mut merged, startup, tui);
    merged.subscriptions.clone_from(&tui.subscriptions);
    merged
}

fn merge_top_level(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.$field != startup.$field {
                merged.$field.clone_from(&tui.$field);
            }
        };
        (copy $field:ident) => {
            if tui.$field != startup.$field {
                merged.$field = tui.$field;
            }
        };
    }
    pick!(copy bind);
    pick!(copy top_n);
    pick!(copy refresh_seconds);
    pick!(copy ping_seconds);
    pick!(copy encoded_subscription);
    pick!(copy prioritize_stability);
    pick!(copy return_configs_asap);
    pick!(copy scan_all_configs);
    pick!(copy fetch_timeout_ms);
    pick!(copy fetch_concurrency);
    pick!(copy max_subscription_bytes);
    pick!(copy use_cache_only);
    pick!(emergency_config);
    pick!(copy clean_offlines_after_days);
}

fn merge_section_sharing(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.sharing.$field != startup.sharing.$field {
                merged.sharing.$field.clone_from(&tui.sharing.$field);
            }
        };
        (copy $field:ident) => {
            if tui.sharing.$field != startup.sharing.$field {
                merged.sharing.$field = tui.sharing.$field;
            }
        };
    }
    pick!(copy enabled);
    pick!(copy require_token);
    pick!(token);
}

fn merge_section_proxy(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.proxy.$field != startup.proxy.$field {
                merged.proxy.$field.clone_from(&tui.proxy.$field);
            }
        };
        (copy $field:ident) => {
            if tui.proxy.$field != startup.proxy.$field {
                merged.proxy.$field = tui.proxy.$field;
            }
        };
    }
    pick!(copy enabled);
    pick!(copy port);
    pick!(copy discoverable);
    pick!(copy rotating_proxy);
    pick!(health_check_url);
    pick!(copy health_check_interval_seconds);
}

fn merge_section_probe(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.probe.$field != startup.probe.$field {
                merged.probe.$field.clone_from(&tui.probe.$field);
            }
        };
        (copy $field:ident) => {
            if tui.probe.$field != startup.probe.$field {
                merged.probe.$field = tui.probe.$field;
            }
        };
    }
    pick!(copy mode);
    if tui.probe.sing_box_path != startup.probe.sing_box_path {
        merged
            .probe
            .sing_box_path
            .clone_from(&tui.probe.sing_box_path);
        merged.probe.sing_box_path_auto = tui.probe.sing_box_path_auto;
    }
    pick!(copy connect_timeout_ms);
    pick!(copy active_timeout_ms);
    pick!(copy startup_timeout_ms);
    pick!(copy concurrency);
    pick!(copy batch_size);
    pick!(copy process_concurrency);
    pick!(test_url);
    pick!(accepted_statuses);
    pick!(download_url);
    pick!(copy download_bytes_limit);
}

fn save_json_config(path: &Path, config: &AppConfig) -> Result<()> {
    let config = persistable_config(config);
    let content = serde_json::to_string_pretty(&config).context("unable to serialize config")?;
    fs::write(path, format!("{content}\n"))
        .with_context(|| format!("unable to write config to {}", path.display()))?;
    restrict_file_permissions(path);
    Ok(())
}

fn save_yaml_config(path: &Path, config: &AppConfig) -> Result<()> {
    let config = persistable_config(config);
    let original = fs::read_to_string(path)
        .with_context(|| format!("unable to read existing config {}", path.display()))?;
    let previous = AppConfig::load(path)
        .with_context(|| format!("unable to parse existing config {}", path.display()))?;

    let mut document = YamlDocument::new(original);
    update_top_level_scalars(&mut document, &previous, &config);
    update_sharing_section(&mut document, &previous, &config);
    update_proxy_section(&mut document, &previous, &config);
    update_probe_section(&mut document, &previous, &config);
    if previous.subscriptions != config.subscriptions
        && !document.update_subscriptions(&previous.subscriptions, &config.subscriptions)
    {
        document.replace_top_level_section(
            "subscriptions",
            format_subscriptions_section(&config.subscriptions),
        );
    }

    fs::write(path, document.finish())
        .with_context(|| format!("unable to write config to {}", path.display()))?;
    restrict_file_permissions(path);
    Ok(())
}

/// Restrict a config file to owner-only (0600 on Unix).
/// Best-effort, no-op on Windows; single syscall, no UI delay.
#[allow(clippy::missing_const_for_fn)]
fn restrict_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn persistable_config(config: &AppConfig) -> AppConfig {
    let mut config = config.clone();
    if config.probe.sing_box_path_auto {
        config.probe.sing_box_path.clear();
        config.probe.sing_box_path_auto = false;
    }
    config
}

fn update_top_level_scalars(document: &mut YamlDocument, previous: &AppConfig, config: &AppConfig) {
    if previous.bind != config.bind {
        document.set_top_level_scalar("bind", config.bind.to_string());
    }
    if previous.top_n != config.top_n {
        document.set_top_level_scalar("top_n", config.top_n.to_string());
    }
    if previous.refresh_seconds != config.refresh_seconds {
        document.set_top_level_scalar("refresh_seconds", config.refresh_seconds.to_string());
    }
    if previous.ping_seconds != config.ping_seconds {
        document.set_top_level_scalar("ping_seconds", config.ping_seconds.to_string());
    }
    if previous.encoded_subscription != config.encoded_subscription {
        document.set_top_level_scalar(
            "encoded_subscription",
            config.encoded_subscription.to_string(),
        );
    }
    if previous.prioritize_stability != config.prioritize_stability {
        document.set_top_level_scalar(
            "prioritize_stability",
            config.prioritize_stability.to_string(),
        );
    }
    if previous.return_configs_asap != config.return_configs_asap {
        document.set_top_level_scalar(
            "return_configs_asap",
            config.return_configs_asap.to_string(),
        );
    }
    if previous.scan_all_configs != config.scan_all_configs {
        document.set_top_level_scalar("scan_all_configs", config.scan_all_configs.to_string());
    }
    if previous.fetch_timeout_ms != config.fetch_timeout_ms {
        document.set_top_level_scalar("fetch_timeout_ms", config.fetch_timeout_ms.to_string());
    }
    if previous.fetch_concurrency != config.fetch_concurrency {
        document.set_top_level_scalar("fetch_concurrency", config.fetch_concurrency.to_string());
    }
    if previous.max_subscription_bytes != config.max_subscription_bytes {
        document.set_top_level_scalar(
            "max_subscription_bytes",
            config.max_subscription_bytes.to_string(),
        );
    }
    if previous.use_cache_only != config.use_cache_only {
        document.set_top_level_scalar("use_cache_only", config.use_cache_only.to_string());
    }
    if previous.emergency_config != config.emergency_config {
        document.set_top_level_scalar(
            "emergency_config",
            config
                .emergency_config
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map_or_else(|| "null".to_string(), yaml_scalar),
        );
    }
    if previous.clean_offlines_after_days != config.clean_offlines_after_days {
        document.set_top_level_scalar(
            "clean_offlines_after_days",
            config.clean_offlines_after_days.to_string(),
        );
    }
    if previous.geoip_db_path != config.geoip_db_path {
        document.set_top_level_scalar(
            "geoip_db_path",
            config
                .geoip_db_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map_or_else(|| "null".to_string(), yaml_scalar),
        );
    }
}

fn update_sharing_section(document: &mut YamlDocument, previous: &AppConfig, config: &AppConfig) {
    if previous.sharing.enabled != config.sharing.enabled {
        document.set_nested_scalar("sharing", "enabled", config.sharing.enabled.to_string());
    }
    if previous.sharing.require_token != config.sharing.require_token {
        document.set_nested_scalar(
            "sharing",
            "require_token",
            config.sharing.require_token.to_string(),
        );
    }
    if previous.sharing.token != config.sharing.token {
        document.set_nested_scalar("sharing", "token", nullable_string(&config.sharing.token));
    }
}

fn update_proxy_section(document: &mut YamlDocument, previous: &AppConfig, config: &AppConfig) {
    if previous.proxy.enabled != config.proxy.enabled {
        document.set_nested_scalar("proxy", "enabled", config.proxy.enabled.to_string());
    }
    if previous.proxy.port != config.proxy.port {
        document.set_nested_scalar("proxy", "port", config.proxy.port.to_string());
    }
    if previous.proxy.discoverable != config.proxy.discoverable {
        document.set_nested_scalar(
            "proxy",
            "discoverable",
            config.proxy.discoverable.to_string(),
        );
    }
    if previous.proxy.rotating_proxy != config.proxy.rotating_proxy {
        document.set_nested_scalar(
            "proxy",
            "rotating_proxy",
            config.proxy.rotating_proxy.to_string(),
        );
    }
    if previous.proxy.health_check_url != config.proxy.health_check_url {
        document.set_nested_scalar(
            "proxy",
            "health_check_url",
            config.proxy.health_check_url.clone(),
        );
    }
    if previous.proxy.health_check_interval_seconds != config.proxy.health_check_interval_seconds {
        document.set_nested_scalar(
            "proxy",
            "health_check_interval_seconds",
            config.proxy.health_check_interval_seconds.to_string(),
        );
    }
}

fn update_probe_section(document: &mut YamlDocument, previous: &AppConfig, config: &AppConfig) {
    if previous.probe.mode != config.probe.mode {
        document.set_nested_scalar("probe", "mode", probe_mode(config.probe.mode));
    }
    if previous.probe.sing_box_path != config.probe.sing_box_path {
        document.set_nested_scalar(
            "probe",
            "sing_box_path",
            nullable_string(&config.probe.sing_box_path),
        );
    }
    if previous.probe.connect_timeout_ms != config.probe.connect_timeout_ms {
        document.set_nested_scalar(
            "probe",
            "connect_timeout_ms",
            config.probe.connect_timeout_ms.to_string(),
        );
    }
    if previous.probe.active_timeout_ms != config.probe.active_timeout_ms {
        document.set_nested_scalar(
            "probe",
            "active_timeout_ms",
            config.probe.active_timeout_ms.to_string(),
        );
    }
    if previous.probe.startup_timeout_ms != config.probe.startup_timeout_ms {
        document.set_nested_scalar(
            "probe",
            "startup_timeout_ms",
            config.probe.startup_timeout_ms.to_string(),
        );
    }
    if previous.probe.concurrency != config.probe.concurrency {
        document.set_nested_scalar("probe", "concurrency", config.probe.concurrency.to_string());
    }
    if previous.probe.batch_size != config.probe.batch_size {
        document.set_nested_scalar(
            "probe",
            "batch_size",
            config
                .probe
                .batch_size
                .map_or_else(|| "null".to_string(), |value| value.to_string()),
        );
    }
    if previous.probe.process_concurrency != config.probe.process_concurrency {
        document.set_nested_scalar(
            "probe",
            "process_concurrency",
            config
                .probe
                .process_concurrency
                .map_or_else(|| "null".to_string(), |value| value.to_string()),
        );
    }
    if previous.probe.test_url != config.probe.test_url {
        document.set_nested_scalar("probe", "test_url", yaml_scalar(&config.probe.test_url));
    }
    if previous.probe.accepted_statuses != config.probe.accepted_statuses {
        document.set_nested_scalar(
            "probe",
            "accepted_statuses",
            format_inline_u16_list(&config.probe.accepted_statuses),
        );
    }
    if previous.probe.download_url != config.probe.download_url {
        document.set_nested_scalar(
            "probe",
            "download_url",
            config
                .probe
                .download_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map_or_else(|| "null".to_string(), yaml_scalar),
        );
    }
    if previous.probe.download_bytes_limit != config.probe.download_bytes_limit {
        document.set_nested_scalar(
            "probe",
            "download_bytes_limit",
            config.probe.download_bytes_limit.to_string(),
        );
    }
}

/// The shipped example config, embedded at compile time: the single source
/// of truth for canonical key order. Editing `configs.example.yaml`
/// automatically moves backfill positions — no code lists to keep in sync.
const EXAMPLE_CONFIG: &str = include_str!("../../configs.example.yaml");

/// Canonical key order derived from the example config: top-level keys plus
/// nested keys per section, in file order.
#[derive(Debug, Default)]
struct ExampleKeyOrder {
    top: Vec<String>,
    nested: HashMap<String, Vec<String>>,
}

fn example_key_order() -> ExampleKeyOrder {
    let mut order = ExampleKeyOrder::default();
    let mut section: Option<String> = None;
    for line in EXAMPLE_CONFIG.lines() {
        let Some((indent, key)) = parse_yaml_key(line) else {
            continue;
        };
        if indent == 0 {
            if !order.top.iter().any(|existing| existing == key) {
                order.top.push(key.to_string());
            }
            section = Some(key.to_string());
        } else if indent == 2
            && let Some(section) = section.as_ref()
        {
            let keys = order.nested.entry(section.clone()).or_default();
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_string());
            }
        }
    }
    order
}

/// Backfill settings missing from an older `configs.yaml` with current defaults.
///
/// Runs once at startup (never from the file watcher): only *adds* absent
/// keys at their `configs.example.yaml` positions (derived from the embedded
/// example, never hardcoded), never modifies existing values, comments,
/// ordering, or the `subscriptions` list. Returns the number of added keys
/// and writes the file only when something was added, so mtime stays stable
/// otherwise (no watcher reload loop). YAML only — JSON configs already
/// round-trip every field on save. Unparseable input is an error (the caller
/// logs it and continues with in-memory defaults).
// Long by construction: one row per known setting, like the save helpers.
#[allow(clippy::too_many_lines)]
pub fn backfill_missing_defaults(path: &Path) -> Result<usize> {
    static NO_KEYS: &[String] = &[];
    if path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .eq_ignore_ascii_case("json")
    {
        return Ok(0);
    }

    let original = fs::read_to_string(path)
        .with_context(|| format!("unable to read existing config {}", path.display()))?;
    let parsed: serde_yaml::Value = serde_yaml::from_str(&original)
        .with_context(|| format!("unable to parse existing config {}", path.display()))?;
    let defaults = AppConfig::default_for_first_run();
    let canonical = example_key_order();

    let mut document = YamlDocument::new(&original);
    let mut added = 0;
    let mut ensure = |section: Option<&str>, key: &str, value: String| {
        if !mapping_has_key(&parsed, section, key) {
            match section {
                None => {
                    document.insert_missing_top_level(key, &value, &canonical.top);
                }
                Some(section) => document.insert_missing_nested(
                    section,
                    key,
                    &value,
                    canonical.nested.get(section).map_or(NO_KEYS, Vec::as_slice),
                    &canonical.top,
                ),
            }
            added += 1;
        }
    };
    let null_or = |value: Option<&str>| {
        value
            .filter(|value| !value.trim().is_empty())
            .map_or_else(|| "null".to_string(), yaml_scalar)
    };
    let maybe_number = |value: Option<usize>| {
        value.map_or_else(|| "null".to_string(), |number| number.to_string())
    };
    // (section, key, rendered default); `None` section means top level.
    // Positions come from the embedded example (see [`example_key_order`]).
    let entries: Vec<(Option<&str>, &str, String)> = vec![
        (None, "bind", defaults.bind.to_string()),
        (None, "top_n", defaults.top_n.to_string()),
        (
            None,
            "refresh_seconds",
            defaults.refresh_seconds.to_string(),
        ),
        (None, "ping_seconds", defaults.ping_seconds.to_string()),
        (
            None,
            "encoded_subscription",
            defaults.encoded_subscription.to_string(),
        ),
        (
            None,
            "prioritize_stability",
            defaults.prioritize_stability.to_string(),
        ),
        (
            None,
            "return_configs_asap",
            defaults.return_configs_asap.to_string(),
        ),
        (
            None,
            "scan_all_configs",
            defaults.scan_all_configs.to_string(),
        ),
        (
            None,
            "fetch_timeout_ms",
            defaults.fetch_timeout_ms.to_string(),
        ),
        (
            None,
            "fetch_concurrency",
            defaults.fetch_concurrency.to_string(),
        ),
        (
            None,
            "max_subscription_bytes",
            defaults.max_subscription_bytes.to_string(),
        ),
        (None, "use_cache_only", defaults.use_cache_only.to_string()),
        (
            None,
            "emergency_config",
            null_or(defaults.emergency_config.as_deref()),
        ),
        (
            None,
            "geoip_db_path",
            null_or(defaults.geoip_db_path.as_deref()),
        ),
        (
            None,
            "clean_offlines_after_days",
            defaults.clean_offlines_after_days.to_string(),
        ),
        (
            Some("sharing"),
            "enabled",
            defaults.sharing.enabled.to_string(),
        ),
        (
            Some("sharing"),
            "require_token",
            defaults.sharing.require_token.to_string(),
        ),
        (
            Some("sharing"),
            "token",
            nullable_string(&defaults.sharing.token),
        ),
        (Some("proxy"), "enabled", defaults.proxy.enabled.to_string()),
        (Some("proxy"), "port", defaults.proxy.port.to_string()),
        (
            Some("proxy"),
            "discoverable",
            defaults.proxy.discoverable.to_string(),
        ),
        (
            Some("proxy"),
            "rotating_proxy",
            defaults.proxy.rotating_proxy.to_string(),
        ),
        (
            Some("proxy"),
            "health_check_url",
            yaml_scalar(&defaults.proxy.health_check_url),
        ),
        (
            Some("proxy"),
            "health_check_interval_seconds",
            defaults.proxy.health_check_interval_seconds.to_string(),
        ),
        (Some("probe"), "mode", probe_mode(defaults.probe.mode)),
        (
            Some("probe"),
            "sing_box_path",
            nullable_string(&defaults.probe.sing_box_path),
        ),
        (
            Some("probe"),
            "connect_timeout_ms",
            defaults.probe.connect_timeout_ms.to_string(),
        ),
        (
            Some("probe"),
            "active_timeout_ms",
            defaults.probe.active_timeout_ms.to_string(),
        ),
        (
            Some("probe"),
            "startup_timeout_ms",
            defaults.probe.startup_timeout_ms.to_string(),
        ),
        (
            Some("probe"),
            "concurrency",
            defaults.probe.concurrency.to_string(),
        ),
        (
            Some("probe"),
            "batch_size",
            maybe_number(defaults.probe.batch_size),
        ),
        (
            Some("probe"),
            "process_concurrency",
            maybe_number(defaults.probe.process_concurrency),
        ),
        (
            Some("probe"),
            "test_url",
            yaml_scalar(&defaults.probe.test_url),
        ),
        (
            Some("probe"),
            "accepted_statuses",
            format_inline_u16_list(&defaults.probe.accepted_statuses),
        ),
        (
            Some("probe"),
            "download_url",
            null_or(defaults.probe.download_url.as_deref()),
        ),
        (
            Some("probe"),
            "download_bytes_limit",
            defaults.probe.download_bytes_limit.to_string(),
        ),
    ];
    for (section, key, value) in entries {
        ensure(section, key, value);
    }

    if added == 0 {
        return Ok(0);
    }
    fs::write(path, document.finish())
        .with_context(|| format!("unable to write config to {}", path.display()))?;
    restrict_file_permissions(path);
    Ok(added)
}

fn mapping_has_key(document: &serde_yaml::Value, section: Option<&str>, key: &str) -> bool {
    let target = match section {
        None => document,
        Some(section) => match document.get(serde_yaml::Value::String(section.to_string())) {
            Some(nested) => nested,
            None => return false,
        },
    };
    target
        .as_mapping()
        .is_some_and(|mapping| mapping.contains_key(serde_yaml::Value::String(key.to_string())))
}

struct YamlDocument {
    lines: Vec<String>,
    newline: &'static str,
    had_trailing_newline: bool,
}

impl YamlDocument {
    fn new(content: impl AsRef<str>) -> Self {
        let content = content.as_ref();
        let newline = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let had_trailing_newline = content.ends_with('\n');
        let lines = content.lines().map(ToString::to_string).collect();

        Self {
            lines,
            newline,
            had_trailing_newline,
        }
    }

    fn finish(self) -> String {
        let mut content = self.lines.join(self.newline);
        if self.had_trailing_newline && !content.is_empty() {
            content.push_str(self.newline);
        }
        content
    }

    fn set_top_level_scalar(&mut self, key: &str, value: impl AsRef<str>) {
        let value = value.as_ref();
        if let Some(index) = self.find_direct_key(key, 0, self.lines.len(), 0) {
            self.replace_scalar_line(index, key, value);
            return;
        }

        let insert_at = self
            .find_top_level_key("sharing")
            .or_else(|| self.find_top_level_key("probe"))
            .or_else(|| self.find_top_level_key("subscriptions"))
            .unwrap_or(self.lines.len());
        self.lines.insert(insert_at, format!("{key}: {value}"));
    }

    fn set_nested_scalar(&mut self, section: &str, key: &str, value: impl AsRef<str>) {
        let value = value.as_ref();
        if let Some((start, end, indent)) = self.section_range(section) {
            if let Some(index) = self.find_direct_key(key, start + 1, end, indent + 2) {
                self.replace_scalar_line(index, key, value);
                return;
            }

            self.lines
                .insert(end, format!("{}{}: {}", " ".repeat(indent + 2), key, value));
            return;
        }

        self.append_section(vec![format!("{section}:"), format!("  {key}: {value}")]);
    }

    fn replace_top_level_section(&mut self, section: &str, replacement: Vec<String>) {
        if let Some((start, end, _)) = self.section_range(section) {
            self.lines.splice(start..end, replacement);
        } else {
            self.append_section(replacement);
        }
    }

    /// Insert a missing top-level scalar at its canonical position: right
    /// after the nearest preceding canonical key's block, else before the
    /// nearest following key's comment run. Existing lines are never touched.
    fn insert_missing_top_level(&mut self, key: &str, value: &str, order: &[String]) {
        let pos = order
            .iter()
            .position(|item| item.as_str() == key)
            .unwrap_or(usize::MAX);
        let end = pos.min(order.len());
        if let Some(after) = order[..end]
            .iter()
            .rev()
            .find_map(|prev| self.top_level_block_end(prev))
        {
            self.lines
                .insert(after.min(self.lines.len()), format!("{key}: {value}"));
            return;
        }
        if let Some(before) = order
            .iter()
            .skip(end.saturating_add(1))
            .find_map(|next| self.find_top_level_key(next))
        {
            let at = self.comment_run_start(before, 0);
            self.insert_with_blank_separator(at, format!("{key}: {value}"));
            return;
        }
        self.lines.push(format!("{key}: {value}"));
    }

    /// Insert a missing top-level section header at its canonical position,
    /// separated from the previous block by one blank line.
    fn insert_missing_top_level_section(&mut self, section: &str, top_order: &[String]) {
        let pos = top_order
            .iter()
            .position(|item| item.as_str() == section)
            .unwrap_or(usize::MAX);
        let end = pos.min(top_order.len());
        if let Some(after) = top_order[..end]
            .iter()
            .rev()
            .find_map(|prev| self.top_level_block_end(prev))
        {
            self.insert_section_block(after, format!("{section}:"));
            return;
        }
        if let Some(before) = top_order
            .iter()
            .skip(end.saturating_add(1))
            .find_map(|next| self.find_top_level_key(next))
        {
            let at = self.comment_run_start(before, 0);
            self.insert_section_block(at, format!("{section}:"));
            return;
        }
        if !self.lines.is_empty()
            && self
                .lines
                .last()
                .is_some_and(|line| !line.trim().is_empty())
        {
            self.lines.push(String::new());
        }
        self.lines.push(format!("{section}:"));
    }

    /// Insert a missing nested key at its canonical position inside its
    /// section (after the nearest preceding key, else before the nearest
    /// following key's comment run). A missing section is created at its
    /// canonical top-level slot first.
    fn insert_missing_nested(
        &mut self,
        section: &str,
        key: &str,
        value: &str,
        order: &[String],
        top_order: &[String],
    ) {
        if self.section_range(section).is_none() {
            self.insert_missing_top_level_section(section, top_order);
        }
        let Some((start, end, indent)) = self.section_range(section) else {
            return;
        };
        let child_indent = " ".repeat(indent + 2);
        let rendered = format!("{child_indent}{key}: {value}");
        let pos = order
            .iter()
            .position(|item| item.as_str() == key)
            .unwrap_or(usize::MAX);
        let end_pos = pos.min(order.len());
        if let Some(index) = order[..end_pos]
            .iter()
            .map(String::as_str)
            .rev()
            .find_map(|prev| self.find_direct_key(prev, start + 1, end, indent + 2))
        {
            self.lines.insert(index + 1, rendered);
            return;
        }
        if let Some(index) = order
            .iter()
            .skip(end_pos.saturating_add(1))
            .map(String::as_str)
            .find_map(|next| self.find_direct_key(next, start + 1, end, indent + 2))
        {
            let at = self.comment_run_start(index, start + 1);
            self.lines.insert(at, rendered);
            return;
        }
        let at = self.comment_run_start(end, start + 1);
        self.lines.insert(at.min(self.lines.len()), rendered);
    }

    /// End of a top-level key's block, excluding the blank/comment run that
    /// belongs to the following key: the insert point for a missing key that
    /// canonically follows it.
    fn top_level_block_end(&self, key: &str) -> Option<usize> {
        let index = self.find_top_level_key(key)?;
        let end = self.section_range(key).map_or(index + 1, |(_, end, _)| end);
        Some(self.comment_run_start(end, index + 1))
    }

    /// Walk up over blank/comment lines: the start of the comment run
    /// belonging to `index` (never below `lower_bound`).
    fn comment_run_start(&self, index: usize, lower_bound: usize) -> usize {
        let mut at = index.min(self.lines.len());
        let lower_bound = lower_bound.min(at);
        while at > lower_bound {
            let trimmed = self.lines[at - 1].trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                at -= 1;
            } else {
                break;
            }
        }
        at
    }

    /// Insert one line, keeping a single blank separator when squeezing
    /// between two content lines.
    fn insert_with_blank_separator(&mut self, at: usize, line: String) {
        let at = at.min(self.lines.len());
        let crowded = at > 0
            && !self.lines[at - 1].trim().is_empty()
            && (at >= self.lines.len() || !self.lines[at].trim().is_empty());
        if crowded {
            self.lines.splice(at..at, [String::new(), line]);
        } else {
            self.lines.insert(at, line);
        }
    }

    /// Insert a section header, keeping one blank line above it when it
    /// follows content.
    fn insert_section_block(&mut self, at: usize, header: String) {
        let at = at.min(self.lines.len());
        if at > 0 && !self.lines[at - 1].trim().is_empty() {
            self.lines.splice(at..at, [String::new(), header]);
        } else {
            self.lines.insert(at, header);
        }
    }

    fn update_subscriptions(
        &mut self,
        previous: &[SubscriptionSource],
        current: &[SubscriptionSource],
    ) -> bool {
        let Some(ranges) = self.sequence_item_ranges("subscriptions") else {
            return current.is_empty();
        };
        if ranges.len() != previous.len() {
            return false;
        }

        if previous.len() == current.len() {
            return self.update_subscription_items_in_place(&ranges, previous, current);
        }

        if previous.len() == current.len().saturating_add(1)
            && let Some(index) = removed_subscription_index(previous, current)
        {
            let (start, end, _) = ranges[index];
            self.lines.drain(start..end);
            return true;
        }

        if current.len() == previous.len().saturating_add(1)
            && let Some(index) = inserted_subscription_index(previous, current)
        {
            let insert_at = ranges
                .get(index)
                .map(|(start, _, _)| *start)
                .or_else(|| ranges.last().map(|(_, end, _)| *end))
                .or_else(|| self.section_range("subscriptions").map(|(_, end, _)| end))
                .unwrap_or(self.lines.len());
            let indent = ranges
                .first()
                .map(|(_, _, indent)| *indent)
                .or_else(|| {
                    self.section_range("subscriptions")
                        .map(|(_, _, indent)| indent + 2)
                })
                .unwrap_or(2);
            self.lines.splice(
                insert_at..insert_at,
                format_subscription_item(&current[index], indent),
            );
            return true;
        }

        false
    }

    fn update_subscription_items_in_place(
        &mut self,
        ranges: &[(usize, usize, usize)],
        previous: &[SubscriptionSource],
        current: &[SubscriptionSource],
    ) -> bool {
        for ((item_start, item_end, item_indent), (before, after)) in
            ranges.iter().copied().zip(previous.iter().zip(current))
        {
            if before.name != after.name
                && !self.set_sequence_item_scalar(
                    item_start,
                    item_end,
                    item_indent,
                    "name",
                    yaml_scalar(&after.name),
                )
            {
                return false;
            }
            if before.url != after.url
                && !self.set_sequence_item_scalar(
                    item_start,
                    item_end,
                    item_indent,
                    "url",
                    yaml_scalar(&after.url),
                )
            {
                return false;
            }
            if before.enabled != after.enabled
                && !self.set_sequence_item_scalar(
                    item_start,
                    item_end,
                    item_indent,
                    "enabled",
                    after.enabled.to_string(),
                )
            {
                return false;
            }
            if before.priority != after.priority
                && !self.set_sequence_item_scalar(
                    item_start,
                    item_end,
                    item_indent,
                    "priority",
                    after.priority.to_string(),
                )
            {
                return false;
            }
        }

        true
    }

    fn set_sequence_item_scalar(
        &mut self,
        item_start: usize,
        item_end: usize,
        item_indent: usize,
        key: &str,
        value: impl AsRef<str>,
    ) -> bool {
        let value = value.as_ref();
        if let Some((line_key, _)) = parse_sequence_item_key(&self.lines[item_start])
            && line_key == key
        {
            let indent = leading_whitespace(&self.lines[item_start]);
            let comment = inline_comment(&self.lines[item_start]).unwrap_or_default();
            self.lines[item_start] = format!("{indent}- {key}: {value}{comment}");
            return true;
        }

        if let Some(index) = self.find_direct_key(key, item_start + 1, item_end, item_indent + 2) {
            self.replace_scalar_line(index, key, value);
            return true;
        }

        false
    }

    fn sequence_item_ranges(&self, section: &str) -> Option<Vec<(usize, usize, usize)>> {
        let (start, end, section_indent) = self.section_range(section)?;
        let item_indent = section_indent + 2;
        let starts = (start + 1..end)
            .filter(|&index| {
                let line = &self.lines[index];
                line.len() >= item_indent
                    && leading_whitespace_len(line) == item_indent
                    && line[item_indent..].starts_with("- ")
            })
            .collect::<Vec<_>>();

        let ranges = starts
            .iter()
            .enumerate()
            .map(|(index, start)| {
                let end = starts.get(index + 1).copied().unwrap_or(end);
                (*start, end, item_indent)
            })
            .collect();

        Some(ranges)
    }

    fn append_section(&mut self, replacement: Vec<String>) {
        if !self.lines.is_empty()
            && self
                .lines
                .last()
                .is_some_and(|line| !line.trim().is_empty())
        {
            self.lines.push(String::new());
        }
        self.lines.extend(replacement);
    }

    fn replace_scalar_line(&mut self, index: usize, key: &str, value: &str) {
        let old_line = self.lines[index].clone();
        let indent = leading_whitespace(&self.lines[index]);
        let comment = inline_comment(&self.lines[index]).unwrap_or_default();
        self.lines[index] = format!("{indent}{key}: {value}{comment}");
        if scalar_line_has_empty_value(&old_line) {
            self.remove_block_value_lines_after(index, indent.len());
        }
    }

    fn remove_block_value_lines_after(&mut self, index: usize, parent_indent: usize) {
        let child_index = index + 1;
        while child_index < self.lines.len() {
            let line = &self.lines[child_index];
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                break;
            }

            let indent = leading_whitespace_len(line);
            let is_block_child =
                indent > parent_indent || (indent == parent_indent && trimmed.starts_with("- "));
            if !is_block_child {
                break;
            }

            self.lines.remove(child_index);
        }
    }

    fn find_top_level_key(&self, key: &str) -> Option<usize> {
        self.find_direct_key(key, 0, self.lines.len(), 0)
    }

    fn find_direct_key(
        &self,
        key: &str,
        start: usize,
        end: usize,
        expected_indent: usize,
    ) -> Option<usize> {
        (start..end).find(|&index| {
            parse_yaml_key(&self.lines[index])
                .is_some_and(|(indent, found)| indent == expected_indent && found == key)
        })
    }

    fn section_range(&self, section: &str) -> Option<(usize, usize, usize)> {
        let start = self.find_top_level_key(section)?;
        let (section_indent, _) = parse_yaml_key(&self.lines[start])?;
        let mut end = start + 1;
        while end < self.lines.len() {
            if let Some((indent, _)) = parse_yaml_key(&self.lines[end])
                && indent <= section_indent
            {
                break;
            }
            end += 1;
        }

        Some((start, end, section_indent))
    }
}

fn parse_yaml_key(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }

    let (key, _) = trimmed.split_once(':')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_')
    {
        return None;
    }

    Some((line.len() - trimmed.len(), key))
}

fn parse_sequence_item_key(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("- ")?;
    let (key, value) = rest.split_once(':')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_')
    {
        return None;
    }

    Some((key, value.trim()))
}

fn scalar_line_has_empty_value(line: &str) -> bool {
    let Some((_, value)) = line.split_once(':') else {
        return false;
    };

    value
        .split_once(" #")
        .map_or(value, |(value, _)| value)
        .trim()
        .is_empty()
}

fn leading_whitespace(line: &str) -> String {
    line.chars()
        .take_while(|value| value.is_whitespace())
        .collect()
}

fn leading_whitespace_len(line: &str) -> usize {
    line.chars()
        .take_while(|value| value.is_whitespace())
        .map(char::len_utf8)
        .sum()
}

fn inline_comment(line: &str) -> Option<&str> {
    line.find(" #").map(|index| &line[index..])
}

fn probe_mode(mode: ProbeMode) -> String {
    match mode {
        ProbeMode::Active => "active".to_string(),
        ProbeMode::Tcp => "tcp".to_string(),
    }
}

fn nullable_string(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "null".to_string()
    } else {
        yaml_scalar(value)
    }
}

fn yaml_scalar(value: &str) -> String {
    let value = value.trim();
    if is_plain_yaml_scalar(value) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn is_plain_yaml_scalar(value: &str) -> bool {
    if value.is_empty() || value != value.trim() || value.contains('\n') || value.contains(" #") {
        return false;
    }

    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "true" | "false" | "yes" | "no" | "on" | "off"
    ) || value.parse::<f64>().is_ok()
    {
        return false;
    }

    !value.starts_with(|value: char| {
        matches!(
            value,
            '-' | '?'
                | ':'
                | ','
                | '['
                | ']'
                | '{'
                | '}'
                | '#'
                | '&'
                | '*'
                | '!'
                | '|'
                | '>'
                | '@'
                | '`'
                | '"'
                | '\''
        )
    })
}

fn format_inline_u16_list(values: &[u16]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn format_subscriptions_section(subscriptions: &[SubscriptionSource]) -> Vec<String> {
    if subscriptions.is_empty() {
        return vec!["subscriptions: []".to_string()];
    }

    let mut lines = vec!["subscriptions:".to_string()];
    for source in subscriptions {
        lines.extend(format_subscription_item(source, 2));
    }
    lines
}

fn format_subscription_item(source: &SubscriptionSource, indent: usize) -> Vec<String> {
    let item_indent = " ".repeat(indent);
    let field_indent = " ".repeat(indent + 2);
    vec![
        format!("{item_indent}- name: {}", yaml_scalar(&source.name)),
        format!("{field_indent}url: {}", yaml_scalar(&source.url)),
        format!("{field_indent}enabled: {}", source.enabled),
        format!("{field_indent}priority: {}", source.priority),
    ]
}

fn removed_subscription_index(
    previous: &[SubscriptionSource],
    current: &[SubscriptionSource],
) -> Option<usize> {
    if previous.len() != current.len().saturating_add(1) {
        return None;
    }

    (0..previous.len()).find(|&index| {
        previous
            .iter()
            .enumerate()
            .filter(|(candidate, _)| *candidate != index)
            .map(|(_, source)| source)
            .eq(current.iter())
    })
}

fn inserted_subscription_index(
    previous: &[SubscriptionSource],
    current: &[SubscriptionSource],
) -> Option<usize> {
    if current.len() != previous.len().saturating_add(1) {
        return None;
    }

    (0..current.len()).find(|&index| {
        current
            .iter()
            .enumerate()
            .filter(|(candidate, _)| *candidate != index)
            .map(|(_, source)| source)
            .eq(previous.iter())
    })
}

pub fn human_bytes(bytes: u64) -> String {
    let mut value = u64_to_f64(bytes);
    let mut unit = 0_usize;
    while value >= BYTES_PER_UNIT && unit < BYTE_UNITS.len() - 1 {
        value /= BYTES_PER_UNIT;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", BYTE_UNITS[unit])
    } else {
        format!("{value:.2} {}", BYTE_UNITS[unit])
    }
}

#[allow(clippy::cast_precision_loss)]
const fn u64_to_f64(value: u64) -> f64 {
    value as f64
}

pub const fn bool_text(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Draw a ratatui `Scrollbar` inside the block's inner area.
///
/// `visible_rows` = actual content rows visible (excluding borders & headers).
/// `invert` = true when offset=0 means newest/bottom content.
pub fn draw_scrollbar(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    total_items: usize,
    visible_rows: usize,
    scroll_offset: usize,
    invert: bool,
) {
    use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

    if area.height < 3 || visible_rows == 0 {
        return;
    }
    let inner_height = area.height.saturating_sub(2) as usize;
    if total_items <= visible_rows || inner_height < 3 {
        return;
    }

    // The scrollbar renders in the block's inner area (between borders).
    // vertical margin=1 strips top/bottom borders; horizontal=0 keeps full width.
    let inner = area.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 0,
    });

    // Map scroll_offset → position for the scrollbar.
    // scroll_offset ranges 0..max_scroll where max_scroll = total_items - visible_rows.
    // Ratatui maps position = content_length - 1 to the very bottom of the track.
    let max_scroll = total_items.saturating_sub(visible_rows);
    let last_position = total_items.saturating_sub(1);
    let position = (scroll_offset.min(max_scroll) * last_position)
        .checked_div(max_scroll)
        .map_or(0, |raw| {
            if invert {
                last_position.saturating_sub(raw)
            } else {
                raw
            }
        });

    let mut state = ScrollbarState::new(total_items)
        .position(position)
        .viewport_content_length(1);

    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("▣"),
        inner,
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_config_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "v2raydar-util-{name}-{}-{nonce}.yaml",
            std::process::id()
        ))
    }

    fn temp_config_path_with_extension(name: &str, extension: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "v2raydar-util-{name}-{}-{nonce}.{extension}",
            std::process::id()
        ))
    }

    fn write_config(name: &str, content: &str) -> PathBuf {
        let path = temp_config_path(name);
        fs::write(&path, content).expect("temp config can be written");
        path
    }

    #[test]
    fn yaml_save_preserves_unrelated_shape_and_inline_lists() {
        let path = write_config(
            "preserve-shape",
            r"bind: 127.0.0.1:27141
top_n: 10

# Keep this blank line and comment.
probe:
  accepted_statuses: [204, 200] # keep inline
  active_timeout_ms: 30000

sharing:
  enabled: false # keep comment

subscriptions:
  - name: first
    url: data:,vless://uuid@example.com:443%23demo
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        config.top_n = 12;
        config.sharing.enabled = true;

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert!(saved.contains("top_n: 12"));
        assert!(saved.contains("\n\n# Keep this blank line and comment.\nprobe:"));
        assert!(saved.contains("  accepted_statuses: [204, 200] # keep inline"));
        assert!(saved.contains("  enabled: true # keep comment"));
    }

    #[test]
    fn yaml_save_updates_existing_subscription_item_in_place() {
        let path = write_config(
            "subscription-in-place",
            r"bind: 127.0.0.1:27141
top_n: 10

probe:
  accepted_statuses: [204, 200]

subscriptions:
  - name: first
    url: data:,vless://first@example.com:443%23demo
    enabled: true # keep enabled comment
    priority: 1
  - name: second
    url: data:,vless://second@example.com:443%23demo
    enabled: true
    priority: 2 # keep priority comment
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        config.subscriptions[0].enabled = false;
        config.subscriptions[1].priority = 5;

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert!(saved.contains("    enabled: false # keep enabled comment"));
        assert!(saved.contains("    priority: 5 # keep priority comment"));
        assert!(saved.contains("  accepted_statuses: [204, 200]"));
        assert!(saved.contains("  - name: first"));
        assert!(saved.contains("  - name: second"));
    }

    #[test]
    fn yaml_save_removes_one_subscription_item_without_touching_others() {
        let path = write_config(
            "subscription-remove",
            r"bind: 127.0.0.1:27141
top_n: 10

probe:
  accepted_statuses: [204, 200]

subscriptions:
  - name: first
    url: data:,vless://first@example.com:443%23demo
    enabled: true # keep first comment
    priority: 1
  - name: second
    url: data:,vless://second@example.com:443%23demo
    enabled: true
    priority: 2
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        config.subscriptions.remove(1);

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert!(saved.contains("    enabled: true # keep first comment"));
        assert!(!saved.contains("  - name: second"));
        assert!(saved.contains("  accepted_statuses: [204, 200]"));
    }

    #[test]
    fn yaml_save_inserts_one_subscription_item_without_touching_others() {
        let path = write_config(
            "subscription-insert",
            r"bind: 127.0.0.1:27141
top_n: 10

probe:
  accepted_statuses: [204, 200]

subscriptions:
  - name: first
    url: data:,vless://first@example.com:443%23demo
    enabled: true # keep first comment
    priority: 1
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        config.subscriptions.push(SubscriptionSource {
            name: "second".to_string(),
            url: "data:,vless://second@example.com:443%23demo".to_string(),
            enabled: true,
            priority: 2,
        });

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert!(saved.contains("    enabled: true # keep first comment"));
        assert!(saved.contains("  - name: second"));
        assert!(saved.contains("    priority: 2"));
        assert!(saved.contains("  accepted_statuses: [204, 200]"));
    }

    #[test]
    fn yaml_save_does_not_persist_auto_sing_box_path() {
        let path = write_config(
            "auto-sing-box-path",
            r"probe:
  sing_box_path: null

subscriptions:
  - name: first
    url: data:,vless://first@example.com:443%23demo
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        config.probe.sing_box_path = "/tmp/v2raydar/sing-box".to_string();
        config.probe.sing_box_path_auto = true;
        config.top_n = 11;

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert!(saved.contains("  sing_box_path: null"));
        assert!(!saved.contains("/tmp/v2raydar/sing-box"));
        assert!(saved.contains("top_n: 11"));
    }

    #[test]
    fn json_save_does_not_persist_auto_sing_box_path_or_flag() {
        let path = temp_config_path_with_extension("auto-sing-box-path", "json");
        let mut config = AppConfig::default_for_first_run();
        config.probe.sing_box_path = "/tmp/v2raydar/sing-box".to_string();
        config.probe.sing_box_path_auto = true;

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert!(saved.contains(r#""sing_box_path": """#));
        assert!(!saved.contains("sing_box_path_auto"));
        assert!(!saved.contains("/tmp/v2raydar/sing-box"));
    }

    #[test]
    fn yaml_save_persists_proxy_settings() {
        let path = write_config(
            "proxy-persist",
            r"bind: 127.0.0.1:27141
top_n: 10

proxy:
  enabled: false
  port: 27910
  discoverable: false
  health_check_url: https://www.gstatic.com/generate_204
  health_check_interval_seconds: 60

probe:
  accepted_statuses: [204, 200]

subscriptions:
  - name: first
    url: data:,vless://uuid@example.com:443%23demo
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        assert!(!config.proxy.enabled, "starts disabled");
        config.proxy.enabled = true;
        config.proxy.discoverable = true;
        config.proxy.port = 10808;

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        let reloaded = AppConfig::load(&path).expect("saved config reloads");
        fs::remove_file(&path).ok();

        assert!(saved.contains("enabled: true"));
        assert!(saved.contains("discoverable: true"));
        assert!(saved.contains("port: 10808"));
        assert!(reloaded.proxy.enabled);
        assert!(reloaded.proxy.discoverable);
        assert_eq!(reloaded.proxy.port, 10808);
    }

    #[test]
    fn backfill_adds_missing_keys_and_preserves_user_values() {
        let path = write_config(
            "backfill-old",
            r"bind: 127.0.0.1:27141
top_n: 10
# Keep this comment and the custom refresh below.
refresh_seconds: 300

sharing:
  enabled: true

probe:
  mode: tcp

subscriptions:
  - name: first
    url: data:,vless://uuid@example.com:443%23demo
",
        );
        let added = backfill_missing_defaults(&path).expect("backfill runs");
        assert!(added > 0);
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        // New keys appear with defaults …
        assert!(saved.contains("ping_seconds: 300"));
        assert!(saved.contains("geoip_db_path: null"));
        assert!(saved.contains("clean_offlines_after_days: 7"));
        // … while user values, comments, and subscriptions stay intact.
        assert!(saved.contains("refresh_seconds: 300"));
        assert!(saved.contains("# Keep this comment and the custom refresh below."));
        assert!(saved.contains("enabled: true"));
        assert!(saved.contains("  - name: first"));
        assert!(!saved.contains("refresh_seconds: 900"));
    }

    // Older config missing ping_seconds, geoip_db_path,
    // clean_offlines_after_days, and the whole proxy section.
    const OLD_CONFIG_FIXTURE: &str = r"bind: 127.0.0.1:27141
top_n: 20
# Auto-refresh interval in seconds; 0 disables timer refreshes.
refresh_seconds: 600
# true returns /subscription as base64 for v2rayN/v2rayNG compatibility.
encoded_subscription: true
# true re-pings the last saved top-N first and keeps stable configs ahead.
prioritize_stability: true
# true publishes early working configs before final ranking; results may be less optimal & unstable.
return_configs_asap: true
# true tests every candidate; expect longer runs and more CPU/network use.
scan_all_configs: false
# Per-source download timeout in milliseconds; raise only for slow sources.
fetch_timeout_ms: 30000
# Subscription sources fetched in parallel; high values can stress network/RAM.
fetch_concurrency: 8
# Maximum bytes accepted per source; high values allow large memory use.
max_subscription_bytes: 33554432
# true skips live source downloads and tests only cached subscription snapshots.
use_cache_only: false
# Optional private share link used as a fetch bridge on restricted networks
emergency_config: null

#* LAN sharing controls for phones or other devices on your network.
sharing:
  # false keeps endpoints local-only unless bind is manually exposed.
  enabled: true
  # true requires ?token=... for LAN subscription/results requests.
  require_token: false
  # true auto-generates a token; a string sets your own shared secret.
  token: null

#* Candidate validation settings.
probe:
  # active uses sing-box for real HTTP tests; tcp is only a lightweight diagnostic.
  mode: active
  # null auto-detects bundled/Termux sing-box; set a path if needed.
  sing_box_path: null
  # TCP diagnostic timeout in milliseconds; lower fails faster on bad networks.
  connect_timeout_ms: 5000
  # HTTP active-probe timeout per candidate; high values slow failed probes.
  active_timeout_ms: 30000
  # Wait time for each temporary sing-box process to start.
  startup_timeout_ms: 5000
  # HTTP checks per active batch; high values can increase CPU/network pressure.
  concurrency: 16
  # Initial candidates per sing-box batch; high values can raise process RAM use.
  batch_size: 20
  # Parallel sing-box processes; keep low or expect high RAM/socket usage.
  process_concurrency: null
  # URL loaded through each candidate to prove reachability.
  test_url: https://www.gstatic.com/generate_204
  # HTTP status codes treated as successful active probes.
  accepted_statuses: [204, 200]
  # Optional speed-test URL; enabling it consumes extra bandwidth
  # You can use this URL for example: https://archive.org/download/steamboat-willie_1928/steamboat-willie_1928.ia.mp4
  download_url: null
  # Maximum bytes read per speed test; high values use more data/time.
  download_bytes_limit: 1048576

subscriptions:
  - name: first
    url: data:,vless://uuid@example.com:443%23demo
";

    #[test]
    fn backfill_inserts_missing_keys_at_example_positions() {
        // An older config missing ping_seconds, geoip_db_path,
        // clean_offlines_after_days, and the whole proxy section.
        let path = write_config("backfill-positions", OLD_CONFIG_FIXTURE);
        // 3 top-level scalars + the 6 proxy keys.
        assert_eq!(backfill_missing_defaults(&path).expect("backfill runs"), 9);
        let saved = fs::read_to_string(&path).expect("config can be read");

        let lines: Vec<&str> = saved.lines().collect();
        let position = |needle: &str| {
            lines
                .iter()
                .position(|line| line.trim() == needle)
                .unwrap_or_else(|| panic!("missing line: {needle}"))
        };
        // Missing scalars land right after their example predecessor.
        assert_eq!(
            position("ping_seconds: 300"),
            position("refresh_seconds: 600") + 1
        );
        assert_eq!(
            position("geoip_db_path: null"),
            position("emergency_config: null") + 1
        );
        assert_eq!(
            position("clean_offlines_after_days: 7"),
            position("geoip_db_path: null") + 1
        );
        // The absent proxy section lands between sharing and probe, in order.
        let proxy_at = position("proxy:");
        assert!(proxy_at > position("token: null"));
        assert!(lines[proxy_at - 1].trim().is_empty());
        for (offset, key) in [
            "enabled: false",
            "port: 27910",
            "discoverable: false",
            "rotating_proxy: true",
            "health_check_url: https://cp.cloudflare.com",
            "health_check_interval_seconds: 60",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(lines[proxy_at + 1 + offset].trim(), *key);
        }
        assert!(proxy_at + 6 < position("probe:"));
        // User values, comments, and subscriptions stay intact.
        assert!(saved.contains("top_n: 20"));
        assert!(saved.contains("refresh_seconds: 600"));
        assert!(saved.contains("enabled: true"));
        assert!(saved.contains("mode: active"));
        assert!(saved.contains("#* Candidate validation settings."));
        assert!(saved.contains("  - name: first"));

        // Second run is a no-op: positions are stable, nothing rewrites.
        assert_eq!(
            backfill_missing_defaults(&path).expect("backfill reruns"),
            0
        );
        assert_eq!(fs::read_to_string(&path).expect("config reread"), saved);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn backfill_inserts_missing_nested_keys_in_section_order() {
        // Partial sections: proxy keeps only `enabled`, probe is missing
        // `mode` (inserted before the surviving `sing_box_path`).
        let path = write_config(
            "backfill-nested-order",
            r"bind: 127.0.0.1:27141
top_n: 10
refresh_seconds: 900
ping_seconds: 300
encoded_subscription: false
prioritize_stability: true
return_configs_asap: false
scan_all_configs: false
fetch_timeout_ms: 30000
fetch_concurrency: 8
max_subscription_bytes: 33554432
use_cache_only: false
emergency_config: null
geoip_db_path: null
clean_offlines_after_days: 7
sharing:
  enabled: false
  require_token: false
  token: null
proxy:
  enabled: false
probe:
  sing_box_path: null
subscriptions:
  - name: first
    url: data:,vless://uuid@example.com:443%23demo
",
        );
        // 5 proxy keys + mode + 10 probe keys after sing_box_path.
        assert_eq!(backfill_missing_defaults(&path).expect("backfill runs"), 16);
        let saved = fs::read_to_string(&path).expect("config can be read");
        let lines: Vec<&str> = saved.lines().collect();
        let block = |section: &str, end_section: &str| {
            let start = lines
                .iter()
                .position(|line| line.trim() == section)
                .unwrap_or_else(|| panic!("missing section: {section}"));
            let end = lines
                .iter()
                .position(|line| line.trim() == end_section)
                .unwrap_or_else(|| panic!("missing section: {end_section}"));
            lines[start..end]
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            block("proxy:", "probe:"),
            vec![
                "proxy:",
                "enabled: false",
                "port: 27910",
                "discoverable: false",
                "rotating_proxy: true",
                "health_check_url: https://cp.cloudflare.com",
                "health_check_interval_seconds: 60",
            ]
        );
        // `mode` slots before the surviving `sing_box_path`, the rest append
        // in example order.
        let probe = block("probe:", "subscriptions:");
        assert_eq!(
            &probe[..4],
            [
                "probe:",
                "mode: active",
                "sing_box_path: null",
                "connect_timeout_ms: 5000",
            ]
        );
        assert_eq!(
            &probe[4..],
            [
                "active_timeout_ms: 30000",
                "startup_timeout_ms: 5000",
                "concurrency: 16",
                "batch_size: 20",
                "process_concurrency: null",
                "test_url: https://www.gstatic.com/generate_204",
                "accepted_statuses: [204, 200]",
                "download_url: null",
                "download_bytes_limit: 1048576",
            ]
        );
        fs::remove_file(&path).ok();
    }

    #[test]
    fn every_example_key_round_trips_through_backfill_at_its_position() {
        // No hardcoded keys: the embedded example enumerates the coverage.
        // Delete each key from a complete config and require the backfill to
        // restore the exact content order with one addition. (The
        // subscriptions list is user data, never backfilled.) If the example
        // gains a key without backfill support, its round trip fails here.
        fn content_lines(config: &str) -> Vec<&str> {
            config
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect()
        }
        let canonical = example_key_order();
        let pristine = {
            let path = temp_config_path("backfill-roundtrip-pristine");
            AppConfig::write_default(&path).expect("default config writes");
            let content = fs::read_to_string(&path).expect("config can be read");
            fs::remove_file(&path).ok();
            content
        };
        let expected = content_lines(&pristine);

        let mut cases: Vec<(Option<String>, String)> = Vec::new();
        for key in &canonical.top {
            // Section headers and the subscriptions list never round-trip:
            // deleting one orphans its block (invalid YAML, rightly refused)
            // or touches user data.
            if key != "subscriptions" && !canonical.nested.contains_key(key) {
                cases.push((None, key.clone()));
            }
        }
        for (section, keys) in &canonical.nested {
            if section == "subscriptions" {
                continue;
            }
            for key in keys {
                cases.push((Some(section.clone()), key.clone()));
            }
        }
        assert!(!cases.is_empty());

        for (section, key) in cases {
            let reduced = remove_config_key_line(&pristine, section.as_deref(), &key);
            assert_ne!(reduced, pristine, "{key} removal must change the file");
            let path = write_config("backfill-roundtrip", &reduced);
            let added = backfill_missing_defaults(&path).expect("backfill runs");
            let restored = fs::read_to_string(&path).expect("config can be read");
            fs::remove_file(&path).ok();
            assert_eq!(added, 1, "{key} must be the only addition");
            assert_eq!(
                content_lines(&restored),
                expected,
                "{key} must return to its example position"
            );
        }
    }

    #[test]
    fn every_backfilled_key_has_an_example_position() {
        // The reverse drift guard (no hardcoded keys): backfill a config
        // that only has user data and require every added key — top-level
        // and nested — to exist in the derived example order. A setting
        // added to the backfill table but forgotten in configs.example.yaml
        // fails here instead of silently appending at block end.
        let path = write_config("backfill-coverage", "subscriptions: []\n");
        let added = backfill_missing_defaults(&path).expect("backfill runs");
        assert!(added > 0);
        let saved = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&saved).expect("backfilled config parses");
        let canonical = example_key_order();
        let top = parsed.as_mapping().expect("top-level mapping");
        for (key, value) in top {
            let Some(key) = key.as_str() else {
                continue;
            };
            if key == "subscriptions" {
                continue;
            }
            assert!(
                canonical.top.iter().any(|known| known == key),
                "top-level {key} missing from configs.example.yaml"
            );
            if let Some(nested) = value.as_mapping() {
                for (nested_key, _) in nested {
                    let Some(nested_key) = nested_key.as_str() else {
                        continue;
                    };
                    assert!(
                        canonical
                            .nested
                            .get(key)
                            .is_some_and(|keys| keys.iter().any(|known| known == nested_key)),
                        "{key}.{nested_key} missing from configs.example.yaml"
                    );
                }
            }
        }
    }

    /// Delete one key's line (top-level, or nested in `section`) from config
    /// text. Returns the input unchanged when the key is absent.
    fn remove_config_key_line(config: &str, section: Option<&str>, key: &str) -> String {
        fn is_key_line(line: &str, indent: usize, key: &str) -> bool {
            let trimmed = line.trim_start();
            !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && line.len() - trimmed.len() == indent
                && trimmed
                    .split_once(':')
                    .is_some_and(|(name, _)| name.trim() == key)
        }

        let lines: Vec<&str> = config.lines().collect();
        let drop_line = if let Some(section) = section {
            let header = format!("{section}:");
            let Some(start) = lines.iter().position(|line| line.trim() == header) else {
                return config.to_string();
            };
            let indent = lines[start].len() - lines[start].trim_start().len();
            lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .take_while(|(_, line)| {
                    let trimmed = line.trim_start();
                    trimmed.is_empty()
                        || trimmed.starts_with('#')
                        || line.len() - trimmed.len() > indent
                })
                .find(|(_, line)| is_key_line(line, indent + 2, key))
                .map(|(index, _)| index)
        } else {
            lines.iter().position(|line| is_key_line(line, 0, key))
        };
        let Some(drop) = drop_line else {
            return config.to_string();
        };
        let mut kept = lines[..drop].join("\n");
        if drop + 1 < lines.len() {
            kept.push('\n');
            kept.push_str(&lines[drop + 1..].join("\n"));
        }
        if config.ends_with('\n') && !kept.ends_with('\n') {
            kept.push('\n');
        }
        kept
    }

    #[test]
    fn backfill_is_noop_on_complete_config() {
        let path = temp_config_path("backfill-complete");
        AppConfig::write_default(&path).expect("default config writes");
        let before = fs::read_to_string(&path).expect("config can be read");

        assert_eq!(backfill_missing_defaults(&path).expect("backfill runs"), 0);
        let after = fs::read_to_string(&path).expect("config can be read");
        fs::remove_file(&path).ok();

        assert_eq!(before, after);
    }

    #[test]
    fn backfill_rejects_invalid_yaml_without_touching_it() {
        let path = write_config("backfill-invalid", "not: valid: yaml: [[[[\n");
        let before = fs::read_to_string(&path).expect("config can be read");

        assert!(backfill_missing_defaults(&path).is_err());
        assert_eq!(
            fs::read_to_string(&path).expect("config can be read"),
            before
        );
        fs::remove_file(&path).ok();
    }

    #[test]
    fn yaml_save_persists_clean_offline_days() {
        let path = write_config(
            "clean-offline-persist",
            r"bind: 127.0.0.1:27141
top_n: 10
clean_offlines_after_days: 7

subscriptions:
  - name: first
    url: data:,vless://uuid@example.com:443%23demo
",
        );
        let mut config = AppConfig::load(&path).expect("config loads");
        config.clean_offlines_after_days = 30;

        save_config(&path, &config).expect("config saves");
        let saved = fs::read_to_string(&path).expect("config can be read");
        let reloaded = AppConfig::load(&path).expect("saved config reloads");
        fs::remove_file(&path).ok();

        assert!(saved.contains("clean_offlines_after_days: 30"));
        assert_eq!(reloaded.clean_offlines_after_days, 30);
    }
}
