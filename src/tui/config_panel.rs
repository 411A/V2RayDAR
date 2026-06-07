use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{model::RuntimeConfig, paths::AppPaths};

use super::{state::TuiState, util::bool_text};

pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    _state: &TuiState,
    runtime_config: &RuntimeConfig,
    _paths: &AppPaths,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Current Configuration");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [service, network] =
        Layout::horizontal([Constraint::Percentage(44), Constraint::Percentage(56)]).areas(inner);
    draw_group(
        frame,
        service,
        "Service",
        vec![
            ("bind", runtime_config.bind.to_string()),
            ("top_n", runtime_config.top_n.to_string()),
            ("refresh", format!("{}s", runtime_config.refresh_seconds)),
            (
                "stability",
                bool_text(runtime_config.prioritize_stability).to_string(),
            ),
            (
                "max_sub_mb",
                format_mb(runtime_config.max_subscription_bytes),
            ),
            ("probe", runtime_config.probe_mode.clone()),
            ("batch", format_batch_size(runtime_config.probe_batch_size)),
        ],
    );
    draw_group(
        frame,
        network,
        "Network",
        vec![
            (
                "sharing",
                bool_text(runtime_config.sharing_enabled).to_string(),
            ),
            ("token", bool_text(runtime_config.require_token).to_string()),
            (
                "discoverable",
                if runtime_config.sharing_enabled && !runtime_config.bind.ip().is_loopback() {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
            ),
            ("firewall", firewall_hint(runtime_config)),
        ],
    );
}

fn format_mb(bytes: usize) -> String {
    format!("{}MB", bytes / 1_048_576)
}

fn format_batch_size(value: Option<usize>) -> String {
    value
        .map(|size| size.to_string())
        .unwrap_or_else(|| "auto".to_string())
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
    lines.extend(rows.into_iter().map(|(key, value)| {
        Line::from(vec![
            Span::styled(format!("{key:<13}"), Style::default().fg(Color::DarkGray)),
            Span::styled(value, Style::default().fg(Color::White)),
        ])
    }));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn firewall_hint(config: &RuntimeConfig) -> String {
    if config.sharing_enabled && !config.bind.ip().is_loopback() {
        "allow inbound TCP if clients cannot reach /health".to_string()
    } else {
        "not required for local-only bind".to_string()
    }
}
