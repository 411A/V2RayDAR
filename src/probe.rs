use std::{
    cmp::Ordering,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use futures_util::{StreamExt, stream};
use reqwest::Proxy;
use serde_json::{Map, Value, json};
use tokio::{
    fs,
    net::{TcpListener, TcpStream},
    process::Command,
    time::Instant,
};
use url::Url;

use crate::{
    config::{ProbeConfig, ProbeMode},
    model::{Candidate, RankedConfig},
};

pub async fn probe_candidates(
    candidates: Vec<Candidate>,
    config: &ProbeConfig,
) -> Vec<RankedConfig> {
    if config.mode == ProbeMode::Active && !sing_box_available(&config.sing_box_path).await {
        return rank_configs(
            candidates
                .into_iter()
                .map(|candidate| {
                    failed_config(
                        candidate,
                        "active_http",
                        format!(
                            "sing-box executable '{}' was not found or did not run; install sing-box or set probe.sing_box_path",
                            config.sing_box_path
                        ),
                    )
                })
                .collect(),
        );
    }

    let ranked = stream::iter(candidates.into_iter().map(|candidate| async move {
        match config.mode {
            ProbeMode::Active => probe_active(candidate, config).await,
            ProbeMode::Tcp => {
                probe_tcp(candidate, Duration::from_millis(config.connect_timeout_ms)).await
            }
        }
    }))
    .buffer_unordered(config.concurrency)
    .collect::<Vec<_>>()
    .await;

    rank_configs(ranked)
}

fn rank_configs(mut ranked: Vec<RankedConfig>) -> Vec<RankedConfig> {
    ranked.sort_by(compare_ranked);
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }

    ranked
}

async fn probe_tcp(candidate: Candidate, timeout: Duration) -> RankedConfig {
    let target = (candidate.endpoint.host.as_str(), candidate.endpoint.port);
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, TcpStream::connect(target)).await;

    let (reachable, latency_ms, error) = match result {
        Ok(Ok(_stream)) => (true, Some(started.elapsed().as_millis()), None),
        Ok(Err(err)) => (false, None, Some(err.to_string())),
        Err(_) => (
            false,
            None,
            Some(format!("timed out after {} ms", timeout.as_millis())),
        ),
    };

    RankedConfig {
        rank: 0,
        stability_count: 0,
        id: candidate.id,
        source: candidate.source,
        priority: candidate.priority,
        protocol: candidate.protocol,
        name: candidate.name,
        endpoint: candidate.endpoint,
        uri: candidate.uri,
        reachable,
        validation: "tcp_connect".to_string(),
        latency_ms,
        http_status: None,
        download_mbps: None,
        download_bytes: None,
        error,
    }
}

async fn probe_active(candidate: Candidate, config: &ProbeConfig) -> RankedConfig {
    let started = Instant::now();
    let result = probe_active_inner(&candidate, config).await;

    match result {
        Ok(active) => RankedConfig {
            rank: 0,
            stability_count: 0,
            id: candidate.id,
            source: candidate.source,
            priority: candidate.priority,
            protocol: candidate.protocol,
            name: candidate.name,
            endpoint: candidate.endpoint,
            uri: candidate.uri,
            reachable: true,
            validation: "active_http".to_string(),
            latency_ms: Some(started.elapsed().as_millis()),
            http_status: Some(active.http_status),
            download_mbps: active.download_mbps,
            download_bytes: active.download_bytes,
            error: None,
        },
        Err(err) => failed_config(candidate, "active_http", err.to_string()),
    }
}

struct ActiveProbeSuccess {
    http_status: u16,
    download_mbps: Option<f64>,
    download_bytes: Option<usize>,
}

async fn probe_active_inner(
    candidate: &Candidate,
    config: &ProbeConfig,
) -> Result<ActiveProbeSuccess> {
    let outbound = sing_box_outbound_from_share_link(&candidate.uri)?;
    let port = reserve_local_port().await?;
    let config_path = write_sing_box_config(&candidate.id, port, outbound).await?;
    let cleanup_path = config_path.clone();

    let mut child = Command::new(&config.sing_box_path)
        .arg("run")
        .arg("-c")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start sing-box using '{}'", config.sing_box_path))?;

    let result = async {
        wait_for_local_proxy(
            &mut child,
            port,
            Duration::from_millis(config.startup_timeout_ms),
        )
        .await?;
        let proxy_url = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.active_timeout_ms))
            .proxy(Proxy::all(&proxy_url)?)
            .build()?;

        let response = client.get(&config.test_url).send().await?;
        let status = response.status().as_u16();
        if !config.accepted_statuses.contains(&status) {
            return Err(anyhow!(
                "active HTTP probe returned status {}; accepted statuses are {:?}",
                status,
                config.accepted_statuses
            ));
        }

        let (download_mbps, download_bytes) = if let Some(download_url) = &config.download_url {
            let measurement =
                measure_download(&client, download_url, config.download_bytes_limit).await?;
            (Some(measurement.mbps), Some(measurement.bytes))
        } else {
            (None, None)
        };

        Ok(ActiveProbeSuccess {
            http_status: status,
            download_mbps,
            download_bytes,
        })
    }
    .await;

    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = fs::remove_file(cleanup_path).await;

    result
}

struct DownloadMeasurement {
    mbps: f64,
    bytes: usize,
}

async fn measure_download(
    client: &reqwest::Client,
    download_url: &str,
    bytes_limit: usize,
) -> Result<DownloadMeasurement> {
    let started = Instant::now();
    let response = client
        .get(download_url)
        .header(
            reqwest::header::RANGE,
            format!("bytes=0-{}", bytes_limit.saturating_sub(1)),
        )
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    let elapsed = started.elapsed().as_secs_f64();
    if elapsed == 0.0 || body.is_empty() {
        return Err(anyhow!("download probe returned no measurable data"));
    }

    let measured_bytes = body.len().min(bytes_limit);
    Ok(DownloadMeasurement {
        mbps: (measured_bytes as f64 * 8.0) / elapsed / 1_000_000.0,
        bytes: measured_bytes,
    })
}

async fn sing_box_available(path: &str) -> bool {
    Command::new(path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn reserve_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    Ok(listener.local_addr()?.port())
}

async fn wait_for_local_proxy(
    child: &mut tokio::process::Child,
    port: u16,
    timeout: Duration,
) -> Result<()> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(anyhow!("sing-box exited before proxy was ready: {status}"));
        }

        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return Ok(());
        }

        if started.elapsed() >= timeout {
            return Err(anyhow!(
                "sing-box local proxy did not become ready within {} ms",
                timeout.as_millis()
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn write_sing_box_config(id: &str, port: u16, outbound: Value) -> Result<std::path::PathBuf> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "v2raydar-sing-box-{}-{id}-{timestamp}.json",
        std::process::id()
    ));
    let config = json!({
        "log": {
            "disabled": true
        },
        "inbounds": [
            {
                "type": "mixed",
                "tag": "mixed-in",
                "listen": "127.0.0.1",
                "listen_port": port
            }
        ],
        "outbounds": [
            outbound
        ],
        "route": {
            "final": "proxy",
            "auto_detect_interface": true
        }
    });

    fs::write(&path, serde_json::to_vec_pretty(&config)?).await?;
    Ok(path)
}

fn failed_config(candidate: Candidate, validation: &str, error: String) -> RankedConfig {
    RankedConfig {
        rank: 0,
        stability_count: 0,
        id: candidate.id,
        source: candidate.source,
        priority: candidate.priority,
        protocol: candidate.protocol,
        name: candidate.name,
        endpoint: candidate.endpoint,
        uri: candidate.uri,
        reachable: false,
        validation: validation.to_string(),
        latency_ms: None,
        http_status: None,
        download_mbps: None,
        download_bytes: None,
        error: Some(error),
    }
}

fn compare_ranked(left: &RankedConfig, right: &RankedConfig) -> Ordering {
    right
        .reachable
        .cmp(&left.reachable)
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| {
            left.latency_ms
                .unwrap_or(u128::MAX)
                .cmp(&right.latency_ms.unwrap_or(u128::MAX))
        })
        .then_with(|| {
            right
                .download_mbps
                .partial_cmp(&left.download_mbps)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.protocol.cmp(&right.protocol))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.uri.cmp(&right.uri))
}

fn sing_box_outbound_from_share_link(uri: &str) -> Result<Value> {
    let lower = uri.to_ascii_lowercase();
    if lower.starts_with("vmess://") {
        vmess_outbound(uri)
    } else if lower.starts_with("vless://") {
        standard_outbound(uri, StandardProtocol::Vless)
    } else if lower.starts_with("trojan://") {
        standard_outbound(uri, StandardProtocol::Trojan)
    } else if lower.starts_with("ss://") {
        shadowsocks_outbound(uri)
    } else if lower.starts_with("hysteria2://") || lower.starts_with("hy2://") {
        standard_outbound(uri, StandardProtocol::Hysteria2)
    } else if lower.starts_with("tuic://") {
        standard_outbound(uri, StandardProtocol::Tuic)
    } else {
        Err(anyhow!(
            "active sing-box probe does not support this URI scheme"
        ))
    }
}

#[derive(Clone, Copy)]
enum StandardProtocol {
    Vless,
    Trojan,
    Hysteria2,
    Tuic,
}

fn standard_outbound(uri: &str, protocol: StandardProtocol) -> Result<Value> {
    let url = Url::parse(uri)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("share link has no host"))?
        .to_string();
    let port = url
        .port()
        .ok_or_else(|| anyhow!("share link has no port"))?;
    let params = query_map(&url);

    match protocol {
        StandardProtocol::Vless => {
            let uuid = percent_decode(url.username());
            if uuid.is_empty() {
                return Err(anyhow!("VLESS link has no UUID"));
            }

            let mut outbound = base_outbound("vless", &host, port);
            outbound.insert("uuid".to_string(), json!(uuid));
            if let Some(flow) = first_param(&params, &["flow"]) {
                outbound.insert("flow".to_string(), json!(flow));
            }
            if let Some(tls) = tls_config(&params, false, &host) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_config(&params) {
                outbound.insert("transport".to_string(), transport);
            }
            Ok(Value::Object(outbound))
        }
        StandardProtocol::Trojan => {
            let password = percent_decode(url.username());
            if password.is_empty() {
                return Err(anyhow!("Trojan link has no password"));
            }

            let mut outbound = base_outbound("trojan", &host, port);
            outbound.insert("password".to_string(), json!(password));
            if let Some(tls) = tls_config(&params, true, &host) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_config(&params) {
                outbound.insert("transport".to_string(), transport);
            }
            Ok(Value::Object(outbound))
        }
        StandardProtocol::Hysteria2 => {
            let password = percent_decode(url.username());
            if password.is_empty() {
                return Err(anyhow!("Hysteria2 link has no password"));
            }

            let mut outbound = base_outbound("hysteria2", &host, port);
            outbound.insert("password".to_string(), json!(password));
            outbound.insert(
                "tls".to_string(),
                tls_config(&params, true, &host).unwrap_or_else(|| json!({"enabled": true})),
            );
            if let Some(obfs) = first_param(&params, &["obfs"]) {
                let mut obfs_config = Map::new();
                obfs_config.insert("type".to_string(), json!(obfs));
                if let Some(password) = first_param(&params, &["obfs-password", "obfs_password"]) {
                    obfs_config.insert("password".to_string(), json!(password));
                }
                outbound.insert("obfs".to_string(), Value::Object(obfs_config));
            }
            Ok(Value::Object(outbound))
        }
        StandardProtocol::Tuic => {
            let uuid = percent_decode(url.username());
            let password = url.password().map(percent_decode).unwrap_or_default();
            if uuid.is_empty() {
                return Err(anyhow!("TUIC link has no UUID"));
            }

            let mut outbound = base_outbound("tuic", &host, port);
            outbound.insert("uuid".to_string(), json!(uuid));
            outbound.insert("password".to_string(), json!(password));
            outbound.insert(
                "tls".to_string(),
                tls_config(&params, true, &host).unwrap_or_else(|| json!({"enabled": true})),
            );
            if let Some(value) = first_param(&params, &["congestion_control", "congestion"]) {
                outbound.insert("congestion_control".to_string(), json!(value));
            }
            if let Some(value) = first_param(&params, &["udp_relay_mode", "udp-relay-mode"]) {
                outbound.insert("udp_relay_mode".to_string(), json!(value));
            }
            Ok(Value::Object(outbound))
        }
    }
}

fn vmess_outbound(uri: &str) -> Result<Value> {
    let payload = uri
        .strip_prefix("vmess://")
        .ok_or_else(|| anyhow!("invalid VMess URI"))?;
    let decoded = decode_base64_to_string(payload)
        .ok_or_else(|| anyhow!("VMess URI payload is not valid base64 UTF-8"))?;
    let json: Value = serde_json::from_str(&decoded).context("VMess payload is not JSON")?;

    let host = json_string(&json, &["add", "address"])
        .ok_or_else(|| anyhow!("VMess payload has no server address"))?;
    let port = json_u16(&json, &["port"]).ok_or_else(|| anyhow!("VMess payload has no port"))?;
    let uuid = json_string(&json, &["id"]).ok_or_else(|| anyhow!("VMess payload has no UUID"))?;

    let mut outbound = base_outbound("vmess", &host, port);
    outbound.insert("uuid".to_string(), json!(uuid));
    outbound.insert(
        "security".to_string(),
        json!(json_string(&json, &["scy", "security"]).unwrap_or_else(|| "auto".to_string())),
    );
    outbound.insert(
        "alter_id".to_string(),
        json!(json_u64(&json, &["aid", "alterId"]).unwrap_or(0)),
    );

    let tls_enabled = json_string(&json, &["tls"])
        .map(|value| value.eq_ignore_ascii_case("tls"))
        .unwrap_or(false);
    if tls_enabled {
        let tls = tls_config_from_values(
            true,
            json_string(&json, &["sni"]).or_else(|| json_string(&json, &["host"])),
            json_string(&json, &["alpn"]),
            json_string(&json, &["fp"]),
            None,
            None,
            false,
        );
        outbound.insert("tls".to_string(), tls);
    }

    let mut params = std::collections::BTreeMap::new();
    if let Some(network) = json_string(&json, &["net"]) {
        params.insert("type".to_string(), network);
    }
    if let Some(path) = json_string(&json, &["path"]) {
        params.insert("path".to_string(), path);
    }
    if let Some(host) = json_string(&json, &["host"]) {
        params.insert("host".to_string(), host);
    }
    if let Some(transport) = transport_config(&params) {
        outbound.insert("transport".to_string(), transport);
    }

    Ok(Value::Object(outbound))
}

fn shadowsocks_outbound(uri: &str) -> Result<Value> {
    let body = uri
        .strip_prefix("ss://")
        .ok_or_else(|| anyhow!("invalid Shadowsocks URI"))?;
    let (without_fragment, _) = split_once(body, '#');
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

    let mut outbound = base_outbound("shadowsocks", &host, port);
    outbound.insert("method".to_string(), json!(method));
    outbound.insert("password".to_string(), json!(password));

    if let Some(query) = query {
        let params = query_pairs(query);
        if let Some(plugin) = first_param(&params, &["plugin"]) {
            let (plugin_name, plugin_opts) = split_once(&plugin, ';');
            outbound.insert("plugin".to_string(), json!(plugin_name));
            if let Some(plugin_opts) = plugin_opts {
                outbound.insert("plugin_opts".to_string(), json!(plugin_opts));
            }
        }
    }

    Ok(Value::Object(outbound))
}

fn base_outbound(protocol: &str, host: &str, port: u16) -> Map<String, Value> {
    let mut outbound = Map::new();
    outbound.insert("type".to_string(), json!(protocol));
    outbound.insert("tag".to_string(), json!("proxy"));
    outbound.insert("server".to_string(), json!(host));
    outbound.insert("server_port".to_string(), json!(port));
    outbound
}

fn tls_config(
    params: &std::collections::BTreeMap<String, String>,
    default_enabled: bool,
    host: &str,
) -> Option<Value> {
    let security = first_param(params, &["security", "tls"]).unwrap_or_default();
    let reality_key = first_param(params, &["pbk", "public_key", "reality_pbk"]);
    let enabled = default_enabled
        || security.eq_ignore_ascii_case("tls")
        || security.eq_ignore_ascii_case("reality")
        || reality_key.is_some();

    if !enabled || security.eq_ignore_ascii_case("none") {
        return None;
    }

    Some(tls_config_from_values(
        true,
        first_param(params, &["sni", "serverName", "peer"]).or_else(|| Some(host.to_string())),
        first_param(params, &["alpn"]),
        first_param(params, &["fp", "fingerprint"]),
        reality_key,
        first_param(params, &["sid", "short_id"]),
        first_param(params, &["allowInsecure", "insecure", "skip-cert-verify"])
            .map(|value| truthy(&value))
            .unwrap_or(false),
    ))
}

fn tls_config_from_values(
    enabled: bool,
    server_name: Option<String>,
    alpn: Option<String>,
    fingerprint: Option<String>,
    reality_public_key: Option<String>,
    reality_short_id: Option<String>,
    insecure: bool,
) -> Value {
    let mut tls = Map::new();
    tls.insert("enabled".to_string(), json!(enabled));
    if let Some(server_name) = server_name.filter(|value| !value.is_empty()) {
        tls.insert("server_name".to_string(), json!(server_name));
    }
    if insecure {
        tls.insert("insecure".to_string(), json!(true));
    }
    if let Some(alpn) = alpn.filter(|value| !value.is_empty()) {
        let values = alpn
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if !values.is_empty() {
            tls.insert("alpn".to_string(), json!(values));
        }
    }
    if let Some(fingerprint) = fingerprint.filter(|value| !value.is_empty()) {
        tls.insert(
            "utls".to_string(),
            json!({
                "enabled": true,
                "fingerprint": fingerprint
            }),
        );
    }
    if let Some(public_key) = reality_public_key.filter(|value| !value.is_empty()) {
        let mut reality = Map::new();
        reality.insert("enabled".to_string(), json!(true));
        reality.insert("public_key".to_string(), json!(public_key));
        if let Some(short_id) = reality_short_id {
            reality.insert("short_id".to_string(), json!(short_id));
        }
        tls.insert("reality".to_string(), Value::Object(reality));
    }

    Value::Object(tls)
}

fn transport_config(params: &std::collections::BTreeMap<String, String>) -> Option<Value> {
    let transport_type = first_param(params, &["type", "net", "network"])?.to_ascii_lowercase();
    let path = first_param(params, &["path"]).unwrap_or_default();
    let host = first_param(params, &["host"]);

    match transport_type.as_str() {
        "tcp" | "" => None,
        "ws" | "websocket" => {
            let mut transport = Map::new();
            transport.insert("type".to_string(), json!("ws"));
            if !path.is_empty() {
                transport.insert("path".to_string(), json!(path));
            }
            if let Some(host) = host.filter(|value| !value.is_empty()) {
                transport.insert(
                    "headers".to_string(),
                    json!({
                        "Host": host
                    }),
                );
            }
            Some(Value::Object(transport))
        }
        "grpc" => {
            let mut transport = Map::new();
            transport.insert("type".to_string(), json!("grpc"));
            if let Some(service_name) = first_param(params, &["serviceName", "service_name"]) {
                transport.insert("service_name".to_string(), json!(service_name));
            }
            Some(Value::Object(transport))
        }
        "h2" | "http" => {
            let mut transport = Map::new();
            transport.insert("type".to_string(), json!("http"));
            if !path.is_empty() {
                transport.insert("path".to_string(), json!(path));
            }
            if let Some(host) = host.filter(|value| !value.is_empty()) {
                transport.insert("host".to_string(), json!([host]));
            }
            Some(Value::Object(transport))
        }
        "httpupgrade" => {
            let mut transport = Map::new();
            transport.insert("type".to_string(), json!("httpupgrade"));
            if !path.is_empty() {
                transport.insert("path".to_string(), json!(path));
            }
            if let Some(host) = host.filter(|value| !value.is_empty()) {
                transport.insert("host".to_string(), json!(host));
            }
            Some(Value::Object(transport))
        }
        unsupported => Some(json!({ "type": unsupported })),
    }
}

fn query_map(url: &Url) -> std::collections::BTreeMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn query_pairs(query: &str) -> std::collections::BTreeMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn first_param(
    params: &std::collections::BTreeMap<String, String>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| params.get(*key).filter(|value| !value.is_empty()).cloned())
}

fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y"
    )
}

fn json_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .filter(|value| !value.is_empty())
    })
}

fn json_u16(value: &Value, keys: &[&str]) -> Option<u16> {
    json_u64(value, keys).and_then(|value| u16::try_from(value).ok())
}

fn json_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    })
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

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .to_string()
}

fn decode_base64_to_string(value: &str) -> Option<String> {
    let normalized = value.trim().replace(['\r', '\n'], "");
    let padded = pad_base64(&normalized);
    for candidate in [normalized, padded] {
        for engine in [
            &base64::engine::general_purpose::STANDARD,
            &base64::engine::general_purpose::URL_SAFE,
            &base64::engine::general_purpose::STANDARD_NO_PAD,
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ] {
            if let Ok(decoded) = base64::Engine::decode(engine, candidate.as_bytes())
                && let Ok(text) = String::from_utf8(decoded)
            {
                return Some(text);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    #[test]
    fn builds_vless_reality_outbound() {
        let outbound = sing_box_outbound_from_share_link(
            "vless://uuid@example.com:443?security=reality&sni=www.example.com&pbk=pub&sid=abcd&fp=chrome&type=grpc&serviceName=svc#node",
        )
        .expect("vless outbound");

        assert_eq!(outbound["type"], "vless");
        assert_eq!(outbound["server"], "example.com");
        assert_eq!(outbound["tls"]["reality"]["public_key"], "pub");
        assert_eq!(outbound["transport"]["type"], "grpc");
    }

    #[test]
    fn builds_vmess_ws_outbound() {
        let vmess = STANDARD.encode(
            r#"{"v":"2","ps":"demo","add":"example.com","port":"443","id":"uuid","scy":"auto","net":"ws","host":"cdn.example.com","path":"/ws","tls":"tls","sni":"cdn.example.com"}"#,
        );
        let outbound =
            sing_box_outbound_from_share_link(&format!("vmess://{vmess}")).expect("vmess outbound");

        assert_eq!(outbound["type"], "vmess");
        assert_eq!(outbound["server_port"], 443);
        assert_eq!(outbound["transport"]["type"], "ws");
        assert_eq!(outbound["tls"]["server_name"], "cdn.example.com");
    }

    #[test]
    fn builds_shadowsocks_outbound() {
        let outbound =
            sing_box_outbound_from_share_link("ss://YWVzLTI1Ni1nY206cGFzcw@example.net:8388#SS")
                .expect("shadowsocks outbound");

        assert_eq!(outbound["type"], "shadowsocks");
        assert_eq!(outbound["method"], "aes-256-gcm");
        assert_eq!(outbound["password"], "pass");
    }
}
