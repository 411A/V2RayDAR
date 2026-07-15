use std::collections::BTreeMap;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value as JsonValue;

use crate::{convert::decode_base64_to_string, model::Candidate};

use super::parse_share_link;

/// Try to parse subscription body as a JSON array of full Xray/V2Ray client
/// configs (with `outbounds` arrays). Converts each outbound to a share-link URI.
///
/// Returns `None` if the body doesn't look like Xray configs.
pub fn try_parse_xray_configs(source: &str, priority: u32, body: &[u8]) -> Option<Vec<Candidate>> {
    let text = String::from_utf8_lossy(body);

    // Try direct JSON parse, then base64-decoded
    let configs = parse_xray_json(&text).or_else(|| {
        let trimmed = text.trim();
        let decoded = decode_base64_to_string(trimmed)?;
        parse_xray_json(&decoded)
    })?;

    if configs.is_empty() {
        return None;
    }

    // Validate first element has "outbounds" key
    if !configs[0].get("outbounds").is_some_and(JsonValue::is_array) {
        return None;
    }

    let mut candidates = Vec::new();

    for config in &configs {
        let name = extract_name(config);
        let Some(outbounds) = config.get("outbounds").and_then(JsonValue::as_array) else {
            continue;
        };

        for outbound in outbounds {
            if let Some(uri) = xray_outbound_to_uri(outbound, &name)
                && let Ok(candidate) = parse_share_link(source, priority, &uri)
            {
                candidates.push(candidate);
            }
        }
    }

    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

/// Parse the text as a JSON array (or single object) of Xray configs.
fn parse_xray_json(text: &str) -> Option<Vec<JsonValue>> {
    // Try as array
    if let Ok(JsonValue::Array(arr)) = serde_json::from_str::<JsonValue>(text)
        && !arr.is_empty()
    {
        return Some(arr);
    }

    // Try as single object
    if let Ok(JsonValue::Object(_)) = serde_json::from_str::<JsonValue>(text) {
        let val: JsonValue = serde_json::from_str(text).ok()?;
        return Some(vec![val]);
    }

    None
}

/// Extract display name from a config.
fn extract_name(config: &JsonValue) -> String {
    config
        .get("remarks")
        .or_else(|| config.get("tag"))
        .and_then(JsonValue::as_str)
        .unwrap_or("Unknown")
        .to_string()
}

/// Dispatch outbound conversion based on protocol field.
fn xray_outbound_to_uri(outbound: &JsonValue, name: &str) -> Option<String> {
    let protocol = outbound.get("protocol")?.as_str()?;
    let settings = outbound.get("settings")?;
    let stream = outbound.get("streamSettings");

    match protocol {
        "vless" => xray_vless_to_uri(settings, stream, name),
        "vmess" => xray_vmess_to_uri(settings, stream, name),
        "trojan" => xray_trojan_to_uri(settings, stream, name),
        "shadowsocks" | "ss" => xray_shadowsocks_to_uri(settings, name),
        _ => None,
    }
}

/// Convert an Xray VLESS outbound to a `vless://` share link URI.
#[allow(clippy::too_many_lines)]
fn xray_vless_to_uri(
    settings: &JsonValue,
    stream: Option<&JsonValue>,
    name: &str,
) -> Option<String> {
    let vnext = settings.get("vnext")?.as_array()?;
    let server = vnext.first()?;
    let address = server.get("address")?.as_str()?;
    let port = server.get("port")?.as_u64()?;
    let users = server.get("users")?.as_array()?;
    let user = users.first()?;
    let uuid = user.get("id")?.as_str()?;
    let encryption = user
        .get("encryption")
        .and_then(JsonValue::as_str)
        .unwrap_or("none");
    let flow = user.get("flow").and_then(JsonValue::as_str).unwrap_or("");

    let stream = stream?;
    let network = stream
        .get("network")
        .and_then(JsonValue::as_str)
        .unwrap_or("tcp");
    let security = stream
        .get("security")
        .and_then(JsonValue::as_str)
        .unwrap_or("none");

    let mut params = BTreeMap::new();

    if !encryption.is_empty() && encryption != "none" {
        params.insert("encryption".to_string(), encryption.to_string());
    }
    if !flow.is_empty() {
        params.insert("flow".to_string(), flow.to_string());
    }
    if security != "none" {
        params.insert("security".to_string(), security.to_string());
    }

    apply_security_params(security, stream, &mut params);
    apply_transport_params(network, stream, &mut params);

    let query = encode_params(&params);
    let encoded_name = percent_encode_name(name);

    Some(format!(
        "vless://{uuid}@{address}:{port}?{query}#{encoded_name}"
    ))
}

/// Apply TLS or Reality security parameters.
fn apply_security_params(
    security: &str,
    stream: &JsonValue,
    params: &mut BTreeMap<String, String>,
) {
    if security == "tls"
        && let Some(tls) = stream.get("tlsSettings")
    {
        set_param(params, "sni", tls, "serverName");
        set_param(params, "fp", tls, "fingerprint");
        if let Some(alpn) = tls.get("alpn").and_then(JsonValue::as_array) {
            let alpn_str: Vec<&str> = alpn.iter().filter_map(JsonValue::as_str).collect();
            if !alpn_str.is_empty() {
                params.insert("alpn".to_string(), alpn_str.join(","));
            }
        }
    }

    if security == "reality"
        && let Some(reality) = stream.get("realitySettings")
    {
        set_param(params, "pbk", reality, "publicKey");
        set_param(params, "sid", reality, "shortId");
        set_param(params, "fp", reality, "fingerprint");
        if let Some(sni) = reality
            .get("serverNames")
            .and_then(JsonValue::as_array)
            .and_then(|arr| arr.first())
            .and_then(JsonValue::as_str)
        {
            params.insert("sni".to_string(), sni.to_string());
        }
    }
}

/// Apply transport-specific parameters (ws, grpc, h2).
fn apply_transport_params(
    network: &str,
    stream: &JsonValue,
    params: &mut BTreeMap<String, String>,
) {
    if network != "tcp" {
        params.insert("type".to_string(), network.to_string());
    }

    match network {
        "ws" => {
            if let Some(ws) = stream.get("wsSettings") {
                set_param(params, "path", ws, "path");
                if let Some(host) = ws
                    .get("headers")
                    .and_then(|h| h.get("Host"))
                    .and_then(JsonValue::as_str)
                {
                    params.insert("host".to_string(), host.to_string());
                } else {
                    set_param(params, "host", ws, "host");
                }
            }
        }
        "grpc" => {
            if let Some(grpc) = stream.get("grpcSettings") {
                set_param(params, "serviceName", grpc, "serviceName");
            }
        }
        "h2" | "http" => {
            if let Some(h2) = stream.get("httpSettings") {
                set_param(params, "path", h2, "path");
                set_param(params, "host", h2, "host");
            }
        }
        _ => {}
    }
}

/// Convert an Xray `VMess` outbound to a `vmess://` share link URI.
fn xray_vmess_to_uri(
    settings: &JsonValue,
    stream: Option<&JsonValue>,
    name: &str,
) -> Option<String> {
    let vnext = settings.get("vnext")?.as_array()?;
    let server = vnext.first()?;
    let address = server.get("address")?.as_str()?;
    let port = server.get("port")?.as_u64()?;
    let users = server.get("users")?.as_array()?;
    let user = users.first()?;
    let uuid = user.get("id")?.as_str()?;
    let alter_id = user.get("alterId").and_then(JsonValue::as_u64).unwrap_or(0);
    let security = user
        .get("security")
        .and_then(JsonValue::as_str)
        .unwrap_or("auto");

    let stream = stream?;
    let network = stream
        .get("network")
        .and_then(JsonValue::as_str)
        .unwrap_or("tcp");
    let tls = stream
        .get("security")
        .and_then(JsonValue::as_str)
        .unwrap_or("none");

    // Build the VMess JSON object
    let mut vmess = serde_json::Map::new();
    vmess.insert("v".to_string(), JsonValue::Number(2.into()));
    vmess.insert("ps".to_string(), JsonValue::String(name.to_string()));
    vmess.insert("add".to_string(), JsonValue::String(address.to_string()));
    vmess.insert("port".to_string(), JsonValue::Number(port.into()));
    vmess.insert("id".to_string(), JsonValue::String(uuid.to_string()));
    vmess.insert("aid".to_string(), JsonValue::Number(alter_id.into()));
    vmess.insert("scy".to_string(), JsonValue::String(security.to_string()));
    vmess.insert("net".to_string(), JsonValue::String(network.to_string()));
    vmess.insert("tls".to_string(), JsonValue::String(tls.to_string()));

    if tls != "none"
        && let Some(tls_settings) = stream.get("tlsSettings")
    {
        if let Some(sni) = tls_settings.get("serverName").and_then(JsonValue::as_str) {
            vmess.insert("sni".to_string(), JsonValue::String(sni.to_string()));
        }
        if let Some(fp) = tls_settings.get("fingerprint").and_then(JsonValue::as_str) {
            vmess.insert("fp".to_string(), JsonValue::String(fp.to_string()));
        }
        if let Some(alpn) = tls_settings.get("alpn") {
            vmess.insert("alpn".to_string(), alpn.clone());
        }
    }

    match network {
        "ws" => {
            if let Some(ws) = stream.get("wsSettings") {
                if let Some(path) = ws.get("path").and_then(JsonValue::as_str) {
                    vmess.insert("path".to_string(), JsonValue::String(path.to_string()));
                }
                if let Some(host) = ws
                    .get("headers")
                    .and_then(|h| h.get("Host"))
                    .and_then(JsonValue::as_str)
                {
                    vmess.insert("host".to_string(), JsonValue::String(host.to_string()));
                }
            }
        }
        "grpc" => {
            if let Some(grpc) = stream.get("grpcSettings")
                && let Some(sn) = grpc.get("serviceName").and_then(JsonValue::as_str)
            {
                vmess.insert("path".to_string(), JsonValue::String(sn.to_string()));
            }
        }
        "h2" | "http" => {
            if let Some(h2) = stream.get("httpSettings") {
                if let Some(path) = h2.get("path").and_then(JsonValue::as_str) {
                    vmess.insert("path".to_string(), JsonValue::String(path.to_string()));
                }
                if let Some(host) = h2.get("host").and_then(JsonValue::as_str) {
                    vmess.insert("host".to_string(), JsonValue::String(host.to_string()));
                }
            }
        }
        _ => {}
    }

    let json_str = serde_json::to_string(&vmess).ok()?;
    let encoded = base64_encode(&json_str);

    Some(format!("vmess://{encoded}"))
}

/// Convert an Xray Trojan outbound to a `trojan://` share link URI.
fn xray_trojan_to_uri(
    settings: &JsonValue,
    stream: Option<&JsonValue>,
    name: &str,
) -> Option<String> {
    let servers = settings.get("servers")?.as_array()?;
    let server = servers.first()?;
    let password = server.get("password")?.as_str()?;
    let address = server.get("address")?.as_str()?;
    let port = server.get("port")?.as_u64()?;

    let stream = stream?;
    let network = stream
        .get("network")
        .and_then(JsonValue::as_str)
        .unwrap_or("tcp");
    let security = stream
        .get("security")
        .and_then(JsonValue::as_str)
        .unwrap_or("tls");

    let mut params = BTreeMap::new();

    if security != "none" {
        params.insert("security".to_string(), security.to_string());
    }
    if network != "tcp" {
        params.insert("type".to_string(), network.to_string());
    }

    // TLS settings
    if security == "tls"
        && let Some(tls) = stream.get("tlsSettings")
    {
        set_param(&mut params, "sni", tls, "serverName");
        set_param(&mut params, "fp", tls, "fingerprint");
    }

    // Transport settings
    match network {
        "ws" => {
            if let Some(ws) = stream.get("wsSettings") {
                set_param(&mut params, "path", ws, "path");
                if let Some(host) = ws
                    .get("headers")
                    .and_then(|h| h.get("Host"))
                    .and_then(JsonValue::as_str)
                {
                    params.insert("host".to_string(), host.to_string());
                }
            }
        }
        "grpc" => {
            if let Some(grpc) = stream.get("grpcSettings") {
                set_param(&mut params, "serviceName", grpc, "serviceName");
            }
        }
        _ => {}
    }

    let query = encode_params(&params);
    let encoded_name = percent_encode_name(name);

    Some(format!(
        "trojan://{password}@{address}:{port}?{query}#{encoded_name}"
    ))
}

/// Convert an Xray Shadowsocks outbound to an `ss://` share link URI.
fn xray_shadowsocks_to_uri(settings: &JsonValue, name: &str) -> Option<String> {
    let servers = settings.get("servers")?.as_array()?;
    let server = servers.first()?;
    let method = server.get("method")?.as_str()?;
    let password = server.get("password")?.as_str()?;
    let address = server.get("address")?.as_str()?;
    let port = server.get("port")?.as_u64()?;

    // SIP002 format: ss://base64(method:password)@host:port#name
    let auth = format!("{method}:{password}");
    let encoded_auth = base64_encode(&auth);
    let encoded_name = percent_encode_name(name);

    Some(format!(
        "ss://{encoded_auth}@{address}:{port}#{encoded_name}"
    ))
}

/// Encode query parameters into a URL query string.
/// Insert a parameter if the JSON source has the field.
fn set_param(params: &mut BTreeMap<String, String>, key: &str, source: &JsonValue, field: &str) {
    if let Some(val) = source.get(field).and_then(JsonValue::as_str) {
        params.insert(key.to_string(), val.to_string());
    }
}

fn encode_params(params: &BTreeMap<String, String>) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// URL-encode a string (simple percent encoding for query params).
/// Preserves unreserved characters: A-Z a-z 0-9 - _ . ~
fn url_encode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                write!(result, "{byte:02X}").unwrap();
            }
        }
    }
    result
}

/// Percent-encode a URI fragment/name.
fn percent_encode_name(name: &str) -> String {
    utf8_percent_encode(name, NON_ALPHANUMERIC).to_string()
}

/// Base64-encode a string using standard encoding.
fn base64_encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRAY_VLESS_WS_CONFIG: &str = r#"[
{
    "remarks": "Test VLESS WS",
    "outbounds": [{
        "protocol": "vless",
        "settings": {
            "vnext": [{
                "address": "example.com",
                "port": 443,
                "users": [{
                    "id": "abc-123-def",
                    "encryption": "none"
                }]
            }]
        },
        "streamSettings": {
            "network": "ws",
            "security": "tls",
            "tlsSettings": {
                "serverName": "example.com",
                "fingerprint": "chrome"
            },
            "wsSettings": {
                "path": "/ws-path",
                "headers": { "Host": "ws.example.com" }
            }
        }
    }]
}
]"#;

    const XRAY_VLESS_REALITY_CONFIG: &str = r#"[
{
    "remarks": "Test Reality",
    "outbounds": [{
        "protocol": "vless",
        "settings": {
            "vnext": [{
                "address": "1.2.3.4",
                "port": 443,
                "users": [{
                    "id": "uuid-test",
                    "encryption": "none",
                    "flow": "xtls-rprx-vision"
                }]
            }]
        },
        "streamSettings": {
            "network": "tcp",
            "security": "reality",
            "realitySettings": {
                "serverNames": ["example.com"],
                "publicKey": "abc123",
                "shortId": "def456",
                "fingerprint": "chrome"
            }
        }
    }]
}
]"#;

    const XRAY_TROJAN_CONFIG: &str = r#"[
{
    "remarks": "Test Trojan",
    "outbounds": [{
        "protocol": "trojan",
        "settings": {
            "servers": [{
                "password": "mypassword",
                "address": "trojan.example.com",
                "port": 443
            }]
        },
        "streamSettings": {
            "security": "tls",
            "tlsSettings": {
                "serverName": "trojan.example.com"
            }
        }
    }]
}
]"#;

    const XRAY_SS_CONFIG: &str = r#"[
{
    "remarks": "Test SS",
    "outbounds": [{
        "protocol": "shadowsocks",
        "settings": {
            "servers": [{
                "method": "aes-256-gcm",
                "password": "testpass",
                "address": "ss.example.com",
                "port": 8388
            }]
        }
    }]
}
]"#;

    const XRAY_VMESS_CONFIG: &str = r#"[
{
    "remarks": "Test VMess",
    "outbounds": [{
        "protocol": "vmess",
        "settings": {
            "vnext": [{
                "address": "vmess.example.com",
                "port": 443,
                "users": [{
                    "id": "vmess-uuid",
                    "alterId": 0
                }]
            }]
        },
        "streamSettings": {
            "network": "ws",
            "security": "tls",
            "tlsSettings": {
                "serverName": "vmess.example.com"
            },
            "wsSettings": {
                "path": "/vmess-ws"
            }
        }
    }]
}
]"#;

    #[test]
    fn converts_vless_ws_to_uri() {
        let result = try_parse_xray_configs("test", 100, XRAY_VLESS_WS_CONFIG.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "vless");
        assert_eq!(candidates[0].name, "Test VLESS WS");
        assert!(candidates[0].uri.starts_with("vless://"));
        assert!(candidates[0].uri.contains("example.com"));
        assert!(candidates[0].uri.contains("security=tls"));
        assert!(candidates[0].uri.contains("type=ws"));
    }

    #[test]
    fn converts_vless_reality_to_uri() {
        let result = try_parse_xray_configs("test", 100, XRAY_VLESS_REALITY_CONFIG.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].uri.contains("security=reality"));
        assert!(candidates[0].uri.contains("pbk=abc123"));
        assert!(candidates[0].uri.contains("sid=def456"));
        assert!(candidates[0].uri.contains("flow=xtls-rprx-vision"));
    }

    #[test]
    fn converts_trojan_to_uri() {
        let result = try_parse_xray_configs("test", 100, XRAY_TROJAN_CONFIG.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "trojan");
        assert!(candidates[0].uri.starts_with("trojan://"));
        assert!(candidates[0].uri.contains("mypassword@"));
    }

    #[test]
    fn converts_shadowsocks_to_uri() {
        let result = try_parse_xray_configs("test", 100, XRAY_SS_CONFIG.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "ss");
        assert!(candidates[0].uri.starts_with("ss://"));
    }

    #[test]
    fn converts_vmess_to_uri() {
        let result = try_parse_xray_configs("test", 100, XRAY_VMESS_CONFIG.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "vmess");
        assert!(candidates[0].uri.starts_with("vmess://"));
    }

    #[test]
    fn returns_none_for_non_xray_json() {
        let json = r#"{"not": "xray configs"}"#;
        let result = try_parse_xray_configs("test", 100, json.as_bytes());
        assert!(result.is_none());
    }

    #[test]
    fn returns_none_for_empty_array() {
        let result = try_parse_xray_configs("test", 100, b"[]");
        assert!(result.is_none());
    }

    #[test]
    fn handles_single_xray_config_object() {
        let json = r#"{
            "remarks": "Single Config",
            "outbounds": [{
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "address": "single.example.com",
                        "port": 443,
                        "users": [{"id": "single-uuid", "encryption": "none"}]
                    }]
                },
                "streamSettings": {"network": "tcp", "security": "tls", "tlsSettings": {"serverName": "single.example.com"}}
            }]
        }"#;
        let result = try_parse_xray_configs("test", 100, json.as_bytes());
        assert!(result.is_some());
        let candidates = result.unwrap();
        assert_eq!(candidates.len(), 1);
    }
}
