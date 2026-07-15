use std::collections::BTreeMap;

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
    match method.to_ascii_lowercase().as_str() {
        "ss" => Err(anyhow!("unsupported Shadowsocks method: ss")),
        "chacha20-poly1305" => Ok("chacha20-ietf-poly1305".to_string()),
        "xchacha20-poly1305" => Ok("xchacha20-ietf-poly1305".to_string()),
        other => Ok(other.to_string()),
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
        query_parts.push(format!("plugin={plugin}{opts}"));
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
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
    // Common path/query characters that v2ray clients expect unencoded
    const URI_QUERY: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b'/');
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
            }
            proxy_values.push(proxy_value);
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
    format!("{HEADER}\n")
}

fn generate_proxy_groups(proxy_names: &[String]) -> String {
    let all_names: Vec<&str> = proxy_names.iter().map(String::as_str).collect();

    // Build the auto group with all proxies
    let auto_proxies = all_names
        .iter()
        .map(|n| format!("      - {n}"))
        .collect::<Vec<_>>()
        .join("\n");

    // Detect countries present in proxy names
    let countries = detect_countries(proxy_names);

    // Build country groups with regex filter from keywords
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

    // Country group names for the manual select list
    let country_refs = countries
        .iter()
        .map(|(_, label, _)| format!("      - {label}"))
        .collect::<Vec<_>>()
        .join("\n");

    let multiple_countries = countries.len() > 1;

    let country_select = if multiple_countries {
        format!(
            "\n  - name: 🌍 Regions\n    type: select\n    proxies:\n{country_refs}\n      - ♻️ Auto"
        )
    } else {
        String::new()
    };

    format!(
        r"proxy-groups:
  - name: 🚀 Manual
    type: select
    proxies:
      - ♻️ Auto{country_select}
      - DIRECT

  - name: ♻️ Auto
    type: url-test
    url: https://www.gstatic.com/generate_204
    interval: 300
    tolerance: 150
    proxies:
{auto_proxies}

{country_groups}"
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
  - MATCH,DIRECT",
    )
}

/// Country code → (emoji+label, match keywords)
type CountryDef = (&'static str, &'static str, &'static [&'static str]);

fn detect_countries(proxy_names: &[String]) -> Vec<CountryDef> {
    let all_defs: Vec<CountryDef> = vec![
        ("HK", "🇭🇰 HK", &["HK", "Hong Kong", "港"]),
        ("JP", "🇯🇵 JP", &["JP", "Japan", "东京", "大阪", "日本"]),
        ("SG", "🇸🇬 SG", &["SG", "Singapore", "新加坡", "狮城"]),
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
            ],
        ),
        ("TW", "🇹🇼 TW", &["TW", "Taiwan", "台湾", "新北"]),
        (
            "DE",
            "🇩🇪 DE",
            &["DE", "Germany", "德国", "法兰克福", "柏林"],
        ),
        ("FR", "🇫🇷 FR", &["FR", "France", "法国", "巴黎"]),
        ("GB", "🇬🇧 UK", &["GB", "UK", "英国", "伦敦"]),
        ("KR", "🇰🇷 KR", &["KR", "Korea", "韩国", "首尔"]),
        ("NL", "🇳🇱 NL", &["NL", "Netherlands", "荷兰", "阿姆斯特丹"]),
        (
            "CA",
            "🇨🇦 CA",
            &["CA", "Canada", "加拿大", "多伦多", "温哥华"],
        ),
        (
            "AU",
            "🇦🇺 AU",
            &["AU", "Australia", "澳大利亚", "悉尼", "墨尔本"],
        ),
        ("IN", "🇮🇳 IN", &["IN", "India", "印度", "孟买", "德里"]),
        ("IR", "🇮🇷 IR", &["IR", "Iran", "伊朗", "德黑兰"]),
    ];

    let mut present = Vec::new();
    for &(code, label, keywords) in &all_defs {
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
        assert!(!config.contains("proxies:"));
        assert!(!config.contains("proxy-groups:"));
    }

    #[test]
    fn extract_proxy_name_works() {
        let yaml: YamlValue =
            serde_yaml::from_str("name: \"Test Node\"\ntype: vmess\nserver: example.com\n")
                .unwrap();
        assert_eq!(yaml_string(&yaml, &["name"]).unwrap(), "Test Node");
    }
}
