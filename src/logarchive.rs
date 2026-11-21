//! Compressed session log archive.
//!
//! The live log ring stays small and uncompressed for real-time rendering and
//! SSE delivery. Older lines move into independently compressed chunks so the
//! complete session stays available without unbounded raw memory:
//!
//! - every line is appended in arrival order and labelled with its global
//!   sequence number;
//! - the active buffer is sealed into a DEFLATE chunk every
//!   [`LOG_CHUNK_LINES`] lines (about 20 KiB uncompressed), so a page read
//!   only decompresses the chunks it needs;
//! - a compressed-byte budget bounds the archive. With the default budget a
//!   1000-hour session of ordinary probe/proxy lines fits; only a pathologically
//!   talkative run loses its very oldest chunks, never the live tail.
//!
//! This is the same idea as the Zstandard seekable format (independent
//! frames plus a page index), using the pure-Rust DEFLATE backend so every
//! target - Windows, Linux, macOS, Termux - builds without a C toolchain.

use std::{
    collections::VecDeque,
    io::{Read, Write},
};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde::Serialize;

/// Lines sealed into one compressed chunk.
pub const LOG_CHUNK_LINES: usize = 256;

/// Compressed history budget. 16 MiB is roughly 300k ordinary log lines at
/// this application's line shape; the live ring is separate and always kept.
pub const LOG_ARCHIVE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Default page size for the dashboard's "load older" request.
pub const LOG_PAGE_DEFAULT: usize = 500;

/// Hard page cap so one request can never decompress an unbounded range.
pub const LOG_PAGE_MAX: usize = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub seq: u64,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogPage {
    pub lines: Vec<LogLine>,
    /// Oldest sequence still retained (0 when the archive is empty).
    pub first_seq: u64,
    /// Lines ever pushed, including chunks pruned by the byte budget.
    pub total: u64,
    pub has_more: bool,
}

#[derive(Debug)]
struct LogChunk {
    first_seq: u64,
    count: usize,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct LogArchive {
    chunks: VecDeque<LogChunk>,
    active: Vec<String>,
    active_first_seq: u64,
    next_seq: u64,
    compressed_bytes: usize,
    max_bytes: usize,
    warned_prune: bool,
}

impl Default for LogArchive {
    fn default() -> Self {
        Self::new(LOG_ARCHIVE_MAX_BYTES)
    }
}

impl LogArchive {
    #[must_use]
    pub const fn new(max_bytes: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            active: Vec::new(),
            active_first_seq: 0,
            next_seq: 1,
            compressed_bytes: 0,
            max_bytes,
            warned_prune: false,
        }
    }

    /// Append one line with its global 1-based sequence number.
    pub fn push(&mut self, seq: u64, line: &str) {
        if self.active.is_empty() {
            self.active_first_seq = seq;
        }
        self.active.push(line.to_string());
        self.next_seq = self.next_seq.max(seq.saturating_add(1));
        if self.active.len() >= LOG_CHUNK_LINES {
            self.seal();
        }
    }

    /// Oldest sequence still retained, or 0 when nothing is stored.
    #[must_use]
    pub fn first_seq(&self) -> u64 {
        self.chunks.front().map_or_else(
            || {
                if self.active.is_empty() {
                    0
                } else {
                    self.active_first_seq
                }
            },
            |chunk| chunk.first_seq,
        )
    }

    /// Total lines ever pushed (the live tail is included).
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.next_seq.saturating_sub(1)
    }

    #[cfg(test)]
    #[must_use]
    pub const fn compressed_bytes(&self) -> usize {
        self.compressed_bytes
    }

    /// Newest `limit` lines strictly older than `before` (exclusive cursor).
    /// `before = u64::MAX` returns the newest page.
    #[must_use]
    pub fn page_before(&self, before: u64, limit: usize) -> LogPage {
        let limit = limit.clamp(1, LOG_PAGE_MAX);
        let mut picked = Vec::new();
        if before == 0 {
            return self.page_from_result(picked);
        }

        for (index, line) in self.active.iter().enumerate().rev() {
            let seq = self.active_first_seq.saturating_add(index as u64);
            if seq < before {
                picked.push(LogLine {
                    seq,
                    line: line.clone(),
                });
                if picked.len() == limit {
                    break;
                }
            }
        }
        if picked.len() < limit {
            'chunks: for chunk in self.chunks.iter().rev() {
                for (index, line) in decompress_chunk(chunk).into_iter().enumerate().rev() {
                    let seq = chunk.first_seq.saturating_add(index as u64);
                    if seq < before {
                        picked.push(LogLine { seq, line });
                        if picked.len() == limit {
                            break 'chunks;
                        }
                    }
                }
            }
        }

        picked.reverse();
        self.page_from_result(picked)
    }

    /// Up to `limit` lines starting at `from` (inclusive). Used by the
    /// "oldest" jump and by the full download.
    #[must_use]
    pub fn page_from(&self, from: u64, limit: usize) -> LogPage {
        let limit = limit.clamp(1, LOG_PAGE_MAX);
        let mut picked = Vec::new();
        'chunks: for chunk in &self.chunks {
            if chunk.first_seq.saturating_add(chunk.count as u64) <= from {
                continue;
            }
            for (index, line) in decompress_chunk(chunk).into_iter().enumerate() {
                let seq = chunk.first_seq.saturating_add(index as u64);
                if seq >= from {
                    picked.push(LogLine { seq, line });
                    if picked.len() == limit {
                        break 'chunks;
                    }
                }
            }
        }
        if picked.len() < limit {
            for (index, line) in self.active.iter().enumerate() {
                let seq = self.active_first_seq.saturating_add(index as u64);
                if seq >= from {
                    picked.push(LogLine {
                        seq,
                        line: line.clone(),
                    });
                    if picked.len() == limit {
                        break;
                    }
                }
            }
        }
        self.page_from_result(picked)
    }

    /// Append every retained line, oldest first, to `out`. Used only by the
    /// explicit full-log download; no per-frame path calls this.
    pub fn append_all(&self, out: &mut String) {
        for chunk in &self.chunks {
            for line in decompress_chunk(chunk) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        for line in &self.active {
            out.push_str(line);
            out.push('\n');
        }
    }

    fn page_from_result(&self, lines: Vec<LogLine>) -> LogPage {
        let first_seq = self.first_seq();
        let has_more = lines
            .first()
            .is_some_and(|line| first_seq != 0 && line.seq > first_seq);
        LogPage {
            lines,
            first_seq,
            total: self.total(),
            has_more,
        }
    }

    fn seal(&mut self) {
        if self.active.is_empty() {
            return;
        }
        let count = self.active.len();
        let first_seq = self.active_first_seq;
        let bytes = compress_lines(&self.active);
        self.compressed_bytes = self.compressed_bytes.saturating_add(bytes.len());
        self.chunks.push_back(LogChunk {
            first_seq,
            count,
            bytes,
        });
        self.active.clear();
        self.active_first_seq = 0;
        self.prune();
    }

    fn prune(&mut self) {
        // Always keep the newest chunk, even if a single chunk is larger than
        // the budget: losing the newest history is worse than a small overrun.
        while self.compressed_bytes > self.max_bytes && self.chunks.len() > 1 {
            if let Some(front) = self.chunks.pop_front() {
                self.compressed_bytes = self.compressed_bytes.saturating_sub(front.bytes.len());
            }
        }
        if self.compressed_bytes > self.max_bytes && !self.warned_prune {
            self.warned_prune = true;
            tracing::warn!(
                max_bytes = self.max_bytes,
                compressed_bytes = self.compressed_bytes,
                chunks = self.chunks.len(),
                "log archive budget reached; oldest chunks will be pruned"
            );
        }
    }
}

fn compress_lines(lines: &[String]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    let mut first = true;
    for line in lines {
        if !first && encoder.write_all(b"\n").is_err() {
            return Vec::new();
        }
        first = false;
        if encoder.write_all(line.as_bytes()).is_err() {
            return Vec::new();
        }
    }
    encoder.finish().unwrap_or_default()
}

fn decompress_chunk(chunk: &LogChunk) -> Vec<String> {
    let mut bytes = Vec::new();
    if ZlibDecoder::new(chunk.bytes.as_slice())
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.split('\n').map(ToString::to_string).collect();
    // Compression never changes the line count; keep the page indexes safe
    // if a truncated/corrupt chunk ever yields a mismatch.
    lines.truncate(chunk.count);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(archive: &mut LogArchive, count: u64) {
        for seq in 1..=count {
            archive.push(seq, &format!("line {seq:06}"));
        }
    }

    #[test]
    fn every_line_survives_and_pages_are_ordered() {
        let mut archive = LogArchive::default();
        fill(&mut archive, 1000);

        assert_eq!(archive.total(), 1000);
        assert_eq!(archive.first_seq(), 1);

        let newest = archive.page_before(u64::MAX, 500);
        assert_eq!(newest.lines.len(), 500);
        assert_eq!(newest.lines.first().map(|l| l.seq), Some(501));
        assert_eq!(newest.lines.last().map(|l| l.seq), Some(1000));
        assert!(newest.has_more);

        let oldest = archive.page_before(501, 500);
        assert_eq!(oldest.lines.len(), 500);
        assert_eq!(oldest.lines.first().map(|l| l.seq), Some(1));
        assert_eq!(oldest.lines.last().map(|l| l.seq), Some(500));
        assert!(!oldest.has_more);

        let from = archive.page_from(1, 100);
        assert_eq!(from.lines.first().map(|l| l.seq), Some(1));
        assert_eq!(from.lines.last().map(|l| l.seq), Some(100));
        assert_eq!(from.lines[0].line, "line 000001");
    }

    #[test]
    fn thousand_hour_volume_fits_the_default_budget() {
        // 300k lines is a conservative 1000-hour volume for this app's log
        // cadence (probe/proxy events, not per-request access logs). The
        // default archive must keep every one, including the oldest.
        let mut archive = LogArchive::default();
        for seq in 1..=300_000_u64 {
            archive.push(
                seq,
                &format!(
                    "2026/10/25 13:20:59.324 [INFO] probe batch {seq:06} config example.com:443 latency {}ms",
                    seq % 900
                ),
            );
        }
        assert_eq!(archive.first_seq(), 1, "the oldest line survives");
        assert!(archive.compressed_bytes() <= LOG_ARCHIVE_MAX_BYTES);
    }

    #[test]
    fn active_lines_before_a_seal_are_pageable() {
        let mut archive = LogArchive::default();
        fill(&mut archive, 10);
        let page = archive.page_before(u64::MAX, 10);
        assert_eq!(page.lines.len(), 10);
        assert_eq!(page.lines[9].line, "line 000010");
        assert_eq!(page.first_seq, 1);
    }

    #[test]
    fn compression_reduces_the_history_and_the_budget_prunes_oldest() {
        let mut archive = LogArchive::new(1024);
        fill(&mut archive, 5000);

        // Far more raw text than the compressed budget, and the newest chunk
        // is always retained.
        assert_eq!(archive.total(), 5000);
        assert!(archive.compressed_bytes() <= 16 * 1024);
        assert!(archive.first_seq() > 1, "the oldest chunks are pruned");
        let newest = archive.page_before(u64::MAX, 10);
        assert_eq!(newest.lines.len(), 10);
        assert_eq!(newest.lines[9].line, "line 005000");
    }

    #[test]
    fn empty_archive_pages_safely() {
        let archive = LogArchive::default();
        assert_eq!(archive.first_seq(), 0);
        assert!(archive.page_before(u64::MAX, 10).lines.is_empty());
        assert!(archive.page_from(1, 10).lines.is_empty());
        assert!(!archive.page_before(u64::MAX, 10).has_more);
    }

    #[test]
    fn append_all_is_oldest_first_and_keeps_every_retained_line() {
        let mut archive = LogArchive::default();
        fill(&mut archive, 300);
        let mut text = String::new();
        archive.append_all(&mut text);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 300);
        assert_eq!(lines[0], "line 000001");
        assert_eq!(lines[299], "line 000300");
    }
}
