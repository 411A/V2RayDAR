mod config;
mod model;
mod parser;
mod probe;
mod server;
mod subscription;
mod terminal;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use chrono::Utc;
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
    probe::probe_candidates,
    server::serve,
    subscription::load_candidates,
    terminal::print_summary,
};

#[derive(Debug, Parser)]
#[command(name = "v2raydar")]
#[command(about = "Fast V2Ray subscription reachability scanner and local top-N endpoint")]
struct Cli {
    #[arg(short, long, default_value = "configs.yaml")]
    config: PathBuf,

    #[arg(
        long,
        help = "Run one refresh and print results without starting the endpoint"
    )]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "v2raydar=info,tower_http=warn".into()),
        )
        .init();

    let cli = Cli::parse();
    let config = AppConfig::load(&cli.config)
        .with_context(|| format!("failed to load config from {}", cli.config.display()))?;

    let state = Arc::new(RwLock::new(RuntimeState::default()));
    let runtime_config = Arc::new(RwLock::new(RuntimeConfig::from(&config)));
    refresh_once(&config, state.clone(), runtime_config.clone()).await?;

    if cli.once {
        return Ok(());
    }

    let endpoint = format!("http://{}/subscription", config.bind);
    println!("Serving top {} configs at {}", config.top_n, endpoint);
    println!("Watching {} for live config changes.", cli.config.display());

    let (config_tx, config_rx) = watch::channel(config.clone());
    spawn_refresh_loop(config_rx, state.clone(), runtime_config.clone());
    spawn_config_watcher(cli.config.clone(), config.bind, config_tx);

    serve(config.bind, state, runtime_config).await
}

async fn refresh_once(
    config: &AppConfig,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
) -> Result<()> {
    info!("refresh started");
    let started_at = Utc::now();
    *runtime_config.write().await = RuntimeConfig::from(config);
    let fetched = load_candidates(config).await?;
    let ranked = probe_candidates(fetched.candidates, &config.probe).await;
    let reachable_count = ranked.iter().filter(|item| item.reachable).count();

    let runtime = RuntimeState {
        last_refresh: Some(started_at.to_rfc3339()),
        last_error: None,
        total_candidates: ranked.len(),
        reachable_candidates: reachable_count,
        fetch_errors: fetched.errors,
        ranked,
    };

    print_summary(&runtime, config.top_n);
    *state.write().await = runtime;
    info!("refresh finished");
    Ok(())
}

fn spawn_refresh_loop(
    mut config_rx: watch::Receiver<AppConfig>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
) {
    tokio::spawn(async move {
        loop {
            let refresh_seconds = config_rx.borrow().refresh_seconds;

            if refresh_seconds == 0 {
                warn!("automatic refresh is disabled because refresh_seconds is 0");
                if config_rx.changed().await.is_err() {
                    return;
                }
                let config = config_rx.borrow().clone();
                if let Err(err) = refresh_once(&config, state.clone(), runtime_config.clone()).await
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
                    if let Err(err) = refresh_once(&config, state.clone(), runtime_config.clone()).await {
                        error!(error = %err, "refresh failed");
                        record_refresh_error(&state, err.to_string()).await;
                    }
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }

                    let config = config_rx.borrow().clone();
                    if let Err(err) = refresh_once(&config, state.clone(), runtime_config.clone()).await {
                        error!(error = %err, "refresh after config reload failed");
                        record_refresh_error(&state, err.to_string()).await;
                    }
                }
            }
        }
    });
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
    state.last_error = Some(error);
}

impl From<&AppConfig> for RuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            bind: config.bind,
            top_n: config.top_n,
            refresh_seconds: config.refresh_seconds,
            encoded_subscription: config.encoded_subscription,
            probe_mode: format!("{:?}", config.probe.mode).to_ascii_lowercase(),
        }
    }
}
