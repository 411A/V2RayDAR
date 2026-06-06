use chrono::{DateTime, Local, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::model::{RuntimeConfig, RuntimeState};

use super::util::human_bytes;

pub fn draw(frame: &mut Frame<'_>, area: Rect, runtime: &RuntimeState, config: &RuntimeConfig) {
    let now = Utc::now();
    let failed = runtime
        .total_candidates
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
            "Time",
            now.with_timezone(&Local).format("%H:%M:%S").to_string(),
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

fn refresh_status(runtime: &RuntimeState, refresh_seconds: u64, now: DateTime<Utc>) -> String {
    if runtime.refreshing {
        return "running".to_string();
    }

    if refresh_seconds == 0 {
        return "manual".to_string();
    }

    let Some(finished_at) = runtime.refresh_finished_at.as_deref() else {
        return "pending".to_string();
    };
    let Ok(finished_at) = DateTime::parse_from_rfc3339(finished_at) else {
        return format!("next {}", format_duration(refresh_seconds as u128 * 1000));
    };

    let elapsed = now
        .signed_duration_since(finished_at.with_timezone(&Utc))
        .num_seconds()
        .max(0) as u64;
    let remaining = refresh_seconds.saturating_sub(elapsed);
    format!("next {}", format_duration(remaining as u128 * 1000))
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
