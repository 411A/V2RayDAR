use std::time::Instant;

use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::model::RuntimeConfig;

use super::{util::human_bytes, view::RuntimeView};

pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    runtime: &RuntimeView,
    config: &RuntimeConfig,
    app_started_at: Instant,
) {
    let now = Utc::now();
    let failed = runtime
        .tested_candidates
        .saturating_sub(runtime.reachable_candidates);
    let refresh = refresh_status(runtime, config.refresh_seconds, now);
    let speedtest = if config.speedtest_enabled {
        human_bytes(runtime.speedtest_bytes)
    } else {
        "off".to_string()
    };
    let scan_time = runtime
        .refresh_duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "-".to_string());
    let cells = [
        (
            "Running For",
            format_duration_hms(app_started_at.elapsed().as_secs()),
        ),
        ("Refresh", refresh),
        ("Last Scan Time", scan_time),
        ("Fetched", runtime.total_candidates.to_string()),
        ("Failed", failed.to_string()),
        ("Working", runtime.reachable_candidates.to_string()),
        ("Sub Usage", human_bytes(runtime.fetch_bytes)),
        ("Speedtest Usage", speedtest),
    ];

    let chunks = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(area);
    for (row, chunk) in chunks.iter().enumerate() {
        let inner = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(*chunk);
        for (column, cell_area) in inner.iter().enumerate() {
            let index = row * 2 + column;
            let (label, value) = &cells[index];
            let text = vec![
                Line::from(Span::styled(*label, Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled(
                    value.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
            ];
            frame.render_widget(
                Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
                *cell_area,
            );
        }
    }
}

fn refresh_status(runtime: &RuntimeView, refresh_seconds: u64, now: DateTime<Utc>) -> String {
    if runtime.refreshing {
        let elapsed = runtime
            .refresh_started_at
            .map(|started_at| now.signed_duration_since(started_at).num_seconds().max(0) as u64)
            .unwrap_or(0);
        return format!("running {}", format_duration_ms(elapsed));
    }

    if refresh_seconds == 0 {
        return "manual".to_string();
    }

    let Some(finished_at) = runtime.refresh_finished_at else {
        return "pending".to_string();
    };
    let elapsed = now.signed_duration_since(finished_at).num_seconds().max(0) as u64;
    let remaining = refresh_seconds.saturating_sub(elapsed);
    format!("next {}", format_duration(remaining as u128 * 1000))
}

fn format_duration_hms(total_seconds: u64) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn format_duration_ms(total_seconds: u64) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn format_duration(ms: u128) -> String {
    let seconds = (ms / 1000) as u64;
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}
