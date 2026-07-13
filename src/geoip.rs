use std::{net::IpAddr, sync::OnceLock};

use maxminddb::Reader;
use tracing::info;

/// Embedded `GeoLite2-Country.mmdb` database.
///
/// To enable embedded `GeoIP`, place the database file at
/// `data/GeoLite2-Country.mmdb` before building. The file
/// is loaded from memory at runtime with zero external deps.
///
/// Get the database free from: <https://dev.maxmind.com/geoip/geolite2-free-geolocation-data>
const EMBEDDED_GEOIP: &[u8] = include_bytes!("../data/GeoLite2-Country.mmdb");

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

/// Look up the country ISO code for an IP address.
///
/// Returns the 2-letter ISO 3166-1 alpha-2 code (e.g., "US", "JP", "DE")
/// or `None` if the database is not loaded or the IP is not found.
pub fn lookup_country(ip: IpAddr) -> Option<String> {
    let reader = GEOIP_READER.get()?;
    let reader = reader.as_ref()?;
    let result = reader.lookup(ip).ok()?;
    let country = result.decode::<maxminddb::geoip2::Country>().ok()??;
    country.country.iso_code.map(ToString::to_string)
}

/// Reader that borrows either the embedded `&'static [u8]` or a file-backed `Vec<u8>`.
/// The enum avoids heap-copying the 8.9MB embedded database on startup.
enum GeoIpSource {
    Embedded(&'static [u8]),
    File(Vec<u8>),
}

impl AsRef<[u8]> for GeoIpSource {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Embedded(data) => data,
            Self::File(data) => data,
        }
    }
}

static GEOIP_READER: OnceLock<Option<Reader<GeoIpSource>>> = OnceLock::new();

/// Initialize the `GeoIP` database.
///
/// Tries the embedded database first (zero-config). If a custom path
/// is provided and the embedded database failed, falls back to file.
pub fn init(custom_path: Option<&std::path::Path>) {
    // Try embedded database first — zero-copy borrow of the static bytes
    match Reader::from_source(GeoIpSource::Embedded(EMBEDDED_GEOIP)) {
        Ok(reader) => {
            info!("GeoIP database loaded (embedded)");
            let _ = GEOIP_READER.set(Some(reader));
            return;
        }
        Err(err) => {
            tracing::warn!(error = %err, "embedded GeoIP database failed to load");
        }
    }

    // Fall back to file-based lookup
    if let Some(path) = custom_path
        && path.exists()
        && let Ok(bytes) = std::fs::read(path)
    {
        match Reader::from_source(GeoIpSource::File(bytes)) {
            Ok(reader) => {
                info!(geoip_path = %path.display(), "GeoIP database loaded (file)");
                let _ = GEOIP_READER.set(Some(reader));
                return;
            }
            Err(err) => {
                tracing::warn!(geoip_path = %path.display(), error = %err, "failed to load GeoIP from file");
            }
        }
    }

    tracing::info!("GeoIP disabled; country detection unavailable");
    let _ = GEOIP_READER.set(None);
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
/// - `country_code=Some("US"), remark="@ProxyChannel"` -> `"\u{1F1FA}\u{1F1F8} @ProxyChannel"`
/// - `country_code=Some("NL"), remark="\u{1F1EE}\u{1F1F9} | @WhiteDNS"` -> `"\u{1F1F3}\u{1F1F1} @WhiteDNS"`
/// - `country_code=None, remark="@WhiteDNS \u{1F1EE}\u{1F1F9}"` -> `"\u{1F1EE}\u{1F1F9} @WhiteDNS"`
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
/// proxy remarks (e.g. `"\u{1F1EE}\u{1F1F9} | @WhiteDNS"` -> `"@WhiteDNS"`).
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

    #[test]
    fn country_flag_us() {
        assert_eq!(country_flag("US"), "\u{1F1FA}\u{1F1F8}");
    }

    #[test]
    fn country_flag_jp() {
        assert_eq!(country_flag("JP"), "\u{1F1EF}\u{1F1F5}");
    }

    #[test]
    fn country_flag_de() {
        assert_eq!(country_flag("DE"), "\u{1F1E9}\u{1F1EA}");
    }

    #[test]
    fn country_flag_hk() {
        assert_eq!(country_flag("HK"), "\u{1F1ED}\u{1F1F0}");
    }

    #[test]
    fn country_flag_lowercase() {
        assert_eq!(country_flag("us"), "\u{1F1FA}\u{1F1F8}");
    }

    #[test]
    fn country_flag_invalid() {
        assert_eq!(country_flag(""), "");
        assert_eq!(country_flag("X"), "");
        assert_eq!(country_flag("12"), "");
    }

    #[test]
    fn strip_leading_flag_removes_emoji() {
        let input = "\u{1F1EE}\u{1F1F9} | @WhiteDNS";
        assert_eq!(strip_leading_flag(input), "@WhiteDNS");
    }

    #[test]
    fn strip_leading_flag_no_flag() {
        assert_eq!(strip_leading_flag("Plain remark"), "Plain remark");
    }

    #[test]
    fn strip_leading_flag_with_space_after() {
        let input = "\u{1F1FA}\u{1F1F8} some name";
        assert_eq!(strip_leading_flag(input), "some name");
    }

    #[test]
    fn format_replaces_existing_flag_with_geoip() {
        let existing = "\u{1F1EE}\u{1F1F9} | @WhiteDNS";
        let result = format_display_name(Some("US"), existing);
        assert_eq!(result, "\u{1F1FA}\u{1F1F8} @WhiteDNS");
    }

    #[test]
    fn format_no_existing_flag_adds_geoip() {
        let result = format_display_name(Some("US"), "@ProxyChannel");
        assert_eq!(result, "\u{1F1FA}\u{1F1F8} @ProxyChannel");
    }

    #[test]
    fn format_no_geoip_keeps_remark() {
        let result = format_display_name(None, "My Config");
        assert_eq!(result, "My Config");
    }

    #[test]
    fn format_no_geoip_keeps_existing_flag_at_start() {
        // Flag already at start — stays there
        let remark = "\u{1F1EE}\u{1F1F9} Original";
        let result = format_display_name(None, remark);
        assert_eq!(result, "\u{1F1EE}\u{1F1F9} Original");
    }

    #[test]
    fn format_no_geoip_moves_flag_from_end() {
        // Flag at end — moved to beginning
        let remark = "@WhiteDNS \u{1F1EE}\u{1F1F9}";
        let result = format_display_name(None, remark);
        assert_eq!(result, "\u{1F1EE}\u{1F1F9} @WhiteDNS");
    }

    #[test]
    fn format_no_geoip_moves_flag_from_middle() {
        // Flag in middle with pipes — extracted and moved
        let remark = "@WhiteDNS \u{1F1EE}\u{1F1F9} | extra";
        let result = format_display_name(None, remark);
        assert_eq!(result, "\u{1F1EE}\u{1F1F9} @WhiteDNS | extra");
    }

    #[test]
    fn format_no_geoip_no_flag_stays_as_is() {
        let result = format_display_name(None, "No flags here");
        assert_eq!(result, "No flags here");
    }

    #[test]
    fn format_geoip_replaces_existing_flag() {
        let existing = "\u{1F1EE}\u{1F1F9} | @WhiteDNS";
        let result = format_display_name(Some("US"), existing);
        assert_eq!(result, "\u{1F1FA}\u{1F1F8} @WhiteDNS");
    }

    #[test]
    fn format_geoip_adds_flag_to_plain_remark() {
        let result = format_display_name(Some("US"), "@ProxyChannel");
        assert_eq!(result, "\u{1F1FA}\u{1F1F8} @ProxyChannel");
    }

    #[test]
    fn format_no_geoip_plain_remark_unchanged() {
        let result = format_display_name(None, "My Config");
        assert_eq!(result, "My Config");
    }

    #[test]
    fn format_empty_remark_with_geoip() {
        let result = format_display_name(Some("US"), "");
        assert_eq!(result, "\u{1F1FA}\u{1F1F8}");
    }

    #[test]
    fn format_empty_country_code() {
        let result = format_display_name(Some(""), "@ProxyChannel");
        assert_eq!(result, "@ProxyChannel");
    }

    #[test]
    fn extract_any_flag_finds_trailing() {
        let result = extract_any_flag("name \u{1F1E9}\u{1F1EA}");
        assert!(result.is_some());
        let (flag, rest) = result.unwrap();
        assert_eq!(flag, "\u{1F1E9}\u{1F1EA}");
        assert_eq!(rest, "name");
    }

    #[test]
    fn extract_any_flag_finds_leading() {
        let result = extract_any_flag("\u{1F1FA}\u{1F1F8} name");
        assert!(result.is_some());
        let (flag, rest) = result.unwrap();
        assert_eq!(flag, "\u{1F1FA}\u{1F1F8}");
        assert_eq!(rest, "name");
    }

    #[test]
    fn extract_any_flag_none_when_no_emoji() {
        assert!(extract_any_flag("plain text").is_none());
    }

    #[test]
    fn extract_any_flag_strips_pipe_separator() {
        let result = extract_any_flag("name \u{1F1EE}\u{1F1F9} more");
        let (flag, rest) = result.unwrap();
        assert_eq!(flag, "\u{1F1EE}\u{1F1F9}");
        assert_eq!(rest, "name more");
    }
}
