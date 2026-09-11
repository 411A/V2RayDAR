use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use reqwest::Proxy;
use serde::Deserialize;
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
        LOCALHOST_IP, PROXY_CLASH_API_TIMEOUT, PROXY_CLASH_CONTROLLER_PORT_ATTEMPTS,
        PROXY_CLASH_UNREACHABLE_RETRY_POLLS, PROXY_DNS_FALLBACK, PROXY_DNS_PRIMARY,
        PROXY_FAILOVER_COOLDOWN, PROXY_HEALTH_CHECK_BODY_BYTES, PROXY_HEALTH_CHECK_TIMEOUT,
        PROXY_HEALTH_FULL_BPS, PROXY_HEALTH_LIGHT_TIMEOUT, PROXY_HEALTH_MIN_BPS,
        PROXY_MAX_CONSECUTIVE_FAILURES, PROXY_MAX_RECENTLY_FAILED_KEYS, PROXY_MIN_SWITCH_INTERVAL,
        PROXY_PORT_POLL_INTERVAL, PROXY_SING_BOX_TAG_DIRECT, PROXY_SING_BOX_TAG_DNS_DIRECT,
        PROXY_SING_BOX_TAG_DNS_FALLBACK, PROXY_SING_BOX_TAG_DNS_PROXY, PROXY_SING_BOX_TAG_INBOUND,
        PROXY_SING_BOX_TAG_OUTBOUND, PROXY_SLOW_CHECK_WARN, PROXY_STARTUP_TIMEOUT,
        PROXY_STARVATION_MIN_AGE, PROXY_STARVATION_MIN_UPLOAD_BYTES,
        PROXY_STARVATION_POLL_INTERVAL, SING_BOX_CLEANUP_TIMEOUT, SING_BOX_CONFIG_FILE_PREFIX,
    },
    model::{ProgressEvent, RankedConfig, RuntimeState},
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
    /// Degraded pool: every reachable config has failed, so the floor drops
    /// to the absolute minimum until a switch serves a new config. Never
    /// lower — below 1KB/s nothing interactive survives anyway.
    degraded: bool,
    /// Last config switch (update or failover): post-cycle updates hold the
    /// current config inside `PROXY_MIN_SWITCH_INTERVAL` instead of flapping
    /// on ranking jitter.
    last_switch: Option<Instant>,
    failed_config_keys: Vec<String>,
    proxy_config: ProxyConfig,
    manual_proxy_uri: Option<String>,
    /// Clash API controller of the running sing-box (localhost-only).
    /// `None` when no free port was found at start: the proxy works
    /// normally, only starvation detection stays off.
    clash_controller: Option<SocketAddr>,
    /// Random per-start bearer secret for the controller. Never logged.
    clash_secret: Option<String>,
}

/// Clash API controller allocation for one proxy start: localhost listener
/// plus a fresh random secret.
#[derive(Debug, Clone)]
struct ClashApi {
    port: u16,
    secret: String,
}

/// Brief shared snapshot of proxy state. The proxy mutex is released before
/// this returns, so the result must be owned data — never hold it across
/// network awaits (that is exactly the stall this refactor removes).
async fn read_proxy_state<R>(proxy: &SharedProxy, read: impl FnOnce(&ProxyState) -> R) -> R {
    let guard = proxy.lock().await;
    let state = guard.state.read().await;
    let result = read(&state);
    drop(state);
    drop(guard);
    result
}

/// Brief exclusive access to proxy state. Same no-await-across rule as above.
async fn write_proxy_state<R>(proxy: &SharedProxy, write: impl FnOnce(&mut ProxyState) -> R) -> R {
    let guard = proxy.lock().await;
    let mut state = guard.state.write().await;
    let result = write(&mut state);
    drop(state);
    drop(guard);
    result
}

/// Background reader forwarding sing-box stderr to the log at matching
/// levels; abort it (via the `JoinHandle`) once the child is reaped.
fn spawn_sing_box_stderr_reader(
    stderr: Option<tokio::process::ChildStderr>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
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
    })
}

struct ManagedProcess {
    child: tokio::process::Child,
    config_path: PathBuf,
    stderr_task: tokio::task::JoinHandle<()>,
    /// Spawn instant: unexpected exits log process age so crash-loops read
    /// differently from config flaps in the log.
    started_at: Instant,
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
        let manual_proxy_uri = config.manual_proxy_uri.clone();
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
                degraded: false,
                last_switch: None,
                failed_config_keys: Vec::new(),
                proxy_config: config,
                manual_proxy_uri,
                clash_controller: None,
                clash_secret: None,
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
    ///
    /// `clear_blacklist` grants failed configs a fresh chance on genuinely
    /// new data (refresh cycles). Ping cycles re-verify the same set, so
    /// clearing there would re-arm configs the health/starvation loops just
    /// failed over away from — flapping back onto the dead config.
    #[allow(clippy::too_many_lines)]
    pub async fn update(
        &self,
        config: &ProxyConfig,
        ranked: &[RankedConfig],
        clear_blacklist: bool,
    ) {
        // Sync the latest config into state so the health loop can read it
        {
            let mut state = self.state.write().await;
            state.proxy_config = config.clone();
            state.manual_proxy_uri.clone_from(&config.manual_proxy_uri);
            if clear_blacklist {
                state.failed_config_keys.clear();
            }
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

        // Recently failed configs stay out of auto-selection so a post-cycle
        // update can't flap straight back onto a config failover just left.
        // A manual pin bypasses the blacklist: explicit user choice wins.
        let blacklisted: std::collections::HashSet<String> = if config.manual_proxy_uri.is_some() {
            std::collections::HashSet::new()
        } else {
            self.state
                .read()
                .await
                .failed_config_keys
                .iter()
                .cloned()
                .collect()
        };
        let active_uri = {
            let state = self.state.read().await;
            state.active_config_uri.clone()
        };
        let (best, clear_stale_blacklist) =
            select_proxy_config(config, ranked, active_uri.as_deref(), &blacklisted);
        if clear_stale_blacklist {
            // Everything reachable recently failed: grant one fresh chance
            // rather than leaving the proxy with nothing to serve. The pool
            // is degraded: drop the floor until a switch serves anew.
            info!("proxy: all reachable configs blacklisted, clearing blacklist");
            self.emit_log("proxy: blacklist cleared".into());
            let mut state = self.state.write().await;
            state.failed_config_keys.clear();
            state.degraded = true;
        }

        let Some(best) = best else {
            if config.manual_proxy_uri.is_some() {
                info!("proxy: manual proxy config not reachable, keeping current");
                self.emit_log("proxy: manual config not reachable".into());
            } else {
                warn!("proxy: no reachable configs available");
                self.emit_log("proxy: no reachable configs".into());
            }
            return;
        };

        let (should_switch, reason) = {
            let state = self.state.read().await;
            match &state.active_config_uri {
                None => (true, "starting"),
                Some(uri) if uri != &best.uri => {
                    // Anti-flap dwell: rankings jitter between cycles, and
                    // every "new config" switch drops the port for seconds.
                    // Hold the current config inside the dwell window; proven
                    // failures (restart/failover arms below) always bypass.
                    // An explicit manual pin also bypasses: user choice wins.
                    if dwell_allows(
                        state.last_switch,
                        Instant::now(),
                        state.manual_proxy_uri.is_some(),
                    ) {
                        (true, "new config")
                    } else {
                        (false, "dwell")
                    }
                }
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
            if reason == "dwell" {
                info!(
                    name = %best.name,
                    "proxy: rankings changed but switched recently; holding current config"
                );
            }
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

        let switched = active_uri.as_deref() != Some(best.uri.as_str());
        let mut state = self.state.write().await;
        state.active_config_uri = Some(best.uri.clone());
        state.active_config_name = Some(best.name.clone());
        state.active_config_country.clone_from(&best.country_code);
        state.consecutive_failures = 0;
        state.last_switch = Some(Instant::now());
        if switched {
            // A new config means fresh hope: back to the full floor.
            state.degraded = false;
        }
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
                let age = managed.started_at.elapsed();
                warn!(
                    status = %status,
                    age_secs = age.as_secs(),
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

    /// Allocate the localhost Clash API controller for one proxy start, or
    /// `None` when unsupported/unavailable (the proxy still works, only
    /// starvation detection stays off).
    async fn maybe_alloc_clash_api(&self, port: u16) -> Option<ClashApi> {
        if !clash_api_supported(&self.sing_box_path).await {
            warn!("proxy: sing-box predates the Clash API, starvation detection disabled");
            return None;
        }
        let api = alloc_controller_port(port).map(|port| ClashApi {
            port,
            secret: random_clash_secret(),
        });
        if api.is_none() {
            warn!(
                port,
                "proxy: no free controller port, starvation detection disabled"
            );
        }
        api
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

        let clash_api = self.maybe_alloc_clash_api(current_config.port).await;
        let config_json =
            build_sing_box_config(&outbound, current_config.port, listen, clash_api.as_ref());
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
        let stderr_task = spawn_sing_box_stderr_reader(child.stderr.take());

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
            started_at: Instant::now(),
        };

        let (controller, secret) = clash_api
            .map(|api| {
                let addr = SocketAddr::new(
                    LOCALHOST_IP
                        .parse()
                        .expect("localhost IP is a valid address"),
                    api.port,
                );
                (addr, api.secret)
            })
            .unzip();
        {
            let mut state = self.state.write().await;
            state.running = true;
            state.consecutive_failures = 0;
            state.last_health_check = None;
            state.last_health_ok = false;
            state.clash_controller = controller;
            state.clash_secret = secret;
        }

        *self.process.lock().await = Some(managed);

        info!(
            port = current_config.port,
            name = %config.name,
            "proxy: started"
        );

        Ok(())
    }

    /// Transfer health check through the running proxy. Takes the shared
    /// handle (not `&self`) so callers never hold the proxy mutex across
    /// the network awaits below — a dead proxy's sequential URL timeouts
    /// would otherwise stall every post-cycle proxy update for minutes.
    /// Only brief state snapshots are taken under lock.
    pub async fn health_check(proxy: &SharedProxy) -> bool {
        Self::health_check_with(proxy, PROXY_HEALTH_CHECK_TIMEOUT, false).await
    }

    /// Idle-path verification: the primary URL on a short budget. A dead
    /// proxy with nobody connected must still be caught — but it must not
    /// burn the full five-URL gauntlet every interval while idle.
    pub async fn health_check_light(proxy: &SharedProxy) -> bool {
        Self::health_check_with(proxy, PROXY_HEALTH_LIGHT_TIMEOUT, true).await
    }

    async fn health_check_with(proxy: &SharedProxy, timeout: Duration, light: bool) -> bool {
        if !proxy.lock().await.is_process_alive().await {
            return false;
        }

        let port = read_proxy_state(proxy, |state| state.proxy_config.port).await;

        let proxy_url = format!("http://{LOCALHOST_IP}:{port}");
        let Ok(proxy_client) = Proxy::all(&proxy_url) else {
            warn!("proxy: invalid proxy URL for health check");
            return false;
        };
        let Ok(client) = reqwest::Client::builder()
            .timeout(timeout)
            .proxy(proxy_client)
            .build()
        else {
            warn!("proxy: health check client build failed");
            return false;
        };

        let (health_url, degraded) = read_proxy_state(proxy, |state| {
            (state.proxy_config.health_check_url.clone(), state.degraded)
        })
        .await;
        // Adaptive floor: full speed while the pool is healthy, absolute
        // minimum once everything reachable has failed.
        let floor_bps = if degraded {
            PROXY_HEALTH_MIN_BPS
        } else {
            PROXY_HEALTH_FULL_BPS
        };

        // Race the primary against page-sized fallbacks: first success wins
        // within one timeout budget instead of five sequential ones. The
        // tiny-JSON responders are out — they can never clear a speed floor,
        // so racing them only burns connections.
        // Light (idle) mode probes the primary alone on a short budget.
        let fallback_urls = ["https://1.1.1.1", "https://cloudflare.com"];
        let started = Instant::now();
        let ok = if light {
            body_transfers(&client, &health_url, floor_bps).await
        } else {
            let mut urls = vec![health_url.as_str()];
            urls.extend_from_slice(&fallback_urls);
            let any_ok = any_transfers(&client, &urls, floor_bps).await;
            if !any_ok {
                warn!(
                    primary = %health_url,
                    "proxy: all health check URLs failed"
                );
            }
            any_ok
        };
        if ok {
            let elapsed = started.elapsed();
            if elapsed >= PROXY_SLOW_CHECK_WARN {
                warn!(
                    elapsed_secs = elapsed.as_secs(),
                    "proxy: health check passed but slow — config degrading"
                );
            }
        }

        write_proxy_state(proxy, |state| {
            state.last_health_check = Some(Instant::now());
            state.last_health_ok = ok;
            if ok {
                state.consecutive_failures = 0;
            } else {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            }
        })
        .await;

        ok
    }

    /// Shared failure path for the health loop's full and light checks:
    /// a pinned config stays loud, otherwise the post-check failure count
    /// decides between waiting and failing over. Returns true when a
    /// failover was attempted (so the loop can resync the snapshot).
    async fn on_health_failure(
        proxy: &SharedProxy,
        ranked: &Arc<RwLock<Vec<RankedConfig>>>,
        runtime: &Arc<RwLock<RuntimeState>>,
        manual: Option<String>,
    ) -> bool {
        if manual.is_some() {
            warn!(
                "proxy: health check failed on a manually pinned config; staying (clear the pin to auto-failover)"
            );
            proxy.lock().await.emit_log(
                "proxy: health fail on manual pin; staying (clear the pin to auto-failover)".into(),
            );
            return false;
        }

        // The count below is post-check (health_check_with just updated it),
        // so exactly PROXY_MAX_CONSECUTIVE_FAILURES failures trip failover —
        // no extra interval burned on a snapshot taken before the check ran.
        let failures = read_proxy_state(proxy, |state| state.consecutive_failures).await;
        if !failover_tripped(failures) {
            warn!(
                failures = failures,
                max = PROXY_MAX_CONSECUTIVE_FAILURES,
                "proxy: health check failed, waiting for more failures before failover"
            );
            proxy.lock().await.emit_log(format!(
                "proxy: health fail {failures}/{PROXY_MAX_CONSECUTIVE_FAILURES}"
            ));
            return false;
        }

        warn!("proxy: health check failed, attempting failover");
        if let Err(err) = Self::failover(proxy, ranked).await {
            error!(error = %err, "proxy: failover failed");
            proxy
                .lock()
                .await
                .emit_log(format!("proxy: failover failed: {err}"));
        }
        sync_runtime_proxy_snapshot(proxy, runtime).await;
        true
    }

    /// Revive a proxy that stopped itself: after an exhausted failover sets
    /// `running = false`, both loops skip forever — a dead-end latch only a
    /// cycle would clear, minutes later. When the feature is still enabled
    /// and the pool has anything reachable, try to serve again right away.
    async fn revive_if_enabled(proxy: &SharedProxy, ranked: &Arc<RwLock<Vec<RankedConfig>>>) {
        let (enabled, proxy_config, active_uri, blacklisted) = read_proxy_state(proxy, |state| {
            (
                state.proxy_config.enabled,
                state.proxy_config.clone(),
                state.active_config_uri.clone(),
                state
                    .failed_config_keys
                    .iter()
                    .cloned()
                    .collect::<HashSet<String>>(),
            )
        })
        .await;
        if !enabled {
            return;
        }
        let ranked_snapshot = ranked.read().await.clone();
        let (best, _) = select_proxy_config(
            &proxy_config,
            &ranked_snapshot,
            active_uri.as_deref(),
            &blacklisted,
        );
        let Some(best) = best else {
            return;
        };
        info!(name = %best.name, "proxy: attempting revival after stopped failover");
        let started = proxy.lock().await.start_with_config(best).await.is_ok();
        if !started {
            warn!(name = %best.name, "proxy: revival failed, will retry next interval");
            return;
        }
        proxy.lock().await.emit_log(format!(
            "proxy: revived → {} (port {})",
            best.name, proxy_config.port
        ));
        write_proxy_state(proxy, |state| {
            state.active_config_uri = Some(best.uri.clone());
            state.active_config_name = Some(best.name.clone());
            state.active_config_country.clone_from(&best.country_code);
            state.consecutive_failures = 0;
            state.last_switch = Some(Instant::now());
            // Revival is fresh hope by definition: back to the full floor.
            state.degraded = false;
        })
        .await;
    }
    /// Switch away from the active config to the next best reachable one.
    /// Takes the shared handle (not `&self`) so callers never hold the
    /// proxy mutex across candidate restarts, transfer checks, and the
    /// cooldown sleep — that would stall every post-cycle proxy update.
    /// Only brief state snapshots are taken under lock.
    pub async fn failover(
        proxy: &SharedProxy,
        ranked: &Arc<RwLock<Vec<RankedConfig>>>,
    ) -> Result<()> {
        let (current_uri, last_failover, failed_keys) = read_proxy_state(proxy, |state| {
            (
                state.active_config_uri.clone(),
                state.last_failover,
                state.failed_config_keys.clone(),
            )
        })
        .await;

        // Failover cooldown: wait between failovers to avoid rapid cycling
        if let Some(last) = last_failover {
            let remaining = PROXY_FAILOVER_COOLDOWN.saturating_sub(last.elapsed());
            if !remaining.is_zero() {
                info!(
                    remaining_ms = remaining.as_millis(),
                    "proxy: failover cooldown active, waiting"
                );
                time::sleep(remaining).await;
            }
        }

        // Re-read the pool after the cooldown: minutes may have passed and a
        // cycle may have published fresher truth meanwhile.
        let ranked_snapshot = ranked.read().await.clone();
        // Try configs that haven't failed recently
        let candidates: Vec<&RankedConfig> = ranked_snapshot
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
            proxy
                .lock()
                .await
                .emit_log("proxy: blacklist cleared".into());
            // Degraded pool: nothing reachable survived, so drop the floor
            // to the absolute minimum until a switch serves a new config.
            write_proxy_state(proxy, |state| {
                state.failed_config_keys.clear();
                state.degraded = true;
            })
            .await;
            ranked_snapshot
                .iter()
                .filter(|c| c.reachable && current_uri.as_deref() != Some(&c.uri))
                .collect()
        } else {
            candidates
        };

        for candidate in candidates {
            info!(name = %candidate.name, "proxy: attempting failover");
            let started = proxy
                .lock()
                .await
                .start_with_config(candidate)
                .await
                .is_ok();
            if started && Self::health_check(proxy).await {
                let port = read_proxy_state(proxy, |state| state.proxy_config.port).await;
                info!(name = %candidate.name, "proxy: failover succeeded");
                proxy.lock().await.emit_log(format!(
                    "proxy: failover → {} (port {})",
                    candidate.name, port
                ));
                write_proxy_state(proxy, |state| {
                    state.active_config_uri = Some(candidate.uri.clone());
                    state.active_config_name = Some(candidate.name.clone());
                    state
                        .active_config_country
                        .clone_from(&candidate.country_code);
                    state.consecutive_failures = 0;
                    state.last_failover = Some(Instant::now());
                    state.last_switch = Some(Instant::now());
                    // A new config means fresh hope: back to the full floor.
                    state.degraded = false;
                })
                .await;
                return Ok(());
            }
            // Mark this config as failed
            write_proxy_state(proxy, |state| {
                if state.failed_config_keys.len() < PROXY_MAX_RECENTLY_FAILED_KEYS {
                    state.failed_config_keys.push(candidate.dedup_key.clone());
                }
            })
            .await;
        }

        write_proxy_state(proxy, |state| state.running = false).await;

        proxy
            .lock()
            .await
            .emit_log("proxy: failover exhausted".into());
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
        // The controller dies with the process; a stale address must never
        // be polled (the secret is dropped here too).
        state.clash_controller = None;
        state.clash_secret = None;
    }

    pub async fn shutdown(&self) {
        self.stop().await;
    }

    pub async fn snapshot(&self) -> ProxySnapshot {
        let state = self.state.read().await;
        ProxySnapshot {
            active_config: state.active_config_name.clone(),
            active_uri: state.active_config_uri.clone(),
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

/// Pure proxy target selection: best reachable config honoring the manual
/// pin, the no-rotate keep-alive, and the recent-failure blacklist.
///
/// Returns the pick plus whether the blacklist went stale (everything
/// reachable is blacklisted: the caller clears it and picks the best
/// reachable instead of leaving the proxy with nothing to serve).
fn select_proxy_config<'a>(
    config: &ProxyConfig,
    ranked: &'a [RankedConfig],
    active_uri: Option<&str>,
    blacklisted: &std::collections::HashSet<String>,
) -> (Option<&'a RankedConfig>, bool) {
    let eligible = |candidate: &RankedConfig| {
        candidate.reachable && !blacklisted.contains(&candidate.dedup_key)
    };

    // Use manual proxy if set, otherwise auto-select the best reachable config.
    let best = config.manual_proxy_uri.as_ref().map_or_else(
        || ranked.iter().find(|candidate| eligible(candidate)),
        |manual_uri| {
            ranked
                .iter()
                .find(|candidate| candidate.reachable && candidate.uri == *manual_uri)
        },
    );

    // When rotating_proxy is disabled and a config is already active, prefer
    // keeping it if it's still reachable (even if it's no longer the best).
    // A blacklisted active config is not keepable: failover moved away
    // from it for a reason.
    let best = if !config.rotating_proxy && config.manual_proxy_uri.is_none() {
        active_uri.map_or(best, |uri| {
            let keepable = ranked
                .iter()
                .any(|candidate| candidate.uri == uri && eligible(candidate));
            if keepable {
                ranked.iter().find(|candidate| candidate.uri == uri)
            } else {
                best
            }
        })
    } else {
        best
    };

    if best.is_none() && config.manual_proxy_uri.is_none() && !blacklisted.is_empty() {
        return (ranked.iter().find(|candidate| candidate.reachable), true);
    }
    (best, false)
}

/// GET `url` through the proxy; true only when the response body actually
/// transfers at or above `floor_bps`. A stall anywhere — connect, handshake,
/// headers, mid-body — fails via the client timeout. Status alone is never
/// enough: false-positive configs complete handshakes and headers, then
/// deliver nothing — or a tiny page with a clean EOF on a link too slow for
/// real apps. Empty 204s (connectivity proven, nothing to carry) and capped
/// bodies always pass.
async fn body_transfers(client: &reqwest::Client, url: &str, floor_bps: u64) -> bool {
    let outcome = transfer_outcome(client, url).await;
    transfer_ok(outcome, floor_bps)
}

/// Measurable result of one transfer attempt, so the pass rule stays a pure
/// function under test.
#[derive(Debug, Clone, Copy)]
struct TransferOutcome {
    status_ok: bool,
    empty_status: bool,
    received: u64,
    complete: bool,
    elapsed: Duration,
}

impl TransferOutcome {
    const fn idle() -> Self {
        Self {
            status_ok: false,
            empty_status: false,
            received: 0,
            complete: false,
            elapsed: Duration::ZERO,
        }
    }
}

async fn transfer_outcome(client: &reqwest::Client, url: &str) -> TransferOutcome {
    let started = Instant::now();
    let Ok(resp) = client.get(url).send().await else {
        return TransferOutcome::idle();
    };
    let status = resp.status();
    if !(status.is_success() || status.as_u16() == 204) {
        return TransferOutcome::idle();
    }
    let (received, complete) =
        accumulate_limited(resp.bytes_stream(), PROXY_HEALTH_CHECK_BODY_BYTES).await;
    TransferOutcome {
        status_ok: true,
        empty_status: status.as_u16() == 204,
        received,
        complete,
        elapsed: started.elapsed(),
    }
}

/// Pure pass rule with an explicit speed floor: empty-status with clean EOF,
/// capped volume, or a complete body delivered at or above `floor_bps`.
/// Sub-second deliveries always pass (anything that fast proves flow); slower
/// ones must clear the floor. Tiny fast pages pass by design — the proxy
/// moved them, so switching cannot help, and the starvation detector owns
/// real user traffic. Slow trickles fail instead of choking apps silently.
const fn transfer_ok(outcome: TransferOutcome, floor_bps: u64) -> bool {
    if !outcome.status_ok {
        return false;
    }
    if outcome.received >= PROXY_HEALTH_CHECK_BODY_BYTES {
        return true;
    }
    if !outcome.complete {
        return false;
    }
    if outcome.empty_status {
        return true;
    }
    outcome.received >= floor_bps * outcome.elapsed.as_secs()
}

/// Race several URLs through one client: first success wins. A dead proxy
/// used to burn sequential per-URL timeouts (5 × 15s); racing bounds every
/// check by a single timeout instead.
async fn any_transfers(client: &reqwest::Client, urls: &[&str], floor_bps: u64) -> bool {
    let mut pending: Vec<_> = urls
        .iter()
        .map(|url| Box::pin(body_transfers(client, url, floor_bps)))
        .collect();
    while !pending.is_empty() {
        let (ok, _index, rest) = futures_util::future::select_all(pending).await;
        if ok {
            return true;
        }
        pending = rest;
    }
    false
}

/// Post-check failure count trips failover at exactly
/// `PROXY_MAX_CONSECUTIVE_FAILURES` — no extra interval burned on a snapshot
/// taken before the check ran.
const fn failover_tripped(failures: u32) -> bool {
    failures >= PROXY_MAX_CONSECUTIVE_FAILURES
}

/// Anti-flap dwell: a "new config" switch inside the dwell window is held
/// unless the user pinned explicitly. Restarts and failovers bypass.
fn dwell_allows(last_switch: Option<Instant>, now: Instant, pinned: bool) -> bool {
    if pinned {
        return true;
    }
    last_switch.is_none_or(|switched| now.duration_since(switched) >= PROXY_MIN_SWITCH_INTERVAL)
}

/// Drain a byte stream up to `cap`, returning `(received, complete)`.
/// `complete` is true only on clean EOF; any error (e.g. a timeout stall)
/// ends the stream incomplete with whatever arrived so far. Reaching `cap`
/// reports `(cap, false)` — the caller treats "plenty received" as success.
async fn accumulate_limited<S, B, E>(mut stream: S, cap: u64) -> (u64, bool)
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut received = 0u64;
    while received < cap {
        match futures_util::StreamExt::next(&mut stream).await {
            Some(Ok(chunk)) => {
                received = received
                    .saturating_add(u64::try_from(chunk.as_ref().len()).unwrap_or(u64::MAX));
            }
            Some(Err(_)) => return (received, false),
            None => return (received, true),
        }
    }
    (received, false)
}

/// Mirror the proxy snapshot into the TUI runtime state so the 🚪 door tracks
/// a mid-cycle failover immediately instead of pointing at (or vanishing
/// from) the dead config until the next ping/refresh syncs it minutes later.
/// Same five fields `update_proxy_and_ranked` copies; ranked is untouched.
async fn sync_runtime_proxy_snapshot(proxy: &SharedProxy, runtime: &Arc<RwLock<RuntimeState>>) {
    let snapshot = proxy.lock().await.snapshot().await;
    let mut state = runtime.write().await;
    state
        .proxy_active_config
        .clone_from(&snapshot.active_config);
    state.proxy_active_uri = snapshot.active_uri;
    state.proxy_running = snapshot.running;
    state.proxy_port = snapshot.port;
    state.proxy_discoverable = snapshot.discoverable;
}

pub fn spawn_health_loop(
    proxy: SharedProxy,
    ranked: Arc<RwLock<Vec<RankedConfig>>>,
    runtime: Arc<RwLock<RuntimeState>>,
) {
    tokio::spawn(async move {
        let mut last_interval = 0u64;

        loop {
            // Read interval from state — reactive to config changes
            let (running, interval, controller, secret, manual) = {
                let p = proxy.lock().await;
                let state = p.state.read().await;
                let result = (
                    state.running,
                    state.proxy_config.health_check_interval_seconds,
                    state.clash_controller,
                    state.clash_secret.clone(),
                    state.manual_proxy_uri.clone(),
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
                // Failover-exhausted dead end: revive while enabled instead
                // of skipping until the next cycle, minutes later.
                PersistentProxy::revive_if_enabled(&proxy, &ranked).await;
                continue;
            }

            // Idle users must never trigger a switch: failover is driven by
            // real user traffic (the starvation detector), not by background
            // probes. When the Clash controller is available, an empty
            // connection table gets the cheap single-URL verification
            // instead of the full gauntlet — a dead proxy with nobody
            // connected must still be caught. A manually pinned config never
            // auto-switches either — it only gets loud log lines from the
            // starvation path.
            // No proxy lock is held across the transfer checks below: a dead
            // proxy burns URL timeouts and must not stall updates.
            let idle = if let (Some(addr), Some(secret)) = (controller, secret) {
                match fetch_clash_connections(addr, &secret).await {
                    Ok(connections) => connections.is_empty(),
                    Err(err) => {
                        warn!(
                            error = %err,
                            "proxy: clash API unreachable, falling back to transfer health check"
                        );
                        false
                    }
                }
            } else {
                false
            };

            let health_ok = if idle {
                PersistentProxy::health_check_light(&proxy).await
            } else {
                PersistentProxy::health_check(&proxy).await
            };

            if health_ok {
                continue;
            }

            PersistentProxy::on_health_failure(&proxy, &ranked, &runtime, manual).await;
        }
    });
}

/// One tracked user connection: first-seen instant plus latest counters.
/// Local clock only — sing-box timestamps are never parsed.
#[derive(Debug, Clone, Copy)]
struct ConnSample {
    first_seen: Instant,
    upload: u64,
    download: u64,
}

/// sing-box `/connections` entry (only the fields the detector needs;
/// unknown fields such as metadata/chains/rules are ignored).
#[derive(Debug, Clone, Deserialize)]
struct ClashConnection {
    id: String,
    upload: i64,
    download: i64,
}

#[derive(Debug, Deserialize)]
struct ClashConnectionsSnapshot {
    connections: Vec<ClashConnection>,
}

/// Starvation rule: old enough, uploaded real volume, zero bytes back.
/// Idle connections (no upload) never match; any working link carries
/// TCP/TLS acknowledgement traffic, so `download == 0` at this volume is
/// pathological rather than slow.
fn connection_starved(age: Duration, upload: u64, download: u64) -> bool {
    download == 0 && upload >= PROXY_STARVATION_MIN_UPLOAD_BYTES && age >= PROXY_STARVATION_MIN_AGE
}

/// Read the active connection list from the sing-box Clash API.
/// `secret` is sent as a bearer token and never logged.
async fn fetch_clash_connections(addr: SocketAddr, secret: &str) -> Result<Vec<ClashConnection>> {
    let client = reqwest::Client::builder()
        .timeout(PROXY_CLASH_API_TIMEOUT)
        .build()
        .context("clash API client build failed")?;
    let snapshot: ClashConnectionsSnapshot = client
        .get(format!("http://{addr}/connections"))
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
        .context("clash API snapshot request failed")?
        .error_for_status()
        .context("clash API snapshot status failed")?
        .json()
        .await
        .context("clash API snapshot decode failed")?;
    Ok(snapshot.connections)
}

/// Passive starvation detector: polls the sing-box connection table and
/// fails over when a user connection keeps trying with nothing coming back.
///
/// Idle users are untouched (no upload → never starved). A suspect triggers
/// one immediate transfer health check; failover happens only when that
/// fails too (the config genuinely carries no data). A manually pinned
/// config is never switched away from — it only gets a loud log line.
pub fn spawn_starvation_loop(
    proxy: SharedProxy,
    ranked: Arc<RwLock<Vec<RankedConfig>>>,
    runtime: Arc<RwLock<RuntimeState>>,
) {
    tokio::spawn(async move {
        let mut samples: HashMap<String, ConnSample> = HashMap::new();
        let mut reported: HashSet<String> = HashSet::new();
        // Latch (controller, skipped polls) while the API is unreachable so
        // one failing start doesn't spam every poll — but retry periodically
        // in case the same port comes back on a later restart.
        let mut unreachable: Option<(SocketAddr, u8)> = None;

        loop {
            time::sleep(PROXY_STARVATION_POLL_INTERVAL).await;

            let (running, controller, secret) = read_proxy_state(&proxy, |state| {
                (
                    state.running,
                    state.clash_controller,
                    state.clash_secret.clone(),
                )
            })
            .await;
            let (Some(addr), Some(secret)) = (controller, secret) else {
                samples.clear();
                reported.clear();
                unreachable = None;
                continue;
            };
            if !running {
                samples.clear();
                reported.clear();
                unreachable = None;
                continue;
            }
            // A proxy restart mints a fresh controller; re-arm on change.
            if unreachable.is_some_and(|(unreachable_addr, _)| unreachable_addr != addr) {
                unreachable = None;
            }
            if let Some((_, skipped)) = unreachable {
                // Retry roughly every minute; same-port restarts re-arm.
                if skipped < PROXY_CLASH_UNREACHABLE_RETRY_POLLS {
                    unreachable = Some((addr, skipped + 1));
                    continue;
                }
                unreachable = None;
            }

            let connections = match fetch_clash_connections(addr, &secret).await {
                Ok(connections) => connections,
                Err(err) => {
                    warn!(
                        error = %err,
                        "proxy: clash API unreachable, starvation detection paused"
                    );
                    unreachable = Some((addr, 0));
                    continue;
                }
            };

            let now = Instant::now();
            let live: HashSet<&str> = connections.iter().map(|conn| conn.id.as_str()).collect();
            for conn in &connections {
                let sample = samples.entry(conn.id.clone()).or_insert(ConnSample {
                    first_seen: now,
                    upload: 0,
                    download: 0,
                });
                sample.upload = u64::try_from(conn.upload.max(0)).unwrap_or(u64::MAX);
                sample.download = u64::try_from(conn.download.max(0)).unwrap_or(u64::MAX);
                if reported.contains(&conn.id) {
                    continue;
                }
                if connection_starved(
                    now.saturating_duration_since(sample.first_seen),
                    sample.upload,
                    sample.download,
                ) {
                    reported.insert(conn.id.clone());
                    let retry_allowed =
                        on_starved_connection(&proxy, &ranked, &conn.id, sample.upload, &runtime)
                            .await;
                    if retry_allowed {
                        // The failover itself failed: let this connection
                        // retrigger once the pool recovers instead of going
                        // silent for the rest of its lifetime.
                        reported.remove(&conn.id);
                    }
                    // One trigger per poll: a failover restarts the process,
                    // so the rest of this snapshot is already stale.
                    break;
                }
            }
            samples.retain(|id, _| live.contains(id.as_str()));
            reported.retain(|id| live.contains(id.as_str()));
        }
    });
}

/// One starved connection found: verify the active config with a transfer
/// check and fail over to the next best config when it carries nothing.
/// A manually pinned config is verified the same way but never auto-switched:
/// it stays with a loud log so the user can pick another proxy themselves.
/// Returns true when the caller should clear the per-connection latch (the
/// failover itself failed, so the same connection may legitimately retrigger
/// once the pool recovers). Every other outcome stays latched: a healthy
/// config means the connection — not the proxy — is at fault, and a pin
/// means the user must act.
async fn on_starved_connection(
    proxy: &SharedProxy,
    ranked: &Arc<RwLock<Vec<RankedConfig>>>,
    conn_id: &str,
    upload: u64,
    runtime: &Arc<RwLock<RuntimeState>>,
) -> bool {
    warn!(
        connection = %conn_id,
        upload_bytes = upload,
        "proxy: connection uploaded with zero download, verifying active config"
    );
    proxy.lock().await.emit_log(format!(
        "proxy: a connection sent {upload} bytes with nothing back; verifying active config"
    ));
    // Even a manually chosen config is transfer-verified here: the user cannot
    // tell from the list whether it actually carries data. Manual only changes
    // what happens on failure (loud stay, no auto-switch).
    if PersistentProxy::health_check(proxy).await {
        info!("proxy: starved connection, but the active config transfers data; keeping");
        return false;
    }
    let manual = proxy
        .lock()
        .await
        .state
        .read()
        .await
        .manual_proxy_uri
        .clone();
    if manual.is_some() {
        let message = "proxy: manually pinned config verified starved; staying (clear the pin to auto-failover)";
        warn!("{message}");
        proxy.lock().await.emit_log(message.to_string());
        return false;
    }
    warn!("proxy: active config verified starved, attempting failover");
    if let Err(err) = PersistentProxy::failover(proxy, ranked).await {
        error!(error = %err, "proxy: starvation failover failed");
        proxy
            .lock()
            .await
            .emit_log(format!("proxy: starvation failover failed: {err}"));
        sync_runtime_proxy_snapshot(proxy, runtime).await;
        return true;
    }
    sync_runtime_proxy_snapshot(proxy, runtime).await;
    false
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProxySnapshot {
    pub active_config: Option<String>,
    pub active_uri: Option<String>,
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
    restrict_file_permissions(&path);
    Ok(path)
}

/// Restrict a sing-box temp config to owner-only (0600 on Unix).
/// No-op on Windows; single syscall, no proxy delay.
#[allow(clippy::missing_const_for_fn)]
fn restrict_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Whether the sing-box binary supports the Clash API controller config.
/// The API ships in every 1.x; older binaries would reject the unknown
/// `experimental` key and fail to start, so they get a controller-less
/// proxy (starvation detection stays off) instead of an outage.
async fn clash_api_supported(sing_box_path: &str) -> bool {
    if sing_box_version_at_least(1, 0, 0) {
        return true;
    }
    crate::probe::sing_box_major_version(sing_box_path)
        .await
        .is_some_and(|major| major >= 1)
}

/// Find a free localhost port for the Clash API controller, probing above
/// the proxy port. Returns `None` when the range is exhausted — the proxy
/// then starts without a controller (starvation detection stays off).
fn alloc_controller_port(proxy_port: u16) -> Option<u16> {
    for offset in 1..=PROXY_CLASH_CONTROLLER_PORT_ATTEMPTS {
        let candidate = proxy_port.saturating_add(offset);
        if candidate == proxy_port {
            continue;
        }
        if std::net::TcpListener::bind((LOCALHOST_IP, candidate)).is_ok() {
            return Some(candidate);
        }
    }
    None
}

/// Fresh random bearer secret for one proxy start (64 hex chars from the OS
/// RNG). Localhost-only listener; still never logged or persisted.
fn random_clash_secret() -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 32];
    if getrandom::fill(&mut buf).is_ok() {
        let mut secret = String::with_capacity(64);
        for byte in buf {
            secret.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
            secret.push(char::from(HEX_DIGITS[usize::from(byte & 0x0F)]));
        }
        return secret;
    }
    // Practically unreachable; unique per start, localhost-only.
    format!(
        "fallback-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
}

fn build_sing_box_config(
    outbound: &Value,
    port: u16,
    listen: &str,
    clash_api: Option<&ClashApi>,
) -> Value {
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

    // The control plane always stays on localhost, even for a discoverable
    // (LAN-shared) proxy inbound: only this process may query connections.
    let experimental = clash_api.map(|api| {
        json!({
            "clash_api": {
                "external_controller": format!("{LOCALHOST_IP}:{}", api.port),
                "secret": api.secret,
            }
        })
    });

    let mut config = json!({
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
    });
    if let Some(experimental) = experimental
        && let Some(obj) = config.as_object_mut()
    {
        obj.insert("experimental".to_string(), experimental);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn selectable(name: &str, reachable: bool) -> RankedConfig {
        RankedConfig {
            rank: 0,
            stability_count: 0,
            id: name.to_string(),
            dedup_key: format!("{name}-key"),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: name.to_string(),
            endpoint: crate::model::Endpoint {
                host: "example.com".to_string(),
                port: 443,
            },
            uri: format!("vless://{name}@example.com:443"),
            reachable,
            validation: "active_http".to_string(),
            latency_ms: Some(10),
            http_status: Some(204),
            download_mbps: None,
            download_bytes: None,
            error: None,
            country_code: None,
        }
    }

    fn auto_proxy_config() -> ProxyConfig {
        ProxyConfig {
            enabled: true,
            rotating_proxy: true,
            ..Default::default()
        }
    }

    fn blacklist_of(keys: &[&str]) -> std::collections::HashSet<String> {
        keys.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn selection_skips_blacklisted_config() {
        // Failover just left A for dead: the next update must serve B, not
        // flap straight back onto A.
        let ranked = vec![selectable("a", true), selectable("b", true)];
        let (best, clear) = select_proxy_config(
            &auto_proxy_config(),
            &ranked,
            None,
            &blacklist_of(&["a-key"]),
        );
        assert_eq!(best.map(|item| item.name.as_str()), Some("b"));
        assert!(!clear);
    }

    #[test]
    fn selection_abandons_blacklisted_active_without_rotation() {
        // rotating_proxy=false keeps a still-reachable active config — unless
        // failover blacklisted it, in which case it must move on.
        let mut config = auto_proxy_config();
        config.rotating_proxy = false;
        let ranked = vec![selectable("a", true), selectable("b", true)];
        let (best, _) = select_proxy_config(
            &config,
            &ranked,
            Some("vless://a@example.com:443"),
            &blacklist_of(&["a-key"]),
        );
        assert_eq!(best.map(|item| item.name.as_str()), Some("b"));
    }

    #[test]
    fn selection_keeps_healthy_active_without_rotation() {
        let mut config = auto_proxy_config();
        config.rotating_proxy = false;
        let ranked = vec![selectable("a", true), selectable("b", true)];
        let (best, _) = select_proxy_config(
            &config,
            &ranked,
            Some("vless://b@example.com:443"),
            &blacklist_of(&["a-key"]),
        );
        assert_eq!(best.map(|item| item.name.as_str()), Some("b"));
    }

    #[test]
    fn selection_clears_stale_blacklist_when_all_blocked() {
        let ranked = vec![selectable("a", true), selectable("b", true)];
        let (best, clear) = select_proxy_config(
            &auto_proxy_config(),
            &ranked,
            None,
            &blacklist_of(&["a-key", "b-key"]),
        );
        assert_eq!(best.map(|item| item.name.as_str()), Some("a"));
        assert!(clear);
    }

    #[test]
    fn selection_manual_pin_bypasses_blacklist() {
        let mut config = auto_proxy_config();
        config.manual_proxy_uri = Some("vless://a@example.com:443".to_string());
        let ranked = vec![selectable("a", true), selectable("b", true)];
        let (best, clear) = select_proxy_config(&config, &ranked, None, &blacklist_of(&["a-key"]));
        assert_eq!(best.map(|item| item.name.as_str()), Some("a"));
        assert!(!clear);
    }

    #[tokio::test]
    async fn update_keeps_blacklist_on_ping_and_clears_on_refresh() {
        // The flap guard: a ping (same set re-verified) must not re-arm the
        // config failover just left; only a refresh (new data) clears.
        let proxy = PersistentProxy::new(
            ProxyConfig {
                enabled: false,
                ..Default::default()
            },
            String::new(),
            None,
        );
        proxy.state.write().await.failed_config_keys = vec!["dead-key".to_string()];
        let disabled = ProxyConfig {
            enabled: false,
            ..Default::default()
        };
        proxy.update(&disabled, &[], false).await;
        assert_eq!(
            proxy.state.read().await.failed_config_keys,
            vec!["dead-key".to_string()]
        );
        proxy.update(&disabled, &[], true).await;
        assert!(proxy.state.read().await.failed_config_keys.is_empty());
    }

    fn minimal_outbound() -> Value {
        json!({
            "type": "vless",
            "settings": {
                "vnext": [{
                    "address": "example.com",
                    "port": 443,
                    "users": [{ "id": "test-uuid" }]
                }]
            }
        })
    }

    #[test]
    fn build_config_clash_api_present_when_allocated() {
        let api = ClashApi {
            port: 27911,
            secret: "s3cr3t".to_string(),
        };
        let config = build_sing_box_config(&minimal_outbound(), 27910, "127.0.0.1", Some(&api));
        assert_eq!(
            config["experimental"]["clash_api"]["external_controller"]
                .as_str()
                .unwrap(),
            "127.0.0.1:27911"
        );
        assert_eq!(
            config["experimental"]["clash_api"]["secret"]
                .as_str()
                .unwrap(),
            "s3cr3t"
        );
    }

    #[test]
    fn build_config_clash_api_stays_localhost_when_discoverable() {
        let api = ClashApi {
            port: 27911,
            secret: "s3cr3t".to_string(),
        };
        let config = build_sing_box_config(&minimal_outbound(), 27910, "0.0.0.0", Some(&api));
        assert_eq!(config["inbounds"][0]["listen"], "0.0.0.0");
        assert_eq!(
            config["experimental"]["clash_api"]["external_controller"]
                .as_str()
                .unwrap(),
            "127.0.0.1:27911"
        );
    }

    #[test]
    fn build_config_no_experimental_without_clash_api() {
        let config = build_sing_box_config(&minimal_outbound(), 27910, "127.0.0.1", None);
        assert!(config.get("experimental").is_none());
    }

    #[test]
    fn alloc_controller_port_returns_port_above_proxy() {
        let picked = alloc_controller_port(27910).expect("free port near default");
        assert!(picked > 27910);
        assert!(picked <= 27910 + PROXY_CLASH_CONTROLLER_PORT_ATTEMPTS);
    }

    #[test]
    fn alloc_controller_port_skips_occupied_port() {
        let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let taken = occupied.local_addr().expect("addr").port();
        // `occupied` is held, so the allocator must skip `taken` even when
        // it is the first candidate.
        let base = taken.saturating_sub(1);
        let picked = alloc_controller_port(base).expect("later port is free");
        assert_ne!(picked, taken);
        drop(occupied);
    }

    #[test]
    fn clash_secret_is_long_hex_and_unique() {
        let first = random_clash_secret();
        let second = random_clash_secret();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn connection_starved_matches_only_trying_without_data() {
        // Idle: no upload, never starved.
        assert!(!connection_starved(
            PROXY_STARVATION_MIN_AGE + Duration::from_secs(60),
            0,
            0
        ));
        // Young: heavy upload but not old enough.
        assert!(!connection_starved(
            PROXY_STARVATION_MIN_AGE.saturating_sub(Duration::from_secs(1)),
            10 * 1024 * 1024,
            0
        ));
        // Light upload: below the trying threshold.
        assert!(!connection_starved(
            PROXY_STARVATION_MIN_AGE + Duration::from_secs(60),
            PROXY_STARVATION_MIN_UPLOAD_BYTES - 1,
            0
        ));
        // Anything downloaded proves data flows.
        assert!(!connection_starved(
            PROXY_STARVATION_MIN_AGE + Duration::from_secs(3600),
            10 * 1024 * 1024,
            1
        ));
        // Starved: old, real volume up, zero back — on the boundary too.
        assert!(connection_starved(
            PROXY_STARVATION_MIN_AGE,
            PROXY_STARVATION_MIN_UPLOAD_BYTES,
            0
        ));
        assert!(connection_starved(
            PROXY_STARVATION_MIN_AGE + Duration::from_secs(3600),
            10 * 1024 * 1024,
            0
        ));
    }

    #[test]
    fn clash_snapshot_parses_connections_and_ignores_unknown_fields() {
        let body = serde_json::json!({
            "downloadTotal": 100,
            "uploadTotal": 200,
            "connections": [
                {
                    "id": "a",
                    "upload": 70000,
                    "download": 0,
                    "start": "2026-09-07T00:00:00Z",
                    "metadata": {
                        "network": "tcp",
                        "sourceIP": "127.0.0.1",
                        "destinationIP": "1.2.3.4",
                        "destinationPort": "443",
                        "host": "example.com"
                    },
                    "chains": ["proxy-0"],
                    "rule": "final",
                    "rulePayload": ""
                },
                { "id": "b", "upload": 10, "download": 5 }
            ],
            "memory": 123
        });
        let snapshot: ClashConnectionsSnapshot =
            serde_json::from_value(body).expect("snapshot decodes");
        assert_eq!(snapshot.connections.len(), 2);
        assert_eq!(snapshot.connections[0].id, "a");
        assert_eq!(snapshot.connections[0].upload, 70000);
        assert_eq!(snapshot.connections[0].download, 0);
    }

    #[tokio::test]
    async fn accumulate_reports_complete_stream() {
        let stream =
            futures_util::stream::iter(vec![Ok::<Vec<u8>, &str>(vec![1, 2, 3]), Ok(vec![4, 5])]);
        assert_eq!(accumulate_limited(stream, 1024).await, (5, true));
    }

    #[tokio::test]
    async fn accumulate_stops_at_cap_without_claiming_eof() {
        // Chunks are atomic: 3 + 2 bytes overshoot the cap of 4, the loop
        // stops anyway, and EOF is not claimed.
        let stream =
            futures_util::stream::iter(vec![Ok::<Vec<u8>, &str>(vec![1, 2, 3]), Ok(vec![4, 5])]);
        assert_eq!(accumulate_limited(stream, 4).await, (5, false));
    }

    #[tokio::test]
    async fn accumulate_reports_partial_on_mid_stream_error() {
        let stream =
            futures_util::stream::iter(vec![Ok::<Vec<u8>, &str>(vec![1, 2, 3]), Err("stalled")]);
        assert_eq!(accumulate_limited(stream, 1024).await, (3, false));
    }

    #[tokio::test]
    async fn accumulate_empty_stream_is_complete() {
        let stream = futures_util::stream::iter(Vec::<Result<Vec<u8>, &str>>::new());
        assert_eq!(accumulate_limited(stream, 1024).await, (0, true));
    }

    /// Minimal mock of the sing-box Clash API: one canned `/connections`
    /// snapshot, bearer-checked. Validates the exact request shape the
    /// detector sends (path, auth) plus the decode path, with no sing-box.
    struct MockClashApi {
        addr: SocketAddr,
        task: tokio::task::JoinHandle<()>,
    }

    impl MockClashApi {
        async fn start(secret: &'static str, body: &'static str) -> Self {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("mock binds");
            let addr = listener.local_addr().expect("mock addr");
            let task = tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                while let Ok((mut stream, _)) = listener.accept().await {
                    let mut raw = Vec::new();
                    let mut chunk = vec![0u8; 1024];
                    let request = loop {
                        let Ok(n) = stream.read(&mut chunk).await else {
                            break String::new();
                        };
                        if n == 0 {
                            break String::new();
                        }
                        raw.extend_from_slice(&chunk[..n]);
                        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                            break String::from_utf8_lossy(&raw).into_owned();
                        }
                    };
                    // Header names arrive lowercase on the wire; values keep
                    // their case (the secret is lowercase hex, so lowering
                    // the whole request is safe here).
                    let lowered = request.to_lowercase();
                    let authorized = lowered.contains(&format!("authorization: bearer {secret}"));
                    let response = if lowered.starts_with("get /connections ") && authorized {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    };
                    let _ = stream.write_all(response.as_bytes()).await;
                }
            });
            Self { addr, task }
        }
    }

    impl Drop for MockClashApi {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    const MOCK_SNAPSHOT: &str = r#"{"downloadTotal":0,"uploadTotal":70000,"connections":[{"id":"x","upload":70000,"download":0,"metadata":{"network":"tcp"}}],"memory":1}"#;

    #[tokio::test]
    async fn fetch_clash_connections_parses_snapshot() {
        let mock = MockClashApi::start("topsecret", MOCK_SNAPSHOT).await;
        let connections = tokio::time::timeout(
            Duration::from_secs(10),
            fetch_clash_connections(mock.addr, "topsecret"),
        )
        .await
        .expect("no hang")
        .expect("fetch");
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].id, "x");
        assert_eq!(connections[0].upload, 70000);
        assert_eq!(connections[0].download, 0);
        assert!(connection_starved(
            Duration::from_secs(3600),
            connections[0].upload.max(0).cast_unsigned(),
            connections[0].download.max(0).cast_unsigned(),
        ));
    }

    #[tokio::test]
    async fn fetch_clash_connections_rejects_wrong_secret() {
        let mock = MockClashApi::start("topsecret", MOCK_SNAPSHOT).await;
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            fetch_clash_connections(mock.addr, "wrong"),
        )
        .await
        .expect("no hang");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_clash_connections_fails_on_closed_port() {
        let port = {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
        let result =
            tokio::time::timeout(Duration::from_secs(10), fetch_clash_connections(addr, "x"))
                .await
                .expect("no hang");
        assert!(result.is_err());
    }

    /// Raw HTTP stub: replays one canned response per connection, with an
    /// optional pre-response delay and an optional truncate-and-hang (send
    /// only the first N bytes, then hold the socket open forever).
    struct MockHttp {
        addr: SocketAddr,
        task: tokio::task::JoinHandle<()>,
    }

    impl MockHttp {
        async fn start(response: Vec<u8>, delay: Duration, send_bytes: Option<usize>) -> Self {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("mock binds");
            let addr = listener.local_addr().expect("mock addr");
            let task = tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                while let Ok((mut stream, _)) = listener.accept().await {
                    let response = response.clone();
                    tokio::spawn(async move {
                        let mut head = vec![0u8; 1024];
                        let _ = stream.read(&mut head).await;
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        let end = send_bytes.unwrap_or(response.len()).min(response.len());
                        let _ = stream.write_all(&response[..end]).await;
                        if end < response.len() {
                            futures_util::future::pending::<()>().await;
                        }
                    });
                }
            });
            Self { addr, task }
        }

        fn url(&self) -> String {
            format!("http://{}/", self.addr)
        }
    }

    impl Drop for MockHttp {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("client builds")
    }

    fn outcome(
        status_ok: bool,
        empty_status: bool,
        received: u64,
        complete: bool,
        elapsed_secs: u64,
    ) -> TransferOutcome {
        TransferOutcome {
            status_ok,
            empty_status,
            received,
            complete,
            elapsed: Duration::from_secs(elapsed_secs),
        }
    }

    #[test]
    fn transfer_ok_passes_empty_204_and_capped_bodies() {
        assert!(transfer_ok(
            outcome(true, true, 0, true, 0),
            PROXY_HEALTH_FULL_BPS
        ));
        assert!(transfer_ok(
            outcome(true, false, 2048, true, 0),
            PROXY_HEALTH_FULL_BPS
        ));
        assert!(transfer_ok(
            outcome(true, false, PROXY_HEALTH_CHECK_BODY_BYTES, false, 9),
            PROXY_HEALTH_FULL_BPS
        ));
    }

    #[test]
    fn transfer_ok_judges_small_bodies_by_speed_not_size() {
        // Fast tiny page: data flows, so the proxy is exonerated no matter
        // the size — switching cannot fix upstream content.
        assert!(transfer_ok(
            outcome(true, false, 512, true, 0),
            PROXY_HEALTH_FULL_BPS
        ));
        // Same page delivered slowly: unusable for interactive apps.
        assert!(!transfer_ok(
            outcome(true, false, 512, true, 3),
            PROXY_HEALTH_FULL_BPS
        ));
        assert!(!transfer_ok(
            outcome(true, false, 5000, true, 1),
            PROXY_HEALTH_FULL_BPS
        ));
        // Boundary: exactly the floor passes.
        assert!(transfer_ok(
            outcome(true, false, PROXY_HEALTH_FULL_BPS, true, 1),
            PROXY_HEALTH_FULL_BPS
        ));
        assert!(!transfer_ok(
            outcome(true, false, 16, false, 0),
            PROXY_HEALTH_FULL_BPS
        ));
        assert!(!transfer_ok(
            outcome(false, false, 4096, true, 0),
            PROXY_HEALTH_FULL_BPS
        ));
    }

    #[test]
    fn transfer_ok_degraded_floor_accepts_slow_links() {
        // Degraded pool (nothing else works): a 2KB/s link passes the
        // absolute minimum instead of churning failovers.
        assert!(transfer_ok(
            outcome(true, false, 2048, true, 1),
            PROXY_HEALTH_MIN_BPS
        ));
        // ... but below 1KB/s nothing interactive survives: still fails.
        assert!(!transfer_ok(
            outcome(true, false, 512, true, 3),
            PROXY_HEALTH_MIN_BPS
        ));
        // And the same link fails the full floor.
        assert!(!transfer_ok(
            outcome(true, false, 2048, true, 1),
            PROXY_HEALTH_FULL_BPS
        ));
    }

    #[test]
    fn failover_trips_at_exactly_max_failures() {
        assert!(!failover_tripped(0));
        assert!(!failover_tripped(PROXY_MAX_CONSECUTIVE_FAILURES - 1));
        assert!(failover_tripped(PROXY_MAX_CONSECUTIVE_FAILURES));
        assert!(failover_tripped(PROXY_MAX_CONSECUTIVE_FAILURES + 1));
    }

    #[test]
    fn dwell_holds_recent_switches_unless_pinned() {
        let now = Instant::now();
        assert!(dwell_allows(None, now, false));
        let recent = now.checked_sub(Duration::from_secs(10)).expect("time");
        assert!(!dwell_allows(Some(recent), now, false));
        assert!(dwell_allows(Some(recent), now, true));
        let old = now
            .checked_sub(PROXY_MIN_SWITCH_INTERVAL + Duration::from_secs(1))
            .expect("time");
        assert!(dwell_allows(Some(old), now, false));
    }

    #[tokio::test]
    async fn transfer_outcome_passes_204_empty() {
        let mock = MockHttp::start(
            b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec(),
            Duration::ZERO,
            None,
        )
        .await;
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            transfer_outcome(&test_client(), &mock.url()),
        )
        .await
        .expect("no hang");
        assert!(transfer_ok(outcome, PROXY_HEALTH_FULL_BPS));
    }

    #[tokio::test]
    async fn transfer_outcome_passes_fast_tiny_complete_200() {
        // Instant 5 bytes: data flows, so size alone never fails it.
        let mock = MockHttp::start(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
            Duration::ZERO,
            None,
        )
        .await;
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            transfer_outcome(&test_client(), &mock.url()),
        )
        .await
        .expect("no hang");
        assert!(transfer_ok(outcome, PROXY_HEALTH_FULL_BPS));
    }

    #[tokio::test]
    async fn transfer_outcome_rejects_slow_tiny_complete_200() {
        // Same 5 bytes after a 2s stall (~2 B/s): unusable, must fail.
        let mock = MockHttp::start(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
            Duration::from_secs(2),
            None,
        )
        .await;
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            transfer_outcome(&test_client(), &mock.url()),
        )
        .await
        .expect("no hang");
        assert!(!transfer_ok(outcome, PROXY_HEALTH_FULL_BPS));
    }

    #[tokio::test]
    async fn transfer_outcome_short_circuits_huge_hanging_body() {
        // 256KB arrive, then the socket hangs forever: success must return
        // at the cap without waiting for EOF or the client timeout.
        let cap = usize::try_from(PROXY_HEALTH_CHECK_BODY_BYTES).expect("cap fits pointer");
        let header =
            b"HTTP/1.1 200 OK\r\nContent-Length: 10000000\r\nConnection: close\r\n\r\n".to_vec();
        let body = vec![b'x'; cap + 1024];
        let mut response = header.clone();
        response.extend_from_slice(&body);
        let mock = MockHttp::start(response, Duration::ZERO, Some(header.len() + cap)).await;
        let started = Instant::now();
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            transfer_outcome(&test_client(), &mock.url()),
        )
        .await
        .expect("no hang");
        assert!(transfer_ok(outcome, PROXY_HEALTH_FULL_BPS));
        assert!(
            started.elapsed() < Duration::from_secs(9),
            "cap short-circuit must beat the client timeout"
        );
    }

    #[tokio::test]
    async fn any_transfers_races_slow_urls() {
        // Two URLs stall 3s each, then serve 64KB (~21KB/s, clears the
        // floor): sequential code would take 6s+; the race must finish near
        // a single delay.
        let slow_body =
            b"HTTP/1.1 200 OK\r\nContent-Length: 65536\r\nConnection: close\r\n\r\n".to_vec();
        let mut slow_body = slow_body;
        slow_body.extend(std::iter::repeat_n(b'y', 65536));
        let slow_a = MockHttp::start(slow_body.clone(), Duration::from_secs(3), None).await;
        let slow_b = MockHttp::start(slow_body, Duration::from_secs(3), None).await;
        let url_a = slow_a.url();
        let url_b = slow_b.url();
        let refs = [url_a.as_str(), url_b.as_str()];
        let started = Instant::now();
        let ok = tokio::time::timeout(
            Duration::from_secs(10),
            any_transfers(&test_client(), &refs, PROXY_HEALTH_FULL_BPS),
        )
        .await
        .expect("no hang");
        assert!(ok);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "race must beat the sequential sum"
        );
    }

    #[tokio::test]
    async fn any_transfers_false_only_when_all_fail() {
        let dead_a = "http://127.0.0.1:1/".to_string();
        let dead_b = "http://127.0.0.1:2/".to_string();
        let refs = [dead_a.as_str(), dead_b.as_str()];
        let ok = tokio::time::timeout(
            Duration::from_secs(10),
            any_transfers(&test_client(), &refs, PROXY_HEALTH_FULL_BPS),
        )
        .await
        .expect("no hang");
        assert!(!ok);
    }

    #[tokio::test]
    async fn revive_stays_stopped_without_pool_or_when_disabled() {
        // Hermetic: bogus sing-box path, so any start attempt fails fast
        // with no network and no process.
        let proxy = PersistentProxy::new(
            ProxyConfig {
                enabled: false,
                ..Default::default()
            },
            String::new(),
            None,
        );
        let proxy = Arc::new(Mutex::new(proxy));
        let ranked = Arc::new(RwLock::new(vec![selectable("a", true)]));
        // Disabled: no attempt, still stopped.
        PersistentProxy::revive_if_enabled(&proxy, &ranked).await;
        assert!(!read_proxy_state(&proxy, |state| state.running).await);

        let proxy = PersistentProxy::new(
            ProxyConfig {
                enabled: true,
                ..Default::default()
            },
            String::new(),
            None,
        );
        let proxy = Arc::new(Mutex::new(proxy));
        // Enabled but nothing reachable: no attempt, still stopped.
        let empty = Arc::new(RwLock::new(vec![selectable("a", false)]));
        PersistentProxy::revive_if_enabled(&proxy, &empty).await;
        assert!(!read_proxy_state(&proxy, |state| state.running).await);
        // Enabled with a pool but no binary: attempt fails fast, stays
        // stopped without hanging the loop.
        tokio::time::timeout(
            Duration::from_secs(10),
            PersistentProxy::revive_if_enabled(&proxy, &ranked),
        )
        .await
        .expect("no hang");
        assert!(!read_proxy_state(&proxy, |state| state.running).await);
    }

    #[tokio::test]
    async fn update_stale_clear_marks_pool_degraded() {
        // Everything reachable blacklisted: update clears and stamps the
        // pool degraded (start itself fails here — no binary — but the flag
        // must already be set before the attempt).
        let proxy = PersistentProxy::new(auto_proxy_config(), String::new(), None);
        proxy.state.write().await.failed_config_keys = vec!["a-key".to_string()];
        let ranked = vec![selectable("a", true)];
        proxy.update(&auto_proxy_config(), &ranked, false).await;
        let degraded = proxy.state.read().await.degraded;
        assert!(degraded);
    }

    #[tokio::test]
    async fn failover_clear_marks_pool_degraded() {
        // Same via the failover path: exhausted pool clears and degrades.
        // The lone candidate cannot start (no binary), so failover errors —
        // the flag must survive the failure.
        let proxy = Arc::new(Mutex::new(PersistentProxy::new(
            auto_proxy_config(),
            String::new(),
            None,
        )));
        proxy.lock().await.state.write().await.failed_config_keys = vec!["a-key".to_string()];
        let ranked = Arc::new(RwLock::new(vec![selectable("a", true)]));
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            PersistentProxy::failover(&proxy, &ranked),
        )
        .await
        .expect("no hang");
        assert!(result.is_err());
        let degraded = proxy.lock().await.state.read().await.degraded;
        assert!(degraded);
    }

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

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1", None);

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

        let config = build_sing_box_config(&outbound, 10808, "0.0.0.0", None);
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

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1", None);
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

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1", None);
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

        let config = build_sing_box_config(&outbound, 27910, "127.0.0.1", None);
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
