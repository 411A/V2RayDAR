mod html;
mod xray;

use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use anyhow::{Result, anyhow};
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use url::Url;

use crate::{
    clash::try_parse_clash_subscription,
    constants::SUPPORTED_URI_SCHEMES,
    convert::{
        decode_base64_to_string, parse_host_port as shared_parse_host_port, percent_decode,
        query_pairs, split_once,
    },
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

    // Try Clash/Mihomo structured parsing before generic YAML extraction.
    // If the body contains a `proxies:` list, this yields better results.
    if candidates.is_empty()
        && let Some(clash_candidates) = try_parse_clash_subscription(source, priority, body)
    {
        for candidate in clash_candidates {
            if seen_entries.insert(candidate.uri.clone()) {
                candidates.push(candidate);
            }
        }
    }

    if let Ok(json) = serde_json::from_str::<JsonValue>(&text) {
        collect_entries_from_json(source, priority, &json, &mut candidates, &mut seen_entries);
    }

    if let Ok(yaml) = serde_yaml::from_str::<YamlValue>(&text) {
        collect_entries_from_yaml(source, priority, &yaml, &mut candidates, &mut seen_entries);
    }

    // HTML panel extraction fallback (Marzban/Xray-style subscription panels).
    // Extracts URIs from HTML attributes, base64 blocks in <textarea>/<code>/<pre>,
    // and <script> content.
    if candidates.is_empty()
        && let Some(html_candidates) = html::try_extract_from_html(source, priority, body)
    {
        for candidate in html_candidates {
            if seen_entries.insert(candidate.uri.clone()) {
                candidates.push(candidate);
            }
        }
    }

    // Full Xray/V2Ray JSON config conversion fallback.
    // Handles JSON arrays of complete Xray client configs with outbounds[],
    // converting them to share-link URIs.
    if candidates.is_empty()
        && let Some(xray_candidates) = xray::try_parse_xray_configs(source, priority, body)
    {
        for candidate in xray_candidates {
            if seen_entries.insert(candidate.uri.clone()) {
                candidates.push(candidate);
            }
        }
    }

    candidates
}

pub fn collect_entries_from_text(
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
            collect_entries_from_text(source, priority, text, candidates, seen);
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
            collect_entries_from_text(source, priority, text, candidates, seen);
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

const fn is_token_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']')
}

pub fn parse_share_link(source: &str, priority: u32, uri: &str) -> Result<Candidate> {
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
    let dedup_key = config_dedup_key(&parsed.protocol, &parsed.endpoint, uri);
    Ok(Candidate {
        id,
        dedup_key,
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
            .map(percent_decode)
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
        .map(percent_decode)
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
        .map_or(authority.as_str(), |(_, endpoint)| endpoint);
    let (host, port) = shared_parse_host_port(endpoint_part)?;
    let name = fragment
        .map(percent_decode)
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
                .map(|text| percent_decode(&text));
        }
    }

    None
}

fn json_value_to_u16(value: &JsonValue) -> Option<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| value.as_str().and_then(|value| value.parse::<u16>().ok()))
}

fn hash_uri(uri: &str) -> String {
    let mut hasher = DefaultHasher::new();
    uri.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn config_dedup_key(protocol: &str, endpoint: &Endpoint, uri: &str) -> String {
    let lower = uri.to_ascii_lowercase();
    let (transport, tls) = if lower.starts_with("vmess://") {
        vmess_dedup_parts(uri)
    } else if lower.starts_with("ssr://") {
        ("tcp".to_string(), "none".to_string())
    } else if lower.starts_with("ss://") {
        shadowsocks_dedup_parts(uri)
    } else {
        standard_uri_dedup_parts(uri, protocol_defaults_to_tls(protocol))
    };

    format!(
        "{}|{}|{}|{}|{}",
        protocol.to_ascii_lowercase(),
        normalize_host(&endpoint.host),
        endpoint.port,
        transport,
        tls
    )
}

fn vmess_dedup_parts(uri: &str) -> (String, String) {
    let Some(payload) = uri.strip_prefix("vmess://") else {
        return ("tcp".to_string(), "none".to_string());
    };
    let Some(decoded) = decode_base64_to_string(payload) else {
        return standard_uri_dedup_parts(uri, false);
    };
    let Ok(json) = serde_json::from_str::<JsonValue>(&decoded) else {
        return standard_uri_dedup_parts(uri, false);
    };

    let transport = json
        .get("net")
        .or_else(|| json.get("type"))
        .and_then(JsonValue::as_str)
        .map_or_else(|| "tcp".to_string(), normalize_transport);
    let tls = json
        .get("tls")
        .and_then(JsonValue::as_str)
        .map_or_else(|| "none".to_string(), normalize_tls);

    (transport, tls)
}

fn shadowsocks_dedup_parts(uri: &str) -> (String, String) {
    let Some(body) = uri.strip_prefix("ss://") else {
        return ("tcp".to_string(), "none".to_string());
    };
    let (without_fragment, _) = split_once(body, '#');
    let (_, query) = split_once(without_fragment, '?');
    query.map_or_else(
        || ("tcp".to_string(), "none".to_string()),
        |query| query_dedup_parts(query, false),
    )
}

fn standard_uri_dedup_parts(uri: &str, default_tls: bool) -> (String, String) {
    Url::parse(uri).ok().map_or_else(
        || ("tcp".to_string(), "none".to_string()),
        |url| query_dedup_parts(url.query().unwrap_or_default(), default_tls),
    )
}

fn query_dedup_parts(query: &str, default_tls: bool) -> (String, String) {
    let params = query_pairs(query);
    let transport = params
        .get("type")
        .or_else(|| params.get("net"))
        .or_else(|| params.get("network"))
        .map_or_else(|| "tcp".to_string(), |value| normalize_transport(value));
    let reality_key = params
        .get("pbk")
        .or_else(|| params.get("public_key"))
        .or_else(|| params.get("reality_pbk"))
        .filter(|value| !value.trim().is_empty());
    let tls = match params.get("security").or_else(|| params.get("tls")) {
        Some(value) if normalize_tls(value) == "none" => "none".to_string(),
        Some(value) if normalize_tls(value) == "reality" || reality_key.is_some() => {
            "reality".to_string()
        }
        Some(value) => normalize_tls(value),
        None if reality_key.is_some() => "reality".to_string(),
        None if default_tls => "tls".to_string(),
        None => "none".to_string(),
    };

    (transport, tls)
}

fn normalize_transport(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "tcp" => "tcp".to_string(),
        "websocket" => "ws".to_string(),
        other => other.to_string(),
    }
}

fn normalize_tls(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "none" | "false" | "0" => "none".to_string(),
        "reality" => "reality".to_string(),
        _ => "tls".to_string(),
    }
}

fn normalize_host(host: &str) -> String {
    host.trim_matches(['[', ']']).to_ascii_lowercase()
}

fn protocol_defaults_to_tls(protocol: &str) -> bool {
    matches!(
        protocol.to_ascii_lowercase().as_str(),
        "trojan" | "hysteria2" | "tuic"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};

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
    fn dedup_key_ignores_remarks_for_same_endpoint_type_transport_and_tls() {
        let parsed = parse_subscription_document(
            "test",
            1,
            b"vless://uuid@example.org:8443?security=tls&type=ws#First\nvless://uuid@example.org:8443?type=ws&security=tls#Second",
        );

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].dedup_key, parsed[1].dedup_key);
    }

    #[test]
    fn dedup_key_keeps_different_transport_distinct() {
        let parsed = parse_subscription_document(
            "test",
            1,
            b"vless://uuid@example.org:8443?security=tls&type=tcp#Tcp\nvless://uuid@example.org:8443?security=tls&type=ws#Ws",
        );

        assert_eq!(parsed.len(), 2);
        assert_ne!(parsed[0].dedup_key, parsed[1].dedup_key);
    }

    #[test]
    fn dedup_key_uses_tls_default_for_trojan() {
        let parsed = parse_subscription_document(
            "test",
            1,
            b"trojan://password@example.org:443#DefaultTls\ntrojan://password@example.org:443?security=tls#ExplicitTls",
        );

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].dedup_key, parsed[1].dedup_key);
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
