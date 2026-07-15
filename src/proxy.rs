use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use reqwest::Proxy;
use serde_json::{Value, json};
use tokio::{
    fs,
    io::AsyncReadExt,
    net::TcpStream,
    process::Command,
    sync::{Mutex, RwLock},
    time,
};
use tracing::{error, info, warn};

use crate::{
    config::ProxyConfig,
    constants::{
        LOCALHOST_IP, PROXY_DNS_FALLBACK, PROXY_DNS_PRIMARY, PROXY_FAILOVER_COOLDOWN,
        PROXY_HEALTH_CHECK_TIMEOUT, PROXY_MAX_CONSECUTIVE_FAILURES, PROXY_MAX_RECENTLY_FAILED_KEYS,
        PROXY_PORT_POLL_INTERVAL, PROXY_SING_BOX_TAG_DIRECT, PROXY_SING_BOX_TAG_DNS_DIRECT,
        PROXY_SING_BOX_TAG_DNS_FALLBACK, PROXY_SING_BOX_TAG_DNS_PROXY, PROXY_SING_BOX_TAG_INBOUND,
        PROXY_SING_BOX_TAG_OUTBOUND, PROXY_STARTUP_TIMEOUT, SING_BOX_CLEANUP_TIMEOUT,
        SING_BOX_CONFIG_FILE_PREFIX,
    },
    model::{ProgressEvent, RankedConfig},
    probe::{sing_box_outbound_from_share_link, sing_box_version_at_least},
};

pub type SharedProxy = Arc<Mutex<PersistentProxy>>;

pub struct PersistentProxy {
    sing_box_path: String,
    state: Arc<RwLock<ProxyState>>,
    process: Mutex<Option<ManagedProcess>>,
    events: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
}

struct ProxyState {
    active_config_uri: Option<String>,
    active_config_name: Option<String>,
    active_config_country: Option<String>,
    running: bool,
    consecutive_failures: u32,
    last_health_check: Option<Instant>,
    last_health_ok: bool,
    last_failover: Option<Instant>,
    failed_config_keys: Vec<String>,
    proxy_config: ProxyConfig,
}

struct ManagedProcess {
    child: tokio::process::Child,
    config_path: PathBuf,
    stderr_task: tokio::task::JoinHandle<()>,
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        // Cancel the stderr reader task before cleaning up
        self.stderr_task.abort();
        // Sync file removal — safe because temp files are small and infrequent.
        // Async removal happens in stop(); this is the fallback for Drop paths
        // (e.g. startup failure, panic, or kill_on_drop).
        let _ = std::fs::remove_file(&self.config_path);
    }
}

impl PersistentProxy {
    pub fn new(
        config: ProxyConfig,
        sing_box_path: String,
        events: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
    ) -> Self {
        Self {
            sing_box_path,
            state: Arc::new(RwLock::new(ProxyState {
                active_config_uri: None,
                active_config_name: None,
                active_config_country: None,
                running: false,
                consecutive_failures: 0,
                last_health_check: None,
                last_health_ok: false,
                last_failover: None,
                failed_config_keys: Vec::new(),
                proxy_config: config,
            })),
            process: Mutex::new(None),
            events,
        }
    }

    fn emit_log(&self, message: String) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ProgressEvent::LiveLog(message));
        }
    }

    /// Update the proxy with the best config from the ranked list.
    /// Takes the current `ProxyConfig` so it reacts to TUI config changes
    /// (enable/disable, port, discoverable) without restart.
    pub async fn update(&self, config: &ProxyConfig, ranked: &[RankedConfig]) {
        // Sync the latest config into state so the health loop can read it
        {
            let mut state = self.state.write().await;
            state.proxy_config = config.clone();
            // Clear recently-failed blacklist on each refresh cycle so configs
            // get a fresh chance to be tried after network conditions change.
            state.failed_config_keys.clear();
        }

        if !config.enabled {
            let was_running = {
                let state = self.state.read().await;
                state.running
            } || self.is_process_alive().await;
            if was_running {
                info!("proxy: disabled in config, stopping");
                self.emit_log("proxy: disabled".into());
                self.stop().await;
            }
            return;
        }

        let Some(best) = ranked.iter().find(|c| c.reachable) else {
            warn!("proxy: no reachable configs available");
            self.emit_log("proxy: no reachable configs".into());
            return;
        };

        let (should_switch, reason) = {
            let state = self.state.read().await;
            match &state.active_config_uri {
                None => (true, "starting"),
                Some(uri) if uri != &best.uri => (true, "new config"),
                Some(_) => {
                    let running = state.running;
                    let failures = state.consecutive_failures;
                    drop(state);

                    if !running || !self.is_process_alive().await {
                        (true, "restarted")
                    } else if failures >= PROXY_MAX_CONSECUTIVE_FAILURES {
                        (true, "failover")
                    } else {
                        (false, "")
                    }
                }
            }
        };

        if !should_switch {
            return;
        }

        info!(
            name = %best.name,
            protocol = %best.protocol,
            latency_ms = ?best.latency_ms,
            reason,
            "proxy: switching to new config"
        );

        if let Err(err) = self.start_with_config(best).await {
            error!(error = %err, "proxy: failed to start");
            self.emit_log(format!("proxy: failed → {err}"));
            return;
        }

        self.emit_log(format!(
            "proxy: {reason} → {} (port {})",
            best.name, config.port
        ));

        let mut state = self.state.write().await;
        state.active_config_uri = Some(best.uri.clone());
        state.active_config_name = Some(best.name.clone());
        state.active_config_country.clone_from(&best.country_code);
        state.consecutive_failures = 0;
    }

    /// Check if the managed sing-box process is still alive.
    /// Returns `true` if the process is running, `false` if it has exited.
    /// Cleans up the dead process entry when detected.
    async fn is_process_alive(&self) -> bool {
        let mut process = self.process.lock().await;
        let Some(ref mut managed) = *process else {
            return false;
        };

        match managed.child.try_wait() {
            Ok(Some(status)) => {
                // Process exited — capture stderr before cleanup
                let stderr_msg = read_child_stderr(&mut managed.child).await;
                warn!(
                    status = %status,
                    stderr = stderr_msg.as_deref().unwrap_or("(no output)"),
                    "proxy: sing-box process exited"
                );
                let _ = fs::remove_file(&managed.config_path).await;
                *process = None;
                drop(process);
                let mut state = self.state.write().await;
                state.running = false;
                false
            }
            Ok(None) => true,
            Err(err) => {
                warn!(error = %err, "proxy: failed to check process status");
                false
            }
        }
    }

    async fn start_with_config(&self, config: &RankedConfig) -> Result<()> {
        self.stop().await;

        let outbound = sing_box_outbound_from_share_link(&config.uri)
            .context("failed to convert config to sing-box outbound")?;

        let current_config = {
            let state = self.state.read().await;
            state.proxy_config.clone()
        };

        let listen = if current_config.discoverable {
            "0.0.0.0"
        } else {
            LOCALHOST_IP
        };

        let config_json = build_sing_box_config(&outbound, current_config.port, listen);
        let config_path = write_proxy_config(&config_json).await?;

        let mut child = Command::new(&self.sing_box_path)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("failed to start sing-box proxy process")?;

        // Spawn background stderr reader to capture sing-box logs
        let stderr = child.stderr.take();
        let stderr_task = tokio::spawn(async move {
            if let Some(mut stderr) = stderr {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(&mut stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        // Log sing-box stderr lines at appropriate levels
                        if trimmed.contains("error") || trimmed.contains("fatal") {
                            error!(target: "sing-box", "{trimmed}");
                        } else if trimmed.contains("warn") {
                            warn!(target: "sing-box", "{trimmed}");
                        } else {
                            info!(target: "sing-box", "{trimmed}");
                        }
                    }
                }
            }
        });

        // Wait for port — if this fails, the Drop impl on ManagedProcess
        // cleans up the config file, and kill_on_drop kills the process.
        wait_for_port(current_config.port, PROXY_STARTUP_TIMEOUT)
            .await
            .map_err(|err| {
                let stderr_msg = child.block_on_stderr();
                if stderr_msg.is_empty() {
                    err
                } else {
                    anyhow!("{err}: sing-box stderr: {stderr_msg}")
                }
            })?;

        let managed = ManagedProcess {
            child,
            config_path,
            stderr_task,
        };

        {
            let mut state = self.state.write().await;
            state.running = true;
            state.consecutive_failures = 0;
            state.last_health_check = None;
            state.last_health_ok = false;
        }

        *self.process.lock().await = Some(managed);

        info!(
            port = current_config.port,
            name = %config.name,
            "proxy: started"
        );

        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        if !self.is_process_alive().await {
            return false;
        }

        let port = {
            let state = self.state.read().await;
            state.proxy_config.port
        };

        let proxy_url = format!("http://{LOCALHOST_IP}:{port}");
        let Ok(proxy) = Proxy::all(&proxy_url) else {
            warn!("proxy: invalid proxy URL for health check");
            return false;
        };
        let Ok(client) = reqwest::Client::builder()
            .timeout(PROXY_HEALTH_CHECK_TIMEOUT)
            .proxy(proxy)
            .build()
        else {
            warn!("proxy: health check client build failed");
            return false;
        };

        let health_url = {
            let state = self.state.read().await;
            state.proxy_config.health_check_url.clone()
        };

        // Try primary URL first; if it fails, try fallback URLs
        let fallback_urls = [
            "https://1.1.1.1",
            "https://cloudflare.com",
            "https://api.ipify.org?format=json",
            "https://httpbin.org/ip",
        ];

        let ok = if let Ok(resp) = client.get(&health_url).send().await {
            resp.status().is_success() || resp.status().as_u16() == 204
        } else {
            // Primary failed - try fallbacks
            let mut any_ok = false;
            for fallback in &fallback_urls {
                if let Ok(resp) = client.get(*fallback).send().await
                    && (resp.status().is_success() || resp.status().as_u16() == 204)
                {
                    any_ok = true;
                    break;
                }
            }
            if !any_ok {
                warn!(
                    primary = %health_url,
                    "proxy: all health check URLs failed"
                );
            }
            any_ok
        };

        let mut state = self.state.write().await;
        state.last_health_check = Some(Instant::now());
        state.last_health_ok = ok;
        if ok {
            state.consecutive_failures = 0;
        } else {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        }

        ok
    }

    pub async fn failover(&self, ranked: &[RankedConfig]) -> Result<()> {
        let (current_uri, last_failover, failed_keys) = {
            let state = self.state.read().await;
            (
                state.active_config_uri.clone(),
                state.last_failover,
                state.failed_config_keys.clone(),
            )
        };

        // Failover cooldown: wait between failovers to avoid rapid cycling
        if let Some(last) = last_failover {
            let elapsed = last.elapsed();
            if elapsed < PROXY_FAILOVER_COOLDOWN {
                info!(
                    remaining_ms = (PROXY_FAILOVER_COOLDOWN - elapsed).as_millis(),
                    "proxy: failover cooldown active, waiting"
                );
                time::sleep(PROXY_FAILOVER_COOLDOWN - elapsed).await;
            }
        }

        // Try configs that haven't failed recently
        let candidates: Vec<&RankedConfig> = ranked
            .iter()
            .filter(|c| {
                c.reachable
                    && current_uri.as_deref() != Some(&c.uri)
                    && !failed_keys.contains(&c.dedup_key)
            })
            .collect();

        // If all candidates recently failed, clear the blacklist and retry
        let candidates = if candidates.is_empty() {
            warn!("proxy: all configs recently failed, clearing blacklist");
            self.emit_log("proxy: blacklist cleared".into());
            {
                let mut state = self.state.write().await;
                state.failed_config_keys.clear();
            }
            ranked
                .iter()
                .filter(|c| c.reachable && current_uri.as_deref() != Some(&c.uri))
                .collect()
        } else {
            candidates
        };

        for candidate in candidates {
            info!(name = %candidate.name, "proxy: attempting failover");
            if self.start_with_config(candidate).await.is_ok() && self.health_check().await {
                let port = {
                    let state = self.state.read().await;
                    state.proxy_config.port
                };
                info!(name = %candidate.name, "proxy: failover succeeded");
                self.emit_log(format!(
                    "proxy: failover → {} (port {})",
                    candidate.name, port
                ));
                {
                    let mut state = self.state.write().await;
                    state.active_config_uri = Some(candidate.uri.clone());
                    state.active_config_name = Some(candidate.name.clone());
                    state
                        .active_config_country
                        .clone_from(&candidate.country_code);
                    state.consecutive_failures = 0;
                    state.last_failover = Some(Instant::now());
                }
                return Ok(());
            }
            // Mark this config as failed
            let mut state = self.state.write().await;
            if state.failed_config_keys.len() < PROXY_MAX_RECENTLY_FAILED_KEYS {
                state.failed_config_keys.push(candidate.dedup_key.clone());
            }
        }

        {
            let mut state = self.state.write().await;
            state.running = false;
        }

        self.emit_log("proxy: failover exhausted".into());
        Err(anyhow!("proxy: all failover candidates exhausted"))
    }

    pub async fn stop(&self) {
        {
            let mut process = self.process.lock().await;
            if let Some(mut managed) = process.take() {
                let _ = managed.child.start_kill();
                let _ = time::timeout(SING_BOX_CLEANUP_TIMEOUT, managed.child.wait()).await;
                let _ = fs::remove_file(&managed.config_path).await;
                info!("proxy: stopped");
                self.emit_log("proxy: stopped".into());
            }
        }

        let mut state = self.state.write().await;
        state.running = false;
    }

    pub async fn shutdown(&self) {
        self.stop().await;
    }

    pub async fn snapshot(&self) -> ProxySnapshot {
        let state = self.state.read().await;
        ProxySnapshot {
            active_config: state.active_config_name.clone(),
            running: state.running,
            port: if state.running {
                Some(state.proxy_config.port)
            } else {
                None
            },
            discoverable: state.proxy_config.discoverable,
            country: state.active_config_country.clone(),
        }
    }
}

pub fn spawn_health_loop(proxy: SharedProxy, ranked: Arc<RwLock<Vec<RankedConfig>>>) {
    tokio::spawn(async move {
        let mut last_interval = 0u64;

        loop {
            // Read interval from state — reactive to config changes
            let (running, consecutive_failures, interval) = {
                let p = proxy.lock().await;
                let state = p.state.read().await;
                let result = (
                    state.running,
                    state.consecutive_failures,
                    state.proxy_config.health_check_interval_seconds,
                );
                drop(state);
                drop(p);
                result
            };

            // Restart ticker if interval changed (e.g. user edited config)
            if interval != last_interval && interval > 0 {
                last_interval = interval;
            }

            if last_interval == 0 {
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }

            time::sleep(Duration::from_secs(last_interval)).await;

            if !running {
                continue;
            }

            let health_ok = {
                let p = proxy.lock().await;
                p.health_check().await
            };

            if health_ok {
                continue;
            }

            if consecutive_failures < PROXY_MAX_CONSECUTIVE_FAILURES {
                let next = consecutive_failures + 1;
                warn!(
                    failures = next,
                    max = PROXY_MAX_CONSECUTIVE_FAILURES,
                    "proxy: health check failed, waiting for more failures before failover"
                );
                proxy.lock().await.emit_log(format!(
                    "proxy: health fail {next}/{PROXY_MAX_CONSECUTIVE_FAILURES}"
                ));
                continue;
            }

            warn!("proxy: health check failed, attempting failover");
            let ranked_snapshot = ranked.read().await.clone();
            let p = proxy.lock().await;
            if let Err(err) = p.failover(&ranked_snapshot).await {
                error!(error = %err, "proxy: failover failed");
                p.emit_log(format!("proxy: failover failed: {err}"));
            }
            drop(p);
        }
    });
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProxySnapshot {
    pub active_config: Option<String>,
    pub running: bool,
    pub port: Option<u16>,
    pub discoverable: bool,
    pub country: Option<String>,
}

/// Read remaining stderr from a child process (non-blocking attempt).
async fn read_child_stderr(child: &mut tokio::process::Child) -> Option<String> {
    let mut stderr = child.stderr.take()?;
    let mut buf = String::new();
    let _ = tokio::time::timeout(Duration::from_millis(100), stderr.read_to_string(&mut buf)).await;
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Blocking stderr read for use in sync-like contexts (e.g. error mapping).
trait ReadChildStderr {
    fn block_on_stderr(&mut self) -> String;
}

impl ReadChildStderr for tokio::process::Child {
    fn block_on_stderr(&mut self) -> String {
        let Some(mut stderr) = self.stderr.take() else {
            return String::new();
        };
        let mut buf = String::new();
        // Best-effort: if it's not ready in 100ms, return what we have
        tokio::runtime::Handle::current().block_on(async {
            let _ =
                tokio::time::timeout(Duration::from_millis(100), stderr.read_to_string(&mut buf))
                    .await;
        });
        buf.trim().to_string()
    }
}

async fn wait_for_port(port: u16, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let addr = format!("{LOCALHOST_IP}:{port}");

    while Instant::now() < deadline {
        if TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        time::sleep(PROXY_PORT_POLL_INTERVAL).await;
    }

    Err(anyhow!(
        "port {port} did not become available within {timeout:?}"
    ))
}

async fn write_proxy_config(config: &Value) -> Result<PathBuf> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "{SING_BOX_CONFIG_FILE_PREFIX}-proxy-{}-{timestamp}.json",
        std::process::id()
    ));
    fs::write(&path, serde_json::to_vec_pretty(config)?).await?;
    Ok(path)
}

fn build_sing_box_config(outbound: &Value, port: u16, listen: &str) -> Value {
    let mut outbound = outbound.clone();
    if let Some(obj) = outbound.as_object_mut() {
        obj.insert("tag".to_string(), json!(PROXY_SING_BOX_TAG_OUTBOUND));
    }

    let direct_outbound = json!({
        "type": "direct",
        "tag": PROXY_SING_BOX_TAG_DIRECT
    });

    let dns_servers = if sing_box_version_at_least(1, 12, 0) {
        json!([
            { "tag": PROXY_SING_BOX_TAG_DNS_DIRECT, "type": "udp", "server": PROXY_DNS_PRIMARY },
            { "tag": PROXY_SING_BOX_TAG_DNS_FALLBACK, "type": "udp", "server": PROXY_DNS_FALLBACK },
            { "tag": PROXY_SING_BOX_TAG_DNS_PROXY, "type": "udp", "server": PROXY_DNS_PRIMARY, "detour": PROXY_SING_BOX_TAG_OUTBOUND }
        ])
    } else {
        json!([
            { "tag": PROXY_SING_BOX_TAG_DNS_DIRECT, "address": PROXY_DNS_PRIMARY, "strategy": "prefer_ipv4", "detour": PROXY_SING_BOX_TAG_DIRECT },
            { "tag": PROXY_SING_BOX_TAG_DNS_FALLBACK, "address": PROXY_DNS_FALLBACK, "strategy": "prefer_ipv4", "detour": PROXY_SING_BOX_TAG_DIRECT },
            { "tag": PROXY_SING_BOX_TAG_DNS_PROXY, "address": PROXY_DNS_PRIMARY, "strategy": "prefer_ipv4", "detour": PROXY_SING_BOX_TAG_OUTBOUND }
        ])
    };

    let route = if sing_box_version_at_least(1, 12, 0) {
        json!({
            "rules": [{ "protocol": "bittorrent", "action": "reject" }],
            "final": PROXY_SING_BOX_TAG_OUTBOUND,
            "default_domain_resolver": PROXY_SING_BOX_TAG_DNS_DIRECT
        })
    } else {
        json!({
            "rules": [{ "protocol": "bittorrent", "action": "reject" }],
            "final": PROXY_SING_BOX_TAG_OUTBOUND
        })
    };

    json!({
        "log": { "level": "warning" },
        "dns": { "servers": dns_servers },
        "inbounds": [{
            "type": "mixed",
            "tag": PROXY_SING_BOX_TAG_INBOUND,
            "listen": listen,
            "listen_port": port
        }],
        "outbounds": [outbound, direct_outbound],
        "route": route
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_config_basic() {
        let outbound = json!({
            "type": "vless",
            "settings": {
                "vnext": [{
                    "address": "example.com",
                    "port": 443,
                    "users": [{ "id": "test-uuid" }]
                }]
            }
        });

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1");

        assert_eq!(config["inbounds"][0]["listen_port"], 27910);
        assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(config["inbounds"][0]["type"], "mixed");
        assert_eq!(
            config["outbounds"][0]["tag"].as_str().unwrap(),
            PROXY_SING_BOX_TAG_OUTBOUND
        );
        assert_eq!(config["outbounds"][1]["type"], "direct");
        assert_eq!(
            config["route"]["final"].as_str().unwrap(),
            PROXY_SING_BOX_TAG_OUTBOUND
        );
    }

    #[test]
    fn build_config_discoverable() {
        let outbound = json!({
            "type": "vmess",
            "settings": {
                "vnext": [{
                    "address": "1.2.3.4",
                    "port": 443,
                    "users": [{ "id": "test" }]
                }]
            }
        });

        let config = build_sing_box_config(&outbound, 10808, "0.0.0.0");
        assert_eq!(config["inbounds"][0]["listen"], "0.0.0.0");
    }

    #[test]
    fn build_config_bittorrent_blocked() {
        let outbound = json!({
            "type": "shadowsocks",
            "settings": {
                "servers": [{
                    "address": "example.com",
                    "port": 443,
                    "method": "aes-256-gcm",
                    "password": "test"
                }]
            }
        });

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1");
        let rules = config["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["protocol"] == json!("bittorrent")));
    }

    #[test]
    fn build_config_has_proxy_dns() {
        let outbound = json!({
            "type": "vless",
            "settings": {
                "vnext": [{
                    "address": "example.com",
                    "port": 443,
                    "users": [{ "id": "test-uuid" }]
                }]
            }
        });

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1");
        let dns_servers = config["dns"]["servers"].as_array().unwrap();
        let has_proxy_dns = dns_servers
            .iter()
            .any(|s| s["tag"].as_str() == Some(PROXY_SING_BOX_TAG_DNS_PROXY));
        assert!(
            has_proxy_dns,
            "dns-proxy server should be present for outbound resolution"
        );
    }

    #[test]
    fn build_config_uses_centralized_dns_constants() {
        let outbound = json!({
            "type": "vless",
            "settings": {
                "vnext": [{
                    "address": "example.com",
                    "port": 443,
                    "users": [{ "id": "test-uuid" }]
                }]
            }
        });

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1");
        let dns_servers = config["dns"]["servers"].as_array().unwrap();

        let (server_key, _addr_key) = if sing_box_version_at_least(1, 12, 0) {
            ("server", "server")
        } else {
            ("address", "detour")
        };

        let primary = dns_servers
            .iter()
            .find(|s| s["tag"].as_str() == Some(PROXY_SING_BOX_TAG_DNS_DIRECT))
            .expect("dns-direct should exist");
        assert_eq!(
            primary[server_key].as_str().unwrap(),
            PROXY_DNS_PRIMARY,
            "primary DNS should use centralized constant"
        );

        let fallback = dns_servers
            .iter()
            .find(|s| s["tag"].as_str() == Some(PROXY_SING_BOX_TAG_DNS_FALLBACK))
            .expect("dns-fallback should exist");
        assert_eq!(
            fallback[server_key].as_str().unwrap(),
            PROXY_DNS_FALLBACK,
            "fallback DNS should use centralized constant"
        );

        if !sing_box_version_at_least(1, 12, 0) {
            assert_eq!(
                primary["detour"].as_str().unwrap(),
                PROXY_SING_BOX_TAG_DIRECT,
                "old-format DNS direct should detour to direct-out"
            );
        }
    }
}
