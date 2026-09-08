mod clash;
mod config;
mod constants;
mod convert;
mod db;
mod geoip;
mod model;
mod network;
mod parser;
mod paths;
mod probe;
mod proxy;
mod qr;
mod server;
mod sing_box;
mod subscription;
mod terminal;
mod tui;

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, anyhow};
use chrono::{Local, Utc};
use clap::Parser;
use tokio::{
    fs,
    sync::{RwLock, mpsc, watch},
    time,
};
use tracing::{debug, error, info, warn};

use crate::{
    config::{AppConfig, ProbeMode},
    constants::{
        APP_DATA_DIR_NAME, APP_NAME, CACHE_DIR_NAME, CONFIG_FILE_NAME, CONFIG_WATCH_INTERVAL,
        DB_FILE_NAME, DEFAULT_LOG_FILTER_PLAIN, DEFAULT_LOG_FILTER_TUI, DEFAULT_LOG_FILTER_VERBOSE,
        FIREWALL_STATE_FILE_NAME, GEOIP_DIR_NAME, GEOIP_MMDB_FILE_NAME,
        LEGACY_APP_MARKER_FILE_NAME, LEGACY_CACHE_MARKER_FILE_NAME, LOCALHOST_IP, MAX_TUI_LOGS,
        sing_box_download_url,
    },
    db::Database,
    model::{Candidate, ProbeStopPolicy, ProgressEvent, RankedConfig, RuntimeConfig, RuntimeState},
    paths::AppPaths,
    probe::{ping_configs, probe_candidates},
    server::serve,
    sing_box::{
        active_probe_needs_setup, apply_runtime_sing_box_path, recommended_version, setup_guide,
    },
    subscription::{
        FetchFailure, FetchOutcome, load_candidates_with_cache, retry_failed_sources_with_proxy,
    },
    terminal::{PlainProgressReporter, print_log, print_startup, print_summary},
};

/// Preconfigured TLS for Android where rustls-platform-verifier can't initialize.
static FALLBACK_TLS: std::sync::OnceLock<rustls::ClientConfig> = std::sync::OnceLock::new();

fn build_tls_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(
        rustls::crypto::aws_lc_rs::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("TLS protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth()
}

#[derive(Debug, Parser)]
#[command(name = "v2raydar", version)]
#[command(about = "Fast V2Ray subscription reachability scanner and local top-N endpoint")]
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    #[arg(
        short,
        long,
        help = "Use a specific config file; app cache/state stays in the sibling data folder"
    )]
    config: Option<PathBuf>,

    #[arg(long, help = "Keep the data folder beside the executable")]
    portable: bool,

    #[arg(
        long,
        help = "Use plain terminal output instead of the interactive TUI"
    )]
    no_tui: bool,

    #[arg(long, help = "Show detailed fetch/probe logs in plain terminal output")]
    verbose: bool,

    #[arg(
        long,
        help = "Run one refresh and print results without starting the endpoint"
    )]
    once: bool,

    #[arg(
        long,
        help = "Remove this app's generated data folder and owned firewall rules, then exit"
    )]
    uninstall: bool,

    #[arg(long, help = "Skip confirmation for --uninstall")]
    yes: bool,

    #[arg(
        long,
        help = "Ping config URIs and print latency results",
        num_args = 1..
    )]
    ping: Vec<String>,

    #[arg(
        long,
        help = "Ping config URIs read from a file (one per line)",
        conflicts_with = "ping"
    )]
    ping_file: Option<PathBuf>,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    // On Android builds, pre-build a rustls config with webpki-roots to bypass
    // the platform verifier which requires JNI context unavailable in Termux.
    // On non-Android builds the platform verifier works correctly with system CAs.
    if cfg!(target_os = "android") {
        let _ = FALLBACK_TLS.set(build_tls_config());
    }

    let cli = Cli::parse();
    if cli.no_tui || cli.once {
        let filter = if cli.verbose {
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG_FILTER_VERBOSE.into())
        } else {
            tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER_PLAIN)
        };
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER_TUI))
            .with_writer(io::sink)
            .init();
    }
    let paths = resolve_paths(&cli)?;

    if cli.uninstall {
        uninstall(&paths, cli.yes).await?;
        return Ok(());
    }

    if paths.config_path.exists() {
        paths.ensure().await?;
    } else {
        if !paths.generated_config {
            return Err(anyhow!(
                "config file does not exist: {}; create it or omit --config to use {}",
                paths.config_path.display(),
                paths.root_dir.join(CONFIG_FILE_NAME).display()
            ));
        }

        paths.ensure().await?;
        AppConfig::write_default(&paths.config_path)?;
        println!("Created default config at {}", paths.config_path.display());
    }

    let mut config = load_config_and_persist_generated_token(&paths.config_path)
        .with_context(|| format!("failed to load config from {}", paths.config_path.display()))?;

    // Backfill settings added by newer versions into older configs.yaml files.
    // Add-only and skipped when nothing is missing, so the watcher never loops.
    match tui::util::backfill_missing_defaults(&paths.config_path) {
        Ok(0) => {}
        Ok(added) => info!(
            added,
            path = %paths.config_path.display(),
            "backfilled missing config defaults"
        ),
        Err(err) => warn!(
            error = %err,
            path = %paths.config_path.display(),
            "config backfill skipped; in-memory defaults still apply"
        ),
    }

    // Initialize GeoIP: MaxMind database first (most accurate), ipdeny
    // country zones as fallback. Both are refreshed by the installer
    // independently of app releases; the app itself never downloads anything.
    // Explicit `geoip_db_path` (file or directory) wins, otherwise
    // `<root_dir>/geoip` (+ its `GeoLite2-Country.mmdb`) is used.
    let configured_geoip = config.geoip_db_path.as_ref().map(std::path::PathBuf::from);
    let mmdb_path = configured_geoip
        .clone()
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("mmdb"))
        })
        .or_else(|| {
            let bundled = paths
                .root_dir
                .join(GEOIP_DIR_NAME)
                .join(GEOIP_MMDB_FILE_NAME);
            if bundled.is_file() {
                Some(bundled)
            } else {
                let legacy = paths.root_dir.join(GEOIP_MMDB_FILE_NAME);
                if legacy.is_file() { Some(legacy) } else { None }
            }
        });
    let zone_dir = configured_geoip.filter(|path| path.is_dir()).or_else(|| {
        let dir = paths.root_dir.join(GEOIP_DIR_NAME);
        if dir.is_dir() { Some(dir) } else { None }
    });
    crate::geoip::init(mmdb_path.as_deref(), zone_dir.as_deref());

    if active_probe_needs_setup(&config, &paths).await {
        if cli.no_tui || cli.once {
            print_sing_box_setup_required(&paths);
            return Ok(());
        }

        tui::run_sing_box_setup(&mut config, &paths).await?;
    }

    let state = Arc::new(RwLock::new(RuntimeState::default()));
    let runtime_config = Arc::new(RwLock::new(RuntimeConfig::from(&config)));

    let db_path = paths.root_dir.join(DB_FILE_NAME);
    let database = Arc::new(
        Database::open(&db_path)
            .with_context(|| format!("failed to open database at {}", db_path.display()))?,
    );

    if cli.once {
        print_startup(&config, &paths, cli.verbose);
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        refresh_once(
            &config,
            database.clone(),
            state.clone(),
            runtime_config.clone(),
            cycle,
            true,
            !cli.verbose,
            None,
            true,
        )
        .await?;
        return Ok(());
    }

    if !cli.ping.is_empty() || cli.ping_file.is_some() {
        let uris = if cli.ping.is_empty() {
            let path = cli.ping_file.as_ref().expect("ping_file is Some");
            fs::read_to_string(path)
                .await
                .with_context(|| format!("failed to read ping file: {}", path.display()))?
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect()
        } else {
            cli.ping
        };
        let probe_config = config.probe.clone();
        let results = ping_configs(uris, &probe_config).await;
        terminal::print_ping_results(&results);
        return Ok(());
    }

    if cli.no_tui {
        print_startup(&config, &paths, cli.verbose);
        println!(
            "Serving top {} configs at {}",
            config.top_n,
            config.subscription_url(LOCALHOST_IP, true)
        );
        println!(
            "Watching {} for live config changes.",
            paths.config_path.display()
        );
    }

    let shared_ranked: Arc<RwLock<Vec<RankedConfig>>> = Arc::new(RwLock::new(Vec::new()));
    let (proxy_log_tx, mut proxy_log_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let shared = Arc::new(tokio::sync::Mutex::new(proxy::PersistentProxy::new(
        config.proxy.clone(),
        config.probe.sing_box_path.clone(),
        Some(proxy_log_tx),
    )));

    // Drain proxy log events into TUI live logs
    {
        let state = state.clone();
        let previous_top_n = HashSet::new();
        tokio::spawn(async move {
            while let Some(event) = proxy_log_rx.recv().await {
                push_tui_progress(&state, event, &previous_top_n, true).await;
            }
        });
    }

    if config.proxy.enabled
        && config.proxy.discoverable
        && let Err(err) = crate::tui::firewall::apply(
            &paths.root_dir,
            true,
            config.proxy.port,
            constants::FIREWALL_PROXY_RULE_NAME,
        )
    {
        tracing::warn!(error = %err, "failed to add proxy firewall rule");
    }

    proxy::spawn_health_loop(shared.clone(), shared_ranked.clone());
    proxy::spawn_starvation_loop(shared.clone(), shared_ranked.clone());
    let proxy = shared;

    let (config_tx, config_rx) = watch::channel(config.clone());
    // Serializes fetch and ping cycles so ranked/counter writes never interleave.
    let cycle = Arc::new(tokio::sync::Mutex::new(()));
    // Manual cycle triggers (TUI Ctrl+R / Ctrl+P, `:refresh`, `:ping`).
    let (refresh_trigger_tx, refresh_trigger_rx) = mpsc::unbounded_channel::<()>();
    let (ping_trigger_tx, ping_trigger_rx) = mpsc::unbounded_channel::<()>();
    // Tells the ping loop to restart its sleep after every successful fetch:
    // the refresh just revalidated everything, so the ping countdown restarts
    // full instead of resuming a stale partial interval.
    let (ping_restart_tx, ping_restart_rx) = mpsc::unbounded_channel::<()>();
    // Refresh preempts ping through this switch (see `refresh_once`): set
    // when a fetch comes due mid-ping, cleared when the refresh takes over.
    let ping_cancel = Arc::new(AtomicBool::new(false));
    spawn_refresh_loop(
        config_rx.clone(),
        database.clone(),
        state.clone(),
        runtime_config.clone(),
        proxy.clone(),
        shared_ranked.clone(),
        cycle.clone(),
        refresh_trigger_rx,
        ping_restart_tx,
        ping_cancel.clone(),
        cli.no_tui,
        cli.no_tui && !cli.verbose,
    );
    spawn_ping_loop(
        config_rx,
        database.clone(),
        state.clone(),
        runtime_config.clone(),
        proxy.clone(),
        shared_ranked,
        cycle,
        ping_trigger_rx,
        ping_restart_rx,
        ping_cancel,
        cli.no_tui && !cli.verbose,
    );
    spawn_config_watcher(paths.config_path.clone(), config.bind, config_tx.clone());

    let result = if cli.no_tui {
        // No TUI owns the senders headless; hold them so trigger channels stay open.
        let _manual_triggers = (refresh_trigger_tx, ping_trigger_tx);
        serve(config.bind, state, runtime_config).await
    } else {
        tokio::select! {
            result = serve(config.bind, state.clone(), runtime_config.clone()) => result,
            result = tui::run(config, paths, state, runtime_config, database.clone(), config_tx, refresh_trigger_tx, ping_trigger_tx) => result,
        }
    };

    proxy.lock().await.shutdown().await;

    result
}

async fn uninstall(paths: &AppPaths, assume_yes: bool) -> Result<()> {
    let targets = uninstall_targets(paths).await?;
    let firewall_cleanup = has_firewall_cleanup(paths);
    if targets.is_empty() && !firewall_cleanup {
        println!(
            "Nothing to remove; no V2RayDAR-owned files were found for {}.",
            paths.root_dir.display()
        );
        return Ok(());
    }

    if !assume_yes {
        println!("This will permanently remove V2RayDAR-owned app data and firewall rules:");
        for target in &targets {
            println!("  {}", target.display());
        }
        if firewall_cleanup {
            println!("  V2RayDAR-owned firewall rules");
        }
        print!("Type DELETE to continue: ");
        io::stdout().flush().ok();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if answer.trim() != "DELETE" {
            println!("Uninstall cancelled.");
            return Ok(());
        }
    }

    if firewall_cleanup {
        for message in tui::remove_owned_firewall_rules(&paths.root_dir)? {
            println!("Firewall cleanup: {message}");
        }
    }

    for target in targets {
        remove_uninstall_target(&target).await?;
        println!("Removed {}", target.display());
    }

    println!(
        "V2RayDAR uninstall cleanup finished. Delete the V2RayDAR executable manually if desired."
    );
    Ok(())
}

async fn uninstall_targets(paths: &AppPaths) -> Result<Vec<PathBuf>> {
    if !paths.root_dir.exists() {
        return Ok(Vec::new());
    }
    if let Some(app_dir) = installed_app_dir_uninstall_target(paths).await? {
        return Ok(vec![app_dir]);
    }
    if contains_only_known_app_artifacts(&paths.root_dir).await? {
        return Ok(vec![paths.root_dir.clone()]);
    }

    known_app_root_targets(paths).await
}

async fn installed_app_dir_uninstall_target(paths: &AppPaths) -> Result<Option<PathBuf>> {
    if paths.portable
        || !paths.generated_config
        || paths.root_dir.file_name().and_then(|name| name.to_str()) != Some(APP_DATA_DIR_NAME)
        || !contains_only_known_app_artifacts(&paths.root_dir).await?
    {
        return Ok(None);
    }

    let Some(app_dir) = paths.root_dir.parent() else {
        return Ok(None);
    };
    if app_dir.file_name().and_then(|name| name.to_str()) != Some(APP_NAME) {
        return Ok(None);
    }
    if app_dir_contains_only_data_root(app_dir).await? {
        Ok(Some(app_dir.to_path_buf()))
    } else {
        Ok(None)
    }
}

async fn app_dir_contains_only_data_root(path: &Path) -> Result<bool> {
    let mut entries = fs::read_dir(path)
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?;
    let mut found_data_root = false;
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?
    {
        let file_name = entry.file_name();
        if file_name.to_str() != Some(APP_DATA_DIR_NAME) {
            return Ok(false);
        }

        let file_type = entry
            .file_type()
            .await
            .with_context(|| format!("unable to inspect {}", entry.path().display()))?;
        if !file_type.is_dir() {
            return Ok(false);
        }
        found_data_root = true;
    }

    Ok(found_data_root)
}

async fn contains_only_known_app_artifacts(path: &Path) -> Result<bool> {
    let mut entries = fs::read_dir(path)
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?
    {
        if !is_known_app_root_entry(&entry).await? {
            return Ok(false);
        }
    }

    Ok(true)
}

async fn is_known_app_root_entry(entry: &fs::DirEntry) -> Result<bool> {
    let file_name = entry.file_name();
    let Some(name) = file_name.to_str() else {
        return Ok(false);
    };

    let file_type = entry
        .file_type()
        .await
        .with_context(|| format!("unable to inspect {}", entry.path().display()))?;

    match name {
        CONFIG_FILE_NAME
        | FIREWALL_STATE_FILE_NAME
        | LEGACY_APP_MARKER_FILE_NAME
        | DB_FILE_NAME => Ok(file_type.is_file()),
        CACHE_DIR_NAME => Ok(file_type.is_dir() && is_known_cache_dir(&entry.path()).await?),
        GEOIP_DIR_NAME => Ok(file_type.is_dir()),
        _ => Ok(false),
    }
}

async fn known_app_root_targets(paths: &AppPaths) -> Result<Vec<PathBuf>> {
    let mut targets = Vec::new();
    push_existing(&mut targets, paths.root_dir.join(CONFIG_FILE_NAME));
    push_existing(
        &mut targets,
        paths.root_dir.join(LEGACY_APP_MARKER_FILE_NAME),
    );
    push_existing(&mut targets, paths.root_dir.join(DB_FILE_NAME));
    if is_known_cache_dir(&paths.cache_dir).await? {
        targets.push(paths.cache_dir.clone());
    } else {
        targets.extend(known_cache_file_targets(&paths.cache_dir).await?);
    }
    push_existing(&mut targets, paths.root_dir.join(FIREWALL_STATE_FILE_NAME));
    Ok(targets)
}

fn push_existing(targets: &mut Vec<PathBuf>, path: PathBuf) {
    if path.exists() {
        targets.push(path);
    }
}

fn has_firewall_cleanup(paths: &AppPaths) -> bool {
    cfg!(target_os = "windows") || paths.root_dir.join(FIREWALL_STATE_FILE_NAME).exists()
}

async fn is_known_cache_dir(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }

    let mut entries = fs::read_dir(path)
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?
    {
        if !is_known_cache_entry(&entry).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn known_cache_file_targets(path: &Path) -> Result<Vec<PathBuf>> {
    if !path.is_dir() {
        return Ok(Vec::new());
    }

    let mut targets = Vec::new();
    let mut entries = fs::read_dir(path)
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?
    {
        if is_known_cache_entry(&entry).await? {
            targets.push(entry.path());
        }
    }
    targets.sort();

    Ok(targets)
}

async fn is_known_cache_entry(entry: &fs::DirEntry) -> Result<bool> {
    let file_name = entry.file_name();
    let Some(name) = file_name.to_str() else {
        return Ok(false);
    };
    let file_type = entry
        .file_type()
        .await
        .with_context(|| format!("unable to inspect {}", entry.path().display()))?;

    Ok(file_type.is_file() && (name == LEGACY_CACHE_MARKER_FILE_NAME || name == DB_FILE_NAME))
}

async fn remove_uninstall_target(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .await
        .with_context(|| format!("unable to inspect {}", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .await
            .with_context(|| format!("unable to remove {}", path.display()))
    } else {
        fs::remove_file(path)
            .await
            .with_context(|| format!("unable to remove {}", path.display()))
    }
}

fn print_sing_box_setup_required(paths: &AppPaths) {
    let guide = setup_guide();

    println!("V2RayDAR active probing requires sing-box before it can refresh.");
    println!("Config: {}", paths.config_path.display());
    println!("Detected OS: {}", guide.platform);
    println!("Recommended sing-box version: v{}", recommended_version());
    println!("Download: {}", sing_box_download_url());
    println!("Choose the release asset: {}", guide.release_asset);
    println!("Use the executable named: {}", guide.executable_name);
    println!("Embedded desktop builds also work when the executable is beside V2RayDAR.");
    println!("Set probe.sing_box_path to the executable path or a working PATH command.");
    println!("Examples:");
    for path in guide.example_paths {
        println!("  {path}");
    }
    println!("Notes:");
    for note in guide.notes {
        println!("  {note}");
    }
    println!("Then run V2RayDAR again.");
}

/// Working results kept from a ping that a refresh preempted.
///
/// The refresh skips re-probing these (verified seconds ago) and only
/// gathers the shortfall to `top_n`; counters continue from the carried
/// values instead of resetting to zero mid-cycle.
struct PingCarry {
    working: Vec<RankedConfig>,
    tested: usize,
    reachable: usize,
}

/// Snapshot the live ping partials for a refresh that is about to preempt it.
/// Called after the cycle lock is held, so the ping's final write is done.
async fn take_ping_carry(state: &Arc<RwLock<RuntimeState>>) -> PingCarry {
    let runtime = state.read().await;
    PingCarry {
        working: runtime
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .cloned()
            .collect(),
        tested: runtime.tested_candidates,
        reachable: runtime.reachable_candidates,
    }
}

#[allow(clippy::too_many_arguments)]
async fn probe_refresh_candidates(
    candidates: Vec<Candidate>,
    config: &AppConfig,
    previous_top_n: &HashSet<String>,
    state: &Arc<RwLock<RuntimeState>>,
    progress_tx: &mpsc::UnboundedSender<ProgressEvent>,
    print_compact_progress: bool,
    label: &str,
    carry: Option<&PingCarry>,
    cancel: Option<Arc<AtomicBool>>,
) -> Vec<RankedConfig> {
    let candidate_count = candidates.len();
    if candidates.is_empty() {
        let message = if label == "Probe" {
            "Probe skipped: no configs were loaded.".to_string()
        } else {
            format!("{label} skipped: no new configs were loaded.")
        };
        info!(label, "probe skipped because no candidates were loaded");
        if print_compact_progress {
            print_log(&message);
        }
        push_tui_progress(
            state,
            ProgressEvent::LiveLog(message.trim_end_matches('.').to_string()),
            &HashSet::new(),
            true,
        )
        .await;
        return Vec::new();
    }

    info!(
        candidates = candidate_count,
        mode = ?config.probe.mode,
        label,
        "probe started"
    );
    if print_compact_progress {
        print_log(format!(
            "{label} started: {candidate_count} candidates with {} mode.",
            format!("{:?}", config.probe.mode).to_ascii_lowercase()
        ));
    }
    push_tui_progress(
        state,
        ProgressEvent::LiveLog(format!(
            "{label} started: {candidate_count} candidates with {:?}",
            config.probe.mode
        )),
        &HashSet::new(),
        true,
    )
    .await;

    let stop_policy = probe_stop_policy(config, previous_top_n, &candidates, carry);
    probe_candidates(
        candidates,
        &config.probe,
        Some(progress_tx.clone()),
        &stop_policy,
        cancel,
    )
    .await
}

fn subscription_retry_proxy_uri(
    config: &AppConfig,
    ranked: &[RankedConfig],
    previous: &RuntimeState,
) -> Option<String> {
    if config.probe.mode != ProbeMode::Active || config.probe.sing_box_path.trim().is_empty() {
        return None;
    }

    if let Some(uri) = config.emergency_config.as_deref() {
        let uri = uri.trim();
        if !uri.is_empty() {
            return Some(uri.to_string());
        }
    }

    ranked
        .iter()
        .chain(previous.ranked.iter())
        .find(|item| item.reachable)
        .map(|item| item.uri.clone())
}

fn merged_fetch_errors(
    initial_failures: &[FetchFailure],
    retry: Option<&FetchOutcome>,
) -> Vec<String> {
    let Some(retry) = retry else {
        return initial_failures
            .iter()
            .map(|failure| failure.error.clone())
            .collect();
    };

    let mut errors = Vec::new();
    for initial in initial_failures {
        if retry
            .successes
            .iter()
            .any(|source| source == &initial.source)
        {
            continue;
        }

        if let Some(retry_failure) = retry
            .failures
            .iter()
            .find(|failure| failure.source == initial.source)
        {
            errors.push(retry_failure.error.clone());
        } else {
            errors.push(initial.error.clone());
        }
    }

    errors
}

fn resolve_paths(cli: &Cli) -> Result<AppPaths> {
    if let Some(config_path) = &cli.config {
        return Ok(AppPaths::from_config_override(config_path.clone()));
    }

    if cli.portable || AppPaths::auto_detect_portable() {
        return AppPaths::portable();
    }

    AppPaths::installed()
}

fn load_config_and_persist_generated_token(path: &Path) -> Result<AppConfig> {
    let (mut config, generated_token_requested) = AppConfig::load_with_generated_token_flag(path)?;
    if generated_token_requested {
        tui::util::save_config(path, &config)?;
    }
    apply_runtime_sing_box_path(&mut config);
    Ok(config)
}

/// Persist probed configs and stable top-N keys; optionally clean offline rows.
/// Shared by the fetch cycle (cleanup on) and the ping cycle (cleanup off,
/// since the fetch cycle already handles it on its own cadence).
async fn persist_ranked_configs(
    database: &Arc<Database>,
    ranked: &[RankedConfig],
    top_n: usize,
    clean_offlines_after_days: u32,
    clean_offlines: bool,
) -> Result<()> {
    {
        let db = database.clone();
        let configs = ranked.to_vec();
        tokio::task::spawn_blocking(move || {
            db.upsert_configs(&configs)?;
            if configs.is_empty() {
                db.delete_stable_top_keys()?;
            } else {
                let keys: Vec<String> = configs
                    .iter()
                    .filter(|c| c.reachable)
                    .take(top_n)
                    .map(|c| c.dedup_key.clone())
                    .collect();
                if keys.is_empty() {
                    db.delete_stable_top_keys()?;
                } else {
                    db.save_stable_top_keys(&keys)?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("{e}"))?
        .context("failed to persist configs to database")?;
    }

    if clean_offlines {
        let db = database.clone();
        tokio::task::spawn_blocking(move || db.clean_offline_configs(clean_offlines_after_days))
            .await
            .map_err(|e| anyhow!("{e}"))?
            .map(|deleted| {
                if deleted > 0 {
                    info!(deleted, "cleaned offline configs from database");
                }
            })
            .context("failed to clean offline configs")?;
    }
    Ok(())
}

/// Cycle origin marker for Recent Logs: 🤖 automatic (timer, startup,
/// config reload), 👤 manual (TUI chord/command, CLI invocation).
fn cycle_actor(manual: bool) -> &'static str {
    if manual { "👤" } else { "🤖" }
}

#[allow(
    clippy::significant_drop_tightening,
    clippy::too_many_lines,
    clippy::too_many_arguments
)]
async fn refresh_once(
    config: &AppConfig,
    database: Arc<Database>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    cycle: Arc<tokio::sync::Mutex<()>>,
    print_terminal_summary: bool,
    print_compact_progress: bool,
    ping_cancel: Option<Arc<AtomicBool>>,
    manual: bool,
) -> Result<()> {
    // Refresh is always prioritized over ping: when a fetch comes due while
    // a ping is still running, stop the ping and keep the working results it
    // gathered (carried below) instead of resetting the count. `refreshing`
    // flips at once so the TUI shows running, not pinging, while the ping
    // winds down; the cycle lock below still serializes the actual work.
    if ping_cancel.as_ref().is_some_and(|cancel| {
        cancel.load(AtomicOrdering::SeqCst) || state.try_read().is_ok_and(|runtime| runtime.pinging)
    }) {
        if let Some(cancel) = &ping_cancel {
            cancel.store(true, AtomicOrdering::SeqCst);
        }
        // Footprint for the ping's "preempted by refresh" line: anyone
        // doubting a preemption can match it against this entry's timestamp.
        info!("refresh preempting running ping; partials will be carried over");
        push_tui_progress(
            &state,
            ProgressEvent::LiveLog(timestamped_log(
                "Refresh preempting running ping".to_string(),
            )),
            &HashSet::new(),
            true,
        )
        .await;
        state.write().await.refreshing = true;
    }
    let _cycle_guard = cycle.lock().await;
    // The ping's final write is done once the lock is held: snapshot its
    // partials (if it was asked to stop) and clear the request.
    let carry: Option<PingCarry> = if ping_cancel
        .as_ref()
        .is_some_and(|cancel| cancel.swap(false, AtomicOrdering::SeqCst))
    {
        Some(take_ping_carry(&state).await)
    } else {
        None
    };
    info!(
        enabled_subscriptions = config
            .subscriptions
            .iter()
            .filter(|source| source.enabled)
            .count(),
        fetch_concurrency = config.fetch_concurrency,
        fetch_timeout_ms = config.fetch_timeout_ms,
        probe_mode = ?config.probe.mode,
        probe_concurrency = config.probe.concurrency,
        active_timeout_ms = config.probe.active_timeout_ms,
        startup_timeout_ms = config.probe.startup_timeout_ms,
        "refresh started"
    );
    let started_at = Utc::now();
    let started_instant = std::time::Instant::now();
    let previous_before_refresh = state.read().await.clone();
    let previous_top_n = if config.prioritize_stability {
        let db = database.clone();
        tokio::task::spawn_blocking(move || db.load_stable_top_keys())
            .await
            .map_err(|e| anyhow!("{e}"))?
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    let (progress_tx, progress_task) = spawn_tui_progress_forwarder(
        state.clone(),
        previous_top_n.clone(),
        print_compact_progress,
        true,
    );
    if print_compact_progress {
        print_log(format!(
            "Refresh started at {}.",
            started_at.with_timezone(&Local).format("%H:%M:%S")
        ));
    }
    *runtime_config.write().await = RuntimeConfig::from(config);
    let refresh_started_log = timestamped_log(format!(
        "Refresh started at {}",
        started_at.with_timezone(&Local).format("%H:%M:%S")
    ));
    {
        let mut runtime = state.write().await;
        runtime.refreshing = true;
        runtime.refresh_started_at = Some(started_at.to_rfc3339());
        runtime.refresh_started_instant = Some(std::time::Instant::now());
        runtime.last_error = None;
        runtime.refresh_finished_at = None;
        runtime.refresh_finished_instant = None;
        runtime.next_refresh_instant = None;
        runtime.refresh_duration_ms = None;
        runtime.total_candidates = 0;
        // A carried-over ping keeps its count: the refresh gathers the
        // shortfall on top instead of dropping to zero mid-cycle.
        runtime.tested_candidates = carry.as_ref().map_or(0, |carry| carry.tested);
        runtime.reachable_candidates = carry.as_ref().map_or(0, |carry| carry.reachable);
        runtime.fetch_errors.clear();
        runtime.live_logs.clear();
        if config.return_configs_asap && carry.is_none() {
            runtime.ranked.clear();
        }
        push_live_log(&mut runtime, refresh_started_log);
    }

    let fetch_started = std::time::Instant::now();
    info!("subscription load started");
    let cache_only = config.use_cache_only;
    let mut fetched = if cache_only {
        let db = database.clone();
        let top_n = config.top_n;
        let ranked_from_db = tokio::task::spawn_blocking(move || db.load_ranked_configs(top_n))
            .await
            .map_err(|e| anyhow!("{e}"))?
            .unwrap_or_default();

        if ranked_from_db.is_empty() {
            FetchOutcome {
                candidates: Vec::new(),
                errors: vec!["no previously-probed configs in database".to_string()],
                failures: Vec::new(),
                successes: Vec::new(),
            }
        } else {
            let _ = progress_tx.send(ProgressEvent::LiveLog(format!(
                "Loaded {} previously-probed configs from database",
                ranked_from_db.len()
            )));
            FetchOutcome {
                candidates: ranked_from_db
                    .into_iter()
                    .map(|rc| Candidate {
                        id: rc.id,
                        dedup_key: rc.dedup_key,
                        source: rc.source,
                        priority: rc.priority,
                        protocol: rc.protocol,
                        name: rc.name,
                        endpoint: rc.endpoint,
                        uri: rc.uri,
                    })
                    .collect(),
                errors: Vec::new(),
                failures: Vec::new(),
                successes: config
                    .subscriptions
                    .iter()
                    .filter(|s| s.enabled)
                    .cloned()
                    .collect(),
            }
        }
    } else {
        load_candidates_with_cache(
            config,
            |bytes| {
                let state = state.clone();
                async move {
                    add_fetch_bytes(&state, bytes).await;
                }
            },
            Some(progress_tx.clone()),
        )
        .await?
    };
    let mut fresh_success_count = fetched.successes.len();
    let mut fetched_count = fetched.candidates.len();
    let mut seen_candidate_keys = fetched
        .candidates
        .iter()
        .map(|candidate| candidate.dedup_key.clone())
        .collect::<HashSet<_>>();
    let mut fetch_errors = fetched.errors.clone();
    info!(
        candidates = fetched_count,
        fetch_errors = fetch_errors.len(),
        duration_ms = fetch_started.elapsed().as_millis(),
        "subscription load finished"
    );
    if print_compact_progress {
        print_log(format!(
            "Subscription loading finished: {} configs, {} source errors in {}.",
            fetched_count,
            fetch_errors.len(),
            format_duration_short(fetch_started.elapsed().as_millis())
        ));
    }
    let load_finished_log = timestamped_log(format!(
        "Subscription loading finished: {} configs, {} source errors in {}",
        fetched_count,
        fetch_errors.len(),
        format_duration_short(fetch_started.elapsed().as_millis())
    ));
    {
        let mut runtime = state.write().await;
        runtime.total_candidates = fetched_count;
        runtime.fetch_errors.clone_from(&fetch_errors);
        push_live_log(&mut runtime, load_finished_log);
    }

    // The preempted ping verified these seconds ago: keep them (they count
    // toward top_n via the stop policy) and don't probe them again. They join
    // every probe input's seen-set so retry/cache paths can't reintroduce them.
    let carried_keys: HashSet<&str> = carry
        .iter()
        .flat_map(|carry| carry.working.iter().map(|item| item.dedup_key.as_str()))
        .collect();
    seen_candidate_keys.extend(carried_keys.iter().map(|key| (*key).to_string()));
    fetched
        .candidates
        .retain(|candidate| !carried_keys.contains(candidate.dedup_key.as_str()));

    // Remember every sighting (insert-only) before probing consumes the
    // list: untested leftovers stay available for ping backfill.
    cache_fetched_candidates(&database, &fetched.candidates).await;

    let probe_started = std::time::Instant::now();
    let mut ranked = probe_refresh_candidates(
        std::mem::take(&mut fetched.candidates),
        config,
        &previous_top_n,
        &state,
        &progress_tx,
        print_compact_progress,
        "Probe",
        carry.as_ref(),
        None,
    )
    .await;

    if !cache_only && !fetched.failures.is_empty() {
        if let Some(proxy_uri) =
            subscription_retry_proxy_uri(config, &ranked, &previous_before_refresh)
        {
            let retry_started = std::time::Instant::now();
            match retry_failed_sources_with_proxy(
                config,
                &fetched.failures,
                &proxy_uri,
                |bytes| {
                    let state = state.clone();
                    async move {
                        add_fetch_bytes(&state, bytes).await;
                    }
                },
                Some(progress_tx.clone()),
            )
            .await
            {
                Ok(mut retry) => {
                    let retry_before_dedup = retry.candidates.len();
                    retry.candidates.retain(|candidate| {
                        seen_candidate_keys.insert(candidate.dedup_key.clone())
                    });
                    let retry_count = retry.candidates.len();
                    fetched_count = fetched_count.saturating_add(retry_count);
                    fresh_success_count = fresh_success_count.saturating_add(retry.successes.len());
                    fetch_errors = merged_fetch_errors(&fetched.failures, Some(&retry));
                    info!(
                        parsed = retry_before_dedup,
                        unique = retry_count,
                        remaining_fetch_errors = fetch_errors.len(),
                        duration_ms = retry_started.elapsed().as_millis(),
                        "proxied subscription retry finished"
                    );
                    if print_compact_progress {
                        print_log(format!(
                            "Subscription retry finished: {} new configs, {} source errors in {}.",
                            retry_count,
                            fetch_errors.len(),
                            format_duration_short(retry_started.elapsed().as_millis())
                        ));
                    }
                    let retry_finished_log = timestamped_log(format!(
                        "Subscription retry finished: {} new configs from {} entries; {} source errors remain",
                        retry_count,
                        retry_before_dedup,
                        fetch_errors.len()
                    ));
                    {
                        let mut runtime = state.write().await;
                        runtime.total_candidates = fetched_count;
                        runtime.fetch_errors.clone_from(&fetch_errors);
                        push_live_log(&mut runtime, retry_finished_log);
                    }
                    if retry_count > 0 {
                        cache_fetched_candidates(&database, &retry.candidates).await;
                        let mut retry_ranked = probe_refresh_candidates(
                            std::mem::take(&mut retry.candidates),
                            config,
                            &previous_top_n,
                            &state,
                            &progress_tx,
                            print_compact_progress,
                            "Retry probe",
                            carry.as_ref(),
                            None,
                        )
                        .await;
                        ranked.append(&mut retry_ranked);
                    }
                }
                Err(err) => {
                    let error = err.to_string();
                    warn!(error = %error, "proxied subscription retry failed");
                    push_tui_progress(
                        &state,
                        ProgressEvent::LiveLog(format!(
                            "Subscription retry through first working config failed: {error}"
                        )),
                        &HashSet::new(),
                        true,
                    )
                    .await;
                }
            }
        } else {
            push_tui_progress(
                &state,
                ProgressEvent::LiveLog(
                    "Subscription retry skipped: no emergency or active working config is available"
                        .to_string(),
                ),
                &HashSet::new(),
                true,
            )
            .await;
        }
    }
    if !cache_only && fresh_success_count == 0 {
        let cache_started = std::time::Instant::now();
        let db = database.clone();
        let top_n = config.top_n;
        match tokio::task::spawn_blocking(move || db.load_ranked_configs(top_n))
            .await
            .map_err(|e| anyhow!("{e}"))
        {
            Ok(Ok(db_configs)) if !db_configs.is_empty() => {
                let candidates: Vec<Candidate> = db_configs
                    .into_iter()
                    .filter(|rc| seen_candidate_keys.insert(rc.dedup_key.clone()))
                    .map(|rc| Candidate {
                        id: rc.id,
                        dedup_key: rc.dedup_key,
                        source: rc.source,
                        priority: rc.priority,
                        protocol: rc.protocol,
                        name: rc.name,
                        endpoint: rc.endpoint,
                        uri: rc.uri,
                    })
                    .collect();
                let cache_count = candidates.len();
                fetched_count = fetched_count.saturating_add(cache_count);
                info!(
                    unique = cache_count,
                    duration_ms = cache_started.elapsed().as_millis(),
                    "database fallback finished"
                );
                if print_compact_progress {
                    print_log(format!(
                        "Database fallback finished: {} configs in {}.",
                        cache_count,
                        format_duration_short(cache_started.elapsed().as_millis())
                    ));
                }
                let cache_finished_log = timestamped_log(format!(
                    "Database fallback finished: {cache_count} configs from previously-probed entries"
                ));
                {
                    let mut runtime = state.write().await;
                    runtime.total_candidates = fetched_count;
                    runtime.fetch_errors.clone_from(&fetch_errors);
                    push_live_log(&mut runtime, cache_finished_log);
                }
                if cache_count > 0 {
                    // Already database rows: nothing new to remember.
                    let mut cached_ranked = probe_refresh_candidates(
                        candidates,
                        config,
                        &previous_top_n,
                        &state,
                        &progress_tx,
                        print_compact_progress,
                        "Cache probe",
                        carry.as_ref(),
                        None,
                    )
                    .await;
                    ranked.append(&mut cached_ranked);
                }
            }
            Ok(Ok(_) | Err(_)) | Err(_) => {
                // Database fallback returned no configs or failed
            }
        }
    }
    drop(progress_tx);
    let _ = progress_task.await;
    info!(
        ranked = ranked.len(),
        reachable = ranked.iter().filter(|item| item.reachable).count(),
        duration_ms = probe_started.elapsed().as_millis(),
        "probe finished"
    );
    if fetched_count == 0 && ranked.is_empty() && !fetch_errors.is_empty() {
        return Err(anyhow!(
            "no usable configs were loaded; first error: {}",
            fetch_errors[0]
        ));
    }
    let speedtest_bytes = ranked
        .iter()
        .filter_map(|item| item.download_bytes)
        .map(|value| value as u64)
        .sum::<u64>();
    let finished_at = Utc::now();

    let progress_state = state.read().await.clone();
    let mut stable_working_counts = previous_before_refresh.stable_working_counts.clone();
    deduplicate_ranked_configs(&mut ranked);
    let fresh_tested = ranked.len();
    if let Some(carry) = &carry {
        // Fresh results win: carried configs were verified by the preempted
        // ping, new ones just now. No overlap is expected (carried keys were
        // filtered from every probe input); the dedupe is defensive.
        let fresh_keys: HashSet<&str> = ranked.iter().map(|item| item.dedup_key.as_str()).collect();
        let carried: Vec<RankedConfig> = carry
            .working
            .iter()
            .filter(|item| !fresh_keys.contains(item.dedup_key.as_str()))
            .cloned()
            .collect();
        ranked.extend(carried);
        deduplicate_ranked_configs(&mut ranked);
    }
    apply_stability_ranking(
        &mut ranked,
        &mut stable_working_counts,
        &previous_top_n,
        config.prioritize_stability,
    );

    // Stability fallback: a run that verifies nothing working must not
    // publish an empty working set over a live one (or wipe stability
    // memory) — keep serving the previous working configs instead.
    let fell_back = keep_previous_working_set(
        config.prioritize_stability,
        &ranked,
        &previous_before_refresh.ranked,
    );
    let fallback_working = previous_before_refresh
        .ranked
        .iter()
        .filter(|item| item.reachable)
        .count();
    if fell_back {
        warn!(
            previous_working = fallback_working,
            "refresh verified no working configs; keeping previous working set"
        );
        ranked.clone_from(&previous_before_refresh.ranked);
        stable_working_counts.clone_from(&previous_before_refresh.stable_working_counts);
    } else {
        persist_ranked_configs(
            &database,
            &ranked,
            config.top_n,
            config.clean_offlines_after_days,
            true,
        )
        .await?;
    }
    // Use the accumulated reachable count from probing — do NOT recalculate
    // from the final ranked list, as deduplication may reduce the count and
    // cause the "Working" display to drop after refresh finishes.
    let fetch_bytes = state.read().await.fetch_bytes;
    let refresh_fetch_bytes = fetch_bytes.saturating_sub(previous_before_refresh.fetch_bytes);
    let speedtest_bytes = progress_state
        .speedtest_bytes
        .saturating_add(speedtest_bytes);
    let mut runtime = RuntimeState {
        last_refresh: Some(started_at.to_rfc3339()),
        last_error: None,
        logs: progress_state.logs,
        live_logs: progress_state.live_logs,
        refresh_started_at: Some(started_at.to_rfc3339()),
        refresh_finished_at: Some(finished_at.to_rfc3339()),
        refresh_started_instant: Some(started_instant),
        refresh_finished_instant: Some(std::time::Instant::now()),
        next_refresh_instant: None,
        refresh_duration_ms: Some(started_instant.elapsed().as_millis()),
        refreshing: false,
        pinging: progress_state.pinging,
        next_ping_instant: progress_state.next_ping_instant,
        last_ping_instant: progress_state.last_ping_instant,
        total_candidates: fetched_count,
        tested_candidates: carry
            .as_ref()
            .map_or(0, |carry| carry.tested)
            .saturating_add(fresh_tested),
        reachable_candidates: if fell_back {
            fallback_working
        } else {
            progress_state.reachable_candidates
        },
        fetch_bytes,
        speedtest_bytes,
        fetch_errors,
        ranked,
        stable_working_counts,
        proxy_active_config: progress_state.proxy_active_config.clone(),
        proxy_active_uri: progress_state.proxy_active_uri.clone(),
        proxy_running: progress_state.proxy_running,
        proxy_port: progress_state.proxy_port,
        proxy_discoverable: progress_state.proxy_discoverable,
    };

    let failed_count = runtime
        .tested_candidates
        .saturating_sub(runtime.reachable_candidates);
    let summary = format!(
        "{} {} → {} ({}) · {} fetched, {} failed, {} working ({} used)",
        cycle_actor(manual),
        started_at.with_timezone(&Local).format("%H:%M:%S"),
        finished_at.with_timezone(&Local).format("%H:%M:%S"),
        format_duration_short(runtime.refresh_duration_ms.unwrap_or_default()),
        runtime.total_candidates,
        failed_count,
        runtime.reachable_candidates,
        format_bytes(refresh_fetch_bytes),
    );
    if fell_back {
        push_live_log(
            &mut runtime,
            timestamped_log(format!(
                "Refresh verified no working configs; kept {fallback_working} previous working configs"
            )),
        );
    }
    push_runtime_log(&mut runtime, summary);

    if print_terminal_summary {
        print_summary(&runtime, config.top_n);
    }
    *state.write().await = runtime;
    info!(
        duration_ms = started_instant.elapsed().as_millis(),
        "refresh finished"
    );
    Ok(())
}

fn probe_stop_policy(
    config: &AppConfig,
    previous_top_n: &HashSet<String>,
    candidates: &[crate::model::Candidate],
    carry: Option<&PingCarry>,
) -> ProbeStopPolicy {
    let current_keys = candidates
        .iter()
        .map(|candidate| candidate.dedup_key.as_str())
        .collect::<HashSet<_>>();

    let previous_working_keys: HashSet<String> = previous_top_n
        .iter()
        .filter(|key| current_keys.contains(key.as_str()))
        .cloned()
        .collect();
    // Carried-over ping results count toward the target (and the stability
    // quorum) so the interrupting refresh only gathers the shortfall.
    let (prefound_working, prefound_previous_working) = carry.map_or((0, 0), |carry| {
        (
            carry.reachable,
            carry
                .working
                .iter()
                .filter(|item| previous_working_keys.contains(&item.dedup_key))
                .count(),
        )
    });

    ProbeStopPolicy {
        scan_all_configs: config.scan_all_configs,
        top_n: config.top_n,
        prioritize_stability: config.prioritize_stability,
        return_configs_asap: config.return_configs_asap,
        previous_working_keys,
        prefound_working,
        prefound_previous_working,
    }
}

fn deduplicate_ranked_configs(ranked: &mut Vec<RankedConfig>) {
    let mut seen_keys = HashSet::new();
    ranked.retain(|item| seen_keys.insert(item.dedup_key.clone()));
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }
}

fn apply_stability_ranking(
    ranked: &mut [RankedConfig],
    stable_working_counts: &mut HashMap<String, u32>,
    _previous_top_n: &HashSet<String>,
    prioritize_stability: bool,
) {
    let ranked_keys: HashSet<String> = ranked.iter().map(|item| item.dedup_key.clone()).collect();
    stable_working_counts.retain(|key, _| ranked_keys.contains(key));

    for item in ranked.iter_mut() {
        if item.reachable {
            let count = stable_working_counts
                .entry(item.dedup_key.clone())
                .or_default();
            *count = count.saturating_add(1);
            item.stability_count = *count;
        } else {
            item.stability_count = stable_working_counts
                .get(&item.dedup_key)
                .copied()
                .unwrap_or(0);
        }
    }

    if prioritize_stability {
        ranked.sort_by(compare_stability_ranked);
        for (index, item) in ranked.iter_mut().enumerate() {
            item.rank = index + 1;
        }
    }
}

fn compare_stability_ranked(left: &RankedConfig, right: &RankedConfig) -> Ordering {
    right
        .reachable
        .cmp(&left.reachable)
        .then_with(|| right.stability_count.cmp(&left.stability_count))
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
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| left.protocol.cmp(&right.protocol))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.uri.cmp(&right.uri))
}

/// Stability fallback predicate: with `prioritize_stability`, a run that
/// verifies nothing working must keep serving the previous working set
/// instead of publishing an empty list (and wiping stability memory).
fn keep_previous_working_set(
    prioritize_stability: bool,
    new_ranked: &[RankedConfig],
    previous_ranked: &[RankedConfig],
) -> bool {
    prioritize_stability
        && !new_ranked.iter().any(|item| item.reachable)
        && previous_ranked.iter().any(|item| item.reachable)
}

/// Cap for one ping backfill round: bounds the extra probing when a ping
/// verifies fewer than `top_n` working configs.
const PING_BACKFILL_MAX_CANDIDATES: usize = 256;

fn candidate_from_ranked(item: &RankedConfig) -> Candidate {
    Candidate {
        id: item.id.clone(),
        dedup_key: item.dedup_key.clone(),
        source: item.source.clone(),
        priority: item.priority,
        protocol: item.protocol.clone(),
        name: item.name.clone(),
        endpoint: item.endpoint.clone(),
        uri: item.uri.clone(),
    }
}

/// Probe previously-seen database configs the current ping did not test,
/// merging fresh results into `ranked`. Returns how many backfill
/// candidates were tested (0 when the pool is empty or unreadable — the
/// ping still publishes its own results then).
///
/// Probe bytes never touch Sub Usage, like the main ping probe.
#[allow(clippy::too_many_arguments)]
async fn probe_ping_backfill(
    config: &AppConfig,
    database: &Arc<Database>,
    state: &Arc<RwLock<RuntimeState>>,
    previous_top_n: &HashSet<String>,
    tested_keys: &HashSet<String>,
    ranked: &mut Vec<RankedConfig>,
    print_compact_progress: bool,
    ping_cancel: &Arc<AtomicBool>,
) -> usize {
    let db = database.clone();
    let tested = tested_keys.clone();
    let extras = tokio::task::spawn_blocking(move || {
        db.load_backfill_candidates(&tested, PING_BACKFILL_MAX_CANDIDATES)
    })
    .await;
    let extras = match extras {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            warn!(error = %error, "ping backfill skipped: database unreadable");
            return 0;
        }
        Err(error) => {
            warn!(error = %error, "ping backfill skipped: database task failed");
            return 0;
        }
    };
    if extras.is_empty() {
        return 0;
    }
    let extra_candidates: Vec<Candidate> = extras.iter().map(candidate_from_ranked).collect();
    let backfill_count = extra_candidates.len();
    let (backfill_tx, backfill_task) = spawn_tui_progress_forwarder(
        state.clone(),
        previous_top_n.clone(),
        print_compact_progress,
        false,
    );
    let mut backfill_ranked = probe_refresh_candidates(
        extra_candidates,
        config,
        previous_top_n,
        state,
        &backfill_tx,
        print_compact_progress,
        "Ping backfill",
        None,
        Some(ping_cancel.clone()),
    )
    .await;
    drop(backfill_tx);
    let _ = backfill_task.await;
    ranked.append(&mut backfill_ranked);
    deduplicate_ranked_configs(ranked);
    backfill_count
}

/// Cache freshly fetched configs in the database (insert-only, never
/// touching known rows) so later pings can backfill from the whole pool —
/// including configs no cycle has tested yet. Failures never fail the
/// refresh; the fetch already succeeded.
async fn cache_fetched_candidates(database: &Arc<Database>, candidates: &[Candidate]) {
    if candidates.is_empty() {
        return;
    }
    let db = database.clone();
    let sightings: Vec<Candidate> = candidates.to_vec();
    let result = tokio::task::spawn_blocking(move || db.insert_new_candidates(&sightings)).await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn!(error = %error, "fetch cache skipped: database write failed"),
        Err(error) => warn!(error = %error, "fetch cache skipped: database task failed"),
    }
}

/// Re-probe cached configs without re-fetching subscriptions.
///
/// Runs on the independent `ping_seconds` timer. Skips silently when there
/// is nothing cached yet, or when a fetch cycle holds the cycle lock.
/// Counters mirror a refresh (reset, then accumulate) so the top bar always
/// describes the last cycle; fetch errors and totals from the last fetch
/// are preserved. Recent Logs keeps just the compact final result.
///
/// With stability on, previously served working configs that still verify
/// keep their seats and the shortfall to `top_n` fills from newly verified
/// ones; whatever verifies is published, and only a zero-verified run keeps
/// the previous set. When the cache alone verifies fewer than `top_n`, the
/// ping additionally probes previously-seen database configs (never
/// re-fetching) to try to refill the shortfall.
///
/// Returns whether a refresh preempted this ping (`true`): the probe stopped
/// early and the caller must skip the proxy update — the refresh publishes
/// right after. Preempted partials that verify less than served also keep
/// the previous set instead of wiping it.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn ping_once(
    config: &AppConfig,
    database: Arc<Database>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    cycle: Arc<tokio::sync::Mutex<()>>,
    print_compact_progress: bool,
    ping_cancel: Arc<AtomicBool>,
    manual: bool,
) -> Result<bool> {
    let actor = cycle_actor(manual);
    let Ok(_cycle_guard) = cycle.try_lock() else {
        debug!("ping skipped: fetch cycle is running");
        if manual {
            // The loop already checked `cycle_busy`, but a refresh can win
            // the lock in between: say so instead of going silent after the
            // TUI announced the manual ping.
            push_tui_progress(
                &state,
                ProgressEvent::LiveLog(timestamped_log(
                    "Manual ping skipped: a cycle is already running".to_string(),
                )),
                &HashSet::new(),
                true,
            )
            .await;
        }
        return Ok(false);
    };
    // No reset of `ping_cancel` here: a refresh only sets it while `pinging`
    // is observable (after this point), and always consumes it via take-over
    // in the same `refresh_once` — so a set flag always means "stop now".

    let cached: Vec<Candidate> = state
        .read()
        .await
        .ranked
        .iter()
        .map(candidate_from_ranked)
        .collect();
    if cached.is_empty() {
        debug!("ping skipped: no cached configs yet");
        return Ok(false);
    }
    let cached_count = cached.len();
    let tested_keys: HashSet<String> = cached.iter().map(|item| item.dedup_key.clone()).collect();

    // Baseline for every keep/top-up decision below: ASAP progress events
    // rewrite `state.ranked` live mid-probe (truncated to top_n), so the
    // post-probe state can no longer tell us what was served before this
    // ping started. The cycle guard above makes this snapshot stable.
    let previous_before_ping = state.read().await.clone();
    let started_instant = std::time::Instant::now();
    let previous_top_n = if config.prioritize_stability {
        let db = database.clone();
        tokio::task::spawn_blocking(move || db.load_stable_top_keys())
            .await
            .map_err(|e| anyhow!("{e}"))?
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    let (progress_tx, progress_task) = spawn_tui_progress_forwarder(
        state.clone(),
        previous_top_n.clone(),
        print_compact_progress,
        // A ping re-tests cached configs without fetching: probe byte
        // counts must not leak into Sub Usage (see `push_tui_progress`).
        false,
    );
    *runtime_config.write().await = RuntimeConfig::from(config);
    {
        let mut runtime = state.write().await;
        runtime.pinging = true;
        runtime.last_ping_instant = Some(started_instant);
        runtime.tested_candidates = 0;
        runtime.reachable_candidates = 0;
        // `total_candidates` (Fetched) belongs to the last fetch: a ping
        // re-tests the cached configs, so only Failed/Working move.
        // (The "Ping started" line stays live-only; Recent Logs keeps just
        // the compact final result, like fetch summaries.)
    }

    let mut ranked = probe_refresh_candidates(
        cached,
        config,
        &previous_top_n,
        &state,
        &progress_tx,
        print_compact_progress,
        "Ping",
        None,
        Some(ping_cancel.clone()),
    )
    .await;
    drop(progress_tx);
    let _ = progress_task.await;

    let mut progress_state = state.read().await.clone();
    // Database backfill: the cache alone may verify fewer than `top_n`
    // (configs die between cycles), so dip into previously-seen database
    // configs — never re-fetching — to try to refill the shortfall.
    // Skipped on preemption (the refresh owns the results then) and when
    // `top_n` is already met or unlimited.
    let mut backfill_tested = 0_usize;
    if !ping_cancel.load(AtomicOrdering::SeqCst)
        && config.top_n > 0
        && ranked.iter().filter(|item| item.reachable).count() < config.top_n
    {
        backfill_tested = probe_ping_backfill(
            config,
            &database,
            &state,
            &previous_top_n,
            &tested_keys,
            &mut ranked,
            print_compact_progress,
            &ping_cancel,
        )
        .await;
        progress_state = state.read().await.clone();
    }

    if ping_cancel.load(AtomicOrdering::SeqCst) {
        // Preempted by a refresh (no re-ranking, no persist — the refresh
        // owns all of that now). But partials skew toward fast failures, so
        // publishing them as-is could wipe a live set with zero verified:
        // when they verify fewer than served before, keep the previous set
        // instead. The refresh still carries the partial counts separately.
        deduplicate_ranked_configs(&mut ranked);
        let partial_working = ranked.iter().filter(|item| item.reachable).count();
        let previous_served = previous_before_ping
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .count();
        let keep_served = partial_working < previous_served && previous_served > 0;
        let tested = progress_state.tested_candidates;
        let working = progress_state.reachable_candidates;
        {
            let mut runtime = state.write().await;
            if keep_served {
                warn!(
                    previous_served,
                    partial_working,
                    "preempted ping verified fewer working configs; keeping previous working set"
                );
                runtime.ranked.clone_from(&previous_before_ping.ranked);
                runtime
                    .stable_working_counts
                    .clone_from(&previous_before_ping.stable_working_counts);
                // Counters describe the kept set, not the aborted run: the
                // top bar must show the still-served working configs instead
                // of freezing the partial zeros. The refresh seeds its stop
                // policy from these consistent values via the carry.
                runtime.tested_candidates = previous_before_ping.tested_candidates;
                runtime.reachable_candidates = previous_served;
            } else {
                runtime.ranked = ranked;
                // Counters stay partial (honest progress, and the refresh seeds
                // its stop policy from them — no ghost working). The ranked list
                // above is what continuity needs.
                runtime.tested_candidates = tested;
                runtime.reachable_candidates = working;
            }
            runtime.pinging = false;
            runtime.last_error = None;
        }
        let summary = if keep_served {
            format!(
                "{actor} Ping preempted by refresh: kept previous {previous_served} working configs (partials verified {partial_working} of {tested} tested in {})",
                format_duration_short(started_instant.elapsed().as_millis())
            )
        } else {
            format!(
                "{actor} Ping preempted by refresh: kept {partial_working} working of {tested} tested configs in {}",
                format_duration_short(started_instant.elapsed().as_millis())
            )
        };
        info!(summary = %summary, "ping preempted");
        if print_compact_progress {
            print_log(&summary);
        }
        push_tui_progress(
            &state,
            ProgressEvent::LiveLog(summary.clone()),
            &HashSet::new(),
            true,
        )
        .await;
        // A preemption is a final result for this ping: it belongs in Recent
        // Logs like the finished summary, so a "preempted but no refresh in
        // sight" report can be checked against the refresh line that must
        // follow it within the same minute.
        {
            let mut runtime = state.write().await;
            push_runtime_log(&mut runtime, summary);
        }
        return Ok(true);
    }
    // Baseline is the pre-probe snapshot: the live state above may already
    // carry ASAP truncations from this very probe.
    let mut stable_working_counts = previous_before_ping.stable_working_counts.clone();
    deduplicate_ranked_configs(&mut ranked);
    if !config.prioritize_stability {
        // The stability branch below does its own counting while rebuilding
        // order (previous seats, then latency); without it, count only.
        apply_stability_ranking(
            &mut ranked,
            &mut stable_working_counts,
            &previous_top_n,
            config.prioritize_stability,
        );
    }
    // Same stability fallback as the fetch cycle: a ping that verifies
    // nothing working must not blank a live working set (or wipe memory).
    // Stability top-up: previously served working configs that still verify
    // keep their seats in previous order (even when slower than newcomers);
    // the shortfall to top_n fills from newly verified working configs by
    // latency. Whatever verifies gets published — fresh truth always wins
    // over a stale list; only a zero-verified run keeps the previous set.
    let previous_working = previous_before_ping
        .ranked
        .iter()
        .filter(|item| item.reachable)
        .count();
    let fell_back = keep_previous_working_set(
        config.prioritize_stability,
        &ranked,
        &previous_before_ping.ranked,
    );
    // (still_count, fill_count) when the top-up below publishes a mix.
    let mut topped_up: Option<(usize, usize)> = None;
    if fell_back {
        warn!(
            previous_working,
            "ping verified no working configs; keeping previous working set"
        );
        ranked.clone_from(&previous_before_ping.ranked);
        stable_working_counts.clone_from(&previous_before_ping.stable_working_counts);
    } else if config.prioritize_stability {
        let mut fresh_working: HashMap<String, RankedConfig> = ranked
            .iter()
            .filter(|item| item.reachable)
            .map(|item| (item.dedup_key.clone(), item.clone()))
            .collect();
        let mut published: Vec<RankedConfig> = previous_before_ping
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .filter_map(|item| fresh_working.remove(&item.dedup_key))
            .collect();
        let still_count = published.len();
        let fill_target = if config.top_n == 0 {
            usize::MAX
        } else {
            config.top_n
        };
        let mut fill: Vec<RankedConfig> = fresh_working.into_values().collect();
        fill.sort_by(|left, right| {
            left.latency_ms
                .unwrap_or(u128::MAX)
                .cmp(&right.latency_ms.unwrap_or(u128::MAX))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.uri.cmp(&right.uri))
        });
        fill.truncate(fill_target.saturating_sub(still_count));
        let fill_count = fill.len();
        published.extend(fill);
        // Unverified entries trail for record-keeping (DB offline memory,
        // Failed counters); the served head is still-working then new.
        published.extend(ranked.iter().filter(|item| !item.reachable).cloned());
        let published_keys: HashSet<String> = published
            .iter()
            .map(|item| item.dedup_key.clone())
            .collect();
        stable_working_counts.retain(|key, _| published_keys.contains(key));
        for item in published.iter_mut() {
            if item.reachable {
                let count = stable_working_counts
                    .entry(item.dedup_key.clone())
                    .or_default();
                *count = count.saturating_add(1);
                item.stability_count = *count;
            } else {
                item.stability_count = stable_working_counts
                    .get(&item.dedup_key)
                    .copied()
                    .unwrap_or(0);
            }
        }
        for (index, item) in published.iter_mut().enumerate() {
            item.rank = index + 1;
        }
        ranked = published;
        persist_ranked_configs(&database, &ranked, config.top_n, 0, false).await?;
        if fill_count > 0 && still_count > 0 {
            topped_up = Some((still_count, fill_count));
        }
    } else {
        persist_ranked_configs(&database, &ranked, config.top_n, 0, false).await?;
    }
    let working = if fell_back {
        previous_working
    } else {
        progress_state.reachable_candidates
    };
    {
        let mut runtime = state.write().await;
        runtime.ranked = ranked;
        runtime.stable_working_counts = stable_working_counts;
        runtime.tested_candidates = progress_state.tested_candidates;
        runtime.reachable_candidates = working;
        runtime.pinging = false;
        runtime.last_error = None;
    }

    let scope = if backfill_tested > 0 {
        format!("{cached_count} cached + {backfill_tested} DB backfill")
    } else {
        format!("{} cached", progress_state.tested_candidates)
    };
    let summary = if let Some((still_count, fill_count)) = topped_up {
        format!(
            "{actor} Ping finished: kept {still_count} previous + {fill_count} new working of {scope} configs in {}",
            format_duration_short(started_instant.elapsed().as_millis())
        )
    } else {
        format!(
            "{actor} Ping finished: {working} working of {scope} configs in {}",
            format_duration_short(started_instant.elapsed().as_millis())
        )
    };
    info!(summary = %summary, "ping finished");
    if print_compact_progress {
        print_log(&summary);
    }
    push_tui_progress(
        &state,
        ProgressEvent::LiveLog(summary.clone()),
        &HashSet::new(),
        true,
    )
    .await;
    {
        let mut runtime = state.write().await;
        push_runtime_log(&mut runtime, summary);
    }
    Ok(false)
}

async fn set_next_ping_deadline(state: &Arc<RwLock<RuntimeState>>, ping_seconds: u64) {
    let mut state = state.write().await;
    if ping_seconds == 0 || state.pinging {
        state.next_ping_instant = None;
    } else {
        state.next_ping_instant =
            Some(std::time::Instant::now() + Duration::from_secs(ping_seconds));
    }
}
/// Run one ping cycle and publish its results — unless a refresh preempted
/// it, in which case the refresh publishes right after and the proxy update
/// is skipped so the proxy never flaps through a partial list.
#[allow(clippy::too_many_arguments)]
async fn run_ping_cycle(
    config: &AppConfig,
    database: &Arc<Database>,
    state: &Arc<RwLock<RuntimeState>>,
    runtime_config: &Arc<RwLock<RuntimeConfig>>,
    proxy: &proxy::SharedProxy,
    shared_ranked: &Arc<RwLock<Vec<RankedConfig>>>,
    cycle: &Arc<tokio::sync::Mutex<()>>,
    ping_cancel: &Arc<AtomicBool>,
    print_compact_progress: bool,
    label: &str,
) {
    let outcome = ping_once(
        config,
        database.clone(),
        state.clone(),
        runtime_config.clone(),
        cycle.clone(),
        print_compact_progress,
        ping_cancel.clone(),
        label == "manual",
    )
    .await;
    // `pinging` must never stick: `ping_once` resets it on its terminal
    // paths, but an error return (e.g. a failed persist) would otherwise
    // leave it set forever — and every later refresh would then take the
    // preemption path on ghost data.
    state.write().await.pinging = false;
    match outcome {
        Ok(true) => {}
        Ok(false) => update_proxy_and_ranked(proxy, shared_ranked, state, config, false).await,
        Err(err) => {
            error!(error = %err, "{label} ping failed");
            update_proxy_and_ranked(proxy, shared_ranked, state, config, false).await;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn spawn_ping_loop(
    mut config_rx: watch::Receiver<AppConfig>,
    database: Arc<Database>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    proxy: proxy::SharedProxy,
    shared_ranked: Arc<RwLock<Vec<RankedConfig>>>,
    cycle: Arc<tokio::sync::Mutex<()>>,
    mut trigger_rx: mpsc::UnboundedReceiver<()>,
    mut restart_rx: mpsc::UnboundedReceiver<()>,
    ping_cancel: Arc<AtomicBool>,
    print_compact_progress: bool,
) {
    tokio::spawn(async move {
        loop {
            let config_snapshot = config_rx.borrow().clone();
            let ping_seconds = config_snapshot.ping_seconds;
            // A ping on the refresh cadence would only re-verify what the
            // fetch just revalidated (both loops fire together and race the
            // cycle lock every round, and their countdowns drift a second
            // apart): park the automatic timer and let refreshes carry the
            // cadence. Manual pings still run on demand.
            let auto_ping = ping_seconds != 0
                && (config_snapshot.refresh_seconds == 0
                    || ping_seconds != config_snapshot.refresh_seconds);

            if !auto_ping {
                set_next_ping_deadline(&state, 0).await;
                tokio::select! {
                    trigger = trigger_rx.recv() => {
                        if trigger.is_none() {
                            return;
                        }
                        drain_triggers(&mut trigger_rx);
                        if cycle_busy(&state).await {
                            continue;
                        }
                        let config = config_rx.borrow().clone();
                        *runtime_config.write().await = RuntimeConfig::from(&config);
                        run_ping_cycle(
                            &config,
                            &database,
                            &state,
                            &runtime_config,
                            &proxy,
                            &shared_ranked,
                            &cycle,
                            &ping_cancel,
                            print_compact_progress,
                            "manual",
                        )
                        .await;
                        drain_triggers(&mut trigger_rx);
                    }
                    changed = config_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        let config = config_rx.borrow().clone();
                        *runtime_config.write().await = RuntimeConfig::from(&config);
                    }
                    restart = restart_rx.recv() => {
                        // A fetch just revalidated everything: drop the stale
                        // sleep and restart the countdown full from here.
                        if restart.is_none() {
                            return;
                        }
                        drain_triggers(&mut restart_rx);
                    }
                }
                continue;
            }

            set_next_ping_deadline(&state, ping_seconds).await;
            let sleep = time::sleep(Duration::from_secs(ping_seconds));
            tokio::pin!(sleep);

            tokio::select! {
                () = &mut sleep => {
                    drain_triggers(&mut trigger_rx);
                    let config = config_rx.borrow().clone();
                    run_ping_cycle(
                        &config,
                        &database,
                        &state,
                        &runtime_config,
                        &proxy,
                        &shared_ranked,
                        &cycle,
                        &ping_cancel,
                        print_compact_progress,
                        "automatic",
                    )
                    .await;
                }
                trigger = trigger_rx.recv() => {
                    if trigger.is_none() {
                        return;
                    }
                    drain_triggers(&mut trigger_rx);
                    if cycle_busy(&state).await {
                        // The TUI already said "started": correct the record
                        // so a lost start-race doesn't look like a no-op.
                        push_tui_progress(
                            &state,
                            ProgressEvent::LiveLog(timestamped_log(
                                "Manual ping skipped: a cycle is already running".to_string(),
                            )),
                            &HashSet::new(),
                            true,
                        )
                        .await;
                        continue;
                    }
                    let config = config_rx.borrow().clone();
                    run_ping_cycle(
                        &config,
                        &database,
                        &state,
                        &runtime_config,
                        &proxy,
                        &shared_ranked,
                        &cycle,
                        &ping_cancel,
                        print_compact_progress,
                        "manual",
                    )
                    .await;
                    drain_triggers(&mut trigger_rx);
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let config = config_rx.borrow().clone();
                    *runtime_config.write().await = RuntimeConfig::from(&config);
                }
                restart = restart_rx.recv() => {
                    // A fetch just revalidated everything: drop the stale
                    // sleep and restart the countdown full from here.
                    if restart.is_none() {
                        return;
                    }
                    drain_triggers(&mut restart_rx);
                }
            }
        }
    });
}

/// True while a fetch or ping cycle is running.
async fn cycle_busy(state: &Arc<RwLock<RuntimeState>>) -> bool {
    let runtime = state.read().await;
    runtime.refreshing || runtime.pinging
}

/// Drop queued manual triggers; the TUI refuses new ones while busy, so any
/// leftovers are stale duplicates (double-press) or races with the timer
/// that the just-finished cycle already satisfied. One chord, one cycle.
fn drain_triggers(rx: &mut mpsc::UnboundedReceiver<()>) {
    while rx.try_recv().is_ok() {}
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn spawn_refresh_loop(
    mut config_rx: watch::Receiver<AppConfig>,
    database: Arc<Database>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    proxy: proxy::SharedProxy,
    shared_ranked: Arc<RwLock<Vec<RankedConfig>>>,
    cycle: Arc<tokio::sync::Mutex<()>>,
    mut trigger_rx: mpsc::UnboundedReceiver<()>,
    ping_restart_tx: mpsc::UnboundedSender<()>,
    ping_cancel: Arc<AtomicBool>,
    print_terminal_summary: bool,
    print_compact_progress: bool,
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
                    database.clone(),
                    state.clone(),
                    runtime_config.clone(),
                    cycle.clone(),
                    print_terminal_summary,
                    print_compact_progress,
                    Some(ping_cancel.clone()),
                    false,
                )
                .await
                {
                    error!(error = %err, "initial refresh failed");
                    record_refresh_error(&state, err.to_string()).await;
                } else {
                    // The fetch just revalidated everything: restart the ping
                    // countdown full instead of resuming a stale partial one.
                    let _ = ping_restart_tx.send(());
                }

                update_proxy_and_ranked(&proxy, &shared_ranked, &state, &config, true).await;

                continue;
            }

            if refresh_seconds == 0 {
                set_next_refresh_deadline(&state, 0).await;
                warn!("automatic refresh is disabled because refresh_seconds is 0");
                tokio::select! {
                    trigger = trigger_rx.recv() => {
                        if trigger.is_none() {
                            return;
                        }
                        drain_triggers(&mut trigger_rx);
                        // A running ping doesn't block a manual refresh: the
                        // refresh preempts it inside `refresh_once` and
                        // carries its partial results over.
                        if state.read().await.refreshing {
                            continue;
                        }
                        let config = config_rx.borrow().clone();
                        *runtime_config.write().await = RuntimeConfig::from(&config);
                        last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                        if let Err(err) = refresh_once(&config, database.clone(), state.clone(), runtime_config.clone(), cycle.clone(), print_terminal_summary, print_compact_progress, Some(ping_cancel.clone()), true).await {
                            error!(error = %err, "manual refresh failed");
                            record_refresh_error(&state, err.to_string()).await;
                        } else {
                            // The fetch just revalidated everything: restart the ping
                            // countdown full instead of resuming a stale partial one.
                            let _ = ping_restart_tx.send(());
                        }

                        update_proxy_and_ranked(&proxy, &shared_ranked, &state, &config, true).await;
                        drain_triggers(&mut trigger_rx);
                        continue;
                    }
                    changed = config_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
                let config = config_rx.borrow().clone();
                *runtime_config.write().await = RuntimeConfig::from(&config);
                let fingerprint = RefreshFingerprint::from(&config);
                if last_refresh_fingerprint.as_ref() != Some(&fingerprint) {
                    last_refresh_fingerprint = Some(fingerprint);
                    if let Err(err) = refresh_once(
                        &config,
                        database.clone(),
                        state.clone(),
                        runtime_config.clone(),
                        cycle.clone(),
                        print_terminal_summary,
                        print_compact_progress,
                        Some(ping_cancel.clone()),
                        false,
                    )
                    .await
                    {
                        error!(error = %err, "refresh after config reload failed");
                        record_refresh_error(&state, err.to_string()).await;
                    } else {
                        // The fetch just revalidated everything: restart the ping
                        // countdown full instead of resuming a stale partial one.
                        let _ = ping_restart_tx.send(());
                    }
                }

                // Always update proxy on config change — even if the refresh
                // fingerprint hasn't changed (e.g. proxy enable/disable).
                update_proxy_and_ranked(&proxy, &shared_ranked, &state, &config, true).await;

                continue;
            }

            set_next_refresh_deadline(&state, refresh_seconds).await;
            let sleep = time::sleep(Duration::from_secs(refresh_seconds));
            tokio::pin!(sleep);

            tokio::select! {
                () = &mut sleep => {
                    drain_triggers(&mut trigger_rx);
                    let config = config_rx.borrow().clone();
                    last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                    if let Err(err) = refresh_once(&config, database.clone(), state.clone(), runtime_config.clone(), cycle.clone(), print_terminal_summary, print_compact_progress, Some(ping_cancel.clone()), false).await {
                        error!(error = %err, "refresh failed");
                        record_refresh_error(&state, err.to_string()).await;
                    } else {
                        // The fetch just revalidated everything: restart the ping
                        // countdown full instead of resuming a stale partial one.
                        let _ = ping_restart_tx.send(());
                    }

                    update_proxy_and_ranked(&proxy, &shared_ranked, &state, &config, true).await;
                }
                trigger = trigger_rx.recv() => {
                    if trigger.is_none() {
                        return;
                    }
                    drain_triggers(&mut trigger_rx);
                    // A running ping doesn't block a manual refresh: the
                    // refresh preempts it inside `refresh_once` and carries
                    // its partial results over.
                    if state.read().await.refreshing {
                        // The TUI already said "started": correct the record
                        // so a lost start-race doesn't look like a no-op.
                        push_tui_progress(
                            &state,
                            ProgressEvent::LiveLog(timestamped_log(
                                "Manual refresh skipped: a refresh is already running".to_string(),
                            )),
                            &HashSet::new(),
                            true,
                        )
                        .await;
                        continue;
                    }
                    let config = config_rx.borrow().clone();
                    last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                    if let Err(err) = refresh_once(&config, database.clone(), state.clone(), runtime_config.clone(), cycle.clone(), print_terminal_summary, print_compact_progress, Some(ping_cancel.clone()), true).await {
                        error!(error = %err, "manual refresh failed");
                        record_refresh_error(&state, err.to_string()).await;
                    } else {
                        // A manual fetch revalidates everything like a timer
                        // one: restart the ping countdown full as well.
                        let _ = ping_restart_tx.send(());
                    }

                    update_proxy_and_ranked(&proxy, &shared_ranked, &state, &config, true).await;
                    drain_triggers(&mut trigger_rx);
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }

                    let config = config_rx.borrow().clone();
                    *runtime_config.write().await = RuntimeConfig::from(&config);
                    let fingerprint = RefreshFingerprint::from(&config);
                    if last_refresh_fingerprint.as_ref() != Some(&fingerprint) {
                        last_refresh_fingerprint = Some(fingerprint);
                        if let Err(err) = refresh_once(&config, database.clone(), state.clone(), runtime_config.clone(), cycle.clone(), print_terminal_summary, print_compact_progress, Some(ping_cancel.clone()), false).await {
                            error!(error = %err, "refresh after config reload failed");
                            record_refresh_error(&state, err.to_string()).await;
                        } else {
                            // The fetch just revalidated everything: restart the ping
                            // countdown full instead of resuming a stale partial one.
                            let _ = ping_restart_tx.send(());
                        }
                    }

                    // Always update proxy on config change — even if the refresh
                    // fingerprint hasn't changed (e.g. proxy enable/disable).
                    update_proxy_and_ranked(&proxy, &shared_ranked, &state, &config, true).await;
                }
            }
        }
    });
}

/// Update the proxy with fresh ranked configs, sync the shared ranked list for
/// the health-check failover loop, and reflect proxy state in `RuntimeState`.
/// Push the latest ranked list to the proxy (and the failover loop's copy).
///
/// `clear_blacklist` is true after refreshes (new data: failed configs get a
/// fresh chance) and false after pings (same set re-verified: clearing would
/// re-arm configs failover just left and flap back onto them).
async fn update_proxy_and_ranked(
    proxy: &proxy::SharedProxy,
    shared_ranked: &Arc<RwLock<Vec<RankedConfig>>>,
    state: &Arc<RwLock<RuntimeState>>,
    config: &AppConfig,
    clear_blacklist: bool,
) {
    let ranked = state.read().await.ranked.clone();

    // Sync ranked list for the health-check failover loop
    (*shared_ranked.write().await).clone_from(&ranked);

    proxy
        .lock()
        .await
        .update(&config.proxy, &ranked, clear_blacklist)
        .await;

    // Reflect proxy state in RuntimeState so the TUI shows live info
    let snapshot = proxy.lock().await.snapshot().await;
    let mut runtime = state.write().await;
    runtime.proxy_running = snapshot.running;
    runtime
        .proxy_active_config
        .clone_from(&snapshot.active_config);
    runtime.proxy_active_uri = snapshot.active_uri;
    runtime.proxy_port = snapshot.port;
    runtime.proxy_discoverable = snapshot.discoverable;
}

/// Record the exact deadline the refresh loop is sleeping until, so the TUI
/// `Refresh` countdown matches the real timer instead of drifting by the
/// proxy-switch gap after each refresh. One write per cycle; zero hot-path cost.
async fn set_next_refresh_deadline(state: &Arc<RwLock<RuntimeState>>, refresh_seconds: u64) {
    let mut state = state.write().await;
    if refresh_seconds == 0 || state.refreshing {
        state.next_refresh_instant = None;
    } else {
        state.next_refresh_instant =
            Some(std::time::Instant::now() + Duration::from_secs(refresh_seconds));
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
struct RefreshFingerprint {
    top_n: usize,
    encoded_subscription: bool,
    prioritize_stability: bool,
    return_configs_asap: bool,
    scan_all_configs: bool,
    fetch_timeout_ms: u64,
    fetch_concurrency: usize,
    max_subscription_bytes: usize,
    use_cache_only: bool,
    probe: crate::config::ProbeConfig,
    subscriptions: Vec<crate::config::SubscriptionSource>,
}

impl From<&AppConfig> for RefreshFingerprint {
    fn from(config: &AppConfig) -> Self {
        Self {
            top_n: config.top_n,
            encoded_subscription: config.encoded_subscription,
            prioritize_stability: config.prioritize_stability,
            return_configs_asap: config.return_configs_asap,
            scan_all_configs: config.scan_all_configs,
            fetch_timeout_ms: config.fetch_timeout_ms,
            fetch_concurrency: config.fetch_concurrency,
            max_subscription_bytes: config.max_subscription_bytes,
            use_cache_only: config.use_cache_only,
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
            time::sleep(CONFIG_WATCH_INTERVAL).await;
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
            match load_config_and_persist_generated_token(&config_path) {
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
    state.refresh_finished_instant = Some(std::time::Instant::now());
    state.total_candidates = 0;
    state.tested_candidates = 0;
    state.reachable_candidates = 0;
    state.fetch_errors = vec![error.clone()];
    push_runtime_log(&mut state, format!("refresh error: {error}"));
    drop(state);
}

async fn add_fetch_bytes(state: &Arc<RwLock<RuntimeState>>, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let mut state = state.write().await;
    state.fetch_bytes = state.fetch_bytes.saturating_add(bytes);
}

/// Progress events from one probe run into live TUI state.
///
/// `account_bytes` attributes probe byte counts to Sub Usage: true for
/// fetch cycles (real downloads), false for ping cycles (re-testing cached
/// configs must not look like fetching).
#[allow(clippy::too_many_arguments)]
fn spawn_tui_progress_forwarder(
    state: Arc<RwLock<RuntimeState>>,
    previous_top_n: HashSet<String>,
    print_compact_progress: bool,
    account_bytes: bool,
) -> (
    mpsc::UnboundedSender<ProgressEvent>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let task = tokio::spawn(async move {
        let mut reporter = print_compact_progress.then(PlainProgressReporter::new);
        while let Some(event) = rx.recv().await {
            if let Some(reporter) = reporter.as_mut() {
                reporter.on_event(&event);
            }
            push_tui_progress(&state, event, &previous_top_n, account_bytes).await;
        }
    });
    (tx, task)
}

async fn push_tui_progress(
    state: &Arc<RwLock<RuntimeState>>,
    event: ProgressEvent,
    previous_top_n: &HashSet<String>,
    account_bytes: bool,
) {
    let mut state = state.write().await;
    match event {
        ProgressEvent::LiveLog(message) => push_live_log(&mut state, timestamped_log(message)),
        ProgressEvent::ProbeDelta {
            tested,
            working,
            bytes,
        } => {
            state.tested_candidates = state.tested_candidates.saturating_add(tested);
            state.reachable_candidates = state.reachable_candidates.saturating_add(working);
            if account_bytes {
                state.fetch_bytes = state.fetch_bytes.saturating_add(bytes);
            }
        }
        ProgressEvent::RankedSnapshot(mut ranked) => {
            apply_snapshot_stability_counts(
                &mut ranked,
                &state.stable_working_counts,
                previous_top_n,
            );
            apply_snapshot_ranks(&mut ranked);
            state.ranked = ranked;
        }
        ProgressEvent::WorkingConfigsFound { configs, top_n } => {
            append_asap_working_configs(&mut state, configs, top_n, previous_top_n);
        }
        ProgressEvent::FetchedDelta(count) => {
            state.total_candidates = count;
        }
    }
    drop(state);
}

fn append_asap_working_configs(
    state: &mut RuntimeState,
    configs: Vec<RankedConfig>,
    top_n: usize,
    previous_top_n: &HashSet<String>,
) {
    if top_n == 0 {
        return;
    }

    let mut seen_keys = HashSet::new();
    state
        .ranked
        .retain(|item| item.reachable && seen_keys.insert(item.dedup_key.clone()));

    for item in configs.into_iter().filter(|item| item.reachable) {
        if state.ranked.len() >= top_n {
            break;
        }
        if seen_keys.insert(item.dedup_key.clone()) {
            state.ranked.push(item);
        }
    }

    state.ranked.truncate(top_n);
    apply_snapshot_stability_counts(
        &mut state.ranked,
        &state.stable_working_counts,
        previous_top_n,
    );
    apply_found_order_ranks(&mut state.ranked);
}

fn apply_found_order_ranks(ranked: &mut [RankedConfig]) {
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }
}

fn apply_snapshot_stability_counts(
    ranked: &mut [RankedConfig],
    stable_working_counts: &HashMap<String, u32>,
    previous_top_n: &HashSet<String>,
) {
    for item in ranked {
        if item.reachable && previous_top_n.contains(&item.dedup_key) {
            item.stability_count = stable_working_counts
                .get(&item.dedup_key)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
        } else if item.reachable {
            item.stability_count = 1;
        } else {
            item.stability_count = stable_working_counts
                .get(&item.dedup_key)
                .copied()
                .unwrap_or(0);
        }
    }
}

fn apply_snapshot_ranks(ranked: &mut [RankedConfig]) {
    ranked.sort_by(compare_ranked_snapshot);
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }
}

fn compare_ranked_snapshot(left: &RankedConfig, right: &RankedConfig) -> Ordering {
    right
        .reachable
        .cmp(&left.reachable)
        .then_with(|| right.stability_count.cmp(&left.stability_count))
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
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| left.protocol.cmp(&right.protocol))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.uri.cmp(&right.uri))
}

fn timestamped_log(message: impl Into<String>) -> String {
    format!("{} {}", Local::now().format("%H:%M:%S%.3f"), message.into())
}

fn push_runtime_log(state: &mut RuntimeState, message: String) {
    state.logs.push(message);
    if state.logs.len() > MAX_TUI_LOGS {
        let extra = state.logs.len() - MAX_TUI_LOGS;
        state.logs.drain(0..extra);
    }
}

fn push_live_log(state: &mut RuntimeState, message: String) {
    state.live_logs.push(message);
    if state.live_logs.len() > MAX_TUI_LOGS {
        let extra = state.live_logs.len() - MAX_TUI_LOGS;
        state.live_logs.drain(0..extra);
    }
}

fn format_duration_short(ms: u128) -> String {
    let seconds = millis_to_seconds(ms);
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes}m {seconds}s")
}

fn millis_to_seconds(ms: u128) -> u64 {
    u64::try_from(ms / 1000).unwrap_or(u64::MAX)
}

#[allow(clippy::cast_precision_loss)]
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

impl From<&AppConfig> for RuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            bind: config.bind,
            top_n: config.top_n,
            refresh_seconds: config.refresh_seconds,
            ping_seconds: config.ping_seconds,
            encoded_subscription: config.encoded_subscription,
            prioritize_stability: config.prioritize_stability,
            return_configs_asap: config.return_configs_asap,
            scan_all_configs: config.scan_all_configs,
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
            probe_batch_size: config.probe.batch_size,
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
            proxy_enabled: config.proxy.enabled,
            proxy_port: config.proxy.port,
            proxy_discoverable: config.proxy.discoverable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Endpoint;

    fn temp_uninstall_root(name: &str) -> PathBuf {
        let unique = format!(
            "v2raydar-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system time is after unix epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn test_paths(root_dir: PathBuf) -> AppPaths {
        AppPaths {
            config_path: root_dir.join(CONFIG_FILE_NAME),
            cache_dir: root_dir.join(CACHE_DIR_NAME),
            root_dir,
            portable: false,
            generated_config: true,
        }
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent directory can be created");
        }
        std::fs::write(path, content).expect("test file can be written");
    }

    fn create_cache(cache_dir: &Path) {
        std::fs::create_dir_all(cache_dir).expect("cache directory can be created");
        write_file(&cache_dir.join(DB_FILE_NAME), "");
    }

    fn remove_test_root(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }

    fn ranked(name: &str, uri: &str, reachable: bool, latency_ms: Option<u128>) -> RankedConfig {
        RankedConfig {
            rank: 0,
            stability_count: 0,
            id: uri.to_string(),
            dedup_key: uri.to_string(),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: name.to_string(),
            endpoint: Endpoint {
                host: "example.com".to_string(),
                port: 443,
            },
            uri: uri.to_string(),
            reachable,
            validation: "active_http".to_string(),
            latency_ms,
            http_status: Some(204),
            download_mbps: None,
            download_bytes: None,
            error: None,
            country_code: None,
        }
    }

    #[test]
    fn stable_ranking_promotes_repeat_working_configs_when_enabled() {
        let mut ranked = vec![
            ranked("fast-new", "vless://fast@example.com:443", true, Some(100)),
            ranked(
                "slow-stable",
                "vless://slow@example.com:443",
                true,
                Some(5_000),
            ),
        ];
        let mut counts = HashMap::from([("vless://slow@example.com:443".to_string(), 2)]);
        let previous_top_n = HashSet::from(["vless://slow@example.com:443".to_string()]);

        apply_stability_ranking(&mut ranked, &mut counts, &previous_top_n, true);

        assert_eq!(ranked[0].name, "slow-stable");
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[0].stability_count, 3);
        assert_eq!(ranked[1].name, "fast-new");
    }

    #[test]
    fn stable_ranking_sorts_first_seen_configs_by_latency() {
        let mut slow = ranked(
            "slow-first",
            "vless://slow@example.com:443",
            true,
            Some(5_000),
        );
        slow.priority = 1;
        let mut fast = ranked(
            "fast-first",
            "vless://fast@example.com:443",
            true,
            Some(100),
        );
        fast.priority = 99;
        let mut ranked = vec![slow, fast];
        let mut counts = HashMap::new();
        let previous_top_n = HashSet::new();

        apply_stability_ranking(&mut ranked, &mut counts, &previous_top_n, true);

        assert_eq!(ranked[0].name, "fast-first");
        assert_eq!(ranked[0].stability_count, 1);
        assert_eq!(ranked[1].name, "slow-first");
        assert_eq!(ranked[1].stability_count, 1);
    }

    #[test]
    fn stable_ranking_prefers_higher_seen_count_before_latency() {
        let mut slow_stable = ranked(
            "slow-stable",
            "vless://slow@example.com:443",
            true,
            Some(5_000),
        );
        slow_stable.stability_count = 2;
        let mut fast_first = ranked(
            "fast-first",
            "vless://fast@example.com:443",
            true,
            Some(100),
        );
        fast_first.stability_count = 1;
        let mut ranked = [fast_first, slow_stable];

        ranked.sort_by(compare_stability_ranked);

        assert_eq!(ranked[0].name, "slow-stable");
        assert_eq!(ranked[1].name, "fast-first");
    }

    #[test]
    fn stability_counts_do_not_reorder_when_disabled() {
        let mut ranked = vec![
            ranked("fast-new", "vless://fast@example.com:443", true, Some(100)),
            ranked(
                "slow-stable",
                "vless://slow@example.com:443",
                true,
                Some(5_000),
            ),
        ];
        let mut counts = HashMap::from([("vless://slow@example.com:443".to_string(), 2)]);
        let previous_top_n = HashSet::from(["vless://slow@example.com:443".to_string()]);

        apply_stability_ranking(&mut ranked, &mut counts, &previous_top_n, false);

        assert_eq!(ranked[0].name, "fast-new");
        assert_eq!(ranked[1].name, "slow-stable");
        assert_eq!(ranked[1].stability_count, 3);
    }

    #[test]
    fn stable_ranking_keeps_unreachable_configs_after_working_configs() {
        let mut ranked = vec![
            ranked(
                "failed-stable",
                "vless://failed@example.com:443",
                false,
                None,
            ),
            ranked(
                "working-new",
                "vless://working@example.com:443",
                true,
                Some(300),
            ),
        ];
        let mut counts = HashMap::from([("vless://failed@example.com:443".to_string(), 5)]);
        let previous_top_n = HashSet::from(["vless://failed@example.com:443".to_string()]);

        apply_stability_ranking(&mut ranked, &mut counts, &previous_top_n, true);

        assert_eq!(ranked[0].name, "working-new");
        assert!(ranked[0].reachable);
        assert_eq!(ranked[1].name, "failed-stable");
        assert!(!ranked[1].reachable);
    }

    #[test]
    fn final_deduplication_keeps_first_seen_config_for_same_key() {
        let mut first = ranked(
            "first",
            "vless://uuid@example.com:443#first",
            true,
            Some(200),
        );
        first.dedup_key = "vless|example.com|443|tcp|tls".to_string();
        let mut duplicate = ranked(
            "duplicate",
            "vless://uuid@example.com:443#duplicate",
            true,
            Some(10),
        );
        duplicate.dedup_key = first.dedup_key.clone();
        let mut different_transport = ranked(
            "different",
            "vless://uuid@example.com:443?type=ws#different",
            true,
            Some(5),
        );
        different_transport.dedup_key = "vless|example.com|443|ws|tls".to_string();
        let mut ranked = vec![first, duplicate, different_transport];

        deduplicate_ranked_configs(&mut ranked);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].name, "first");
        assert_eq!(ranked[1].name, "different");
    }

    #[test]
    fn stable_ranking_counts_same_key_after_remark_changes() {
        let key = "vless|example.com|443|tcp|tls".to_string();
        let mut stable = ranked(
            "renamed-stable",
            "vless://uuid@example.com:443#new-remark",
            true,
            Some(5_000),
        );
        stable.dedup_key = key.clone();
        let mut ranked = vec![
            ranked("fast-new", "vless://fast@example.com:443", true, Some(100)),
            stable,
        ];
        let mut counts = HashMap::from([(key.clone(), 2)]);
        let previous_top_n = HashSet::from([key]);

        apply_stability_ranking(&mut ranked, &mut counts, &previous_top_n, true);

        assert_eq!(ranked[0].name, "renamed-stable");
        assert_eq!(ranked[0].stability_count, 3);
    }

    #[tokio::test]
    async fn ranked_snapshot_does_not_overwrite_live_working_counter() {
        let state = Arc::new(RwLock::new(RuntimeState {
            reachable_candidates: 5,
            ..RuntimeState::default()
        }));

        push_tui_progress(
            &state,
            ProgressEvent::RankedSnapshot(vec![ranked(
                "early",
                "vless://early@example.com:443",
                true,
                Some(100),
            )]),
            &HashSet::new(),
            true,
        )
        .await;

        let state = state.read().await;
        assert_eq!(state.reachable_candidates, 5);
        assert_eq!(state.ranked.len(), 1);
        drop(state);
    }

    #[tokio::test]
    async fn ranked_snapshot_updates_seen_from_stable_key_counts() {
        let key = "vless|example.com|443|tcp|tls".to_string();
        let state = Arc::new(RwLock::new(RuntimeState {
            stable_working_counts: HashMap::from([(key.clone(), 2)]),
            ..RuntimeState::default()
        }));
        let mut item = ranked(
            "early",
            "vless://uuid@example.com:443#renamed",
            true,
            Some(100),
        );
        item.dedup_key = key.clone();

        push_tui_progress(
            &state,
            ProgressEvent::RankedSnapshot(vec![item]),
            &HashSet::from([key]),
            true,
        )
        .await;

        let state = state.read().await;
        assert_eq!(state.ranked[0].stability_count, 3);
        drop(state);
    }

    #[tokio::test]
    async fn asap_working_configs_accumulate_without_touching_recent_logs() {
        let state = Arc::new(RwLock::new(RuntimeState {
            logs: vec!["previous summary".to_string()],
            ..RuntimeState::default()
        }));

        push_tui_progress(
            &state,
            ProgressEvent::WorkingConfigsFound {
                configs: vec![ranked(
                    "first",
                    "vless://first@example.com:443",
                    true,
                    Some(50),
                )],
                top_n: 2,
            },
            &HashSet::new(),
            true,
        )
        .await;
        push_tui_progress(
            &state,
            ProgressEvent::WorkingConfigsFound {
                configs: vec![
                    ranked("second", "vless://second@example.com:443", true, Some(10)),
                    ranked("third", "vless://third@example.com:443", true, Some(5)),
                ],
                top_n: 2,
            },
            &HashSet::new(),
            true,
        )
        .await;

        let state = state.read().await;
        assert_eq!(state.logs, vec!["previous summary"]);
        assert_eq!(
            state
                .ranked
                .iter()
                .map(|item| (item.rank, item.name.as_str()))
                .collect::<Vec<_>>(),
            [(1, "first"), (2, "second")]
        );
        drop(state);
    }

    #[tokio::test]
    async fn asap_working_configs_ignore_duplicates_and_failed_items() {
        let first = ranked("first", "vless://first@example.com:443", true, Some(50));
        let mut duplicate = ranked("renamed", "vless://renamed@example.com:443", true, Some(5));
        duplicate.dedup_key = first.dedup_key.clone();
        let failed = ranked("failed", "vless://failed@example.com:443", false, None);
        let state = Arc::new(RwLock::new(RuntimeState {
            ranked: vec![first],
            ..RuntimeState::default()
        }));

        push_tui_progress(
            &state,
            ProgressEvent::WorkingConfigsFound {
                configs: vec![
                    duplicate,
                    failed,
                    ranked("second", "vless://second@example.com:443", true, Some(10)),
                ],
                top_n: 3,
            },
            &HashSet::new(),
            true,
        )
        .await;

        let state = state.read().await;
        assert_eq!(
            state
                .ranked
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        drop(state);
    }

    #[tokio::test]
    async fn uninstall_removes_entire_data_root_with_only_app_artifacts() {
        let root = temp_uninstall_root("owned-clean");
        let paths = test_paths(root.clone());
        write_file(&paths.config_path, "subscriptions: []\n");
        create_cache(&paths.cache_dir);

        let targets = uninstall_targets(&paths)
            .await
            .expect("clean app root is removable");

        assert_eq!(targets, vec![root.clone()]);
        remove_test_root(&root);
    }

    #[tokio::test]
    async fn uninstall_installed_layout_removes_app_dir_with_only_data_root() {
        let base = temp_uninstall_root("installed-clean");
        let app_dir = base.join(APP_NAME);
        let data_root = app_dir.join(APP_DATA_DIR_NAME);
        let paths = test_paths(data_root);
        write_file(&paths.config_path, "subscriptions: []\n");
        create_cache(&paths.cache_dir);

        let targets = uninstall_targets(&paths)
            .await
            .expect("installed app dir is removable");

        assert_eq!(targets, vec![app_dir]);
        remove_test_root(&base);
    }

    #[tokio::test]
    async fn uninstall_installed_layout_preserves_app_dir_with_unknown_files() {
        let base = temp_uninstall_root("installed-mixed");
        let app_dir = base.join(APP_NAME);
        let data_root = app_dir.join(APP_DATA_DIR_NAME);
        let paths = test_paths(data_root.clone());
        write_file(&paths.config_path, "subscriptions: []\n");
        write_file(&app_dir.join("notes.txt"), "user data");
        create_cache(&paths.cache_dir);

        let targets = uninstall_targets(&paths)
            .await
            .expect("installed mixed app dir targets data root only");

        assert_eq!(targets, vec![data_root]);
        remove_test_root(&base);
    }

    #[tokio::test]
    async fn uninstall_data_root_with_unknown_files_only_targets_known_artifacts() {
        let root = temp_uninstall_root("owned-unknown");
        let paths = test_paths(root.clone());
        write_file(&paths.config_path, "subscriptions: []\n");
        write_file(&root.join("notes.txt"), "user data");
        create_cache(&paths.cache_dir);

        let targets = uninstall_targets(&paths)
            .await
            .expect("mixed root falls back to known artifacts");

        assert_eq!(
            targets,
            vec![paths.config_path.clone(), paths.cache_dir.clone()]
        );
        remove_test_root(&root);
    }

    #[tokio::test]
    async fn uninstall_mixed_cache_dir_only_targets_known_cache_files() {
        let root = temp_uninstall_root("mixed-cache");
        let paths = test_paths(root.clone());
        write_file(&paths.config_path, "subscriptions: []\n");
        create_cache(&paths.cache_dir);
        write_file(&paths.cache_dir.join("notes.txt"), "user data");

        let targets = uninstall_targets(&paths)
            .await
            .expect("mixed cache falls back to known cache files");

        assert_eq!(
            targets,
            vec![
                paths.config_path.clone(),
                paths.cache_dir.join(DB_FILE_NAME),
            ]
        );
        remove_test_root(&root);
    }

    #[tokio::test]
    async fn custom_config_uninstall_removes_only_sibling_data_dir() {
        let root = temp_uninstall_root("custom-config");
        let config_path = root.join("custom.yaml");
        let paths = AppPaths::from_config_override(config_path.clone());
        write_file(&config_path, "subscriptions: []\n");
        write_file(&root.join("notes.txt"), "user data");
        create_cache(&paths.cache_dir);

        let targets = uninstall_targets(&paths)
            .await
            .expect("custom config data target selection succeeds");

        assert_eq!(targets, vec![paths.root_dir.clone()]);
        assert!(!targets.contains(&config_path));
        remove_test_root(&root);
    }

    #[tokio::test]
    async fn uninstall_mixed_root_targets_firewall_state_inside_data_dir() {
        let root = temp_uninstall_root("mixed-firewall");
        let paths = test_paths(root.clone());
        write_file(&paths.config_path, "subscriptions: []\n");
        write_file(&root.join(FIREWALL_STATE_FILE_NAME), "{}");
        write_file(&root.join("notes.txt"), "user data");

        let targets = uninstall_targets(&paths)
            .await
            .expect("mixed root targets known files");

        assert_eq!(
            targets,
            vec![
                paths.config_path.clone(),
                root.join(FIREWALL_STATE_FILE_NAME),
            ]
        );
        remove_test_root(&root);
    }

    #[test]
    fn stability_accumulates_for_all_reachable_configs_not_just_previous_top_n() {
        let mut ranked_a = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(500)),
            ranked("config-c", "vless://c@example.com:443", true, Some(100)),
        ];
        let mut counts = HashMap::new();
        let previous_top_n: HashSet<String> = HashSet::new();

        apply_stability_ranking(&mut ranked_a, &mut counts, &previous_top_n, true);

        assert_eq!(ranked_a[0].stability_count, 1);
        assert_eq!(ranked_a[1].stability_count, 1);
        assert_eq!(counts.len(), 2);
    }

    #[test]
    fn alternating_configs_both_accumulate_stability_across_refreshes() {
        let mut counts = HashMap::new();

        let mut ranked_run1 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(500)),
            ranked("config-c", "vless://c@example.com:443", true, Some(100)),
        ];
        let previous_top_n1: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run1, &mut counts, &previous_top_n1, true);

        assert_eq!(counts["vless://a@example.com:443"], 1);
        assert_eq!(counts["vless://c@example.com:443"], 1);

        let mut ranked_run2 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(500)),
            ranked("config-c", "vless://c@example.com:443", true, Some(100)),
        ];
        let previous_top_n2: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run2, &mut counts, &previous_top_n2, true);

        assert_eq!(counts["vless://a@example.com:443"], 2);
        assert_eq!(counts["vless://c@example.com:443"], 2);

        let mut ranked_run3 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(500)),
            ranked("config-c", "vless://c@example.com:443", true, Some(100)),
        ];
        let previous_top_n3: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run3, &mut counts, &previous_top_n3, true);

        assert_eq!(counts["vless://a@example.com:443"], 3);
        assert_eq!(counts["vless://c@example.com:443"], 3);

        assert_eq!(ranked_run3[0].stability_count, 3);
        assert_eq!(ranked_run3[1].stability_count, 3);
    }

    #[test]
    fn config_that_was_absent_then_reappears_starts_fresh() {
        let mut counts = HashMap::new();

        let mut ranked_run1 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(100)),
            ranked("config-b", "vless://b@example.com:443", true, Some(200)),
        ];
        let previous_top_n1: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run1, &mut counts, &previous_top_n1, true);
        assert_eq!(counts["vless://a@example.com:443"], 1);
        assert_eq!(counts["vless://b@example.com:443"], 1);

        let mut ranked_run2 = vec![ranked(
            "config-a",
            "vless://a@example.com:443",
            true,
            Some(100),
        )];
        let previous_top_n2: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run2, &mut counts, &previous_top_n2, true);
        assert_eq!(counts["vless://a@example.com:443"], 2);
        assert!(!counts.contains_key("vless://b@example.com:443"));

        let mut ranked_run3 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(100)),
            ranked("config-b", "vless://b@example.com:443", true, Some(200)),
        ];
        let previous_top_n3: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run3, &mut counts, &previous_top_n3, true);

        assert_eq!(counts["vless://a@example.com:443"], 3);
        assert_eq!(counts["vless://b@example.com:443"], 1);
    }

    #[test]
    fn config_becoming_unreachable_loses_count_retains_in_map_for_recovery() {
        let mut counts = HashMap::new();

        let mut ranked_run1 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(100)),
            ranked("config-b", "vless://b@example.com:443", true, Some(200)),
        ];
        let previous_top_n1: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run1, &mut counts, &previous_top_n1, true);

        let mut ranked_run2 = vec![
            ranked("config-a", "vless://a@example.com:443", true, Some(100)),
            ranked("config-b", "vless://b@example.com:443", false, None),
        ];
        let previous_top_n2: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut ranked_run2, &mut counts, &previous_top_n2, true);

        assert_eq!(counts["vless://a@example.com:443"], 2);
        assert_eq!(counts["vless://b@example.com:443"], 1);
        assert_eq!(ranked_run2[0].stability_count, 2);
        assert_eq!(ranked_run2[1].stability_count, 1);
    }

    #[test]
    fn slow_stable_config_beats_fast_new_config_in_ranking() {
        let mut counts = HashMap::new();

        for _ in 0..5 {
            let mut ranked = vec![
                ranked(
                    "slow-stable",
                    "vless://slow@example.com:443",
                    true,
                    Some(5_000),
                ),
                ranked("fast-new", "vless://fast@example.com:443", true, Some(50)),
            ];
            let previous_top_n: HashSet<String> = HashSet::new();
            apply_stability_ranking(&mut ranked, &mut counts, &previous_top_n, true);
        }

        assert_eq!(counts["vless://slow@example.com:443"], 5);
        assert_eq!(counts["vless://fast@example.com:443"], 5);

        let mut final_ranked = vec![
            ranked(
                "slow-stable",
                "vless://slow@example.com:443",
                true,
                Some(5_000),
            ),
            ranked("fast-new", "vless://fast@example.com:443", true, Some(50)),
        ];
        let previous_top_n: HashSet<String> = HashSet::new();
        apply_stability_ranking(&mut final_ranked, &mut counts, &previous_top_n, true);

        assert_eq!(counts["vless://slow@example.com:443"], 6);
        assert_eq!(counts["vless://fast@example.com:443"], 6);
        assert_eq!(final_ranked[0].stability_count, 6);
        assert_eq!(final_ranked[1].stability_count, 6);
    }

    fn previous_working_ranked(uri: &str, key: &str) -> crate::model::RankedConfig {
        crate::model::RankedConfig {
            rank: 1,
            stability_count: 7,
            id: key.to_string(),
            dedup_key: key.to_string(),
            source: "previous".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: "prev".to_string(),
            endpoint: crate::model::Endpoint {
                host: "127.0.0.1".to_string(),
                port: 9,
            },
            uri: uri.to_string(),
            reachable: true,
            validation: "tcp".to_string(),
            latency_ms: Some(50),
            http_status: None,
            download_mbps: None,
            download_bytes: None,
            error: None,
            country_code: None,
        }
    }

    #[test]
    fn stability_fallback_predicate() {
        let working = previous_working_ranked("vless://a@example.com:443", "a");
        let failed = crate::model::RankedConfig {
            reachable: false,
            ..working.clone()
        };
        let working_ref = std::slice::from_ref(&working);
        let failed_ref = std::slice::from_ref(&failed);
        // Total failure over a live set: keep.
        assert!(keep_previous_working_set(true, failed_ref, working_ref));
        assert!(keep_previous_working_set(true, &[], working_ref));
        // Fresh truth wins whenever anything verifies.
        assert!(!keep_previous_working_set(true, working_ref, working_ref));
        // Nothing to keep, or stability off: publish as-is.
        assert!(!keep_previous_working_set(true, failed_ref, &[]));
        assert!(!keep_previous_working_set(true, failed_ref, failed_ref));
        assert!(!keep_previous_working_set(false, failed_ref, working_ref));
    }

    #[tokio::test]
    async fn refresh_keeps_previous_working_set_when_new_run_finds_nothing() {
        // Fetch succeeds (one data: candidate on a closed port) but the probe
        // verifies nothing: with stability on, the previous working set must
        // stay published and stability memory must survive.
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        let database = manual_trigger_test_database();
        database
            .save_stable_top_keys(&["prev-key".to_string()])
            .expect("stable keys save");
        let previous = previous_working_ranked("vless://prev@example.com:443", "prev-key");
        let state = Arc::new(tokio::sync::RwLock::new(crate::model::RuntimeState {
            ranked: vec![previous],
            stable_working_counts: HashMap::from([("prev-key".to_string(), 7)]),
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        refresh_once(
            &config,
            database.clone(),
            state.clone(),
            runtime_config,
            cycle,
            false,
            false,
            None,
            false,
        )
        .await
        .expect("refresh succeeds");

        let runtime = state.read().await;
        assert_eq!(runtime.ranked.len(), 1);
        assert_eq!(runtime.ranked[0].uri, "vless://prev@example.com:443");
        assert!(runtime.ranked[0].reachable);
        assert_eq!(runtime.reachable_candidates, 1);
        assert_eq!(runtime.stable_working_counts.get("prev-key"), Some(&7));
        drop(runtime);
        assert!(
            database
                .load_stable_top_keys()
                .expect("stable keys load")
                .contains("prev-key")
        );
    }

    #[tokio::test]
    async fn refresh_publishes_workless_result_when_stability_off() {
        // Same total failure with stability off: fresh truth wins, even when
        // it serves nothing (documents the opt-out scope).
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = false;
        let database = manual_trigger_test_database();
        let previous = previous_working_ranked("vless://prev@example.com:443", "prev-key");
        let state = Arc::new(tokio::sync::RwLock::new(crate::model::RuntimeState {
            ranked: vec![previous],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        refresh_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            false,
            None,
            false,
        )
        .await
        .expect("refresh succeeds");

        let runtime = state.read().await;
        assert!(!runtime.ranked.iter().any(|item| item.reachable));
        drop(runtime);
    }

    #[tokio::test]
    async fn ping_keeps_previous_working_set_when_nothing_verifies() {
        // Ping re-probes a stale working entry on a closed port: with
        // stability on, the cached working set must survive the failed ping.
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        let database = manual_trigger_test_database();
        let previous = previous_working_ranked("vless://prev@example.com:443", "prev-key");
        let state = Arc::new(tokio::sync::RwLock::new(crate::model::RuntimeState {
            ranked: vec![previous],
            stable_working_counts: HashMap::from([("prev-key".to_string(), 7)]),
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.ranked.len(), 1);
        assert_eq!(runtime.ranked[0].uri, "vless://prev@example.com:443");
        assert!(runtime.ranked[0].reachable);
        drop(runtime);
    }

    fn manual_trigger_test_config() -> crate::config::AppConfig {
        let mut config = crate::config::AppConfig::default_for_first_run();
        config.refresh_seconds = 3600;
        config.ping_seconds = 3600;
        config.top_n = 2;
        config.probe.mode = crate::config::ProbeMode::Tcp;
        config.subscriptions = vec![crate::config::SubscriptionSource {
            name: "local".to_string(),
            url: "data:,vless://00000000-0000-0000-0000-000000000000@127.0.0.1:9%23e2e".to_string(),
            enabled: true,
            priority: 1,
        }];
        config
    }

    fn manual_trigger_test_database() -> Arc<crate::db::Database> {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-trigger-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        Arc::new(crate::db::Database::open(&dir.join("data.db")).expect("db opens"))
    }

    fn manual_trigger_test_proxy() -> proxy::SharedProxy {
        Arc::new(tokio::sync::Mutex::new(crate::proxy::PersistentProxy::new(
            crate::config::ProxyConfig {
                enabled: false,
                ..Default::default()
            },
            String::new(),
            None,
        )))
    }

    async fn wait_for_condition(message: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
        loop {
            if done() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting: {message}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    async fn live_log_count(state: &Arc<tokio::sync::RwLock<RuntimeState>>, needle: &str) -> usize {
        state
            .read()
            .await
            .live_logs
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }

    async fn summary_count(state: &Arc<tokio::sync::RwLock<RuntimeState>>) -> usize {
        state
            .read()
            .await
            .logs
            .iter()
            .filter(|line| line.contains("fetched,"))
            .count()
    }

    #[tokio::test]
    async fn manual_refresh_trigger_runs_exactly_one_extra_cycle() {
        let config = manual_trigger_test_config();
        let database = manual_trigger_test_database();
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState::default()));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let proxy = manual_trigger_test_proxy();
        let shared_ranked = Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let (config_tx, config_rx) = tokio::sync::watch::channel(config.clone());
        let _config_tx = config_tx;
        let (trigger_tx, trigger_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (restart_tx, _restart_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let ping_cancel = Arc::new(AtomicBool::new(false));

        spawn_refresh_loop(
            config_rx,
            database,
            state.clone(),
            runtime_config,
            proxy,
            shared_ranked,
            cycle,
            trigger_rx,
            restart_tx,
            ping_cancel,
            false,
            false,
        );

        wait_for_condition("initial refresh", || {
            state
                .try_read()
                .is_ok_and(|runtime| !runtime.refreshing && runtime.last_refresh.is_some())
        })
        .await;
        // Double chord press while idle must coalesce into a single cycle.
        // NOTE: counted via persistent `logs` summaries, not `live_logs` —
        // every refresh clears live logs at start, so live entries can't
        // accumulate across cycles.
        let baseline = summary_count(&state).await;
        assert!(baseline >= 1);
        trigger_tx.send(()).expect("trigger sends");
        trigger_tx.send(()).expect("trigger sends");
        wait_for_condition("manual refresh", || {
            state.try_read().is_ok_and(|runtime| {
                !runtime.refreshing
                    && runtime
                        .logs
                        .iter()
                        .filter(|line| line.contains("fetched,"))
                        .count()
                        > baseline
            })
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        assert_eq!(summary_count(&state).await, baseline + 1);
        // Manual trigger: the Recent Logs summary carries 👤.
        assert!(
            state
                .read()
                .await
                .logs
                .iter()
                .any(|line| line.contains("👤") && line.contains("fetched,"))
        );
    }

    #[tokio::test]
    async fn manual_ping_trigger_repings_cache_without_refetch() {
        let config = manual_trigger_test_config();
        let database = manual_trigger_test_database();
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState::default()));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let proxy = manual_trigger_test_proxy();
        let shared_ranked = Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let (config_tx, config_rx) = tokio::sync::watch::channel(config.clone());
        let _config_tx = config_tx;
        let (refresh_trigger_tx, refresh_trigger_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let _refresh_trigger_tx = refresh_trigger_tx;
        let (ping_trigger_tx, ping_trigger_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (restart_tx, restart_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let ping_cancel = Arc::new(AtomicBool::new(false));

        spawn_refresh_loop(
            config_rx.clone(),
            database.clone(),
            state.clone(),
            runtime_config.clone(),
            proxy.clone(),
            shared_ranked.clone(),
            cycle.clone(),
            refresh_trigger_rx,
            restart_tx,
            ping_cancel.clone(),
            false,
            false,
        );
        spawn_ping_loop(
            config_rx,
            database,
            state.clone(),
            runtime_config,
            proxy,
            shared_ranked,
            cycle,
            ping_trigger_rx,
            restart_rx,
            ping_cancel,
            false,
        );

        wait_for_condition("initial refresh populates cache", || {
            state.try_read().is_ok_and(|runtime| {
                !runtime.refreshing && runtime.last_refresh.is_some() && !runtime.ranked.is_empty()
            })
        })
        .await;
        let fetch_mark = live_log_count(&state, "Subscription loading finished").await;
        assert!(fetch_mark >= 1);

        ping_trigger_tx.send(()).expect("trigger sends");
        ping_trigger_tx.send(()).expect("trigger sends");
        wait_for_condition("manual ping", || {
            state.try_read().is_ok_and(|runtime| {
                !runtime.pinging
                    && runtime
                        .live_logs
                        .iter()
                        .any(|line| line.contains("Ping finished"))
            })
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        assert_eq!(live_log_count(&state, "Ping finished").await, 1);
        // No re-fetch happened for the ping cycle.
        assert_eq!(
            live_log_count(&state, "Subscription loading finished").await,
            fetch_mark
        );
        // Manual ping: Recent Logs gains just the compact 👤 summary.
        assert!(
            state
                .read()
                .await
                .logs
                .iter()
                .any(|line| line.contains("👤 Ping finished"))
        );
    }

    #[tokio::test]
    async fn ping_restart_resets_stale_countdown_without_cycling() {
        let mut config = manual_trigger_test_config();
        // Unequal cadences: equal ones park the automatic ping timer (the
        // fetch already covers that cadence), which would leave no deadline
        // for this test to observe.
        config.refresh_seconds = 1800;
        let database = manual_trigger_test_database();
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState::default()));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let proxy = manual_trigger_test_proxy();
        let shared_ranked = Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let (config_tx, config_rx) = tokio::sync::watch::channel(config.clone());
        let _config_tx = config_tx;
        let (trigger_tx, trigger_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let _trigger_tx = trigger_tx;
        let (restart_tx, restart_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let ping_cancel = Arc::new(AtomicBool::new(false));

        spawn_ping_loop(
            config_rx,
            database,
            state.clone(),
            runtime_config,
            proxy,
            shared_ranked,
            cycle,
            trigger_rx,
            restart_rx,
            ping_cancel,
            false,
        );

        // Wait until the loop scheduled its sleep, then fake the stale partial
        // countdown a long refresh leaves behind.
        wait_for_condition("ping sleep scheduled", || {
            state
                .try_read()
                .is_ok_and(|runtime| runtime.next_ping_instant.is_some())
        })
        .await;
        let old_deadline = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or_else(std::time::Instant::now);
        state.write().await.next_ping_instant = Some(old_deadline);

        restart_tx.send(()).expect("restart sends");
        wait_for_condition("countdown restarted full", || {
            state.try_read().is_ok_and(|runtime| {
                runtime.next_ping_instant.is_some_and(|deadline| {
                    deadline.saturating_duration_since(std::time::Instant::now())
                        > std::time::Duration::from_secs(3590)
                })
            })
        })
        .await;
        // A restart reschedules; it must not run a cycle.
        assert_eq!(live_log_count(&state, "Ping finished").await, 0);
        assert_eq!(live_log_count(&state, "Ping started").await, 0);
    }

    #[tokio::test]
    async fn refresh_carries_preempted_ping_working_set() {
        // Simulates a refresh coming due mid-ping: the preemption request is
        // set, the ping's partials are live in state. The refresh must keep
        // the working config (no reset, no re-probe loss) and only gather
        // the shortfall with its fresh probe.
        let config = manual_trigger_test_config();
        let database = manual_trigger_test_database();
        let carried = ranked("carried", "vless://carried@example.com:443", true, Some(50));
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![carried],
            tested_candidates: 5,
            reachable_candidates: 1,
            pinging: true,
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let ping_cancel = Arc::new(AtomicBool::new(true));

        refresh_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            false,
            Some(ping_cancel.clone()),
            false,
        )
        .await
        .expect("refresh succeeds");

        // The request is consumed by the take-over.
        assert!(!ping_cancel.load(AtomicOrdering::SeqCst));
        let runtime = state.read().await;
        // The fetched candidate (closed port, fails) joins the carried one.
        assert_eq!(runtime.ranked.len(), 2);
        assert!(
            runtime
                .ranked
                .iter()
                .any(|item| item.uri == "vless://carried@example.com:443" && item.reachable)
        );
        assert_eq!(runtime.total_candidates, 1);
        assert_eq!(runtime.reachable_candidates, 1);
        // Counters continue from the carried values plus the fresh probe.
        assert_eq!(runtime.tested_candidates, 6);
        assert!(!runtime.refreshing);
        // Automatic refresh: Recent Logs summary carries 🤖.
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("🤖") && line.contains("fetched,"))
        );
        drop(runtime);
    }

    #[tokio::test]
    async fn probe_deltas_attribute_bytes_only_for_fetch_cycles() {
        // Ping re-tests without fetching: its probe bytes must not leak
        // into Sub Usage, while counters still accumulate in both modes.
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState::default()));
        push_tui_progress(
            &state,
            ProgressEvent::ProbeDelta {
                tested: 2,
                working: 1,
                bytes: 100,
            },
            &HashSet::new(),
            false,
        )
        .await;
        let runtime = state.read().await;
        assert_eq!(runtime.tested_candidates, 2);
        assert_eq!(runtime.reachable_candidates, 1);
        assert_eq!(runtime.fetch_bytes, 0);
        drop(runtime);
        push_tui_progress(
            &state,
            ProgressEvent::ProbeDelta {
                tested: 2,
                working: 1,
                bytes: 100,
            },
            &HashSet::new(),
            true,
        )
        .await;
        let runtime = state.read().await;
        assert_eq!(runtime.tested_candidates, 4);
        assert_eq!(runtime.reachable_candidates, 2);
        assert_eq!(runtime.fetch_bytes, 100);
        drop(runtime);
    }

    #[tokio::test]
    async fn ping_leaves_fetched_total_and_sub_usage_alone() {
        // A ping re-tests cached configs without fetching: the Fetched total
        // and Sub Usage from the last fetch must survive, while Failed and
        // Working follow the ping results. Recent Logs gains just the
        // compact 👤 summary (started/preempted stay live-only).
        let config = manual_trigger_test_config();
        let database = manual_trigger_test_database();
        let mut cached = ranked(
            "cached",
            "vless://00000000-0000-0000-0000-000000000000@127.0.0.1:9#e2e",
            false,
            None,
        );
        cached.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: 9,
        };
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached],
            total_candidates: 8816,
            fetch_bytes: 5_505_024,
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.total_candidates, 8816);
        assert_eq!(runtime.fetch_bytes, 5_505_024);
        assert_eq!(runtime.tested_candidates, 1);
        assert_eq!(runtime.reachable_candidates, 0);
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("👤 Ping finished"))
        );
        assert!(
            !runtime
                .logs
                .iter()
                .any(|line| line.contains("Ping started"))
        );
        drop(runtime);
    }

    #[tokio::test]
    async fn ping_publishes_partial_results_when_verifying_fewer() {
        // Fresh truth wins over a stale list: two previously working
        // configs, one now failing with nothing new to fill the gap — the
        // ping publishes the one that still verifies instead of keeping
        // the dead one served.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let open_port = listener.local_addr().expect("listener addr").port();
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        let database = manual_trigger_test_database();
        let mut alive = ranked(
            "alive",
            "vless://00000000-0000-0000-0000-000000000001@127.0.0.1#alive",
            true,
            Some(5),
        );
        alive.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: open_port,
        };
        let mut dead = ranked(
            "dead",
            "vless://00000000-0000-0000-0000-000000000002@127.0.0.1#dead",
            true,
            Some(7),
        );
        dead.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: 9,
        };
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![alive, dead],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.reachable_candidates, 1);
        let head: Vec<&str> = runtime
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(head, vec!["alive"]);
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("1 working of 2 cached"))
        );
        drop(runtime);
        drop(listener);
    }

    #[tokio::test]
    async fn ping_backfills_shortfall_from_database() {
        // The cache alone verifies fewer than top_n: the ping must dip into
        // previously-seen database configs (never re-fetching) to refill.
        // One cached working + one DB working with top_n 2 must serve 2.
        let first = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let second = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port_of =
            |listener: &std::net::TcpListener| listener.local_addr().expect("listener addr").port();
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        config.top_n = 2;
        let database = manual_trigger_test_database();
        let mut cached_alive = ranked(
            "cached-alive",
            "vless://00000000-0000-0000-0000-000000000031@127.0.0.1#cached-alive",
            true,
            Some(5),
        );
        cached_alive.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: port_of(&first),
        };
        let mut db_alive = ranked(
            "db-alive",
            "vless://00000000-0000-0000-0000-000000000032@127.0.0.1#db-alive",
            true,
            Some(7),
        );
        db_alive.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: port_of(&second),
        };
        let mut db_dead = ranked(
            "db-dead",
            "vless://00000000-0000-0000-0000-000000000033@127.0.0.1#db-dead",
            false,
            None,
        );
        // Unroutable loopback: the widened pool probes it, but it must
        // fail fast and hermetically (no real network in tests).
        db_dead.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: 9,
        };
        persist_ranked_configs(&database, &[db_alive, db_dead], 2, 0, false)
            .await
            .expect("db seeds");
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached_alive],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.reachable_candidates, 2);
        let head: Vec<&str> = runtime
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(head, vec!["cached-alive", "db-alive"]);
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("1 cached + 2 DB backfill")),
            "backfill scope missing: {:?}",
            runtime.logs
        );
        drop(runtime);
        drop((first, second));
    }

    #[tokio::test]
    async fn ping_backfills_never_tested_database_rows() {
        // The exact fetch-cache flow: a sighting the database holds but no
        // cycle ever tested (reachable 0, no measurements) still refills a
        // shortfall when it verifies on probe.
        let first = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let second = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port_of =
            |listener: &std::net::TcpListener| listener.local_addr().expect("listener addr").port();
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        config.top_n = 2;
        let database = manual_trigger_test_database();
        let mut cached_alive = ranked(
            "cached-alive",
            "vless://00000000-0000-0000-0000-000000000061@127.0.0.1#cached-alive",
            true,
            Some(5),
        );
        cached_alive.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: port_of(&first),
        };
        let sighting = crate::model::Candidate {
            id: "sighting".to_string(),
            dedup_key: "vless://00000000-0000-0000-0000-000000000062@127.0.0.1#sighting"
                .to_string(),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: "sighting".to_string(),
            endpoint: crate::model::Endpoint {
                host: "127.0.0.1".to_string(),
                port: port_of(&second),
            },
            uri: "vless://00000000-0000-0000-0000-000000000062@127.0.0.1#sighting".to_string(),
        };
        database
            .insert_new_candidates(std::slice::from_ref(&sighting))
            .expect("sighting inserts");
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached_alive],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.reachable_candidates, 2);
        let head: Vec<&str> = runtime
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(head, vec!["cached-alive", "sighting"]);
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("1 cached + 1 DB backfill")),
            "backfill scope missing: {:?}",
            runtime.logs
        );
        drop(runtime);
        drop((first, second));
    }

    #[tokio::test]
    async fn insert_new_candidates_never_clobbers_known_rows() {
        // Re-fetching a known working config must not reset it to untested,
        // while genuinely new sightings become backfill-eligible pool rows.
        let database = manual_trigger_test_database();
        let mut known = ranked(
            "known",
            "vless://00000000-0000-0000-0000-000000000071@127.0.0.1#known",
            true,
            Some(100),
        );
        known.stability_count = 4;
        persist_ranked_configs(&database, std::slice::from_ref(&known), 1, 0, false)
            .await
            .expect("known persists");
        let repeat = crate::model::Candidate {
            id: "repeat".to_string(),
            dedup_key: known.dedup_key.clone(),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: "repeat".to_string(),
            endpoint: crate::model::Endpoint {
                host: "127.0.0.1".to_string(),
                port: 9,
            },
            uri: known.uri.clone(),
        };
        let fresh = crate::model::Candidate {
            id: "fresh".to_string(),
            dedup_key: "vless://00000000-0000-0000-0000-000000000072@127.0.0.1#fresh".to_string(),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: "fresh".to_string(),
            endpoint: crate::model::Endpoint {
                host: "127.0.0.1".to_string(),
                port: 9,
            },
            uri: "vless://00000000-0000-0000-0000-000000000072@127.0.0.1#fresh".to_string(),
        };
        database
            .insert_new_candidates(&[repeat, fresh])
            .expect("sightings insert");

        let loaded = database
            .load_backfill_candidates(&HashSet::new(), 10)
            .expect("loads");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "known");
        assert!(loaded[0].reachable);
        assert_eq!(loaded[0].stability_count, 4);
        assert_eq!(loaded[0].latency_ms, Some(100));
        assert_eq!(loaded[1].name, "fresh");
        assert!(!loaded[1].reachable);
    }

    #[tokio::test]
    async fn ping_skips_backfill_when_top_n_met() {
        // Cache already verifies top_n: the database must stay untouched.
        // A poison row (unparsable URI, would fail preparation if probed)
        // proves no extra probing happened via the tested count.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let open_port = listener.local_addr().expect("listener addr").port();
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        config.top_n = 1;
        let database = manual_trigger_test_database();
        let mut cached_alive = ranked(
            "cached-alive",
            "vless://00000000-0000-0000-0000-000000000041@127.0.0.1#cached-alive",
            true,
            Some(5),
        );
        cached_alive.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: open_port,
        };
        let poison = ranked("poison", "not-a-uri", true, None);
        persist_ranked_configs(&database, std::slice::from_ref(&poison), 1, 0, false)
            .await
            .expect("db seeds");
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached_alive],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.reachable_candidates, 1);
        assert_eq!(runtime.tested_candidates, 1);
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("1 working of 1 cached configs")),
            "unexpected scope: {:?}",
            runtime.logs
        );
        assert!(
            !runtime.logs.iter().any(|line| line.contains("DB backfill")),
            "backfill must not run: {:?}",
            runtime.logs
        );
        drop(runtime);
        drop(listener);
    }

    #[tokio::test]
    async fn backfill_loader_orders_excludes_and_caps() {
        // Veterans first (working, then stable, then fast), then failed and
        // never-tested rows; tested keys excluded, hard cap honored.
        let database = manual_trigger_test_database();
        let mut steady = ranked(
            "steady",
            "vless://00000000-0000-0000-0000-000000000051@127.0.0.1#steady",
            true,
            Some(900),
        );
        steady.stability_count = 5;
        let mut quick = ranked(
            "quick",
            "vless://00000000-0000-0000-0000-000000000052@127.0.0.1#quick",
            true,
            Some(50),
        );
        quick.stability_count = 3;
        let mut stale = ranked(
            "stale",
            "vless://00000000-0000-0000-0000-000000000053@127.0.0.1#stale",
            true,
            Some(10),
        );
        stale.stability_count = 1;
        let buried = ranked(
            "buried",
            "vless://00000000-0000-0000-0000-000000000054@127.0.0.1#buried",
            false,
            None,
        );
        persist_ranked_configs(
            &database,
            &[steady.clone(), quick.clone(), stale.clone(), buried],
            4,
            0,
            false,
        )
        .await
        .expect("db seeds");

        let exclude = HashSet::from([steady.dedup_key.clone()]);
        let loaded = database
            .load_backfill_candidates(&exclude, 2)
            .expect("loads");
        let names: Vec<&str> = loaded.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["quick", "stale"]);

        let capped = database
            .load_backfill_candidates(&HashSet::new(), 1)
            .expect("loads capped");
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].name, "steady");

        let all = database
            .load_backfill_candidates(&HashSet::new(), 10)
            .expect("loads all");
        let names: Vec<&str> = all.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["steady", "quick", "stale", "buried"]);
    }

    #[tokio::test]
    async fn ping_tops_up_previous_working_with_new_configs() {
        // Previously working configs that still verify keep their seats in
        // previous order (even if slower); the shortfall to top_n fills from
        // newly verified working configs by latency.
        let first = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let second = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let newcomer = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port_of =
            |listener: &std::net::TcpListener| listener.local_addr().expect("listener addr").port();
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        config.top_n = 3;
        let database = manual_trigger_test_database();
        let mut slow_old = ranked(
            "slow-old",
            "vless://00000000-0000-0000-0000-000000000011@127.0.0.1#slow-old",
            true,
            Some(9_000),
        );
        slow_old.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: port_of(&first),
        };
        let mut fast_old = ranked(
            "fast-old",
            "vless://00000000-0000-0000-0000-000000000012@127.0.0.1#fast-old",
            true,
            Some(100),
        );
        fast_old.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: port_of(&second),
        };
        let mut revived = ranked(
            "revived",
            "vless://00000000-0000-0000-0000-000000000013@127.0.0.1#revived",
            false,
            None,
        );
        revived.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: port_of(&newcomer),
        };
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![slow_old, fast_old, revived],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);

        let runtime = state.read().await;
        assert_eq!(runtime.reachable_candidates, 3);
        // Previous order kept (slow-old ahead of fast-old despite latency),
        // newcomer fills the shortfall last.
        let head: Vec<&str> = runtime
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(head, vec!["slow-old", "fast-old", "revived"]);
        assert!(
            runtime
                .logs
                .iter()
                .any(|line| line.contains("kept 2 previous + 1 new working"))
        );
        drop(runtime);
        drop((first, second, newcomer));
    }

    #[tokio::test]
    async fn preempted_ping_keeps_previous_working_set() {
        // A preempted ping whose partials verify nothing must not wipe the
        // live set: the previous working configs stay served with their
        // counters (the top bar must show the 2 still served, not the
        // partial zeros) — Debug-asserted here with a preset cancel flag
        // standing in for the interrupting refresh.
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        let database = manual_trigger_test_database();
        let mut first = ranked(
            "first",
            "vless://00000000-0000-0000-0000-000000000021@127.0.0.1#first",
            true,
            Some(11),
        );
        first.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: 9,
        };
        let mut second = ranked(
            "second",
            "vless://00000000-0000-0000-0000-000000000022@127.0.0.1#second",
            true,
            Some(13),
        );
        second.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: 9,
        };
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![first, second],
            tested_candidates: 7,
            reachable_candidates: 2,
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let ping_cancel = Arc::new(AtomicBool::new(true));

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle,
            false,
            ping_cancel,
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(preempted);

        let runtime = state.read().await;
        assert!(!runtime.pinging);
        assert_eq!(runtime.reachable_candidates, 2);
        assert_eq!(runtime.tested_candidates, 7);
        assert_eq!(runtime.ranked.len(), 2);
        assert!(runtime.ranked.iter().all(|item| item.reachable));
        assert!(
            runtime
                .live_logs
                .iter()
                .any(|line| line.contains("kept previous 2 working configs"))
        );
        drop(runtime);
    }

    #[tokio::test]
    async fn manual_ping_reports_skip_when_cycle_locked() {
        // The TUI announces the manual ping before the loop runs it; when a
        // refresh wins the lock in between, the ping must say so in Live
        // Logs instead of going silent after the announcement.
        let config = manual_trigger_test_config();
        let database = manual_trigger_test_database();
        let cached = ranked(
            "cached",
            "vless://00000000-0000-0000-0000-000000000000@127.0.0.1:9#e2e",
            false,
            None,
        );
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let _guard = cycle.lock().await;

        let preempted = ping_once(
            &config,
            database,
            state.clone(),
            runtime_config,
            cycle.clone(),
            false,
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .await
        .expect("ping succeeds");
        assert!(!preempted);
        assert!(live_log_count(&state, "Manual ping skipped").await >= 1);
        drop(_guard);
    }

    #[tokio::test]
    async fn ping_aborts_when_preempted_and_skips_publish() {
        // A set preemption flag (what a refresh raises mid-ping) stops the
        // probe at the first result: no stability publish, no persist, no
        // fallback swap — the interrupting refresh owns all of that.
        let mut config = manual_trigger_test_config();
        config.prioritize_stability = true;
        let database = manual_trigger_test_database();
        database
            .save_stable_top_keys(&["keep-me".to_string()])
            .expect("stable keys save");
        let mut cached = ranked(
            "cached",
            "vless://00000000-0000-0000-0000-000000000000@127.0.0.1:9#e2e",
            false,
            None,
        );
        cached.endpoint = crate::model::Endpoint {
            host: "127.0.0.1".to_string(),
            port: 9,
        };
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let ping_cancel = Arc::new(AtomicBool::new(true));

        let preempted = ping_once(
            &config,
            database.clone(),
            state.clone(),
            runtime_config,
            cycle,
            false,
            ping_cancel,
            false,
        )
        .await
        .expect("ping succeeds");
        assert!(preempted);

        let runtime = state.read().await;
        assert!(!runtime.pinging);
        // A completed ping would have wiped the stable keys (nothing
        // verified); the preempted one must leave persistence alone.
        assert!(
            database
                .load_stable_top_keys()
                .expect("stable keys load")
                .contains("keep-me")
        );
        assert!(
            runtime
                .live_logs
                .iter()
                .any(|line| line.contains("Ping preempted by refresh"))
        );
        drop(runtime);
    }

    #[tokio::test]
    async fn equal_intervals_park_automatic_ping_but_allow_manual() {
        // A ping on the refresh cadence only re-verifies what the fetch just
        // did: the automatic timer stays parked (no deadline, no cycles) on
        // a 2s cadence that would otherwise fire constantly, while a manual
        // ping still runs on demand without arming the timer.
        let mut config = manual_trigger_test_config();
        config.refresh_seconds = 2;
        config.ping_seconds = 2;
        let database = manual_trigger_test_database();
        let cached = ranked(
            "cached",
            "vless://00000000-0000-0000-0000-000000000000@127.0.0.1:9#e2e",
            false,
            None,
        );
        let state = Arc::new(tokio::sync::RwLock::new(RuntimeState {
            ranked: vec![cached],
            ..Default::default()
        }));
        let runtime_config = Arc::new(tokio::sync::RwLock::new(RuntimeConfig::from(&config)));
        let proxy = manual_trigger_test_proxy();
        let shared_ranked = Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let cycle = Arc::new(tokio::sync::Mutex::new(()));
        let (config_tx, config_rx) = tokio::sync::watch::channel(config.clone());
        let _config_tx = config_tx;
        let (ping_trigger_tx, ping_trigger_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (_restart_tx, restart_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let ping_cancel = Arc::new(AtomicBool::new(false));

        spawn_ping_loop(
            config_rx,
            database,
            state.clone(),
            runtime_config,
            proxy,
            shared_ranked,
            cycle,
            ping_trigger_rx,
            restart_rx,
            ping_cancel,
            false,
        );

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(state.read().await.next_ping_instant.is_none());
        assert_eq!(live_log_count(&state, "Ping finished").await, 0);

        ping_trigger_tx.send(()).expect("trigger sends");
        wait_for_condition("manual ping", || {
            state.try_read().is_ok_and(|runtime| {
                !runtime.pinging
                    && runtime
                        .live_logs
                        .iter()
                        .any(|line| line.contains("Ping finished"))
            })
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert_eq!(live_log_count(&state, "Ping finished").await, 1);
        assert!(state.read().await.next_ping_instant.is_none());
    }

    #[test]
    fn drain_triggers_drops_queued_duplicates() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        tx.send(()).expect("send works");
        tx.send(()).expect("send works");
        drain_triggers(&mut rx);
        assert!(rx.try_recv().is_err());
        tx.send(()).expect("channel still usable");
        assert!(rx.try_recv().is_ok());
    }
}
