use std::{collections::HashSet, future::Future, time::Duration};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use percent_encoding::percent_decode_str;
use reqwest::Client;
use reqwest::Proxy;
use tokio::{fs, sync::mpsc::UnboundedSender, time::sleep};
use tracing::{debug, info, warn};

use crate::{
    applog::{self, LogLevel},
    config::{AppConfig, SubscriptionSource, redact_subscription_url},
    constants::{
        FETCH_RETRY_ATTEMPTS, FETCH_RETRY_BASE_DELAY_MS, HTTP_EXCHANGE_OVERHEAD_BYTES, LOCALHOST_IP,
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

pub async fn load_candidates_with_cache<F, Fut>(
    config: &AppConfig,
    mut report_bytes: F,
    progress: Option<UnboundedSender<ProgressEvent>>,
) -> Result<FetchOutcome>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    if !config.subscriptions.iter().any(|source| source.enabled) {
        info!("subscription load skipped because no subscriptions are enabled");
        return Ok(FetchOutcome {
            candidates: Vec::new(),
            errors: Vec::new(),
            failures: Vec::new(),
            successes: Vec::new(),
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

    let outcome = fetch_sources_with_client(
        client,
        sources,
        FetchContext {
            max_bytes: config.max_subscription_bytes,
            concurrency: config.fetch_concurrency,
            progress,
        },
        &mut report_bytes,
    )
    .await;

    Ok(outcome)
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
    let sources = failures
        .iter()
        .map(|failure| failure.source.clone())
        .filter(|source| is_http_url(&source.url))
        .collect::<Vec<_>>();

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
    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    let mut failures = Vec::new();
    let mut successes = Vec::new();
    let mut results = stream::iter(sources.into_iter().enumerate().map(|(index, source)| {
        let client = client.clone();
        let progress = context.progress.clone();
        let max_bytes = context.max_bytes;
        let source_for_result = source.clone();
        async move {
            // Timed from first poll (queue wait excluded): the per-source
            // line below reports honest fetch + parse time.
            let started = std::time::Instant::now();
            let result = fetch_source(&client, source, max_bytes, progress).await;
            (index, source_for_result, result, started.elapsed())
        }
    }))
    .buffer_unordered(context.concurrency);
    let mut fetched_sources = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut unique_count: usize = 0;

    while let Some((index, source, result, elapsed)) = results.next().await {
        match result {
            Ok(fetched) => {
                report_subscription_bytes(fetched.bytes_read, report_bytes).await;
                let parsed = fetched.candidates.len();
                let new_unique = fetched
                    .candidates
                    .iter()
                    .filter(|c| seen_keys.insert(c.dedup_key.clone()))
                    .count();
                unique_count = unique_count.saturating_add(new_unique);
                info!(
                    parsed,
                    new_unique,
                    bytes_read = fetched.bytes_read,
                    "subscription fetch result parsed"
                );
                send_progress(
                    context.progress.as_ref(),
                    LogLevel::Info,
                    success_line(&source, parsed, new_unique, fetched.bytes_read, elapsed),
                );
                send_fetched_delta(context.progress.as_ref(), unique_count);
                successes.push(source);
                fetched_sources.push((index, fetched.candidates));
            }
            Err(err) => {
                report_subscription_bytes(err.bytes_read, report_bytes).await;
                let message = fetch_error_message(&source.name, &err.error, &source.url);
                warn!(error = %message, "subscription fetch failed");
                send_progress(
                    context.progress.as_ref(),
                    LogLevel::Warn,
                    failure_line(&source, &message, err.bytes_read, elapsed),
                );
                errors.push(message.clone());
                failures.push((
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
    fetched_sources.sort_by_key(|(index, _)| *index);
    let mut dedup_keys = HashSet::new();
    for (_, mut fetched) in fetched_sources {
        fetched.retain(|candidate| dedup_keys.insert(candidate.dedup_key.clone()));
        candidates.append(&mut fetched);
    }
    failures.sort_by_key(|(index, _)| *index);
    let failures = failures
        .into_iter()
        .map(|(_, failure)| failure)
        .collect::<Vec<_>>();

    FetchOutcome {
        candidates,
        errors,
        failures,
        successes,
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
        detail.replace(source_url, &redact_subscription_url(source_url))
    };
    format!("{headline}:\n{detail}")
}

type FetchResult<T> = std::result::Result<T, FetchError>;

#[derive(Debug)]
struct FetchError {
    error: anyhow::Error,
    bytes_read: u64,
}

impl FetchError {
    const fn new(error: anyhow::Error, bytes_read: u64) -> Self {
        Self { error, bytes_read }
    }

    fn with_error_context(self, context: impl std::fmt::Display) -> Self {
        Self {
            error: self.error.context(context.to_string()),
            bytes_read: self.bytes_read,
        }
    }

    fn add_bytes(self, bytes: u64) -> Self {
        Self {
            error: self.error,
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

fn build_http_client(timeout_ms: u64) -> Result<Client> {
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
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
    info!(
        source = %source.name,
        body_bytes = fetched.body.len(),
        bytes_read = fetched.bytes_read,
        duration_ms = started.elapsed().as_millis(),
        "subscription source fetch finished"
    );
    let parse_started = std::time::Instant::now();
    let candidates = parse_subscription_document(&source.name, source.priority, &fetched.body);
    info!(
        source = %source.name,
        candidates = candidates.len(),
        duration_ms = parse_started.elapsed().as_millis(),
        "subscription source parse finished"
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

/// Backoff before retry `attempt` (1-based): 500ms, then 1s.
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
                send_progress(
                    progress,
                    LogLevel::Debug,
                    format!(
                        "Sub '{source_name}': attempt {attempt} failed ({detail}), retrying in {delay_ms}ms"
                    ),
                );
                sleep(Duration::from_millis(delay_ms)).await;
            }
            Err(err) => return Err(err),
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
    debug!(url, "HTTP subscription request prepared");

    let request_bytes = estimated_request_bytes(url);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| FetchError::new(err.into(), request_bytes))?;
    debug!(
        url,
        status = response.status().as_u16(),
        content_length = response.content_length(),
        "HTTP subscription response received"
    );
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
        url,
        body_bytes = body.len(),
        bytes_read,
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
