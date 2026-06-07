mod config;
mod model;
mod parser;
mod paths;
mod probe;
mod server;
mod sing_box;
mod subscription;
mod terminal;
mod tui;

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use chrono::{Local, Utc};
use clap::Parser;
use tokio::{
    fs,
    sync::{RwLock, watch},
    time,
};
use tracing::{error, info, warn};

use crate::{
    config::AppConfig,
    model::{RuntimeConfig, RuntimeState},
    paths::AppPaths,
    probe::probe_candidates,
    server::serve,
    sing_box::{DOWNLOAD_URL, active_probe_needs_setup},
    subscription::load_candidates_with_cache,
    terminal::{print_startup, print_summary},
};

const MAX_TUI_LOGS: usize = 8;

#[derive(Debug, Parser)]
#[command(name = "v2raydar")]
#[command(about = "Fast V2Ray subscription reachability scanner and local top-N endpoint")]
struct Cli {
    #[arg(
        short,
        long,
        help = "Use a specific config file instead of the app folder"
    )]
    config: Option<PathBuf>,

    #[arg(
        long,
        help = "Keep config/data beside the executable in a V2RayDAR folder"
    )]
    portable: bool,

    #[arg(
        long,
        help = "Use plain terminal output instead of the interactive TUI"
    )]
    no_tui: bool,

    #[arg(
        long,
        help = "Run one refresh and print results without starting the endpoint"
    )]
    once: bool,

    #[arg(
        long,
        help = "Remove this app's generated config and cache folder, then exit"
    )]
    uninstall: bool,

    #[arg(long, help = "Skip confirmation for --uninstall")]
    yes: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let default_log_filter = if cli.no_tui || cli.once {
        "v2raydar=info,tower_http=warn"
    } else {
        "v2raydar=off,tower_http=warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_log_filter.into()),
        )
        .init();
    let paths = resolve_paths(&cli)?;

    if cli.uninstall {
        uninstall(&paths, cli.config.is_some(), cli.yes).await?;
        return Ok(());
    }

    paths.ensure().await?;

    if !paths.config_path.exists() {
        AppConfig::write_default(&paths.config_path)?;
        println!("Created default config at {}", paths.config_path.display());
    }

    let mut config = AppConfig::load(&paths.config_path)
        .with_context(|| format!("failed to load config from {}", paths.config_path.display()))?;

    if active_probe_needs_setup(&config, &paths).await {
        if cli.no_tui || cli.once {
            print_sing_box_setup_required(&paths);
            return Ok(());
        }

        tui::run_sing_box_setup(&mut config, &paths).await?;
    }

    let state = Arc::new(RwLock::new(RuntimeState::default()));
    let runtime_config = Arc::new(RwLock::new(RuntimeConfig::from(&config)));

    if cli.once {
        print_startup(&config, &paths);
        refresh_once(
            &config,
            &paths.cache_dir,
            state.clone(),
            runtime_config.clone(),
            true,
        )
        .await?;
        return Ok(());
    }

    if cli.no_tui {
        print_startup(&config, &paths);
        println!(
            "Serving top {} configs at {}",
            config.top_n,
            config.subscription_url("127.0.0.1", false)
        );
        println!(
            "Watching {} for live config changes.",
            paths.config_path.display()
        );
    }

    let (config_tx, config_rx) = watch::channel(config.clone());
    spawn_refresh_loop(
        config_rx,
        paths.cache_dir.clone(),
        state.clone(),
        runtime_config.clone(),
        cli.no_tui,
    );
    spawn_config_watcher(paths.config_path.clone(), config.bind, config_tx);

    if cli.no_tui {
        serve(config.bind, state, runtime_config).await
    } else {
        tokio::select! {
            result = serve(config.bind, state.clone(), runtime_config.clone()) => result,
            result = tui::run(config, paths, state, runtime_config) => result,
        }
    }
}

async fn uninstall(paths: &AppPaths, config_override: bool, assume_yes: bool) -> Result<()> {
    if config_override {
        println!(
            "--uninstall does not remove arbitrary --config directories. Remove {} manually if needed.",
            paths.config_path.display()
        );
        return Ok(());
    }

    if !paths.root_dir.exists() {
        println!(
            "Nothing to remove; {} does not exist.",
            paths.root_dir.display()
        );
        return Ok(());
    }

    if !assume_yes {
        println!(
            "This will permanently remove generated app data: {}",
            paths.root_dir.display()
        );
        print!("Type DELETE to continue: ");
        io::stdout().flush().ok();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if answer.trim() != "DELETE" {
            println!("Uninstall cancelled.");
            return Ok(());
        }
    }

    println!("Removing generated app data: {}", paths.root_dir.display());
    fs::remove_dir_all(&paths.root_dir)
        .await
        .with_context(|| format!("unable to remove {}", paths.root_dir.display()))?;
    println!("Removed generated app data. Delete the V2RayDAR executable manually if desired.");
    Ok(())
}

fn print_sing_box_setup_required(paths: &AppPaths) {
    println!("V2RayDAR active probing requires sing-box before it can refresh.");
    println!("Config: {}", paths.config_path.display());
    println!("Set probe.sing_box_path to the full sing-box executable path.");
    println!("If you use v2rayN, check its installation folder for sing-box.exe.");
    println!("Download sing-box from: {DOWNLOAD_URL}");
    println!("Then run V2RayDAR again.");
}

fn resolve_paths(cli: &Cli) -> Result<AppPaths> {
    if let Some(config_path) = &cli.config {
        return Ok(AppPaths::from_config_override(config_path.clone()));
    }

    if cli.portable {
        return AppPaths::portable();
    }

    AppPaths::installed()
}

async fn refresh_once(
    config: &AppConfig,
    cache_dir: &Path,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    print_terminal_summary: bool,
) -> Result<()> {
    info!("refresh started");
    let started_at = Utc::now();
    let started_instant = std::time::Instant::now();
    *runtime_config.write().await = RuntimeConfig::from(config);
    {
        let mut runtime = state.write().await;
        runtime.refreshing = true;
        runtime.refresh_started_at = Some(started_at.to_rfc3339());
        runtime.last_error = None;
    }

    let fetched = load_candidates_with_cache(config, Some(cache_dir), |bytes| {
        let state = state.clone();
        async move {
            add_fetch_bytes(&state, bytes).await;
        }
    })
    .await?;
    let ranked = if fetched.candidates.is_empty() {
        Vec::new()
    } else {
        probe_candidates(fetched.candidates, &config.probe).await
    };
    let reachable_count = ranked.iter().filter(|item| item.reachable).count();
    let speedtest_bytes = ranked
        .iter()
        .filter_map(|item| item.download_bytes)
        .map(|value| value as u64)
        .sum::<u64>();
    let finished_at = Utc::now();

    let previous = state.read().await.clone();
    let fetch_bytes = previous.fetch_bytes;
    let speedtest_bytes = previous.speedtest_bytes.saturating_add(speedtest_bytes);
    let mut runtime = RuntimeState {
        last_refresh: Some(started_at.to_rfc3339()),
        last_error: None,
        logs: previous.logs,
        refresh_started_at: Some(started_at.to_rfc3339()),
        refresh_finished_at: Some(finished_at.to_rfc3339()),
        refresh_duration_ms: Some(started_instant.elapsed().as_millis()),
        refreshing: false,
        total_candidates: ranked.len(),
        reachable_candidates: reachable_count,
        fetch_bytes,
        speedtest_bytes,
        fetch_errors: fetched.errors,
        ranked,
    };

    let summary = format!(
        "{} Refresh Started; {} Refresh Ended ➔  Checked {} configs, {} reachable, {} fetch errors.",
        started_at.with_timezone(&Local).format("%H:%M:%S"),
        finished_at.with_timezone(&Local).format("%H:%M:%S"),
        runtime.total_candidates,
        runtime.reachable_candidates,
        runtime.fetch_errors.len()
    );
    push_runtime_log(&mut runtime, summary);
    for error in runtime.fetch_errors.clone() {
        push_runtime_log(&mut runtime, format!("fetch error: {error}"));
    }

    if print_terminal_summary {
        print_summary(&runtime, config.top_n);
    }
    *state.write().await = runtime;
    info!("refresh finished");
    Ok(())
}

fn spawn_refresh_loop(
    mut config_rx: watch::Receiver<AppConfig>,
    cache_dir: PathBuf,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    print_terminal_summary: bool,
) {
    tokio::spawn(async move {
        let mut refresh_now = true;
        let mut last_refresh_fingerprint: Option<RefreshFingerprint> = None;

        loop {
            let refresh_seconds = config_rx.borrow().refresh_seconds;
            let current_config = config_rx.borrow().clone();

            if refresh_now {
                refresh_now = false;
                let config = current_config;
                last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                if let Err(err) = refresh_once(
                    &config,
                    &cache_dir,
                    state.clone(),
                    runtime_config.clone(),
                    print_terminal_summary,
                )
                .await
                {
                    error!(error = %err, "initial refresh failed");
                    record_refresh_error(&state, err.to_string()).await;
                }
                continue;
            }

            if refresh_seconds == 0 {
                warn!("automatic refresh is disabled because refresh_seconds is 0");
                if config_rx.changed().await.is_err() {
                    return;
                }
                let config = config_rx.borrow().clone();
                *runtime_config.write().await = RuntimeConfig::from(&config);
                let fingerprint = RefreshFingerprint::from(&config);
                if last_refresh_fingerprint.as_ref() == Some(&fingerprint) {
                    continue;
                }
                last_refresh_fingerprint = Some(fingerprint);
                if let Err(err) = refresh_once(
                    &config,
                    &cache_dir,
                    state.clone(),
                    runtime_config.clone(),
                    print_terminal_summary,
                )
                .await
                {
                    error!(error = %err, "refresh after config reload failed");
                    record_refresh_error(&state, err.to_string()).await;
                }
                continue;
            }

            let sleep = time::sleep(Duration::from_secs(refresh_seconds));
            tokio::pin!(sleep);

            tokio::select! {
                _ = &mut sleep => {
                    let config = config_rx.borrow().clone();
                    last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                    if let Err(err) = refresh_once(&config, &cache_dir, state.clone(), runtime_config.clone(), print_terminal_summary).await {
                        error!(error = %err, "refresh failed");
                        record_refresh_error(&state, err.to_string()).await;
                    }
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }

                    let config = config_rx.borrow().clone();
                    *runtime_config.write().await = RuntimeConfig::from(&config);
                    let fingerprint = RefreshFingerprint::from(&config);
                    if last_refresh_fingerprint.as_ref() == Some(&fingerprint) {
                        continue;
                    }
                    last_refresh_fingerprint = Some(fingerprint);
                    if let Err(err) = refresh_once(&config, &cache_dir, state.clone(), runtime_config.clone(), print_terminal_summary).await {
                        error!(error = %err, "refresh after config reload failed");
                        record_refresh_error(&state, err.to_string()).await;
                    }
                }
            }
        }
    });
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct RefreshFingerprint {
    top_n: usize,
    encoded_subscription: bool,
    fetch_timeout_ms: u64,
    fetch_concurrency: usize,
    max_subscription_bytes: usize,
    probe: crate::config::ProbeConfig,
    subscriptions: Vec<crate::config::SubscriptionSource>,
}

impl From<&AppConfig> for RefreshFingerprint {
    fn from(config: &AppConfig) -> Self {
        Self {
            top_n: config.top_n,
            encoded_subscription: config.encoded_subscription,
            fetch_timeout_ms: config.fetch_timeout_ms,
            fetch_concurrency: config.fetch_concurrency,
            max_subscription_bytes: config.max_subscription_bytes,
            probe: config.probe.clone(),
            subscriptions: config.subscriptions.clone(),
        }
    }
}

fn spawn_config_watcher(
    config_path: PathBuf,
    initial_bind: std::net::SocketAddr,
    config_tx: watch::Sender<AppConfig>,
) {
    tokio::spawn(async move {
        let mut last_modified = modified_time(&config_path).await.ok();

        loop {
            time::sleep(Duration::from_secs(1)).await;
            let modified = match modified_time(&config_path).await {
                Ok(value) => value,
                Err(err) => {
                    warn!(
                        path = %config_path.display(),
                        error = %err,
                        "unable to stat config file"
                    );
                    continue;
                }
            };

            if last_modified == Some(modified) {
                continue;
            }

            last_modified = Some(modified);
            match AppConfig::load(&config_path) {
                Ok(config) => {
                    if config.bind != initial_bind {
                        warn!(
                            configured_bind = %config.bind,
                            active_bind = %initial_bind,
                            "config bind changed; restart V2RayDAR to apply the HTTP bind address"
                        );
                    }

                    if config_tx.send(config).is_err() {
                        return;
                    }

                    info!(path = %config_path.display(), "config file reloaded");
                }
                Err(err) => {
                    warn!(
                        path = %config_path.display(),
                        error = %err,
                        "config reload failed; keeping previous valid config"
                    );
                }
            }
        }
    });
}

async fn modified_time(path: &Path) -> Result<SystemTime> {
    let metadata = fs::metadata(path)
        .await
        .with_context(|| format!("unable to read metadata for {}", path.display()))?;
    metadata
        .modified()
        .with_context(|| format!("unable to read modification time for {}", path.display()))
}

async fn record_refresh_error(state: &Arc<RwLock<RuntimeState>>, error: String) {
    let mut state = state.write().await;
    state.last_error = Some(error.clone());
    state.refreshing = false;
    state.refresh_finished_at = Some(Utc::now().to_rfc3339());
    state.total_candidates = 0;
    state.reachable_candidates = 0;
    state.fetch_errors = vec![error.clone()];
    state.ranked.clear();
    push_runtime_log(&mut state, format!("refresh error: {error}"));
}

async fn add_fetch_bytes(state: &Arc<RwLock<RuntimeState>>, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let mut state = state.write().await;
    state.fetch_bytes = state.fetch_bytes.saturating_add(bytes);
}

fn push_runtime_log(state: &mut RuntimeState, message: String) {
    state.logs.push(message);
    if state.logs.len() > MAX_TUI_LOGS {
        let extra = state.logs.len() - MAX_TUI_LOGS;
        state.logs.drain(0..extra);
    }
}

impl From<&AppConfig> for RuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            bind: config.bind,
            top_n: config.top_n,
            refresh_seconds: config.refresh_seconds,
            encoded_subscription: config.encoded_subscription,
            fetch_timeout_ms: config.fetch_timeout_ms,
            fetch_concurrency: config.fetch_concurrency,
            max_subscription_bytes: config.max_subscription_bytes,
            sharing_enabled: config.sharing.enabled,
            require_token: config.sharing.require_token,
            token: config.sharing.token.clone(),
            probe_mode: format!("{:?}", config.probe.mode).to_ascii_lowercase(),
            speedtest_enabled: config
                .probe
                .download_url
                .as_deref()
                .is_some_and(|url| !url.trim().is_empty()),
            probe_concurrency: config.probe.concurrency,
            active_timeout_ms: config.probe.active_timeout_ms,
            startup_timeout_ms: config.probe.startup_timeout_ms,
            test_url: config.probe.test_url.clone(),
            accepted_statuses: config.probe.accepted_statuses.clone(),
            download_bytes_limit: config.probe.download_bytes_limit,
            subscription_count: config.subscriptions.len(),
            enabled_subscription_count: config
                .subscriptions
                .iter()
                .filter(|source| source.enabled)
                .count(),
        }
    }
}
