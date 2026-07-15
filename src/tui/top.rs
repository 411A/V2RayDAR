use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::model::RuntimeConfig;

use super::{util::human_bytes, view::RuntimeView};

// Fixed widths per value prevent the Paragraph trailing-space style boundary
// from shifting when digits change, which is what causes terminal flicker.
const W_RUNNING_FOR: usize = 8;
const W_REFRESH: usize = 14;
const W_LAST_SCAN: usize = 6;
const W_FETCHED: usize = 6;
const W_FAILED: usize = 6;
const W_WORKING: usize = 6;
const W_SUB_USAGE: usize = 10;
const W_SPEEDTEST: usize = 10;

#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    runtime: &RuntimeView,
    config: &RuntimeConfig,
    app_started_at: Instant,
    instant_now: Instant,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let elapsed = instant_now.saturating_duration_since(app_started_at);
    let failed = runtime
        .tested_candidates
        .saturating_sub(runtime.reachable_candidates);
    let refresh = refresh_status(runtime, config.refresh_seconds, instant_now);
    let speedtest = if config.speedtest_enabled {
        human_bytes(runtime.speedtest_bytes)
    } else {
        "off".to_string()
    };
    let scan_time = runtime
        .refresh_duration_ms
        .map_or_else(|| "-".to_string(), format_duration);
    let cells = [
        (
            "Running For",
            format!(
                "{:<w$}",
                format_duration_hms(elapsed.as_secs()),
                w = W_RUNNING_FOR
            ),
        ),
        ("Refresh", format!("{refresh:<W_REFRESH$}")),
        ("Last Scan", format!("{scan_time:<W_LAST_SCAN$}")),
        (
            "Fetched",
            format!(
                "{runtime_fetched:<W_FETCHED$}",
                runtime_fetched = runtime.total_candidates
            ),
        ),
        ("Failed", format!("{failed:<W_FAILED$}")),
        (
            "Working",
            format!(
                "{runtime_working:<W_WORKING$}",
                runtime_working = runtime.reachable_candidates
            ),
        ),
        (
            "Sub Usage",
            format!(
                "{sub_usage:<W_SUB_USAGE$}",
                sub_usage = human_bytes(runtime.fetch_bytes)
            ),
        ),
        ("Speedtest", format!("{speedtest:<W_SPEEDTEST$}")),
    ];

    if area.height >= 5 {
        draw_full_grid(frame, area, &cells);
    } else if area.height >= 3 {
        draw_dense_grid(frame, area, &cells);
    } else {
        draw_minimal_line(frame, area, &cells);
    }
}

fn draw_full_grid(frame: &mut Frame<'_>, area: Rect, cells: &[(&str, String); 8]) {
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

fn draw_dense_grid(frame: &mut Frame<'_>, area: Rect, cells: &[(&str, String); 8]) {
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let half = area.height / 2;
    let [top_half, bottom_half] =
        Layout::vertical([Constraint::Length(half), Constraint::Min(half)]).areas(area);

    for (row_index, row_area) in [top_half, bottom_half].iter().enumerate() {
        if row_area.height == 0 {
            continue;
        }
        let start = row_index * 4;
        let row_cells = &cells[start..start + 4];
        let columns = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(*row_area);
        for (col_index, col_area) in columns.iter().enumerate() {
            if col_area.width == 0 {
                continue;
            }
            let (label, value) = &row_cells[col_index];
            let text = Line::from(vec![
                Span::styled(format!("{label}: "), label_style),
                Span::styled(value.clone(), value_style),
            ]);
            frame.render_widget(Paragraph::new(text), *col_area);
        }
    }
}

fn draw_minimal_line(frame: &mut Frame<'_>, area: Rect, cells: &[(&str, String); 8]) {
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let sep_span = Span::styled(" | ", Style::default().fg(Color::DarkGray));
    let sep_width: usize = 3;

    let all_stats: [(usize, &str); 8] = [
        (0, "Up"),
        (1, "Refresh"),
        (2, "Scan"),
        (3, "Fetch"),
        (4, "Fail"),
        (5, "Work"),
        (6, "Sub"),
        (7, "Speed"),
    ];

    let mut spans = Vec::new();
    let mut used_width: usize = 0;
    let max_width = area.width as usize;

    for (i, (idx, short_label)) in all_stats.iter().enumerate() {
        let (_, value) = &cells[*idx];
        let entry_width = short_label.len() + 2 + value.len();
        let needed = if i == 0 {
            entry_width
        } else {
            sep_width + entry_width
        };

        if used_width + needed > max_width {
            break;
        }
        used_width += needed;

        if !spans.is_empty() {
            spans.push(sep_span.clone());
        }
        spans.push(Span::styled(format!("{short_label}: "), label_style));
        spans.push(Span::styled(value.clone(), value_style));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn refresh_status(runtime: &RuntimeView, refresh_seconds: u64, now: Instant) -> String {
    if runtime.refreshing {
        let elapsed = runtime
            .refresh_started_instant
            .map_or(0, |t| now.saturating_duration_since(t).as_secs());
        return format!("running {}", format_duration_ms(elapsed));
    }

    if refresh_seconds == 0 {
        return "manual".to_string();
    }

    let Some(finished_at) = runtime.refresh_finished_instant else {
        return "pending".to_string();
    };
    let elapsed = now.saturating_duration_since(finished_at).as_secs();
    let remaining = refresh_seconds.saturating_sub(elapsed);
    format!("next {}", format_duration(u128::from(remaining) * 1000))
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
    let seconds = millis_to_seconds(ms);
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn millis_to_seconds(ms: u128) -> u64 {
    u64::try_from(ms / 1000).unwrap_or(u64::MAX)
}
