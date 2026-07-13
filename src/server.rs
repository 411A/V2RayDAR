use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use tokio::{
    net::TcpListener,
    sync::RwLock,
    time::{Duration, Instant, sleep},
};
use tracing::{info, warn};

use crate::{
    constants::{SUBSCRIPTION_READY_POLL, SUBSCRIPTION_READY_WAIT},
    model::{RuntimeConfig, RuntimeState},
    network::primary_lan_ip,
};

type SharedState = Arc<RwLock<RuntimeState>>;
type SharedConfig = Arc<RwLock<RuntimeConfig>>;

#[derive(Clone)]
struct HttpState {
    runtime: SharedState,
    config: SharedConfig,
}

pub async fn serve(bind: SocketAddr, runtime: SharedState, config: SharedConfig) -> Result<()> {
    let state = HttpState { runtime, config };
    let router = router(state.clone());

    tokio::select! {
        result = serve_listener(bind, router.clone()) => result,
        result = serve_lan_sharing(bind, router, state.config.clone()) => result,
    }
}

fn router(state: HttpState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/results", get(results))
        .route("/subscription", get(subscription))
        .route("/subscription.txt", get(subscription_txt))
        .route("/mihomo.yaml", get(mihomo_yaml))
        .with_state(state)
}

async fn serve_listener(bind: SocketAddr, router: Router) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| bind_error_context(bind))?;
    info!(bind = %bind, "HTTP endpoint listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn serve_lan_sharing(
    local_bind: SocketAddr,
    router: Router,
    config: SharedConfig,
) -> Result<()> {
    let mut active: Option<(SocketAddr, tokio::task::JoinHandle<Result<()>>)> = None;

    loop {
        let runtime_config = config.read().await;
        let desired = desired_lan_bind(local_bind, &runtime_config);
        drop(runtime_config);
        let active_bind = active.as_ref().map(|(bind, _)| *bind);

        if desired != active_bind {
            if let Some((bind, task)) = active.take() {
                task.abort();
                info!(bind = %bind, "LAN sharing listener stopped");
            }

            if let Some(bind) = desired {
                let router = router.clone();
                let task = tokio::spawn(async move { serve_listener(bind, router).await });
                info!(bind = %bind, "LAN sharing listener starting");
                active = Some((bind, task));
            }
        }

        if let Some((bind, task)) = active.as_ref()
            && task.is_finished()
        {
            let bind = *bind;
            let (_, task) = active.take().expect("active task exists");
            match task.await {
                Ok(Ok(())) => info!(bind = %bind, "LAN sharing listener exited"),
                Ok(Err(error)) => {
                    warn!(bind = %bind, error = %error, "LAN sharing listener failed");
                }
                Err(error) => {
                    warn!(bind = %bind, error = %error, "LAN sharing listener task failed");
                }
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

fn desired_lan_bind(local_bind: SocketAddr, config: &RuntimeConfig) -> Option<SocketAddr> {
    if !config.sharing_enabled || !local_bind.ip().is_loopback() {
        return None;
    }

    primary_lan_ip().map(|ip| SocketAddr::new(ip, local_bind.port()))
}

fn bind_error_context(bind: SocketAddr) -> String {
    if cfg!(target_os = "windows") {
        return format!(
            "unable to bind configured address {bind}; Windows may forbid this port even when no app is using it. Check reserved ranges with: netsh interface ipv4 show excludedportrange protocol=tcp"
        );
    }

    format!("unable to bind configured address {bind}")
}

async fn health() -> &'static str {
    "ok\n"
}

#[derive(Debug, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

async fn results(
    State(state): State<HttpState>,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
) -> Response {
    match authorize(&state, remote_addr, query.token.as_deref()).await {
        Ok(()) => Json(state.runtime.read().await.clone()).into_response(),
        Err(response) => response,
    }
}

async fn subscription(
    State(state): State<HttpState>,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
) -> Response {
    let encoded = state.config.read().await.encoded_subscription;
    subscription_response(&state, remote_addr, query.token.as_deref(), encoded).await
}

async fn subscription_txt(
    State(state): State<HttpState>,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
) -> Response {
    subscription_response(&state, remote_addr, query.token.as_deref(), false).await
}

async fn mihomo_yaml(
    State(state): State<HttpState>,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
) -> Response {
    mihomo_response(&state, remote_addr, query.token.as_deref()).await
}

async fn mihomo_response(
    state: &HttpState,
    remote_addr: SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }

    let config = state.config.read().await.clone();
    let runtime = subscription_snapshot(&state.runtime).await;
    let uris: Vec<String> = runtime
        .ranked
        .iter()
        .filter(|item| item.reachable)
        .take(config.top_n)
        .map(|item| item.uri.clone())
        .collect();

    let uri_refs: Vec<&str> = uris.iter().map(String::as_str).collect();
    let body = match crate::convert::generate_clash_config(&uri_refs) {
        Ok(yaml) => yaml,
        Err(err) => {
            warn!(error = %err, "failed to generate clash config");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                format!("failed to generate clash config: {err}"),
            )
                .into_response();
        }
    };

    // Always return raw YAML — Clash Verge / Mihomo clients parse it directly
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store, no-cache, max-age=0"),
            (header::PRAGMA, "no-cache"),
            (header::EXPIRES, "0"),
        ],
        body,
    )
        .into_response()
}

async fn subscription_response(
    state: &HttpState,
    remote_addr: SocketAddr,
    token: Option<&str>,
    encoded: bool,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }

    let config = state.config.read().await.clone();
    let runtime = subscription_snapshot(&state.runtime).await;
    let mut body = subscription_body(&runtime, &config);

    if encoded {
        body = STANDARD.encode(body);
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store, no-cache, max-age=0"),
            (header::PRAGMA, "no-cache"),
            (header::EXPIRES, "0"),
        ],
        body,
    )
        .into_response()
}

async fn subscription_snapshot(state: &SharedState) -> RuntimeState {
    let started = Instant::now();
    loop {
        let runtime = state.read().await.clone();
        if !runtime.refreshing
            || runtime.ranked.iter().any(|item| item.reachable)
            || started.elapsed() >= SUBSCRIPTION_READY_WAIT
        {
            return runtime;
        }

        sleep(SUBSCRIPTION_READY_POLL).await;
    }
}

fn subscription_body(runtime: &RuntimeState, config: &RuntimeConfig) -> String {
    let mut body = runtime
        .ranked
        .iter()
        .filter(|item| item.reachable)
        .take(config.top_n)
        .map(|item| item.uri.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    if !body.is_empty() {
        body.push('\n');
    }
    body
}

async fn authorize(
    state: &HttpState,
    remote_addr: SocketAddr,
    token: Option<&str>,
) -> Result<(), Response> {
    let config = state.config.read().await;
    authorize_request(&config, remote_addr, token).map_err(AuthFailure::into_response)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct AuthFailure {
    status: StatusCode,
    message: &'static str,
}

impl AuthFailure {
    fn into_response(self) -> Response {
        (
            self.status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            self.message,
        )
            .into_response()
    }
}

fn authorize_request(
    config: &RuntimeConfig,
    remote_addr: SocketAddr,
    token: Option<&str>,
) -> Result<(), AuthFailure> {
    let local_request = remote_addr.ip().is_loopback();

    if !local_request {
        if !config.sharing_enabled {
            return Err(AuthFailure {
                status: StatusCode::FORBIDDEN,
                message: "LAN sharing is disabled\n",
            });
        }

        if config.require_token && token != Some(config.token.as_str()) {
            return Err(AuthFailure {
                status: StatusCode::UNAUTHORIZED,
                message: "missing or invalid token\n",
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::Arc};

    use axum::body::to_bytes;
    use tokio::sync::RwLock;

    use super::{HttpState, authorize_request, bind_error_context, subscription_response};
    use crate::{
        constants::{
            DEFAULT_ACCEPTED_STATUSES, DEFAULT_ACTIVE_TIMEOUT_MS, DEFAULT_BIND,
            DEFAULT_DOWNLOAD_BYTES_LIMIT, DEFAULT_ENCODED_SUBSCRIPTION, DEFAULT_FETCH_CONCURRENCY,
            DEFAULT_FETCH_TIMEOUT_MS, DEFAULT_MAX_SUBSCRIPTION_BYTES, DEFAULT_PRIORITIZE_STABILITY,
            DEFAULT_PROBE_CONCURRENCY, DEFAULT_REFRESH_SECONDS, DEFAULT_RETURN_CONFIGS_ASAP,
            DEFAULT_SCAN_ALL_CONFIGS, DEFAULT_STARTUP_TIMEOUT_MS, DEFAULT_TEST_URL, DEFAULT_TOP_N,
            LOCALHOST_IP,
        },
        model::{Endpoint, RankedConfig, RuntimeConfig, RuntimeState},
    };

    fn runtime_config(sharing_enabled: bool, require_token: bool) -> RuntimeConfig {
        RuntimeConfig {
            bind: DEFAULT_BIND.parse().expect("valid bind"),
            top_n: DEFAULT_TOP_N,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
            encoded_subscription: DEFAULT_ENCODED_SUBSCRIPTION,
            prioritize_stability: DEFAULT_PRIORITIZE_STABILITY,
            return_configs_asap: DEFAULT_RETURN_CONFIGS_ASAP,
            scan_all_configs: DEFAULT_SCAN_ALL_CONFIGS,
            fetch_timeout_ms: DEFAULT_FETCH_TIMEOUT_MS,
            fetch_concurrency: DEFAULT_FETCH_CONCURRENCY,
            max_subscription_bytes: DEFAULT_MAX_SUBSCRIPTION_BYTES,
            sharing_enabled,
            require_token,
            token: "secret".to_string(),
            probe_mode: "active".to_string(),
            speedtest_enabled: false,
            probe_concurrency: DEFAULT_PROBE_CONCURRENCY,
            probe_batch_size: None,
            active_timeout_ms: DEFAULT_ACTIVE_TIMEOUT_MS,
            startup_timeout_ms: DEFAULT_STARTUP_TIMEOUT_MS,
            test_url: DEFAULT_TEST_URL.to_string(),
            accepted_statuses: DEFAULT_ACCEPTED_STATUSES.to_vec(),
            download_bytes_limit: DEFAULT_DOWNLOAD_BYTES_LIMIT,
            subscription_count: 0,
            enabled_subscription_count: 0,
            proxy_enabled: false,
            proxy_port: 27910,
            proxy_discoverable: false,
        }
    }

    fn addr(value: &str) -> SocketAddr {
        value.parse().expect("valid socket address")
    }

    fn ranked(name: &str, uri: &str, reachable: bool) -> RankedConfig {
        RankedConfig {
            rank: 1,
            stability_count: 1,
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
            latency_ms: Some(100),
            http_status: Some(204),
            download_mbps: None,
            download_bytes: None,
            error: None,
            country_code: None,
        }
    }

    #[test]
    fn allows_local_request_when_lan_sharing_is_disabled() {
        let config = runtime_config(false, false);

        assert!(authorize_request(&config, addr(&format!("{LOCALHOST_IP}:50000")), None).is_ok());
    }

    #[test]
    fn blocks_lan_request_when_lan_sharing_is_disabled() {
        let config = runtime_config(false, false);
        let error = authorize_request(&config, addr("192.168.1.50:50000"), None)
            .expect_err("LAN request should be blocked");

        assert_eq!(error.status, axum::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn allows_lan_request_when_open_sharing_is_enabled() {
        let config = runtime_config(true, false);

        assert!(authorize_request(&config, addr("192.168.1.50:50000"), None).is_ok());
    }

    #[test]
    fn requires_token_for_lan_when_enabled() {
        let config = runtime_config(true, true);

        assert!(authorize_request(&config, addr("192.168.1.50:50000"), Some("wrong")).is_err());
        assert!(authorize_request(&config, addr("192.168.1.50:50000"), Some("secret")).is_ok());
    }

    #[test]
    fn bind_error_context_includes_configured_address() {
        let message = bind_error_context(addr(&format!("{LOCALHOST_IP}:27141")));

        assert!(message.contains("127.0.0.1:27141"));
    }

    #[tokio::test]
    async fn subscription_serves_live_ranked_state_during_refresh() {
        let runtime = RuntimeState {
            refreshing: true,
            ranked: vec![ranked(
                "live",
                "vless://live@example.com:443?security=tls#live",
                true,
            )],
            ..RuntimeState::default()
        };
        let config = runtime_config(false, false);
        let state = HttpState {
            runtime: Arc::new(RwLock::new(runtime)),
            config: Arc::new(RwLock::new(config)),
        };

        let response =
            subscription_response(&state, addr(&format!("{LOCALHOST_IP}:50000")), None, false)
                .await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(axum::http::header::CACHE_CONTROL),
            Some(&axum::http::HeaderValue::from_static(
                "no-store, no-cache, max-age=0"
            ))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert_eq!(
            std::str::from_utf8(&body).expect("body is utf-8"),
            "vless://live@example.com:443?security=tls#live\n"
        );
    }
}
