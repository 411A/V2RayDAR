use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use anyhow::{Result, anyhow};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use percent_encoding::percent_decode_str;
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use url::Url;

use crate::{
    constants::SUPPORTED_URI_SCHEMES,
    model::{Candidate, Endpoint},
};

pub fn parse_subscription_document(source: &str, priority: u32, body: &[u8]) -> Vec<Candidate> {
    let text = String::from_utf8_lossy(body);
    let mut candidates = Vec::new();
    let mut seen_entries = HashSet::new();

    collect_entries_from_text(source, priority, &text, &mut candidates, &mut seen_entries);

    let compact = text.trim();
    if let Some(decoded) = decode_base64_to_string(compact) {
        collect_entries_from_text(
            source,
            priority,
            &decoded,
            &mut candidates,
            &mut seen_entries,
        );
    }

    if let Ok(json) = serde_json::from_str::<JsonValue>(&text) {
        collect_entries_from_json(source, priority, &json, &mut candidates, &mut seen_entries);
    }

    if let Ok(yaml) = serde_yaml::from_str::<YamlValue>(&text) {
        collect_entries_from_yaml(source, priority, &yaml, &mut candidates, &mut seen_entries);
    }

    candidates
}

fn collect_entries_from_text(
    source: &str,
    priority: u32,
    text: &str,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for token in text.split(is_token_boundary) {
        let entry = token.trim().trim_matches(['"', '\'', ',', ';']);
        if SUPPORTED_URI_SCHEMES
            .iter()
            .any(|scheme| entry.to_ascii_lowercase().starts_with(scheme))
            && seen.insert(entry.to_string())
            && let Ok(candidate) = parse_share_link(source, priority, entry)
        {
            candidates.push(candidate);
        }
    }
}

fn collect_entries_from_json(
    source: &str,
    priority: u32,
    value: &JsonValue,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    match value {
        JsonValue::String(text) => {
            collect_entries_from_text(source, priority, text, candidates, seen)
        }
        JsonValue::Array(values) => {
            for item in values {
                collect_entries_from_json(source, priority, item, candidates, seen);
            }
        }
        JsonValue::Object(map) => {
            for item in map.values() {
                collect_entries_from_json(source, priority, item, candidates, seen);
            }
        }
        _ => {}
    }
}

fn collect_entries_from_yaml(
    source: &str,
    priority: u32,
    value: &YamlValue,
    candidates: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    match value {
        YamlValue::String(text) => {
            collect_entries_from_text(source, priority, text, candidates, seen)
        }
        YamlValue::Sequence(values) => {
            for item in values {
                collect_entries_from_yaml(source, priority, item, candidates, seen);
            }
        }
        YamlValue::Mapping(map) => {
            for item in map.values() {
                collect_entries_from_yaml(source, priority, item, candidates, seen);
            }
        }
        _ => {}
    }
}

fn is_token_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']')
}

fn parse_share_link(source: &str, priority: u32, uri: &str) -> Result<Candidate> {
    let lower = uri.to_ascii_lowercase();
    let parsed = if lower.starts_with("vmess://") {
        parse_vmess(uri)
    } else if lower.starts_with("ss://") {
        parse_shadowsocks(uri)
    } else if lower.starts_with("ssr://") {
        parse_shadowsocksr(uri)
    } else {
        parse_standard_uri(uri)
    }?;

    let id = hash_uri(uri);
    Ok(Candidate {
        id,
        source: source.to_string(),
        priority,
        protocol: parsed.protocol,
        name: parsed.name,
        endpoint: parsed.endpoint,
        uri: uri.to_string(),
    })
}

struct ParsedLink {
    protocol: String,
    name: String,
    endpoint: Endpoint,
}

fn parse_vmess(uri: &str) -> Result<ParsedLink> {
    let payload = uri
        .strip_prefix("vmess://")
        .ok_or_else(|| anyhow!("invalid vmess link"))?;

    if let Some(decoded) = decode_base64_to_string(payload)
        && let Ok(json) = serde_json::from_str::<JsonValue>(&decoded)
    {
        let host = json
            .get("add")
            .or_else(|| json.get("address"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| anyhow!("vmess link has no address"))?;
        let port = json
            .get("port")
            .and_then(json_value_to_u16)
            .ok_or_else(|| anyhow!("vmess link has no port"))?;
        let name = json
            .get("ps")
            .and_then(JsonValue::as_str)
            .map(clean_name)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("{host}:{port}"));

        return Ok(ParsedLink {
            protocol: "vmess".to_string(),
            name,
            endpoint: Endpoint {
                host: host.to_string(),
                port,
            },
        });
    }

    parse_standard_uri(uri).map(|mut parsed| {
        parsed.protocol = "vmess".to_string();
        parsed
    })
}

fn parse_standard_uri(uri: &str) -> Result<ParsedLink> {
    let url = Url::parse(uri).map_err(|err| anyhow!("invalid uri: {err}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("uri has no host"))?
        .to_string();
    let port = url.port().ok_or_else(|| anyhow!("uri has no port"))?;
    let protocol = match url.scheme() {
        "hy2" => "hysteria2",
        other => other,
    }
    .to_string();
    let name = url
        .fragment()
        .map(clean_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{host}:{port}"));

    Ok(ParsedLink {
        protocol,
        name,
        endpoint: Endpoint { host, port },
    })
}

fn parse_shadowsocks(uri: &str) -> Result<ParsedLink> {
    let body = uri
        .strip_prefix("ss://")
        .ok_or_else(|| anyhow!("invalid shadowsocks link"))?;
    let (without_fragment, fragment) = split_once(body, '#');
    let (authority_part, _) = split_once(without_fragment, '?');
    let authority = if authority_part.contains('@') {
        authority_part.to_string()
    } else {
        decode_base64_to_string(authority_part)
            .ok_or_else(|| anyhow!("invalid base64 shadowsocks authority"))?
    };

    let endpoint_part = authority
        .rsplit_once('@')
        .map(|(_, endpoint)| endpoint)
        .unwrap_or(authority.as_str());
    let (host, port) = parse_host_port(endpoint_part)?;
    let name = fragment
        .map(clean_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{host}:{port}"));

    Ok(ParsedLink {
        protocol: "ss".to_string(),
        name,
        endpoint: Endpoint { host, port },
    })
}

fn parse_shadowsocksr(uri: &str) -> Result<ParsedLink> {
    let payload = uri
        .strip_prefix("ssr://")
        .ok_or_else(|| anyhow!("invalid shadowsocksr link"))?;
    let decoded = decode_base64_to_string(payload)
        .ok_or_else(|| anyhow!("invalid base64 shadowsocksr payload"))?;
    let (main, query) = split_once(&decoded, '?');
    let mut pieces = main.split(':');
    let host = pieces
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("ssr link has no host"))?
        .to_string();
    let port = pieces
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("ssr link has no port"))?;
    let name = query
        .and_then(extract_ssr_remarks)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{host}:{port}"));

    Ok(ParsedLink {
        protocol: "ssr".to_string(),
        name,
        endpoint: Endpoint { host, port },
    })
}

fn extract_ssr_remarks(query: &str) -> Option<String> {
    for pair in query.split('&') {
        let (key, value) = split_once(pair, '=');
        if key == "remarks" {
            return value
                .and_then(decode_base64_to_string)
                .map(|text| clean_name(&text));
        }
    }

    None
}

fn parse_host_port(value: &str) -> Result<(String, u16)> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("invalid IPv6 endpoint"))?;
        let port = tail
            .strip_prefix(':')
            .and_then(|port| port.parse::<u16>().ok())
            .ok_or_else(|| anyhow!("endpoint has no port"))?;
        return Ok((host.to_string(), port));
    }

    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("endpoint has no port"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow!("invalid endpoint port"))?;
    Ok((host.to_string(), port))
}

fn split_once(value: &str, delimiter: char) -> (&str, Option<&str>) {
    value
        .split_once(delimiter)
        .map(|(left, right)| (left, Some(right)))
        .unwrap_or((value, None))
}

fn json_value_to_u16(value: &JsonValue) -> Option<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| value.as_str().and_then(|value| value.parse::<u16>().ok()))
}

fn decode_base64_to_string(value: &str) -> Option<String> {
    let decoded = decode_base64(value.trim())?;
    String::from_utf8(decoded).ok()
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    let normalized = value.trim().replace(['\r', '\n'], "");
    if normalized.is_empty() {
        return None;
    }

    for engine in [&STANDARD, &URL_SAFE, &STANDARD_NO_PAD, &URL_SAFE_NO_PAD] {
        if let Ok(decoded) = engine.decode(normalized.as_bytes()) {
            return Some(decoded);
        }
    }

    let padded = pad_base64(&normalized);
    for engine in [&STANDARD, &URL_SAFE] {
        if let Ok(decoded) = engine.decode(padded.as_bytes()) {
            return Some(decoded);
        }
    }

    None
}

fn pad_base64(value: &str) -> String {
    let mut padded = value.to_string();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    padded
}

fn clean_name(value: &str) -> String {
    percent_decode_str(value)
        .decode_utf8_lossy()
        .trim()
        .to_string()
}

fn hash_uri(uri: &str) -> String {
    let mut hasher = DefaultHasher::new();
    uri.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    #[test]
    fn parses_base64_vmess_subscription() {
        let vmess =
            STANDARD.encode(r#"{"v":"2","ps":"demo","add":"example.com","port":"443","id":"id"}"#);
        let body = STANDARD.encode(format!("vmess://{vmess}\n"));
        let parsed = parse_subscription_document("test", 1, body.as_bytes());

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].protocol, "vmess");
        assert_eq!(parsed[0].endpoint.host, "example.com");
        assert_eq!(parsed[0].endpoint.port, 443);
    }

    #[test]
    fn parses_vless_uri() {
        let parsed = parse_subscription_document(
            "test",
            1,
            b"vless://uuid@example.org:8443?security=tls#Fast%20Node",
        );

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].protocol, "vless");
        assert_eq!(parsed[0].name, "Fast Node");
        assert_eq!(parsed[0].endpoint.host, "example.org");
        assert_eq!(parsed[0].endpoint.port, 8443);
    }

    #[test]
    fn parses_sip002_shadowsocks_uri() {
        let parsed = parse_subscription_document(
            "test",
            1,
            b"ss://YWVzLTI1Ni1nY206cGFzcw@example.net:8388#SS",
        );

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].protocol, "ss");
        assert_eq!(parsed[0].endpoint.host, "example.net");
        assert_eq!(parsed[0].endpoint.port, 8388);
    }
}
