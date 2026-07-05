use std::{
    collections::{BTreeMap, HashSet},
    future::Future,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use percent_encoding::percent_decode_str;
use reqwest::Client;
use reqwest::Proxy;
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::mpsc::UnboundedSender};
use tracing::{debug, info, warn};

use crate::{
    config::{AppConfig, SubscriptionSource},
    constants::{
        CACHE_METADATA_FILE_NAME, FNV_OFFSET_BASIS, FNV_PRIME, HTTP_EXCHANGE_OVERHEAD_BYTES,
        LOCALHOST_IP,
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
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FetchMode {
    FreshOnly,
    CacheOnly,
}

pub async fn load_candidates_with_cache<F, Fut>(
    config: &AppConfig,
    cache_dir: Option<&Path>,
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
        format!(
            "Subscription load: fetching {} enabled sources",
            sources.len()
        ),
    );

    let outcome = fetch_sources_with_client(
        client,
        sources,
        FetchContext {
            cache_dir,
            max_bytes: config.max_subscription_bytes,
            concurrency: config.fetch_concurrency,
            progress,
            mode: FetchMode::FreshOnly,
        },
        &mut report_bytes,
    )
    .await;

    Ok(outcome)
}

pub async fn load_cached_candidates<F, Fut>(
    config: &AppConfig,
    cache_dir: &Path,
    mut report_bytes: F,
    progress: Option<UnboundedSender<ProgressEvent>>,
) -> Result<FetchOutcome>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    let client =
        build_http_client(config.fetch_timeout_ms).context("failed to build HTTP client")?;
    let sources = config
        .subscriptions
        .iter()
        .filter(|source| source.enabled)
        .cloned()
        .collect::<Vec<_>>();

    send_progress(
        progress.as_ref(),
        "All subscription fetches failed; trying cached subscription snapshots",
    );

    Ok(fetch_sources_with_client(
        client,
        sources,
        FetchContext {
            cache_dir: Some(cache_dir),
            max_bytes: config.max_subscription_bytes,
            concurrency: config.fetch_concurrency,
            progress,
            mode: FetchMode::CacheOnly,
        },
        &mut report_bytes,
    )
    .await)
}

pub async fn retry_failed_sources_with_proxy<F, Fut>(
    config: &AppConfig,
    failures: &[FetchFailure],
    proxy_uri: &str,
    cache_dir: Option<&Path>,
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
                        cache_dir,
                        max_bytes,
                        concurrency: fetch_concurrency,
                        progress,
                        mode: FetchMode::FreshOnly,
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
struct FetchContext<'a> {
    cache_dir: Option<&'a Path>,
    max_bytes: usize,
    concurrency: usize,
    progress: Option<UnboundedSender<ProgressEvent>>,
    mode: FetchMode,
}

async fn fetch_sources_with_client<F, Fut>(
    client: Client,
    sources: Vec<SubscriptionSource>,
    context: FetchContext<'_>,
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
        let cache_dir = context.cache_dir.map(Path::to_path_buf);
        let progress = context.progress.clone();
        let mode = context.mode;
        let max_bytes = context.max_bytes;
        let source_for_result = source.clone();
        async move {
            (
                index,
                source_for_result,
                fetch_source(
                    &client,
                    source,
                    cache_dir.as_deref(),
                    max_bytes,
                    progress,
                    mode,
                )
                .await,
            )
        }
    }))
    .buffer_unordered(context.concurrency);
    let mut fetched_sources = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut unique_count: usize = 0;

    while let Some((index, source, result)) = results.next().await {
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
                    format!(
                        "Subscription parsed: {new_unique} unique configs from {parsed} fetched links",
                    ),
                );
                send_fetched_delta(context.progress.as_ref(), unique_count);
                successes.push(source);
                fetched_sources.push((index, fetched.candidates));
            }
            Err(err) => {
                report_subscription_bytes(err.bytes_read, report_bytes).await;
                warn!(error = %err.error, "subscription fetch failed");
                send_progress(
                    context.progress.as_ref(),
                    format!("Subscription fetch failed: {}", err.error),
                );
                let error = err.error.to_string();
                errors.push(error.clone());
                failures.push((index, FetchFailure { source, error }));
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
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(concat!("v2raydar/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")
}

fn build_proxied_http_client(timeout_ms: u64, port: u16) -> Result<Client> {
    let proxy_url = format!("http://{LOCALHOST_IP}:{port}");
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(concat!("v2raydar/", env!("CARGO_PKG_VERSION")))
        .proxy(Proxy::all(&proxy_url)?)
        .build()
        .context("failed to build proxied HTTP client")
}

async fn fetch_source(
    client: &Client,
    source: SubscriptionSource,
    cache_dir: Option<&Path>,
    max_bytes: usize,
    progress: Option<UnboundedSender<ProgressEvent>>,
    mode: FetchMode,
) -> FetchResult<FetchedSource> {
    let started = std::time::Instant::now();
    info!(
        source = %source.name,
        url = %source.url,
        priority = source.priority,
        "subscription source fetch started"
    );
    send_progress(
        progress.as_ref(),
        format!("Fetching subscription '{}'", source.name),
    );
    let fetched = fetch_body(client, &source.url, cache_dir, max_bytes, mode)
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

fn send_progress(progress: Option<&UnboundedSender<ProgressEvent>>, message: impl Into<String>) {
    if let Some(progress) = progress {
        let _ = progress.send(ProgressEvent::LiveLog(message.into()));
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

#[derive(Debug, Default, Deserialize, Serialize)]
struct CacheMetadata {
    subscriptions: BTreeMap<String, Vec<CacheSnapshot>>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
struct CacheSnapshot {
    file: String,
    hash: String,
}

async fn fetch_body(
    client: &Client,
    url: &str,
    cache_dir: Option<&Path>,
    max_bytes: usize,
    mode: FetchMode,
) -> FetchResult<FetchedBody> {
    if is_http_url(url) {
        return match mode {
            FetchMode::FreshOnly => fetch_http_body(client, url, cache_dir, max_bytes).await,
            FetchMode::CacheOnly => fetch_cached_http_body(url, cache_dir, max_bytes).await,
        };
    }

    if mode == FetchMode::CacheOnly {
        return Err(FetchError::new(
            anyhow!("cache fallback only supports HTTP subscriptions"),
            0,
        ));
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

async fn fetch_http_body(
    client: &Client,
    url: &str,
    cache_dir: Option<&Path>,
    max_bytes: usize,
) -> FetchResult<FetchedBody> {
    debug!(
        url,
        has_cache = cache_dir.is_some(),
        "HTTP subscription request prepared"
    );

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
    if let Some(cache_dir) = cache_dir {
        write_cache_snapshot(cache_dir, url, &body).await;
    }
    Ok(FetchedBody { body, bytes_read })
}

async fn fetch_cached_http_body(
    url: &str,
    cache_dir: Option<&Path>,
    max_bytes: usize,
) -> FetchResult<FetchedBody> {
    let Some(cache_dir) = cache_dir else {
        return Err(FetchError::new(anyhow!("no cache directory configured"), 0));
    };
    let metadata = read_cache_metadata(&cache_dir.join(CACHE_METADATA_FILE_NAME))
        .await
        .map_err(|err| FetchError::new(err, 0))?;
    let Some(snapshots) = metadata.subscriptions.get(url) else {
        return Err(FetchError::new(
            anyhow!("no cached subscription snapshots for {url}"),
            0,
        ));
    };

    for snapshot in snapshots.iter().rev() {
        if !is_cache_snapshot_file_name(&snapshot.file) {
            continue;
        }
        let path = cache_dir.join(&snapshot.file);
        match fs::read(&path).await {
            Ok(body) => {
                ensure_body_size(body.len(), max_bytes)
                    .map_err(|err| FetchError::new(err, body.len() as u64))?;
                info!(
                    url,
                    file = %snapshot.file,
                    body_bytes = body.len(),
                    "HTTP subscription loaded from cache snapshot"
                );
                return Ok(FetchedBody {
                    bytes_read: 0,
                    body,
                });
            }
            Err(err) => {
                warn!(url, file = %snapshot.file, error = %err, "cached subscription snapshot unreadable");
            }
        }
    }

    Err(FetchError::new(
        anyhow!("no readable cached subscription snapshots for {url}"),
        0,
    ))
}

async fn read_cache_metadata(path: &Path) -> Result<CacheMetadata> {
    let bytes = fs::read(path).await?;
    serde_json::from_slice(&bytes).context("invalid subscription cache metadata")
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
    if content_length > max_bytes as u64 {
        warn!(
            content_length,
            max_bytes, "subscription body rejected by content-length"
        );
        return Err(FetchError::new(
            anyhow!("subscription body is larger than max_subscription_bytes ({max_bytes})"),
            0,
        ));
    }

    let mut body = Vec::with_capacity(usize::try_from(content_length).unwrap_or(max_bytes));
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

async fn write_cache_snapshot(cache_dir: &Path, url: &str, body: &[u8]) {
    let _ = fs::create_dir_all(cache_dir).await;
    let hash = cache_body_hash(body);
    let metadata_path = cache_dir.join(CACHE_METADATA_FILE_NAME);
    let mut metadata = read_cache_metadata(&metadata_path)
        .await
        .unwrap_or_default();
    let snapshots = metadata.subscriptions.entry(url.to_string()).or_default();
    if let Some(index) = snapshots.iter().position(|snapshot| snapshot.hash == hash) {
        let snapshot = snapshots.remove(index);
        snapshots.push(snapshot);
        if let Ok(bytes) = serde_json::to_vec_pretty(&metadata) {
            let _ = fs::write(metadata_path, bytes).await;
        }
        return;
    }

    let file_name = cache_snapshot_file_name();
    if fs::write(cache_dir.join(&file_name), body).await.is_err() {
        return;
    }
    snapshots.push(CacheSnapshot {
        file: file_name,
        hash,
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&metadata) {
        let _ = fs::write(metadata_path, bytes).await;
    }
}

fn cache_body_hash(body: &[u8]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

fn cache_snapshot_file_name() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%d_%H-%M-%S%.3f.txt")
        .to_string()
}

pub fn is_cache_snapshot_file_name(name: &str) -> bool {
    name.len() >= "2026-06-08_22-08-09.985.txt".len()
        && std::path::Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        && !name.contains('/')
        && !name.contains('\\')
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

    fn temp_cache_dir(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "v2raydar-cache-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("system time is after unix epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn cache_snapshot_file_names_are_safe() {
        let file_name = cache_snapshot_file_name();
        assert!(is_cache_snapshot_file_name(&file_name));
        assert!(!is_cache_snapshot_file_name("../bad.txt"));
        assert!(!is_cache_snapshot_file_name("metadata.json"));
    }

    #[tokio::test]
    async fn duplicate_cache_body_reuses_existing_snapshot() {
        let cache_dir = temp_cache_dir("dedup");
        let url = "https://example.com/sub";

        write_cache_snapshot(&cache_dir, url, b"same body").await;
        write_cache_snapshot(&cache_dir, url, b"same body").await;

        let metadata = read_cache_metadata(&cache_dir.join(CACHE_METADATA_FILE_NAME))
            .await
            .expect("metadata can be read");
        let snapshots = metadata
            .subscriptions
            .get(url)
            .expect("subscription metadata exists");
        assert_eq!(snapshots.len(), 1);

        let txt_count = std::fs::read_dir(&cache_dir)
            .expect("cache dir can be read")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(is_cache_snapshot_file_name)
            })
            .count();
        assert_eq!(txt_count, 1);

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[tokio::test]
    async fn duplicate_cache_body_moves_snapshot_to_newest_position() {
        let cache_dir = temp_cache_dir("dedup-newest");
        let url = "https://example.com/sub";

        write_cache_snapshot(&cache_dir, url, b"first").await;
        write_cache_snapshot(&cache_dir, url, b"second").await;
        let metadata = read_cache_metadata(&cache_dir.join(CACHE_METADATA_FILE_NAME))
            .await
            .expect("metadata can be read");
        let first_snapshot = metadata
            .subscriptions
            .get(url)
            .expect("subscription metadata exists")
            .first()
            .expect("first snapshot exists")
            .clone();

        write_cache_snapshot(&cache_dir, url, b"first").await;

        let metadata = read_cache_metadata(&cache_dir.join(CACHE_METADATA_FILE_NAME))
            .await
            .expect("metadata can be read");
        let snapshots = metadata
            .subscriptions
            .get(url)
            .expect("subscription metadata exists");
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots.last(), Some(&first_snapshot));

        let _ = std::fs::remove_dir_all(cache_dir);
    }
}
