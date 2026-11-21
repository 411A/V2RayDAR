use std::{collections::HashSet, future::Future, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use percent_encoding::percent_decode_str;
use reqwest::Client;
use reqwest::Proxy;
use tokio::{fs, sync::mpsc::UnboundedSender, time::sleep};
use tracing::{debug, info, warn};

use crate::{
    applog::{self, LogLevel},
    config::{AppConfig, SubscriptionSource, redact_subscription_url},
    constants::{
        FETCH_RETRY_ATTEMPTS, FETCH_RETRY_BASE_DELAY_MS, HTTP_EXCHANGE_OVERHEAD_BYTES,
        LOCALHOST_IP, SUBSCRIPTION_SOFT_WAIT,
    },
    model::{Candidate, ProgressEvent},
    parser::parse_subscription_document,
    probe::run_with_sing_box_proxy,
};

#[derive(Debug)]
pub struct FetchOutcome {
    pub candidates: Vec<Candidate>,
    pub errors: Vec<String>,
    pub failures: Vec<FetchFailure>,
    pub successes: Vec<SubscriptionSource>,
}

#[derive(Debug, Clone)]
pub struct FetchFailure {
    pub source: SubscriptionSource,
    pub error: String,
    /// The failure looks egress-scoped (blocked connect/DNS, timeout,
    /// 403/429), so fetching through a working proxy may succeed where
    /// the direct path failed. Decides the proxy-retry gate in `main`.
    pub proxy_retry: bool,
}

/// The sources a proxy retry can actually help with.
///
/// Only the failures classified as egress-scoped qualify. A 404 or a sick
/// origin fails identically through a tunnel, and retrying it spins up a
/// sing-box instance for nothing. Local files are not fetched over HTTP
/// either.
fn proxy_retry_sources(failures: &[FetchFailure]) -> Vec<SubscriptionSource> {
    failures
        .iter()
        .filter(|failure| failure.proxy_retry)
        .map(|failure| failure.source.clone())
        .filter(|source| is_http_url(&source.url))
        .collect()
}

pub async fn load_candidates_with_cache<F, Fut>(
    config: &AppConfig,
    report_bytes: &mut F,
    progress: Option<UnboundedSender<ProgressEvent>>,
) -> Result<FetchStage>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    if !config.subscriptions.iter().any(|source| source.enabled) {
        info!(
            configured = config.subscriptions.len(),
            enabled = 0,
            "subscription load skipped because no subscriptions are enabled"
        );
        return Ok(FetchStage {
            outcome: FetchOutcome {
                candidates: Vec::new(),
                errors: Vec::new(),
                failures: Vec::new(),
                successes: Vec::new(),
            },
            pending: None,
        });
    }

    let client =
        build_http_client(config.fetch_timeout_ms).context("failed to build HTTP client")?;

    let sources = config
        .subscriptions
        .iter()
        .filter(|source| source.enabled)
        .cloned()
        .collect::<Vec<_>>();
    info!(
        sources = sources.len(),
        fetch_concurrency = config.fetch_concurrency,
        timeout_ms = config.fetch_timeout_ms,
        max_bytes = config.max_subscription_bytes,
        cache_only = config.use_cache_only,
        require_transport_security = config.probe.require_transport_security,
        "subscription fetch queue built"
    );
    send_progress(
        progress.as_ref(),
        LogLevel::Info,
        format!(
            "Subscription load: fetching {} enabled sources",
            sources.len()
        ),
    );

    let stage = fetch_sources_streamed(
        client,
        sources,
        FetchContext {
            max_bytes: config.max_subscription_bytes,
            concurrency: config.fetch_concurrency,
            progress,
        },
        report_bytes,
        Some(SUBSCRIPTION_SOFT_WAIT),
    )
    .await;

    Ok(stage)
}

pub async fn retry_failed_sources_with_proxy<F, Fut>(
    config: &AppConfig,
    failures: &[FetchFailure],
    proxy_uri: &str,
    report_bytes: F,
    progress: Option<UnboundedSender<ProgressEvent>>,
) -> Result<FetchOutcome>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    let sources = proxy_retry_sources(failures);

    if sources.is_empty() {
        return Ok(FetchOutcome {
            candidates: Vec::new(),
            errors: Vec::new(),
            failures: Vec::new(),
            successes: Vec::new(),
        });
    }

    info!(
        sources = sources.len(),
        "retrying failed subscription fetches through sing-box proxy"
    );
    send_progress(
        progress.as_ref(),
        LogLevel::Info,
        format!(
            "Subscription retry: fetching {} failed sources through sing-box proxy",
            sources.len()
        ),
    );

    let fetch_timeout_ms = config.fetch_timeout_ms;
    let max_bytes = config.max_subscription_bytes;
    let fetch_concurrency = config.fetch_concurrency;
    let startup_timeout = Duration::from_millis(config.probe.startup_timeout_ms);
    let sing_box_path = config.probe.sing_box_path.clone();
    let progress_for_proxy = progress.clone();

    let outcome =
        run_with_sing_box_proxy(&sing_box_path, proxy_uri, startup_timeout, move |port| {
            let mut report_bytes = report_bytes;
            let progress = progress_for_proxy.clone();
            let sources = sources.clone();
            async move {
                let client = build_proxied_http_client(fetch_timeout_ms, port)?;
                Ok(fetch_sources_with_client(
                    client,
                    sources,
                    FetchContext {
                        max_bytes,
                        concurrency: fetch_concurrency,
                        progress,
                    },
                    &mut report_bytes,
                )
                .await)
            }
        })
        .await?;
    Ok(outcome)
}

#[derive(Clone)]
struct FetchContext {
    max_bytes: usize,
    concurrency: usize,
    progress: Option<UnboundedSender<ProgressEvent>>,
}

/// One-line Live log for a finished source fetch (info): config counts,
/// bytes and elapsed — everything about the source on one line.
fn success_line(
    source: &SubscriptionSource,
    parsed: usize,
    new_unique: usize,
    bytes_read: u64,
    elapsed: Duration,
) -> String {
    format!(
        "Sub '{}' prio {}: OK, {parsed} configs ({new_unique} new), {} in {}",
        source.name,
        source.priority,
        applog::format_bytes(bytes_read),
        applog::format_elapsed(elapsed),
    )
}

/// One-line Live log for a failed source fetch (warn): elapsed, bytes
/// salvaged and the flattened headline — the URL stays redacted.
fn failure_line(
    source: &SubscriptionSource,
    message: &str,
    bytes_read: u64,
    elapsed: Duration,
) -> String {
    format!(
        "Sub '{}' prio {}: FAILED in {}, {} read — {message}",
        source.name,
        source.priority,
        applog::format_elapsed(elapsed),
        applog::format_bytes(bytes_read),
    )
}

/// A fetch run that may still have sources in flight. `pending` is `Some`
/// when the probe was allowed to start before every source finished.
pub struct FetchStage {
    pub outcome: FetchOutcome,
    pub pending: Option<PendingFetches>,
}

/// Sources still retrying when the cycle moved on. Their results remain
/// available through `collect`; dropping this handle aborts them (the target
/// was already met, so the remaining URLs are moot).
pub struct PendingFetches {
    rx: tokio::sync::mpsc::UnboundedReceiver<SourceFetch>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    seen_keys: HashSet<String>,
    unique_count: usize,
    progress: Option<UnboundedSender<ProgressEvent>>,
    remaining: usize,
}

type SourceFetch = (
    usize,
    SubscriptionSource,
    FetchResult<FetchedSource>,
    Duration,
);

impl PendingFetches {
    /// Sources still in flight at the time of the call.
    pub const fn remaining(&self) -> usize {
        self.remaining
    }

    /// Drain the stragglers for at most `wait`. A broken origin cannot hold
    /// the cycle hostage: anything past the window is abandoned and logged.
    pub async fn collect<F, Fut>(mut self, wait: Duration, report_bytes: &mut F) -> FetchOutcome
    where
        F: FnMut(u64) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut accumulator = FetchAccumulator::new(
            self.progress.take(),
            std::mem::take(&mut self.seen_keys),
            self.unique_count,
        );
        let deadline = (!wait.is_zero()).then(|| tokio::time::Instant::now() + wait);
        while self.remaining > 0 {
            let received = if let Some(at) = deadline {
                if let Ok(received) = tokio::time::timeout_at(at, self.rx.recv()).await {
                    received
                } else {
                    warn!(
                        abandoned = self.remaining,
                        "subscription fetch drain timed out; abandoning remaining sources"
                    );
                    break;
                }
            } else {
                self.rx.recv().await
            };
            let Some(item) = received else {
                break;
            };
            self.remaining = self.remaining.saturating_sub(1);
            accumulator.absorb(item, report_bytes).await;
        }
        accumulator.finish().0
    }
}

impl Drop for PendingFetches {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Per-source results folded into one outcome. Shared by the first phase and
/// the late drain so logging, dedupe counters, and byte accounting match.
struct FetchAccumulator {
    fetched: Vec<(usize, Vec<Candidate>)>,
    errors: Vec<String>,
    failures: Vec<(usize, FetchFailure)>,
    successes: Vec<SubscriptionSource>,
    seen_keys: HashSet<String>,
    unique_count: usize,
    progress: Option<UnboundedSender<ProgressEvent>>,
}

impl FetchAccumulator {
    const fn new(
        progress: Option<UnboundedSender<ProgressEvent>>,
        seen_keys: HashSet<String>,
        unique_count: usize,
    ) -> Self {
        Self {
            fetched: Vec::new(),
            errors: Vec::new(),
            failures: Vec::new(),
            successes: Vec::new(),
            seen_keys,
            unique_count,
            progress,
        }
    }

    async fn absorb<F, Fut>(
        &mut self,
        (index, source, result, elapsed): SourceFetch,
        report_bytes: &mut F,
    ) where
        F: FnMut(u64) -> Fut,
        Fut: Future<Output = ()>,
    {
        match result {
            Ok(fetched) => {
                report_subscription_bytes(fetched.bytes_read, report_bytes).await;
                let parsed = fetched.candidates.len();
                let new_unique = fetched
                    .candidates
                    .iter()
                    .filter(|c| self.seen_keys.insert(c.dedup_key.clone()))
                    .count();
                self.unique_count = self.unique_count.saturating_add(new_unique);
                info!(
                    source = %source.name,
                    parsed,
                    new_unique,
                    unique_total = self.unique_count,
                    bytes_read = fetched.bytes_read,
                    duration_ms = elapsed.as_millis(),
                    "subscription fetch result parsed"
                );
                send_progress(
                    self.progress.as_ref(),
                    LogLevel::Info,
                    success_line(&source, parsed, new_unique, fetched.bytes_read, elapsed),
                );
                send_fetched_delta(self.progress.as_ref(), self.unique_count);
                self.successes.push(source);
                self.fetched.push((index, fetched.candidates));
            }
            Err(err) => {
                report_subscription_bytes(err.bytes_read, report_bytes).await;
                let message = fetch_error_message(&source.name, &err.error, &source.url);
                warn!(
                    source = %source.name,
                    bytes_read = err.bytes_read,
                    attempts = err.attempts,
                    duration_ms = elapsed.as_millis(),
                    proxy_retry = proxy_may_help(&err.error),
                    error = %message,
                    "subscription fetch failed"
                );
                send_progress(
                    self.progress.as_ref(),
                    LogLevel::Warn,
                    failure_line(&source, &message, err.bytes_read, elapsed),
                );
                self.errors.push(message.clone());
                self.failures.push((
                    index,
                    FetchFailure {
                        source,
                        error: message,
                        proxy_retry: proxy_may_help(&err.error),
                    },
                ));
            }
        }
    }

    fn finish(self) -> (FetchOutcome, HashSet<String>, usize) {
        let Self {
            mut fetched,
            errors,
            mut failures,
            successes,
            seen_keys,
            unique_count,
            ..
        } = self;
        fetched.sort_by_key(|(index, _)| *index);
        let mut dedup_keys = HashSet::new();
        let mut candidates = Vec::new();
        for (_, mut source_candidates) in fetched {
            source_candidates.retain(|candidate| dedup_keys.insert(candidate.dedup_key.clone()));
            candidates.append(&mut source_candidates);
        }
        failures.sort_by_key(|(index, _)| *index);
        let failures = failures
            .into_iter()
            .map(|(_, failure)| failure)
            .collect::<Vec<_>>();
        (
            FetchOutcome {
                candidates,
                errors,
                failures,
                successes,
            },
            seen_keys,
            unique_count,
        )
    }
}

/// Proxy retries and tests want the final tally: never return early here.
async fn fetch_sources_with_client<F, Fut>(
    client: Client,
    sources: Vec<SubscriptionSource>,
    context: FetchContext,
    report_bytes: &mut F,
) -> FetchOutcome
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    fetch_sources_streamed(client, sources, context, report_bytes, None)
        .await
        .outcome
}

/// Start every source at once and fold results in as they arrive. With a soft
/// wait the caller gets the fast sources immediately plus a handle for the
/// stragglers, instead of blocking on the slowest retry loop.
async fn fetch_sources_streamed<F, Fut>(
    client: Client,
    sources: Vec<SubscriptionSource>,
    context: FetchContext,
    report_bytes: &mut F,
    soft_wait: Option<Duration>,
) -> FetchStage
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    let total = sources.len();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SourceFetch>();
    // Keep the configured parallelism: every source is queued, only the
    // permit holders actually fetch, and the timer starts after the wait
    // (the old buffer_unordered behaviour).
    let limit = Arc::new(tokio::sync::Semaphore::new(context.concurrency.max(1)));
    let tasks = sources
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let client = client.clone();
            let progress = context.progress.clone();
            let max_bytes = context.max_bytes;
            let tx = tx.clone();
            let limit = Arc::clone(&limit);
            tokio::spawn(async move {
                let permit = limit
                    .acquire_owned()
                    .await
                    .expect("fetch semaphore is never closed");
                let started = std::time::Instant::now();
                let result = fetch_source(&client, source.clone(), max_bytes, progress).await;
                let _ = tx.send((index, source, result, started.elapsed()));
                drop(permit);
            })
        })
        .collect::<Vec<_>>();
    drop(tx);

    let mut accumulator = FetchAccumulator::new(context.progress.clone(), HashSet::new(), 0);
    let mut done = 0usize;
    let mut deadline = soft_wait.map(|wait| tokio::time::Instant::now() + wait);
    while done < total {
        let received = if let Some(at) = deadline {
            if let Ok(received) = tokio::time::timeout_at(at, rx.recv()).await {
                received
            } else {
                // Nothing to probe and nothing to bridge yet: keep waiting
                // instead of starting an empty cycle. A completed failure is
                // enough material for the proxy retry, so a restricted
                // network can start that while the rest still retries.
                if accumulator.fetched.is_empty() && accumulator.failures.is_empty() {
                    deadline = None;
                    continue;
                }
                break;
            }
        } else {
            rx.recv().await
        };
        let Some(item) = received else {
            break;
        };
        done = done.saturating_add(1);
        accumulator.absorb(item, report_bytes).await;
    }

    if done < total {
        let (outcome, seen_keys, unique_count) = accumulator.finish();
        FetchStage {
            outcome,
            pending: Some(PendingFetches {
                rx,
                tasks,
                seen_keys,
                unique_count,
                progress: context.progress,
                remaining: total.saturating_sub(done),
            }),
        }
    } else {
        FetchStage {
            outcome: accumulator.finish().0,
            pending: None,
        }
    }
}

async fn report_subscription_bytes<F, Fut>(bytes: u64, report_bytes: &mut F)
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    if bytes > 0 {
        report_bytes(bytes).await;
    }
}

/// Render a fetch failure for `errors`/`fetch_errors` consumers (logs, TUI,
/// dashboard) on two lines: the subscription headline, then the WHY
/// (timeout, DNS failure, HTTP status, oversized body…) from the full anyhow
/// chain. The request URL is redacted: HTTP errors echo it and it may carry
/// `?token=` or userinfo.
fn fetch_error_message(name: &str, error: &anyhow::Error, source_url: &str) -> String {
    let chained = format!("{error:#}");
    let headline = format!("failed to fetch subscription '{name}'");
    let detail: &str = chained
        .strip_prefix(headline.as_str())
        .map_or(chained.as_str(), |tail| {
            tail.strip_prefix(": ").unwrap_or(tail)
        });
    let detail = if source_url.is_empty() {
        detail.to_string()
    } else {
        redact_urls_in(detail)
    };
    format!("{headline}:\n{detail}")
}

/// Redact every URL that appears in a free-form error message.
///
/// The failing URL inside a reqwest error is the *rendered* one, which is
/// not always byte-identical to the configured URL: reqwest drops the
/// userinfo but keeps the query and the fragment. Redacting by replacing the
/// configured URL therefore misses the rendered one and leaks a private
/// `?token=` value into `fetch_errors` (which the dashboard and the TUI
/// both show). Scan the text instead and redact every URL-looking run.
fn redact_urls_in(text: &str) -> String {
    const SCHEMES: [&str; 2] = ["https://", "http://"];

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        // Earliest scheme start wins.
        let mut found: Option<(usize, &str)> = None;
        for scheme in SCHEMES {
            if let Some(index) = rest.find(scheme)
                && found.is_none_or(|(earliest, _)| index < earliest)
            {
                found = Some((index, scheme));
            }
        }
        let Some((index, _)) = found else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        // A URL run ends at whitespace or at a closing/quoting delimiter.
        let end = tail
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ')' | '"' | '\'' | '<' | '>'))
            .unwrap_or(tail.len());
        out.push_str(&crate::config::redact_subscription_url(&tail[..end]));
        rest = &tail[end..];
    }
    out
}

type FetchResult<T> = std::result::Result<T, FetchError>;

#[derive(Debug)]
struct FetchError {
    error: anyhow::Error,
    bytes_read: u64,
    /// Fetch attempts spent before giving up (1 = no retry was made).
    attempts: usize,
}

impl FetchError {
    const fn new(error: anyhow::Error, bytes_read: u64) -> Self {
        Self {
            error,
            bytes_read,
            attempts: 1,
        }
    }

    fn with_error_context(self, context: impl std::fmt::Display) -> Self {
        Self {
            error: self.error.context(context.to_string()),
            attempts: self.attempts,
            bytes_read: self.bytes_read,
        }
    }

    fn add_bytes(self, bytes: u64) -> Self {
        Self {
            error: self.error,
            attempts: self.attempts,
            bytes_read: self.bytes_read.saturating_add(bytes),
        }
    }
}

impl From<anyhow::Error> for FetchError {
    fn from(error: anyhow::Error) -> Self {
        Self::new(error, 0)
    }
}

struct FetchedSource {
    candidates: Vec<Candidate>,
    bytes_read: u64,
}

/// Connect (DNS + TCP + TLS) budget: a dead or hostile endpoint must fail
/// into the retry path quickly instead of eating the whole request timeout.
/// Capped at 10s, never zero.
fn connect_budget(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms.clamp(1, 10_000))
}

fn build_http_client(timeout_ms: u64) -> Result<Client> {
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .connect_timeout(connect_budget(timeout_ms))
        .user_agent(concat!("v2raydar/", env!("CARGO_PKG_VERSION")));

    if cfg!(target_os = "android")
        && let Some(tls) = crate::FALLBACK_TLS.get()
    {
        builder = builder.tls_backend_preconfigured(tls.clone());
    }

    builder.build().context("failed to build HTTP client")
}

fn build_proxied_http_client(timeout_ms: u64, port: u16) -> Result<Client> {
    let proxy_url = format!("http://{LOCALHOST_IP}:{port}");
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .connect_timeout(connect_budget(timeout_ms))
        .user_agent(concat!("v2raydar/", env!("CARGO_PKG_VERSION")))
        .proxy(Proxy::all(&proxy_url)?);

    if cfg!(target_os = "android")
        && let Some(tls) = crate::FALLBACK_TLS.get()
    {
        builder = builder.tls_backend_preconfigured(tls.clone());
    }

    builder
        .build()
        .context("failed to build proxied HTTP client")
}

async fn fetch_source(
    client: &Client,
    source: SubscriptionSource,
    max_bytes: usize,
    progress: Option<UnboundedSender<ProgressEvent>>,
) -> FetchResult<FetchedSource> {
    let started = std::time::Instant::now();
    info!(
        source = %source.name,
        url = %redact_subscription_url(&source.url),
        priority = source.priority,
        "subscription source fetch started"
    );
    send_progress(
        progress.as_ref(),
        LogLevel::Debug,
        format!("Fetching subscription '{}'", source.name),
    );
    let fetched = fetch_body_with_retries(
        client,
        &source.url,
        max_bytes,
        &source.name,
        progress.as_ref(),
    )
    .await
    .map_err(|err| {
        err.with_error_context(format!("failed to fetch subscription '{}'", source.name))
    })?;
    let fetch_ms = started.elapsed().as_millis();
    let parse_started = std::time::Instant::now();
    let candidates = parse_subscription_document(&source.name, source.priority, &fetched.body);
    info!(
        source = %source.name,
        body_bytes = fetched.body.len(),
        bytes_read = fetched.bytes_read,
        candidates = candidates.len(),
        fetch_ms,
        parse_ms = parse_started.elapsed().as_millis(),
        "subscription source loaded"
    );
    send_progress(
        progress.as_ref(),
        LogLevel::Debug,
        format!(
            "Loaded subscription '{}': {} configs, {} bytes",
            source.name,
            candidates.len(),
            fetched.bytes_read
        ),
    );

    if candidates.is_empty() {
        warn!(
            source = source.name,
            body_bytes = fetched.body.len(),
            bytes_read = fetched.bytes_read,
            "subscription did not contain supported share links"
        );
    }

    Ok(FetchedSource {
        candidates,
        bytes_read: fetched.bytes_read,
    })
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Failures where fetching through a working proxy may succeed: the
/// direct path looks network-restricted (blocked connect/DNS, timeouts)
/// or the status is commonly per-IP / geo-scoped (403, 429). Origin-side
/// failures (other 4xx/5xx, bad content) would fail identically through
/// a proxy, so they never trigger the proxy path.
fn proxy_may_help(error: &anyhow::Error) -> bool {
    let Some(request_error) = error.downcast_ref::<reqwest::Error>() else {
        return false;
    };
    request_error.is_connect()
        || request_error.is_timeout()
        || matches!(request_error.status(), Some(status) if status.as_u16() == 403 || status.as_u16() == 429)
}

/// Transient fetch failures worth another direct attempt: connect-phase
/// resets (e.g. WSAECONNRESET 10054 on Windows), timeouts, and 429/5xx
/// statuses. Everything else (other 4xx, TLS verification, local
/// file/data: errors) fails fast — retrying those only burns time.
fn is_transient_fetch_error(error: &anyhow::Error) -> bool {
    let Some(request_error) = error.downcast_ref::<reqwest::Error>() else {
        return false;
    };
    request_error.is_timeout()
        || request_error.is_connect()
        || matches!(request_error.status(), Some(status) if status.as_u16() == 429 || status.is_server_error())
}

/// Backoff before retry `attempt` (1-based): 1s, then 2s.
const fn fetch_retry_delay_ms(attempt: usize) -> u64 {
    FETCH_RETRY_BASE_DELAY_MS.saturating_mul(1u64 << attempt.saturating_sub(1))
}

/// Fetch one body, retrying transient failures with exponential backoff.
/// The final error is returned undecorated (the caller adds its context
/// once), so failure text is identical to a first-try failure.
async fn fetch_body_with_retries(
    client: &Client,
    url: &str,
    max_bytes: usize,
    source_name: &str,
    progress: Option<&UnboundedSender<ProgressEvent>>,
) -> FetchResult<FetchedBody> {
    let mut attempt: usize = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match fetch_body(client, url, max_bytes).await {
            Ok(fetched) => return Ok(fetched),
            Err(err) if attempt < FETCH_RETRY_ATTEMPTS && is_transient_fetch_error(&err.error) => {
                let delay_ms = fetch_retry_delay_ms(attempt);
                let detail = format!("{:#}", err.error);
                let detail = if is_http_url(url) {
                    detail.replace(url, &redact_subscription_url(url))
                } else {
                    detail
                };
                // Both sinks: tracing for --verbose runs, progress for the
                // dashboard, so a restricted network is diagnosable in either.
                debug!(
                    source = source_name,
                    attempt,
                    attempts = FETCH_RETRY_ATTEMPTS,
                    delay_ms,
                    bytes_read = err.bytes_read,
                    error = %detail,
                    "subscription fetch retry"
                );
                send_progress(
                    progress,
                    LogLevel::Debug,
                    format!(
                        "Sub '{source_name}': attempt {attempt} failed ({detail}), retrying in {delay_ms}ms"
                    ),
                );
                sleep(Duration::from_millis(delay_ms)).await;
            }
            Err(mut err) => {
                err.attempts = attempt;
                return Err(err);
            }
        }
    }
}

fn send_progress(
    progress: Option<&UnboundedSender<ProgressEvent>>,
    level: LogLevel,
    message: impl Into<String>,
) {
    if let Some(progress) = progress {
        let _ = progress.send(ProgressEvent::LiveLog(applog::line(level, &message.into())));
    }
}

fn send_fetched_delta(progress: Option<&UnboundedSender<ProgressEvent>>, total: usize) {
    if let Some(progress) = progress {
        let _ = progress.send(ProgressEvent::FetchedDelta(total));
    }
}

struct FetchedBody {
    body: Vec<u8>,
    bytes_read: u64,
}

async fn fetch_body(client: &Client, url: &str, max_bytes: usize) -> FetchResult<FetchedBody> {
    if is_http_url(url) {
        return fetch_http_body(client, url, max_bytes).await;
    }

    if url.starts_with("data:") {
        let body = parse_data_url(url)?;
        ensure_body_size(body.len(), max_bytes)?;
        return Ok(FetchedBody {
            bytes_read: 0,
            body,
        });
    }

    let path = url.strip_prefix("file://").unwrap_or(url);
    let body = fs::read(path)
        .await
        .with_context(|| format!("unable to read local subscription file {path}"))?;
    ensure_body_size(body.len(), max_bytes)?;
    Ok(FetchedBody {
        bytes_read: 0,
        body,
    })
}

async fn fetch_http_body(client: &Client, url: &str, max_bytes: usize) -> FetchResult<FetchedBody> {
    // Never log the raw URL: it can carry userinfo and a private token.
    let safe_url = redact_subscription_url(url);
    let started = std::time::Instant::now();
    let request_bytes = estimated_request_bytes(url);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| FetchError::new(err.into(), request_bytes))?;
    let status = response.status().as_u16();
    let content_length = response.content_length();
    let response_bytes = estimated_response_bytes(&response);
    let exchange_bytes = request_bytes.saturating_add(response_bytes);
    let response = response
        .error_for_status()
        .map_err(|err| FetchError::new(err.into(), exchange_bytes))?;
    let body = read_limited_response(response, max_bytes)
        .await
        .map_err(|err| err.add_bytes(exchange_bytes))?;
    let bytes_read = exchange_bytes.saturating_add(body.len() as u64);
    debug!(
        url = %safe_url,
        status,
        content_length,
        body_bytes = body.len(),
        bytes_read,
        duration_ms = started.elapsed().as_millis(),
        "HTTP subscription body read"
    );
    Ok(FetchedBody { body, bytes_read })
}

const fn estimated_request_bytes(url: &str) -> u64 {
    HTTP_EXCHANGE_OVERHEAD_BYTES.saturating_add(url.len() as u64)
}

fn estimated_response_bytes(response: &reqwest::Response) -> u64 {
    response
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len() as u64 + value.as_bytes().len() as u64 + 4)
        .sum::<u64>()
        .saturating_add(64)
}

async fn read_limited_response(
    response: reqwest::Response,
    max_bytes: usize,
) -> FetchResult<Vec<u8>> {
    let content_length = response.content_length().unwrap_or(0);
    let capacity = if content_length > 0 && content_length <= max_bytes as u64 {
        usize::try_from(content_length).unwrap_or_else(|_| max_bytes.min(256 * 1024))
    } else {
        max_bytes.min(256 * 1024)
    };

    let mut body = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| FetchError::new(err.into(), body.len() as u64))?;
        let next_size = body.len().saturating_add(chunk.len());
        ensure_body_size(next_size, max_bytes)
            .map_err(|err| FetchError::new(err, next_size as u64))?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn ensure_body_size(size: usize, max_bytes: usize) -> Result<()> {
    if size <= max_bytes {
        return Ok(());
    }
    Err(anyhow!(
        "subscription body is larger than max_subscription_bytes ({max_bytes})"
    ))
}

fn parse_data_url(url: &str) -> Result<Vec<u8>> {
    let (_, payload) = url
        .split_once(',')
        .ok_or_else(|| anyhow!("invalid data URL subscription"))?;
    let metadata = url.split_once(',').map_or("", |(metadata, _)| metadata);

    if metadata.ends_with(";base64") {
        return STANDARD
            .decode(payload.as_bytes())
            .context("invalid base64 data URL payload");
    }

    Ok(percent_decode_str(payload)
        .decode_utf8_lossy()
        .as_bytes()
        .to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[test]
    fn data_url_parsing_works() {
        let body = parse_data_url("data:,hello%20world").expect("data url parses");
        assert_eq!(body, b"hello world");
    }

    #[test]
    fn base64_data_url_parsing_works() {
        let body =
            parse_data_url("data:text/plain;base64,aGVsbG8gd29ybGQ=").expect("base64 data url");
        assert_eq!(body, b"hello world");
    }

    #[test]
    fn http_url_detection() {
        assert!(is_http_url("https://example.com"));
        assert!(is_http_url("http://example.com"));
        assert!(!is_http_url("data:,test"));
        assert!(!is_http_url("file:///path"));
    }

    #[test]
    fn fetch_error_message_splits_headline_and_redacts_url() {
        let url = "https://user:pass@example.com/sub?token=secret#frag";
        let error = anyhow!("operation timed out")
            .context(format!("error sending request for url ({url})"))
            .context("failed to fetch subscription 'demo'");
        let message = fetch_error_message("demo", &error, url);

        assert_eq!(
            message,
            "failed to fetch subscription 'demo':\nerror sending request for url (https://example.com/sub): operation timed out"
        );
        assert!(!message.contains("secret"), "token leaked: {message}");
        assert!(!message.contains("user:pass"), "userinfo leaked: {message}");
        assert!(!message.contains("#frag"), "fragment leaked: {message}");
    }

    /// The failing URL inside a reqwest error is the *rendered* one, which
    /// drops the userinfo but keeps the query and the fragment. It therefore
    /// never matches the configured URL byte for byte, and a redactor that
    /// replaces the configured URL misses it and leaks the token.
    #[test]
    fn fetch_error_message_redacts_the_rendered_url() {
        let configured = "https://user:pass@example.com/sub?token=secret#frag";
        let rendered = "https://example.com/sub?token=secret#frag";
        let error = anyhow!("operation timed out")
            .context(format!("error sending request for url ({rendered})"))
            .context("failed to fetch subscription 'demo'");
        let message = fetch_error_message("demo", &error, configured);

        assert!(!message.contains("secret"), "token leaked: {message}");
        assert!(!message.contains("user:pass"), "userinfo leaked: {message}");
        assert!(!message.contains("#frag"), "fragment leaked: {message}");
        assert!(
            message.contains("https://example.com/sub"),
            "host should survive: {message}"
        );
    }

    #[test]
    fn redact_urls_in_covers_every_url_in_the_text() {
        let text = "first https://a.example.com/x?token=one then \
                    http://b.example.com/y?token=two end";
        let redacted = redact_urls_in(text);

        assert!(!redacted.contains("token=one"), "leaked: {redacted}");
        assert!(!redacted.contains("token=two"), "leaked: {redacted}");
        assert!(redacted.contains("https://a.example.com/x"), "{redacted}");
        assert!(redacted.contains("http://b.example.com/y"), "{redacted}");
    }

    #[test]
    fn redact_urls_in_leaves_plain_text_untouched() {
        let text = "unable to read local subscription file /tmp/sub.txt";
        assert_eq!(redact_urls_in(text), text);
    }

    #[test]
    fn fetch_error_message_without_url_shows_chain_verbatim() {
        let error = anyhow!("permission denied")
            .context("unable to read local subscription file /tmp/sub.txt")
            .context("failed to fetch subscription 'local'");
        let message = fetch_error_message("local", &error, "/tmp/sub.txt");

        assert_eq!(
            message,
            "failed to fetch subscription 'local':\nunable to read local subscription file /tmp/sub.txt: permission denied"
        );
    }

    #[test]
    fn fetch_retry_backoff_doubles() {
        assert_eq!(fetch_retry_delay_ms(1), 1000);
        assert_eq!(fetch_retry_delay_ms(2), 2000);
        assert_eq!(FETCH_RETRY_ATTEMPTS, 3);
    }

    #[tokio::test]
    async fn transient_classifier_covers_connect_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("loopback addr").port();
        drop(listener);
        let err = Client::new()
            .get(format!("http://127.0.0.1:{port}/sub"))
            .send()
            .await
            .expect_err("refused port fails");
        assert!(err.is_connect());
        let anyhow_err: anyhow::Error = err.into();
        assert!(is_transient_fetch_error(&anyhow_err));
    }

    #[tokio::test]
    async fn transient_classifier_covers_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("loopback addr").port();
        tokio::spawn(async move {
            if let Ok((_socket, _)) = listener.accept().await {
                // Hold the connection open without responding: the client
                // must give up via its own timeout, not a server close.
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        let client = Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .expect("client");
        let err = client
            .get(format!("http://127.0.0.1:{port}/sub"))
            .send()
            .await
            .expect_err("silent server times out");
        assert!(err.is_timeout());
        let anyhow_err: anyhow::Error = err.into();
        assert!(is_transient_fetch_error(&anyhow_err));
    }

    /// One-shot local HTTP server replying `status` to the next connection.
    async fn serve_status_once(status: u16) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let url = format!(
            "http://{}/sub",
            listener.local_addr().expect("loopback addr")
        );
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                // Read the request first: answering (or closing) before the
                // client finishes sending aborts the connection (WSA 10053)
                // instead of delivering the intended status.
                let mut head = vec![0u8; 1024];
                let mut seen = 0;
                while seen < head.len() {
                    let Ok(read) = socket.read(&mut head[seen..]).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    seen += read;
                    if head[..seen].windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let phrase = match status {
                    503 => "Service Unavailable",
                    404 => "Not Found",
                    403 => "Forbidden",
                    429 => "Too Many Requests",
                    _ => "OK",
                };
                let response = format!(
                    "HTTP/1.1 {status} {phrase}\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx"
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        url
    }

    /// One-shot local HTTP server replying with `body` after `delay`.
    async fn serve_body_once(body: &'static str, delay: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let url = format!(
            "http://{}/sub",
            listener.local_addr().expect("loopback addr")
        );
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut head = vec![0u8; 1024];
                let mut seen = 0;
                while seen < head.len() {
                    let Ok(read) = socket.read(&mut head[seen..]).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    seen += read;
                    if head[..seen].windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        url
    }

    #[tokio::test]
    async fn transient_classifier_splits_status_codes() {
        let client = Client::new();
        let busy = client
            .get(serve_status_once(503).await)
            .send()
            .await
            .expect("503 responds");
        let err = busy.error_for_status().expect_err("503 is an error");
        assert_eq!(err.status(), Some(reqwest::StatusCode::SERVICE_UNAVAILABLE));
        let anyhow_err: anyhow::Error = err.into();
        assert!(is_transient_fetch_error(&anyhow_err));

        let missing = client
            .get(serve_status_once(404).await)
            .send()
            .await
            .expect("404 responds");
        let err = missing.error_for_status().expect_err("404 is an error");
        let anyhow_err: anyhow::Error = err.into();
        assert!(!is_transient_fetch_error(&anyhow_err));

        assert!(!is_transient_fetch_error(&anyhow!("boom")));
    }

    #[tokio::test]
    async fn proxy_classifier_covers_restricted_not_origin() {
        let client = Client::new();
        for (status, expected) in [(403, true), (429, true), (503, false), (404, false)] {
            let response = client
                .get(serve_status_once(status).await)
                .send()
                .await
                .expect("status responds");
            let err = response.error_for_status().expect_err("status is an error");
            let anyhow_err: anyhow::Error = err.into();
            assert_eq!(proxy_may_help(&anyhow_err), expected, "status {status}");
        }
        assert!(!proxy_may_help(&anyhow!("boom")));
    }

    #[tokio::test]
    async fn failure_flag_marks_proxy_retry_only_for_restricted() {
        // Refused port (blocked connect) qualifies for the proxy path; a
        // 404 origin fails identically through any egress, so it does not.
        let refused = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = refused.local_addr().expect("loopback addr").port();
        drop(refused);
        let sources = vec![
            SubscriptionSource {
                name: "blocked".to_string(),
                url: format!("http://127.0.0.1:{port}/sub"),
                enabled: true,
                priority: 1,
            },
            SubscriptionSource {
                name: "gone".to_string(),
                url: serve_status_once(404).await,
                enabled: true,
                priority: 2,
            },
        ];
        let outcome = fetch_sources_with_client(
            Client::new(),
            sources,
            FetchContext {
                max_bytes: 1024 * 1024,
                concurrency: 2,
                progress: None,
            },
            &mut |_bytes: u64| async {},
        )
        .await;
        assert_eq!(outcome.failures.len(), 2);
        let blocked = outcome
            .failures
            .iter()
            .find(|failure| failure.source.name == "blocked")
            .expect("blocked fails");
        assert!(blocked.proxy_retry);
        let gone = outcome
            .failures
            .iter()
            .find(|failure| failure.source.name == "gone")
            .expect("gone fails");
        assert!(!gone.proxy_retry);
    }

    #[tokio::test]
    async fn slow_source_does_not_block_the_fast_ones() {
        let fast = format!(
            "data:text/plain;base64,{}",
            STANDARD.encode("vless://uuid@fast.example.org:8443?security=tls#Fast")
        );
        let slow = serve_body_once(
            "vless://uuid@slow.example.org:8443?security=tls#Slow",
            Duration::from_millis(1500),
        )
        .await;
        let sources = vec![
            SubscriptionSource {
                name: "fast".to_string(),
                url: fast,
                enabled: true,
                priority: 1,
            },
            SubscriptionSource {
                name: "slow".to_string(),
                url: slow,
                enabled: true,
                priority: 2,
            },
        ];

        let stage = fetch_sources_streamed(
            Client::new(),
            sources,
            FetchContext {
                max_bytes: 1024 * 1024,
                concurrency: 2,
                progress: None,
            },
            &mut |_bytes: u64| async {},
            Some(Duration::from_millis(150)),
        )
        .await;

        assert_eq!(
            stage.outcome.candidates.len(),
            1,
            "the fast source is returned while the slow one retries"
        );
        let pending = stage.pending.expect("slow source is still in flight");
        let late = pending
            .collect(Duration::from_secs(5), &mut |_bytes: u64| async {})
            .await;
        assert_eq!(late.candidates.len(), 1, "the late config is delivered");
        assert_eq!(late.successes.len(), 1);
    }

    #[tokio::test]
    async fn soft_wait_does_not_return_an_empty_cycle() {
        let slow = serve_body_once(
            "vless://uuid@slow.example.org:8443?security=tls#Slow",
            Duration::from_millis(400),
        )
        .await;
        let stage = fetch_sources_streamed(
            Client::new(),
            vec![SubscriptionSource {
                name: "slow".to_string(),
                url: slow,
                enabled: true,
                priority: 1,
            }],
            FetchContext {
                max_bytes: 1024 * 1024,
                concurrency: 1,
                progress: None,
            },
            &mut |_bytes: u64| async {},
            Some(Duration::from_millis(50)),
        )
        .await;

        assert!(
            stage.pending.is_none(),
            "with no material yet the loader waits instead of starting an empty probe"
        );
        assert_eq!(stage.outcome.candidates.len(), 1);
    }

    #[tokio::test]
    async fn soft_wait_returns_known_failures_for_the_bridge() {
        let gone = serve_status_once(404).await;
        let slow = serve_body_once(
            "vless://uuid@slow.example.org:8443?security=tls#Slow",
            Duration::from_millis(1500),
        )
        .await;
        let stage = fetch_sources_streamed(
            Client::new(),
            vec![
                SubscriptionSource {
                    name: "gone".to_string(),
                    url: gone,
                    enabled: true,
                    priority: 1,
                },
                SubscriptionSource {
                    name: "slow".to_string(),
                    url: slow,
                    enabled: true,
                    priority: 2,
                },
            ],
            FetchContext {
                max_bytes: 1024 * 1024,
                concurrency: 2,
                progress: None,
            },
            &mut |_bytes: u64| async {},
            Some(Duration::from_millis(150)),
        )
        .await;

        assert!(
            stage.outcome.candidates.is_empty(),
            "nothing verified yet, but the known failure is enough for the proxy retry"
        );
        assert_eq!(stage.outcome.failures.len(), 1);
        let pending = stage.pending.expect("slow source still running");
        let late = pending
            .collect(Duration::from_secs(5), &mut |_bytes: u64| async {})
            .await;
        assert_eq!(
            late.candidates.len(),
            1,
            "the late config is still delivered"
        );
    }

    /// A proxy retry spins up a sing-box instance, so it must only cover the
    /// failures a tunnel can actually fix.
    #[test]
    fn proxy_retry_only_covers_failures_a_tunnel_can_fix() {
        let failure = |name: &str, url: &str, proxy_retry: bool| FetchFailure {
            source: SubscriptionSource {
                name: name.to_string(),
                url: url.to_string(),
                enabled: true,
                priority: 1,
            },
            error: "boom".to_string(),
            proxy_retry,
        };
        let failures = vec![
            failure("blocked", "http://127.0.0.1:9/sub", true),
            failure("gone", "http://127.0.0.1:9/other", false),
            failure("local", "/tmp/sub.txt", true),
        ];

        let sources = proxy_retry_sources(&failures);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "blocked");
    }

    #[tokio::test]
    async fn fetch_recovers_after_transient_503s() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let url = format!(
            "http://{}/sub",
            listener.local_addr().expect("loopback addr")
        );
        let hits = Arc::new(AtomicUsize::new(0));
        let served = Arc::clone(&hits);
        tokio::spawn(async move {
            for _ in 0..3 {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut head = vec![0u8; 1024];
                let mut seen = 0;
                while seen < head.len() {
                    let Ok(read) = socket.read(&mut head[seen..]).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    seen += read;
                    if head[..seen].windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let attempt = served.fetch_add(1, Ordering::SeqCst) + 1;
                let response = if attempt < 3 {
                    "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx"
                        .to_string()
                } else {
                    let body = "# nothing\n";
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        let source = SubscriptionSource {
            name: "flappy".to_string(),
            url,
            enabled: true,
            priority: 1,
        };
        let fetched = fetch_source(&Client::new(), source, 1024 * 1024, None)
            .await
            .expect("recovers after 503s");
        assert!(fetched.candidates.is_empty());
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }
}
