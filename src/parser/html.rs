use std::collections::HashSet;

use crate::{constants::SUPPORTED_URI_SCHEMES, convert::decode_base64_to_string, model::Candidate};

use super::parse_share_link;

/// Try to extract proxy configs from an HTML subscription panel.
///
/// Handles Marzban/Xray-style panels with configs in attributes, event
/// handlers, base64-encoded blocks, and embedded script content.
///
/// Returns `None` if the body doesn't look like HTML or yields no candidates.
pub fn try_extract_from_html(source: &str, priority: u32, body: &[u8]) -> Option<Vec<Candidate>> {
    if !is_likely_html(body) {
        return None;
    }

    let text = String::from_utf8_lossy(body);
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    // 1. Extract URIs from HTML attributes (value="", onclick="", data-link="", etc.)
    extract_uris_from_attributes(&text, &mut candidates, &mut seen);

    // 2. Extract base64-encoded URI lists from <textarea>/<code>/<pre> tags
    extract_base64_from_tags(source, priority, &text, &mut candidates, &mut seen);

    // 3. Extract URIs from <script> blocks (JSON arrays, inline config)
    extract_from_scripts(source, priority, &text, &mut candidates, &mut seen);

    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

/// Quick heuristic: scan first 2048 bytes for HTML tag signatures.
fn is_likely_html(body: &[u8]) -> bool {
    let len = body.len().min(2048);
    // Safe: we're only checking for ASCII tag names
    let head = String::from_utf8_lossy(&body[..len]).to_ascii_lowercase();
    head.contains("<html")
        || head.contains("<!doctype")
        || head.contains("<input")
        || head.contains("<div")
        || head.contains("<body")
        || head.contains("<textarea")
        || head.contains("<script")
        || head.contains("<table")
}

/// Extract URIs from HTML attribute contexts.
///
/// Finds supported URI scheme prefixes and expands backward/forward to the
/// enclosing attribute delimiters (`"` or `'`). This cleanly handles:
/// - `<input value="vless://...">`
/// - `onclick="copyLink('vmess://...')"`
/// - `data-link="trojan://..."`
fn extract_uris_from_attributes(
    text: &str,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let lower = text.to_ascii_lowercase();

    for scheme in SUPPORTED_URI_SCHEMES {
        let mut offset = 0;
        while offset < lower.len() {
            let Some(pos) = lower[offset..].find(scheme) else {
                break;
            };
            let abs_pos = offset + pos;

            // Scan backward for opening delimiter
            let open_delim = text[..abs_pos]
                .chars()
                .rev()
                .find(|&ch| ch == '"' || ch == '\'');

            let Some(delim) = open_delim else {
                offset = abs_pos + scheme.len();
                continue;
            };

            // Scan forward from scheme start for matching closing delimiter
            let search_from = abs_pos + scheme.len();
            let close_rel = text[search_from..].find(delim);

            let Some(close_offset) = close_rel else {
                offset = search_from;
                continue;
            };

            let candidate = text[abs_pos..search_from + close_offset].trim();

            if !candidate.is_empty()
                && seen.insert(candidate.to_string())
                && let Ok(parsed) = parse_share_link("", 0, candidate)
            {
                candidates.push(parsed);
            }

            offset = search_from + close_offset + 1;
        }
    }
}

/// Extract base64-encoded URI lists from `<textarea>`, `<code>`, `<pre>` tags.
fn extract_base64_from_tags(
    source: &str,
    priority: u32,
    text: &str,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let lower = text.to_ascii_lowercase();

    for tag in ["textarea", "code", "pre"] {
        let open_tag = format!("<{tag}");
        let close_tag = format!("</{tag}>");
        let close_tag_lower = close_tag.to_ascii_lowercase();

        let mut offset = 0;
        while offset < lower.len() {
            let Some(open_pos) = lower[offset..].find(&open_tag) else {
                break;
            };

            // Find end of opening tag (the `>`)
            let tag_search_start = offset + open_pos;
            let Some(tag_end_rel) = text[tag_search_start..].find('>') else {
                break;
            };
            let content_start = tag_search_start + tag_end_rel + 1;

            // Find closing tag
            let Some(close_rel) = lower[content_start..].find(&close_tag_lower) else {
                break;
            };
            let content = text[content_start..content_start + close_rel].trim();

            if !content.is_empty() {
                // Try base64 decode
                if let Some(decoded) = decode_base64_to_string(content) {
                    // Scan decoded text for URI schemes
                    for token in decoded.split(|ch: char| {
                        ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']')
                    }) {
                        let entry = token.trim().trim_matches(['"', '\'', ',', ';']);
                        if SUPPORTED_URI_SCHEMES
                            .iter()
                            .any(|s| entry.to_ascii_lowercase().starts_with(s))
                            && seen.insert(entry.to_string())
                            && let Ok(candidate) = parse_share_link(source, priority, entry)
                        {
                            candidates.push(candidate);
                        }
                    }
                }

                // Also try the raw content as a text list (not base64)
                for token in content.split(|ch: char| {
                    ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']')
                }) {
                    let entry = token.trim().trim_matches(['"', '\'', ',', ';']);
                    if SUPPORTED_URI_SCHEMES
                        .iter()
                        .any(|s| entry.to_ascii_lowercase().starts_with(s))
                        && seen.insert(entry.to_string())
                        && let Ok(candidate) = parse_share_link(source, priority, entry)
                    {
                        candidates.push(candidate);
                    }
                }
            }

            offset = content_start + close_rel + close_tag.len();
        }
    }
}

/// Extract URIs from `<script>` blocks.
///
/// Handles inline JSON arrays (e.g., `var links = ["vless://..."]`) and
/// general string content containing URI schemes.
fn extract_from_scripts(
    source: &str,
    priority: u32,
    text: &str,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let lower = text.to_ascii_lowercase();
    let script_open = "<script";
    let script_close = "</script>";

    let mut offset = 0;
    while offset < lower.len() {
        let Some(open_pos) = lower[offset..].find(script_open) else {
            break;
        };

        let tag_search_start = offset + open_pos;
        let Some(tag_end_rel) = text[tag_search_start..].find('>') else {
            break;
        };
        let content_start = tag_search_start + tag_end_rel + 1;

        let Some(close_rel) = lower[content_start..].find(script_close) else {
            break;
        };
        let script_content = text[content_start..content_start + close_rel].trim();

        if !script_content.is_empty() {
            // Try to parse the script content as JSON
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(script_content) {
                collect_json_uris(source, priority, &json, candidates, seen);
            }

            // Also scan as plain text for URI schemes
            for token in script_content
                .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']'))
            {
                let entry = token.trim().trim_matches(['"', '\'', ',', ';']);
                if SUPPORTED_URI_SCHEMES
                    .iter()
                    .any(|s| entry.to_ascii_lowercase().starts_with(s))
                    && seen.insert(entry.to_string())
                    && let Ok(candidate) = parse_share_link(source, priority, entry)
                {
                    candidates.push(candidate);
                }
            }
        }

        offset = content_start + close_rel + script_close.len();
    }
}

/// Recursively walk a JSON value and collect URI strings from string leaves.
fn collect_json_uris(
    source: &str,
    priority: u32,
    value: &serde_json::Value,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::String(text) => {
            for token in text
                .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']'))
            {
                let entry = token.trim().trim_matches(['"', '\'', ',', ';']);
                if SUPPORTED_URI_SCHEMES
                    .iter()
                    .any(|s| entry.to_ascii_lowercase().starts_with(s))
                    && seen.insert(entry.to_string())
                    && let Ok(candidate) = parse_share_link(source, priority, entry)
                {
                    candidates.push(candidate);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_json_uris(source, priority, item, candidates, seen);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_json_uris(source, priority, item, candidates, seen);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_uri_from_input_value_attribute() {
        let html = r#"<!DOCTYPE html>
<html><body>
<input type="text" value="vless://abc123@example.com:443?security=tls#TestNode">
</body></html>"#;

        let result = try_extract_from_html("test", 100, html.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert!(!candidates.is_empty());
        assert!(candidates.iter().any(|c| c.protocol == "vless"));
        assert!(candidates.iter().any(|c| c.name == "TestNode"));
    }

    #[test]
    fn extracts_uri_from_onclick_handler() {
        let html = r#"<html><body>
<a onclick="copyLink('trojan://mypassword@server.example.com:443#OneclickNode')">Copy</a>
</body></html>"#;

        let result = try_extract_from_html("test", 100, html.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert!(candidates.iter().any(|c| c.protocol == "trojan"));
    }

    #[test]
    fn extracts_uri_from_data_link_attribute() {
        let html = r#"<html><body>
<button data-link="trojan://password@host.example.com:443#TrojanNode">Copy</button>
</body></html>"#;

        let result = try_extract_from_html("test", 100, html.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "trojan");
    }

    #[test]
    fn extracts_multiple_uris_from_html() {
        let html = r#"<!DOCTYPE html>
<html><body>
<input value="vless://uuid1@host1:443?security=tls#Node1">
<input value="vmess://eyJ2IjoiMiJ9#Node2">
<button data-link="ss://YWVzLXBhc3N3b3Jk@host3:8388#Node3">Copy</button>
</body></html>"#;

        let result = try_extract_from_html("test", 100, html.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert!(candidates.len() >= 2);
    }

    #[test]
    fn returns_none_for_plain_text() {
        let text = "vless://uuid@host:443#Node1\nvmess://base64payload#Node2";
        let result = try_extract_from_html("test", 100, text.as_bytes());
        assert!(result.is_none());
    }

    #[test]
    fn returns_none_for_non_html() {
        let text = "this is just some random text without html tags";
        let result = try_extract_from_html("test", 100, text.as_bytes());
        assert!(result.is_none());
    }

    #[test]
    fn handles_mixed_html_content() {
        let html = r#"<!DOCTYPE html>
<html>
<head><title>Sub Panel</title></head>
<body>
<h1>Subscription</h1>
<div class="links">
    <input value="vless://uuid@server1.com:443?security=reality#Reality Node" readonly>
    <input value="vless://uuid@server2.com:443?security=tls&type=ws#WS Node" readonly>
</div>
<script>
var links = ["vless://uuid@server3.com:8443#Script Node"];
</script>
</body>
</html>"#;

        let result = try_extract_from_html("test", 100, html.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        // Should find at least the 2 input values and possibly the script one
        assert!(candidates.len() >= 2);
    }
}
