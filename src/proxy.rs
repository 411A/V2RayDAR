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
    net::TcpStream,
    process::Command,
    sync::{Mutex, RwLock},
    time,
};
use tracing::{error, info, warn};

use crate::{
    config::ProxyConfig,
    constants::{LOCALHOST_IP, SING_BOX_CLEANUP_TIMEOUT, SING_BOX_CONFIG_FILE_PREFIX},
    model::RankedConfig,
    probe::{sing_box_outbound_from_share_link, sing_box_version_at_least},
};

pub type SharedProxy = Arc<Mutex<PersistentProxy>>;

pub struct PersistentProxy {
    sing_box_path: String,
    config: ProxyConfig,
    state: Arc<RwLock<ProxyState>>,
    process: Mutex<Option<ManagedProcess>>,
}

struct ProxyState {
    active_config_uri: Option<String>,
    active_config_name: Option<String>,
    active_config_country: Option<String>,
    running: bool,
    last_health_check: Option<Instant>,
    last_health_ok: bool,
}

struct ManagedProcess {
    child: tokio::process::Child,
    config_path: PathBuf,
}

impl PersistentProxy {
    pub fn new(config: ProxyConfig, sing_box_path: String) -> Self {
        Self {
            sing_box_path,
            config,
            state: Arc::new(RwLock::new(ProxyState {
                active_config_uri: None,
                active_config_name: None,
                active_config_country: None,
                running: false,
                last_health_check: None,
                last_health_ok: false,
            })),
            process: Mutex::new(None),
        }
    }

    pub async fn update(&self, ranked: &[RankedConfig]) {
        let best = match ranked.iter().find(|c| c.reachable) {
            Some(config) => config,
            None => {
                warn!("proxy: no reachable configs available");
                return;
            }
        };

        let current_uri = {
            let state = self.state.read().await;
            state.active_config_uri.clone()
        };

        if current_uri.as_deref() == Some(&best.uri) {
            return;
        }

        info!(
            name = %best.name,
            protocol = %best.protocol,
            latency_ms = ?best.latency_ms,
            "proxy: switching to new config"
        );

        if let Err(err) = self.start_with_config(best).await {
            error!(error = %err, "proxy: failed to start");
            return;
        }

        let mut state = self.state.write().await;
        state.active_config_uri = Some(best.uri.clone());
        state.active_config_name = Some(best.name.clone());
        state.active_config_country = best.country_code.clone();
    }

    async fn start_with_config(&self, config: &RankedConfig) -> Result<()> {
        self.stop().await;

        let outbound = sing_box_outbound_from_share_link(&config.uri)
            .context("failed to convert config to sing-box outbound")?;

        let listen = if self.config.discoverable {
            "0.0.0.0"
        } else {
            LOCALHOST_IP
        };

        let config_json = build_sing_box_config(&outbound, self.config.port, listen);
        let config_path = write_proxy_config(&config_json).await?;

        let child = Command::new(&self.sing_box_path)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("failed to start sing-box proxy process")?;

        let managed = ManagedProcess { child, config_path };

        wait_for_port(self.config.port, Duration::from_secs(5))
            .await
            .context("sing-box proxy did not start in time")?;

        {
            let mut state = self.state.write().await;
            state.running = true;
            state.last_health_check = None;
            state.last_health_ok = false;
        }

        *self.process.lock().await = Some(managed);

        info!(
            port = self.config.port,
            name = %config.name,
            "proxy: started"
        );

        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        let proxy_url = format!("http://{}:{}", LOCALHOST_IP, self.config.port);
        let Ok(proxy) = Proxy::all(&proxy_url) else {
            warn!("proxy: invalid proxy URL for health check");
            return false;
        };
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .proxy(proxy)
            .build()
        {
            Ok(client) => client,
            Err(err) => {
                warn!(error = %err, "proxy: health check client build failed");
                return false;
            }
        };

        let result = client.get(&self.config.health_check_url).send().await;

        let ok = match result {
            Ok(resp) => resp.status().is_success() || resp.status().as_u16() == 204,
            Err(err) => {
                warn!(error = %err, "proxy: health check failed");
                false
            }
        };

        let mut state = self.state.write().await;
        state.last_health_check = Some(Instant::now());
        state.last_health_ok = ok;

        ok
    }

    pub async fn failover(&self, ranked: &[RankedConfig]) -> Result<()> {
        let current_uri = {
            let state = self.state.read().await;
            state.active_config_uri.clone()
        };

        let candidates: Vec<&RankedConfig> = ranked
            .iter()
            .filter(|c| c.reachable && current_uri.as_deref() != Some(&c.uri))
            .collect();

        for candidate in candidates {
            info!(
                name = %candidate.name,
                "proxy: attempting failover"
            );
            if self.start_with_config(candidate).await.is_ok() && self.health_check().await {
                info!(name = %candidate.name, "proxy: failover succeeded");
                return Ok(());
            }
        }

        {
            let mut state = self.state.write().await;
            state.running = false;
        }

        Err(anyhow!("proxy: all failover candidates exhausted"))
    }

    pub async fn stop(&self) {
        let mut process = self.process.lock().await;
        if let Some(mut managed) = process.take() {
            let _ = managed.child.start_kill();
            let _ = time::timeout(SING_BOX_CLEANUP_TIMEOUT, managed.child.wait()).await;
            let _ = fs::remove_file(&managed.config_path).await;
            info!("proxy: stopped");
        }

        let mut state = self.state.write().await;
        state.running = false;
    }

    pub async fn shutdown(&self) {
        self.stop().await;
    }

    #[allow(dead_code)]
    pub async fn snapshot(&self) -> ProxySnapshot {
        let state = self.state.read().await;
        ProxySnapshot {
            active_config: state.active_config_name.clone(),
            running: state.running,
            port: if state.running {
                Some(self.config.port)
            } else {
                None
            },
            discoverable: self.config.discoverable,
            country: state.active_config_country.clone(),
        }
    }
}

pub fn spawn_health_loop(proxy: SharedProxy, ranked: Arc<RwLock<Vec<RankedConfig>>>) {
    tokio::spawn(async move {
        let interval_secs = {
            let p = proxy.lock().await;
            p.config.health_check_interval_seconds
        };

        let mut ticker = time::interval(Duration::from_secs(interval_secs));
        ticker.tick().await;

        loop {
            ticker.tick().await;

            let running = {
                let p = proxy.lock().await;
                let state = p.state.read().await;
                state.running
            };

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

            warn!("proxy: health check failed, attempting failover");
            let ranked_guard = ranked.read().await;
            let p = proxy.lock().await;
            if let Err(err) = p.failover(&ranked_guard).await {
                error!(error = %err, "proxy: failover failed");
            }
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

async fn wait_for_port(port: u16, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let addr = format!("{}:{port}", LOCALHOST_IP);

    while Instant::now() < deadline {
        match TcpStream::connect(&addr).await {
            Ok(_) => return Ok(()),
            Err(_) => time::sleep(Duration::from_millis(50)).await,
        }
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
        obj.insert("tag".to_string(), json!("proxy-0"));
    }

    let direct_outbound = json!({
        "type": "direct",
        "tag": "direct-out"
    });

    let dns_servers = if sing_box_version_at_least(1, 12, 0) {
        json!([
            { "tag": "dns-direct", "type": "udp", "server": "8.8.8.8" },
            { "tag": "dns-fallback", "type": "udp", "server": "1.1.1.1" }
        ])
    } else {
        json!([
            { "tag": "dns-direct", "address": "8.8.8.8", "strategy": "prefer_ipv4", "detour": "direct-out" },
            { "tag": "dns-fallback", "address": "1.1.1.1", "strategy": "prefer_ipv4", "detour": "direct-out" }
        ])
    };

    let route = if sing_box_version_at_least(1, 12, 0) {
        json!({
            "rules": [{ "protocol": "bittorrent", "action": "reject" }],
            "final": "proxy-0",
            "default_domain_resolver": "dns-direct"
        })
    } else {
        json!({
            "rules": [{ "protocol": "bittorrent", "action": "reject" }],
            "final": "proxy-0"
        })
    };

    json!({
        "log": { "level": "warning" },
        "dns": { "servers": dns_servers },
        "inbounds": [{
            "type": "mixed",
            "tag": "proxy-in",
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
        assert_eq!(config["outbounds"][0]["tag"], "proxy-0");
        assert_eq!(config["outbounds"][1]["type"], "direct");
        assert_eq!(config["route"]["final"], "proxy-0");
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
}
