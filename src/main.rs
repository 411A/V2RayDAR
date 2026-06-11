mod config;
mod constants;
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
        APP_DATA_DIR_NAME, APP_NAME, CACHE_DIR_NAME, CACHE_METADATA_FILE_NAME, CONFIG_FILE_NAME,
        CONFIG_WATCH_INTERVAL, DEFAULT_LOG_FILTER_PLAIN, DEFAULT_LOG_FILTER_TUI,
        DEFAULT_LOG_FILTER_VERBOSE, FIREWALL_STATE_FILE_NAME, LEGACY_APP_MARKER_FILE_NAME,
        LEGACY_CACHE_MARKER_FILE_NAME, LOCALHOST_IP, MAX_TUI_LOGS, SING_BOX_DOWNLOAD_URL,
        STABLE_WORKING_APPEARANCES,
    },
    model::{Candidate, ProbeStopPolicy, ProgressEvent, RankedConfig, RuntimeConfig, RuntimeState},
    paths::AppPaths,
    probe::probe_candidates,
    server::serve,
    sing_box::{active_probe_needs_setup, setup_guide},
    subscription::{
        FetchFailure, FetchOutcome, load_cached_candidates, load_candidates_with_cache,
        retry_failed_sources_with_proxy,
    },
    terminal::{PlainProgressReporter, print_log, print_startup, print_summary},
};

#[derive(Debug, Parser)]
#[command(name = "v2raydar")]
#[command(about = "Fast V2Ray subscription reachability scanner and local top-N endpoint")]
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
}

#[tokio::main]
async fn main() -> Result<()> {
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

    if !paths.config_path.exists() {
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
    } else {
        paths.ensure().await?;
    }

    let mut config = load_config_and_persist_generated_token(&paths.config_path)
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
        print_startup(&config, &paths, cli.verbose);
        refresh_once(
            &config,
            &paths.cache_dir,
            state.clone(),
            runtime_config.clone(),
            true,
            !cli.verbose,
        )
        .await?;
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
        paths.cache_dir.clone(),
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
            result = tui::run(config, paths, state, runtime_config) => result,
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
        CONFIG_FILE_NAME | FIREWALL_STATE_FILE_NAME | LEGACY_APP_MARKER_FILE_NAME => {
            Ok(file_type.is_file())
        }
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

    Ok(file_type.is_file()
        && (name == LEGACY_CACHE_MARKER_FILE_NAME || is_subscription_cache_file_name(name)))
}

fn is_subscription_cache_file_name(name: &str) -> bool {
    if name == CACHE_METADATA_FILE_NAME || subscription::is_cache_snapshot_file_name(name) {
        return true;
    }
    if let Some(key) = name.strip_suffix(".body") {
        return is_cache_key(key);
    }
    if let Some(key) = name.strip_suffix(".json") {
        return is_cache_key(key);
    }
    false
}

fn is_cache_key(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
    println!("Download: {}", SING_BOX_DOWNLOAD_URL);
    println!("Choose the release asset: {}", guide.release_asset);
    println!("Use the executable named: {}", guide.executable_name);
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
    previous_before_refresh: &RuntimeState,
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
    )
    .await;

    let stop_policy = probe_stop_policy(config, previous_before_refresh, &candidates);
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
    let (config, generated_token_requested) = AppConfig::load_with_generated_token_flag(path)?;
    if generated_token_requested {
        tui::util::save_config(path, &config)?;
    }
    Ok(config)
}

async fn refresh_once(
    config: &AppConfig,
    cache_dir: &Path,
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
    let (progress_tx, progress_task) =
        spawn_tui_progress_forwarder(state.clone(), print_compact_progress);
    if print_compact_progress {
        print_log(format!(
            "Refresh started at {}.",
            started_at.with_timezone(&Local).format("%H:%M:%S")
        ));
    }
    *runtime_config.write().await = RuntimeConfig::from(config);
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
        push_live_log(
            &mut runtime,
            timestamped_log(format!(
                "Refresh started at {}",
                started_at.with_timezone(&Local).format("%H:%M:%S")
            )),
        );
    }

    let fetch_started = std::time::Instant::now();
    info!("subscription load started");
    let cache_only = config.use_cache_only;
    let mut fetched = if cache_only {
        load_cached_candidates(
            config,
            cache_dir,
            |bytes| {
                let state = state.clone();
                async move {
                    add_fetch_bytes(&state, bytes).await;
                }
            },
            Some(progress_tx.clone()),
        )
        .await?
    } else {
        load_candidates_with_cache(
            config,
            Some(cache_dir),
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
    let mut seen_candidate_uris = fetched
        .candidates
        .iter()
        .map(|candidate| candidate.uri.clone())
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
    {
        let mut runtime = state.write().await;
        runtime.total_candidates = fetched_count;
        runtime.fetch_errors = fetch_errors.clone();
        push_live_log(
            &mut runtime,
            timestamped_log(format!(
                "Subscription loading finished: {} configs, {} source errors in {}",
                fetched_count,
                fetch_errors.len(),
                format_duration_short(fetch_started.elapsed().as_millis())
            )),
        );
    }

    let probe_started = std::time::Instant::now();
    let mut ranked = probe_refresh_candidates(
        std::mem::take(&mut fetched.candidates),
        config,
        &previous_before_refresh,
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
                Some(cache_dir),
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
                    retry
                        .candidates
                        .retain(|candidate| seen_candidate_uris.insert(candidate.uri.clone()));
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
                    {
                        let mut runtime = state.write().await;
                        runtime.total_candidates = fetched_count;
                        runtime.fetch_errors = fetch_errors.clone();
                        push_live_log(
                            &mut runtime,
                            timestamped_log(format!(
                                "Subscription retry finished: {} new configs from {} fetched links; {} source errors remain",
                                retry_count,
                                retry_before_dedup,
                                fetch_errors.len()
                            )),
                        );
                    }
                    if retry_count > 0 {
                        let mut retry_ranked = probe_refresh_candidates(
                            std::mem::take(&mut retry.candidates),
                            config,
                            &previous_before_refresh,
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
            )
            .await;
        }
    }
    if !cache_only && fresh_success_count == 0 {
        let cache_started = std::time::Instant::now();
        match load_cached_candidates(
            config,
            cache_dir,
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
            Ok(mut cached) => {
                let cache_before_dedup = cached.candidates.len();
                cached
                    .candidates
                    .retain(|candidate| seen_candidate_uris.insert(candidate.uri.clone()));
                let cache_count = cached.candidates.len();
                fetched_count = fetched_count.saturating_add(cache_count);
                if !cached.errors.is_empty() {
                    fetch_errors = cached.errors.clone();
                }
                info!(
                    parsed = cache_before_dedup,
                    unique = cache_count,
                    cache_errors = cached.errors.len(),
                    duration_ms = cache_started.elapsed().as_millis(),
                    "cached subscription fallback finished"
                );
                if print_compact_progress {
                    print_log(format!(
                        "Cache fallback finished: {} configs, {} source errors in {}.",
                        cache_count,
                        cached.errors.len(),
                        format_duration_short(cache_started.elapsed().as_millis())
                    ));
                }
                {
                    let mut runtime = state.write().await;
                    runtime.total_candidates = fetched_count;
                    runtime.fetch_errors = fetch_errors.clone();
                    push_live_log(
                        &mut runtime,
                        timestamped_log(format!(
                            "Cache fallback finished: {} configs from {} cached links",
                            cache_count, cache_before_dedup
                        )),
                    );
                }
                if cache_count > 0 {
                    let mut cached_ranked = probe_refresh_candidates(
                        std::mem::take(&mut cached.candidates),
                        config,
                        &previous_before_refresh,
                        &state,
                        &progress_tx,
                        print_compact_progress,
                        "Cache probe",
                    )
                    .await;
                    ranked.append(&mut cached_ranked);
                }
            }
            Err(err) => {
                let error = err.to_string();
                warn!(error = %error, "cached subscription fallback failed");
                push_tui_progress(
                    &state,
                    ProgressEvent::LiveLog(format!("Cache fallback failed: {error}")),
                )
                .await;
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
    apply_stability_ranking(
        &mut ranked,
        &mut stable_working_counts,
        config.prioritize_stability,
    );
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
    previous: &RuntimeState,
    candidates: &[crate::model::Candidate],
) -> ProbeStopPolicy {
    let current_uris = candidates
        .iter()
        .map(|candidate| candidate.uri.as_str())
        .collect::<HashSet<_>>();

    ProbeStopPolicy {
        scan_all_configs: config.scan_all_configs,
        top_n: config.top_n,
        prioritize_stability: config.prioritize_stability,
        previous_working_uris: previous
            .ranked
            .iter()
            .filter(|item| item.reachable && current_uris.contains(item.uri.as_str()))
            .map(|item| item.uri.clone())
            .collect(),
    }
}

fn apply_stability_ranking(
    ranked: &mut [RankedConfig],
    stable_working_counts: &mut HashMap<String, u32>,
    prioritize_stability: bool,
) {
    for item in ranked.iter_mut() {
        if item.reachable {
            let count = stable_working_counts.entry(item.uri.clone()).or_default();
            *count = count.saturating_add(1);
            item.stability_count = *count;
        } else {
            item.stability_count = stable_working_counts.get(&item.uri).copied().unwrap_or(0);
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
        .then_with(|| is_stable(right).cmp(&is_stable(left)))
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

fn is_stable(config: &RankedConfig) -> bool {
    config.reachable && config.stability_count >= STABLE_WORKING_APPEARANCES
}

fn spawn_refresh_loop(
    mut config_rx: watch::Receiver<AppConfig>,
    cache_dir: PathBuf,
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
                    &cache_dir,
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
                    &cache_dir,
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
                _ = &mut sleep => {
                    let config = config_rx.borrow().clone();
                    last_refresh_fingerprint = Some(RefreshFingerprint::from(&config));
                    mark_refresh_pending(&state).await;
                    if let Err(err) = refresh_once(&config, &cache_dir, state.clone(), runtime_config.clone(), print_terminal_summary, print_compact_progress).await {
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
                    if let Err(err) = refresh_once(&config, &cache_dir, state.clone(), runtime_config.clone(), print_terminal_summary, print_compact_progress).await {
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
struct RefreshFingerprint {
    top_n: usize,
    encoded_subscription: bool,
    prioritize_stability: bool,
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
            push_tui_progress(&state, event).await;
        }
    });
    (tx, task)
}

async fn push_tui_progress(state: &Arc<RwLock<RuntimeState>>, event: ProgressEvent) {
    let mut state = state.write().await;
    match event {
        ProgressEvent::LiveLog(message) => push_live_log(&mut state, timestamped_log(message)),
        ProgressEvent::ProbeDelta { tested, working } => {
            state.tested_candidates = state.tested_candidates.saturating_add(tested);
            state.reachable_candidates = state.reachable_candidates.saturating_add(working);
        }
        ProgressEvent::RankedSnapshot(mut ranked) => {
            apply_snapshot_ranks(&mut ranked);
            state.ranked = ranked;
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

fn timestamped_log(message: impl Into<String>) -> String {
    format!("{} {}", Local::now().format("%H:%M:%S"), message.into())
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
    let seconds = (ms / 1000) as u64;
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes}m {seconds}s")
}

fn format_minutes_seconds(ms: u128) -> String {
    let total_seconds = (ms / 1000) as u64;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02} minutes")
}

impl From<&AppConfig> for RuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            bind: config.bind,
            top_n: config.top_n,
            refresh_seconds: config.refresh_seconds,
            encoded_subscription: config.encoded_subscription,
            prioritize_stability: config.prioritize_stability,
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
        write_file(&cache_dir.join("0123456789abcdef.body"), "body");
        write_file(&cache_dir.join("0123456789abcdef.json"), "{}");
    }

    fn remove_test_root(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }

    fn ranked(name: &str, uri: &str, reachable: bool, latency_ms: Option<u128>) -> RankedConfig {
        RankedConfig {
            rank: 0,
            stability_count: 0,
            id: uri.to_string(),
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

        apply_stability_ranking(&mut ranked, &mut counts, true);

        assert_eq!(ranked[0].name, "slow-stable");
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[0].stability_count, 3);
        assert_eq!(ranked[1].name, "fast-new");
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

        apply_stability_ranking(&mut ranked, &mut counts, false);

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

        apply_stability_ranking(&mut ranked, &mut counts, true);

        assert_eq!(ranked[0].name, "working-new");
        assert!(ranked[0].reachable);
        assert_eq!(ranked[1].name, "failed-stable");
        assert!(!ranked[1].reachable);
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
        )
        .await;

        let state = state.read().await;
        assert_eq!(state.reachable_candidates, 5);
        assert_eq!(state.ranked.len(), 1);
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
                paths.cache_dir.join("0123456789abcdef.body"),
                paths.cache_dir.join("0123456789abcdef.json"),
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
}
