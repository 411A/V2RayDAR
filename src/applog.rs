//! User-facing log lines for Live logs (dashboard) and the TUI panels.
//!
//! Footprint rules — keep them: every line is built once as a short
//! `String`, flattened to a single line, truncated to
//! [`MAX_LOG_LINE_LEN`] chars, and stored in the existing bounded
//! `live_logs` ring ([`MAX_TUI_LOGS`](crate::constants::MAX_TUI_LOGS)
//! entries). No file writes, no background tasks, no per-line heap beyond
//! the line itself — worst case ~256 KiB of RAM. Nothing here allocates
//! unless a line is actually emitted.

use std::borrow::Cow;
use std::time::Duration;

use crate::constants::MAX_TUI_LOGS;
use crate::model::RuntimeState;

/// Severity of a user-facing log line, lowest first.
///
/// The wire names are matched by the dashboard parser — keep them in sync
/// with `parseLogLine` in `frontend/app.js`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Wire name (`DEBUG` / `INFO` / `WARN` / `ERROR`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Longest message payload kept per line (chars, not bytes); the
/// timestamp + level tag ride on top.
pub const MAX_LOG_LINE_LEN: usize = 480;

/// Wall-clock stamp for a log line: `20:20:02.792`.
pub fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

/// Tag a payload with its level: `[INFO] message`.
///
/// Single line (SSE framing breaks on raw newlines) and truncated.
/// No timestamp — the progress funnel (`push_tui_progress`) and [`push`]
/// stamp it exactly once.
pub fn line(level: LogLevel, message: &str) -> String {
    let flat: Cow<'_, str> = if message.contains(['\n', '\r']) {
        Cow::Owned(message.replace(['\n', '\r'], " "))
    } else {
        Cow::Borrowed(message)
    };
    if flat.chars().count() <= MAX_LOG_LINE_LEN {
        return format!("[{}] {flat}", level.name());
    }
    // Cut after the first MAX_LOG_LINE_LEN chars: `nth` finds the byte
    // offset where the next char starts, so multibyte chars stay whole.
    let end = flat
        .char_indices()
        .nth(MAX_LOG_LINE_LEN)
        .map_or(flat.len(), |(index, _)| index);
    format!("[{}] {}…", level.name(), &flat[..end])
}

/// Convenience constructor for [`line`] at info level.
pub fn info_line(message: &str) -> String {
    line(LogLevel::Info, message)
}

/// Convenience constructor for [`line`] at warn level.
pub fn warn_line(message: &str) -> String {
    line(LogLevel::Warn, message)
}

/// Timestamped + tagged line for sites that push to the ring directly
/// (bypassing the progress funnel).
pub fn stamped(level: LogLevel, message: &str) -> String {
    format!("{} {}", timestamp(), line(level, message))
}

/// Push one line to the bounded ring, dropping the oldest first.
/// Same cap as the funnel path, so direct pushes can't grow memory either.
pub fn push(state: &mut RuntimeState, level: LogLevel, message: &str) {
    state.live_logs.push(stamped(level, message));
    if state.live_logs.len() > MAX_TUI_LOGS {
        let extra = state.live_logs.len() - MAX_TUI_LOGS;
        state.live_logs.drain(..extra);
    }
}

/// One database-persist line for Live logs: always the debug upsert line,
/// plus an info line when rows were actually deleted. Call it right after
/// a persist so the lines land in cycle order.
pub fn log_persist_stats(
    state: &mut RuntimeState,
    upserted: usize,
    stable_keys: usize,
    cleaned: usize,
    clean_after_days: u32,
) {
    push(
        state,
        LogLevel::Debug,
        &format!("DB: upserted {upserted} configs, {stable_keys} stable keys"),
    );
    if cleaned > 0 {
        push(
            state,
            LogLevel::Info,
            &format!("DB: cleaned {cleaned} offline configs older than {clean_after_days}d"),
        );
    }
}

/// Human byte count for fetch lines: `512 B`, `41.02 KB`, `3.10 MB`.
#[allow(clippy::cast_precision_loss)]
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Short elapsed time for fetch lines: `850ms`, `1.3s`, `75s`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let ms = elapsed.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        format!("{}s", ms / 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_tags_level_and_keeps_message() {
        assert_eq!(info_line("hello"), "[INFO] hello");
        assert_eq!(warn_line("x"), "[WARN] x");
        assert_eq!(line(LogLevel::Debug, "x"), "[DEBUG] x");
        assert_eq!(line(LogLevel::Error, "x"), "[ERROR] x");
    }

    #[test]
    fn line_flattens_newlines_to_single_line() {
        let tagged = warn_line("first\nsecond\rthird");
        assert_eq!(tagged, "[WARN] first second third");
        assert!(!tagged.contains(['\n', '\r']));
    }

    #[test]
    fn line_truncates_at_char_boundary_with_ellipsis() {
        let long = "é".repeat(MAX_LOG_LINE_LEN + 20);
        let tagged = info_line(&long);
        let payload = tagged.strip_prefix("[INFO] ").expect("tagged");
        assert!(payload.ends_with('…'));
        assert_eq!(payload.chars().count(), MAX_LOG_LINE_LEN + 1);
        // `é` is two bytes: a byte-wise cut would panic or split it.
        assert!(payload.is_char_boundary(payload.len() - "…".len()));
    }

    #[test]
    fn timestamp_keeps_hh_mm_ss_mmm_shape() {
        let ts = timestamp();
        assert_eq!(ts.len(), 12);
        assert_eq!(&ts[2..3], ":");
        assert_eq!(&ts[5..6], ":");
        assert_eq!(&ts[8..9], ".");
    }

    #[test]
    fn levels_order_debug_below_error() {
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn formatters_stay_compact() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(42_000), "41.02 KB");
        assert_eq!(format_elapsed(Duration::from_millis(850)), "850ms");
        assert_eq!(format_elapsed(Duration::from_millis(1300)), "1.3s");
    }
}
