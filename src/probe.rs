use std::{cmp::Ordering, time::Duration};

use futures::{StreamExt, stream};
use tokio::{net::TcpStream, time::Instant};

use crate::{
    config::ProbeConfig,
    model::{Candidate, RankedConfig},
};

pub async fn probe_candidates(
    candidates: Vec<Candidate>,
    config: &ProbeConfig,
) -> Vec<RankedConfig> {
    let timeout = Duration::from_millis(config.connect_timeout_ms);
    let mut ranked = stream::iter(
        candidates
            .into_iter()
            .map(|candidate| async move { probe_one(candidate, timeout).await }),
    )
    .buffer_unordered(config.concurrency)
    .collect::<Vec<_>>()
    .await;

    ranked.sort_by(compare_ranked);
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }

    ranked
}

async fn probe_one(candidate: Candidate, timeout: Duration) -> RankedConfig {
    let target = (candidate.endpoint.host.as_str(), candidate.endpoint.port);
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, TcpStream::connect(target)).await;

    let (reachable, latency_ms, error) = match result {
        Ok(Ok(_stream)) => (true, Some(started.elapsed().as_millis()), None),
        Ok(Err(err)) => (false, None, Some(err.to_string())),
        Err(_) => (
            false,
            None,
            Some(format!("timed out after {} ms", timeout.as_millis())),
        ),
    };

    RankedConfig {
        rank: 0,
        id: candidate.id,
        source: candidate.source,
        priority: candidate.priority,
        protocol: candidate.protocol,
        name: candidate.name,
        endpoint: candidate.endpoint,
        uri: candidate.uri,
        reachable,
        latency_ms,
        download_mbps: None,
        error,
    }
}

fn compare_ranked(left: &RankedConfig, right: &RankedConfig) -> Ordering {
    right
        .reachable
        .cmp(&left.reachable)
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| {
            left.latency_ms
                .unwrap_or(u128::MAX)
                .cmp(&right.latency_ms.unwrap_or(u128::MAX))
        })
        .then_with(|| left.protocol.cmp(&right.protocol))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.uri.cmp(&right.uri))
}
