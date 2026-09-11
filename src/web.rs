//! Local web dashboard served from the existing HTTP endpoint.
//!
//! Serves the self-contained `frontend/` assets (`GET /`, `/style.css`,
//! `/app.js`), a lightweight state summary (`GET /api/summary`) and a
//! server-sent-events live feed (`GET /api/events`). Every route reuses
//! [`crate::server`] auth, so the dashboard appears exactly when the
//! subscription endpoints appear: always on loopback, on LAN only with
//! `sharing.enabled` (+ token when `require_token`).
//!
//! Read-only in this phase: the bundled UI degrades action buttons to
//! guidance toasts until the mutation API lands. The feed pushes full
//! state on connect (`hello`), then diffs (`probe-delta`, `ranked`, `log`)
//! polled from shared runtime state — no changes to the refresh/ping loops.

use std::{
    convert::Infallible,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Json,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    config::should_include_token_in_url,
    model::{RankedConfig, RuntimeConfig},
    server::{AuthQuery, HttpState, SharedState, authorize, bearer_token},
};

/// Embedded dashboard shell (single file, no external references).
const DASHBOARD_HTML: &str = include_str!("../frontend/index.html");
/// Embedded dashboard stylesheet (CSS variables, no `url(` references).
const DASHBOARD_CSS: &str = include_str!("../frontend/style.css");
/// Embedded dashboard script (vanilla JS, same-origin API calls only).
const DASHBOARD_JS: &str = include_str!("../frontend/app.js");
/// Embedded QR encoder (single-file MIT library, no network use).
const DASHBOARD_QR: &str = include_str!("../frontend/qr.js");
/// Radar-mark favicon (static SVG, zero sensitivity — served ungated like
/// `/health` so tab icons never produce auth console noise on LAN instances).
const FAVICON_SVG: &str = concat!(
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\">",
    "<circle cx=\"32\" cy=\"32\" r=\"30\" fill=\"#0d1117\"/>",
    "<circle cx=\"32\" cy=\"32\" r=\"27\" fill=\"none\" stroke=\"#1f6feb\" stroke-width=\"4\"/>",
    "<circle cx=\"32\" cy=\"32\" r=\"15\" fill=\"none\" stroke=\"#54aeff\" stroke-width=\"3\"/>",
    "<line x1=\"32\" y1=\"32\" x2=\"52\" y2=\"14\" stroke=\"#3fb950\" stroke-width=\"4\" stroke-linecap=\"round\"/>",
    "<circle cx=\"32\" cy=\"32\" r=\"4\" fill=\"#3fb950\"/>",
    "</svg>",
);

/// Budget from PLAN.md §5: the whole initial payload must fit one loopback
/// exchange and stay usable on low-end phones.
#[cfg(test)]
const DASHBOARD_ASSET_BUDGET_BYTES: usize = 153_600;
/// Feed diff cadence: matches the dashboard's ≤1 Hz ranked refresh.
const FEED_TICK: Duration = Duration::from_secs(2);
/// SSE heartbeat so idle connections survive NATs/proxies.
const FEED_KEEP_ALIVE: Duration = Duration::from_secs(15);
/// Cap for log lines fanned out per tick (ring buffer stays bounded).
const FEED_MAX_LOG_FANOUT: usize = 10;
/// Cap concurrent SSE connections; extras get 503 and fall back to polling
/// so one dashboard tab farm can never stall the probe loops.
const FEED_MAX_CONNECTIONS: usize = 16;

/// Live SSE connections currently open (incremented before spawn, decremented
/// by [`FeedGuard`] when the feed task ends for any reason).
static ACTIVE_FEEDS: AtomicUsize = AtomicUsize::new(0);

/// `GET /` — dashboard shell. Same auth as every non-`/health` route.
pub async fn dashboard(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    static_response(
        &state,
        remote_addr,
        token,
        DASHBOARD_HTML,
        "text/html; charset=utf-8",
    )
    .await
}

/// `GET /style.css` — dashboard stylesheet.
pub async fn dashboard_css(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    static_response(
        &state,
        remote_addr,
        token,
        DASHBOARD_CSS,
        "text/css; charset=utf-8",
    )
    .await
}

/// `GET /app.js` — dashboard script.
pub async fn dashboard_js(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    static_response(
        &state,
        remote_addr,
        token,
        DASHBOARD_JS,
        "text/javascript; charset=utf-8",
    )
    .await
}

/// `GET /qr.js` — vendored offline QR encoder for per-config QR dialogs.
pub async fn dashboard_qr(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    static_response(
        &state,
        remote_addr,
        token,
        DASHBOARD_QR,
        "text/javascript; charset=utf-8",
    )
    .await
}

/// `GET /favicon.ico` — radar mark. Public by design (see [`FAVICON_SVG`]).
pub async fn favicon() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
    )
        .into_response()
}

async fn static_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
    body: &'static str,
    content_type: &'static str,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    // `no-store`: the bytes ride with the binary, so a fixed filename must
    // never outlive the release that served it in a browser cache.
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

/// Proxy slice of the summary (mirrors `RuntimeState` proxy fields).
#[derive(Debug, Clone, Serialize)]
struct ProxySummary {
    running: bool,
    active_config: Option<String>,
    active_uri: Option<String>,
    port: Option<u16>,
    discoverable: bool,
}

/// Lightweight dashboard state: everything in `RuntimeState` except the two
/// log buffers (the live feed carries new lines; resync uses `/results`),
/// plus whether a saved QR sheet exists (so the UI never probes a missing
/// file — failed probes log console errors).
#[derive(Debug, Clone, Serialize)]
struct StateSummary {
    refreshing: bool,
    pinging: bool,
    total_candidates: usize,
    tested_candidates: usize,
    reachable_candidates: usize,
    fetch_bytes: u64,
    speedtest_bytes: u64,
    last_error: Option<String>,
    last_refresh: Option<String>,
    /// Cycle timestamps: without these the dashboard sticks on "running" and
    /// "Last scan" never updates (the `refreshing` bool alone flips with no
    /// timestamp to anchor the idle countdown to).
    refresh_started_at: Option<String>,
    refresh_finished_at: Option<String>,
    refresh_duration_ms: Option<u128>,
    /// Fetch problems (distinct from probe failures); small by construction.
    fetch_errors: Vec<String>,
    /// Wall anchor of the last ping-deadline (re)arm (RFC3339 UTC): the
    /// dashboard counts `ping in …` down live from anchor + `ping_seconds`,
    /// exactly like fetch counts from `refresh_finished_at`.
    last_ping_at: Option<String>,
    proxy: ProxySummary,
    ranked: Vec<RankedConfig>,
    qr_available: bool,
    /// Server start (RFC3339 UTC) — dashboard "Running For" clock.
    started_at: String,
    /// Cycle intervals — dashboard "fetch in / ping in" countdowns.
    refresh_seconds: u64,
    ping_seconds: u64,
}

impl StateSummary {
    fn of(
        runtime: &crate::model::RuntimeState,
        config: &crate::model::RuntimeConfig,
        started_at: &str,
        qr_available: bool,
    ) -> Self {
        Self {
            refreshing: runtime.refreshing,
            pinging: runtime.pinging,
            total_candidates: runtime.total_candidates,
            tested_candidates: runtime.tested_candidates,
            reachable_candidates: runtime.reachable_candidates,
            fetch_bytes: runtime.fetch_bytes,
            speedtest_bytes: runtime.speedtest_bytes,
            last_error: runtime.last_error.clone(),
            last_refresh: runtime.last_refresh.clone(),
            refresh_started_at: runtime.refresh_started_at.clone(),
            refresh_finished_at: runtime.refresh_finished_at.clone(),
            refresh_duration_ms: runtime.refresh_duration_ms,
            fetch_errors: runtime.fetch_errors.clone(),
            last_ping_at: runtime.last_ping_at.clone(),
            proxy: ProxySummary {
                running: runtime.proxy_running,
                active_config: runtime.proxy_active_config.clone(),
                active_uri: runtime.proxy_active_uri.clone(),
                port: runtime.proxy_port,
                discoverable: runtime.proxy_discoverable,
            },
            ranked: runtime.ranked.clone(),
            qr_available,
            started_at: started_at.to_string(),
            refresh_seconds: config.refresh_seconds,
            ping_seconds: config.ping_seconds,
        }
    }
}

/// `GET /api/summary` — polling fallback for browsers without `EventSource`.
pub async fn api_summary(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    summary_response(&state, remote_addr, token).await
}

async fn summary_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    let runtime = state.runtime.read().await;
    let config = state.config.read().await;
    // Single metadata stat: cheap, and lets clients skip probing a missing file.
    let qr_available = state.data_dir.join(crate::qr::QR_IMAGE_FILE_NAME).exists();
    Json(StateSummary::of(
        &runtime,
        &config,
        &state.started,
        qr_available,
    ))
    .into_response()
}

/// Read-only subscription list served from shared watcher-synced state.
/// Response shape is exactly `{list: [{name, url, enabled, priority}],
/// dirty: false}`; the bundled UI already redacts full URLs client-side.
/// Same [`crate::server::authorize`] as every other route.
#[derive(Debug, Clone, Serialize)]
struct SubscriptionsResponse {
    list: Vec<crate::config::SubscriptionSource>,
    dirty: bool,
}

/// `GET /api/subscriptions` — live list from `configs.yaml` (read-only).
pub async fn api_subscriptions(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    subscriptions_response(&state, remote_addr, token).await
}

async fn subscriptions_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    let list = state.subscriptions.read().await.clone();
    Json(SubscriptionsResponse { list, dirty: false }).into_response()
}

/// Read-only settings table served from the live [`RuntimeConfig`].
/// Response shape is exactly `{groups: [{title, keys: [{key, value,
/// guide}]}], dirty: false}`; the bundled Settings tab already renders this
/// shape and degrades its click-to-edit PATCH to a guidance toast until the
/// mutation API lands. Same [`crate::server::authorize`] as every route.
///
/// Guides live here rather than reusing the TUI's `config_editor::guide`:
/// that module is private to `tui`, its `value()` reads `AppConfig` (not the
/// live `RuntimeConfig` the server holds), and several web keys (presence
/// markers, derived counts) have no TUI counterpart — reuse would couple
/// `web` to `tui` for no benefit.
#[derive(Debug, Clone, Serialize)]
struct ConfigKeyRow {
    key: String,
    value: String,
    guide: String,
}

/// One Settings card (`Connection`, `Fetch`, …).
#[derive(Debug, Clone, Serialize)]
struct ConfigGroup {
    title: String,
    keys: Vec<ConfigKeyRow>,
}

/// Grouped settings snapshot; always `dirty: false` (read-only for now).
#[derive(Debug, Clone, Serialize)]
struct ConfigResponse {
    groups: Vec<ConfigGroup>,
    dirty: bool,
}

impl ConfigResponse {
    fn of(config: &RuntimeConfig) -> Self {
        Self {
            groups: vec![
                connection_group(config),
                fetch_group(config),
                probe_group(config),
                sharing_group(config),
                proxy_group(config),
                maintenance_group(config),
            ],
            dirty: false,
        }
    }
}

fn config_row(key: &'static str, value: String, guide: &'static str) -> ConfigKeyRow {
    ConfigKeyRow {
        key: key.to_string(),
        value,
        guide: guide.to_string(),
    }
}

/// Unset optionals render as `"null"` (never blank: the UI shows `—` for
/// blank and that would look like a missing row).
fn optional_number(value: Option<usize>) -> String {
    value.map_or_else(|| "null".to_string(), |number| number.to_string())
}

/// Renders as `"[204, 200]"` so the row reads like the YAML list.
fn status_list(statuses: &[u16]) -> String {
    let inner = statuses
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Token presence marker — the secret itself must never enter a response.
fn token_presence(token: &str) -> &'static str {
    if should_include_token_in_url(token) {
        "set"
    } else {
        "empty"
    }
}

fn connection_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        title: "Connection".to_string(),
        keys: vec![
            config_row(
                "bind",
                config.bind.to_string(),
                "host:port the HTTP endpoint listens on",
            ),
            config_row(
                "top_n",
                config.top_n.to_string(),
                "configs kept in the ranked list",
            ),
            config_row(
                "refresh_seconds",
                config.refresh_seconds.to_string(),
                "seconds between refreshes",
            ),
            config_row(
                "ping_seconds",
                config.ping_seconds.to_string(),
                "seconds between re-pings; 0 disables",
            ),
            config_row(
                "encoded_subscription",
                config.encoded_subscription.to_string(),
                "true serves a base64 feed; false serves a raw list",
            ),
            config_row(
                "prioritize_stability",
                config.prioritize_stability.to_string(),
                "true favors repeat working configs over new wins",
            ),
            config_row(
                "return_configs_asap",
                config.return_configs_asap.to_string(),
                "true publishes working configs immediately",
            ),
            config_row(
                "scan_all_configs",
                config.scan_all_configs.to_string(),
                "true scans every config; false stops when enough work",
            ),
        ],
    }
}

fn fetch_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        title: "Fetch".to_string(),
        keys: vec![
            config_row(
                "fetch_timeout_ms",
                config.fetch_timeout_ms.to_string(),
                "fetch timeout in ms",
            ),
            config_row(
                "fetch_concurrency",
                config.fetch_concurrency.to_string(),
                "parallel fetch count",
            ),
            config_row(
                "max_subscription_bytes",
                config.max_subscription_bytes.to_string(),
                "max bytes accepted per subscription",
            ),
        ],
    }
}

fn probe_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        title: "Probe".to_string(),
        keys: vec![
            config_row(
                "probe.mode",
                config.probe_mode.clone(),
                "active uses sing-box; tcp is diagnostic only",
            ),
            config_row(
                "probe.concurrency",
                config.probe_concurrency.to_string(),
                "parallel probe count",
            ),
            config_row(
                "probe.batch_size",
                optional_number(config.probe_batch_size),
                "configs per probe batch; null selects automatically",
            ),
            config_row(
                "probe.active_timeout_ms",
                config.active_timeout_ms.to_string(),
                "active probe timeout in ms",
            ),
            config_row(
                "probe.startup_timeout_ms",
                config.startup_timeout_ms.to_string(),
                "probe startup timeout in ms",
            ),
            config_row(
                "probe.test_url",
                config.test_url.clone(),
                "URL used for the active probe",
            ),
            config_row(
                "probe.accepted_statuses",
                status_list(&config.accepted_statuses),
                "HTTP codes that count as working",
            ),
            config_row(
                "probe.download_bytes_limit",
                config.download_bytes_limit.to_string(),
                "speedtest byte limit",
            ),
            config_row(
                "probe.speedtest_enabled",
                config.speedtest_enabled.to_string(),
                "true runs a download speedtest",
            ),
        ],
    }
}

fn sharing_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        title: "Sharing".to_string(),
        keys: vec![
            config_row(
                "sharing.enabled",
                config.sharing_enabled.to_string(),
                "true exposes the subscription on the LAN",
            ),
            config_row(
                "sharing.require_token",
                config.require_token.to_string(),
                "true requires ?token= on LAN requests",
            ),
            config_row(
                "sharing.token",
                token_presence(&config.token).to_string(),
                "presence only; the secret never leaves the server",
            ),
        ],
    }
}

fn proxy_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        title: "Proxy".to_string(),
        keys: vec![
            config_row(
                "proxy.enabled",
                config.proxy_enabled.to_string(),
                "true runs the persistent local proxy",
            ),
            config_row(
                "proxy.port",
                config.proxy_port.to_string(),
                "local proxy port",
            ),
            config_row(
                "proxy.discoverable",
                config.proxy_discoverable.to_string(),
                "true binds LAN and opens the firewall",
            ),
        ],
    }
}

/// Derived counts (`configs.yaml` truth, read-only like everything here).
fn maintenance_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        title: "Maintenance".to_string(),
        keys: vec![
            config_row(
                "subscription_count",
                config.subscription_count.to_string(),
                "subscriptions in configs.yaml (read-only)",
            ),
            config_row(
                "enabled_subscription_count",
                config.enabled_subscription_count.to_string(),
                "enabled subscriptions (read-only)",
            ),
        ],
    }
}

/// `GET /api/config` — grouped live settings (read-only).
pub async fn api_config(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    config_response(&state, remote_addr, token).await
}

async fn config_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    let config = state.config.read().await.clone();
    Json(ConfigResponse::of(&config)).into_response()
}

/// User-enabled QR sheet: `POST /api/qr/generate` plans, renders, and saves
/// `QRCodes.jpg` into the local data dir via [`crate::qr::generate_and_save`]
/// (same planner + firewall check as the TUI row). The response is always
/// `{ok, message, skipped}`: `ok: false` with the planner's own skip reasons
/// when requirements are not met (sharing off, no LAN host, firewall
/// blocked, …). Served behind [`crate::server::authorize`] like every route;
/// the payload is a QR of LAN URLs the holder is already authorized to see
/// (same trust as `/subscription`). Rendering runs in `spawn_blocking` since
/// it is CPU/image work.
#[derive(Debug, Clone, Serialize)]
struct QrGenerateResponse {
    ok: bool,
    message: String,
    skipped: Vec<String>,
}

/// `POST /api/qr/generate` — render the QR sheet on demand.
pub async fn api_qr_generate(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    qr_generate_response(&state, remote_addr, token).await
}

async fn qr_generate_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    let config = state.config.read().await.clone();
    let data_dir = state.data_dir.clone();
    let generated = tokio::task::spawn_blocking(move || {
        match crate::qr::generate_and_save(&config, &data_dir, &|port| {
            crate::tui::firewall::allows_port(&data_dir, port)
        }) {
            Ok(outcome) => Ok((outcome.path, outcome.skipped)),
            Err(error) => {
                let skipped = crate::qr::plan_live(&config, &|port| {
                    crate::tui::firewall::allows_port(&data_dir, port)
                })
                .skipped;
                Err((format!("{error:#}"), skipped))
            }
        }
    })
    .await;
    match generated {
        Ok(Ok((path, skipped))) => {
            let mut message = format!("QR image saved: {}", path.display());
            if !skipped.is_empty() {
                message.push_str(" (skipped: ");
                message.push_str(&skipped.join("; "));
                message.push(')');
            }
            Json(QrGenerateResponse {
                ok: true,
                message,
                skipped,
            })
            .into_response()
        }
        Ok(Err((message, skipped))) => Json(QrGenerateResponse {
            ok: false,
            message,
            skipped,
        })
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("QR generation failed: {error}\n"),
        )
            .into_response(),
    }
}

/// `GET /api/qr.jpg` — serve the saved `QRCodes.jpg` bytes (`image/jpeg`).
/// Generation is an explicit user action, so the file is served whenever it
/// exists; staleness is the user's responsibility (same as the TUI, which
/// overwrites on each action). Missing file → 404 with a "generate first"
/// hint. Same [`crate::server::authorize`] as every other route.
pub async fn api_qr_image(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    qr_image_response(&state, remote_addr, token).await
}

async fn qr_image_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    let path = state.data_dir.join(crate::qr::QR_IMAGE_FILE_NAME);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "QR image not found; use POST /api/qr/generate first\n".to_string(),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("unable to read QR image: {error}\n"),
        )
            .into_response(),
    }
}

/// `GET /api/events` — server-sent-events live feed (`hello` snapshot, then
/// `probe-delta` / `ranked` / `log` diffs). Falls back to 503 when the feed
/// cap is hit; the UI then polls `/api/summary` instead.
pub async fn api_events(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    events_response(state.runtime.clone(), &state, remote_addr, token).await
}

async fn events_response(
    runtime: SharedState,
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Response {
    if let Err(response) = authorize(state, remote_addr, token).await {
        return response;
    }
    if ACTIVE_FEEDS.fetch_add(1, Ordering::Relaxed) >= FEED_MAX_CONNECTIONS {
        ACTIVE_FEEDS.fetch_sub(1, Ordering::Relaxed);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "too many live feeds; use GET /api/summary polling instead\n",
        )
            .into_response();
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(feed_task(runtime, tx));
    Sse::new(FeedStream { rx: Box::pin(rx) })
        .keep_alive(
            KeepAlive::new()
                .interval(FEED_KEEP_ALIVE)
                .text("dashboard ping"),
        )
        .into_response()
}

/// Decrements [`ACTIVE_FEEDS`] when the feed task ends for any reason.
struct FeedGuard;

impl Drop for FeedGuard {
    fn drop(&mut self) {
        ACTIVE_FEEDS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Change fingerprint driving `ranked` / `probe-delta` emission.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FeedFingerprint {
    tested: usize,
    working: usize,
    ranked_len: usize,
    refreshing: bool,
    pinging: bool,
    /// Cycle end must retrip the delta even when counts are unchanged (a
    /// finished refresh with identical tested/working would otherwise never
    /// push, leaving the UI stuck on "running"). Same for the ping anchor: a
    /// manual ping re-arms the full countdown with no count change.
    finished_at: Option<String>,
    ping_at: Option<String>,
}

impl FeedFingerprint {
    fn of(runtime: &crate::model::RuntimeState) -> Self {
        Self {
            tested: runtime.tested_candidates,
            working: runtime.reachable_candidates,
            ranked_len: runtime.ranked.len(),
            refreshing: runtime.refreshing,
            pinging: runtime.pinging,
            finished_at: runtime.refresh_finished_at.clone(),
            ping_at: runtime.last_ping_at.clone(),
        }
    }
}

fn hello_event(runtime: &crate::model::RuntimeState) -> Event {
    Event::default()
        .event("hello")
        .json_data(runtime)
        .unwrap_or_else(|_| Event::default().event("hello").data("{}"))
}

/// SSE framing breaks on raw newlines, so log lines travel single-line.
fn log_event(line: &str) -> Event {
    Event::default()
        .event("log")
        .data(line.replace(['\n', '\r'], " "))
}

async fn feed_task(runtime: SharedState, tx: UnboundedSender<Result<Event, Infallible>>) {
    let _guard = FeedGuard;
    let snapshot = runtime.read().await.clone();
    let mut fingerprint = FeedFingerprint::of(&snapshot);
    let mut last_bytes = snapshot.fetch_bytes;
    let mut log_len = snapshot.live_logs.len();
    if tx.send(Ok(hello_event(&snapshot))).is_err() {
        return;
    }
    loop {
        tokio::time::sleep(FEED_TICK).await;
        let snapshot = runtime.read().await.clone();
        let current = FeedFingerprint::of(&snapshot);
        let bytes_delta = snapshot.fetch_bytes.saturating_sub(last_bytes);
        last_bytes = snapshot.fetch_bytes;
        if current != fingerprint {
            fingerprint = current.clone();
            let delta = serde_json::json!({
                "tested": current.tested,
                "working": current.working,
                "bytes": bytes_delta,
                // Cycle state rides every delta: the fingerprint retrips on
                // refreshing/pinging/finished_at changes, and the UI needs the
                // new values (not just the fact of change) to leave "running".
                "refreshing": snapshot.refreshing,
                "pinging": snapshot.pinging,
                "last_refresh": snapshot.last_refresh,
                "refresh_started_at": snapshot.refresh_started_at,
                "refresh_finished_at": snapshot.refresh_finished_at,
                "refresh_duration_ms": snapshot.refresh_duration_ms,
                "fetch_errors": snapshot.fetch_errors,
                "last_ping_at": snapshot.last_ping_at,
            });
            if let Ok(event) = Event::default().event("probe-delta").json_data(delta)
                && tx.send(Ok(event)).is_err()
            {
                return;
            }
            if let Ok(event) = Event::default().event("ranked").json_data(&snapshot.ranked)
                && tx.send(Ok(event)).is_err()
            {
                return;
            }
        }
        let live_len = snapshot.live_logs.len();
        if live_len < log_len {
            log_len = live_len;
        } else if live_len > log_len {
            let fresh = &snapshot.live_logs[log_len..];
            let start = fresh.len().saturating_sub(FEED_MAX_LOG_FANOUT);
            for line in &fresh[start..] {
                if tx.send(Ok(log_event(line))).is_err() {
                    return;
                }
            }
            log_len = live_len;
        }
    }
}

/// Bridges the feed task's channel into axum's SSE body. The boxed receiver
/// keeps this implementable without pin-projection helpers.
struct FeedStream {
    rx: Pin<Box<UnboundedReceiver<Result<Event, Infallible>>>>,
}

impl futures_util::Stream for FeedStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.as_mut().poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        body::to_bytes,
        http::{HeaderValue, StatusCode, header},
    };
    use tokio::sync::RwLock;

    use super::{
        DASHBOARD_ASSET_BUDGET_BYTES, DASHBOARD_CSS, DASHBOARD_HTML, DASHBOARD_JS, DASHBOARD_QR,
        FAVICON_SVG, config_response, events_response, favicon, qr_generate_response,
        qr_image_response, static_response, subscriptions_response, summary_response,
    };
    use crate::{
        constants::{DEFAULT_BIND, LOCALHOST_IP},
        model::RuntimeState,
        server::HttpState,
    };

    fn addr(value: &str) -> SocketAddr {
        value.parse().expect("valid socket address")
    }

    fn loopback() -> SocketAddr {
        addr(&format!("{LOCALHOST_IP}:50000"))
    }

    fn lan() -> SocketAddr {
        addr("10.20.1.50:50000")
    }

    static TEST_DATA_DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_data_dir(case: &str) -> PathBuf {
        let id = TEST_DATA_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("v2raydar-web-{case}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp data dir creates");
        dir
    }

    fn http_state(runtime: RuntimeState, sharing_enabled: bool) -> HttpState {
        let config = crate::model::RuntimeConfig {
            bind: DEFAULT_BIND.parse().expect("valid bind"),
            top_n: 10,
            refresh_seconds: 900,
            ping_seconds: 300,
            encoded_subscription: true,
            prioritize_stability: true,
            return_configs_asap: false,
            scan_all_configs: false,
            fetch_timeout_ms: 30_000,
            fetch_concurrency: 8,
            max_subscription_bytes: 33_554_432,
            sharing_enabled,
            require_token: false,
            token: String::new(),
            probe_mode: "active".to_string(),
            speedtest_enabled: false,
            probe_concurrency: 16,
            probe_batch_size: None,
            active_timeout_ms: 30_000,
            startup_timeout_ms: 5_000,
            test_url: "https://www.gstatic.com/generate_204".to_string(),
            accepted_statuses: vec![204, 200],
            download_bytes_limit: 1_048_576,
            subscription_count: 0,
            enabled_subscription_count: 0,
            proxy_enabled: false,
            proxy_port: 27910,
            proxy_discoverable: false,
        };
        HttpState {
            runtime: Arc::new(RwLock::new(runtime)),
            config: Arc::new(RwLock::new(config)),
            subscriptions: Arc::new(RwLock::new(vec![
                crate::config::SubscriptionSource {
                    name: "demo".to_string(),
                    url: "https://example.com/sub.txt".to_string(),
                    enabled: true,
                    priority: 7,
                },
                crate::config::SubscriptionSource {
                    name: "paused".to_string(),
                    url: "https://example.com/other.txt".to_string(),
                    enabled: false,
                    priority: 42,
                },
            ])),
            data_dir: temp_data_dir("state"),
            started: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn dashboard_assets_fit_payload_budget_and_stay_self_contained() {
        let total =
            DASHBOARD_HTML.len() + DASHBOARD_CSS.len() + DASHBOARD_JS.len() + DASHBOARD_QR.len();
        assert!(
            total <= DASHBOARD_ASSET_BUDGET_BYTES,
            "dashboard payload {total} exceeds {DASHBOARD_ASSET_BUDGET_BYTES}"
        );
        assert!(DASHBOARD_HTML.contains("<!DOCTYPE html>"));
        assert!(DASHBOARD_HTML.contains("v2raydar-theme"));
        assert!(DASHBOARD_CSS.contains("prefers-color-scheme"));
        assert!(DASHBOARD_JS.contains("EventSource"));
        assert!(DASHBOARD_QR.contains("QREncode"));
        assert!(FAVICON_SVG.contains("<svg"));
        // Bare "http" substrings are fine (the UI builds loopback URLs like
        // "http://" + host at runtime); what must never appear is a remote
        // *reference* that would fetch off-device.
        for asset in [DASHBOARD_HTML, DASHBOARD_CSS, DASHBOARD_JS] {
            assert!(!asset.contains("innerHTML"), "DOM injection sink");
            for snippet in [
                "src=\"http",
                "href=\"http",
                "fetch(\"http",
                "EventSource(\"http",
                "url(http",
                "@import",
                "XMLHttpRequest",
            ] {
                assert!(!asset.contains(snippet), "remote reference: {snippet}");
            }
        }
        // qr.js keeps its upstream MIT header (with a homepage URL in a
        // comment), so it is checked for fetch-capable sinks instead.
        assert!(!DASHBOARD_QR.contains("innerHTML"), "DOM injection sink");
        for snippet in ["XMLHttpRequest", "fetch(", "src=\"http", "href=\"http"] {
            assert!(
                !DASHBOARD_QR.contains(snippet),
                "vendored encoder must not fetch: {snippet}"
            );
        }
    }

    #[tokio::test]
    async fn dashboard_qr_encoder_serves_on_loopback() {
        let state = http_state(RuntimeState::default(), false);
        let response = static_response(
            &state,
            loopback(),
            None,
            DASHBOARD_QR,
            "text/javascript; charset=utf-8",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/javascript; charset=utf-8"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert!(body.starts_with(b"/*"), "vendored encoder header");
    }

    #[tokio::test]
    async fn favicon_is_public_svg() {
        let response = favicon().await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("image/svg+xml"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert!(body.starts_with(b"<svg"), "svg favicon");
    }

    #[tokio::test]
    async fn dashboard_serves_shell_on_loopback() {
        let state = http_state(RuntimeState::default(), false);
        let response = static_response(
            &state,
            loopback(),
            None,
            DASHBOARD_HTML,
            "text/html; charset=utf-8",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/html; charset=utf-8"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = std::str::from_utf8(&body).expect("body is utf-8");
        assert!(text.contains("V2RayDAR Dashboard"));
    }

    #[tokio::test]
    async fn dashboard_blocks_lan_without_sharing() {
        let state = http_state(RuntimeState::default(), false);
        let response = static_response(
            &state,
            lan(),
            None,
            DASHBOARD_HTML,
            "text/html; charset=utf-8",
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn summary_hides_log_buffers_but_keeps_ranked() {
        let runtime = RuntimeState {
            refreshing: true,
            tested_candidates: 4,
            reachable_candidates: 2,
            logs: vec!["recent".to_string()],
            live_logs: vec!["live".to_string()],
            proxy_running: true,
            proxy_port: Some(27910),
            last_ping_at: Some("2026-09-11T15:49:50+00:00".to_string()),
            ..RuntimeState::default()
        };
        let state = http_state(runtime, false);
        let response = summary_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("summary is json");
        assert_eq!(value["refreshing"], serde_json::Value::Bool(true));
        assert_eq!(value["tested_candidates"], serde_json::Value::from(4));
        assert_eq!(value["proxy"]["port"], serde_json::Value::from(27910));
        assert!(value.get("logs").is_none(), "log buffer must not ship");
        assert!(
            value.get("live_logs").is_none(),
            "live log buffer must not ship"
        );
        assert!(value.get("ranked").is_some(), "ranked stays for resync");
        assert!(
            value.get("last_refresh").is_some(),
            "last scan stamp ships so idle state resolves"
        );
        assert!(
            value.get("refresh_started_at").is_some(),
            "cycle start ships for the running clock"
        );
        assert!(
            value.get("refresh_finished_at").is_some(),
            "cycle end ships so the UI leaves running"
        );
        assert!(
            value.get("refresh_duration_ms").is_some(),
            "cycle duration ships for the took line"
        );
        assert!(
            value.get("fetch_errors").is_some(),
            "fetch errors ship for the overview card"
        );
        assert!(
            value.get("last_ping_at").is_some(),
            "ping anchor ships for the live ping countdown"
        );
        assert_eq!(
            value["last_ping_at"],
            serde_json::Value::from("2026-09-11T15:49:50+00:00"),
            "ping anchor value crosses serialization untouched"
        );
        assert_eq!(
            value["started_at"],
            serde_json::Value::from("2026-01-01T00:00:00+00:00"),
            "server start ships for the uptime clock"
        );
        assert_eq!(
            value["refresh_seconds"],
            serde_json::Value::from(900),
            "refresh interval ships for the countdown"
        );
        assert_eq!(
            value["ping_seconds"],
            serde_json::Value::from(300),
            "ping interval ships for the countdown"
        );
        assert_eq!(
            value["qr_available"],
            serde_json::Value::Bool(false),
            "no saved sheet in the temp data dir"
        );
    }

    #[tokio::test]
    async fn summary_reports_saved_qr_sheet() {
        let state = http_state(RuntimeState::default(), false);
        std::fs::write(
            state.data_dir.join(crate::qr::QR_IMAGE_FILE_NAME),
            b"fake-jpeg",
        )
        .expect("seeded qr file writes");
        let response = summary_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("summary is json");
        assert_eq!(value["qr_available"], serde_json::Value::Bool(true));
    }

    #[tokio::test]
    async fn events_open_sse_stream_on_loopback() {
        let state = http_state(RuntimeState::default(), false);
        let response = events_response(state.runtime.clone(), &state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("sse content type")
            .to_str()
            .expect("content type is ascii");
        assert!(
            content_type.starts_with("text/event-stream"),
            "{content_type}"
        );
    }

    #[tokio::test]
    async fn events_reject_lan_without_sharing() {
        let state = http_state(RuntimeState::default(), false);
        let response = summary_response(&state, lan(), None).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn subscriptions_serve_live_list_on_loopback() {
        let state = http_state(RuntimeState::default(), false);
        let response = subscriptions_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("subscriptions is json");
        assert_eq!(value["dirty"], serde_json::Value::Bool(false));
        let list = value["list"].as_array().expect("list is an array");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["name"], serde_json::Value::from("demo"));
        assert_eq!(
            list[0]["url"],
            serde_json::Value::from("https://example.com/sub.txt")
        );
        assert_eq!(list[0]["enabled"], serde_json::Value::Bool(true));
        assert_eq!(list[0]["priority"], serde_json::Value::from(7));
        assert_eq!(list[1]["enabled"], serde_json::Value::Bool(false));
        assert_eq!(list[1]["priority"], serde_json::Value::from(42));
    }

    #[tokio::test]
    async fn subscriptions_reject_lan_without_sharing() {
        let state = http_state(RuntimeState::default(), false);
        let response = subscriptions_response(&state, lan(), None).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn config_serves_grouped_live_settings_without_token_leak() {
        let state = http_state(RuntimeState::default(), false);
        let secret = "config-endpoint-secret-token";
        state.config.write().await.token = secret.to_string();
        let response = config_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = std::str::from_utf8(&body).expect("body is utf-8");
        assert!(
            !text.contains(secret),
            "token value must never appear in the response"
        );
        let value: serde_json::Value = serde_json::from_str(text).expect("config is json");
        assert_eq!(value["dirty"], serde_json::Value::Bool(false));
        let groups = value["groups"].as_array().expect("groups is an array");
        assert!(!groups.is_empty(), "at least one settings group");
        let mut rows = 0;
        let mut seen_token = false;
        let mut seen_statuses = false;
        for group in groups {
            assert!(group["title"].is_string(), "every group has a string title");
            let keys = group["keys"].as_array().expect("keys is an array");
            for row in keys {
                assert!(row["key"].is_string(), "every row has a string key");
                assert!(row["value"].is_string(), "every row has a string value");
                assert!(row["guide"].is_string(), "every row has a string guide");
                rows += 1;
                if row["key"] == "sharing.token" {
                    seen_token = true;
                    assert_eq!(row["value"], serde_json::Value::from("set"));
                }
                if row["key"] == "probe.accepted_statuses" {
                    seen_statuses = true;
                    assert_eq!(row["value"], serde_json::Value::from("[204, 200]"));
                }
            }
        }
        assert!(rows > 0, "groups must carry key rows");
        assert!(seen_token, "sharing.token presence marker is served");
        assert!(seen_statuses, "probe.accepted_statuses list is served");
    }

    #[tokio::test]
    async fn config_marks_empty_token_without_leak() {
        let state = http_state(RuntimeState::default(), false);
        let response = config_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("config is json");
        let token = value["groups"]
            .as_array()
            .expect("groups is an array")
            .iter()
            .filter_map(|group| group["keys"].as_array())
            .flatten()
            .find(|row| row["key"] == "sharing.token")
            .expect("sharing.token row exists");
        assert_eq!(token["value"], serde_json::Value::from("empty"));
    }

    #[tokio::test]
    async fn config_rejects_lan_without_sharing() {
        let state = http_state(RuntimeState::default(), false);
        let response = config_response(&state, lan(), None).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn qr_image_missing_returns_404_with_generate_hint() {
        let state = http_state(RuntimeState::default(), false);
        let response = qr_image_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = std::str::from_utf8(&body).expect("body is utf-8");
        assert!(
            text.contains("generate first"),
            "missing QR must hint at generation: {text}"
        );
    }

    #[tokio::test]
    async fn qr_image_rejects_lan_without_sharing() {
        let state = http_state(RuntimeState::default(), false);
        let response = qr_image_response(&state, lan(), None).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn qr_generate_rejects_lan_without_sharing() {
        let state = http_state(RuntimeState::default(), false);
        let response = qr_generate_response(&state, lan(), None).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn qr_generate_reports_planner_skips_when_sharing_off() {
        // Deterministic without touching the LAN: sharing off + proxy off
        // means the planner yields zero cards, so generation returns
        // `{ok: false}` with the planner's own skip reasons.
        let state = http_state(RuntimeState::default(), false);
        let response = qr_generate_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("qr generate is json");
        assert_eq!(value["ok"], serde_json::Value::Bool(false));
        let skipped = value["skipped"].as_array().expect("skipped is an array");
        assert!(
            !skipped.is_empty(),
            "planner reasons must ride along: {value}"
        );
        let joined = skipped
            .iter()
            .filter_map(|reason| reason.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(joined.contains("sharing is off"), "{joined}");
        assert!(
            value["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "a human message rides along: {value}"
        );
    }

    #[tokio::test]
    async fn qr_image_serves_saved_bytes_when_file_exists() {
        let state = http_state(RuntimeState::default(), false);
        let bytes = vec![0xFF, 0xD8, 0xFF, 0x00, 0x01, 0xFF, 0xD9];
        std::fs::write(state.data_dir.join(crate::qr::QR_IMAGE_FILE_NAME), &bytes)
            .expect("saved QR writes");
        let response = qr_image_response(&state, loopback(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("image/jpeg"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert_eq!(body.to_vec(), bytes);
    }
}
