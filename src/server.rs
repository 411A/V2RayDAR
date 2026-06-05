use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
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
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok\n"
}

async fn results(State(state): State<HttpState>) -> Json<RuntimeState> {
    Json(state.runtime.read().await.clone())
}

async fn subscription(State(state): State<HttpState>) -> Response {
    let encoded = state.config.read().await.encoded_subscription;
    subscription_response(&state, encoded).await
}

async fn subscription_txt(State(state): State<HttpState>) -> Response {
    subscription_response(&state, false).await
}

async fn subscription_response(state: &HttpState, encoded: bool) -> Response {
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
