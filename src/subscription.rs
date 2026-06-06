use std::{
    collections::HashSet,
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use percent_encoding::percent_decode_str;
use reqwest::Client;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::warn;

use crate::{
    config::{AppConfig, SubscriptionSource},
    model::Candidate,
    parser::parse_subscription_document,
};

const HTTP_EXCHANGE_OVERHEAD_BYTES: u64 = 1024;

#[derive(Debug)]
pub struct FetchOutcome {
    pub candidates: Vec<Candidate>,
    pub errors: Vec<String>,
}

pub async fn load_candidates_with_cache<F, Fut>(
    config: &AppConfig,
    cache_dir: Option<&Path>,
    mut report_bytes: F,
) -> Result<FetchOutcome>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    if !config.subscriptions.iter().any(|source| source.enabled) {
        return Ok(FetchOutcome {
            candidates: Vec::new(),
            errors: Vec::new(),
        });
    }

    let client = Client::builder()
        .timeout(Duration::from_millis(config.fetch_timeout_ms))
        .user_agent(concat!("v2raydar/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")?;

    let mut candidates = Vec::new();
    let mut seen_uris = HashSet::new();
    let mut errors = Vec::new();
    let max_bytes = config.max_subscription_bytes;
    let fetch_concurrency = config.fetch_concurrency;

    let sources = config
        .subscriptions
        .iter()
        .filter(|source| source.enabled)
        .cloned()
        .collect::<Vec<_>>();
    let mut results = stream::iter(sources.into_iter().map(|source| {
        let client = client.clone();
        let cache_dir = cache_dir.map(Path::to_path_buf);
        async move { fetch_source(&client, source, cache_dir.as_deref(), max_bytes).await }
    }))
    .buffer_unordered(fetch_concurrency);

    while let Some(result) = results.next().await {
        match result {
            Ok(mut fetched) => {
                report_subscription_bytes(fetched.bytes_read, &mut report_bytes).await;
                fetched
                    .candidates
                    .retain(|candidate| seen_uris.insert(candidate.uri.clone()));
                candidates.append(&mut fetched.candidates);
            }
            Err(err) => {
                report_subscription_bytes(err.bytes_read, &mut report_bytes).await;
                warn!(error = %err.error, "subscription fetch failed");
                errors.push(err.error.to_string());
            }
        }
    }

    if candidates.is_empty() && !errors.is_empty() {
        return Err(anyhow!(
            "no usable configs were loaded; first error: {}",
            errors[0]
        ));
    }

    Ok(FetchOutcome { candidates, errors })
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
    fn new(error: anyhow::Error, bytes_read: u64) -> Self {
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

async fn fetch_source(
    client: &Client,
    source: SubscriptionSource,
    cache_dir: Option<&Path>,
    max_bytes: usize,
) -> FetchResult<FetchedSource> {
    let fetched = fetch_body(client, &source.url, cache_dir, max_bytes)
        .await
        .map_err(|err| {
            err.with_error_context(format!("failed to fetch subscription '{}'", source.name))
        })?;
    let candidates = parse_subscription_document(&source.name, source.priority, &fetched.body);

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

struct FetchedBody {
    body: Vec<u8>,
    bytes_read: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct CacheMetadata {
    etag: Option<String>,
    last_modified: Option<String>,
}

async fn fetch_body(
    client: &Client,
    url: &str,
    cache_dir: Option<&Path>,
    max_bytes: usize,
) -> FetchResult<FetchedBody> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return fetch_http_body(client, url, cache_dir, max_bytes).await;
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
    let cache = cache_dir.map(|dir| cache_paths(dir, url));
    let metadata = match &cache {
        Some(cache) => read_cache_metadata(&cache.metadata)
            .await
            .unwrap_or_default(),
        None => CacheMetadata::default(),
    };

    let mut request = client.get(url);
    let mut request_bytes = estimated_request_bytes(url);
    if let Some(etag) = metadata.etag.as_deref() {
        request = request.header(IF_NONE_MATCH, etag);
        request_bytes = request_bytes.saturating_add(header_wire_bytes("If-None-Match", etag));
    }
    if let Some(last_modified) = metadata.last_modified.as_deref() {
        request = request.header(IF_MODIFIED_SINCE, last_modified);
        request_bytes =
            request_bytes.saturating_add(header_wire_bytes("If-Modified-Since", last_modified));
    }

    let response = request
        .send()
        .await
        .map_err(|err| FetchError::new(err.into(), request_bytes))?;
    let response_bytes = estimated_response_bytes(&response);
    let exchange_bytes = request_bytes.saturating_add(response_bytes);
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let Some(cache) = cache else {
            return Err(FetchError::new(
                anyhow!("server returned 304 without a local cache"),
                exchange_bytes,
            ));
        };
        let body = fs::read(&cache.body)
            .await
            .map_err(|err| FetchError::new(err.into(), exchange_bytes))?;
        ensure_body_size(body.len(), max_bytes)
            .map_err(|err| FetchError::new(err, exchange_bytes))?;
        return Ok(FetchedBody {
            body,
            bytes_read: exchange_bytes,
        });
    }

    let response = response
        .error_for_status()
        .map_err(|err| FetchError::new(err.into(), exchange_bytes))?;
    let etag = header_string(response.headers(), ETAG);
    let last_modified = header_string(response.headers(), LAST_MODIFIED);
    let body = read_limited_response(response, max_bytes)
        .await
        .map_err(|err| err.add_bytes(exchange_bytes))?;
    let bytes_read = exchange_bytes.saturating_add(body.len() as u64);
    if let Some(cache) = cache {
        write_cache(&cache, &body, etag, last_modified).await;
    }
    Ok(FetchedBody { bytes_read, body })
}

struct CachePaths {
    body: PathBuf,
    metadata: PathBuf,
}

fn cache_paths(cache_dir: &Path, url: &str) -> CachePaths {
    let key = cache_key(url);
    CachePaths {
        body: cache_dir.join(format!("{key}.body")),
        metadata: cache_dir.join(format!("{key}.json")),
    }
}

fn cache_key(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

async fn read_cache_metadata(path: &Path) -> Result<CacheMetadata> {
    let bytes = fs::read(path).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn header_string(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn estimated_request_bytes(url: &str) -> u64 {
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

fn header_wire_bytes(name: &str, value: &str) -> u64 {
    name.len() as u64 + value.len() as u64 + 4
}

async fn read_limited_response(
    response: reqwest::Response,
    max_bytes: usize,
) -> FetchResult<Vec<u8>> {
    let content_length = response.content_length().unwrap_or(0);
    if content_length > max_bytes as u64 {
        return Err(FetchError::new(
            anyhow!("subscription body is larger than max_subscription_bytes ({max_bytes})"),
            0,
        ));
    }

    let mut body = Vec::with_capacity(content_length as usize);
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

async fn write_cache(
    cache: &CachePaths,
    body: &[u8],
    etag: Option<String>,
    last_modified: Option<String>,
) {
    if let Some(parent) = cache.body.parent() {
        let _ = fs::create_dir_all(parent).await;
    }
    let metadata = CacheMetadata {
        etag,
        last_modified,
    };
    if fs::write(&cache.body, body).await.is_ok()
        && let Ok(bytes) = serde_json::to_vec(&metadata)
    {
        let _ = fs::write(&cache.metadata, bytes).await;
    }
}

fn parse_data_url(url: &str) -> Result<Vec<u8>> {
    let (_, payload) = url
        .split_once(',')
        .ok_or_else(|| anyhow!("invalid data URL subscription"))?;
    let metadata = url
        .split_once(',')
        .map(|(metadata, _)| metadata)
        .unwrap_or("");

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
