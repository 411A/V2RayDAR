use std::collections::BTreeMap;

use crate::constants::{URI_PLUGIN, URI_QUERY, YAML_NEEDS_QUOTE};
use anyhow::{Result, anyhow};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use percent_encoding::percent_decode_str;
use serde_json::{Value as JsonValue, json};
use serde_yaml::Value as YamlValue;
use url::Url;

// ── Shared helpers (used by parser.rs, probe.rs, and this module) ──────────

pub fn decode_base64_to_string(value: &str) -> Option<String> {
    decode_base64_bytes(value).and_then(|decoded| String::from_utf8(decoded).ok())
}

pub fn decode_base64_bytes(value: &str) -> Option<Vec<u8>> {
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

pub fn pad_base64(value: &str) -> String {
    let mut padded = value.to_string();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    padded
}

pub fn percent_decode(value: &str) -> String {
    percent_decode_str(value)
        .decode_utf8_lossy()
        .trim()
        .to_string()
}

pub fn split_once(value: &str, delimiter: char) -> (&str, Option<&str>) {
    value
        .split_once(delimiter)
        .map_or((value, None), |(left, right)| (left, Some(right)))
}

pub fn parse_host_port(value: &str) -> Result<(String, u16)> {
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

pub fn json_string(value: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(JsonValue::as_str)
            .map(ToString::to_string)
            .filter(|v| !v.is_empty())
    })
}

pub fn json_u64(value: &JsonValue, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse::<u64>().ok()))
    })
}

pub fn json_u16(value: &JsonValue, keys: &[&str]) -> Option<u16> {
    json_u64(value, keys).and_then(|v| u16::try_from(v).ok())
}

pub fn query_pairs(query: &str) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

pub fn first_param(params: &BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| params.get(*key).filter(|v| !v.is_empty()).cloned())
}

pub fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y"
    )
}

// ── Structured field extraction from URIs ──────────────────────────────────

#[allow(dead_code)]
pub struct VmessFields {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub uuid: String,
    pub aid: u64,
    pub security: String,
    pub net: Option<String>,
    pub tls: Option<String>,
    pub sni: Option<String>,
    pub host_header: Option<String>,
    pub path: Option<String>,
    pub alpn: Option<String>,
    pub fp: Option<String>,
}

#[allow(dead_code)]
pub fn vmess_fields(uri: &str) -> Result<VmessFields> {
    let payload = uri
        .strip_prefix("vmess://")
        .ok_or_else(|| anyhow!("invalid VMess URI"))?;
    let decoded = decode_base64_to_string(payload)
        .ok_or_else(|| anyhow!("VMess payload is not valid base64 UTF-8"))?;
    let json: JsonValue =
        serde_json::from_str(&decoded).map_err(|e| anyhow!("VMess payload is not JSON: {e}"))?;

    let host = json_string(&json, &["add", "address"])
        .ok_or_else(|| anyhow!("VMess payload has no server address"))?;
    let port = json_u64(&json, &["port"])
        .and_then(|v| u16::try_from(v).ok())
        .ok_or_else(|| anyhow!("VMess payload has no port"))?;
    let uuid = json_string(&json, &["id"]).ok_or_else(|| anyhow!("VMess payload has no UUID"))?;
    let name = json_string(&json, &["ps"])
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{host}:{port}"));

    Ok(VmessFields {
        host,
        port,
        name,
        uuid,
        aid: json_u64(&json, &["aid", "alterId"]).unwrap_or(0),
        security: json_string(&json, &["scy", "security"]).unwrap_or_else(|| "auto".to_string()),
        net: json_string(&json, &["net"]),
        tls: json_string(&json, &["tls"]),
        sni: json_string(&json, &["sni"]),
        host_header: json_string(&json, &["host"]),
        path: json_string(&json, &["path"]),
        alpn: json_string(&json, &["alpn"]),
        fp: json_string(&json, &["fp"]),
    })
}

#[allow(dead_code)]
pub struct UriFields {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub name: String,
    pub params: BTreeMap<String, String>,
}

#[allow(dead_code)]
pub fn standard_uri_fields(uri: &str) -> Result<UriFields> {
    let url = Url::parse(uri).map_err(|e| anyhow!("invalid URI: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("URI has no host"))?
        .to_string();
    let port = url.port().ok_or_else(|| anyhow!("URI has no port"))?;
    let username = percent_decode(url.username());
    let password = url.password().map(percent_decode);
    let name = url
        .fragment()
        .map(percent_decode)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{host}:{port}"));
    let params = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    Ok(UriFields {
        scheme: url.scheme().to_string(),
        host,
        port,
        username,
        password,
        name,
        params,
    })
}

#[allow(dead_code)]
pub struct SsFields {
    pub host: String,
    pub port: u16,
    pub method: String,
    pub password: String,
    pub name: String,
    pub plugin: Option<String>,
    pub plugin_opts: Option<String>,
}

#[allow(dead_code)]
pub fn shadowsocks_fields(uri: &str) -> Result<SsFields> {
    let body = uri
        .strip_prefix("ss://")
        .ok_or_else(|| anyhow!("invalid Shadowsocks URI"))?;
    let (without_fragment, fragment) = split_once(body, '#');
    let (authority_part, query) = split_once(without_fragment, '?');
    let authority = if authority_part.contains('@') {
        authority_part.to_string()
    } else {
        decode_base64_to_string(authority_part)
            .ok_or_else(|| anyhow!("invalid Shadowsocks base64 authority"))?
    };

    let (userinfo, endpoint) = authority
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("Shadowsocks link has no user info"))?;
    let userinfo = if userinfo.contains(':') {
        percent_decode(userinfo)
    } else {
        decode_base64_to_string(userinfo)
            .ok_or_else(|| anyhow!("invalid Shadowsocks base64 user info"))?
    };
    let (method, password) = userinfo
        .split_once(':')
        .ok_or_else(|| anyhow!("Shadowsocks user info must be method:password"))?;
    let (host, port) = parse_host_port(endpoint)?;
    let name = fragment
        .map(percent_decode)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{host}:{port}"));

    let mut plugin = None;
    let mut plugin_opts = None;
    if let Some(query) = query {
        let params = query_pairs(query);
        if let Some(p) = first_param(&params, &["plugin"]) {
            let (name, opts) = split_once(&p, ';');
            plugin = Some(name.to_string());
            plugin_opts = opts.map(ToString::to_string);
        }
    }

    Ok(SsFields {
        host,
        port,
        method: normalize_ss_method(method)?,
        password: password.to_string(),
        name,
        plugin,
        plugin_opts,
    })
}

#[allow(dead_code)]
fn normalize_ss_method(method: &str) -> Result<String> {
    let lowered = method.to_ascii_lowercase();
    let normalized = match lowered.as_str() {
        "ss" => return Err(anyhow!("unsupported Shadowsocks method: ss")),
        "chacha20-poly1305" => "chacha20-ietf-poly1305",
        "xchacha20-poly1305" => "xchacha20-ietf-poly1305",
        other => other,
    };
    if crate::constants::SUPPORTED_SS_METHODS.contains(&normalized) {
        Ok(normalized.to_string())
    } else {
        Err(anyhow!(
            "unsupported Shadowsocks cipher: {normalized} (only AEAD/2022 methods are supported)"
        ))
    }
}

// ── Clash/Mihomo proxy entry → V2Ray share-link URI ────────────────────────

pub fn clash_proxy_to_uri(proxy: &YamlValue) -> Result<String> {
    let proxy_type =
        yaml_string(proxy, &["type"]).ok_or_else(|| anyhow!("Clash proxy has no type"))?;
    let server =
        yaml_string(proxy, &["server"]).ok_or_else(|| anyhow!("Clash proxy has no server"))?;
    let port = yaml_u16(proxy, &["port"]).ok_or_else(|| anyhow!("Clash proxy has no port"))?;
    let name = yaml_string(proxy, &["name"]).unwrap_or_else(|| format!("{server}:{port}"));

    match proxy_type.to_ascii_lowercase().as_str() {
        "vmess" => clash_vmess_to_uri(proxy, &server, port, &name),
        "vless" => clash_vless_to_uri(proxy, &server, port, &name),
        "trojan" => clash_trojan_to_uri(proxy, &server, port, &name),
        "ss" => clash_ss_to_uri(proxy, &server, port, &name),
        "ssr" => clash_ssr_to_uri(proxy, &server, port, &name),
        "hysteria2" | "hy2" => Ok(clash_hysteria2_to_uri(proxy, &server, port, &name)),
        "tuic" => clash_tuic_to_uri(proxy, &server, port, &name),
        other => Err(anyhow!("unsupported Clash proxy type: {other}")),
    }
}

fn clash_vmess_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> Result<String> {
    let uuid = yaml_string(proxy, &["uuid"]).ok_or_else(|| anyhow!("VMess proxy has no uuid"))?;
    let aid = yaml_u64(proxy, &["alterId"]).unwrap_or(0);
    let cipher = yaml_string(proxy, &["cipher", "security"]).unwrap_or_else(|| "auto".to_string());
    let tls = yaml_bool_or_string(proxy, &["tls"]).unwrap_or(false);
    let network = yaml_string(proxy, &["network"]);

    let mut vmess_json = serde_json::Map::new();
    vmess_json.insert("v".to_string(), json!("2"));
    vmess_json.insert("ps".to_string(), json!(name));
    vmess_json.insert("add".to_string(), json!(server));
    vmess_json.insert("port".to_string(), json!(port.to_string()));
    vmess_json.insert("id".to_string(), json!(uuid));
    vmess_json.insert("aid".to_string(), json!(aid.to_string()));
    vmess_json.insert("scy".to_string(), json!(cipher));

    if tls {
        vmess_json.insert("tls".to_string(), json!("tls"));
        if let Some(sni) = yaml_string(proxy, &["servername"]) {
            vmess_json.insert("sni".to_string(), json!(sni));
        }
        if let Some(fp) = yaml_string(proxy, &["client-fingerprint"]) {
            vmess_json.insert("fp".to_string(), json!(fp));
        }
        if let Some(alpn) = yaml_alpn(proxy) {
            vmess_json.insert("alpn".to_string(), json!(alpn));
        }
        if yaml_bool(proxy, &["skip-cert-verify"]) {
            vmess_json.insert("allowInsecure".to_string(), json!("1"));
        }
    }

    let net = network.unwrap_or_else(|| "tcp".to_string());
    vmess_json.insert("net".to_string(), json!(net));

    match net.as_str() {
        "ws" => {
            if let Some(path) = yaml_nested_string(proxy, "ws-opts.path") {
                vmess_json.insert("path".to_string(), json!(path));
            }
            if let Some(host) = yaml_ws_host(proxy) {
                vmess_json.insert("host".to_string(), json!(host));
            }
        }
        "grpc" => {
            if let Some(sn) = yaml_nested_string(proxy, "grpc-opts.grpc-service-name") {
                vmess_json.insert("path".to_string(), json!(sn));
            }
        }
        "h2" | "http" => {
            if let Some(path) = yaml_nested_string(proxy, "h2-opts.path") {
                vmess_json.insert("path".to_string(), json!(path));
            }
            if let Some(host) = yaml_h2_host(proxy) {
                vmess_json.insert("host".to_string(), json!(host));
            }
        }
        "httpupgrade" => {
            if let Some(path) = yaml_string(proxy, &["httpupgrade-opts", "path"]) {
                vmess_json.insert("path".to_string(), json!(path));
            }
            if let Some(host) = yaml_string(proxy, &["httpupgrade-opts", "host"]) {
                vmess_json.insert("host".to_string(), json!(host));
            }
        }
        _ => {}
    }

    let encoded = STANDARD.encode(serde_json::to_string(&vmess_json)?);
    Ok(format!("vmess://{encoded}"))
}

fn clash_vless_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> Result<String> {
    let uuid = yaml_string(proxy, &["uuid"]).ok_or_else(|| anyhow!("VLESS proxy has no uuid"))?;

    let mut params = BTreeMap::new();

    if let Some(tls) = yaml_bool_or_string(proxy, &["tls"])
        && tls
    {
        params.insert("security".to_string(), "tls".to_string());
    }

    if let Some(sni) = yaml_string(proxy, &["servername"]) {
        params.insert("sni".to_string(), sni);
    }
    if let Some(fp) = yaml_string(proxy, &["client-fingerprint"]) {
        params.insert("fp".to_string(), fp);
    }
    if let Some(alpn) = yaml_string(proxy, &["alpn"]) {
        params.insert("alpn".to_string(), alpn);
    }
    if let Some(flow) = yaml_string(proxy, &["flow"]) {
        params.insert("flow".to_string(), flow);
    }

    // Reality
    if let Some(pk) = yaml_nested_string(proxy, "reality-opts.public-key") {
        params.insert("pbk".to_string(), pk);
        if params.get("security").map(String::as_str) != Some("reality") {
            params.insert("security".to_string(), "reality".to_string());
        }
    }
    if let Some(sid) = yaml_nested_string(proxy, "reality-opts.short-id") {
        params.insert("sid".to_string(), sid);
    }

    if yaml_bool(proxy, &["skip-cert-verify"]) {
        params.insert("allowInsecure".to_string(), "1".to_string());
    }

    // Transport
    let network = yaml_string(proxy, &["network"]).unwrap_or_else(|| "tcp".to_string());
    if network != "tcp" {
        params.insert("type".to_string(), network.clone());
    }

    match network.as_str() {
        "ws" => {
            if let Some(path) = yaml_nested_string(proxy, "ws-opts.path") {
                params.insert("path".to_string(), path);
            }
            if let Some(host) = yaml_ws_host(proxy) {
                params.insert("host".to_string(), host);
            }
        }
        "grpc" => {
            if let Some(sn) = yaml_nested_string(proxy, "grpc-opts.grpc-service-name") {
                params.insert("serviceName".to_string(), sn);
            }
        }
        "h2" | "http" => {
            if let Some(path) = yaml_nested_string(proxy, "h2-opts.path") {
                params.insert("path".to_string(), path);
            }
            if let Some(host) = yaml_h2_host(proxy) {
                params.insert("host".to_string(), host);
            }
        }
        "httpupgrade" => {
            if let Some(path) = yaml_nested_string(proxy, "httpupgrade-opts.path") {
                params.insert("path".to_string(), path);
            }
            if let Some(host) = yaml_string(proxy, &["httpupgrade-opts", "host"]) {
                params.insert("host".to_string(), host);
            }
        }
        _ => {}
    }

    let query = encode_query(&params);
    let encoded_name =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();

    Ok(format!(
        "vless://{uuid}@{server}:{port}?{query}#{encoded_name}"
    ))
}

fn clash_trojan_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> Result<String> {
    let password =
        yaml_string(proxy, &["password"]).ok_or_else(|| anyhow!("Trojan proxy has no password"))?;

    let mut params = BTreeMap::new();

    if yaml_bool_or_string(proxy, &["tls"]).unwrap_or(true) {
        params.insert("security".to_string(), "tls".to_string());
    }
    if let Some(sni) = yaml_string(proxy, &["servername"]) {
        params.insert("sni".to_string(), sni);
    }
    if yaml_bool(proxy, &["skip-cert-verify"]) {
        params.insert("allowInsecure".to_string(), "1".to_string());
    }

    let network = yaml_string(proxy, &["network"]).unwrap_or_else(|| "tcp".to_string());
    if network != "tcp" {
        params.insert("type".to_string(), network.clone());
    }
    match network.as_str() {
        "ws" => {
            if let Some(path) = yaml_string(proxy, &["ws-opts", "path"]) {
                params.insert("path".to_string(), path);
            }
            if let Some(host) = yaml_ws_host(proxy) {
                params.insert("host".to_string(), host);
            }
        }
        "grpc" => {
            if let Some(sn) = yaml_string(proxy, &["grpc-opts", "grpc-service-name"]) {
                params.insert("serviceName".to_string(), sn);
            }
        }
        "h2" | "http" => {
            if let Some(path) = yaml_string(proxy, &["h2-opts", "path"]) {
                params.insert("path".to_string(), path);
            }
            if let Some(host) = yaml_h2_host(proxy) {
                params.insert("host".to_string(), host);
            }
        }
        "httpupgrade" => {
            if let Some(path) = yaml_string(proxy, &["httpupgrade-opts", "path"]) {
                params.insert("path".to_string(), path);
            }
            if let Some(host) = yaml_string(proxy, &["httpupgrade-opts", "host"]) {
                params.insert("host".to_string(), host);
            }
        }
        _ => {}
    }

    let query = encode_query(&params);
    let encoded_name =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();

    Ok(format!(
        "trojan://{password}@{server}:{port}?{query}#{encoded_name}"
    ))
}

fn clash_ss_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> Result<String> {
    let cipher =
        yaml_string(proxy, &["cipher"]).ok_or_else(|| anyhow!("SS proxy has no cipher"))?;
    let password =
        yaml_string(proxy, &["password"]).ok_or_else(|| anyhow!("SS proxy has no password"))?;

    let authority = STANDARD.encode(format!("{cipher}:{password}"));
    let encoded_name =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();

    let mut uri = format!("ss://{authority}@{server}:{port}#{encoded_name}");

    let mut query_parts = Vec::new();
    if let Some(plugin) = yaml_string(proxy, &["plugin"]) {
        let opts = yaml_string(proxy, &["plugin-opts"])
            .map(|o| format!(";{o}"))
            .unwrap_or_default();
        let plugin_value =
            percent_encoding::utf8_percent_encode(&format!("{plugin}{opts}"), URI_PLUGIN)
                .to_string();
        query_parts.push(format!("plugin={plugin_value}"));
    }
    if !query_parts.is_empty() {
        let q = query_parts.join("&");
        let hash_pos = uri.find('#').unwrap_or(uri.len());
        uri.insert_str(hash_pos, &format!("?{q}"));
    }

    Ok(uri)
}

// ── V2Ray share-link URI → Clash/Mihomo proxy entry ────────────────────────

#[allow(dead_code)]
pub fn uri_to_clash_proxy(uri: &str) -> Result<YamlValue> {
    let lower = uri.to_ascii_lowercase();
    if lower.starts_with("vmess://") {
        vmess_uri_to_clash(uri)
    } else if lower.starts_with("vless://") {
        standard_uri_to_clash(uri, "vless")
    } else if lower.starts_with("trojan://") {
        standard_uri_to_clash(uri, "trojan")
    } else if lower.starts_with("ss://") {
        ss_uri_to_clash(uri)
    } else if lower.starts_with("ssr://") {
        ssr_uri_to_clash(uri)
    } else if lower.starts_with("hysteria2://") || lower.starts_with("hy2://") {
        hysteria2_uri_to_clash(uri)
    } else if lower.starts_with("tuic://") {
        tuic_uri_to_clash(uri)
    } else {
        Err(anyhow!("unsupported URI scheme for Clash conversion"))
    }
}

#[allow(dead_code)]
fn vmess_uri_to_clash(uri: &str) -> Result<YamlValue> {
    let f = vmess_fields(uri)?;

    let mut proxy = serde_yaml::Mapping::new();
    proxy.insert(yaml_key("name"), YamlValue::String(f.name));
    proxy.insert(yaml_key("type"), YamlValue::String("vmess".to_string()));
    proxy.insert(yaml_key("server"), YamlValue::String(f.host));
    proxy.insert(yaml_key("port"), YamlValue::Number(f.port.into()));
    proxy.insert(yaml_key("uuid"), YamlValue::String(f.uuid));
    proxy.insert(yaml_key("alterId"), YamlValue::Number(f.aid.into()));
    proxy.insert(yaml_key("cipher"), YamlValue::String(f.security));
    proxy.insert(yaml_key("udp"), YamlValue::Bool(true));

    let tls_enabled = f
        .tls
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("tls"));
    if tls_enabled {
        proxy.insert(yaml_key("tls"), YamlValue::Bool(true));
        if let Some(sni) = f.sni {
            proxy.insert(yaml_key("servername"), YamlValue::String(sni));
        }
        if let Some(fp) = f.fp {
            proxy.insert(yaml_key("client-fingerprint"), YamlValue::String(fp));
        }
        if let Some(alpn) = f.alpn {
            let alpn_vals: Vec<YamlValue> = alpn
                .split(',')
                .map(|s| YamlValue::String(s.trim().to_string()))
                .collect();
            proxy.insert(yaml_key("alpn"), YamlValue::Sequence(alpn_vals));
        }
    }

    let net = f.net.unwrap_or_else(|| "tcp".to_string());
    if net != "tcp" {
        proxy.insert(yaml_key("network"), YamlValue::String(net.clone()));
    }

    match net.as_str() {
        "ws" => {
            let mut ws_opts = serde_yaml::Mapping::new();
            if let Some(path) = f.path {
                ws_opts.insert(yaml_key("path"), YamlValue::String(path));
            }
            if let Some(host) = f.host_header {
                let mut headers = serde_yaml::Mapping::new();
                headers.insert(yaml_key("Host"), YamlValue::String(host));
                ws_opts.insert(yaml_key("headers"), YamlValue::Mapping(headers));
            }
            proxy.insert(yaml_key("ws-opts"), YamlValue::Mapping(ws_opts));
        }
        "grpc" => {
            if let Some(sn) = f.path {
                let mut grpc_opts = serde_yaml::Mapping::new();
                grpc_opts.insert(yaml_key("grpc-service-name"), YamlValue::String(sn));
                proxy.insert(yaml_key("grpc-opts"), YamlValue::Mapping(grpc_opts));
            }
        }
        "h2" | "http" => {
            let mut h2_opts = serde_yaml::Mapping::new();
            if let Some(path) = f.path {
                h2_opts.insert(yaml_key("path"), YamlValue::String(path));
            }
            if let Some(host) = f.host_header {
                h2_opts.insert(
                    yaml_key("host"),
                    YamlValue::Sequence(vec![YamlValue::String(host)]),
                );
            }
            proxy.insert(yaml_key("h2-opts"), YamlValue::Mapping(h2_opts));
        }
        "httpupgrade" => {
            let mut opts = serde_yaml::Mapping::new();
            if let Some(path) = f.path {
                opts.insert(yaml_key("path"), YamlValue::String(path));
            }
            if let Some(host) = f.host_header {
                opts.insert(yaml_key("host"), YamlValue::String(host));
            }
            proxy.insert(yaml_key("httpupgrade-opts"), YamlValue::Mapping(opts));
        }
        _ => {}
    }

    Ok(YamlValue::Mapping(proxy))
}

#[allow(dead_code, clippy::too_many_lines)]
fn standard_uri_to_clash(uri: &str, protocol: &str) -> Result<YamlValue> {
    let f = standard_uri_fields(uri)?;

    let mut proxy = serde_yaml::Mapping::new();
    proxy.insert(yaml_key("name"), YamlValue::String(f.name));
    proxy.insert(yaml_key("type"), YamlValue::String(protocol.to_string()));
    proxy.insert(yaml_key("server"), YamlValue::String(f.host.clone()));
    proxy.insert(yaml_key("port"), YamlValue::Number(f.port.into()));
    proxy.insert(yaml_key("udp"), YamlValue::Bool(true));

    match protocol {
        "vless" => {
            proxy.insert(yaml_key("uuid"), YamlValue::String(f.username.clone()));
            if let Some(flow) = first_param(&f.params, &["flow"]) {
                proxy.insert(yaml_key("flow"), YamlValue::String(flow));
            }
        }
        "trojan" => {
            proxy.insert(yaml_key("password"), YamlValue::String(f.username.clone()));
        }
        _ => {}
    }

    let has_reality = f.params.contains_key("pbk") || f.params.contains_key("public_key");
    let security = first_param(&f.params, &["security", "tls"]).unwrap_or_default();

    if security == "reality" || has_reality {
        proxy.insert(yaml_key("tls"), YamlValue::Bool(true));
        if let Some(sni) = first_param(&f.params, &["sni", "serverName", "peer"]) {
            proxy.insert(yaml_key("servername"), YamlValue::String(sni));
        }
        if let Some(fp) = first_param(&f.params, &["fp", "fingerprint"]) {
            proxy.insert(yaml_key("client-fingerprint"), YamlValue::String(fp));
        }
        let mut reality_opts = serde_yaml::Mapping::new();
        if let Some(pk) = first_param(&f.params, &["pbk", "public_key"]) {
            reality_opts.insert(yaml_key("public-key"), YamlValue::String(pk));
        }
        if let Some(sid) = first_param(&f.params, &["sid", "short_id"]) {
            reality_opts.insert(yaml_key("short-id"), YamlValue::String(sid));
        }
        if !reality_opts.is_empty() {
            proxy.insert(yaml_key("reality-opts"), YamlValue::Mapping(reality_opts));
        }
    } else if security == "tls" || protocol == "trojan" {
        proxy.insert(yaml_key("tls"), YamlValue::Bool(true));
        if let Some(sni) = first_param(&f.params, &["sni", "serverName", "peer"]) {
            proxy.insert(yaml_key("servername"), YamlValue::String(sni));
        }
        if let Some(fp) = first_param(&f.params, &["fp", "fingerprint"]) {
            proxy.insert(yaml_key("client-fingerprint"), YamlValue::String(fp));
        }
    }

    if first_param(
        &f.params,
        &["allowInsecure", "insecure", "skip-cert-verify"],
    )
    .as_deref()
    .is_some_and(truthy)
    {
        proxy.insert(yaml_key("skip-cert-verify"), YamlValue::Bool(true));
    }

    if let Some(alpn) = first_param(&f.params, &["alpn"]) {
        let alpn_vals: Vec<YamlValue> = alpn
            .split(',')
            .map(|s| YamlValue::String(s.trim().to_string()))
            .collect();
        proxy.insert(yaml_key("alpn"), YamlValue::Sequence(alpn_vals));
    }

    // Transport
    let network =
        first_param(&f.params, &["type", "net", "network"]).unwrap_or_else(|| "tcp".to_string());
    if network != "tcp" {
        proxy.insert(yaml_key("network"), YamlValue::String(network.clone()));
    }

    match network.as_str() {
        "ws" | "websocket" => {
            let mut ws_opts = serde_yaml::Mapping::new();
            if let Some(path) = first_param(&f.params, &["path"]) {
                ws_opts.insert(yaml_key("path"), YamlValue::String(path));
            }
            if let Some(host) = first_param(&f.params, &["host"]) {
                let mut headers = serde_yaml::Mapping::new();
                headers.insert(yaml_key("Host"), YamlValue::String(host));
                ws_opts.insert(yaml_key("headers"), YamlValue::Mapping(headers));
            }
            proxy.insert(yaml_key("ws-opts"), YamlValue::Mapping(ws_opts));
        }
        "grpc" => {
            if let Some(sn) = first_param(&f.params, &["serviceName", "service_name"]) {
                let mut grpc_opts = serde_yaml::Mapping::new();
                grpc_opts.insert(yaml_key("grpc-service-name"), YamlValue::String(sn));
                proxy.insert(yaml_key("grpc-opts"), YamlValue::Mapping(grpc_opts));
            }
        }
        "h2" | "http" => {
            let mut h2_opts = serde_yaml::Mapping::new();
            if let Some(path) = first_param(&f.params, &["path"]) {
                h2_opts.insert(yaml_key("path"), YamlValue::String(path));
            }
            if let Some(host) = first_param(&f.params, &["host"]) {
                h2_opts.insert(
                    yaml_key("host"),
                    YamlValue::Sequence(vec![YamlValue::String(host)]),
                );
            }
            proxy.insert(yaml_key("h2-opts"), YamlValue::Mapping(h2_opts));
        }
        "httpupgrade" => {
            let mut opts = serde_yaml::Mapping::new();
            if let Some(path) = first_param(&f.params, &["path"]) {
                opts.insert(yaml_key("path"), YamlValue::String(path));
            }
            if let Some(host) = first_param(&f.params, &["host"]) {
                opts.insert(yaml_key("host"), YamlValue::String(host));
            }
            proxy.insert(yaml_key("httpupgrade-opts"), YamlValue::Mapping(opts));
        }
        _ => {}
    }

    Ok(YamlValue::Mapping(proxy))
}

#[allow(dead_code)]
fn ss_uri_to_clash(uri: &str) -> Result<YamlValue> {
    let f = shadowsocks_fields(uri)?;

    let mut proxy = serde_yaml::Mapping::new();
    proxy.insert(yaml_key("name"), YamlValue::String(f.name));
    proxy.insert(yaml_key("type"), YamlValue::String("ss".to_string()));
    proxy.insert(yaml_key("server"), YamlValue::String(f.host));
    proxy.insert(yaml_key("port"), YamlValue::Number(f.port.into()));
    proxy.insert(yaml_key("cipher"), YamlValue::String(f.method));
    proxy.insert(yaml_key("password"), YamlValue::String(f.password));
    proxy.insert(yaml_key("udp"), YamlValue::Bool(true));

    if let Some(plugin) = f.plugin {
        proxy.insert(yaml_key("plugin"), YamlValue::String(plugin));
    }
    if let Some(opts) = f.plugin_opts {
        proxy.insert(yaml_key("plugin-opts"), YamlValue::String(opts));
    }

    Ok(YamlValue::Mapping(proxy))
}

// ── SSR URI → Clash proxy ──────────────────────────────────────────────────

#[allow(dead_code)]
fn ssr_uri_to_clash(uri: &str) -> Result<YamlValue> {
    let rest = uri
        .strip_prefix("ssr://")
        .ok_or_else(|| anyhow!("not an SSR URI"))?;
    // SSR format: ssr://base64payload/?param=val
    // The base64 payload may be followed by /?params — strip that first.
    let (encoded, params_part) = rest
        .find("/?")
        .map_or((rest, ""), |pos| (&rest[..pos], &rest[pos + 1..]));
    let decoded_bytes = decode_base64_to_string(encoded)
        .ok_or_else(|| anyhow!("failed to base64-decode SSR payload"))?;
    // Format: host:port:protocol:method:obfs:base64pass
    let main_part = decoded_bytes.split('/').next().unwrap_or(&decoded_bytes);

    let fields: Vec<&str> = main_part.splitn(6, ':').collect();
    if fields.len() < 6 {
        return Err(anyhow!("SSR URI has insufficient fields"));
    }
    let host = fields[0];
    let port: u16 = fields[1]
        .parse()
        .map_err(|_| anyhow!("SSR URI has invalid port"))?;
    let protocol = fields[2];
    let method = fields[3];
    let obfs = fields[4];
    let password_b64 = fields[5];
    let password = decode_base64_to_string(password_b64)
        .ok_or_else(|| anyhow!("SSR URI has invalid password base64"))?;

    // Parse query params from the remainder
    let params = parse_query_params(params_part);
    let name = params
        .get("remark")
        .cloned()
        .unwrap_or_else(|| format!("{host}:{port}"));

    let mut proxy = serde_yaml::Mapping::new();
    proxy.insert(yaml_key("name"), YamlValue::String(name));
    proxy.insert(yaml_key("type"), YamlValue::String("ssr".to_string()));
    proxy.insert(yaml_key("server"), YamlValue::String(host.to_string()));
    proxy.insert(yaml_key("port"), YamlValue::Number(port.into()));
    proxy.insert(yaml_key("cipher"), YamlValue::String(method.to_string()));
    proxy.insert(yaml_key("password"), YamlValue::String(password));
    proxy.insert(
        yaml_key("protocol"),
        YamlValue::String(protocol.to_string()),
    );
    proxy.insert(yaml_key("obfs"), YamlValue::String(obfs.to_string()));
    proxy.insert(yaml_key("udp"), YamlValue::Bool(true));

    if let Some(obfs_param) = params.get("obfsparam")
        && !obfs_param.is_empty()
    {
        proxy.insert(
            yaml_key("obfs-param"),
            YamlValue::String(obfs_param.clone()),
        );
    }
    if let Some(protocol_param) = params.get("protoparam")
        && !protocol_param.is_empty()
    {
        proxy.insert(
            yaml_key("protocol-param"),
            YamlValue::String(protocol_param.clone()),
        );
    }

    Ok(YamlValue::Mapping(proxy))
}

// ── Hysteria2 URI → Clash proxy ────────────────────────────────────────────

#[allow(dead_code)]
fn hysteria2_uri_to_clash(uri: &str) -> Result<YamlValue> {
    let lower = uri.to_ascii_lowercase();
    let raw = if lower.starts_with("hysteria2://") {
        &uri["hysteria2://".len()..]
    } else if lower.starts_with("hy2://") {
        &uri["hy2://".len()..]
    } else {
        return Err(anyhow!("not a Hysteria2 URI"));
    };

    let parsed = Url::parse(&format!("hysteria2://{raw}"))
        .map_err(|e| anyhow!("invalid Hysteria2 URI: {e}"))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("Hysteria2 URI has no host"))?;
    let port = parsed
        .port()
        .ok_or_else(|| anyhow!("Hysteria2 URI has no port"))?;

    // Auth is username portion (may be empty for password-only auth)
    let password = if parsed.username().is_empty() {
        String::new()
    } else {
        percent_decode_str(parsed.username())
            .decode_utf8_lossy()
            .into_owned()
    };

    let fragment = percent_decode_str(parsed.fragment().unwrap_or(""))
        .decode_utf8_lossy()
        .into_owned();
    let name = if fragment.is_empty() {
        format!("{host}:{port}")
    } else {
        fragment
    };

    let params = parse_query_params(parsed.query().unwrap_or(""));

    let mut proxy = serde_yaml::Mapping::new();
    proxy.insert(yaml_key("name"), YamlValue::String(name));
    proxy.insert(yaml_key("type"), YamlValue::String("hysteria2".to_string()));
    proxy.insert(yaml_key("server"), YamlValue::String(host.to_string()));
    proxy.insert(yaml_key("port"), YamlValue::Number(port.into()));
    proxy.insert(yaml_key("udp"), YamlValue::Bool(true));

    if !password.is_empty() {
        proxy.insert(yaml_key("password"), YamlValue::String(password));
    }

    if let Some(sni) = params.get("sni") {
        proxy.insert(yaml_key("sni"), YamlValue::String(sni.clone()));
    }
    if params
        .get("insecure")
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        proxy.insert(yaml_key("skip-cert-verify"), YamlValue::Bool(true));
    }
    if let Some(pinsha256) = params.get("pinsha256") {
        proxy.insert(yaml_key("pinSHA256"), YamlValue::String(pinsha256.clone()));
    }
    if let Some(bw) = params.get("up") {
        proxy.insert(yaml_key("up"), YamlValue::String(bw.clone()));
    }
    if let Some(bw) = params.get("down") {
        proxy.insert(yaml_key("down"), YamlValue::String(bw.clone()));
    }

    Ok(YamlValue::Mapping(proxy))
}

// ── TUIC URI → Clash proxy ─────────────────────────────────────────────────

#[allow(dead_code)]
fn tuic_uri_to_clash(uri: &str) -> Result<YamlValue> {
    let raw = uri
        .strip_prefix("tuic://")
        .ok_or_else(|| anyhow!("not a TUIC URI"))?;
    let parsed =
        Url::parse(&format!("tuic://{raw}")).map_err(|e| anyhow!("invalid TUIC URI: {e}"))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("TUIC URI has no host"))?;
    let port = parsed
        .port()
        .ok_or_else(|| anyhow!("TUIC URI has no port"))?;

    let uuid = percent_decode_str(parsed.username())
        .decode_utf8_lossy()
        .into_owned();
    let password = percent_decode_str(parsed.password().unwrap_or(""))
        .decode_utf8_lossy()
        .into_owned();

    let fragment = percent_decode_str(parsed.fragment().unwrap_or(""))
        .decode_utf8_lossy()
        .into_owned();
    let name = if fragment.is_empty() {
        format!("{host}:{port}")
    } else {
        fragment
    };

    let params = parse_query_params(parsed.query().unwrap_or(""));

    let mut proxy = serde_yaml::Mapping::new();
    proxy.insert(yaml_key("name"), YamlValue::String(name));
    proxy.insert(yaml_key("type"), YamlValue::String("tuic".to_string()));
    proxy.insert(yaml_key("server"), YamlValue::String(host.to_string()));
    proxy.insert(yaml_key("port"), YamlValue::Number(port.into()));
    proxy.insert(yaml_key("uuid"), YamlValue::String(uuid));
    proxy.insert(yaml_key("password"), YamlValue::String(password));
    proxy.insert(yaml_key("udp"), YamlValue::Bool(true));

    if let Some(sni) = params.get("sni") {
        proxy.insert(yaml_key("sni"), YamlValue::String(sni.clone()));
    }
    if params
        .get("insecure")
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        proxy.insert(yaml_key("skip-cert-verify"), YamlValue::Bool(true));
    }
    if let Some(alpn) = params.get("alpn") {
        let alpn_vals: Vec<YamlValue> = alpn
            .split(',')
            .map(|s| YamlValue::String(s.trim().to_string()))
            .collect();
        proxy.insert(yaml_key("alpn"), YamlValue::Sequence(alpn_vals));
    }
    if let Some(congestion) = params.get("congestion_control") {
        proxy.insert(
            yaml_key("congestion-control"),
            YamlValue::String(congestion.clone()),
        );
    }

    Ok(YamlValue::Mapping(proxy))
}

// ── Clash SSR proxy → URI ──────────────────────────────────────────────────

#[allow(dead_code)]
fn clash_ssr_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> Result<String> {
    let password =
        yaml_string(proxy, &["password"]).ok_or_else(|| anyhow!("SSR proxy has no password"))?;
    let protocol = yaml_string(proxy, &["protocol"]).unwrap_or_else(|| "origin".to_string());
    let method = yaml_string(proxy, &["cipher"]).unwrap_or_else(|| "none".to_string());
    let obfs = yaml_string(proxy, &["obfs"]).unwrap_or_else(|| "plain".to_string());

    let password_b64 = STANDARD.encode(&password);

    // Build the main payload: host:port:protocol:method:obfs:base64pass
    let main_payload = format!("{server}:{port}:{protocol}:{method}:{obfs}:{password_b64}");

    let mut params = Vec::new();
    if let Some(obfs_param) = yaml_string(proxy, &["obfs-param"]) {
        params.push(format!("obfsparam={}", STANDARD.encode(&obfs_param)));
    }
    if let Some(proto_param) = yaml_string(proxy, &["protocol-param"]) {
        params.push(format!("protoparam={}", STANDARD.encode(&proto_param)));
    }
    params.push(format!("remark={}", STANDARD.encode(name)));

    let encoded_payload = STANDARD.encode(&main_payload);
    let query = if params.is_empty() {
        String::new()
    } else {
        format!("/?{}", params.join("&"))
    };

    Ok(format!("ssr://{encoded_payload}{query}"))
}

// ── Clash Hysteria2 proxy → URI ────────────────────────────────────────────

#[allow(dead_code)]
fn clash_hysteria2_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> String {
    let password = yaml_string(proxy, &["password"]).unwrap_or_default();

    let mut params = BTreeMap::new();
    if let Some(sni) = yaml_string(proxy, &["sni"]) {
        params.insert("sni".to_string(), sni);
    }
    if yaml_bool(proxy, &["skip-cert-verify"]) {
        params.insert("insecure".to_string(), "1".to_string());
    }
    if let Some(pin) = yaml_string(proxy, &["pinSHA256"]) {
        params.insert("pinsha256".to_string(), pin);
    }
    if let Some(up) = yaml_string(proxy, &["up"]) {
        params.insert("up".to_string(), up);
    }
    if let Some(down) = yaml_string(proxy, &["down"]) {
        params.insert("down".to_string(), down);
    }

    let encoded_name =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();

    let auth = if password.is_empty() {
        String::new()
    } else {
        let encoded_pw = percent_encoding::utf8_percent_encode(&password, URI_QUERY).to_string();
        format!("{encoded_pw}@")
    };

    let query = encode_query(&params);

    format!("hysteria2://{auth}{server}:{port}?{query}#{encoded_name}")
}

// ── Clash TUIC proxy → URI ─────────────────────────────────────────────────

#[allow(dead_code)]
fn clash_tuic_to_uri(proxy: &YamlValue, server: &str, port: u16, name: &str) -> Result<String> {
    let uuid = yaml_string(proxy, &["uuid"]).ok_or_else(|| anyhow!("TUIC proxy has no uuid"))?;
    let password =
        yaml_string(proxy, &["password"]).ok_or_else(|| anyhow!("TUIC proxy has no password"))?;

    let mut params = BTreeMap::new();
    if let Some(sni) = yaml_string(proxy, &["sni"]) {
        params.insert("sni".to_string(), sni);
    }
    if yaml_bool(proxy, &["skip-cert-verify"]) {
        params.insert("insecure".to_string(), "1".to_string());
    }
    if let Some(alpn) = yaml_alpn(proxy) {
        params.insert("alpn".to_string(), alpn);
    }
    if let Some(cc) = yaml_string(proxy, &["congestion-control"]) {
        params.insert("congestion_control".to_string(), cc);
    }

    let encoded_name =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();

    let encoded_uuid = percent_encoding::utf8_percent_encode(&uuid, URI_QUERY).to_string();
    let encoded_password = percent_encoding::utf8_percent_encode(&password, URI_QUERY).to_string();

    let query = encode_query(&params);

    Ok(format!(
        "tuic://{encoded_uuid}:{encoded_password}@{server}:{port}?{query}#{encoded_name}"
    ))
}

// ── Query string helpers ───────────────────────────────────────────────────

fn parse_query_params(query: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    if query.is_empty() {
        return params;
    }
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if let (Some(key), Some(val)) = (kv.next(), kv.next())
            && !key.is_empty()
        {
            params.insert(
                key.to_string(),
                percent_decode_str(val).decode_utf8_lossy().into_owned(),
            );
        }
    }
    params
}

// ── YAML helpers ───────────────────────────────────────────────────────────

fn yaml_map_get<'a>(value: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    value.get(YamlValue::String(key.to_string()))
}

/// Look up a nested string value by traversing a dot-separated path of YAML
/// mapping keys. For example, `"reality-opts.public-key"` looks up
/// `proxy["reality-opts"]["public-key"]`.
fn yaml_nested_string(value: &YamlValue, path: &str) -> Option<String> {
    let mut current = value;
    for segment in path.split('.') {
        let map = current.as_mapping()?;
        current = map.get(YamlValue::String(segment.to_string()))?;
    }
    current.as_str().map(ToString::to_string)
}

fn yaml_string(value: &YamlValue, keys: &[&str]) -> Option<String> {
    if let YamlValue::Mapping(map) = value {
        for key in keys {
            if let Some(YamlValue::String(s)) = yaml_map_get(map, key)
                && !s.is_empty()
            {
                return Some(s.clone());
            }
        }
    }
    None
}

fn yaml_u16(value: &YamlValue, keys: &[&str]) -> Option<u16> {
    if let YamlValue::Mapping(map) = value {
        for key in keys {
            if let Some(val) = yaml_map_get(map, key) {
                match val {
                    YamlValue::Number(n) => {
                        if let Some(v) = n.as_u64() {
                            return u16::try_from(v).ok();
                        }
                    }
                    YamlValue::String(s) => {
                        if let Ok(v) = s.parse::<u16>() {
                            return Some(v);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

fn yaml_u64(value: &YamlValue, keys: &[&str]) -> Option<u64> {
    if let YamlValue::Mapping(map) = value {
        for key in keys {
            if let Some(val) = yaml_map_get(map, key) {
                match val {
                    YamlValue::Number(n) => return n.as_u64(),
                    YamlValue::String(s) => {
                        if let Ok(v) = s.parse::<u64>() {
                            return Some(v);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

fn yaml_bool(value: &YamlValue, keys: &[&str]) -> bool {
    if let YamlValue::Mapping(map) = value {
        for key in keys {
            if let Some(val) = yaml_map_get(map, key) {
                match val {
                    YamlValue::Bool(b) => return *b,
                    YamlValue::String(s) => return truthy(s),
                    YamlValue::Number(n) => {
                        return n.as_u64().unwrap_or(0) != 0;
                    }
                    _ => {}
                }
            }
        }
    }
    false
}

fn yaml_bool_or_string(value: &YamlValue, keys: &[&str]) -> Option<bool> {
    if let YamlValue::Mapping(map) = value {
        for key in keys {
            if let Some(val) = yaml_map_get(map, key) {
                match val {
                    YamlValue::Bool(b) => return Some(*b),
                    YamlValue::String(s) => {
                        return match s.to_ascii_lowercase().as_str() {
                            "true" | "1" | "yes" => Some(true),
                            "false" | "0" | "no" | "none" => Some(false),
                            _ => None,
                        };
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

fn yaml_alpn(value: &YamlValue) -> Option<String> {
    if let YamlValue::Mapping(map) = value
        && let Some(val) = yaml_map_get(map, "alpn")
    {
        match val {
            YamlValue::String(s) => return Some(s.clone()),
            YamlValue::Sequence(seq) => {
                let parts: Vec<&str> = seq
                    .iter()
                    .filter_map(|v| match v {
                        YamlValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join(","));
                }
            }
            _ => {}
        }
    }
    None
}

fn yaml_ws_host(value: &YamlValue) -> Option<String> {
    let map = value.as_mapping()?;
    let ws = map.get(YamlValue::String("ws-opts".into()))?;
    let ws_map = ws.as_mapping()?;
    let headers = ws_map.get(YamlValue::String("headers".into()))?;
    let h = headers.as_mapping()?;
    if let Some(YamlValue::String(host)) = h.get(YamlValue::String("Host".into())) {
        return Some(host.clone());
    }
    None
}

fn yaml_h2_host(value: &YamlValue) -> Option<String> {
    let map = value.as_mapping()?;
    let h2 = map.get(YamlValue::String("h2-opts".into()))?;
    let h2_map = h2.as_mapping()?;
    let host = h2_map.get(YamlValue::String("host".into()))?;
    match host {
        YamlValue::String(s) => Some(s.clone()),
        YamlValue::Sequence(seq) => {
            if let Some(YamlValue::String(s)) = seq.first() {
                return Some(s.clone());
            }
            None
        }
        _ => None,
    }
}

fn yaml_key(key: &str) -> serde_yaml::Value {
    YamlValue::String(key.to_string())
}

fn encode_query(params: &BTreeMap<String, String>) -> String {
    // Common path/query characters that v2ray clients expect unencoded
    params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(k, URI_QUERY),
                percent_encoding::utf8_percent_encode(v, URI_QUERY)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

// ── Full Clash/Mihomo config generation ────────────────────────────────────

/// Generate a complete Clash/Mihomo YAML configuration from a list of
/// `V2Ray` share-link URIs. The output is a full Mihomo-compatible config
/// with DNS, country-based proxy groups, and routing rules — ready to
/// import directly into Clash Verge, `ClashTUI`, or any Mihomo client.
#[allow(clippy::unnecessary_wraps)]
pub fn generate_clash_config(uris: &[&str]) -> Result<String> {
    let mut proxy_values = Vec::new();
    let mut proxy_names = Vec::new();
    let mut seen_names = std::collections::HashMap::<String, usize>::new();

    for uri in uris {
        if let Ok(mut proxy_value) = uri_to_clash_proxy(uri) {
            // Deduplicate proxy names by appending (2), (3), etc.
            if let Some(name) = yaml_string(&proxy_value, &["name"]) {
                let count = seen_names.entry(name.clone()).or_insert(0);
                *count += 1;
                if *count > 1 {
                    let deduped = format!("{name} ({count})");
                    if let YamlValue::Mapping(map) = &mut proxy_value {
                        map.insert(
                            YamlValue::String("name".to_string()),
                            YamlValue::String(deduped.clone()),
                        );
                    }
                    proxy_names.push(deduped);
                } else {
                    proxy_names.push(name);
                }
                proxy_values.push(proxy_value);
            }
        }
    }

    if proxy_names.is_empty() {
        return Ok(minimal_clash_config());
    }

    // Build proper YAML sequence for proxies (each entry gets "- " prefix)
    let proxies_yaml = serde_yaml::to_string(&YamlValue::Sequence(proxy_values))?;
    // Remove the leading "---\n" that serde_yaml adds
    let proxies_yaml = proxies_yaml
        .strip_prefix("---\n")
        .unwrap_or(&proxies_yaml)
        .trim_end_matches('\n');

    let groups_yaml = generate_proxy_groups(&proxy_names);
    let rules_yaml = generate_clash_rules();

    Ok(format!(
        "{HEADER}\n\nproxies:\n{proxies_yaml}\n\n{groups_yaml}\n\n{rules_yaml}"
    ))
}

const HEADER: &str = r"mixed-port: 7890
allow-lan: false
mode: rule
log-level: info
external-controller: '127.0.0.1:9090'

dns:
  enable: true
  listen: 0.0.0.0:1053
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  default-nameserver:
    - 223.5.5.5
    - 8.8.8.8
  nameserver:
    - https://dns.alidns.com/dns-query
    - https://doh.pub/dns-query
  fallback:
    - https://1.1.1.1/dns-query
    - https://8.8.8.8/dns-query
  fallback-filter:
    geoip: true
    geoip-code: CN";

fn minimal_clash_config() -> String {
    let rules_yaml = generate_clash_rules();
    format!("{HEADER}\n\nproxies: []\n\n{rules_yaml}")
}

fn yaml_quote_name(name: &str) -> String {
    let needs_quoting = name.is_empty()
        || name.starts_with([' ', '-', '?', '"', '\''])
        || name.ends_with(' ')
        || name.contains(|c: char| YAML_NEEDS_QUOTE.contains(c));

    if !needs_quoting {
        return name.to_string();
    }
    let escaped = name
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

fn generate_proxy_groups(proxy_names: &[String]) -> String {
    // Build the auto group with all proxies (quote names for YAML safety)
    let auto_proxies = proxy_names
        .iter()
        .map(|n| format!("      - {}", yaml_quote_name(n)))
        .collect::<Vec<_>>()
        .join("\n");

    // Detect countries present in proxy names
    let countries = detect_countries(proxy_names);

    // Build per-country url-test groups with regex filter
    let country_groups = countries
        .iter()
        .map(|(_code, label, keywords)| {
            let filter_pattern = keywords_to_regex(keywords);
            format!(
                "  - name: {label}\n    type: url-test\n    url: https://www.gstatic.com/generate_204\n    interval: 300\n    tolerance: 150\n    include-all: true\n    filter: \"{filter_pattern}\""
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    // Country group names for the regions select list
    let country_refs = countries
        .iter()
        .map(|(_, label, _)| format!("      - {label}"))
        .collect::<Vec<_>>()
        .join("\n");

    let multiple_countries = countries.len() > 1;

    // Build the regions select group (only when multiple countries)
    let regions_group = if multiple_countries {
        format!(
            "\n\n  - name: 🌍 Regions\n    type: select\n    proxies:\n{country_refs}\n      - ♻️ Auto"
        )
    } else {
        String::new()
    };

    // Build the Manual group's proxy list
    let mut manual_proxies = vec!["      - ♻️ Auto".to_string()];
    if multiple_countries {
        manual_proxies.push("      - 🌍 Regions".to_string());
    }
    manual_proxies.push("      - DIRECT".to_string());
    let manual_proxies_yaml = manual_proxies.join("\n");

    // Separator before country groups (if any exist)
    let country_block = if country_groups.is_empty() {
        String::new()
    } else {
        format!("\n\n{country_groups}")
    };

    format!(
        r"proxy-groups:
  - name: 🚀 Manual
    type: select
    proxies:
{manual_proxies_yaml}

  - name: ♻️ Auto
    type: url-test
    url: https://www.gstatic.com/generate_204
    interval: 300
    tolerance: 150
    proxies:
{auto_proxies}{regions_group}{country_block}"
    )
}

/// Convert a list of keywords into a simple regex alternation pattern.
/// Example: `["HK", "Hong Kong", "港"]` → `(?i)(HK|Hong Kong|港)`
fn keywords_to_regex(keywords: &[&str]) -> String {
    if keywords.is_empty() {
        return String::new();
    }
    let alternation = keywords.join("|");
    format!("(?i)({alternation})")
}

fn generate_clash_rules() -> String {
    String::from(
        r"rules:
  - GEOIP,private,DIRECT,no-resolve
  - GEOIP,cn,DIRECT,no-resolve
  - DOMAIN-SUFFIX,google.com,🚀 Manual
  - DOMAIN-SUFFIX,googleapis.com,🚀 Manual
  - DOMAIN-SUFFIX,youtube.com,🚀 Manual
  - DOMAIN-SUFFIX,facebook.com,🚀 Manual
  - DOMAIN-SUFFIX,twitter.com,🚀 Manual
  - DOMAIN-SUFFIX,x.com,🚀 Manual
  - DOMAIN-SUFFIX,instagram.com,🚀 Manual
  - DOMAIN-SUFFIX,telegram.org,🚀 Manual
  - DOMAIN-SUFFIX,t.me,🚀 Manual
  - DOMAIN-SUFFIX,wikipedia.org,🚀 Manual
  - DOMAIN-SUFFIX,github.com,🚀 Manual
  - DOMAIN-SUFFIX,githubusercontent.com,🚀 Manual
  - DOMAIN-SUFFIX,openai.com,🚀 Manual
  - DOMAIN-KEYWORD,google,🚀 Manual
  - DOMAIN-KEYWORD,googleapis,🚀 Manual
  - MATCH,DIRECT",
    )
}

/// Country code → (emoji+label, match keywords)
type CountryDef = (&'static str, &'static str, &'static [&'static str]);

const COUNTRY_DEFS: &[CountryDef] = &[
    // East Asia
    ("HK", "🇭🇰 HK", &["HK", "Hong Kong", "港"]),
    ("JP", "🇯🇵 JP", &["JP", "Japan", "东京", "大阪", "日本"]),
    ("KR", "🇰🇷 KR", &["KR", "Korea", "韩国", "首尔"]),
    ("TW", "🇹🇼 TW", &["TW", "Taiwan", "台湾", "新北"]),
    ("MO", "🇲🇴 MO", &["MO", "Macau", "澳门"]),
    // Southeast Asia
    ("SG", "🇸🇬 SG", &["SG", "Singapore", "新加坡", "狮城"]),
    ("TH", "🇹🇭 TH", &["TH", "Thailand", "泰国", "曼谷"]),
    ("VN", "🇻🇳 VN", &["VN", "Vietnam", "越南", "胡志明"]),
    ("MY", "🇲🇾 MY", &["MY", "Malaysia", "马来西亚", "吉隆坡"]),
    ("PH", "🇵🇭 PH", &["PH", "Philippines", "菲律宾", "马尼拉"]),
    (
        "ID",
        "🇮🇩 ID",
        &["ID", "Indonesia", "印尼", "印度尼西亚", "雅加达"],
    ),
    // South Asia
    ("IN", "🇮🇳 IN", &["IN", "India", "印度", "孟买", "德里"]),
    ("PK", "🇵🇰 PK", &["PK", "Pakistan", "巴基斯坦"]),
    ("BD", "🇧🇩 BD", &["BD", "Bangladesh", "孟加拉"]),
    // North America
    (
        "US",
        "🇺🇸 US",
        &[
            "US",
            "USA",
            "美国",
            "硅谷",
            "洛杉矶",
            "波特兰",
            "达拉斯",
            "芝加哥",
            "西雅图",
            "纽约",
            "华盛顿",
            "旧金山",
            "迈阿密",
        ],
    ),
    (
        "CA",
        "🇨🇦 CA",
        &["CA", "Canada", "加拿大", "多伦多", "温哥华"],
    ),
    ("MX", "🇲🇽 MX", &["MX", "Mexico", "墨西哥"]),
    // Europe
    ("GB", "🇬🇧 UK", &["GB", "UK", "英国", "伦敦"]),
    (
        "DE",
        "🇩🇪 DE",
        &["DE", "Germany", "德国", "法兰克福", "柏林"],
    ),
    ("FR", "🇫🇷 FR", &["FR", "France", "法国", "巴黎"]),
    ("NL", "🇳🇱 NL", &["NL", "Netherlands", "荷兰", "阿姆斯特丹"]),
    ("IT", "🇮🇹 IT", &["IT", "Italy", "意大利", "米兰", "罗马"]),
    ("ES", "🇪🇸 ES", &["ES", "Spain", "西班牙", "马德里"]),
    ("PT", "🇵🇹 PT", &["PT", "Portugal", "葡萄牙", "里斯本"]),
    ("PL", "🇵🇱 PL", &["PL", "Poland", "波兰", "华沙"]),
    ("SE", "🇸🇪 SE", &["SE", "Sweden", "瑞典", "斯德哥尔摩"]),
    ("NO", "🇳🇴 NO", &["NO", "Norway", "挪威"]),
    ("DK", "🇩🇰 DK", &["DK", "Denmark", "丹麦", "哥本哈根"]),
    ("FI", "🇫🇮 FI", &["FI", "Finland", "芬兰", "赫尔辛基"]),
    ("CH", "🇨🇭 CH", &["CH", "Switzerland", "瑞士", "苏黎世"]),
    ("AT", "🇦🇹 AT", &["AT", "Austria", "奥地利", "维也纳"]),
    ("BE", "🇧🇪 BE", &["BE", "Belgium", "比利时"]),
    ("IE", "🇮🇪 IE", &["IE", "Ireland", "爱尔兰", "都柏林"]),
    ("RO", "🇷🇴 RO", &["RO", "Romania", "罗马尼亚"]),
    ("CZ", "🇨🇿 CZ", &["CZ", "Czech", "捷克", "布拉格"]),
    ("UA", "🇺🇦 UA", &["UA", "Ukraine", "乌克兰"]),
    ("BG", "🇧🇬 BG", &["BG", "Bulgaria", "保加利亚"]),
    // Oceania
    (
        "AU",
        "🇦🇺 AU",
        &["AU", "Australia", "澳大利亚", "悉尼", "墨尔本"],
    ),
    ("NZ", "🇳🇿 NZ", &["NZ", "New Zealand", "新西兰", "奥克兰"]),
    // Middle East
    ("TR", "🇹🇷 TR", &["TR", "Turkey", "土耳其", "伊斯坦布尔"]),
    ("IR", "🇮🇷 IR", &["IR", "Iran", "伊朗", "德黑兰"]),
    ("IL", "🇮🇱 IL", &["IL", "Israel", "以色列"]),
    ("AE", "🇦🇪 AE", &["AE", "UAE", "阿联酋", "迪拜"]),
    // Africa
    ("ZA", "🇿🇦 ZA", &["ZA", "South Africa", "南非"]),
    // South America
    ("BR", "🇧🇷 BR", &["BR", "Brazil", "巴西", "圣保罗", "里约"]),
    (
        "AR",
        "🇦🇷 AR",
        &["AR", "Argentina", "阿根廷", "布宜诺斯艾利斯"],
    ),
    // East Europe / CIS
    (
        "RU",
        "🇷🇺 RU",
        &["RU", "Russia", "俄罗斯", "莫斯科", "圣彼得堡"],
    ),
];

fn detect_countries(proxy_names: &[String]) -> Vec<CountryDef> {
    let mut present = Vec::new();
    for &(code, label, keywords) in COUNTRY_DEFS {
        let matched = proxy_names.iter().any(|name| {
            let lower = name.to_ascii_lowercase();
            keywords
                .iter()
                .any(|kw| lower.contains(&kw.to_ascii_lowercase()))
        });
        if matched {
            present.push((code, label, keywords));
        }
    }

    if present.is_empty() {
        present.push(("OTHER", "🌐 Other", &[]));
    }

    present
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vmess_uri() -> String {
        let json = r#"{"v":"2","ps":"Test VMess","add":"example.com","port":"443","id":"abc-123","aid":"0","net":"ws","type":"none","host":"example.com","path":"/ws","tls":"tls","sni":"example.com","fp":"chrome"}"#;
        format!("vmess://{}", STANDARD.encode(json))
    }

    fn sample_vless_uri() -> String {
        "vless://abc-123@example.com:443?security=tls&type=ws&path=/ws&host=example.com&sni=example.com&fp=chrome#Test%20VLESS".to_string()
    }

    fn sample_trojan_uri() -> String {
        "trojan://password123@example.com:443?security=tls&sni=example.com#Test%20Trojan"
            .to_string()
    }

    fn sample_ss_uri() -> String {
        let auth = STANDARD.encode("aes-256-gcm:mypassword");
        format!("ss://{auth}@example.com:8388#Test%20SS")
    }

    #[test]
    fn vmess_round_trip() {
        let uri = sample_vmess_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();
        let back = clash_proxy_to_uri(&proxy).unwrap();
        let orig = vmess_fields(&uri).unwrap();
        let round = vmess_fields(&back).unwrap();

        assert_eq!(orig.host, round.host);
        assert_eq!(orig.port, round.port);
        assert_eq!(orig.uuid, round.uuid);
        assert_eq!(orig.aid, round.aid);
    }

    #[test]
    fn vless_round_trip() {
        let uri = sample_vless_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();
        let back = clash_proxy_to_uri(&proxy).unwrap();
        let orig = standard_uri_fields(&uri).unwrap();
        let round = standard_uri_fields(&back).unwrap();

        assert_eq!(orig.host, round.host);
        assert_eq!(orig.port, round.port);
        assert_eq!(orig.username, round.username);
    }

    #[test]
    fn trojan_round_trip() {
        let uri = sample_trojan_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();
        let back = clash_proxy_to_uri(&proxy).unwrap();
        let orig = standard_uri_fields(&uri).unwrap();
        let round = standard_uri_fields(&back).unwrap();

        assert_eq!(orig.host, round.host);
        assert_eq!(orig.port, round.port);
        assert_eq!(orig.username, round.username);
    }

    #[test]
    fn ss_round_trip() {
        let uri = sample_ss_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();
        let back = clash_proxy_to_uri(&proxy).unwrap();
        let orig = shadowsocks_fields(&uri).unwrap();
        let round = shadowsocks_fields(&back).unwrap();

        assert_eq!(orig.host, round.host);
        assert_eq!(orig.port, round.port);
        assert_eq!(orig.method, round.method);
        assert_eq!(orig.password, round.password);
    }

    #[test]
    fn vmess_to_clash_fields() {
        let uri = sample_vmess_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();

        assert_eq!(yaml_string(&proxy, &["type"]).unwrap(), "vmess");
        assert_eq!(yaml_string(&proxy, &["server"]).unwrap(), "example.com");
        assert_eq!(yaml_u16(&proxy, &["port"]).unwrap(), 443);
        assert_eq!(yaml_string(&proxy, &["uuid"]).unwrap(), "abc-123");
        assert_eq!(yaml_string(&proxy, &["network"]).unwrap(), "ws");
    }

    #[test]
    fn clash_vmess_to_uri() {
        let yaml_str = r#"
name: "Test VMess"
type: vmess
server: example.com
port: 443
uuid: abc-123
alterId: 0
cipher: auto
tls: true
network: ws
ws-opts:
  path: /ws
  headers:
    Host: example.com
servername: example.com
client-fingerprint: chrome
"#;
        let proxy: YamlValue = serde_yaml::from_str(yaml_str).unwrap();
        let uri = clash_proxy_to_uri(&proxy).unwrap();

        assert!(uri.starts_with("vmess://"));
        let fields = vmess_fields(&uri).unwrap();
        assert_eq!(fields.host, "example.com");
        assert_eq!(fields.port, 443);
        assert_eq!(fields.uuid, "abc-123");
        assert_eq!(fields.net.as_deref(), Some("ws"));
    }

    #[test]
    fn clash_vless_to_uri() {
        let yaml_str = r#"
name: "Test VLESS"
type: vless
server: example.com
port: 443
uuid: abc-123
tls: true
network: ws
ws-opts:
  path: /ws
  headers:
    Host: example.com
servername: example.com
client-fingerprint: chrome
flow: xtls-rprx-vision
"#;
        let proxy: YamlValue = serde_yaml::from_str(yaml_str).unwrap();
        let uri = clash_proxy_to_uri(&proxy).unwrap();

        assert!(uri.starts_with("vless://"));
        assert!(uri.contains("abc-123@example.com:443"));
        assert!(uri.contains("security=tls"));
        assert!(uri.contains("flow=xtls-rprx-vision"));
    }

    #[test]
    fn clash_trojan_to_uri() {
        let yaml_str = r#"
name: "Test Trojan"
type: trojan
server: example.com
port: 443
password: mypassword
tls: true
servername: example.com
"#;
        let proxy: YamlValue = serde_yaml::from_str(yaml_str).unwrap();
        let uri = clash_proxy_to_uri(&proxy).unwrap();

        assert!(uri.starts_with("trojan://"));
        assert!(uri.contains("mypassword@example.com:443"));
    }

    #[test]
    fn clash_ss_to_uri() {
        let yaml_str = r#"
name: "Test SS"
type: ss
server: example.com
port: 8388
cipher: aes-256-gcm
password: mypassword
"#;
        let proxy: YamlValue = serde_yaml::from_str(yaml_str).unwrap();
        let uri = clash_proxy_to_uri(&proxy).unwrap();

        assert!(uri.starts_with("ss://"));
        let decoded = STANDARD
            .decode(
                uri.strip_prefix("ss://")
                    .unwrap()
                    .split('@')
                    .next()
                    .unwrap(),
            )
            .unwrap();
        let creds = String::from_utf8(decoded).unwrap();
        assert_eq!(creds, "aes-256-gcm:mypassword");
    }

    #[test]
    fn detect_clash_proxy_type() {
        let yaml_str = r#"
name: "test"
type: vmess
server: example.com
port: 443
uuid: test
"#;
        let proxy: YamlValue = serde_yaml::from_str(yaml_str).unwrap();
        let uri = clash_proxy_to_uri(&proxy).unwrap();
        assert!(uri.starts_with("vmess://"));
    }

    #[test]
    fn round_trip_vless_reality() {
        let uri = "vless://abc-123@example.com:443?security=reality&pbk=XYZpubkey&sid=abc123&fp=chrome&sni=example.com#Reality%20Node";
        let proxy = uri_to_clash_proxy(uri).unwrap();
        let back = clash_proxy_to_uri(&proxy).unwrap();

        assert!(back.contains("pbk=XYZpubkey"));
        assert!(back.contains("sid=abc123"));
        assert!(back.contains("security=reality"));
    }

    #[test]
    fn generate_clash_config_vmess() {
        let uri = sample_vmess_uri();
        let config = generate_clash_config(&[&uri]).unwrap();

        assert!(config.contains("mixed-port: 7890"));
        assert!(config.contains("mode: rule"));
        assert!(config.contains("external-controller:"));
        assert!(config.contains("dns:"));
        assert!(config.contains("proxies:"));
        assert!(config.contains("type: vmess"));
        assert!(config.contains("proxy-groups:"));
        assert!(config.contains("type: url-test"));
        assert!(config.contains("rules:"));
        assert!(config.contains("MATCH,"));
    }

    #[test]
    #[allow(clippy::similar_names)]
    fn generate_clash_config_multiple_proxies() {
        let vmess_link = sample_vmess_uri();
        let vless_link = sample_vless_uri();
        let config = generate_clash_config(&[&vmess_link, &vless_link]).unwrap();

        // Should have two proxy entries
        assert!(config.contains("type: vmess"));
        assert!(config.contains("type: vless"));

        // proxy-group should reference both
        let group_section = config
            .split("proxy-groups:")
            .nth(1)
            .unwrap()
            .split("rules:")
            .next()
            .unwrap();
        assert!(group_section.contains('-'));
    }

    #[test]
    fn generate_clash_config_empty() {
        let config = generate_clash_config(&[]).unwrap();
        assert!(config.contains("mixed-port: 7890"));
        assert!(config.contains("mode: rule"));
        assert!(config.contains("proxies: []"));
        assert!(!config.contains("proxy-groups:"));
    }

    #[test]
    fn extract_proxy_name_works() {
        let yaml: YamlValue =
            serde_yaml::from_str("name: \"Test Node\"\ntype: vmess\nserver: example.com\n")
                .unwrap();
        assert_eq!(yaml_string(&yaml, &["name"]).unwrap(), "Test Node");
    }

    // ── SSR tests ──────────────────────────────────────────────────────

    fn sample_ssr_uri() -> String {
        let password_b64 = STANDARD.encode("mypassword");
        let payload = format!("example.com:443:origin:aes-256-cfb:plain:{password_b64}");
        let encoded = STANDARD.encode(&payload);
        format!("ssr://{encoded}/?remark=Test%20SSR")
    }

    #[test]
    fn ssr_to_clash_fields() {
        let uri = sample_ssr_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();

        assert_eq!(yaml_string(&proxy, &["type"]).unwrap(), "ssr");
        assert_eq!(yaml_string(&proxy, &["server"]).unwrap(), "example.com");
        assert_eq!(yaml_u16(&proxy, &["port"]).unwrap(), 443);
        assert_eq!(yaml_string(&proxy, &["cipher"]).unwrap(), "aes-256-cfb");
        assert_eq!(yaml_string(&proxy, &["password"]).unwrap(), "mypassword");
        assert_eq!(yaml_string(&proxy, &["protocol"]).unwrap(), "origin");
        assert_eq!(yaml_string(&proxy, &["obfs"]).unwrap(), "plain");
    }

    #[test]
    fn ssr_round_trip() {
        let uri = sample_ssr_uri();
        let proxy = uri_to_clash_proxy(&uri).unwrap();
        let back = clash_proxy_to_uri(&proxy).unwrap();

        assert!(back.starts_with("ssr://"));
        // Decode the base64 payload (strip query string first)
        let rest = back.strip_prefix("ssr://").unwrap();
        let encoded = rest.find("/?").map_or(rest, |pos| &rest[..pos]);
        let payload = STANDARD.decode(encoded).unwrap();
        let payload_str = String::from_utf8(payload).unwrap();
        assert!(payload_str.contains("example.com:443"));
        assert!(payload_str.contains("aes-256-cfb"));
        assert!(payload_str.contains("origin"));
    }

    // ── Hysteria2 tests ────────────────────────────────────────────────

    #[test]
    fn hysteria2_to_clash_fields() {
        let uri =
            "hysteria2://mypassword@example.com:443?sni=example.com&insecure=1#Test%20Hysteria2";
        let proxy = uri_to_clash_proxy(uri).unwrap();

        assert_eq!(yaml_string(&proxy, &["type"]).unwrap(), "hysteria2");
        assert_eq!(yaml_string(&proxy, &["server"]).unwrap(), "example.com");
        assert_eq!(yaml_u16(&proxy, &["port"]).unwrap(), 443);
        assert_eq!(yaml_string(&proxy, &["password"]).unwrap(), "mypassword");
        assert_eq!(yaml_string(&proxy, &["sni"]).unwrap(), "example.com");
        assert!(yaml_bool(&proxy, &["skip-cert-verify"]));
    }

    #[test]
    fn hy2_alias_parses() {
        let uri = "hy2://mypass@example.com:8443#Hy2%20Test";
        let proxy = uri_to_clash_proxy(uri).unwrap();

        assert_eq!(yaml_string(&proxy, &["type"]).unwrap(), "hysteria2");
        assert_eq!(yaml_string(&proxy, &["password"]).unwrap(), "mypass");
        assert_eq!(yaml_u16(&proxy, &["port"]).unwrap(), 8443);
    }

    #[test]
    fn hysteria2_round_trip() {
        let uri = "hysteria2://mypassword@example.com:443?sni=example.com#Test%20H2";
        let proxy = uri_to_clash_proxy(uri).unwrap();
        let back = clash_hysteria2_to_uri(&proxy, "example.com", 443, "Test H2");

        assert!(back.starts_with("hysteria2://"));
        assert!(back.contains("example.com:443"));
        assert!(back.contains("sni=example.com"));
    }

    // ── TUIC tests ─────────────────────────────────────────────────────

    #[test]
    fn tuic_to_clash_fields() {
        let uri = "tuic://myuuid:mypassword@example.com:443?sni=example.com&alpn=h3&insecure=1#Test%20TUIC";
        let proxy = uri_to_clash_proxy(uri).unwrap();

        assert_eq!(yaml_string(&proxy, &["type"]).unwrap(), "tuic");
        assert_eq!(yaml_string(&proxy, &["server"]).unwrap(), "example.com");
        assert_eq!(yaml_u16(&proxy, &["port"]).unwrap(), 443);
        assert_eq!(yaml_string(&proxy, &["uuid"]).unwrap(), "myuuid");
        assert_eq!(yaml_string(&proxy, &["password"]).unwrap(), "mypassword");
        assert_eq!(yaml_string(&proxy, &["sni"]).unwrap(), "example.com");
        assert!(yaml_bool(&proxy, &["skip-cert-verify"]));
        assert_eq!(yaml_alpn(&proxy).unwrap(), "h3");
    }

    #[test]
    fn tuic_round_trip() {
        let uri = "tuic://myuuid:mypassword@example.com:443?sni=example.com#TUIC%20Node";
        let proxy = uri_to_clash_proxy(uri).unwrap();
        let back = clash_tuic_to_uri(&proxy, "example.com", 443, "TUIC Node").unwrap();

        assert!(back.starts_with("tuic://"));
        assert!(back.contains("example.com:443"));
        assert!(back.contains("sni=example.com"));
    }
}
