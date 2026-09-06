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
// 10 chars fits `HH:MM:SS` past 100h (`100:00:00`) so long runs stay aligned.
// 18 chars fits `running H:MM:SS` on multi-hour stuck refreshes.
const W_RUNNING_FOR: usize = 10;
const W_REFRESH: usize = 18;
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

    // Prefer the explicit deadline set by the refresh loop when it schedules
    // its sleep. It matches the actual timer; recomputing from `finished_at`
    // drifts by proxy-switch/health-check time after each refresh.
    if let Some(deadline) = runtime.next_refresh_instant {
        let remaining = deadline.saturating_duration_since(now).as_secs();
        return format!("next {}", format_duration(u128::from(remaining) * 1000));
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
    // Past one hour (e.g. a refresh stalled for hours on a 30h+ run),
    // switch to `H:MM:SS` so it stays comparable with `Running For`
    // instead of growing an unbounded `MM:SS` minute count.
    if total_seconds >= 3600 {
        return format_duration_hms(total_seconds);
    }
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{format_duration_ms, refresh_status};
    use crate::tui::view::RuntimeView;

    #[test]
    fn running_refresh_uses_hms_past_one_hour() {
        assert_eq!(format_duration_ms(90), "01:30");
        assert_eq!(format_duration_ms(3600), "01:00:00");
        assert_eq!(format_duration_ms(30 * 3600 + 65), "30:01:05");
    }

    #[test]
    fn countdown_prefers_explicit_deadline() {
        let now = Instant::now();
        let runtime = RuntimeView {
            refreshing: false,
            next_refresh_instant: Some(now + Duration::from_secs(300)),
            refresh_finished_instant: Some(now),
            ..RuntimeView::default()
        };
        assert_eq!(refresh_status(&runtime, 300, now), "next 05:00");
    }

    #[test]
    fn countdown_falls_back_to_finished_instant() {
        let now = Instant::now();
        // `checked_sub`: bare `Instant - Duration` panics when the machine
        // uptime is shorter than the span (fresh CI runners, Windows sleep).
        let finished = now.checked_sub(Duration::from_secs(60)).unwrap_or(now);
        let runtime = RuntimeView {
            refreshing: false,
            next_refresh_instant: None,
            refresh_finished_instant: Some(finished),
            ..RuntimeView::default()
        };
        let expected_remaining =
            300u64.saturating_sub(now.saturating_duration_since(finished).as_secs());
        let expected = format!(
            "next {:02}:{:02}",
            expected_remaining / 60,
            expected_remaining % 60
        );
        assert_eq!(refresh_status(&runtime, 300, now), expected);
    }

    #[test]
    fn running_refresh_stays_mm_ss_under_one_hour() {
        let now = Instant::now();
        // Deterministic on any uptime: derive the expectation from the
        // actual elapsed instead of assuming 90s of machine uptime.
        let started = now.checked_sub(Duration::from_secs(90)).unwrap_or(now);
        let elapsed = now.saturating_duration_since(started).as_secs();
        let runtime = RuntimeView {
            refreshing: true,
            refresh_started_instant: Some(started),
            ..RuntimeView::default()
        };
        assert_eq!(
            refresh_status(&runtime, 300, now),
            format!("running {}", format_duration_ms(elapsed))
        );
        if elapsed >= 90 {
            assert_eq!(refresh_status(&runtime, 300, now), "running 01:30");
        }
    }
}
