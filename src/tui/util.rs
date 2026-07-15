use std::{fs, path::Path};

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

fn save_json_config(path: &Path, config: &AppConfig) -> Result<()> {
    let config = persistable_config(config);
    let content = serde_json::to_string_pretty(&config).context("unable to serialize config")?;
    fs::write(path, format!("{content}\n"))
        .with_context(|| format!("unable to write config to {}", path.display()))
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
        .with_context(|| format!("unable to write config to {}", path.display()))
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
}
