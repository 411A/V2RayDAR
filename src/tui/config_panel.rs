use std::num::NonZeroU16;

use ratatui::{
    Frame,
    buffer::{Buffer, CellDiffOption},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    config::should_include_token_in_url,
    constants::{
        TUI_ANSI_UNDERLINE_DISABLE, TUI_ANSI_UNDERLINE_ENABLE, TUI_OSC8_LINK_PREFIX,
        TUI_OSC8_LINK_SEPARATOR, TUI_OSC8_LINK_SUFFIX,
    },
    model::RuntimeConfig,
    network::sharing_status,
    paths::AppPaths,
};

use super::{state::TuiState, util::bool_text};

struct ConfigView {
    live_config: RuntimeConfig,
    sharing: crate::network::SharingStatus,
    show_endpoint: bool,
    discoverable: String,
    proxy: String,
}

impl ConfigView {
    fn from_state(state: &TuiState) -> Self {
        let live_config = RuntimeConfig::from(&state.editable);
        let mut sharing = sharing_status(&live_config);
        let needs_restart = live_config.sharing_enabled
            && live_config.bind != state.active_bind
            && state.active_bind.ip().is_loopback()
            && !live_config.bind.ip().is_loopback();
        if needs_restart {
            sharing.discoverable.push_str(" (restart V2RayDAR)");
        }
        let show_endpoint =
            sharing.subscription_url.is_some() && should_include_token_in_url(&live_config.token);
        let discoverable = if show_endpoint {
            if needs_restart {
                "yes (restart V2RayDAR)".to_string()
            } else {
                "yes".to_string()
            }
        } else {
            sharing.discoverable.clone()
        };
        let proxy = if live_config.proxy_enabled {
            if live_config.proxy_discoverable {
                let lan_ip = crate::network::primary_lan_ip()
                    .map_or_else(|| "?".to_string(), |ip| ip.to_string());
                format!("yes http://{lan_ip}:{}", live_config.proxy_port)
            } else {
                format!("yes http://127.0.0.1:{}", live_config.proxy_port)
            }
        } else {
            "off".to_string()
        };
        Self {
            live_config,
            sharing,
            show_endpoint,
            discoverable,
            proxy,
        }
    }
}

pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    _runtime_config: &RuntimeConfig,
    _paths: &AppPaths,
) {
    if area.height < 3 || area.width < 20 {
        return;
    }

    let view = ConfigView::from_state(state);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Current Configuration");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    if inner.height <= 5 || inner.width < 40 {
        draw_compact(frame, inner, &view);
        return;
    }

    let group_height = inner.height.saturating_sub(1).min(10);
    let endpoint_height = inner.height.saturating_sub(group_height);
    let [groups, endpoint] = Layout::vertical([
        Constraint::Length(group_height),
        Constraint::Length(endpoint_height.max(1)),
    ])
    .areas(inner);
    let [service, network] =
        Layout::horizontal([Constraint::Percentage(44), Constraint::Percentage(56)]).areas(groups);
    draw_group(
        frame,
        service,
        "Service",
        vec![
            ("bind", view.live_config.bind.to_string()),
            ("top_n", view.live_config.top_n.to_string()),
            ("refresh", format!("{}s", view.live_config.refresh_seconds)),
            (
                "stability",
                bool_text(view.live_config.prioritize_stability).to_string(),
            ),
            (
                "asap",
                bool_text(view.live_config.return_configs_asap).to_string(),
            ),
            (
                "scan_all",
                bool_text(view.live_config.scan_all_configs).to_string(),
            ),
            (
                "subscriptions",
                format!(
                    "{}/{}",
                    view.live_config.enabled_subscription_count,
                    view.live_config.subscription_count
                ),
            ),
            (
                "max_sub_mb",
                format_mb(view.live_config.max_subscription_bytes),
            ),
            ("probe", view.live_config.probe_mode.clone()),
            (
                "batch",
                format_batch_size(view.live_config.probe_batch_size),
            ),
        ],
    );
    draw_group(
        frame,
        network,
        "Network",
        vec![
            ("sharing", view.sharing.sharing.to_string()),
            (
                "token",
                bool_text(view.live_config.require_token).to_string(),
            ),
            ("discoverable", view.discoverable),
            ("proxy", view.proxy),
            ("firewall", view.sharing.firewall),
        ],
    );
    draw_subscription_endpoint(
        frame,
        endpoint,
        view.show_endpoint
            .then_some(view.sharing.subscription_url.as_deref())
            .flatten(),
    );
}

fn draw_compact(frame: &mut Frame<'_>, area: Rect, view: &ConfigView) {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("bind: ", Style::default().fg(Color::DarkGray)),
            Span::raw(view.live_config.bind.to_string()),
        ]),
        Line::from(vec![
            Span::styled("top_n: ", Style::default().fg(Color::DarkGray)),
            Span::raw(view.live_config.top_n.to_string()),
            Span::styled("  refresh: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}s", view.live_config.refresh_seconds)),
            Span::styled("  probe: ", Style::default().fg(Color::DarkGray)),
            Span::raw(view.live_config.probe_mode.clone()),
        ]),
        Line::from(vec![
            Span::styled("sharing: ", Style::default().fg(Color::DarkGray)),
            Span::raw(view.sharing.sharing.to_string()),
            Span::styled("  discoverable: ", Style::default().fg(Color::DarkGray)),
            Span::raw(view.discoverable.clone()),
        ]),
    ];
    if view.show_endpoint {
        lines.push(Line::from(vec![
            Span::styled("subs: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!(
                "{}/{}",
                view.live_config.enabled_subscription_count, view.live_config.subscription_count
            )),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn format_mb(bytes: usize) -> String {
    format!("{}MB", bytes / 1_048_576)
}

fn format_batch_size(value: Option<usize>) -> String {
    value.map_or_else(|| "auto".to_string(), |size| size.to_string())
}

fn draw_group(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &'static str,
    rows: Vec<(&'static str, String)>,
) {
    let mut lines = vec![Line::from(Span::styled(
        title,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    let visible = area.height.saturating_sub(1) as usize;
    for (key, value) in rows.into_iter().take(visible) {
        lines.push(Line::from(vec![
            Span::styled(format!("{key:<14}"), Style::default().fg(Color::DarkGray)),
            Span::styled(value, Style::default().fg(Color::White)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn draw_subscription_endpoint(frame: &mut Frame<'_>, area: Rect, url: Option<&str>) {
    if area.is_empty() {
        return;
    }

    let Some(url) = url else {
        clear_endpoint_area(frame.buffer_mut(), area);
        return;
    };

    if !is_safe_terminal_link(url) {
        draw_plain_subscription_endpoint(frame, area, url);
        return;
    }

    let label_width = endpoint_label_width(area.width);
    let url_width = area.width.saturating_sub(label_width);
    if url_width == 0 {
        return;
    }

    let label_style = Style::default().fg(Color::DarkGray);
    let url_style = Style::default().fg(Color::Cyan);
    let mut remaining = url;

    for row in 0..area.height {
        let y = area.y + row;
        let prefix = if row == 0 {
            padded_label("subscription", label_width as usize)
        } else {
            " ".repeat(label_width as usize)
        };
        if label_width > 0 {
            frame
                .buffer_mut()
                .set_string(area.x, y, prefix, label_style);
        }

        let url_x = area.x + label_width;
        if remaining.is_empty() {
            clear_forced_width(frame.buffer_mut(), url_x, y, url_width);
            continue;
        }

        let (chunk, rest) = split_ascii_at_width(remaining, url_width as usize);
        draw_hyperlink_chunk(
            frame.buffer_mut(),
            url_x,
            y,
            url_width,
            url,
            chunk,
            url_style,
        );
        remaining = rest;
    }
}

fn draw_plain_subscription_endpoint(frame: &mut Frame<'_>, area: Rect, url: &str) {
    let text = url
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    "{:<width$}",
                    "subscription",
                    width = 14.min(area.width as usize)
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                text,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ]))
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_hyperlink_chunk(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    target: &str,
    text: &str,
    style: Style,
) {
    if text.is_empty() {
        clear_forced_width(buffer, x, y, width);
        return;
    }

    let mut symbol = String::with_capacity(
        TUI_OSC8_LINK_PREFIX.len()
            + target.len()
            + TUI_OSC8_LINK_SEPARATOR.len()
            + text.len()
            + TUI_OSC8_LINK_SUFFIX.len()
            + width as usize,
    );
    symbol.push_str(TUI_OSC8_LINK_PREFIX);
    symbol.push_str(target);
    symbol.push_str(TUI_OSC8_LINK_SEPARATOR);
    symbol.push_str(TUI_ANSI_UNDERLINE_ENABLE);
    symbol.push_str(text);
    symbol.push_str(TUI_ANSI_UNDERLINE_DISABLE);
    symbol.push_str(TUI_OSC8_LINK_SUFFIX);
    symbol.extend(std::iter::repeat_n(
        ' ',
        (width as usize).saturating_sub(text.len()),
    ));

    set_forced_width_cell(buffer, x, y, width, &symbol, style);
}

fn clear_endpoint_area(buffer: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        clear_forced_width(buffer, area.x, y, area.width);
    }
}

fn clear_forced_width(buffer: &mut Buffer, x: u16, y: u16, width: u16) {
    if width == 0 {
        return;
    }
    let symbol = " ".repeat(width as usize);
    set_forced_width_cell(buffer, x, y, width, &symbol, Style::default());
}

fn set_forced_width_cell(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    symbol: &str,
    style: Style,
) {
    let Some(width) = NonZeroU16::new(width) else {
        return;
    };
    if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_symbol(symbol)
            .set_style(style)
            .set_diff_option(CellDiffOption::ForcedWidth(width));
    }
}

const fn endpoint_label_width(area_width: u16) -> u16 {
    if area_width <= 14 { 0 } else { 14 }
}

fn padded_label(label: &str, width: usize) -> String {
    if width == 0 {
        String::new()
    } else {
        format!("{label:<width$}")
    }
}

fn split_ascii_at_width(value: &str, width: usize) -> (&str, &str) {
    let split = value.len().min(width);
    value.split_at(split)
}

fn is_safe_terminal_link(value: &str) -> bool {
    value.is_ascii() && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_ascii_url_at_display_width() {
        assert_eq!(split_ascii_at_width("abcdef", 4), ("abcd", "ef"));
        assert_eq!(split_ascii_at_width("abc", 4), ("abc", ""));
    }

    #[test]
    fn terminal_link_rejects_control_sequences() {
        assert!(is_safe_terminal_link(
            "http://127.0.0.1:27141/subscription?token=abc"
        ));
        assert!(!is_safe_terminal_link(
            "http://127.0.0.1:27141/subscription?token=\x1b]8;;bad"
        ));
    }
}
