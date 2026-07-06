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
    sync::Arc,
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
use tracing::{error, info, warn};

use crate::{
    config::{AppConfig, ProbeMode},
    constants::{
        APP_DATA_DIR_NAME, APP_NAME, CACHE_DIR_NAME, CONFIG_FILE_NAME, CONFIG_WATCH_INTERVAL,
        DB_FILE_NAME, DEFAULT_LOG_FILTER_PLAIN, DEFAULT_LOG_FILTER_TUI, DEFAULT_LOG_FILTER_VERBOSE,
        FIREWALL_STATE_FILE_NAME, LEGACY_APP_MARKER_FILE_NAME, LEGACY_CACHE_MARKER_FILE_NAME,
        LOCALHOST_IP, MAX_TUI_LOGS, sing_box_download_url,
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
#[command(name = "v2raydar")]
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
    // Pre-build a rustls config with webpki-roots to bypass the platform verifier
    // which requires JNI context unavailable in Termux. On non-Android builds
    // the OnceLock stays empty and reqwest uses its default TLS backend.
    let _ = FALLBACK_TLS.set(build_tls_config());

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

    // Initialize GeoIP database — embedded is primary, file is fallback
    let geoip_path = config
        .geoip_db_path
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let p = paths.root_dir.join("GeoLite2-Country.mmdb");
            if p.exists() { Some(p) } else { None }
        });
    crate::geoip::init(geoip_path.as_deref());

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
        refresh_once(
            &config,
            database.clone(),
            state.clone(),
            runtime_config.clone(),
            true,
            !cli.verbose,
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

    let (config_tx, config_rx) = watch::channel(config.clone());
    spawn_refresh_loop(
        config_rx,
        database.clone(),
        state.clone(),
        runtime_config.clone(),
        cli.no_tui,
        cli.no_tui && !cli.verbose,
    );
    spawn_config_watcher(paths.config_path.clone(), config.bind, config_tx);

    if cli.no_tui {
        serve(config.bind, state, runtime_config).await
    } else {
        tokio::select! {
            result = serve(config.bind, state.clone(), runtime_config.clone()) => result,
            result = tui::run(config, paths, state, runtime_config, database.clone()) => result,
        }
    }
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

async fn probe_refresh_candidates(
    candidates: Vec<Candidate>,
    config: &AppConfig,
    previous_top_n: &HashSet<String>,
    state: &Arc<RwLock<RuntimeState>>,
    progress_tx: &mpsc::UnboundedSender<ProgressEvent>,
    print_compact_progress: bool,
    label: &str,
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
    )
    .await;

    let stop_policy = probe_stop_policy(config, previous_top_n, &candidates);
    probe_candidates(
        candidates,
        &config.probe,
        Some(progress_tx.clone()),
        &stop_policy,
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

    if cli.portable {
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

#[allow(clippy::significant_drop_tightening, clippy::too_many_lines)]
async fn refresh_once(
    config: &AppConfig,
    database: Arc<Database>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
    print_terminal_summary: bool,
    print_compact_progress: bool,
) -> Result<()> {
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
        runtime.last_error = None;
        runtime.refresh_finished_at = None;
        runtime.refresh_duration_ms = None;
        runtime.total_candidates = 0;
        runtime.tested_candidates = 0;
        runtime.reachable_candidates = 0;
        runtime.fetch_errors.clear();
        runtime.live_logs.clear();
        if config.return_configs_asap {
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

    let probe_started = std::time::Instant::now();
    let mut ranked = probe_refresh_candidates(
        std::mem::take(&mut fetched.candidates),
        config,
        &previous_top_n,
        &state,
        &progress_tx,
        print_compact_progress,
        "Probe",
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
                        let mut retry_ranked = probe_refresh_candidates(
                            std::mem::take(&mut retry.candidates),
                            config,
                            &previous_top_n,
                            &state,
                            &progress_tx,
                            print_compact_progress,
                            "Retry probe",
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
                    let mut cached_ranked = probe_refresh_candidates(
                        candidates,
                        config,
                        &previous_top_n,
                        &state,
                        &progress_tx,
                        print_compact_progress,
                        "Cache probe",
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
    apply_stability_ranking(
        &mut ranked,
        &mut stable_working_counts,
        &previous_top_n,
        config.prioritize_stability,
    );

    // Persist configs to database
    {
        let db = database.clone();
        let configs = ranked.clone();
        let top_n = config.top_n;
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

    // Async cleanup of offline configs
    {
        let db = database.clone();
        let days = config.clean_offlines_after_days;
        tokio::task::spawn_blocking(move || db.clean_offline_configs(days))
            .await
            .map_err(|e| anyhow!("{e}"))?
            .map(|deleted| {
                if deleted > 0 {
                    info!(deleted, "cleaned offline configs from database");
                }
            })
            .context("failed to clean offline configs")?;
    }
    let reachable_count = ranked.iter().filter(|item| item.reachable).count();
    let fetch_bytes = progress_state.fetch_bytes;
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
        refresh_duration_ms: Some(started_instant.elapsed().as_millis()),
        refreshing: false,
        total_candidates: fetched_count,
        tested_candidates: ranked.len(),
        reachable_candidates: reachable_count,
        fetch_bytes,
        speedtest_bytes,
        fetch_errors,
        ranked,
        stable_working_counts,
    };

    let failed_count = runtime
        .tested_candidates
        .saturating_sub(runtime.reachable_candidates);
    let summary = format!(
        "Started {}, End: {} (Took {}), {} Fetched, {} Failed, {} Working.",
        started_at.with_timezone(&Local).format("%H:%M:%S"),
        finished_at.with_timezone(&Local).format("%H:%M:%S"),
        format_minutes_seconds(runtime.refresh_duration_ms.unwrap_or_default()),
        runtime.total_candidates,
        failed_count,
        runtime.reachable_candidates
    );
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
) -> ProbeStopPolicy {
    let current_keys = candidates
        .iter()
        .map(|candidate| candidate.dedup_key.as_str())
        .collect::<HashSet<_>>();

    ProbeStopPolicy {
        scan_all_configs: config.scan_all_configs,
        top_n: config.top_n,
        prioritize_stability: config.prioritize_stability,
        return_configs_asap: config.return_configs_asap,
        previous_working_keys: previous_top_n
            .iter()
            .filter(|key| current_keys.contains(key.as_str()))
            .cloned()
            .collect(),
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

fn spawn_refresh_loop(
    mut config_rx: watch::Receiver<AppConfig>,
    database: Arc<Database>,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
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
                    print_terminal_summary,
                    print_compact_progress,
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
                    database.clone(),
                    state.clone(),
                    runtime_config.clone(),
                    print_terminal_summary,
                    print_compact_progress,
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
                () = &mut sleep => {
                    let config = config_rx.borrow().clone();
                    last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                    mark_refresh_pending(&state).await;
                    if let Err(err) = refresh_once(&config, database.clone(), state.clone(), runtime_config.clone(), print_terminal_summary, print_compact_progress).await {
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
                    if let Err(err) = refresh_once(&config, database.clone(), state.clone(), runtime_config.clone(), print_terminal_summary, print_compact_progress).await {
                        error!(error = %err, "refresh after config reload failed");
                        record_refresh_error(&state, err.to_string()).await;
                    }
                }
            }
        }
    });
}

async fn mark_refresh_pending(state: &Arc<RwLock<RuntimeState>>) {
    let mut state = state.write().await;
    state.refreshing = true;
    state.refresh_started_at = Some(Utc::now().to_rfc3339());
    state.refresh_finished_at = None;
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

fn spawn_tui_progress_forwarder(
    state: Arc<RwLock<RuntimeState>>,
    previous_top_n: HashSet<String>,
    print_compact_progress: bool,
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
            push_tui_progress(&state, event, &previous_top_n).await;
        }
    });
    (tx, task)
}

async fn push_tui_progress(
    state: &Arc<RwLock<RuntimeState>>,
    event: ProgressEvent,
    previous_top_n: &HashSet<String>,
) {
    let mut state = state.write().await;
    match event {
        ProgressEvent::LiveLog(message) => push_live_log(&mut state, timestamped_log(message)),
        ProgressEvent::ProbeDelta { tested, working } => {
            state.tested_candidates = state.tested_candidates.saturating_add(tested);
            state.reachable_candidates = state.reachable_candidates.saturating_add(working);
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

fn format_minutes_seconds(ms: u128) -> String {
    let total_seconds = millis_to_seconds(ms);
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02} minutes")
}

fn millis_to_seconds(ms: u128) -> u64 {
    u64::try_from(ms / 1000).unwrap_or(u64::MAX)
}

impl From<&AppConfig> for RuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            bind: config.bind,
            top_n: config.top_n,
            refresh_seconds: config.refresh_seconds,
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
        )
        .await;

        let state = state.read().await;
        assert_eq!(state.reachable_candidates, 5);
        assert_eq!(state.ranked.len(), 1);
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
        )
        .await;

        let state = state.read().await;
        assert_eq!(state.ranked[0].stability_count, 3);
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
}
