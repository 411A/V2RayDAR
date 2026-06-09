use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{model::RuntimeConfig, network::sharing_status, paths::AppPaths};

use super::{state::TuiState, util::bool_text};

pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    _runtime_config: &RuntimeConfig,
    _paths: &AppPaths,
) {
    let live_config = RuntimeConfig::from(&state.editable);
    let mut sharing = sharing_status(&live_config);
    if live_config.sharing_enabled
        && live_config.bind != state.active_bind
        && state.active_bind.ip().is_loopback()
        && !live_config.bind.ip().is_loopback()
    {
        sharing.discoverable.push_str(" (restart V2RayDAR)");
    }
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
            ("bind", live_config.bind.to_string()),
            ("top_n", live_config.top_n.to_string()),
            ("refresh", format!("{}s", live_config.refresh_seconds)),
            (
                "stability",
                bool_text(live_config.prioritize_stability).to_string(),
            ),
            (
                "scan_all",
                bool_text(live_config.scan_all_configs).to_string(),
            ),
            (
                "subscriptions",
                format!(
                    "{}/{}",
                    live_config.enabled_subscription_count, live_config.subscription_count
                ),
            ),
            ("max_sub_mb", format_mb(live_config.max_subscription_bytes)),
            ("probe", live_config.probe_mode.clone()),
            ("batch", format_batch_size(live_config.probe_batch_size)),
        ],
    );
    draw_group(
        frame,
        network,
        "Network",
        vec![
            ("sharing", sharing.sharing.to_string()),
            ("token", bool_text(live_config.require_token).to_string()),
            ("discoverable", sharing.discoverable),
            ("firewall", sharing.firewall),
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
            Span::styled(format!("{key:<14}"), Style::default().fg(Color::DarkGray)),
            Span::styled(value, Style::default().fg(Color::White)),
        ])
    }));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}
