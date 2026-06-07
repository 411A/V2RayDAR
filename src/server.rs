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
use tokio::{net::TcpListener, sync::RwLock};
use tracing::info;

use crate::model::{RuntimeConfig, RuntimeState};

type SharedState = Arc<RwLock<RuntimeState>>;
type SharedConfig = Arc<RwLock<RuntimeConfig>>;

#[derive(Clone)]
struct HttpState {
    runtime: SharedState,
    config: SharedConfig,
}

pub async fn serve(bind: SocketAddr, runtime: SharedState, config: SharedConfig) -> Result<()> {
    let state = HttpState { runtime, config };

    let router = Router::new()
        .route("/health", get(health))
        .route("/results", get(results))
        .route("/subscription", get(subscription))
        .route("/subscription.txt", get(subscription_txt))
        .with_state(state);

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
    let runtime = state.runtime.read().await;
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

    if encoded {
        body = STANDARD.encode(body);
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
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
    use std::net::SocketAddr;

    use super::{authorize_request, bind_error_context};
    use crate::{
        constants::{
            DEFAULT_ACCEPTED_STATUSES, DEFAULT_ACTIVE_TIMEOUT_MS, DEFAULT_BIND,
            DEFAULT_DOWNLOAD_BYTES_LIMIT, DEFAULT_ENCODED_SUBSCRIPTION, DEFAULT_FETCH_CONCURRENCY,
            DEFAULT_FETCH_TIMEOUT_MS, DEFAULT_MAX_SUBSCRIPTION_BYTES, DEFAULT_PRIORITIZE_STABILITY,
            DEFAULT_PROBE_CONCURRENCY, DEFAULT_REFRESH_SECONDS, DEFAULT_STARTUP_TIMEOUT_MS,
            DEFAULT_TEST_URL, DEFAULT_TOP_N, LOCALHOST_IP,
        },
        model::RuntimeConfig,
    };

    fn runtime_config(sharing_enabled: bool, require_token: bool) -> RuntimeConfig {
        RuntimeConfig {
            bind: DEFAULT_BIND.parse().expect("valid bind"),
            top_n: DEFAULT_TOP_N,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
            encoded_subscription: DEFAULT_ENCODED_SUBSCRIPTION,
            prioritize_stability: DEFAULT_PRIORITIZE_STABILITY,
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
        }
    }

    fn addr(value: &str) -> SocketAddr {
        value.parse().expect("valid socket address")
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
}
