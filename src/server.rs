use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
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

    let listener = TcpListener::bind(bind).await?;
    info!(bind = %bind, "HTTP endpoint listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
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

    use super::authorize_request;
    use crate::model::RuntimeConfig;

    fn runtime_config(sharing_enabled: bool, require_token: bool) -> RuntimeConfig {
        RuntimeConfig {
            bind: "127.0.0.1:14127".parse().expect("valid bind"),
            top_n: 10,
            refresh_seconds: 300,
            encoded_subscription: true,
            sharing_enabled,
            require_token,
            token: "secret".to_string(),
            probe_mode: "active".to_string(),
        }
    }

    fn addr(value: &str) -> SocketAddr {
        value.parse().expect("valid socket address")
    }

    #[test]
    fn allows_local_request_when_lan_sharing_is_disabled() {
        let config = runtime_config(false, false);

        assert!(authorize_request(&config, addr("127.0.0.1:50000"), None).is_ok());
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
}
