use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use futures_util::{StreamExt, stream};
use reqwest::Proxy;
use serde_json::{Map, Value, json};
use tokio::{
    fs,
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    process::Command,
    sync::mpsc::UnboundedSender,
    time::Instant,
};
use tracing::{debug, info, warn};
use url::Url;

use crate::{
    config::{ProbeConfig, ProbeMode},
    constants::{
        ACTIVE_PROBE_BATCH_CONCURRENCY_MULTIPLIER, ACTIVE_PROBE_BATCH_MAX_SIZE,
        ACTIVE_PROBE_BATCH_MIN_SIZE, ACTIVE_PROBE_HTTP_MAX_CONCURRENCY,
        ACTIVE_PROBE_PROCESS_MAX_CONCURRENCY, BITS_PER_BYTE, BITS_PER_MEGABIT,
        HTTP_EXCHANGE_OVERHEAD_BYTES, LOCAL_PROXY_CONNECT_TIMEOUT, LOCAL_PROXY_WAIT_INTERVAL,
        LOCALHOST_IP, SING_BOX_CLEANUP_TIMEOUT, SING_BOX_CONFIG_FILE_PREFIX,
        SING_BOX_INBOUND_TAG_PREFIX, SING_BOX_OUTBOUND_TAG_PREFIX,
    },
    convert::{
        decode_base64_bytes, decode_base64_to_string, first_param, json_string, json_u16, json_u64,
        parse_host_port, percent_decode, query_pairs, split_once, truthy,
    },
    model::{Candidate, ProbeStopPolicy, ProgressEvent, RankedConfig},
};

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
struct CachedSingBox {
    path: String,
    available: bool,
    version: Option<(u32, u32, u32)>,
}

static SING_BOX_CACHE: OnceLock<Mutex<Option<CachedSingBox>>> = OnceLock::new();

fn sing_box_cache() -> &'static Mutex<Option<CachedSingBox>> {
    SING_BOX_CACHE.get_or_init(|| Mutex::new(None))
}

/// Parse sing-box version string like "1.13.14" into (major, minor, patch).
fn parse_sing_box_version(output: &str) -> Option<(u32, u32, u32)> {
    let first_line = output.lines().next()?;
    let version_str = first_line.strip_prefix("sing-box version ")?;
    let parts: Vec<u32> = version_str
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() >= 3 {
        Some((parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

/// Major version of a sing-box binary (`None` when it cannot run).
///
/// Powers feature gates without requiring a prior probe run (TCP mode never
/// probes, so the availability cache may be cold at proxy start).
pub async fn sing_box_major_version(path: &str) -> Option<u32> {
    let output = Command::new(path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_sing_box_version(&String::from_utf8_lossy(&output.stdout)).map(|(major, _, _)| major)
}

/// Check if the detected sing-box version is >= (major, minor, patch).
///
/// Uses the version from the last successful availability check.
/// Returns `false` when no successful check has been cached yet.
/// This is a cheap in-memory read with no I/O, safe for hot paths.
pub fn sing_box_version_at_least(major: u32, minor: u32, patch: u32) -> bool {
    let version = sing_box_cache()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .and_then(|cached| cached.version);
    match version {
        Some((maj, min, pat)) => (maj, min, pat) >= (major, minor, patch),
        None => false,
    }
}

/// Check sing-box availability, cached per path.
///
/// The cache key includes the path so TUI path changes take effect
/// without a restart. Re-checks only happen when the path differs from
/// the cached entry, so steady-state probing stays at zero extra cost.
async fn sing_box_available(path: &str) -> bool {
    if let Ok(guard) = sing_box_cache().lock()
        && let Some(cached) = guard.as_ref()
        && cached.path == path
    {
        return cached.available;
    }

    debug!(sing_box_path = %path, "checking sing-box availability");
    let started = Instant::now();
    let output = Command::new(path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok();
    let available = output.as_ref().is_some_and(|o| o.status.success());
    let mut version = None;
    if let Some(ref out) = output
        && let Some(detected) = parse_sing_box_version(&String::from_utf8_lossy(&out.stdout))
    {
        version = Some(detected);
        info!(
            sing_box_path = %path,
            version = ?detected,
            "sing-box version detected"
        );
    }
    info!(
        sing_box_path = %path,
        available,
        duration_ms = started.elapsed().as_millis(),
        "sing-box availability check finished"
    );
    if let Ok(mut guard) = sing_box_cache().lock() {
        *guard = Some(CachedSingBox {
            path: path.to_string(),
            available,
            version,
        });
    }
    available
}

/// Probe `candidates`, stopping early per `stop_policy`.
///
/// `cancel` is an external preemption switch (a refresh interrupting a ping):
/// when set, in-flight probes finish their current item and the run returns
/// whatever was verified so far instead of the full list. `None` probes to
/// completion (normal refreshes, CLI ping).
pub async fn probe_candidates(
    candidates: Vec<Candidate>,
    config: &ProbeConfig,
    progress: Option<UnboundedSender<ProgressEvent>>,
    stop_policy: &ProbeStopPolicy,
    cancel: Option<Arc<AtomicBool>>,
) -> Vec<RankedConfig> {
    info!(
        mode = ?config.mode,
        candidates = candidates.len(),
        concurrency = config.concurrency,
        batch_size = ?config.batch_size,
        "probe candidate queue received"
    );
    send_progress(
        progress.as_ref(),
        format!(
            "Testing {} loaded configs with {:?} mode (concurrency {})",
            candidates.len(),
            config.mode,
            config.concurrency
        ),
    );
    if config.mode == ProbeMode::Active && !sing_box_available(&config.sing_box_path).await {
        warn!(
            sing_box_path = %config.sing_box_path,
            "sing-box availability check failed"
        );
        let failed = candidates.len();
        send_progress(
            progress.as_ref(),
            format!(
                "sing-box unavailable at '{}'; marking {failed} configs failed",
                config.sing_box_path
            ),
        );
        send_probe_delta(progress.as_ref(), failed, 0, 0);
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

    let cancel = cancel.unwrap_or_default();
    let ranked = match config.mode {
        ProbeMode::Active => {
            probe_active_batched(
                candidates,
                config,
                progress.clone(),
                stop_policy,
                Some(cancel.clone()),
            )
            .await
        }
        ProbeMode::Tcp => {
            if !stop_policy.scan_all_configs {
                send_progress(
                    progress.as_ref(),
                    "Early stop is disabled in TCP diagnostic mode; active sing-box validation is required for shortcut results",
                );
            }
            let mut results = stream::iter(candidates.into_iter().map(|candidate| async move {
                probe_tcp(candidate, Duration::from_millis(config.connect_timeout_ms)).await
            }))
            .buffer_unordered(config.concurrency);
            let mut ranked = Vec::new();
            // Dropping the stream cancels in-flight connects; one
            // already-finished result at most slips through per check.
            while let Some(result) = results.next().await {
                if cancel.load(AtomicOrdering::Relaxed) {
                    break;
                }
                send_probe_delta(progress.as_ref(), 1, usize::from(result.reachable), 0);
                ranked.push(result);
                send_asap_configs(progress.as_ref(), &ranked, stop_policy);
            }
            ranked
        }
    };

    info!(ranked = ranked.len(), "probe candidate queue completed");
    send_progress(
        progress.as_ref(),
        format!("Probe queue finished: {} results", ranked.len()),
    );
    rank_configs(ranked)
}

fn rank_configs(mut ranked: Vec<RankedConfig>) -> Vec<RankedConfig> {
    ranked.sort_by(compare_ranked);
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }

    ranked
}

/// Standalone ping: test config URIs without starting the full refresh cycle.
///
/// Runs TCP probe on all URIs, then optionally active probe if sing-box is
/// available. Returns results sorted by latency.
pub async fn ping_configs(uris: Vec<String>, config: &ProbeConfig) -> Vec<RankedConfig> {
    let candidates: Vec<Candidate> = uris
        .into_iter()
        .filter_map(|uri| crate::parser::parse_share_link("ping", 1, &uri).ok())
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    // Always run TCP probe first (no sing-box needed).
    let mut results: Vec<RankedConfig> = stream::iter(candidates.into_iter().map(|candidate| {
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        async move { probe_tcp(candidate, timeout).await }
    }))
    .buffer_unordered(config.concurrency)
    .collect()
    .await;

    // If sing-box is available, upgrade with active probing.
    if config.mode == ProbeMode::Active && sing_box_available(&config.sing_box_path).await {
        let stop_policy = ProbeStopPolicy {
            scan_all_configs: true,
            top_n: results.len(),
            prioritize_stability: false,
            return_configs_asap: false,
            previous_working_keys: std::collections::HashSet::new(),
            prefound_working: 0,
            prefound_previous_working: 0,
        };
        let active_results = probe_active_batched(
            candidates_from_ranked(&results),
            config,
            None,
            &stop_policy,
            None,
        )
        .await;
        if !active_results.is_empty() {
            results = active_results;
        }
    }

    rank_configs(results)
}

fn candidates_from_ranked(ranked: &[RankedConfig]) -> Vec<Candidate> {
    ranked
        .iter()
        .map(|r| Candidate {
            id: r.id.clone(),
            dedup_key: r.dedup_key.clone(),
            source: r.source.clone(),
            priority: r.priority,
            protocol: r.protocol.clone(),
            name: r.name.clone(),
            endpoint: r.endpoint.clone(),
            uri: r.uri.clone(),
        })
        .collect()
}

async fn probe_tcp(candidate: Candidate, timeout: Duration) -> RankedConfig {
    let target = (candidate.endpoint.host.as_str(), candidate.endpoint.port);
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, TcpStream::connect(target)).await;

    let (reachable, latency_ms, error, country_code) = match result {
        Ok(Ok(stream)) => {
            let ip = stream.peer_addr().ok().map(|addr| addr.ip());
            let cc = ip.and_then(crate::geoip::lookup_country);
            (true, Some(started.elapsed().as_millis()), None, cc)
        }
        Ok(Err(err)) => (false, None, Some(err.to_string()), None),
        Err(_) => (
            false,
            None,
            Some(format!("timed out after {} ms", timeout.as_millis())),
            None,
        ),
    };

    RankedConfig {
        rank: 0,
        stability_count: 0,
        id: candidate.id,
        dedup_key: candidate.dedup_key,
        source: candidate.source.clone(),
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
        country_code,
    }
}

struct PreparedActiveCandidate {
    candidate: Candidate,
    aliases: Vec<Candidate>,
    outbound: Value,
}

impl PreparedActiveCandidate {
    const fn candidate_count(&self) -> usize {
        1 + self.aliases.len()
    }

    fn into_candidate(self) -> Candidate {
        self.candidate
    }
}

struct BatchProbeFailure {
    entries: Vec<PreparedActiveCandidate>,
    failed_entry: Option<PreparedActiveCandidate>,
    error: anyhow::Error,
    retry_split: bool,
}

impl BatchProbeFailure {
    const fn retryable(entries: Vec<PreparedActiveCandidate>, error: anyhow::Error) -> Self {
        Self {
            entries,
            failed_entry: None,
            error,
            retry_split: true,
        }
    }

    const fn unrecoverable(entries: Vec<PreparedActiveCandidate>, error: anyhow::Error) -> Self {
        Self {
            entries,
            failed_entry: None,
            error,
            retry_split: false,
        }
    }

    const fn invalid_entry(
        entries: Vec<PreparedActiveCandidate>,
        failed_entry: PreparedActiveCandidate,
        error: anyhow::Error,
    ) -> Self {
        Self {
            entries,
            failed_entry: Some(failed_entry),
            error,
            retry_split: false,
        }
    }
}

struct ReservedLocalPort {
    port: u16,
    _listener: TcpListener,
}

struct ActiveProbeSuccess {
    latency_ms: u128,
    http_status: u16,
    download_mbps: Option<f64>,
    download_bytes: Option<usize>,
    /// Observed request + response-header bytes (no body read, no extra I/O).
    bytes: u64,
}

struct ActivePreparation {
    prepared: Vec<PreparedActiveCandidate>,
    ranked: Vec<RankedConfig>,
    prepared_candidates: usize,
}

#[derive(Debug, Clone)]
struct ProbeStopState {
    half_snapshot_sent: bool,
    stability_search_exhausted: bool,
    remaining_previous_working: usize,
}

struct ActiveBatchSizer {
    current: usize,
    min: usize,
    max: usize,
}

impl ActiveBatchSizer {
    fn new(concurrency: usize, configured: Option<usize>) -> Self {
        let initial = active_probe_batch_size(concurrency, configured);
        let max = ACTIVE_PROBE_BATCH_MAX_SIZE;
        Self {
            current: initial.min(max).max(1),
            min: concurrency.clamp(1, ACTIVE_PROBE_BATCH_MIN_SIZE),
            max: max.max(1),
        }
    }

    fn next_len(&self, remaining: usize) -> usize {
        remaining.min(self.current)
    }

    fn observe(&mut self, stats: &BatchProbeStats) {
        if stats.splits > 0 || stats.failed_candidates > 0 {
            self.current = (self.current / 2).max(self.min).max(1);
        } else if stats.started_cleanly && stats.produced >= self.current {
            self.current = self
                .current
                .saturating_mul(2)
                .min(self.max)
                .max(self.min)
                .max(1);
        }
    }
}

#[derive(Debug, Default)]
struct BatchProbeStats {
    started_cleanly: bool,
    produced: usize,
    splits: usize,
    failed_candidates: usize,
}

impl ProbeStopState {
    fn new(prepared: &[PreparedActiveCandidate], policy: &ProbeStopPolicy) -> Self {
        let remaining_previous_working = if policy.scan_all_configs || !policy.prioritize_stability
        {
            0
        } else {
            previous_working_entry_count(prepared, policy)
        };
        Self {
            half_snapshot_sent: false,
            stability_search_exhausted: !policy.previous_working_keys.is_empty()
                && remaining_previous_working == 0,
            remaining_previous_working,
        }
    }
}

/// Results plus stop-search state shared by every batch of one
/// `probe_active_batched` run.
///
/// Previously each batch evaluated early-stop against only its own results,
/// so N concurrent batches each probed toward `top_n` on their own — and the
/// wave loop kept whichever batch outcome arrived first, which could even be
/// an empty loser that discarded a sibling's real results. Sharing fixes both
/// without touching concurrency at all (same waves, batches, processes):
///
/// - every completed probe appends here first, so finished work is never lost
///   no matter which batch outcome arrives first;
/// - the stop condition sees everything found so far across all batches, so
///   every batch quits within ~one probe of the global target.
struct SharedProbeState {
    ranked: Vec<RankedConfig>,
    stop: ProbeStopState,
    stop_announced: bool,
}

type SharedProbe = std::sync::Arc<std::sync::Mutex<SharedProbeState>>;

fn lock_shared(shared: &SharedProbe) -> std::sync::MutexGuard<'_, SharedProbeState> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Evaluate the shared stop condition; announces the reason once.
/// Pure decision wrapper around [`probe_stop_reason`] — messaging behavior
/// (half snapshot, reason text) is unchanged, it just fires globally now.
fn eval_shared_stop(
    shared: &SharedProbe,
    policy: &ProbeStopPolicy,
    progress: Option<&UnboundedSender<ProgressEvent>>,
) -> Option<String> {
    let mut shared = lock_shared(shared);
    if shared.stop_announced {
        return None;
    }
    let reason = {
        let SharedProbeState { ranked, stop, .. } = &mut *shared;
        probe_stop_reason(ranked, policy, stop, progress)
    };
    if reason.is_some() {
        shared.stop_announced = true;
    }
    reason
}

#[allow(clippy::too_many_lines)]
/// Stop signaling for one active-probe run.
///
/// `external` is the caller's preemption switch (a refresh interrupting a
/// ping) — the probe only ever reads it. `target_reached` is the
/// run-internal early-stop broadcast that batches raise among themselves so
/// sibling waves quit within ~one probe of the target.
///
/// The two flags must stay separate: an early stop is a successful finish,
/// and writing it into the external flag made every ping that reached its
/// target report "preempted by refresh" — and left the flag set, so all
/// later pings probed nothing until the next refresh cleared it.
#[derive(Clone)]
struct ProbeStopSignal {
    external: Arc<AtomicBool>,
    target_reached: Arc<AtomicBool>,
}

impl ProbeStopSignal {
    fn from_external(cancel: Option<Arc<AtomicBool>>) -> Self {
        Self {
            external: cancel.unwrap_or_default(),
            target_reached: Arc::new(AtomicBool::new(false)),
        }
    }

    fn stopped(&self) -> bool {
        self.external.load(AtomicOrdering::Relaxed)
            || self.target_reached.load(AtomicOrdering::Relaxed)
    }

    fn broadcast_target_reached(&self) {
        self.target_reached.store(true, AtomicOrdering::Relaxed);
    }
}

/// Wave of sing-box batches plus the previous-working entries it carries
/// for the stability search update.
type ProbeWave = Vec<(usize, Vec<PreparedActiveCandidate>)>;

async fn probe_active_batched(
    candidates: Vec<Candidate>,
    config: &ProbeConfig,
    progress: Option<UnboundedSender<ProgressEvent>>,
    stop_policy: &ProbeStopPolicy,
    cancel: Option<Arc<AtomicBool>>,
) -> Vec<RankedConfig> {
    let started = Instant::now();
    let (mut prepared, prepared_ranked) =
        prepare_batched_probe_inputs(candidates, stop_policy, progress.as_ref()).await;

    let process_concurrency = active_probe_process_concurrency(config.process_concurrency);
    let mut batch_sizer = ActiveBatchSizer::new(config.concurrency, config.batch_size);
    let shared: SharedProbe = Arc::new(Mutex::new(SharedProbeState {
        ranked: prepared_ranked,
        stop: ProbeStopState::new(&prepared, stop_policy),
        stop_announced: false,
    }));
    // The external switch only stops batches; the early-stop broadcast
    // below uses the run-internal flag so a reached target never looks
    // like an external preemption to the caller.
    let stop = ProbeStopSignal::from_external(cancel);
    let mut batch_index = 0_usize;
    while !prepared.is_empty() && !stop.stopped() {
        let before = lock_shared(&shared).ranked.len();
        let (wave, wave_previous_working) = queue_probe_wave(
            &mut prepared,
            process_concurrency,
            &batch_sizer,
            &mut batch_index,
            stop_policy,
            progress.as_ref(),
            &stop,
        );

        let wave_started = Instant::now();
        let mut outcomes = stream::iter(wave.into_iter().map(|(batch_index, batch)| {
            let progress = progress.clone();
            let stop = stop.clone();
            let shared = shared.clone();
            async move {
                let batch_started = Instant::now();
                let batch_stop_policy = stop_policy.clone();
                let outcome = probe_active_batch_with_fallback(
                    batch_index,
                    batch,
                    config,
                    progress.as_ref(),
                    &batch_stop_policy,
                    &shared,
                    stop,
                )
                .await;
                (batch_index, batch_started.elapsed(), outcome)
            }
        }))
        .buffer_unordered(process_concurrency);

        // Results already live in the shared list the moment each probe
        // completes, so a batch outcome arriving first can neither strand
        // real results nor require re-evaluation here: per-probe evaluation
        // inside the batches is complete (any threshold crossing coincides
        // with a probe completion). Late outcomes only carry sizing stats.
        while let Some((finished_batch_index, batch_duration, batch_stats)) = outcomes.next().await
        {
            batch_sizer.observe(&batch_stats);
            info!(
                batch_index = finished_batch_index,
                next_batch_size = batch_sizer.current,
                duration_ms = batch_duration.as_millis(),
                "active probe batch finished"
            );
            if stop.stopped() {
                break;
            }
        }
        {
            let mut shared = lock_shared(&shared);
            update_stability_search_after_batch(
                wave_previous_working,
                stop_policy,
                &mut shared.stop,
            );
        }
        let after = lock_shared(&shared).ranked.len();
        send_progress(
            progress.as_ref(),
            format!(
                "Batch wave finished: {} configs checked in {}",
                after.saturating_sub(before),
                format_duration_short(wave_started.elapsed())
            ),
        );
    }

    let ranked = std::mem::take(&mut lock_shared(&shared).ranked);
    finish_batched_probe(ranked, started, config, progress.as_ref(), stop_policy).await
}

/// CPU-bound preparation (outbound building) plus its logging, lifted out
/// of [`probe_active_batched`] so the wave loop stays readable. Returns the
/// testable definitions alongside configs rejected before any probing.
async fn prepare_batched_probe_inputs(
    candidates: Vec<Candidate>,
    stop_policy: &ProbeStopPolicy,
    progress: Option<&UnboundedSender<ProgressEvent>>,
) -> (Vec<PreparedActiveCandidate>, Vec<RankedConfig>) {
    let input_count = candidates.len();
    let stop_policy_clone = stop_policy.clone();
    let ActivePreparation {
        prepared,
        ranked: prepared_ranked,
        prepared_candidates,
    } = tokio::task::spawn_blocking(move || {
        prepare_active_candidates(candidates, &stop_policy_clone)
    })
    .await
    .expect("prepare_active_candidates panicked");
    let prepared_failed = prepared_ranked.len();
    info!(
        input = input_count,
        prepared = prepared_candidates,
        test_definitions = prepared.len(),
        parse_failed = prepared_failed,
        "active probe preparation finished"
    );
    send_progress(
        progress,
        format!(
            "Prepared active test: {} sing-box definitions represent {} loaded configs; {} unsupported configs skipped",
            prepared.len(),
            prepared_candidates,
            prepared_failed
        ),
    );
    if prepared_failed > 0 {
        send_probe_delta(progress, prepared_failed, 0, 0);
        // A wall of "unsupported" with no reason is undebuggable: name the
        // actual preparation errors (e.g. a cache full of schemes the active
        // prover cannot build) so the next one answers itself in Live Logs.
        let top_errors = top_preparation_errors(&prepared_ranked, 3);
        if !top_errors.is_empty() {
            let summary = top_errors
                .iter()
                .map(|(error, count)| format!("'{error}' x{count}"))
                .collect::<Vec<_>>()
                .join(", ");
            send_progress(
                progress,
                format!("Preparation skipped {prepared_failed} configs; top errors: {summary}"),
            );
        }
    }
    (prepared, prepared_ranked)
}

/// Drain one wave of queued batches into `prepared`, honoring the stop
/// signal between batches. Returns the wave plus the previous-working
/// entries it carries for the stability search update.
fn queue_probe_wave(
    prepared: &mut Vec<PreparedActiveCandidate>,
    process_concurrency: usize,
    batch_sizer: &ActiveBatchSizer,
    batch_index: &mut usize,
    stop_policy: &ProbeStopPolicy,
    progress: Option<&UnboundedSender<ProgressEvent>>,
    stop: &ProbeStopSignal,
) -> (ProbeWave, usize) {
    let mut wave = Vec::new();
    let mut wave_previous_working = 0_usize;
    for _ in 0..process_concurrency {
        if prepared.is_empty() || stop.stopped() {
            break;
        }
        *batch_index += 1;
        let batch_len = batch_sizer.next_len(prepared.len());
        let batch = prepared.drain(..batch_len).collect::<Vec<_>>();
        wave_previous_working =
            wave_previous_working.saturating_add(previous_working_entry_count(&batch, stop_policy));
        let batch_candidates = candidate_count(&batch);
        info!(
            batch_index = *batch_index,
            remaining = prepared.len(),
            batch_size = batch_sizer.current,
            entries = batch.len(),
            candidates = batch_candidates,
            "active probe batch queued"
        );
        send_progress(
            progress,
            format!(
                "Batch {batch_index}: testing {batch_candidates} configs ({} sing-box definitions)",
                batch.len()
            ),
        );
        wave.push((*batch_index, batch));
    }
    (wave, wave_previous_working)
}

/// Final tally, speedtest enrichment, and completion log for a batched run.
async fn finish_batched_probe(
    mut ranked: Vec<RankedConfig>,
    started: Instant,
    config: &ProbeConfig,
    progress: Option<&UnboundedSender<ProgressEvent>>,
    stop_policy: &ProbeStopPolicy,
) -> Vec<RankedConfig> {
    info!(
        ranked = ranked.len(),
        duration_ms = started.elapsed().as_millis(),
        "active probe batches finished"
    );
    enrich_top_speedtests(&mut ranked, config, progress, stop_policy).await;
    send_progress(
        progress,
        format!(
            "Active test finished: {} configs checked in {}",
            ranked.len(),
            format_duration_short(started.elapsed())
        ),
    );
    ranked
}

fn prepare_active_candidates(
    candidates: Vec<Candidate>,
    stop_policy: &ProbeStopPolicy,
) -> ActivePreparation {
    let scheduled = schedule_active_candidates(candidates, stop_policy);
    let num_cpus = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk_size = (scheduled.len() / num_cpus.max(1)).max(1);

    // Process candidates in parallel across threads
    let mut all_prepared = Vec::new();
    let mut all_ranked = Vec::new();

    std::thread::scope(|s| {
        let handles: Vec<_> = scheduled
            .chunks(chunk_size)
            .map(|chunk| {
                s.spawn(move || {
                    let mut prepared = Vec::new();
                    let mut ranked = Vec::new();
                    for candidate in chunk {
                        match sing_box_outbound_from_share_link(&candidate.uri) {
                            Ok(outbound) => {
                                prepared.push(PreparedActiveCandidate {
                                    candidate: candidate.clone(),
                                    aliases: Vec::new(),
                                    outbound,
                                });
                            }
                            Err(err) => {
                                ranked.push(failed_config(
                                    candidate.clone(),
                                    "active_http",
                                    err.to_string(),
                                ));
                            }
                        }
                    }
                    (prepared, ranked)
                })
            })
            .collect();

        for handle in handles {
            let (prepared, ranked) = handle.join().expect("preparation thread panicked");
            all_prepared.extend(prepared);
            all_ranked.extend(ranked);
        }
    });

    // Deduplicate equivalent outbounds (same outbound definition)
    let mut deduped: Vec<PreparedActiveCandidate> = Vec::new();
    let mut seen_keys = HashMap::<String, usize>::new();
    for entry in all_prepared {
        let key = normalized_outbound_key(&entry.outbound);
        if let Some(index) = seen_keys.get(&key).copied() {
            deduped[index].aliases.push(entry.candidate);
        } else {
            seen_keys.insert(key, deduped.len());
            deduped.push(entry);
        }
    }

    let prepared_candidates = candidate_count(&deduped);
    ActivePreparation {
        prepared: deduped,
        ranked: all_ranked,
        prepared_candidates,
    }
}

/// Most frequent preparation errors across failed configs, most common
/// first (ties keep first-seen order), capped at `limit` entries.
fn top_preparation_errors(ranked: &[RankedConfig], limit: usize) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for item in ranked.iter().filter_map(|item| item.error.as_ref()) {
        if !counts.contains_key(item) {
            order.push(item.clone());
        }
        *counts.entry(item.clone()).or_default() += 1;
    }
    let mut grouped: Vec<(String, usize)> = order
        .into_iter()
        .map(|error| {
            let count = counts.get(&error).copied().unwrap_or_default();
            (error, count)
        })
        .collect();
    grouped.sort_by_key(|item| std::cmp::Reverse(item.1));
    grouped.truncate(limit);
    grouped
}

fn normalized_outbound_key(outbound: &Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    outbound.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn schedule_active_candidates(
    candidates: Vec<Candidate>,
    stop_policy: &ProbeStopPolicy,
) -> Vec<Candidate> {
    if candidates.len() <= 1 {
        return candidates;
    }

    let has_previous =
        stop_policy.prioritize_stability && !stop_policy.previous_working_keys.is_empty();
    source_fair_candidates(candidates, |candidate| {
        has_previous
            && stop_policy
                .previous_working_keys
                .contains(&candidate.dedup_key)
    })
}

struct SourceCandidateQueue {
    source: String,
    priority: u32,
    first_index: usize,
    preferred: VecDeque<Candidate>,
    regular: VecDeque<Candidate>,
}

impl SourceCandidateQueue {
    fn pop_front(&mut self) -> Option<Candidate> {
        self.preferred
            .pop_front()
            .or_else(|| self.regular.pop_front())
    }
}

fn source_fair_candidates(
    candidates: Vec<Candidate>,
    is_preferred: impl Fn(&Candidate) -> bool,
) -> Vec<Candidate> {
    let candidate_count = candidates.len();
    if candidate_count <= 1 {
        return candidates;
    }

    let mut queues = Vec::<SourceCandidateQueue>::new();
    let mut queue_indexes = HashMap::<(String, u32), usize>::new();

    for (index, candidate) in candidates.into_iter().enumerate() {
        let preferred = is_preferred(&candidate);
        let key = (candidate.source.clone(), candidate.priority);
        let queue_index = if let Some(queue_index) = queue_indexes.get(&key).copied() {
            queue_index
        } else {
            let queue_index = queues.len();
            queue_indexes.insert(key, queue_index);
            queues.push(SourceCandidateQueue {
                source: candidate.source.clone(),
                priority: candidate.priority,
                first_index: index,
                preferred: VecDeque::new(),
                regular: VecDeque::new(),
            });
            queue_index
        };
        if preferred {
            queues[queue_index].preferred.push_back(candidate);
        } else {
            queues[queue_index].regular.push_back(candidate);
        }
    }

    queues.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.first_index.cmp(&right.first_index))
            .then_with(|| left.source.cmp(&right.source))
    });

    let mut scheduled = Vec::with_capacity(candidate_count);
    while scheduled.len() < candidate_count {
        for queue in &mut queues {
            if let Some(candidate) = queue.pop_front() {
                scheduled.push(candidate);
            }
        }
    }

    scheduled
}

fn candidate_count(entries: &[PreparedActiveCandidate]) -> usize {
    entries
        .iter()
        .map(PreparedActiveCandidate::candidate_count)
        .sum()
}

fn probe_stop_reason(
    ranked: &[RankedConfig],
    policy: &ProbeStopPolicy,
    state: &mut ProbeStopState,
    progress: Option<&UnboundedSender<ProgressEvent>>,
) -> Option<String> {
    if policy.scan_all_configs || policy.top_n == 0 {
        return None;
    }

    let new_reachable = ranked.iter().filter(|item| item.reachable).count();
    // Working configs carried over from a preempted ping count toward the
    // target, so the interrupting refresh only gathers the shortfall.
    let reachable = new_reachable.saturating_add(policy.prefound_working);
    if !policy.prioritize_stability {
        return (reachable >= policy.top_n).then(|| {
            format!(
                "Early stop: found {reachable}/{} working configs with active sing-box checks",
                policy.top_n
            )
        });
    }

    let first_half = policy.top_n / 2;
    let second_half = policy.top_n.saturating_sub(first_half);
    // `new_reachable > 0`: the snapshot replaces the live list, so it must
    // never fire on carried-over results alone with an empty new list.
    if !policy.return_configs_asap
        && first_half > 0
        && reachable >= first_half
        && !state.half_snapshot_sent
        && new_reachable > 0
    {
        state.half_snapshot_sent = true;
        send_ranked_snapshot(progress, stability_snapshot(ranked, policy, first_half));
        send_progress(
            progress,
            format!("Early publish: found first {first_half} working configs"),
        );
    }

    if reachable < policy.top_n {
        return None;
    }

    let found_previous =
        stable_reachable_count(ranked, policy).saturating_add(policy.prefound_previous_working);
    let required_previous = second_half.min(policy.previous_working_keys.len());
    if found_previous >= required_previous {
        return Some(format!(
            "Early stop: found {reachable}/{} working configs, including {found_previous}/{required_previous} kept from the previous run's saved top-N",
            policy.top_n
        ));
    }

    if state.stability_search_exhausted {
        return Some(format!(
            "Early stop: previous-run top-N stability search finished; filling {} configs by active HTTP results",
            policy.top_n
        ));
    }

    None
}

fn stable_reachable_count(ranked: &[RankedConfig], policy: &ProbeStopPolicy) -> usize {
    ranked
        .iter()
        .filter(|item| item.reachable && policy.previous_working_keys.contains(&item.dedup_key))
        .count()
}

fn stability_snapshot(
    ranked: &[RankedConfig],
    policy: &ProbeStopPolicy,
    limit: usize,
) -> Vec<RankedConfig> {
    let mut working = ranked
        .iter()
        .filter(|item| item.reachable)
        .cloned()
        .collect::<Vec<_>>();
    working.sort_by(compare_ranked);

    if !policy.prioritize_stability {
        working.truncate(limit);
        return working;
    }

    let mut selected = Vec::new();
    let mut used_uris = std::collections::HashSet::new();
    for item in working
        .iter()
        .filter(|item| policy.previous_working_keys.contains(&item.dedup_key))
    {
        if selected.len() >= limit {
            break;
        }
        used_uris.insert(item.dedup_key.clone());
        selected.push(item.clone());
    }
    for item in working {
        if selected.len() >= limit {
            break;
        }
        if used_uris.insert(item.dedup_key.clone()) {
            selected.push(item);
        }
    }
    selected
}

fn update_stability_search_after_batch(
    tested_previous_working: usize,
    policy: &ProbeStopPolicy,
    state: &mut ProbeStopState,
) {
    if policy.scan_all_configs || !policy.prioritize_stability || state.stability_search_exhausted {
        return;
    }

    state.remaining_previous_working = state
        .remaining_previous_working
        .saturating_sub(tested_previous_working);
    if !policy.previous_working_keys.is_empty() && state.remaining_previous_working == 0 {
        state.stability_search_exhausted = true;
    }
}

fn previous_working_entry_count(
    entries: &[PreparedActiveCandidate],
    policy: &ProbeStopPolicy,
) -> usize {
    entries
        .iter()
        .filter(|entry| entry_has_previous_working_uri(entry, policy))
        .count()
}

fn entry_has_previous_working_uri(
    entry: &PreparedActiveCandidate,
    policy: &ProbeStopPolicy,
) -> bool {
    policy
        .previous_working_keys
        .contains(&entry.candidate.dedup_key)
        || entry
            .aliases
            .iter()
            .any(|alias| policy.previous_working_keys.contains(&alias.dedup_key))
}

fn send_ranked_snapshot(
    progress: Option<&UnboundedSender<ProgressEvent>>,
    ranked: Vec<RankedConfig>,
) {
    if let Some(progress) = progress {
        let _ = progress.send(ProgressEvent::RankedSnapshot(ranked));
    }
}

fn send_asap_configs(
    progress: Option<&UnboundedSender<ProgressEvent>>,
    ranked: &[RankedConfig],
    policy: &ProbeStopPolicy,
) {
    if !policy.return_configs_asap {
        return;
    }

    let mut seen_keys = std::collections::HashSet::new();
    let working = ranked
        .iter()
        .filter(|item| item.reachable && seen_keys.insert(item.dedup_key.clone()))
        .cloned()
        .collect::<Vec<_>>();
    if working.is_empty() {
        return;
    }

    if let Some(progress) = progress {
        let _ = progress.send(ProgressEvent::WorkingConfigsFound {
            configs: working,
            top_n: policy.top_n,
        });
    }
}

fn active_probe_batch_size(concurrency: usize, configured: Option<usize>) -> usize {
    configured.unwrap_or_else(|| {
        concurrency
            .saturating_mul(ACTIVE_PROBE_BATCH_CONCURRENCY_MULTIPLIER)
            .clamp(ACTIVE_PROBE_BATCH_MIN_SIZE, ACTIVE_PROBE_BATCH_MAX_SIZE)
    })
}

fn active_probe_http_concurrency(configured: usize, batch_entries: usize) -> usize {
    let configured = configured.clamp(1, ACTIVE_PROBE_HTTP_MAX_CONCURRENCY);
    let adaptive_limit = batch_entries
        .max(1)
        .isqrt()
        .saturating_mul(configured)
        .clamp(1, ACTIVE_PROBE_HTTP_MAX_CONCURRENCY);
    adaptive_limit.min(batch_entries.max(1))
}

fn active_probe_process_concurrency(configured: Option<usize>) -> usize {
    let detected = std::thread::available_parallelism().map_or(1, usize::from);
    let automatic = (detected / 2).clamp(1, ACTIVE_PROBE_PROCESS_MAX_CONCURRENCY);
    configured
        .unwrap_or(automatic)
        .clamp(1, ACTIVE_PROBE_PROCESS_MAX_CONCURRENCY)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn probe_active_batch_with_fallback(
    batch_index: usize,
    batch: Vec<PreparedActiveCandidate>,
    config: &ProbeConfig,
    progress: Option<&UnboundedSender<ProgressEvent>>,
    stop_policy: &ProbeStopPolicy,
    shared: &SharedProbe,
    stop: ProbeStopSignal,
) -> BatchProbeStats {
    let mut pending = vec![batch];
    let mut stats = BatchProbeStats::default();

    while let Some(batch) = pending.pop() {
        if stop.stopped() {
            break;
        }
        let batch_len = batch.len();
        match probe_active_batch(
            batch_index,
            batch,
            config,
            progress,
            stop_policy,
            shared,
            stop.clone(),
        )
        .await
        {
            Ok(produced) => {
                stats.started_cleanly = true;
                stats.produced = stats.produced.saturating_add(produced);
            }
            Err(mut failure) => {
                let error = failure.error.to_string();
                if let Some(failed_entry) = failure.failed_entry.take() {
                    stats.failed_candidates = stats
                        .failed_candidates
                        .saturating_add(failed_entry.candidate_count());
                    warn!(
                        remaining = failure.entries.len(),
                        failed_candidates = failed_entry.candidate_count(),
                        error = %error,
                        "active probe batch rejected one generated outbound"
                    );
                    send_progress(
                        progress,
                        format!(
                            "Batch {batch_index}: sing-box rejected one generated test config; retrying {} remaining definitions",
                            failure.entries.len()
                        ),
                    );
                    send_probe_delta(progress, failed_entry.candidate_count(), 0, 0);
                    lock_shared(shared).ranked.extend(failed_configs(
                        failed_entry,
                        "active_http",
                        &error,
                    ));
                    if !failure.entries.is_empty() {
                        pending.push(failure.entries);
                    }
                } else if failure.retry_split && failure.entries.len() > 1 {
                    stats.splits = stats.splits.saturating_add(1);
                    warn!(
                        entries = failure.entries.len(),
                        error = %failure.error,
                        "active probe batch failed; splitting"
                    );
                    send_progress(
                        progress,
                        format!(
                            "Batch {batch_index}: could not start cleanly; splitting {} server definitions and retrying",
                            failure.entries.len()
                        ),
                    );
                    let mut left = failure.entries;
                    let right = left.split_off(left.len() / 2);
                    pending.push(right);
                    pending.push(left);
                } else {
                    warn!(
                        entries = batch_len,
                        error = %error,
                        "active probe batch failed"
                    );
                    send_progress(
                        progress,
                        format!("Batch {batch_index}: probe failed: {error}"),
                    );
                    let failed_count = candidate_count(&failure.entries);
                    stats.failed_candidates = stats.failed_candidates.saturating_add(failed_count);
                    send_probe_delta(progress, failed_count, 0, 0);
                    lock_shared(shared).ranked.extend(
                        failure
                            .entries
                            .into_iter()
                            .flat_map(|entry| failed_configs(entry, "active_http", &error)),
                    );
                }
            }
        }
    }

    stats
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
// `BatchProbeFailure` carries the batch entries by value so the caller can
// split-retry them; boxing would only add pointer indirection on this cold
// error path for no measurable gain.
#[allow(clippy::result_large_err)]
async fn probe_active_batch(
    batch_index: usize,
    entries: Vec<PreparedActiveCandidate>,
    config: &ProbeConfig,
    progress: Option<&UnboundedSender<ProgressEvent>>,
    stop_policy: &ProbeStopPolicy,
    shared: &SharedProbe,
    stop: ProbeStopSignal,
) -> std::result::Result<usize, BatchProbeFailure> {
    if entries.is_empty() {
        return Ok(0);
    }
    if stop.stopped() {
        return Ok(0);
    }

    let started = Instant::now();
    debug!(
        entries = entries.len(),
        "active probe reserving local ports"
    );
    let entry_candidates = candidate_count(&entries);
    send_progress(
        progress,
        format!("Batch {batch_index}: starting sing-box test for {entry_candidates} configs"),
    );
    let reservations = match reserve_local_ports(entries.len()).await {
        Ok(reservations) => reservations,
        Err(err) => {
            return Err(BatchProbeFailure::unrecoverable(
                entries,
                err.context("unable to reserve local proxy ports"),
            ));
        }
    };
    let ports = reservations
        .iter()
        .map(|reservation| reservation.port)
        .collect::<Vec<_>>();
    debug!(entries = entries.len(), ports = ?ports, "active probe local ports reserved");
    let config_path = match write_sing_box_batch_config(&entries, &ports).await {
        Ok(path) => path,
        Err(err) => {
            return Err(BatchProbeFailure::unrecoverable(
                entries,
                err.context("unable to write sing-box config"),
            ));
        }
    };

    drop(reservations);

    let child = Command::new(&config.sing_box_path)
        .arg("run")
        .arg("-c")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let mut child = match child {
        Ok(child) => {
            debug!(
                entries = entries.len(),
                config_path = %config_path.display(),
                "sing-box batch process spawned"
            );
            child
        }
        Err(err) => {
            let _ = fs::remove_file(config_path).await;
            return Err(BatchProbeFailure::unrecoverable(
                entries,
                anyhow!(err).context(format!(
                    "failed to start sing-box using '{}'",
                    config.sing_box_path
                )),
            ));
        }
    };

    if let Err(err) = wait_for_local_proxies(
        &mut child,
        &ports,
        Duration::from_millis(config.startup_timeout_ms),
    )
    .await
    {
        let stderr = cleanup_sing_box_child(child, config_path).await;
        let err = with_sing_box_stderr(err, &stderr);
        if let Some(index) =
            sing_box_failed_outbound_index(&stderr).filter(|index| *index < entries.len())
        {
            let mut remaining = entries;
            let failed_entry = remaining.remove(index);
            warn!(
                failed_outbound_index = index,
                failed_candidates = failed_entry.candidate_count(),
                remaining = remaining.len(),
                error = %err,
                "sing-box rejected generated outbound"
            );
            return Err(BatchProbeFailure::invalid_entry(
                remaining,
                failed_entry,
                err,
            ));
        }
        warn!(
            entries = entries.len(),
            duration_ms = started.elapsed().as_millis(),
            error = %err,
            "sing-box local proxies were not ready"
        );
        send_progress(
            progress,
            format!("Batch {batch_index}: sing-box test process was not ready: {err}"),
        );
        return Err(BatchProbeFailure::retryable(entries, err));
    }
    if stop.stopped() {
        let stderr = cleanup_sing_box_child(child, config_path).await;
        if !stderr.is_empty() {
            debug!(stderr = %stderr, "sing-box batch stderr after cancellation");
        }
        return Ok(0);
    }
    debug!(
        entries = entries.len(),
        duration_ms = started.elapsed().as_millis(),
        "sing-box local proxies ready"
    );
    let total_entries = entries.len();
    let total_candidates = candidate_count(&entries);
    let http_concurrency = active_probe_http_concurrency(config.concurrency, total_entries);
    let progress_interval = http_concurrency.min(total_entries);
    let mut probe_results = stream::iter(entries.into_iter().zip(ports).map(
        |(entry, port)| async move {
            let result = probe_active_target_inner(port, config).await;
            let bytes = result.as_ref().map_or(0, |ok| ok.bytes);
            (ranked_configs_for_active_result(entry, result), bytes)
        },
    ))
    .buffer_unordered(http_concurrency);
    let mut produced = 0usize;
    let mut working = 0usize;
    let mut completed = 0;
    while let Some((mut results, bytes)) = probe_results.next().await {
        if stop.stopped() {
            break;
        }
        completed += 1;
        let tested_delta = results.len();
        let working_delta = results.iter().filter(|item| item.reachable).count();
        working = working.saturating_add(working_delta);
        produced = produced.saturating_add(tested_delta);
        // Publish to the shared list first: every finished probe counts toward
        // the global stop decision, so all batches quit within ~one probe of
        // the target instead of each batch probing toward `top_n` on its own.
        // Finished work is never lost, no matter which batch outcome the wave
        // loop observes first.
        lock_shared(shared).ranked.append(&mut results);
        let stop_reason = eval_shared_stop(shared, stop_policy, progress);
        send_probe_delta(progress, tested_delta, working_delta, bytes);
        if stop_policy.return_configs_asap {
            let snapshot = lock_shared(shared).ranked.clone();
            send_asap_configs(progress, &snapshot, stop_policy);
        }
        if completed == total_entries || completed % progress_interval == 0 {
            info!(
                completed_probes = completed,
                total_probes = total_entries,
                ranked_candidates = produced,
                total_candidates,
                reachable = working,
                duration_ms = started.elapsed().as_millis(),
                "active probe batch progress"
            );
            send_progress(
                progress,
                format!(
                    "Batch {batch_index}: {produced}/{total_candidates} configs checked, {working} working"
                ),
            );
        }
        if let Some(reason) = stop_reason {
            info!(
                completed_probes = completed,
                total_probes = total_entries,
                ranked_candidates = produced,
                total_candidates,
                reachable = working,
                "active probe batch stopped early"
            );
            send_progress(progress, reason);
            // Internal broadcast only: the caller's external flag stays
            // untouched so a reached target never reports as preempted.
            stop.broadcast_target_reached();
            break;
        }
    }

    let stderr = cleanup_sing_box_child(child, config_path).await;
    if !stderr.is_empty() {
        debug!(stderr = %stderr, "sing-box batch stderr");
    }
    debug!(
        ranked = produced,
        duration_ms = started.elapsed().as_millis(),
        "active probe batch process finished"
    );
    Ok(produced)
}

pub async fn run_with_sing_box_proxy<F, Fut, T>(
    sing_box_path: &str,
    uri: &str,
    startup_timeout: Duration,
    operation: F,
) -> Result<T>
where
    F: FnOnce(u16) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let outbound = sing_box_outbound_from_share_link(uri)
        .context("unable to convert working config into a sing-box retry proxy")?;
    run_with_sing_box_outbound_proxy(sing_box_path, outbound, startup_timeout, operation).await
}

async fn run_with_sing_box_outbound_proxy<F, Fut, T>(
    sing_box_path: &str,
    outbound: Value,
    startup_timeout: Duration,
    operation: F,
) -> Result<T>
where
    F: FnOnce(u16) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let reservations = reserve_local_ports(1)
        .await
        .context("unable to reserve a local proxy port")?;
    let port = reservations
        .first()
        .map(|reservation| reservation.port)
        .ok_or_else(|| anyhow!("no local proxy port was reserved"))?;
    let config_path = write_sing_box_outbound_config(&[outbound], &[port])
        .await
        .context("unable to write sing-box retry proxy config")?;
    drop(reservations);

    let child = Command::new(sing_box_path)
        .arg("run")
        .arg("-c")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_file(config_path).await;
            return Err(anyhow!(err).context(format!(
                "failed to start sing-box retry proxy using '{sing_box_path}'"
            )));
        }
    };

    if let Err(err) = wait_for_local_proxies(&mut child, &[port], startup_timeout).await {
        let stderr = cleanup_sing_box_child(child, config_path).await;
        return Err(with_sing_box_stderr(err, &stderr));
    }

    let result = operation(port).await;
    let stderr = cleanup_sing_box_child(child, config_path).await;
    if !stderr.is_empty() {
        debug!(stderr = %stderr, "sing-box retry proxy stderr");
    }
    result
}

fn ranked_configs_for_active_result(
    entry: PreparedActiveCandidate,
    result: Result<ActiveProbeSuccess>,
) -> Vec<RankedConfig> {
    match result {
        Ok(active) => vec![successful_config(entry.into_candidate(), &active)],
        Err(err) => {
            let error = err.to_string();
            failed_configs(entry, "active_http", &error)
        }
    }
}

fn successful_config(candidate: Candidate, active: &ActiveProbeSuccess) -> RankedConfig {
    // Resolve endpoint host to IP for GeoIP lookup
    let country_code = resolve_endpoint_country(&candidate.endpoint.host);

    RankedConfig {
        rank: 0,
        stability_count: 0,
        id: candidate.id,
        dedup_key: candidate.dedup_key,
        source: candidate.source,
        priority: candidate.priority,
        protocol: candidate.protocol,
        name: candidate.name,
        endpoint: candidate.endpoint,
        uri: candidate.uri,
        reachable: true,
        validation: "active_http".to_string(),
        latency_ms: Some(active.latency_ms),
        http_status: Some(active.http_status),
        download_mbps: active.download_mbps,
        download_bytes: active.download_bytes,
        error: None,
        country_code,
    }
}

/// Resolve a hostname to an IP and look up its country code.
///
/// Tries parsing as IP first, then DNS resolution. Returns the
/// 2-letter ISO country code or `None` if resolution/lookup fails.
fn resolve_endpoint_country(host: &str) -> Option<String> {
    use std::net::ToSocketAddrs;

    let ip = host
        .parse::<std::net::IpAddr>()
        .ok()
        .or_else(|| (host, 0).to_socket_addrs().ok()?.next()?.ip().into())?;

    crate::geoip::lookup_country(ip)
}

async fn probe_active_target_inner(port: u16, config: &ProbeConfig) -> Result<ActiveProbeSuccess> {
    let proxy_url = format!("http://{LOCALHOST_IP}:{port}");
    debug!(port, test_url = %config.test_url, "active HTTP probe started");
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.active_timeout_ms))
        .proxy(Proxy::all(&proxy_url)?);

    if cfg!(target_os = "android")
        && let Some(tls) = crate::FALLBACK_TLS.get()
    {
        builder = builder.tls_backend_preconfigured(tls.clone());
    }

    let client = builder.build()?;

    let started = Instant::now();
    let response = client.get(&config.test_url).send().await?;
    let latency_ms = started.elapsed().as_millis();
    let status = response.status().as_u16();
    let bytes = exchange_bytes(&config.test_url, &response);
    debug!(
        port,
        status, latency_ms, "active HTTP probe response received"
    );
    if !config.accepted_statuses.contains(&status) {
        return Err(anyhow!(
            "active HTTP probe returned status {}; accepted statuses are {:?}",
            status,
            config.accepted_statuses
        ));
    }

    Ok(ActiveProbeSuccess {
        latency_ms,
        http_status: status,
        download_mbps: None,
        download_bytes: None,
        bytes,
    })
}

/// Observed exchange size for Sub Usage accounting: request estimate plus
/// actual response header bytes. The body is never read for this, so no
/// extra traffic or latency is added by counting.
fn exchange_bytes(test_url: &str, response: &reqwest::Response) -> u64 {
    let request = HTTP_EXCHANGE_OVERHEAD_BYTES.saturating_add(test_url.len() as u64);
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len() as u64 + value.as_bytes().len() as u64 + 4)
        .sum::<u64>()
        .saturating_add(64);
    request.saturating_add(headers)
}

async fn enrich_top_speedtests(
    ranked: &mut [RankedConfig],
    config: &ProbeConfig,
    progress: Option<&UnboundedSender<ProgressEvent>>,
    stop_policy: &ProbeStopPolicy,
) {
    let Some(download_url) = config
        .download_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return;
    };

    let mut indices = ranked
        .iter()
        .enumerate()
        .filter(|(_, item)| item.reachable)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if indices.is_empty() {
        return;
    }

    indices.sort_by(|left, right| compare_ranked(&ranked[*left], &ranked[*right]));
    let limit = speedtest_probe_limit(indices.len(), stop_policy);
    indices.truncate(limit);
    send_progress(
        progress,
        format!("Speedtest: measuring {limit} top reachable configs"),
    );

    let concurrency = speedtest_probe_concurrency(config.concurrency, limit);
    let startup_timeout = Duration::from_millis(config.startup_timeout_ms);
    let sing_box_path = config.sing_box_path.clone();
    let bytes_limit = config.download_bytes_limit;
    let mut targets = Vec::with_capacity(indices.len());
    for index in indices {
        targets.push((index, ranked[index].uri.clone()));
    }
    let mut results = stream::iter(targets.into_iter().map(|(index, uri)| {
        let sing_box_path = sing_box_path.clone();
        async move {
            let measurement = measure_download_with_sing_box(
                &sing_box_path,
                &uri,
                startup_timeout,
                download_url,
                bytes_limit,
            )
            .await
            .ok();
            (index, measurement)
        }
    }))
    .buffer_unordered(concurrency);

    while let Some((index, measurement)) = results.next().await {
        if let Some(measurement) = measurement {
            ranked[index].download_mbps = Some(measurement.mbps);
            ranked[index].download_bytes = Some(measurement.bytes);
        }
    }
}

fn speedtest_probe_limit(reachable: usize, stop_policy: &ProbeStopPolicy) -> usize {
    if stop_policy.scan_all_configs {
        return reachable;
    }

    reachable.min(stop_policy.top_n.max(1))
}

fn speedtest_probe_concurrency(configured: usize, targets: usize) -> usize {
    configured.clamp(1, 4).min(targets.max(1))
}

async fn measure_download_with_sing_box(
    sing_box_path: &str,
    uri: &str,
    startup_timeout: Duration,
    download_url: &str,
    bytes_limit: usize,
) -> Result<DownloadMeasurement> {
    run_with_sing_box_proxy(sing_box_path, uri, startup_timeout, |port| async move {
        let proxy_url = format!("http://{LOCALHOST_IP}:{port}");
        let mut builder = reqwest::Client::builder().proxy(Proxy::all(&proxy_url)?);
        if cfg!(target_os = "android")
            && let Some(tls) = crate::FALLBACK_TLS.get()
        {
            builder = builder.tls_backend_preconfigured(tls.clone());
        }
        let client = builder.build()?;
        measure_download(&client, download_url, bytes_limit).await
    })
    .await
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
    let mut stream = client
        .get(download_url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(
            reqwest::header::RANGE,
            format!("bytes=0-{}", bytes_limit.saturating_sub(1)),
        )
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    let mut measured_bytes = 0_usize;
    while measured_bytes < bytes_limit {
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk?;
        let remaining = bytes_limit - measured_bytes;
        measured_bytes = measured_bytes.saturating_add(chunk.len().min(remaining));
    }

    let elapsed = started.elapsed().as_secs_f64();
    if elapsed == 0.0 || measured_bytes == 0 {
        return Err(anyhow!("download probe returned no measurable data"));
    }

    Ok(DownloadMeasurement {
        mbps: (usize_to_f64(measured_bytes) * BITS_PER_BYTE) / elapsed / BITS_PER_MEGABIT,
        bytes: measured_bytes,
    })
}

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(value: usize) -> f64 {
    value as f64
}

async fn reserve_local_ports(count: usize) -> Result<Vec<ReservedLocalPort>> {
    let mut handles = Vec::with_capacity(count);
    for _ in 0..count {
        handles.push(tokio::spawn(async {
            let listener = TcpListener::bind((LOCALHOST_IP, 0)).await?;
            let port = listener.local_addr()?.port();
            Ok::<_, anyhow::Error>(ReservedLocalPort {
                port,
                _listener: listener,
            })
        }));
    }

    let mut reservations = Vec::with_capacity(count);
    for handle in handles {
        reservations.push(handle.await??);
    }

    Ok(reservations)
}

async fn wait_for_local_proxies(
    child: &mut tokio::process::Child,
    ports: &[u16],
    timeout: Duration,
) -> Result<()> {
    let started = Instant::now();
    let mut ready = vec![false; ports.len()];
    debug!(
        ports = ports.len(),
        timeout_ms = timeout.as_millis(),
        "waiting for sing-box local proxies"
    );
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(anyhow!("sing-box exited before proxy was ready: {status}"));
        }

        for (index, port) in ports.iter().enumerate() {
            if !ready[index]
                && tokio::time::timeout(
                    LOCAL_PROXY_CONNECT_TIMEOUT,
                    TcpStream::connect((LOCALHOST_IP, *port)),
                )
                .await
                .is_ok_and(|result| result.is_ok())
            {
                ready[index] = true;
                debug!(
                    port,
                    ready = ready.iter().filter(|is_ready| **is_ready).count(),
                    total = ports.len(),
                    "sing-box local proxy became ready"
                );
            }
        }

        if ready.iter().all(|is_ready| *is_ready) {
            return Ok(());
        }

        if started.elapsed() >= timeout {
            return Err(anyhow!(
                "sing-box local proxy did not become ready within {} ms",
                timeout.as_millis()
            ));
        }

        tokio::time::sleep(LOCAL_PROXY_WAIT_INTERVAL).await;
    }
}

async fn write_sing_box_batch_config(
    entries: &[PreparedActiveCandidate],
    ports: &[u16],
) -> Result<PathBuf> {
    let outbounds = entries
        .iter()
        .map(|entry| entry.outbound.clone())
        .collect::<Vec<_>>();
    write_sing_box_outbound_config(&outbounds, ports).await
}

async fn write_sing_box_outbound_config(outbounds: &[Value], ports: &[u16]) -> Result<PathBuf> {
    if outbounds.len() != ports.len() {
        return Err(anyhow!(
            "sing-box config needs one local port per outbound; got {} outbounds and {} ports",
            outbounds.len(),
            ports.len()
        ));
    }

    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "{SING_BOX_CONFIG_FILE_PREFIX}-{}-{timestamp}.json",
        std::process::id()
    ));
    let mut inbounds = Vec::with_capacity(outbounds.len());
    let mut tagged_outbounds = Vec::with_capacity(outbounds.len());
    let mut rules = Vec::with_capacity(outbounds.len());

    for (index, (outbound, port)) in outbounds.iter().zip(ports.iter()).enumerate() {
        let inbound_tag = format!("{SING_BOX_INBOUND_TAG_PREFIX}-{index}");
        let outbound_tag = format!("{SING_BOX_OUTBOUND_TAG_PREFIX}-{index}");
        let mut outbound = outbound.clone();
        outbound
            .as_object_mut()
            .ok_or_else(|| anyhow!("sing-box outbound is not a JSON object"))?
            .insert("tag".to_string(), json!(outbound_tag));

        inbounds.push(json!({
            "type": "mixed",
            "tag": inbound_tag,
            "listen": LOCALHOST_IP,
            "listen_port": port
        }));
        tagged_outbounds.push(outbound);
        rules.push(json!({
            "inbound": [
                inbound_tag
            ],
            "action": "route",
            "outbound": outbound_tag
        }));
    }

    // Add a direct outbound for DNS queries (DNS must not route through the proxy)
    let direct_outbound = json!({
        "type": "direct",
        "tag": "direct-out"
    });
    tagged_outbounds.push(direct_outbound);

    // Version-aware DNS config: new format for >= 1.12.0, legacy for older
    let dns_servers = if sing_box_version_at_least(1, 12, 0) {
        json!([
            { "tag": "dns-direct", "type": "udp", "server": "8.8.8.8" },
            { "tag": "dns-fallback", "type": "udp", "server": "1.1.1.1" },
            { "tag": "dns-alternative", "type": "udp", "server": "223.5.5.5" }
        ])
    } else {
        json!([
            { "tag": "dns-direct", "address": "8.8.8.8", "strategy": "prefer_ipv4", "detour": "direct-out" },
            { "tag": "dns-fallback", "address": "1.1.1.1", "strategy": "prefer_ipv4", "detour": "direct-out" },
            { "tag": "dns-alternative", "address": "223.5.5.5", "strategy": "prefer_ipv4", "detour": "direct-out" }
        ])
    };

    let config = if sing_box_version_at_least(1, 12, 0) {
        json!({
            "log": {
                "disabled": true
            },
            "dns": {
                "servers": dns_servers
            },
            "inbounds": inbounds,
            "outbounds": tagged_outbounds,
            "route": {
                "rules": rules,
                "final": format!("{SING_BOX_OUTBOUND_TAG_PREFIX}-0"),
                "default_domain_resolver": "dns-direct"
            }
        })
    } else {
        json!({
            "log": {
                "disabled": true
            },
            "dns": {
                "servers": dns_servers
            },
            "inbounds": inbounds,
            "outbounds": tagged_outbounds,
            "route": {
                "rules": rules,
                "final": format!("{SING_BOX_OUTBOUND_TAG_PREFIX}-0")
            }
        })
    };

    fs::write(&path, serde_json::to_vec_pretty(&config)?).await?;
    restrict_file_permissions(&path);
    Ok(path)
}

/// Restrict a sing-box temp config to owner-only (0600 on Unix).
/// No-op on Windows; single syscall, no probing delay.
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

async fn cleanup_sing_box_child(mut child: tokio::process::Child, config_path: PathBuf) -> String {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(SING_BOX_CLEANUP_TIMEOUT, child.wait()).await;
    let stderr: String =
        (tokio::time::timeout(SING_BOX_CLEANUP_TIMEOUT, read_child_stderr(&mut child)).await)
            .unwrap_or_default();
    let _ = fs::remove_file(config_path).await;
    stderr
}

async fn read_child_stderr(child: &mut tokio::process::Child) -> String {
    let mut bytes = Vec::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_end(&mut bytes).await;
    }
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn with_sing_box_stderr(err: anyhow::Error, stderr: &str) -> anyhow::Error {
    if stderr.is_empty() {
        return err;
    }
    anyhow!("{err}; sing-box stderr: {stderr}")
}

fn send_progress(progress: Option<&UnboundedSender<ProgressEvent>>, message: impl Into<String>) {
    if let Some(progress) = progress {
        let _ = progress.send(ProgressEvent::LiveLog(message.into()));
    }
}

fn send_probe_delta(
    progress: Option<&UnboundedSender<ProgressEvent>>,
    tested: usize,
    working: usize,
    bytes: u64,
) {
    if let Some(progress) = progress
        && tested > 0
    {
        let _ = progress.send(ProgressEvent::ProbeDelta {
            tested,
            working,
            bytes,
        });
    }
}

fn format_duration_short(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes}m {seconds}s")
}

fn sing_box_failed_outbound_index(stderr: &str) -> Option<usize> {
    let marker = "outbound[";
    let mut search = stderr;
    while let Some(position) = search.find(marker) {
        let after_marker = &search[position + marker.len()..];
        let end = after_marker.find(']')?;
        let index = &after_marker[..end];
        if !index.is_empty() && index.chars().all(|value| value.is_ascii_digit()) {
            return index.parse().ok();
        }
        search = &after_marker[end + 1..];
    }

    None
}

fn failed_config(candidate: Candidate, validation: &str, error: String) -> RankedConfig {
    RankedConfig {
        rank: 0,
        stability_count: 0,
        id: candidate.id,
        dedup_key: candidate.dedup_key,
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
        country_code: None,
    }
}

fn failed_configs(
    entry: PreparedActiveCandidate,
    validation: &str,
    error: &str,
) -> Vec<RankedConfig> {
    let err = error.to_string();
    let mut configs: Vec<RankedConfig> = entry
        .aliases
        .into_iter()
        .map(|alias| failed_config(alias, validation, err.clone()))
        .collect();
    configs.push(failed_config(entry.candidate, validation, err));
    configs
}

fn compare_ranked(left: &RankedConfig, right: &RankedConfig) -> Ordering {
    right
        .reachable
        .cmp(&left.reachable)
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

pub fn sing_box_outbound_from_share_link(uri: &str) -> Result<Value> {
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
                let flow = supported_vless_flow(&flow)?;
                if !flow.is_empty() {
                    outbound.insert("flow".to_string(), json!(flow));
                }
            }
            if let Some(tls) = tls_config(&params, false, &host)? {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_config(&params)? {
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
            if let Some(tls) = tls_config(&params, true, &host)? {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_config(&params)? {
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
                tls_config(&params, true, &host)?.unwrap_or_else(|| json!({"enabled": true})),
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
                tls_config(&params, true, &host)?.unwrap_or_else(|| json!({"enabled": true})),
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

    let tls_enabled =
        json_string(&json, &["tls"]).is_some_and(|value| value.eq_ignore_ascii_case("tls"));
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
    if let Some(transport) = transport_config(&params)? {
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

    let method = normalize_shadowsocks_method(method)?;
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

fn normalize_shadowsocks_method(method: &str) -> Result<String> {
    let lowered = method.to_ascii_lowercase();
    let normalized = match lowered.as_str() {
        "ss" => return Err(anyhow!("unsupported Shadowsocks method: ss")),
        "chacha20-poly1305" => "chacha20-ietf-poly1305",
        "xchacha20-poly1305" => "xchacha20-ietf-poly1305",
        other => other,
    };
    if crate::constants::SUPPORTED_SS_METHODS.contains(&normalized) {
        Ok(normalized.to_string())
    } else {
        Err(anyhow!(
            "unsupported Shadowsocks cipher: {normalized} (only AEAD/2022 methods are supported)"
        ))
    }
}

fn supported_vless_flow(flow: &str) -> Result<String> {
    let flow = flow.trim();
    if flow.is_empty() || flow.eq_ignore_ascii_case("none") {
        return Ok(String::new());
    }

    match flow.to_ascii_lowercase().as_str() {
        "xtls-rprx-vision" => Ok("xtls-rprx-vision".to_string()),
        unsupported => Err(anyhow!("unsupported VLESS flow: {unsupported}")),
    }
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
) -> Result<Option<Value>> {
    let security = first_param(params, &["security", "tls"]).unwrap_or_default();
    let reality_key = first_param(params, &["pbk", "public_key", "reality_pbk"]);
    let enabled = default_enabled
        || security.eq_ignore_ascii_case("tls")
        || security.eq_ignore_ascii_case("reality")
        || reality_key.is_some();

    if !enabled || security.eq_ignore_ascii_case("none") {
        return Ok(None);
    }

    // Reality: explicit security=reality, OR no security specified but pbk present.
    // When security=tls is explicit, never use Reality (server uses standard TLS).
    let is_reality =
        security.eq_ignore_ascii_case("reality") || (security.is_empty() && reality_key.is_some());

    if is_reality {
        if let Some(public_key) = reality_key.as_deref() {
            validate_reality_public_key(public_key)?;
        }
        let reality_short_id = first_param(params, &["sid", "short_id"]);
        if let Some(short_id) = reality_short_id.as_deref() {
            validate_reality_short_id(short_id)?;
        }

        Ok(Some(tls_config_from_values(
            true,
            first_param(params, &["sni", "serverName", "peer"]).or_else(|| Some(host.to_string())),
            first_param(params, &["alpn"]),
            first_param(params, &["fp", "fingerprint"]),
            reality_key,
            reality_short_id,
            first_param(params, &["allowInsecure", "insecure", "skip-cert-verify"])
                .is_some_and(|value| truthy(&value)),
        )))
    } else {
        Ok(Some(tls_config_from_values(
            true,
            first_param(params, &["sni", "serverName", "peer"]).or_else(|| Some(host.to_string())),
            first_param(params, &["alpn"]),
            first_param(params, &["fp", "fingerprint"]),
            None,
            None,
            first_param(params, &["allowInsecure", "insecure", "skip-cert-verify"])
                .is_some_and(|value| truthy(&value)),
        )))
    }
}

fn validate_reality_public_key(public_key: &str) -> Result<()> {
    let decoded = decode_base64_bytes(public_key)
        .ok_or_else(|| anyhow!("invalid Reality public key: not base64"))?;
    if decoded.len() != 32 {
        return Err(anyhow!(
            "invalid Reality public key: decoded length is {}, expected 32",
            decoded.len()
        ));
    }

    Ok(())
}

fn validate_reality_short_id(short_id: &str) -> Result<()> {
    if short_id.len() > 16 || !short_id.len().is_multiple_of(2) {
        return Err(anyhow!(
            "invalid Reality short_id: expected an even-length hex value up to 16 characters"
        ));
    }
    if !short_id.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(anyhow!("invalid Reality short_id: expected hex"));
    }

    Ok(())
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
    } else if enabled {
        // Default to chrome fingerprint for ALL TLS configs — matches what
        // real clients (v2rayN, Clash Verge) do. Without this, sing-box uses
        // its native Go TLS fingerprint which is distinguishable from real
        // browsers and may be blocked by TLS fingerprint-based firewalls.
        tls.insert(
            "utls".to_string(),
            json!({
                "enabled": true,
                "fingerprint": "chrome"
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

fn transport_config(params: &std::collections::BTreeMap<String, String>) -> Result<Option<Value>> {
    let Some(transport_type) = first_param(params, &["type", "net", "network"]) else {
        return Ok(None);
    };
    let transport_type = transport_type.to_ascii_lowercase();
    let path = first_param(params, &["path"]).unwrap_or_default();
    let host = first_param(params, &["host"]);

    let transport = match transport_type.as_str() {
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
        unsupported => {
            return Err(anyhow!(
                "active sing-box probe does not support transport type '{unsupported}'"
            ));
        }
    };

    Ok(transport)
}

fn query_map(url: &Url) -> std::collections::BTreeMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::TEST_REALITY_PUBLIC_KEY;
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    fn failed_with(error: &str) -> RankedConfig {
        let mut item = ranked("node", "vless://uuid@example.com:443", false, None);
        item.error = Some(error.to_string());
        item
    }

    #[test]
    fn stop_signal_keeps_external_and_internal_separate() {
        // Either flag stops the run, but broadcasting a reached target must
        // never set the caller's external flag: that leak made successful
        // pings report "preempted by refresh" and poisoned later pings.
        let external = Arc::new(AtomicBool::new(false));
        let stop = ProbeStopSignal::from_external(Some(external.clone()));

        assert!(!stop.stopped());
        stop.broadcast_target_reached();
        assert!(stop.stopped());
        assert!(
            !external.load(AtomicOrdering::SeqCst),
            "early-stop broadcast must not touch the external flag"
        );

        external.store(true, AtomicOrdering::SeqCst);
        assert!(stop.stopped());

        let fresh = ProbeStopSignal::from_external(None);
        assert!(!fresh.stopped());
    }

    #[test]
    fn top_preparation_errors_groups_counts_and_caps() {
        let entries = vec![
            failed_with("scheme unsupported"),
            failed_with("no host"),
            failed_with("scheme unsupported"),
            failed_with("no port"),
            failed_with("scheme unsupported"),
            failed_with("no host"),
            ranked("ok", "vless://uuid@example.com:443", true, Some(5)),
        ];
        let top = top_preparation_errors(&entries, 2);
        assert_eq!(
            top,
            vec![
                ("scheme unsupported".to_string(), 3),
                ("no host".to_string(), 2),
            ]
        );
        // Entries without an error never join the ranking.
        let clean = top_preparation_errors(
            &[ranked("ok", "vless://uuid@example.com:443", true, Some(5))],
            3,
        );
        assert!(clean.is_empty());
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
            endpoint: crate::model::Endpoint {
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

    fn stop_policy(
        top_n: usize,
        prioritize_stability: bool,
        previous_working_keys: std::collections::HashSet<String>,
    ) -> ProbeStopPolicy {
        ProbeStopPolicy {
            scan_all_configs: false,
            top_n,
            prioritize_stability,
            return_configs_asap: false,
            previous_working_keys,
            prefound_working: 0,
            prefound_previous_working: 0,
        }
    }

    fn candidate(source: &str, priority: u32, name: &str) -> Candidate {
        Candidate {
            id: name.to_string(),
            dedup_key: format!("vless|example.com|443|ws|tls|{name}"),
            source: source.to_string(),
            priority,
            protocol: "vless".to_string(),
            name: name.to_string(),
            endpoint: crate::model::Endpoint {
                host: "example.com".to_string(),
                port: 443,
            },
            uri: format!(
                "vless://uuid@example.com:443?security=tls&sni=example.com&type=ws&path=/{name}#{name}"
            ),
        }
    }

    fn stop_reason(ranked: &[RankedConfig], policy: &ProbeStopPolicy) -> Option<String> {
        let mut state = ProbeStopState {
            half_snapshot_sent: true,
            stability_search_exhausted: false,
            remaining_previous_working: 0,
        };
        probe_stop_reason(ranked, policy, &mut state, None)
    }

    #[test]
    fn builds_vless_reality_outbound() {
        let outbound = sing_box_outbound_from_share_link(&format!(
            "vless://uuid@example.com:443?security=reality&sni=www.example.com&pbk={TEST_REALITY_PUBLIC_KEY}&sid=abcd&fp=chrome&type=grpc&serviceName=svc#node"
        ))
        .expect("vless outbound");

        assert_eq!(outbound["type"], "vless");
        assert_eq!(outbound["server"], "example.com");
        assert_eq!(
            outbound["tls"]["reality"]["public_key"],
            TEST_REALITY_PUBLIC_KEY
        );
        assert_eq!(outbound["transport"]["type"], "grpc");
    }

    #[test]
    fn builds_vless_reality_outbound_with_default_utls() {
        let outbound = sing_box_outbound_from_share_link(&format!(
            "vless://uuid@example.com:443?security=reality&sni=www.example.com&pbk={TEST_REALITY_PUBLIC_KEY}&sid=abcd&type=grpc&serviceName=svc#node"
        ))
        .expect("vless outbound");

        assert_eq!(
            outbound["tls"]["reality"]["public_key"],
            TEST_REALITY_PUBLIC_KEY
        );
        assert_eq!(outbound["tls"]["utls"]["enabled"], true);
        assert_eq!(outbound["tls"]["utls"]["fingerprint"], "chrome");
    }

    #[test]
    fn rejects_unsupported_transport_before_batching() {
        let err = sing_box_outbound_from_share_link(
            "vless://uuid@example.com:443?security=tls&sni=www.example.com&type=xhttp#node",
        )
        .expect_err("unsupported transport should fail this candidate only");

        assert!(
            err.to_string().contains("transport type 'xhttp'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_reality_public_key_before_batching() {
        let err = sing_box_outbound_from_share_link(
            "vless://uuid@example.com:443?security=reality&sni=www.example.com&pbk=pub&sid=abcd&type=grpc&serviceName=svc#node",
        )
        .expect_err("invalid Reality key should fail this candidate only");

        assert!(
            err.to_string().contains("invalid Reality public key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_reality_short_id_before_batching() {
        let err = sing_box_outbound_from_share_link(&format!(
            "vless://uuid@example.com:443?security=reality&sni=www.example.com&pbk={TEST_REALITY_PUBLIC_KEY}&sid=bad@id&type=grpc&serviceName=svc#node"
        ))
        .expect_err("invalid Reality short_id should fail this candidate only");

        assert!(
            err.to_string().contains("invalid Reality short_id"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unsupported_vless_flow_before_batching() {
        let err = sing_box_outbound_from_share_link(
            "vless://uuid@example.com:443?security=tls&sni=www.example.com&flow=xtls-rprx-vision-udp443#node",
        )
        .expect_err("unsupported flow should fail this candidate only");

        assert!(
            err.to_string().contains("unsupported VLESS flow"),
            "unexpected error: {err}"
        );
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

    #[test]
    fn normalizes_legacy_shadowsocks_method_alias() {
        let outbound = sing_box_outbound_from_share_link(
            "ss://Y2hhY2hhMjAtcG9seTEzMDU6cGFzcw@example.net:8388#SS",
        )
        .expect("shadowsocks outbound");

        assert_eq!(outbound["method"], "chacha20-ietf-poly1305");
    }

    #[test]
    fn rejects_unknown_shadowsocks_method_before_batching() {
        let outbound = sing_box_outbound_from_share_link("ss://c3M6cGFzcw@example.net:8388#SS")
            .expect_err("unknown Shadowsocks method should fail this candidate only");

        assert!(
            outbound
                .to_string()
                .contains("unsupported Shadowsocks method"),
            "unexpected error: {outbound}"
        );
    }

    #[test]
    fn active_probe_batch_size_is_bounded() {
        assert_eq!(
            active_probe_batch_size(1, None),
            ACTIVE_PROBE_BATCH_MIN_SIZE
        );
        assert_eq!(active_probe_batch_size(4, None), 64);
        assert_eq!(
            active_probe_batch_size(usize::MAX, None),
            ACTIVE_PROBE_BATCH_MAX_SIZE
        );
        assert_eq!(active_probe_batch_size(4, Some(10)), 10);
    }

    #[test]
    fn active_probe_http_concurrency_is_bounded() {
        assert_eq!(active_probe_http_concurrency(1, 128), 11);
        assert_eq!(active_probe_http_concurrency(16, 128), 128);
        assert_eq!(active_probe_http_concurrency(16, 20), 20);
        assert_eq!(
            active_probe_http_concurrency(usize::MAX, 256),
            ACTIVE_PROBE_HTTP_MAX_CONCURRENCY
        );
    }

    #[test]
    fn active_batch_sizer_grows_beyond_configured_start_after_clean_batch() {
        let mut sizer = ActiveBatchSizer::new(8, Some(20));
        assert_eq!(sizer.current, 20);

        sizer.observe(&BatchProbeStats {
            started_cleanly: true,
            produced: 20,
            ..BatchProbeStats::default()
        });

        assert_eq!(sizer.current, 40);
    }

    #[test]
    fn active_batch_sizer_shrinks_after_splits() {
        let mut sizer = ActiveBatchSizer::new(8, Some(20));

        sizer.observe(&BatchProbeStats {
            splits: 1,
            ..BatchProbeStats::default()
        });

        assert!(sizer.current < 20);
        assert!(sizer.current >= sizer.min);
    }

    #[test]
    fn active_probe_process_concurrency_is_bounded() {
        assert_eq!(active_probe_process_concurrency(Some(0)), 1);
        assert_eq!(active_probe_process_concurrency(Some(1)), 1);
        assert_eq!(
            active_probe_process_concurrency(Some(usize::MAX)),
            ACTIVE_PROBE_PROCESS_MAX_CONCURRENCY
        );
        assert!(
            active_probe_process_concurrency(None) >= 1
                && active_probe_process_concurrency(None) <= ACTIVE_PROBE_PROCESS_MAX_CONCURRENCY
        );
    }

    #[test]
    fn active_probe_schedule_round_robins_sources_by_priority() {
        let candidates = vec![
            candidate("primary", 1, "primary-1"),
            candidate("primary", 1, "primary-2"),
            candidate("primary", 1, "primary-3"),
            candidate("backup", 2, "backup-1"),
            candidate("backup", 2, "backup-2"),
        ];
        let policy = stop_policy(2, false, std::collections::HashSet::new());

        let scheduled = schedule_active_candidates(candidates, &policy);
        let names = scheduled
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "primary-1",
                "backup-1",
                "primary-2",
                "backup-2",
                "primary-3"
            ]
        );
    }

    #[test]
    fn active_probe_schedule_prioritizes_previous_working_without_starving_sources() {
        let candidates = vec![
            candidate("primary", 1, "primary-new"),
            candidate("primary", 1, "primary-old"),
            candidate("backup", 2, "backup-old"),
            candidate("backup", 2, "backup-new"),
        ];
        let previous_working_keys = std::collections::HashSet::from([
            candidates[1].dedup_key.clone(),
            candidates[2].dedup_key.clone(),
        ]);
        let policy = stop_policy(2, true, previous_working_keys);

        let scheduled = schedule_active_candidates(candidates, &policy);
        let names = scheduled
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            ["primary-old", "backup-old", "primary-new", "backup-new"]
        );
    }

    #[tokio::test]
    async fn asap_event_publishes_working_configs_in_found_order() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut policy = stop_policy(1, false, std::collections::HashSet::new());
        policy.return_configs_asap = true;
        let ranked = vec![
            ranked("one", "vless://one@example.com:443", true, Some(20)),
            ranked("two", "vless://two@example.com:443", true, Some(10)),
        ];

        send_asap_configs(Some(&tx), &ranked, &policy);

        let Some(ProgressEvent::WorkingConfigsFound { configs, top_n }) = rx.recv().await else {
            panic!("expected working configs event");
        };
        assert_eq!(top_n, 1);
        assert_eq!(
            configs
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
    }

    #[tokio::test]
    async fn asap_event_is_disabled_by_default() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = stop_policy(1, false, std::collections::HashSet::new());

        send_asap_configs(
            Some(&tx),
            &[ranked("one", "vless://one@example.com:443", true, Some(20))],
            &policy,
        );

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn extracts_sing_box_failed_outbound_index() {
        let stderr = "FATAL[0000] create service: initialize outbound[7]: unknown method";

        assert_eq!(sing_box_failed_outbound_index(stderr), Some(7));
        assert_eq!(sing_box_failed_outbound_index("no outbound marker"), None);
    }

    #[test]
    fn active_preparation_deduplicates_equivalent_outbounds() {
        let candidates = vec![
            Candidate {
                id: "one".to_string(),
                dedup_key: "vless|example.com|443|ws|tls".to_string(),
                source: "test".to_string(),
                priority: 1,
                protocol: "vless".to_string(),
                name: "one".to_string(),
                endpoint: crate::model::Endpoint {
                    host: "example.com".to_string(),
                    port: 443,
                },
                uri:
                    "vless://uuid@example.com:443?security=tls&sni=example.com&type=ws&path=/ws#one"
                        .to_string(),
            },
            Candidate {
                id: "two".to_string(),
                dedup_key: "vless|example.com|443|ws|tls".to_string(),
                source: "test".to_string(),
                priority: 1,
                protocol: "vless".to_string(),
                name: "two".to_string(),
                endpoint: crate::model::Endpoint {
                    host: "example.com".to_string(),
                    port: 443,
                },
                uri:
                    "vless://uuid@example.com:443?security=tls&sni=example.com&type=ws&path=/ws#two"
                        .to_string(),
            },
        ];

        let policy = stop_policy(2, false, std::collections::HashSet::new());
        let prepared = prepare_active_candidates(candidates, &policy);

        assert_eq!(prepared.prepared.len(), 1);
        assert_eq!(prepared.prepared_candidates, 2);
        assert_eq!(prepared.prepared[0].aliases.len(), 1);
    }

    #[test]
    fn early_stop_without_stability_waits_for_top_n_working() {
        let policy = stop_policy(2, false, std::collections::HashSet::new());
        let ranked = vec![
            ranked("one", "vless://one@example.com:443", true, Some(10)),
            ranked("two", "vless://two@example.com:443", true, Some(20)),
        ];

        assert!(stop_reason(&ranked, &policy).is_some());
    }

    #[test]
    fn early_stop_with_stability_requires_previous_run_half() {
        let policy = stop_policy(
            4,
            true,
            std::collections::HashSet::from([
                "vless://old-1@example.com:443".to_string(),
                "vless://old-2@example.com:443".to_string(),
            ]),
        );
        let ranked = vec![
            ranked("new-1", "vless://new-1@example.com:443", true, Some(10)),
            ranked("new-2", "vless://new-2@example.com:443", true, Some(20)),
            ranked("old-1", "vless://old-1@example.com:443", true, Some(30)),
            ranked("old-2", "vless://old-2@example.com:443", true, Some(40)),
        ];

        assert!(stop_reason(&ranked, &policy).is_some());
    }

    #[test]
    fn early_stop_with_stability_does_not_block_without_previous_run_matches() {
        let policy = stop_policy(2, true, std::collections::HashSet::new());
        let ranked = vec![
            ranked("new-1", "vless://new-1@example.com:443", true, Some(10)),
            ranked("new-2", "vless://new-2@example.com:443", true, Some(20)),
        ];

        assert!(stop_reason(&ranked, &policy).is_some());
    }

    #[test]
    fn early_stop_with_stability_keeps_scanning_for_missing_previous_run_match() {
        let policy = stop_policy(
            4,
            true,
            std::collections::HashSet::from([
                "vless://old-1@example.com:443".to_string(),
                "vless://old-2@example.com:443".to_string(),
            ]),
        );
        let ranked = vec![
            ranked("new-1", "vless://new-1@example.com:443", true, Some(10)),
            ranked("new-2", "vless://new-2@example.com:443", true, Some(20)),
            ranked("new-3", "vless://new-3@example.com:443", true, Some(30)),
            ranked("old-1", "vless://old-1@example.com:443", true, Some(40)),
        ];

        assert!(stop_reason(&ranked, &policy).is_none());
    }

    #[tokio::test]
    async fn writes_batched_sing_box_config_with_one_route_per_entry() {
        let entries = vec![
            PreparedActiveCandidate {
                candidate: Candidate {
                    id: "one".to_string(),
                    dedup_key: "ss|one.example|8388|tcp|none".to_string(),
                    source: "test".to_string(),
                    priority: 1,
                    protocol: "ss".to_string(),
                    name: "one".to_string(),
                    endpoint: crate::model::Endpoint {
                        host: "one.example".to_string(),
                        port: 8388,
                    },
                    uri: "ss://one".to_string(),
                },
                aliases: Vec::new(),
                outbound: json!({
                    "type": "direct",
                    "tag": "will-be-replaced"
                }),
            },
            PreparedActiveCandidate {
                candidate: Candidate {
                    id: "two".to_string(),
                    dedup_key: "ss|two.example|8388|tcp|none".to_string(),
                    source: "test".to_string(),
                    priority: 1,
                    protocol: "ss".to_string(),
                    name: "two".to_string(),
                    endpoint: crate::model::Endpoint {
                        host: "two.example".to_string(),
                        port: 8388,
                    },
                    uri: "ss://two".to_string(),
                },
                aliases: Vec::new(),
                outbound: json!({
                    "type": "direct"
                }),
            },
        ];

        let path = write_sing_box_batch_config(&entries, &[12_001, 12_002])
            .await
            .expect("batch config writes");
        let bytes = fs::read(&path).await.expect("batch config can be read");
        fs::remove_file(&path)
            .await
            .expect("batch config can be removed");
        let config: Value = serde_json::from_slice(&bytes).expect("batch config is JSON");

        assert_eq!(config["inbounds"].as_array().expect("inbounds").len(), 2);
        assert_eq!(config["outbounds"][0]["tag"], "proxy-0");
        assert_eq!(config["outbounds"][1]["tag"], "proxy-1");
        assert_eq!(config["route"]["rules"].as_array().expect("rules").len(), 2);
        assert_eq!(config["route"]["rules"][0]["outbound"], "proxy-0");
        assert_eq!(
            config["route"]["rules"][1]["inbound"][0],
            format!("{SING_BOX_INBOUND_TAG_PREFIX}-1")
        );
    }

    const FAKE_SING_BOX_SOURCE: &str = include_str!("../tests/helpers/fake_singbox.rs");

    /// Serializes the live-stub end-to-end tests: each reserves ~80 ephemeral
    /// ports, releases them, and rebinds them from a child process. Two such
    /// tests racing can hand each other's just-released ports to the other
    /// test's reservations, so the stub fails to bind and every probe fails.
    fn stub_e2e_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    fn healthy_candidate(base: usize, port: u16, remark: &str) -> Candidate {
        let uri = format!(
            "vless://u{base}@127.0.0.1:{port}?security=tls&sni=example.com&type=ws&path=/ws#{remark}"
        );
        Candidate {
            id: uri.clone(),
            dedup_key: format!("vless|127.0.0.1|{port}|ws|tls|u{base}"),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: remark.to_string(),
            endpoint: crate::model::Endpoint {
                host: "127.0.0.1".to_string(),
                port,
            },
            uri,
        }
    }

    fn compile_fake_sing_box() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-fake-singbox-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        let source = dir.join("fake_singbox.rs");
        std::fs::write(&source, FAKE_SING_BOX_SOURCE).expect("stub source can be written");
        let binary = dir.join(format!("fake-singbox{}", std::env::consts::EXE_SUFFIX));
        let output = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg("-O")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("rustc runs (required for hermetic probe tests)");
        assert!(
            output.status.success(),
            "stub compiles: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        binary
    }

    fn active_probe_test_config(sing_box_path: &std::path::Path) -> ProbeConfig {
        ProbeConfig {
            mode: ProbeMode::Active,
            sing_box_path: sing_box_path.to_string_lossy().to_string(),
            connect_timeout_ms: 2000,
            active_timeout_ms: 5000,
            startup_timeout_ms: 10_000,
            concurrency: 16,
            batch_size: Some(20),
            process_concurrency: Some(4),
            test_url: "http://probe.test/".to_string(),
            accepted_statuses: vec![204],
            download_url: None,
            download_bytes_limit: 1_048_576,
            sing_box_path_auto: false,
        }
    }

    /// 200 unique healthy outbounds against the fake sing-box.
    fn two_hundred_healthy_candidates() -> Vec<Candidate> {
        let mut candidates = Vec::with_capacity(200);
        for i in 0..200u16 {
            candidates.push(healthy_candidate(
                usize::from(i),
                20_000 + i,
                &format!("node-{i}"),
            ));
        }
        assert_eq!(candidates.len(), 200);
        candidates
    }

    fn working_count(ranked: &[RankedConfig]) -> usize {
        ranked.iter().filter(|item| item.reachable).count()
    }

    /// End-to-end early stop against a live fake sing-box: four concurrent
    /// batches race, and the run must land on the target instead of each
    /// batch probing toward `top_n` on its own (or dropping results).
    /// The bound is deliberately loose against CI timing variance; exactness
    /// is pinned by the deterministic `shared_stop_*` unit tests below.
    #[tokio::test]
    async fn active_probe_stops_near_top_n_under_full_concurrency() {
        let _e2e_guard = stub_e2e_lock().lock().await;
        let stub = compile_fake_sing_box();
        let dir = stub.parent().expect("stub has a parent").to_path_buf();
        let config = active_probe_test_config(&stub);

        let policy = stop_policy(20, false, std::collections::HashSet::new());
        let ranked = probe_candidates(
            two_hundred_healthy_candidates(),
            &config,
            None,
            &policy,
            None,
        )
        .await;
        assert!(
            working_count(&ranked) >= 20,
            "must find top_n working, got {}",
            working_count(&ranked)
        );
        assert!(
            ranked.len() <= 40,
            "tested {} configs for top_n=20",
            ranked.len()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn active_probe_stops_near_top_n_with_stability() {
        let _e2e_guard = stub_e2e_lock().lock().await;
        let stub = compile_fake_sing_box();
        let dir = stub.parent().expect("stub has a parent").to_path_buf();
        let config = active_probe_test_config(&stub);

        let candidates = two_hundred_healthy_candidates();
        let previous: std::collections::HashSet<String> = candidates
            .iter()
            .take(20)
            .map(|item| item.dedup_key.clone())
            .collect();
        let policy = stop_policy(20, true, previous);
        let ranked = probe_candidates(candidates, &config, None, &policy, None).await;
        assert!(
            working_count(&ranked) >= 20,
            "must find top_n working, got {}",
            working_count(&ranked)
        );
        // Looser than the simple run: confirming the required previous-run
        // configs can lag one wave behind when batches complete in lockstep,
        // so more fresh configs get probed meanwhile by design.
        assert!(
            ranked.len() <= 60,
            "tested {} configs for top_n=20",
            ranked.len()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn shared_probe(
        top_n: usize,
        prioritize_stability: bool,
        previous: std::collections::HashSet<String>,
    ) -> (SharedProbe, ProbeStopPolicy) {
        let policy = stop_policy(top_n, prioritize_stability, previous);
        let shared: SharedProbe = Arc::new(Mutex::new(SharedProbeState {
            ranked: Vec::new(),
            stop: ProbeStopState {
                half_snapshot_sent: true,
                stability_search_exhausted: false,
                remaining_previous_working: 0,
            },
            stop_announced: false,
        }));
        (shared, policy)
    }

    /// Append one working result and evaluate; returns the stop reason.
    fn record_working(shared: &SharedProbe, policy: &ProbeStopPolicy, key: &str) -> Option<String> {
        lock_shared(shared)
            .ranked
            .push(ranked(key, key, true, Some(10)));
        eval_shared_stop(shared, policy, None)
    }

    #[test]
    fn shared_stop_fires_exactly_at_top_n_working() {
        let (shared, policy) = shared_probe(3, false, std::collections::HashSet::new());

        assert!(record_working(&shared, &policy, "a").is_none());
        assert!(record_working(&shared, &policy, "b").is_none());
        let reason = record_working(&shared, &policy, "c");
        assert!(
            reason
                .as_deref()
                .is_some_and(|message| message.contains("3/3")),
            "stop must fire at top_n, got {reason:?}"
        );
        // Announced once: further results never re-fire.
        assert!(record_working(&shared, &policy, "d").is_none());

        // Unreachable results never advance the stop condition.
        let (idle, idle_policy) = shared_probe(1, false, std::collections::HashSet::new());
        lock_shared(&idle)
            .ranked
            .push(ranked("x", "x", false, None));
        assert!(eval_shared_stop(&idle, &idle_policy, None).is_none());
    }

    #[test]
    fn shared_stop_never_fires_when_scanning_all() {
        let mut policy = stop_policy(2, false, std::collections::HashSet::new());
        policy.scan_all_configs = true;
        let shared: SharedProbe = Arc::new(Mutex::new(SharedProbeState {
            ranked: vec![
                ranked("a", "a", true, Some(10)),
                ranked("b", "b", true, Some(10)),
                ranked("c", "c", true, Some(10)),
            ],
            stop: ProbeStopState {
                half_snapshot_sent: true,
                stability_search_exhausted: false,
                remaining_previous_working: 0,
            },
            stop_announced: false,
        }));

        assert!(eval_shared_stop(&shared, &policy, None).is_none());
    }

    #[test]
    fn shared_stop_requires_previous_working_under_stability() {
        let previous: std::collections::HashSet<String> =
            ["old-1", "old-2"].into_iter().map(str::to_string).collect();
        let (shared, policy) = shared_probe(4, true, previous);

        for key in ["new-1", "new-2", "new-3", "new-4"] {
            assert!(
                record_working(&shared, &policy, key).is_none(),
                "must wait for previous-run configs"
            );
        }
        assert!(record_working(&shared, &policy, "old-1").is_none());
        assert!(record_working(&shared, &policy, "old-2").is_some());
    }

    #[test]
    fn shared_stop_fires_on_exhausted_previous_search() {
        let previous: std::collections::HashSet<String> =
            std::iter::once("gone").map(str::to_string).collect();
        let (shared, policy) = shared_probe(2, true, previous);
        lock_shared(&shared).stop.stability_search_exhausted = true;

        assert!(record_working(&shared, &policy, "new-1").is_none());
        assert!(record_working(&shared, &policy, "new-2").is_some());
    }

    #[test]
    fn shared_stop_counts_prefound_working_toward_top_n() {
        // A refresh that preempted a ping with 2 working already verified
        // only needs 1 fresh working config to reach top_n=3.
        let (shared, mut policy) = shared_probe(3, false, std::collections::HashSet::new());
        policy.prefound_working = 2;
        assert!(record_working(&shared, &policy, "a").is_some());
    }

    #[test]
    fn shared_stop_counts_prefound_previous_toward_stability_quorum() {
        // top_n=4 needs 2 previous-run configs; the preempted ping already
        // verified old-1, so one fresh old-2 completes the quorum at 4 total.
        let previous: std::collections::HashSet<String> =
            ["old-1", "old-2"].into_iter().map(str::to_string).collect();
        let (shared, mut policy) = shared_probe(4, true, previous);
        policy.prefound_working = 2;
        policy.prefound_previous_working = 1;
        assert!(record_working(&shared, &policy, "new-1").is_none());
        assert!(record_working(&shared, &policy, "old-2").is_some());
    }

    #[test]
    fn shared_stop_snapshot_never_fires_on_prefound_alone() {
        // Carried-over results count toward the threshold but must never
        // publish an empty fresh list over the live one.
        let (shared, mut policy) = shared_probe(4, true, std::collections::HashSet::new());
        lock_shared(&shared).stop.half_snapshot_sent = false;
        policy.prefound_working = 2;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(eval_shared_stop(&shared, &policy, Some(&tx)).is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn shared_stop_sends_half_snapshot_once() {
        // Snapshot path lives in stability mode; empty previous keys keep the
        // stop rule itself equivalent to a plain count.
        let (shared, policy) = shared_probe(4, true, std::collections::HashSet::new());
        // Re-enable the snapshot path (other tests bypass it).
        lock_shared(&shared).stop.half_snapshot_sent = false;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        for key in ["a", "b"] {
            lock_shared(&shared)
                .ranked
                .push(ranked(key, key, true, Some(10)));
        }
        let progress = Some(&tx);
        assert!(eval_shared_stop(&shared, &policy, progress).is_none());
        assert!(
            matches!(rx.try_recv(), Ok(ProgressEvent::RankedSnapshot(_))),
            "half snapshot must publish once new configs reach top_n/2"
        );

        for key in ["c", "d", "e"] {
            lock_shared(&shared)
                .ranked
                .push(ranked(key, key, true, Some(10)));
        }
        // Stop fires here (4/4); no second snapshot may follow (the live
        // log line the half path also emits is expected and drained here).
        assert!(eval_shared_stop(&shared, &policy, progress).is_some());
        assert!(matches!(rx.try_recv(), Ok(ProgressEvent::LiveLog(_))));
        assert!(rx.try_recv().is_err());
    }
}
