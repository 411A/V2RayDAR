use std::{collections::BTreeMap, time::Duration};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::{StreamExt, stream};
use percent_encoding::percent_decode_str;
use reqwest::Client;
use tokio::fs;
use tracing::warn;

use crate::{
    config::{AppConfig, SubscriptionSource},
    model::Candidate,
    parser::parse_subscription_document,
};

#[derive(Debug)]
pub struct FetchOutcome {
    pub candidates: Vec<Candidate>,
    pub errors: Vec<String>,
}

pub async fn load_candidates(config: &AppConfig) -> Result<FetchOutcome> {
    if config.subscriptions.is_empty() {
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

    let mut groups: BTreeMap<u32, Vec<SubscriptionSource>> = BTreeMap::new();
    for subscription in &config.subscriptions {
        groups
            .entry(subscription.priority)
            .or_default()
            .push(subscription.clone());
    }

    let mut candidates = Vec::new();
    let mut errors = Vec::new();

    for (_, sources) in groups {
        let results = stream::iter(sources.into_iter().map(|source| {
            let client = client.clone();
            async move { fetch_source(&client, source).await }
        }))
        .buffer_unordered(config.fetch_concurrency)
        .collect::<Vec<_>>()
        .await;

        for result in results {
            match result {
                Ok(mut fetched) => candidates.append(&mut fetched),
                Err(err) => {
                    warn!(error = %err, "subscription fetch failed");
                    errors.push(err.to_string());
                }
            }
        }
    }

    deduplicate(&mut candidates);

    if candidates.is_empty() && !errors.is_empty() {
        return Err(anyhow!(
            "no usable configs were loaded; first error: {}",
            errors[0]
        ));
    }

    Ok(FetchOutcome { candidates, errors })
}

async fn fetch_source(client: &Client, source: SubscriptionSource) -> Result<Vec<Candidate>> {
    let body = fetch_body(client, &source.url)
        .await
        .with_context(|| format!("failed to fetch subscription '{}'", source.name))?;
    let candidates = parse_subscription_document(&source.name, source.priority, &body);

    if candidates.is_empty() {
        warn!(
            source = source.name,
            "subscription did not contain supported share links"
        );
    }

    Ok(candidates)
}

async fn fetch_body(client: &Client, url: &str) -> Result<Vec<u8>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let response = client.get(url).send().await?.error_for_status()?;
        return Ok(response.bytes().await?.to_vec());
    }

    if url.starts_with("data:") {
        return parse_data_url(url);
    }

    let path = url.strip_prefix("file://").unwrap_or(url);
    fs::read(path)
        .await
        .with_context(|| format!("unable to read local subscription file {path}"))
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

fn deduplicate(candidates: &mut Vec<Candidate>) {
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.uri.clone()));
}
