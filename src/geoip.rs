use std::{collections::HashMap, net::IpAddr, path::Path, sync::OnceLock};

use anyhow::{Context, Result};
use tracing::{debug, info};

// Country IP database: per-country CIDR zone files (`<cc>.zone`, one
// `address/prefix` per line), refreshed independently of app releases.
//
// Source is ipdeny country blocks (free, keyless, redistribution allowed),
// IPv4 `all-zones.tar.gz` plus the IPv6 archive. The installer downloads
// these into `<data-root>/geoip/` (with an `ipv6/` subdir for the v6 files)
// and refreshes them whenever it runs. This module only reads them: it
// never downloads anything, keeping offline use working.
//
// This replaced the old embedded `GeoLite2-Country.mmdb`, which needed a
// MaxMind license key for updates, baked ~9MB of stale data into every
// release binary, and required the `maxminddb` dependency.

/// Maximum prefix length for IPv4 / IPv6 lookups.
const V4_BITS: u32 = 32;
const V6_BITS: u32 = 128;

/// A single parsed CIDR block tagged with a country pool index.
#[derive(Debug, Clone)]
struct V4Range {
    start: u32,
    end: u32,
    len: u8,
    country: u16,
}

/// A single parsed IPv6 CIDR block tagged with a country pool index.
#[derive(Debug, Clone)]
struct V6Range {
    start: u128,
    end: u128,
    len: u8,
    country: u16,
}

/// In-memory country database: sorted ranges plus an interned country pool
/// (fewer than 300 codes, so a `u16` index is plenty).
#[derive(Debug, Default)]
pub struct ZoneDb {
    v4: Vec<V4Range>,
    v6: Vec<V6Range>,
    countries: Vec<String>,
}

impl ZoneDb {
    /// Look up the 2-letter ISO country code for an IP address using
    /// longest-prefix-match (a more specific block always wins over a
    /// covering one, regardless of file order).
    pub fn lookup(&self, ip: IpAddr) -> Option<&str> {
        match ip {
            IpAddr::V4(addr) => {
                let key = u32::from(addr);
                lookup_v4(&self.v4, key).map(|index| self.countries[index as usize].as_str())
            }
            IpAddr::V6(addr) => {
                if let Some(mapped) = addr.to_ipv4_mapped() {
                    let key = u32::from(mapped);
                    if let Some(index) = lookup_v4(&self.v4, key) {
                        return Some(self.countries[index as usize].as_str());
                    }
                }
                let key = u128::from(addr);
                lookup_v6(&self.v6, key).map(|index| self.countries[index as usize].as_str())
            }
        }
    }

    /// Number of loaded prefixes (v4 + v6), for startup logging.
    pub const fn prefix_count(&self) -> usize {
        self.v4.len() + self.v6.len()
    }
}

/// Binary search on `start`, then walk left tracking the longest containing
/// block. The walk may stop at the first non-containing block whose prefix
/// is no longer than the best match: any block further left starts no later,
/// so a longer match there would have to start after this block's start
/// while still covering the key — impossible for well-formed CIDRs, and the
/// walk stays correct (just longer) even for adversarial input.
fn lookup_v4(ranges: &[V4Range], key: u32) -> Option<u16> {
    let mut index = ranges.partition_point(|range| range.start <= key);
    let mut best: Option<(u8, u16)> = None;
    while index > 0 {
        index -= 1;
        let range = &ranges[index];
        if range.end >= key {
            if best.is_none_or(|(best_len, _)| range.len > best_len) {
                best = Some((range.len, range.country));
            }
        } else if best.is_some_and(|(best_len, _)| range.len <= best_len) {
            break;
        }
    }
    best.map(|(_, country)| country)
}

/// IPv6 twin of [`lookup_v4`]; same walk with the same stop rule.
fn lookup_v6(ranges: &[V6Range], key: u128) -> Option<u16> {
    let mut index = ranges.partition_point(|range| range.start <= key);
    let mut best: Option<(u8, u16)> = None;
    while index > 0 {
        index -= 1;
        let range = &ranges[index];
        if range.end >= key {
            if best.is_none_or(|(best_len, _)| range.len > best_len) {
                best = Some((range.len, range.country));
            }
        } else if best.is_some_and(|(best_len, _)| range.len <= best_len) {
            break;
        }
    }
    best.map(|(_, country)| country)
}

/// Parse one `address/prefix` line into `(start, end, len)` as `u32`s.
/// Host bits are masked off so unnormalized lines still load.
fn parse_cidr_v4(line: &str) -> Option<(u32, u32, u8)> {
    let (addr, len) = line.split_once('/')?;
    let len: u32 = len.trim().parse().ok()?;
    if len > V4_BITS {
        return None;
    }
    let addr: std::net::Ipv4Addr = addr.trim().parse().ok()?;
    let bits = u32::from(addr);
    let mask = if len == 0 {
        0
    } else {
        u32::MAX << (V4_BITS - len)
    };
    let start = bits & mask;
    #[allow(clippy::cast_possible_truncation)]
    let len = len as u8;
    Some((start, start | !mask, len))
}

/// Parse one IPv6 `address/prefix` line into `(start, end, len)` as `u128`s.
fn parse_cidr_v6(line: &str) -> Option<(u128, u128, u8)> {
    let (addr, len) = line.split_once('/')?;
    let len: u32 = len.trim().parse().ok()?;
    if len > V6_BITS {
        return None;
    }
    let addr: std::net::Ipv6Addr = addr.trim().parse().ok()?;
    let bits = u128::from(addr);
    let mask = if len == 0 {
        0
    } else {
        u128::MAX << (V6_BITS - len)
    };
    let start = bits & mask;
    #[allow(clippy::cast_possible_truncation)]
    let len = len as u8;
    Some((start, start | !mask, len))
}

/// Load every `<cc>.zone` file in `dir` plus `dir/ipv6/*.zone` into a [`ZoneDb`].
///
/// The country code is the lowercase 2-letter file stem (`us.zone` → `US`).
/// Blank lines are skipped; malformed lines are skipped with a debug count so
/// one bad line can never take down the whole database. An empty directory
/// yields an empty (but valid) database.
pub fn load_dir(dir: &Path) -> Result<ZoneDb> {
    let mut db = ZoneDb::default();
    let mut pool: HashMap<String, u16> = HashMap::new();
    let mut skipped = ingest_dir(dir, false, &mut db, &mut pool)?;
    let v6_dir = dir.join("ipv6");
    if v6_dir.is_dir() {
        skipped += ingest_dir(&v6_dir, true, &mut db, &mut pool)?;
    }

    // Sort by start so the lookup walk works; longest-first on ties so the
    // best match is found (and the walk stops) as early as possible.
    db.v4.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| b.len.cmp(&a.len))
            .then_with(|| a.country.cmp(&b.country))
    });
    db.v6.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| b.len.cmp(&a.len))
            .then_with(|| a.country.cmp(&b.country))
    });
    if skipped > 0 {
        debug!(skipped, dir = %dir.display(), "skipped malformed zone lines");
    }
    Ok(db)
}

/// Intern a country code into the pool, returning its index.
fn intern_country(db: &mut ZoneDb, pool: &mut HashMap<String, u16>, code: &str) -> u16 {
    if let Some(&index) = pool.get(code) {
        return index;
    }
    #[allow(clippy::cast_possible_truncation)]
    let index = db.countries.len() as u16;
    db.countries.push(code.to_string());
    pool.insert(code.to_string(), index);
    index
}

/// Ingest every `<cc>.zone` file in one directory into the v4 (or v6) table.
/// Returns the count of skipped malformed lines.
fn ingest_dir(
    dir: &Path,
    v6: bool,
    db: &mut ZoneDb,
    pool: &mut HashMap<String, u16>,
) -> Result<usize> {
    let mut skipped = 0usize;
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("unable to read GeoIP directory {}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "zone") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if stem.len() != 2 || !stem.bytes().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let country = intern_country(db, pool, &stem.to_ascii_uppercase());
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("unable to read {}", path.display()))?;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if v6 {
                if let Some((start, end, len)) = parse_cidr_v6(line) {
                    db.v6.push(V6Range {
                        start,
                        end,
                        len,
                        country,
                    });
                } else {
                    skipped += 1;
                }
            } else if let Some((start, end, len)) = parse_cidr_v4(line) {
                db.v4.push(V4Range {
                    start,
                    end,
                    len,
                    country,
                });
            } else {
                skipped += 1;
            }
        }
    }
    Ok(skipped)
}

static GEOIP_DB: OnceLock<Option<ZoneDb>> = OnceLock::new();

/// Look up the country ISO code for an IP address.
///
/// Returns the 2-letter ISO 3166-1 alpha-2 code (e.g., "US", "JP", "DE")
/// or `None` if the database is not loaded or the IP is not found.
pub fn lookup_country(ip: IpAddr) -> Option<String> {
    GEOIP_DB
        .get()
        .and_then(|db| db.as_ref())
        .and_then(|db| db.lookup(ip))
        .map(ToString::to_string)
}

/// Initialize the `GeoIP` database from a zone directory (see [`load_dir`]).
///
/// A missing directory, an empty one, or an unreadable one disables country
/// detection with an informational log — the app keeps working without flags.
pub fn init(dir: Option<&Path>) {
    let Some(dir) = dir else {
        info!("GeoIP disabled; country detection unavailable");
        let _ = GEOIP_DB.set(None);
        return;
    };
    match load_dir(dir) {
        Ok(db) => {
            info!(
                prefixes = db.prefix_count(),
                dir = %dir.display(),
                "GeoIP database loaded (country zones)"
            );
            let _ = GEOIP_DB.set(Some(db));
        }
        Err(err) => {
            tracing::warn!(dir = %dir.display(), error = %err, "failed to load GeoIP zones");
            let _ = GEOIP_DB.set(None);
        }
    }
}

/// Country code to flag emoji conversion.
///
/// Uses Unicode Regional Indicator symbols: each ASCII letter maps to
/// U+1F1E6 + (letter - 'A'). For example, "US" -> flag US, "JP" -> flag JP.
///
/// Requires a terminal that supports color emoji rendering (Windows Terminal,
/// VS Code, Alacritty, `WezTerm`, etc.). Legacy conhost does not support these.
pub fn country_flag(code: &str) -> String {
    let bytes = code.as_bytes();
    if bytes.len() != 2 || !bytes[0].is_ascii_alphabetic() || !bytes[1].is_ascii_alphabetic() {
        return String::new();
    }
    let first = 0x1F1E6 + u32::from(bytes[0].to_ascii_uppercase() - b'A');
    let second = 0x1F1E6 + u32::from(bytes[1].to_ascii_uppercase() - b'A');
    char::from_u32(first)
        .zip(char::from_u32(second))
        .map(|(a, b)| format!("{a}{b}"))
        .unwrap_or_default()
}

/// Format a display name for a config.
///
/// Rules:
/// 1. `GeoIP` flag (if available) always goes at the beginning.
/// 2. If the remark already has a flag emoji, it is removed from its current
///    position to avoid duplicates.
/// 3. If no `GeoIP` flag is available but the remark contains one, that flag
///    is moved to the beginning.
/// 4. If neither has a flag, the remark is returned as-is.
///
/// Examples:
/// - `country_code=Some("US"), remark="@ProxyChannel"` -> `"🇺🇸 @ProxyChannel"`
/// - `country_code=Some("NL"), remark="🇳🇱 | @WhiteDNS"` -> `"🇳🇱 @WhiteDNS"`
/// - `country_code=None, remark="@WhiteDNS 🇳🇱"` -> `"🇳🇱 @WhiteDNS"`
pub fn format_display_name(country_code: Option<&str>, remark: &str) -> String {
    let geoip_flag = country_code.and_then(|code| {
        let f = country_code_flag(code);
        if f.is_empty() { None } else { Some(f) }
    });

    if let Some(flag) = geoip_flag {
        let stripped = strip_leading_flag(remark);
        if stripped.is_empty() {
            return flag;
        }
        return format!("{flag} {stripped}");
    }

    // No GeoIP flag — check if remark has one and move it to the front
    if let Some((existing_flag, rest)) = extract_any_flag(remark) {
        let rest = rest.trim();
        if rest.is_empty() {
            return existing_flag;
        }
        return format!("{existing_flag} {rest}");
    }

    remark.to_string()
}

/// Extract a Regional Indicator flag emoji from anywhere in a string.
///
/// Returns `(flag, remainder)` where `flag` is the two-char emoji and
/// `remainder` is everything else (with leading/trailing separators cleaned).
/// Returns `None` if no flag is found.
fn extract_any_flag(text: &str) -> Option<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return None;
    }
    for i in 0..chars.len() - 1 {
        let a = chars[i] as u32;
        let b = chars[i + 1] as u32;
        if (0x1F1E6..=0x1F1FF).contains(&a) && (0x1F1E6..=0x1F1FF).contains(&b) {
            let flag = format!("{}{}", chars[i], chars[i + 1]);
            let before: String = chars[..i].iter().collect();
            let after: String = chars[i + 2..].iter().collect();
            let combined = format!("{before}{after}");
            // Normalize: trim, strip leading pipe+space, collapse whitespace
            let combined = combined.trim();
            let combined = combined.trim_start_matches(['|', ' ']).trim();
            let combined: String = combined.split_whitespace().collect::<Vec<_>>().join(" ");
            return Some((flag, combined));
        }
    }
    None
}

/// Return the two-character Regional Indicator flag for a 2-letter ISO code.
fn country_code_flag(code: &str) -> String {
    country_flag(code)
}

/// Strip a leading Regional Indicator flag emoji (two chars) from a string.
///
/// Also strips a single trailing space or pipe+space that often follows flags in
/// proxy remarks (e.g. `"🇳🇱 | @WhiteDNS"` -> `"@WhiteDNS"`).
fn strip_leading_flag(remark: &str) -> String {
    let chars: Vec<char> = remark.chars().collect();
    if chars.len() >= 2 {
        let first = chars[0] as u32;
        let second = chars[1] as u32;
        if (0x1F1E6..=0x1F1FF).contains(&first) && (0x1F1E6..=0x1F1FF).contains(&second) {
            let rest: String = chars[2..].iter().collect();
            let rest = rest.trim_start();
            // Strip leading pipe that often follows flags: "🇳🇱 | @WhiteDNS" -> "@WhiteDNS"
            if let Some(stripped) = rest.strip_prefix('|') {
                return stripped.trim().to_string();
            }
            return rest.to_string();
        }
    }
    remark.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn v4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    #[test]
    fn parse_v4_basic_and_masking() {
        assert_eq!(
            parse_cidr_v4("1.2.3.0/24"),
            Some((0x0102_0300, 0x0102_03FF, 24))
        );
        // Host bits are masked off, not rejected.
        assert_eq!(
            parse_cidr_v4("1.2.3.4/24"),
            Some((0x0102_0300, 0x0102_03FF, 24))
        );
        assert_eq!(parse_cidr_v4("0.0.0.0/0"), Some((0, u32::MAX, 0)));
        assert_eq!(
            parse_cidr_v4("8.8.8.8/32"),
            Some((0x0808_0808, 0x0808_0808, 32))
        );
    }

    #[test]
    fn parse_v4_rejects_garbage() {
        assert_eq!(parse_cidr_v4(""), None);
        assert_eq!(parse_cidr_v4("1.2.3.0"), None);
        assert_eq!(parse_cidr_v4("1.2.3.0/33"), None);
        assert_eq!(parse_cidr_v4("1.2.3.0/-1"), None);
        assert_eq!(parse_cidr_v4("999.1.1.0/24"), None);
        assert_eq!(parse_cidr_v4("::1/128"), None);
    }

    #[test]
    fn parse_v6_basic() {
        let (start, end, len) = parse_cidr_v6("2001:db8::/32").expect("parses");
        assert_eq!(len, 32);
        assert_eq!(
            start,
            u128::from(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0))
        );
        assert_eq!(
            end,
            u128::from(Ipv6Addr::new(
                0x2001, 0xdb8, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff
            ))
        );
        assert_eq!(parse_cidr_v6("::/0"), Some((0, u128::MAX, 0)));
        assert_eq!(parse_cidr_v6("::1/129"), None);
    }

    /// Build a sorted table the same way [`load_dir`] does (start asc,
    /// longest first on ties).
    fn sorted_v4(mut ranges: Vec<V4Range>) -> Vec<V4Range> {
        ranges.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then_with(|| b.len.cmp(&a.len))
                .then_with(|| a.country.cmp(&b.country))
        });
        ranges
    }

    fn range(start: [u8; 4], len: u8, country: u16) -> V4Range {
        let (s, e, l) = parse_cidr_v4(&format!(
            "{}.{}.{}.{}/{}",
            start[0], start[1], start[2], start[3], len
        ))
        .expect("fixture parses");
        assert_eq!(l, len);
        V4Range {
            start: s,
            end: e,
            len,
            country,
        }
    }

    #[test]
    fn lookup_longest_prefix_wins_over_covering_block() {
        let ranges = sorted_v4(vec![
            range([10, 0, 0, 0], 8, 0),  // XX
            range([10, 1, 0, 0], 16, 1), // YY
            range([192, 168, 0, 0], 16, 2),
        ]);
        assert_eq!(
            lookup_v4(&ranges, u32::from(Ipv4Addr::new(10, 1, 2, 3))),
            Some(1)
        );
        assert_eq!(
            lookup_v4(&ranges, u32::from(Ipv4Addr::new(10, 2, 0, 1))),
            Some(0)
        );
        assert_eq!(
            lookup_v4(&ranges, u32::from(Ipv4Addr::new(192, 168, 5, 5))),
            Some(2)
        );
        assert_eq!(
            lookup_v4(&ranges, u32::from(Ipv4Addr::new(8, 8, 8, 8))),
            None
        );
    }

    #[test]
    fn lookup_skips_non_containing_blocks_between_matches() {
        // A non-containing /24 sits between two containing blocks by start
        // order; the outer /8 must still be found (and lose to nothing).
        let ranges = sorted_v4(vec![
            range([10, 0, 0, 0], 8, 0),
            range([10, 5, 0, 0], 24, 1), // ends before the key
            range([11, 0, 0, 0], 8, 2),
        ]);
        assert_eq!(
            lookup_v4(&ranges, u32::from(Ipv4Addr::new(10, 9, 0, 1))),
            Some(0)
        );
    }

    /// Deterministic xorshift for the differential test (no extra deps).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    /// Best (longest) prefix length covering `key`, if any. Ties are
    /// intentionally ignored: overlapping same-length blocks from different
    /// countries have no canonical winner, so the test below accepts any
    /// optimal answer instead of pinning a tie-break.
    fn brute_best_len(ranges: &[V4Range], key: u32) -> Option<u8> {
        let mut best: Option<u8> = None;
        for range in ranges {
            if range.start <= key && key <= range.end && best.is_none_or(|len| range.len > len) {
                best = Some(range.len);
            }
        }
        best
    }

    #[test]
    fn lookup_matches_brute_force_on_random_overlapping_tables() {
        // Small address space forces heavy overlap, including duplicates and
        // partial overlaps the real data rarely has.
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut ranges = Vec::new();
        for i in 0..2000u16 {
            let len = 8 + (rng.next() % 25) as u8; // /8..=/32
            let raw = (rng.next() & 0xFFFF) as u32; // 0.0.0.0/16 space
            let mask = if len >= 16 {
                u32::MAX << (32 - len)
            } else {
                0xFFFF_0000u32 & (u32::MAX << (32 - len))
            };
            let start = raw & mask;
            let end = start | !mask;
            ranges.push(V4Range {
                start,
                end,
                len,
                country: i % 7,
            });
        }
        let sorted = sorted_v4(ranges.clone());
        for _ in 0..5000 {
            let key = (rng.next() & 0xFFFF_FFFF) as u32;
            match brute_best_len(&ranges, key) {
                None => assert_eq!(lookup_v4(&sorted, key), None, "key {key}"),
                Some(len) => {
                    let country = lookup_v4(&sorted, key).expect("lookup finds optimal");
                    assert!(
                        sorted.iter().any(|range| range.start <= key
                            && key <= range.end
                            && range.len == len
                            && range.country == country),
                        "non-optimal answer for key {key}"
                    );
                }
            }
        }
    }

    #[test]
    fn load_dir_reads_v4_and_v6_zones() {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-geoip-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("ipv6")).expect("temp dirs");
        std::fs::write(
            dir.join("us.zone"),
            "1.0.0.0/24\n2.0.0.0/16\n\n# comment\nbogus\n",
        )
        .expect("write");
        std::fs::write(dir.join("nl.zone"), "3.0.0.0/24\n").expect("write");
        std::fs::write(dir.join("notes.txt"), "ignored").expect("write");
        std::fs::write(dir.join("ipv6").join("de.zone"), "2001:db8::/32\n").expect("write");

        let db = load_dir(&dir).expect("loads");
        assert_eq!(db.lookup(v4([1, 0, 0, 7])), Some("US"));
        assert_eq!(db.lookup(v4([2, 0, 9, 9])), Some("US"));
        assert_eq!(db.lookup(v4([3, 0, 0, 1])), Some("NL"));
        assert_eq!(db.lookup(v4([9, 9, 9, 9])), None);
        assert_eq!(
            db.lookup(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))),
            Some("DE")
        );
        // IPv4-mapped IPv6 falls back to the v4 table.
        assert_eq!(
            db.lookup(IpAddr::V6(Ipv6Addr::new(
                0, 0, 0, 0, 0, 0xffff, 0x0100, 0x0007
            ))),
            Some("US")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_dir_missing_is_an_error_for_init_to_handle() {
        assert!(load_dir(Path::new("/nonexistent-geoip-dir-xyz")).is_err());
        let empty = std::env::temp_dir().join(format!(
            "v2raydar-geoip-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&empty).expect("temp dir");
        let db = load_dir(&empty).expect("empty dir loads");
        assert_eq!(db.prefix_count(), 0);
        assert_eq!(db.lookup(v4([1, 1, 1, 1])), None);
        std::fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn country_flag_us() {
        assert_eq!(country_flag("US"), "🇺🇸");
    }

    #[test]
    fn country_flag_jp() {
        assert_eq!(country_flag("JP"), "🇯🇵");
    }

    #[test]
    fn country_flag_de() {
        assert_eq!(country_flag("DE"), "🇩🇪");
    }

    #[test]
    fn country_flag_hk() {
        assert_eq!(country_flag("HK"), "🇭🇰");
    }

    #[test]
    fn country_flag_lowercase() {
        assert_eq!(country_flag("us"), "🇺🇸");
    }

    #[test]
    fn country_flag_invalid() {
        assert_eq!(country_flag(""), "");
        assert_eq!(country_flag("X"), "");
        assert_eq!(country_flag("12"), "");
    }

    #[test]
    fn strip_leading_flag_removes_emoji() {
        let input = "🇳🇱 | @WhiteDNS";
        assert_eq!(strip_leading_flag(input), "@WhiteDNS");
    }

    #[test]
    fn strip_leading_flag_no_flag() {
        assert_eq!(strip_leading_flag("Plain remark"), "Plain remark");
    }

    #[test]
    fn strip_leading_flag_with_space_after() {
        let input = "🇺🇸 some name";
        assert_eq!(strip_leading_flag(input), "some name");
    }

    #[test]
    fn format_replaces_existing_flag_with_geoip() {
        let existing = "🇳🇱 | @WhiteDNS";
        let result = format_display_name(Some("US"), existing);
        assert_eq!(result, "🇺🇸 @WhiteDNS");
    }

    #[test]
    fn format_no_existing_flag_adds_geoip() {
        let result = format_display_name(Some("US"), "@ProxyChannel");
        assert_eq!(result, "🇺🇸 @ProxyChannel");
    }

    #[test]
    fn format_no_geoip_keeps_remark() {
        let result = format_display_name(None, "My Config");
        assert_eq!(result, "My Config");
    }

    #[test]
    fn format_no_geoip_keeps_existing_flag_at_start() {
        // Flag already at start — stays there
        let remark = "🇳🇱 Original";
        let result = format_display_name(None, remark);
        assert_eq!(result, "🇳🇱 Original");
    }

    #[test]
    fn format_no_geoip_moves_flag_from_end() {
        // Flag at end — moved to beginning
        let remark = "@WhiteDNS 🇳🇱";
        let result = format_display_name(None, remark);
        assert_eq!(result, "🇳🇱 @WhiteDNS");
    }

    #[test]
    fn format_no_geoip_moves_flag_from_middle() {
        // Flag in middle with pipes — extracted and moved
        let remark = "@WhiteDNS 🇳🇱 | extra";
        let result = format_display_name(None, remark);
        assert_eq!(result, "🇳🇱 @WhiteDNS | extra");
    }

    #[test]
    fn format_no_geoip_no_flag_stays_as_is() {
        let result = format_display_name(None, "No flags here");
        assert_eq!(result, "No flags here");
    }

    #[test]
    fn format_geoip_replaces_existing_flag() {
        let existing = "🇳🇱 | @WhiteDNS";
        let result = format_display_name(Some("US"), existing);
        assert_eq!(result, "🇺🇸 @WhiteDNS");
    }

    #[test]
    fn format_geoip_adds_flag_to_plain_remark() {
        let result = format_display_name(Some("US"), "@ProxyChannel");
        assert_eq!(result, "🇺🇸 @ProxyChannel");
    }

    #[test]
    fn format_no_geoip_plain_remark_unchanged() {
        let result = format_display_name(None, "My Config");
        assert_eq!(result, "My Config");
    }

    #[test]
    fn format_empty_remark_with_geoip() {
        let result = format_display_name(Some("US"), "");
        assert_eq!(result, "🇺🇸");
    }

    #[test]
    fn format_empty_country_code() {
        let result = format_display_name(Some(""), "@ProxyChannel");
        assert_eq!(result, "@ProxyChannel");
    }

    #[test]
    fn extract_any_flag_finds_trailing() {
        let result = extract_any_flag("name 🇩🇪");
        assert!(result.is_some());
        let (flag, rest) = result.unwrap();
        assert_eq!(flag, "🇩🇪");
        assert_eq!(rest, "name");
    }

    #[test]
    fn extract_any_flag_finds_leading() {
        let result = extract_any_flag("🇺🇸 name");
        assert!(result.is_some());
        let (flag, rest) = result.unwrap();
        assert_eq!(flag, "🇺🇸");
        assert_eq!(rest, "name");
    }

    #[test]
    fn extract_any_flag_none_when_no_emoji() {
        assert!(extract_any_flag("plain text").is_none());
    }

    #[test]
    fn extract_any_flag_strips_pipe_separator() {
        let result = extract_any_flag("name 🇳🇱 more");
        let (flag, rest) = result.unwrap();
        assert_eq!(flag, "🇳🇱");
        assert_eq!(rest, "name more");
    }
}
