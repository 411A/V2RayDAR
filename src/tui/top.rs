use std::time::{Duration, Instant};

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
// 18 chars fits `running H:MM:SS` on multi-hour stuck refreshes; the ping
// countdown lives on the box's next line, never beside the fetch value.
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

    // Single quantized clock for every ticking display in this frame.
    // Running For flips on whole seconds since app start, but the refresh
    // and ping countdowns flip on whole seconds since their own sleep-schedule
    // instants — different sub-second phases, rescheduled every cycle. Without
    // a common phase the digits visibly update one-after-another (up to a
    // second apart, alternating order), even though one draw call renders all
    // of them. Rounding frame time down to the app-start second grid makes all
    // three flip in the same frame; values stay truthful within a second.
    let frame_now = frame_tick(instant_now, app_started_at);
    let elapsed = frame_now.saturating_duration_since(app_started_at);
    let failed = runtime
        .tested_candidates
        .saturating_sub(runtime.reachable_candidates);
    let refresh = refresh_status(runtime, config.refresh_seconds, frame_now);
    let refresh_extra = ping_status(runtime, config.ping_seconds, frame_now);
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
        draw_full_grid(frame, area, &cells, refresh_extra.as_deref());
    } else if area.height >= 3 {
        draw_dense_grid(frame, area, &cells);
    } else {
        draw_minimal_line(frame, area, &cells);
    }
}

/// `refresh_extra` is the ping countdown shown on the Refresh box's next
/// line (`None` hides the line when ping is disabled). The box is exactly
/// tall enough (borders + label + two values = 5 rows), so the ping text
/// always stays inside the border instead of overflowing the cell.
fn draw_full_grid(
    frame: &mut Frame<'_>,
    area: Rect,
    cells: &[(&str, String); 8],
    refresh_extra: Option<&str>,
) {
    let chunks = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(area);
    for (row, chunk) in chunks.iter().enumerate() {
        let inner = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(*chunk);
        for (column, cell_area) in inner.iter().enumerate() {
            let index = row * 2 + column;
            let (label, value) = &cells[index];
            let value_style = Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD);
            let mut text = vec![
                Line::from(Span::styled(*label, Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled(value.clone(), value_style)),
            ];
            // Index 1 is the Refresh cell; its extra line is the ping countdown.
            if index == 1
                && let Some(extra) = refresh_extra
            {
                text.push(Line::from(Span::styled(extra.to_string(), value_style)));
            }
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

/// Frame time rounded down to whole seconds since app start: the shared
/// phase grid every ticking display in one frame must use (see `draw`).
/// Never in the future, monotonic with `instant_now`.
fn frame_tick(instant_now: Instant, app_started_at: Instant) -> Instant {
    let tick = instant_now.saturating_duration_since(app_started_at);
    app_started_at + Duration::from_secs(tick.as_secs())
}

/// First line of the Refresh box: fetch state, or the running cycle.
/// Single line by design — the full grid is 15 cells wide, so the ping
/// countdown lives on the box's next line instead of beside it.
fn refresh_status(runtime: &RuntimeView, refresh_seconds: u64, now: Instant) -> String {
    if runtime.refreshing {
        let elapsed = runtime
            .refresh_started_instant
            .map_or(0, |t| now.saturating_duration_since(t).as_secs());
        return format!("running {}", format_duration_ms(elapsed));
    }

    if runtime.pinging {
        let elapsed = runtime
            .last_ping_instant
            .map_or(0, |t| now.saturating_duration_since(t).as_secs());
        return format!("ping {}", format_duration_ms(elapsed));
    }

    fetch_countdown(runtime, refresh_seconds, now).unwrap_or_else(|| "manual".to_string())
}

/// Second line of the Refresh box.
///
/// Hidden while any cycle runs (a refresh already probes, and a ping run
/// announces itself on the first line), and while ping is disabled — the
/// countdown only appears on an idle TUI. At zero remaining the cycle is
/// imminent, so it reads `pinging` instead of `ping 0s`.
fn ping_status(runtime: &RuntimeView, ping_seconds: u64, now: Instant) -> Option<String> {
    if runtime.refreshing || runtime.pinging {
        return None;
    }
    let remaining = ping_remaining(runtime, ping_seconds, now)?;
    if remaining == 0 {
        return Some("pinging".to_string());
    }
    Some(format!("ping {}", format_duration(remaining)))
}

fn fetch_countdown(runtime: &RuntimeView, refresh_seconds: u64, now: Instant) -> Option<String> {
    if refresh_seconds == 0 {
        return None;
    }

    // Prefer the explicit deadline set by the refresh loop when it schedules
    // its sleep. It matches the actual timer; recomputing from `finished_at`
    // drifts by proxy-switch/health-check time after each refresh.
    if let Some(deadline) = runtime.next_refresh_instant {
        let remaining = deadline.saturating_duration_since(now).as_secs();
        return Some(format!(
            "next {}",
            format_duration(u128::from(remaining) * 1000)
        ));
    }

    runtime.refresh_finished_instant.map_or_else(
        || Some("pending".to_string()),
        |finished_at| {
            let elapsed = now.saturating_duration_since(finished_at).as_secs();
            let remaining = refresh_seconds.saturating_sub(elapsed);
            Some(format!(
                "next {}",
                format_duration(u128::from(remaining) * 1000)
            ))
        },
    )
}

/// Remaining ping interval in milliseconds (`None` when ping is disabled).
fn ping_remaining(runtime: &RuntimeView, ping_seconds: u64, now: Instant) -> Option<u128> {
    if ping_seconds == 0 {
        return None;
    }

    if let Some(deadline) = runtime.next_ping_instant {
        let remaining = deadline.saturating_duration_since(now).as_secs();
        return Some(u128::from(remaining) * 1000);
    }

    runtime.last_ping_instant.map_or_else(
        || Some(u128::from(ping_seconds) * 1000),
        |started| {
            let elapsed = now.saturating_duration_since(started).as_secs();
            let remaining = ping_seconds.saturating_sub(elapsed);
            Some(u128::from(remaining) * 1000)
        },
    )
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

    use super::{format_duration_ms, ping_status, refresh_status};
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
    fn fetch_and_ping_lines_are_separate() {
        let now = Instant::now();
        let runtime = RuntimeView {
            refreshing: false,
            next_refresh_instant: Some(now + Duration::from_secs(900)),
            next_ping_instant: Some(now + Duration::from_secs(300)),
            ..RuntimeView::default()
        };
        assert_eq!(refresh_status(&runtime, 900, now), "next 15:00");
        assert_eq!(
            ping_status(&runtime, 300, now),
            Some("ping 05:00".to_string())
        );
    }

    #[test]
    fn pinging_shows_ping_elapsed() {
        let now = Instant::now();
        let started = now.checked_sub(Duration::from_secs(65)).unwrap_or(now);
        let elapsed = now.saturating_duration_since(started).as_secs();
        let runtime = RuntimeView {
            refreshing: false,
            pinging: true,
            last_ping_instant: Some(started),
            ..RuntimeView::default()
        };
        assert_eq!(
            refresh_status(&runtime, 900, now),
            format!("ping {}", format_duration_ms(elapsed))
        );
    }

    #[test]
    fn manual_fetch_leaves_ping_line_intact() {
        let now = Instant::now();
        let runtime = RuntimeView {
            refreshing: false,
            next_ping_instant: Some(now + Duration::from_secs(300)),
            ..RuntimeView::default()
        };
        assert_eq!(refresh_status(&runtime, 0, now), "manual".to_string());
        assert_eq!(
            ping_status(&runtime, 300, now),
            Some("ping 05:00".to_string())
        );
    }

    #[test]
    fn ping_line_hidden_when_disabled() {
        let runtime = RuntimeView::default();
        assert_eq!(ping_status(&runtime, 0, Instant::now()), None);
    }

    #[test]
    fn ping_line_hidden_while_any_cycle_runs() {
        let now = Instant::now();
        let refreshing = RuntimeView {
            refreshing: true,
            next_ping_instant: Some(now + Duration::from_secs(300)),
            ..RuntimeView::default()
        };
        assert_eq!(ping_status(&refreshing, 300, now), None);
        let pinging = RuntimeView {
            pinging: true,
            next_ping_instant: Some(now + Duration::from_secs(300)),
            ..RuntimeView::default()
        };
        assert_eq!(ping_status(&pinging, 300, now), None);
    }

    #[test]
    fn ping_line_shows_pinging_at_zero_remaining() {
        let now = Instant::now();
        let runtime = RuntimeView {
            refreshing: false,
            next_ping_instant: Some(now),
            ..RuntimeView::default()
        };
        assert_eq!(ping_status(&runtime, 300, now), Some("pinging".to_string()));
    }

    #[test]
    fn both_timers_off_shows_manual() {
        let runtime = RuntimeView::default();
        assert_eq!(
            refresh_status(&runtime, 0, Instant::now()),
            "manual".to_string()
        );
    }

    #[test]
    fn refresh_box_shows_ping_on_its_own_line_inside_border() {
        use ratatui::{Terminal, backend::TestBackend};
        let backend = TestBackend::new(120, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let now = Instant::now();
        let runtime = RuntimeView {
            refreshing: false,
            next_refresh_instant: Some(now + Duration::from_secs(900)),
            next_ping_instant: Some(now + Duration::from_secs(300)),
            ..RuntimeView::default()
        };
        let app_config = crate::config::AppConfig::default_for_first_run();
        let config = crate::model::RuntimeConfig::from(&app_config);
        terminal
            .draw(|frame| {
                super::draw(frame, frame.area(), &runtime, &config, now, now);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let mut saw_fetch = false;
        let mut saw_ping_inside = false;
        for y in 0..5 {
            let mut line = String::new();
            for x in 0..120 {
                line.push_str(buffer[(x, y)].symbol());
            }
            if line.contains("next 15:00") {
                saw_fetch = true;
            }
            if let Some(start) = line.find("ping 05:00") {
                // The ping countdown must sit inside the Refresh cell:
                // a cell border must precede it on the same visual row.
                assert!(
                    line[..start].contains(['│', '┃', '|']),
                    "ping line escaped its box: {line}"
                );
                saw_ping_inside = true;
            }
            assert!(
                !line.contains("· ping"),
                "ping must not share the fetch line: {line}"
            );
        }
        assert!(saw_fetch && saw_ping_inside);
    }

    #[test]
    fn frame_tick_never_runs_ahead_and_keeps_whole_seconds() {
        let started = Instant::now();
        let now = started + Duration::from_millis(1500);
        let ticked = super::frame_tick(now, started);
        assert!(ticked <= now);
        assert_eq!(ticked.saturating_duration_since(started).as_secs(), 1);
        assert_eq!(super::frame_tick(started, started), started);
    }

    #[test]
    fn all_timers_flip_on_the_same_frame() {
        use super::{format_duration_hms, ping_status, refresh_status};

        let started = Instant::now();
        // Deliberately off-phase deadlines, like real sleep-schedule instants:
        // sub-second parts differ from each other and from app start.
        let runtime = RuntimeView {
            refreshing: false,
            next_refresh_instant: Some(started + Duration::from_millis(900_350)),
            next_ping_instant: Some(started + Duration::from_millis(200_700)),
            ..RuntimeView::default()
        };
        // Sample every 10ms across 3s, like (denser) frames.
        let mut elapsed_flips = Vec::new();
        let mut refresh_flips = Vec::new();
        let mut ping_flips = Vec::new();
        let (mut prev_e, mut prev_r, mut prev_p) = (String::new(), String::new(), String::new());
        let mut t_ms = 100_000u64;
        while t_ms < 103_000 {
            let now = started + Duration::from_millis(t_ms);
            // Mirror `draw`: one quantized clock feeds every display.
            let frame_now = super::frame_tick(now, started);
            let shown_elapsed =
                format_duration_hms(frame_now.saturating_duration_since(started).as_secs());
            let shown_refresh = refresh_status(&runtime, 900, frame_now);
            let shown_ping = ping_status(&runtime, 300, frame_now).unwrap_or_default();
            if shown_elapsed != prev_e {
                elapsed_flips.push(t_ms);
            }
            if shown_refresh != prev_r {
                refresh_flips.push(t_ms);
            }
            if shown_ping != prev_p {
                ping_flips.push(t_ms);
            }
            prev_e = shown_elapsed;
            prev_r = shown_refresh;
            prev_p = shown_ping;
            t_ms += 10;
        }
        for flips in [&elapsed_flips, &refresh_flips, &ping_flips] {
            assert!(
                flips.len() >= 3,
                "each timer must tick several times in 3s: {flips:?}"
            );
        }
        assert_eq!(
            elapsed_flips, refresh_flips,
            "Running For and Refresh must flip together"
        );
        assert_eq!(
            elapsed_flips, ping_flips,
            "Running For and ping must flip together"
        );
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
