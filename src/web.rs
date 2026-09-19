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
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    config::should_include_token_in_url,
    model::{RankedConfig, RuntimeConfig},
    network,
    server::{AuthQuery, HttpState, SharedState, authorize, bearer_token},
};

/// Embedded dashboard shell (single file, no external references).
const DASHBOARD_HTML: &str = include_str!("../frontend/index.html");
/// Embedded dashboard stylesheet (CSS variables, no `url(` references).
const DASHBOARD_CSS: &str = include_str!("../frontend/style.css");
/// Embedded dashboard script (vanilla JS, same-origin API calls only).
const DASHBOARD_JS: &str = include_str!("../frontend/app.js");
/// Embedded dashboard strings (single i18n table, English default).
const DASHBOARD_I18N: &str = include_str!("../frontend/i18n.js");
/// Language flags served as real files (`/assets/*.svg`), so a flag can be
/// swapped by replacing one SVG — no code or markup change needed.
const FLAG_GB: &str = include_str!("../frontend/assets/GB.svg");
const FLAG_IR: &str = include_str!("../frontend/assets/IR.svg");
const FLAG_CN: &str = include_str!("../frontend/assets/CN.svg");
const FLAG_FR: &str = include_str!("../frontend/assets/FR.svg");
const FLAG_RU: &str = include_str!("../frontend/assets/RU.svg");
const ICON_POWER: &str = include_str!("../frontend/assets/power-off-svgrepo-com.svg");
/// Vendored Vazirmatn (variable 100–900) in Google's unicode subsets:
/// Arabic covers Persian script, Latin covers digits and Latin runs.
const FONT_VAZIR_ARABIC: &[u8] = include_bytes!("../frontend/assets/vazirmatn-arabic.woff2");
const FONT_VAZIR_LATIN: &[u8] = include_bytes!("../frontend/assets/vazirmatn-latin.woff2");
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

/// `GET /i18n.js` — dashboard strings (must load before `/app.js`).
pub async fn dashboard_i18n(
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
        DASHBOARD_I18N,
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

/// `GET /assets/{file}` — language flag SVGs, the power icon, and the
/// vendored Vazirmatn woff2 files. Whitelisted by name (never a raw path:
/// no traversal, no surprises); same auth as the dashboard shell.
pub async fn dashboard_flag(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Path(file): Path<String>,
) -> Response {
    let (body, mime): (&'static [u8], &'static str) = match file.as_str() {
        "GB.svg" => (FLAG_GB.as_bytes(), "image/svg+xml"),
        "IR.svg" => (FLAG_IR.as_bytes(), "image/svg+xml"),
        "CN.svg" => (FLAG_CN.as_bytes(), "image/svg+xml"),
        "FR.svg" => (FLAG_FR.as_bytes(), "image/svg+xml"),
        "RU.svg" => (FLAG_RU.as_bytes(), "image/svg+xml"),
        "power-off-svgrepo-com.svg" => (ICON_POWER.as_bytes(), "image/svg+xml"),
        "vazirmatn-arabic.woff2" => (FONT_VAZIR_ARABIC, "font/woff2"),
        "vazirmatn-latin.woff2" => (FONT_VAZIR_LATIN, "font/woff2"),
        _ => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "unknown asset",
            )
                .into_response();
        }
    };
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    static_bytes_response(&state, remote_addr, token, body, mime).await
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

async fn static_bytes_response(
    state: &HttpState,
    remote_addr: std::net::SocketAddr,
    token: Option<&str>,
    body: &'static [u8],
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

/// `GET /api/subscriptions` — live list from shared state (read-only).
pub async fn api_subscriptions(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    subscriptions_response(&state, remote_addr, token).await
}

/// Ready-to-copy LAN share URL: the endpoint key renders client-side
/// (`subscription`, `subscription_txt`, `mihomo`) so no locale gains strings.
#[derive(Debug, Clone, Serialize)]
struct ShareUrl {
    key: &'static str,
    url: String,
}

/// Ready-to-copy LAN share URLs (`GET /api/share-urls`): the same tokenized
/// links the QR sheet encodes, for dashboards where the raw token stays
/// masked — token protection would otherwise hand out a secret nobody can
/// retrieve. Only reachable by loopback owners and token-holding LAN viewers
/// (same authorization as every route), so nothing new leaks. Empty while
/// LAN sharing is off or no LAN host is found; the dashboard then falls back
/// to its origin-based list.
#[derive(Debug, Clone, Serialize)]
struct ShareUrlsResponse {
    urls: Vec<ShareUrl>,
    sharing_enabled: bool,
}

fn share_urls(config: &RuntimeConfig, host: &str) -> Vec<ShareUrl> {
    vec![
        ShareUrl {
            key: "subscription",
            url: config.subscription_url(host, false),
        },
        ShareUrl {
            key: "subscription_txt",
            url: config.subscription_url(host, true),
        },
        ShareUrl {
            key: "mihomo",
            url: config.mihomo_url(host),
        },
    ]
}

/// `GET /api/share-urls` — tokenized LAN share URLs, ready to copy.
pub async fn api_share_urls(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    // Clone out from under the snapshot lock: discovery below is blocking
    // network I/O and must never hold the guard.
    let config = state.config.read().await.clone();
    if config.sharing_enabled {
        let hosts = network::discoverable_hosts(&config);
        let urls = hosts
            .first()
            .map_or_else(Vec::new, |host| share_urls(&config, host));
        return Json(ShareUrlsResponse {
            urls,
            sharing_enabled: true,
        })
        .into_response();
    }
    Json(ShareUrlsResponse {
        urls: Vec::new(),
        sharing_enabled: false,
    })
    .into_response()
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
/// Response shape is exactly `{groups: [{id, title, keys: [{key, value,
/// guide, kind, options}]}], dirty: false}`; the bundled Settings tab
/// renders typed controls from `kind` (`bool` switch, `int`/`text`/`list`
/// editor, `choice` select over `options`, `secret` setter, `readonly`
/// static) and degrades unknown kinds to the text editor. Same
/// [`crate::server::authorize`] as every route.
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
    /// Control hint: `bool`, `int`, `text`, `choice`, `list`, `secret`,
    /// or `readonly`. The dashboard translates names/guides client-side and
    /// falls back to the text editor when `kind` is absent (old servers).
    kind: &'static str,
    /// Allowed values for `choice` (empty otherwise).
    options: Vec<&'static str>,
}

/// One Settings card (`Connection`, `Fetch`, …); `id` is the stable
/// client key for the translated group title.
#[derive(Debug, Clone, Serialize)]
struct ConfigGroup {
    id: &'static str,
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
                advanced_group(config),
            ],
            dirty: false,
        }
    }
}

fn config_row(key: &'static str, value: String, guide: &'static str) -> ConfigKeyRow {
    config_row_kind(key, value, guide, "text", &[])
}

fn config_row_kind(
    key: &'static str,
    value: String,
    guide: &'static str,
    kind: &'static str,
    options: &[&'static str],
) -> ConfigKeyRow {
    ConfigKeyRow {
        key: key.to_string(),
        value,
        guide: guide.to_string(),
        kind,
        options: options.to_vec(),
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

/// Token presence marker for the polled settings table — the secret itself
/// must never enter a response here (explicit reveal lives behind
/// `GET /api/config/token`, fetched only on Show).
fn token_presence(token: &str) -> &'static str {
    if should_include_token_in_url(token) {
        "set"
    } else {
        "empty"
    }
}

fn connection_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        id: "connection",
        title: "Connection".to_string(),
        keys: vec![
            config_row(
                "bind",
                config.bind.to_string(),
                "host:port the HTTP endpoint listens on",
            ),
            config_row_kind(
                "top_n",
                config.top_n.to_string(),
                "configs kept in the ranked list",
                "int",
                &[],
            ),
            config_row_kind(
                "refresh_seconds",
                config.refresh_seconds.to_string(),
                "seconds between refreshes",
                "int",
                &[],
            ),
            config_row_kind(
                "ping_seconds",
                config.ping_seconds.to_string(),
                "seconds between re-pings; 0 disables",
                "int",
                &[],
            ),
            config_row_kind(
                "encoded_subscription",
                config.encoded_subscription.to_string(),
                "true serves a base64 feed; false serves a raw list",
                "bool",
                &[],
            ),
            config_row_kind(
                "prioritize_stability",
                config.prioritize_stability.to_string(),
                "true favors repeat working configs over new wins",
                "bool",
                &[],
            ),
            config_row_kind(
                "return_configs_asap",
                config.return_configs_asap.to_string(),
                "true publishes working configs immediately",
                "bool",
                &[],
            ),
            config_row_kind(
                "scan_all_configs",
                config.scan_all_configs.to_string(),
                "true scans every config; false stops when enough work",
                "bool",
                &[],
            ),
            config_row_kind(
                "use_cache_only",
                config.use_cache_only.to_string(),
                "true skips downloads and reuses cached subscriptions",
                "bool",
                &[],
            ),
            config_row(
                "emergency_config",
                config.emergency_config.clone().unwrap_or_default(),
                "single proxy config URI used to retry failed subscription fetches; empty clears it",
            ),
        ],
    }
}

fn fetch_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        id: "fetch",
        title: "Fetch".to_string(),
        keys: vec![
            config_row_kind(
                "fetch_timeout_ms",
                config.fetch_timeout_ms.to_string(),
                "fetch timeout in ms",
                "int",
                &[],
            ),
            config_row_kind(
                "fetch_concurrency",
                config.fetch_concurrency.to_string(),
                "parallel fetch count",
                "int",
                &[],
            ),
            config_row_kind(
                "max_subscription_bytes",
                config.max_subscription_bytes.to_string(),
                "max bytes accepted per subscription",
                "int",
                &[],
            ),
        ],
    }
}

fn probe_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        id: "probe",
        title: "Probe".to_string(),
        keys: vec![
            config_row_kind(
                "probe.mode",
                config.probe_mode.clone(),
                "active uses sing-box; tcp is diagnostic only",
                "choice",
                &["active", "tcp"],
            ),
            config_row(
                "probe.sing_box_path",
                config.sing_box_path.clone(),
                "full path to the sing-box executable; empty auto-detects",
            ),
            config_row_kind(
                "probe.connect_timeout_ms",
                config.connect_timeout_ms.to_string(),
                "connect timeout in ms",
                "int",
                &[],
            ),
            config_row_kind(
                "probe.concurrency",
                config.probe_concurrency.to_string(),
                "parallel probe count",
                "int",
                &[],
            ),
            config_row(
                "probe.batch_size",
                optional_number(config.probe_batch_size),
                "configs per probe batch; null selects automatically",
            ),
            config_row(
                "probe.process_concurrency",
                optional_number(config.probe_process_concurrency),
                "parallel sing-box processes; null selects automatically",
            ),
            config_row_kind(
                "probe.active_timeout_ms",
                config.active_timeout_ms.to_string(),
                "active probe timeout in ms",
                "int",
                &[],
            ),
            config_row_kind(
                "probe.startup_timeout_ms",
                config.startup_timeout_ms.to_string(),
                "probe startup timeout in ms",
                "int",
                &[],
            ),
            config_row(
                "probe.test_url",
                config.test_url.clone(),
                "URL used for the active probe",
            ),
            config_row_kind(
                "probe.accepted_statuses",
                status_list(&config.accepted_statuses),
                "HTTP codes that count as working",
                "list",
                &[],
            ),
            config_row_kind(
                "probe.download_bytes_limit",
                config.download_bytes_limit.to_string(),
                "speedtest byte limit",
                "int",
                &[],
            ),
            config_row(
                "probe.download_url",
                config
                    .download_url
                    .clone()
                    .unwrap_or_else(|| "off".to_string()),
                "speedtest download link; off disables it",
            ),
            config_row_kind(
                "probe.speedtest_enabled",
                config.speedtest_enabled.to_string(),
                "on when probe.download_url is set",
                "readonly",
                &[],
            ),
        ],
    }
}

fn sharing_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        id: "sharing",
        title: "Sharing".to_string(),
        keys: vec![
            config_row_kind(
                "sharing.enabled",
                config.sharing_enabled.to_string(),
                "true exposes the subscription on the LAN",
                "bool",
                &[],
            ),
            config_row_kind(
                "sharing.require_token",
                config.require_token.to_string(),
                "true requires ?token= on LAN requests",
                "bool",
                &[],
            ),
            config_row_kind(
                "sharing.token",
                token_presence(&config.token).to_string(),
                "presence only; press Show to reveal it once",
                "secret",
                &[],
            ),
        ],
    }
}

fn proxy_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        id: "proxy",
        title: "Proxy".to_string(),
        keys: vec![
            config_row_kind(
                "proxy.enabled",
                config.proxy_enabled.to_string(),
                "true runs the persistent local proxy",
                "bool",
                &[],
            ),
            config_row_kind(
                "proxy.port",
                config.proxy_port.to_string(),
                "local proxy port",
                "int",
                &[],
            ),
            config_row_kind(
                "proxy.discoverable",
                config.proxy_discoverable.to_string(),
                "true binds LAN and opens the firewall",
                "bool",
                &[],
            ),
            config_row_kind(
                "proxy.rotating_proxy",
                config.rotating_proxy.to_string(),
                "true switches to the lowest ping; false keeps the current config",
                "bool",
                &[],
            ),
            config_row(
                "proxy.health_check_url",
                config.health_check_url.clone(),
                "URL fetched through the running proxy to verify real traffic (not the probe's test URL)",
            ),
            config_row_kind(
                "proxy.health_check_interval_seconds",
                config.health_check_interval_seconds.to_string(),
                "seconds between checks of the running proxy for failover (not the ranked-list re-ping)",
                "int",
                &[],
            ),
        ],
    }
}

fn advanced_group(config: &RuntimeConfig) -> ConfigGroup {
    ConfigGroup {
        id: "advanced",
        title: "Advanced".to_string(),
        keys: vec![
            config_row_kind(
                "clean_offlines_after_days",
                config.clean_offlines_after_days.to_string(),
                "days before offline configs are removed from the database",
                "int",
                &[],
            ),
            config_row(
                "geoip_db_path",
                config.geoip_db_path.clone().unwrap_or_default(),
                "custom GeoIP database file or directory; empty uses the built-in one (takes effect after restart)",
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

/// Small `{ok, status, dirty}` envelope shared by every mutation endpoint.
/// `status` reuses the TUI footer wording verbatim so docs transfer; `dirty`
/// is always `false` — the dashboard persists immediately (no TUI-style
/// two-step save), so the Save bar never arms.
#[derive(Debug, Clone, Serialize)]
struct MutationResult {
    ok: bool,
    status: String,
    dirty: bool,
    /// Machine flag for the dashboard (absent = plain message). Currently
    /// only `FIREWALL_ELEVATION_CODE`, paired with `os`, so the frontend can
    /// pop the run-as-admin guide in the user's language.
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    /// Server OS (`std::env::consts::OS`) for the admin guide. The browser
    /// may run on another LAN device, so the client must not guess from UA.
    #[serde(skip_serializing_if = "Option::is_none")]
    os: Option<&'static str>,
}

/// Firewall rule change failed while toggling proxy mode or sharing.
const FIREWALL_ELEVATION_CODE: &str = "firewall_elevation";

/// Setting saved, but refresh-relevant data changed: the loop does NOT
/// re-fetch at once (a settings tweak must never cost an extra subscription
/// fetch), so the dashboard pops a "takes effect on the next refresh — or
/// press Refresh" notice from this machine flag. Only PATCH/reset responses
/// whose fingerprint actually moved carry it; plain saves stay flag-free.
const APPLIES_NEXT_CYCLE_CODE: &str = "applies_next_cycle";

fn mutation_ok(status: impl Into<String>) -> Response {
    Json(MutationResult {
        ok: true,
        status: status.into(),
        dirty: false,
        code: None,
        os: None,
    })
    .into_response()
}

/// Setting saved with refresh-relevant data changed: same 200 + `ok` as a
/// plain success (existing tests and old frontends only look at those),
/// plus the machine flag new frontends use for the next-cycle notice.
fn mutation_applies_next_cycle(status: String, applies_next_cycle: bool) -> Response {
    Json(MutationResult {
        ok: true,
        status,
        dirty: false,
        code: applies_next_cycle.then_some(APPLIES_NEXT_CYCLE_CODE),
        os: None,
    })
    .into_response()
}

/// Setting saved, firewall rule failed: same 200 + `ok` as a plain success
/// (existing tests and old frontends only look at those), plus the machine
/// flag new frontends use to pop the run-as-admin guide. The failure is
/// almost always missing elevation — `netsh`/`ufw`/`firewall-cmd` refuse
/// without admin/root — so the guide tells the user to stop the server,
/// restart it elevated, and retry.
fn mutation_firewall_elevation(firewall_message: String) -> Response {
    Json(MutationResult {
        ok: true,
        status: firewall_message,
        dirty: false,
        code: Some(FIREWALL_ELEVATION_CODE),
        os: Some(std::env::consts::OS),
    })
    .into_response()
}

fn mutation_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(MutationResult {
            ok: false,
            status: message.into(),
            dirty: false,
            code: None,
            os: None,
        }),
    )
        .into_response()
}

/// `POST /api/refresh` — queue one manual refresh (re-fetch subscriptions).
/// Mirrors the TUI `trigger_refresh`: refused with `409` while a refresh is
/// already running (a running ping is preempted instead — never a refusal).
/// The loops coalesce duplicates, so this sends at most one trigger.
pub async fn api_refresh(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.runtime.read().await.refreshing {
        return mutation_error(StatusCode::CONFLICT, "Refresh already running");
    }
    state.refresh_tx.as_ref().map_or_else(
        || {
            mutation_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Manual refresh unavailable",
            )
        },
        |tx| {
            let _ = tx.send(());
            mutation_ok("Manual refresh started")
        },
    )
}

/// `POST /api/shutdown` — stop the whole instance (same as Ctrl+C: proxy,
/// loops and HTTP all go down; manual start required). Responds 200 first,
/// then exits after a short grace so the confirmation reaches the browser.
/// Never exits the test runner (`cfg!(test)` skips the spawn).
pub async fn api_shutdown(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if !cfg!(test) {
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            std::process::exit(0);
        });
    }
    mutation_ok("Server stopping")
}

/// `POST /api/ping` — queue one manual re-ping of the cached configs.
/// Mirrors the TUI `trigger_ping`: refused with `409` while any cycle (fetch
/// or ping) is running so results can't be overwritten mid-flight.
pub async fn api_ping(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    {
        let runtime = state.runtime.read().await;
        if runtime.refreshing || runtime.pinging {
            return mutation_error(StatusCode::CONFLICT, "A cycle is already running");
        }
    }
    state.ping_tx.as_ref().map_or_else(
        || mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Manual ping unavailable"),
        |tx| {
            let _ = tx.send(());
            mutation_ok("Manual ping started")
        },
    )
}

/// New subscription payload (`POST /api/subscriptions`).
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionAdd {
    url: String,
    name: String,
    #[serde(default = "default_sub_priority")]
    priority: u32,
    #[serde(default = "default_sub_enabled")]
    enabled: bool,
}

const fn default_sub_priority() -> u32 {
    100
}

const fn default_sub_enabled() -> bool {
    true
}

/// Partial subscription edit (`PATCH /api/subscriptions/:index`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubscriptionPatch {
    url: Option<String>,
    name: Option<String>,
    priority: Option<u32>,
    enabled: Option<bool>,
}

/// Settings edit (`PATCH /api/config`).
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigPatch {
    key: String,
    value: String,
}

/// Proxy pin/unpin (`POST /api/proxy/select`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProxySelect {
    uri: Option<String>,
}

/// Cache clean confirm (`POST /api/cache/clean`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CacheClean {
    #[serde(default)]
    confirm: String,
}

// `axum::Response` is large by framework design; these helpers only run on
// mutation paths, so boxing would add indirection for no benefit.
#[allow(clippy::result_large_err)]
fn validate_subscription(url: &str, name: &str) -> Result<(), Response> {
    if name.trim().is_empty() {
        return Err(mutation_error(
            StatusCode::BAD_REQUEST,
            "Subscription name cannot be empty",
        ));
    }
    if name.len() > 120 {
        return Err(mutation_error(
            StatusCode::BAD_REQUEST,
            "Subscription name is too long (max 120)",
        ));
    }
    if !crate::config::is_allowed_subscription_url(url) {
        return Err(mutation_error(
            StatusCode::BAD_REQUEST,
            "Unsupported subscription URL scheme (use http(s), data, file, or a local path)",
        ));
    }
    Ok(())
}

/// Subscription URLs are unique: refuse a URL that already lives at another
/// row, naming its 1-based index so the caller can point at the twin (the
/// dashboard shows the same number as the row's priority).
fn duplicate_subscription_rejection(
    subscriptions: &[crate::config::SubscriptionSource],
    url: &str,
    except: Option<usize>,
) -> Option<Response> {
    crate::config::find_duplicate_subscription_url(subscriptions, url, except).map(|at| {
        mutation_error(
            StatusCode::CONFLICT,
            format!(
                "Subscription URL already exists at index {}",
                at.saturating_add(1)
            ),
        )
    })
}

/// Load the stored config: every mutation starts from the database, never
/// from a file. Rusqlite takes a mutex, so this runs off the async runtime
/// like persistence below.
#[allow(clippy::result_large_err)]
async fn load_db_config(state: &HttpState) -> Result<crate::config::AppConfig, Response> {
    let Some(db) = state.database.clone() else {
        return Err(mutation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Settings database unavailable",
        ));
    };
    tokio::task::spawn_blocking(move || crate::settings::load_app_config(&db))
        .await
        .map_err(|error| {
            mutation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Load task failed: {error}"),
            )
        })?
        .map_err(|error| {
            mutation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to read settings: {error}"),
            )
        })
}

/// Persist `cfg` to the database and push it live (broadcast + shared
/// snapshots) so the dashboard sees the change immediately.
/// Immediate-save: no dirty flag, ever.
#[allow(clippy::result_large_err)]
async fn persist_config(state: &HttpState, cfg: &crate::config::AppConfig) -> Result<(), Response> {
    let Some(db) = state.database.clone() else {
        return Err(mutation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Settings database unavailable",
        ));
    };
    let snapshot = cfg.clone();
    tokio::task::spawn_blocking(move || crate::settings::save_app_config(&db, &snapshot))
        .await
        .map_err(|error| {
            mutation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Save task failed: {error}"),
            )
        })?
        .map_err(|error| {
            mutation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to save settings: {error}"),
            )
        })?;
    *state.subscriptions.write().await = cfg.subscriptions.clone();
    *state.config.write().await = crate::model::RuntimeConfig::from(cfg);
    if let Some(tx) = state.config_tx.as_ref() {
        let _ = tx.send(cfg.clone());
    }
    Ok(())
}

/// Push an in-memory-only config (proxy select): live broadcast + runtime
/// snapshot, never touches the file — same as the TUI `Enter` row action.
async fn push_live_config(state: &HttpState, cfg: &crate::config::AppConfig) {
    *state.config.write().await = crate::model::RuntimeConfig::from(cfg);
    if let Some(tx) = state.config_tx.as_ref() {
        let _ = tx.send(cfg.clone());
    }
}

/// `POST /api/subscriptions` — add one subscription (immediate-save).
pub async fn api_subscriptions_add(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<SubscriptionAdd>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Subscription API unavailable",
        );
    }
    let url = body.url.trim().to_string();
    let name = body.name.trim().to_string();
    if let Err(response) = validate_subscription(&url, &name) {
        return response;
    }
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    if let Some(response) = duplicate_subscription_rejection(&cfg.subscriptions, &url, None) {
        return response;
    }
    cfg.subscriptions.push(crate::config::SubscriptionSource {
        name: name.clone(),
        url,
        enabled: body.enabled,
        priority: body.priority,
    });
    // Priority is the list position: the newcomer lands on its rank at once.
    let last = cfg.subscriptions.len().saturating_sub(1);
    crate::config::move_subscription_to_rank(&mut cfg.subscriptions, last, body.priority);
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    mutation_ok(format!("Added {name}"))
}

/// `PATCH /api/subscriptions/:index` — edit one subscription (immediate-save).
pub async fn api_subscriptions_patch(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Path(index): Path<usize>,
    Json(body): Json<SubscriptionPatch>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Subscription API unavailable",
        );
    }
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    let Some(entry) = cfg.subscriptions.get(index) else {
        return mutation_error(StatusCode::NOT_FOUND, "No subscription at that index");
    };
    // An echo-save of the same row is fine; pointing the row at a sibling's
    // URL is refused with the sibling's index. Checked before the mutable
    // borrow below.
    let next_url = body
        .url
        .as_deref()
        .map_or_else(|| entry.url.clone(), |url| url.trim().to_string());
    if let Some(response) =
        duplicate_subscription_rejection(&cfg.subscriptions, &next_url, Some(index))
    {
        return response;
    }
    let Some(entry) = cfg.subscriptions.get_mut(index) else {
        return mutation_error(StatusCode::NOT_FOUND, "No subscription at that index");
    };
    let mut next = entry.clone();
    if let Some(url) = body.url {
        next.url = url.trim().to_string();
    }
    if let Some(name) = body.name {
        next.name = name.trim().to_string();
    }
    if let Some(priority) = body.priority {
        next.priority = priority;
    }
    if let Some(enabled) = body.enabled {
        next.enabled = enabled;
    }
    if let Err(response) = validate_subscription(&next.url, &next.name) {
        return response;
    }
    let name = next.name.clone();
    let rank = body.priority;
    *entry = next;
    // Priority is the list position: an edited rank moves the row to that
    // exact slot and renumbers 1..=N, so the dashboard refetch shows the
    // replacement in real time instead of a stale in-place number. Edits
    // without a rank keep their slot.
    if let Some(rank) = rank {
        crate::config::move_subscription_to_rank(&mut cfg.subscriptions, index, rank);
    }
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    mutation_ok(format!("Updated {name}"))
}

/// `POST /api/subscriptions/:index/toggle` — flip enabled (immediate-save).
pub async fn api_subscriptions_toggle(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Path(index): Path<usize>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Subscription API unavailable",
        );
    }
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    let Some(entry) = cfg.subscriptions.get_mut(index) else {
        return mutation_error(StatusCode::NOT_FOUND, "No subscription at that index");
    };
    entry.enabled = !entry.enabled;
    let message = format!(
        "{} is now {}",
        entry.name,
        if entry.enabled { "enabled" } else { "disabled" }
    );
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    mutation_ok(message)
}

/// `DELETE /api/subscriptions/:index` — remove one subscription (immediate-save).
pub async fn api_subscriptions_delete(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Path(index): Path<usize>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Subscription API unavailable",
        );
    }
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    if index >= cfg.subscriptions.len() {
        return mutation_error(StatusCode::NOT_FOUND, "No subscription at that index");
    }
    let removed = cfg.subscriptions.remove(index);
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    mutation_ok(format!("Deleted {}", removed.name))
}

/// Reorder body: `order[i]` is the old index of the entry taking new slot `i`.
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionsReorder {
    pub order: Vec<usize>,
}

/// `POST /api/subscriptions/reorder` — drag-and-drop order from the dashboard.
/// Validates a full permutation, reorders, renumbers priorities 1..=N in the
/// new visual order ("lower runs first"), then immediate-saves to the
/// database + live snapshots (probe-result rows pick the new priorities up
/// on the next cycle).
pub async fn api_subscriptions_reorder(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<SubscriptionsReorder>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Subscription API unavailable",
        );
    }
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    let len = cfg.subscriptions.len();
    let mut sorted = body.order.clone();
    sorted.sort_unstable();
    if sorted.len() != len || sorted.iter().enumerate().any(|(slot, got)| *got != slot) {
        return mutation_error(StatusCode::BAD_REQUEST, "Order must list every index once");
    }
    cfg.subscriptions = body
        .order
        .iter()
        .map(|&old| cfg.subscriptions[old].clone())
        .collect();
    let mut priority: u32 = 0;
    for entry in &mut cfg.subscriptions {
        priority = priority.saturating_add(1);
        entry.priority = priority;
    }
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    mutation_ok(format!("Reordered {len} subscription(s)"))
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn parse_positive<T>(raw: &str) -> Option<T>
where
    T: std::str::FromStr + PartialOrd + From<u8>,
{
    let parsed = crate::config::parse_setting_number::<T>(raw)?;
    if parsed > T::from(0) {
        Some(parsed)
    } else {
        None
    }
}

/// Mirrors the TUI `optional()` for single-string settings: empty/off/none/
/// null clears, anything else is kept (quote-stripped like every TUI edit).
fn optional_string_setting(value: &str) -> Option<String> {
    let normalized = crate::sing_box::normalize_path(value);
    let normalized = if normalized.eq_ignore_ascii_case("null") {
        String::new()
    } else {
        normalized
    };
    match normalized.to_ascii_lowercase().as_str() {
        "" | "off" | "none" => None,
        _ => Some(normalized),
    }
}

/// Apply one web settings key to `cfg`. Mirrors the TUI
/// `config_editor::apply` validators for the keys the dashboard serves;
/// derived/read-only rows are refused with a human reason.
fn apply_web_setting(
    cfg: &mut crate::config::AppConfig,
    key: &str,
    raw: &str,
) -> Result<(), String> {
    // Sanitize once for every key: invisible directional controls and
    // surrounding whitespace never reach the validators or the database.
    // Digit folding stays inside the numeric parsers so text keeps its chars.
    let sanitized = crate::config::sanitize_setting_text(raw);
    let value = sanitized.as_str();
    if apply_connection_setting(cfg, key, value)? {
        return Ok(());
    }
    if apply_fetch_setting(cfg, key, value)? {
        return Ok(());
    }
    if apply_probe_setting(cfg, key, value)? {
        return Ok(());
    }
    if apply_sharing_proxy_setting(cfg, key, value)? {
        return Ok(());
    }
    if apply_advanced_setting(cfg, key, value)? {
        return Ok(());
    }
    Err(match key {
        "probe.speedtest_enabled" => {
        "probe.speedtest_enabled is derived from probe.download_url; set probe.download_url instead"
            .to_string()
        }
        _ => format!("unknown key: {key}"),
    })
}

/// Connection-group keys; returns `Ok(false)` when `key` belongs elsewhere.
fn apply_connection_setting(
    cfg: &mut crate::config::AppConfig,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "bind" => {
            cfg.bind = value
                .parse::<std::net::SocketAddr>()
                .map_err(|_| "bind must be host:port, e.g. 127.0.0.1:27141".to_string())?;
        }
        "top_n" => {
            cfg.top_n =
                parse_positive(value).ok_or_else(|| "top_n must be greater than 0".to_string())?;
        }
        "refresh_seconds" => {
            cfg.refresh_seconds = crate::config::parse_setting_number(value)
                .ok_or_else(|| "refresh_seconds must be a number".to_string())?;
        }
        "ping_seconds" => {
            cfg.ping_seconds = crate::config::parse_setting_number(value)
                .ok_or_else(|| "ping_seconds must be a number".to_string())?;
        }
        "encoded_subscription" => {
            cfg.encoded_subscription =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "prioritize_stability" => {
            cfg.prioritize_stability =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "return_configs_asap" => {
            cfg.return_configs_asap =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "scan_all_configs" => {
            cfg.scan_all_configs =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "use_cache_only" => {
            cfg.use_cache_only =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        // Mirrors the TUI `optional()`: empty/off/none/null clears the
        // emergency proxy; anything else is the single config URI.
        "emergency_config" => {
            cfg.emergency_config = optional_string_setting(value);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Fetch-group keys; returns `Ok(false)` when `key` belongs elsewhere.
fn apply_fetch_setting(
    cfg: &mut crate::config::AppConfig,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "fetch_timeout_ms" => {
            cfg.fetch_timeout_ms = parse_positive(value)
                .ok_or_else(|| "fetch_timeout_ms must be greater than 0".to_string())?;
        }
        "fetch_concurrency" => {
            cfg.fetch_concurrency = parse_positive(value)
                .ok_or_else(|| "fetch_concurrency must be greater than 0".to_string())?;
        }
        "max_subscription_bytes" => {
            cfg.max_subscription_bytes = parse_positive(value)
                .ok_or_else(|| "max_subscription_bytes must be greater than 0".to_string())?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Probe-group keys; returns `Ok(false)` when `key` belongs elsewhere.
fn apply_probe_setting(
    cfg: &mut crate::config::AppConfig,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "probe.mode" => {
            cfg.probe.mode = match value.to_ascii_lowercase().as_str() {
                "active" => crate::config::ProbeMode::Active,
                "tcp" => crate::config::ProbeMode::Tcp,
                _ => return Err("probe.mode must be active or tcp".to_string()),
            };
        }
        // Mirrors the TUI edit: any manual path turns auto-detect off (an
        // empty value re-arms it on the next start).
        "probe.sing_box_path" => {
            let normalized = crate::sing_box::normalize_path(value);
            cfg.probe.sing_box_path = if normalized.eq_ignore_ascii_case("null") {
                String::new()
            } else {
                normalized
            };
            cfg.probe.sing_box_path_auto = false;
        }
        "probe.connect_timeout_ms" => {
            cfg.probe.connect_timeout_ms = parse_positive(value)
                .ok_or_else(|| "probe.connect_timeout_ms must be greater than 0".to_string())?;
        }
        "probe.concurrency" => {
            cfg.probe.concurrency = parse_positive(value)
                .ok_or_else(|| "probe.concurrency must be greater than 0".to_string())?;
        }
        "probe.batch_size" => {
            cfg.probe.batch_size = match value.to_ascii_lowercase().as_str() {
                "" | "auto" | "off" | "none" | "null" => None,
                _ => Some(
                    parse_positive(value)
                        .ok_or_else(|| "probe.batch_size must be a number or auto".to_string())?,
                ),
            };
        }
        "probe.process_concurrency" => {
            cfg.probe.process_concurrency = match value.to_ascii_lowercase().as_str() {
                "" | "auto" | "off" | "none" | "null" => None,
                _ => Some(parse_positive(value).ok_or_else(|| {
                    "probe.process_concurrency must be a number or auto".to_string()
                })?),
            };
        }
        "probe.active_timeout_ms" => {
            cfg.probe.active_timeout_ms = parse_positive(value)
                .ok_or_else(|| "probe.active_timeout_ms must be greater than 0".to_string())?;
        }
        "probe.startup_timeout_ms" => {
            cfg.probe.startup_timeout_ms = parse_positive(value)
                .ok_or_else(|| "probe.startup_timeout_ms must be greater than 0".to_string())?;
        }
        "probe.test_url" => {
            if value.is_empty() {
                return Err("probe.test_url cannot be empty".to_string());
            }
            cfg.probe.test_url = value.to_string();
        }
        "probe.accepted_statuses" => {
            cfg.probe.accepted_statuses = parse_status_list(value)?;
        }
        "probe.download_bytes_limit" => {
            cfg.probe.download_bytes_limit = parse_positive(value)
                .ok_or_else(|| "probe.download_bytes_limit must be greater than 0".to_string())?;
        }
        // Mirrors the TUI `optional()`: empty/off/none/null disables the
        // speedtest, anything else is the download link.
        "probe.download_url" => {
            cfg.probe.download_url = match value.to_ascii_lowercase().as_str() {
                "" | "off" | "none" | "null" => None,
                _ => Some(value.to_string()),
            };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn parse_status_list(value: &str) -> Result<Vec<u16>, String> {
    let inner = value.strip_prefix('[').unwrap_or(value);
    let inner = inner.strip_suffix(']').unwrap_or(inner);
    let parsed = inner
        .split(',')
        .map(crate::config::parse_setting_number::<u16>)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "accepted_statuses must be HTTP codes 100..599".to_string())?;
    if parsed.is_empty() || !parsed.iter().all(|status| (100..=599).contains(status)) {
        return Err("accepted_statuses must be HTTP codes 100..599".to_string());
    }
    Ok(parsed)
}

/// Sharing/proxy-group keys; returns `Ok(false)` when `key` belongs elsewhere.
fn apply_sharing_proxy_setting(
    cfg: &mut crate::config::AppConfig,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "sharing.enabled" => {
            cfg.sharing.enabled =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "sharing.require_token" => {
            cfg.sharing.require_token =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
            // Flipping protection on with no token mints a random editable
            // one: storing the trap would brick the next settings load.
            crate::config::ensure_sharing_token(cfg);
        }
        "sharing.token" => {
            cfg.sharing.token = crate::config::normalize_sharing_token(value);
            // Clearing the token while protection is on re-mints instead of
            // storing the trap.
            crate::config::ensure_sharing_token(cfg);
        }
        "proxy.enabled" => {
            cfg.proxy.enabled =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "proxy.port" => {
            cfg.proxy.port = parse_positive(value)
                .ok_or_else(|| "proxy.port must be greater than 0".to_string())?;
        }
        "proxy.discoverable" => {
            cfg.proxy.discoverable =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "proxy.rotating_proxy" => {
            cfg.proxy.rotating_proxy =
                parse_bool(value).ok_or_else(|| "expected true/false".to_string())?;
        }
        "proxy.health_check_url" => {
            if value.is_empty() {
                return Err("proxy.health_check_url cannot be empty".to_string());
            }
            cfg.proxy.health_check_url = value.to_string();
        }
        "proxy.health_check_interval_seconds" => {
            cfg.proxy.health_check_interval_seconds = parse_positive(value).ok_or_else(|| {
                "proxy.health_check_interval_seconds must be greater than 0".to_string()
            })?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Advanced-group keys; returns `Ok(false)` when `key` belongs elsewhere.
fn apply_advanced_setting(
    cfg: &mut crate::config::AppConfig,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "clean_offlines_after_days" => {
            cfg.clean_offlines_after_days = parse_positive(value)
                .ok_or_else(|| "clean_offlines_after_days must be greater than 0".to_string())?;
        }
        // No path normalization here (unlike sing-box): any non-empty value
        // is a custom database path; empty/off/none/null restores built-in.
        "geoip_db_path" => {
            cfg.geoip_db_path = match value.to_ascii_lowercase().as_str() {
                "" | "off" | "none" | "null" => None,
                _ => Some(value.to_string()),
            };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// `PATCH /api/config` — edit one setting (immediate-save, TUI validators).
pub async fn api_config_patch(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<ConfigPatch>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Settings API unavailable");
    }
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    let before = cfg.clone();
    if let Err(message) = apply_web_setting(&mut cfg, body.key.trim(), &body.value) {
        return mutation_error(StatusCode::BAD_REQUEST, message);
    }
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    // No instant re-fetch: the loop picks refresh-relevant edits up on the
    // next cycle (or a manual refresh), so say so when the fingerprint moved.
    let next_cycle = crate::refresh_relevant_changed(&before, &cfg);
    mutation_applies_next_cycle(format!("Updated {}", body.key.trim()), next_cycle)
}

/// `POST /api/config/reset` — restore non-subscription settings from the
/// embedded defaults (subscriptions kept, user-filled empty-by-default
/// essentials kept, merge baseline re-anchored).
pub async fn api_config_reset(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if state.config_tx.is_none() {
        return mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Settings API unavailable");
    }
    let Some(db) = state.database.clone() else {
        return mutation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Settings database unavailable",
        );
    };
    let mut cfg = match load_db_config(&state).await {
        Ok(cfg) => cfg,
        Err(response) => return response,
    };
    let before = cfg.clone();
    if let Err(error) = crate::settings::reset_to_embedded_defaults(&db, &mut cfg) {
        return mutation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to restore defaults: {error:#}"),
        );
    }
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    let next_cycle = crate::refresh_relevant_changed(&before, &cfg);
    mutation_applies_next_cycle(
        "Defaults restored; subscriptions and essentials kept".to_string(),
        next_cycle,
    )
}

/// `GET /api/config/token` — reveal the LAN token to an authorized dashboard
/// viewer. The Settings table masks it by default and the UI fetches it only
/// on explicit Show, so the secret never sits in a polled response. Same
/// authorization as every route: viewers are loopback owners (the TUI shows
/// them the token in plaintext too) or token-holding LAN readers who already
/// know it — nothing new leaks.
pub async fn api_config_token(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    let config = state.config.read().await.clone();
    Json(serde_json::json!({ "token": config.token })).into_response()
}

/// `POST /api/save` — compatibility no-op: the dashboard persists
/// immediately, so there is never anything to flush. Always succeeds so the
/// Save bar (and older UI builds) never 404.
pub async fn api_save(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    mutation_ok("Saved")
}

/// `POST /api/proxy/select` — pin (`{uri}`) or unpin (`{uri: null}`) a manual
/// proxy. Live-push only, never writes the file (same as TUI `Enter`).
pub async fn api_proxy_select(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<ProxySelect>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    let Some(tx) = state.config_tx.as_ref() else {
        return mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Proxy API unavailable");
    };
    let mut cfg = tx.borrow().clone();
    if let Some(uri) = body.uri {
        let uri = uri.trim().to_string();
        if uri.is_empty() {
            cfg.proxy.manual_proxy_uri = None;
        } else {
            cfg.proxy.manual_proxy_uri = Some(uri);
        }
    } else {
        cfg.proxy.manual_proxy_uri = None;
    }
    push_live_config(&state, &cfg).await;
    if cfg.proxy.manual_proxy_uri.is_some() {
        mutation_ok("Proxy set to config")
    } else {
        mutation_ok("Proxy: auto-select (manual cleared)")
    }
}

/// Requested proxy mode (`POST /api/proxy/mode`): `None`/empty keeps the
/// legacy Off → Local → LAN cycle (same as the TUI row).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProxyMode {
    #[serde(default)]
    pub mode: Option<String>,
}

/// `POST /api/proxy/mode` — set `{"mode": "off"|"local"|"lan"}` directly,
/// or cycle Off → Local → LAN when omitted (same as the TUI row).
/// Immediate-save + live-push; firewall message rides along like the TUI.
pub async fn api_proxy_mode(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<ProxyMode>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    let Some(tx) = state.config_tx.as_ref() else {
        return mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Proxy API unavailable");
    };
    // Base the toggle on the live config, not the disk file: a manual pin
    // is live-pushed only (never written), and rebuilding from disk here
    // would silently drop it from both the file and the runtime.
    let mut cfg = tx.borrow().clone();
    match body
        .mode
        .as_deref()
        .map(|mode| mode.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") => {
            if !cfg.proxy.enabled {
                cfg.proxy.enabled = true;
                cfg.proxy.discoverable = false;
            } else if !cfg.proxy.discoverable {
                cfg.proxy.discoverable = true;
            } else {
                cfg.proxy.enabled = false;
                cfg.proxy.discoverable = false;
            }
        }
        Some("off") => {
            cfg.proxy.enabled = false;
            cfg.proxy.discoverable = false;
        }
        Some("local") => {
            cfg.proxy.enabled = true;
            cfg.proxy.discoverable = false;
        }
        Some("lan") => {
            cfg.proxy.enabled = true;
            cfg.proxy.discoverable = true;
        }
        Some(other) => {
            return mutation_error(
                StatusCode::BAD_REQUEST,
                format!("Unknown proxy mode '{other}' (use off, local, or lan)"),
            );
        }
    }
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    let mode_label = if !cfg.proxy.enabled {
        "off"
    } else if cfg.proxy.discoverable {
        "LAN"
    } else {
        "local"
    };
    match crate::tui::firewall::apply(
        &state.data_dir,
        cfg.proxy.discoverable,
        cfg.proxy.port,
        crate::constants::FIREWALL_PROXY_RULE_NAME,
    ) {
        Ok(message) => mutation_ok(format!("Proxy {mode_label} ({message})")),
        Err(error) => mutation_firewall_elevation(format!(
            "Proxy {mode_label} (firewall update failed: {error})"
        )),
    }
}

/// `POST /api/sharing` — toggle LAN sharing (same save + live-push +
/// firewall as the TUI row).
pub async fn api_sharing(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    let Some(tx) = state.config_tx.as_ref() else {
        return mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Sharing API unavailable");
    };
    // Live base, same as the proxy toggle: rebuilding from disk would drop
    // a live-only manual pin.
    let mut cfg = tx.borrow().clone();
    cfg.sharing.enabled = !cfg.sharing.enabled;
    if let Err(response) = persist_config(&state, &cfg).await {
        return response;
    }
    let sharing_label = if cfg.sharing.enabled { "on" } else { "off" };
    match crate::tui::firewall::apply(
        &state.data_dir,
        cfg.sharing.enabled,
        cfg.bind.port(),
        crate::constants::FIREWALL_RULE_NAME,
    ) {
        Ok(message) => mutation_ok(format!("Sharing {sharing_label} ({message})")),
        Err(error) => mutation_firewall_elevation(format!(
            "Sharing {sharing_label} (firewall update failed: {error})"
        )),
    }
}

/// `POST /api/cache/clean` — clear the probe database (`{confirm: "DELETE"}`).
/// Same typed-confirm gate as the TUI; runs off-thread like the TUI handler.
pub async fn api_cache_clean(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ConnectInfo(remote_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<CacheClean>,
) -> Response {
    let token = query.token.as_deref().or_else(|| bearer_token(&headers));
    if let Err(response) = authorize(&state, remote_addr, token).await {
        return response;
    }
    if body.confirm.trim() != "DELETE" {
        return mutation_error(StatusCode::BAD_REQUEST, "Type DELETE to clean cache");
    }
    let Some(database) = state.database.clone() else {
        return mutation_error(StatusCode::SERVICE_UNAVAILABLE, "Cache API unavailable");
    };
    let Ok(cleaned) = tokio::task::spawn_blocking(move || database.delete_all()).await else {
        return mutation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Clean cache failed: task interrupted",
        );
    };
    match cleaned {
        Ok(()) => mutation_ok("Clean cache finished: database cleared"),
        Err(error) => mutation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Clean cache failed: {error}"),
        ),
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
    /// Fetched count lands while tested/working hold still (the fetch phase
    /// completes before any probe result): without it the Fetched badge
    /// sticks at the hello value until a manual reload.
    total: usize,
    ranked_len: usize,
    refreshing: bool,
    pinging: bool,
    /// Cycle end must retrip the delta even when counts are unchanged (a
    /// finished refresh with identical tested/working would otherwise never
    /// push, leaving the UI stuck on "running"). Same for the ping anchor: a
    /// manual ping re-arms the full countdown with no count change.
    finished_at: Option<String>,
    ping_at: Option<String>,
    /// Proxy state rides the delta too: a pin from either UI (dashboard or
    /// TUI) must confirm live on the other side instead of waiting for the
    /// next full snapshot that may never come on a quiet feed.
    proxy_running: bool,
    proxy_active_uri: Option<String>,
}

impl FeedFingerprint {
    fn of(runtime: &crate::model::RuntimeState) -> Self {
        Self {
            tested: runtime.tested_candidates,
            working: runtime.reachable_candidates,
            total: runtime.total_candidates,
            ranked_len: runtime.ranked.len(),
            refreshing: runtime.refreshing,
            pinging: runtime.pinging,
            finished_at: runtime.refresh_finished_at.clone(),
            ping_at: runtime.last_ping_at.clone(),
            proxy_running: runtime.proxy_running,
            proxy_active_uri: runtime.proxy_active_uri.clone(),
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
                "total": snapshot.total_candidates,
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
                "proxy_running": snapshot.proxy_running,
                "proxy_active_config": snapshot.proxy_active_config,
                "proxy_active_uri": snapshot.proxy_active_uri,
                "proxy_port": snapshot.proxy_port,
                "proxy_discoverable": snapshot.proxy_discoverable,
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
        DASHBOARD_CSS, DASHBOARD_HTML, DASHBOARD_I18N, DASHBOARD_JS, DASHBOARD_QR, FAVICON_SVG,
        FLAG_CN, FLAG_FR, FLAG_GB, FLAG_IR, FLAG_RU, FeedFingerprint, config_response,
        events_response, favicon, qr_generate_response, qr_image_response, static_response,
        subscriptions_response, summary_response,
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

    /// Test-only database seeded with example defaults, mirroring a fresh
    /// install right after first-run seeding.
    fn test_db() -> std::sync::Arc<crate::db::Database> {
        let db =
            crate::db::Database::open(&temp_data_dir("db").join("data.db")).expect("test db opens");
        crate::settings::save_app_config(&db, &crate::config::AppConfig::default_for_first_run())
            .expect("seed saves");
        std::sync::Arc::new(db)
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
            use_cache_only: false,
            emergency_config: None,
            clean_offlines_after_days: 7,
            geoip_db_path: None,
            sharing_enabled,
            require_token: false,
            token: String::new(),
            probe_mode: "active".to_string(),
            sing_box_path: String::new(),
            connect_timeout_ms: 5_000,
            speedtest_enabled: false,
            probe_concurrency: 16,
            probe_batch_size: None,
            probe_process_concurrency: None,
            active_timeout_ms: 30_000,
            startup_timeout_ms: 5_000,
            test_url: "https://www.gstatic.com/generate_204".to_string(),
            accepted_statuses: vec![204, 200],
            download_bytes_limit: 1_048_576,
            download_url: None,
            subscription_count: 0,
            enabled_subscription_count: 0,
            proxy_enabled: false,
            proxy_port: 27910,
            proxy_discoverable: false,
            rotating_proxy: true,
            health_check_url: "https://cp.cloudflare.com".to_string(),
            health_check_interval_seconds: 60,
            proxy_manual_uri: None,
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
            config_tx: None,
            refresh_tx: None,
            ping_tx: None,
            database: Some(test_db()),
        }
    }

    #[test]
    fn feed_fingerprint_retrips_on_proxy_switch() {
        // A pin from either UI must push a live probe-delta: without the
        // proxy fields in the fingerprint the dashboard's Pending… button
        // would wait for a full snapshot that never comes on a quiet feed.
        let idle = FeedFingerprint::of(&RuntimeState::default());
        let switched = RuntimeState {
            proxy_running: true,
            proxy_active_uri: Some("vless://uuid@example.com:443#node".to_string()),
            ..RuntimeState::default()
        };
        assert_ne!(idle, FeedFingerprint::of(&switched));

        let unpinned = RuntimeState {
            proxy_running: true,
            proxy_active_uri: None,
            ..RuntimeState::default()
        };
        assert_ne!(
            FeedFingerprint::of(&switched),
            FeedFingerprint::of(&unpinned)
        );
    }

    #[test]
    fn feed_fingerprint_retrips_on_total_change() {
        // The fetch phase lands while tested/working hold still: without
        // total in the fingerprint no probe-delta fires and the Fetched
        // badge sticks at the hello value until a manual reload.
        let before = FeedFingerprint::of(&RuntimeState::default());
        let fetched = RuntimeState {
            total_candidates: 9482,
            ..RuntimeState::default()
        };
        assert_ne!(before, FeedFingerprint::of(&fetched));
    }

    #[test]
    fn dashboard_assets_stay_self_contained() {
        assert!(DASHBOARD_HTML.contains("<!DOCTYPE html>"));
        assert!(DASHBOARD_HTML.contains("v2raydar-theme"));
        assert!(DASHBOARD_HTML.contains("/i18n.js"));
        assert!(DASHBOARD_CSS.contains("prefers-color-scheme"));
        assert!(DASHBOARD_JS.contains("EventSource"));
        assert!(DASHBOARD_I18N.contains("I18N_STRINGS"));
        assert!(DASHBOARD_QR.contains("QREncode"));
        assert!(FAVICON_SVG.contains("<svg"));
        // Bare "http" substrings are fine (the UI builds loopback URLs like
        // "http://" + host at runtime); what must never appear is a remote
        // *reference* that would fetch off-device.
        // Flag files are real SVGs served on demand (not in the payload sum):
        // they must still be self-contained vectors, never remote refs.
        for asset in [FLAG_GB, FLAG_IR, FLAG_CN, FLAG_FR, FLAG_RU] {
            assert!(asset.contains("<svg"), "flag must be an SVG file");
            // Same remote-reference bar as the shell (`xmlns="http://…"` is
            // fine — it is a namespace, not a fetch).
            for snippet in ["src=\"http", "href=\"http", "url(http", "@import"] {
                assert!(
                    !asset.contains(snippet),
                    "flag must not reference anything remote"
                );
            }
        }
        for asset in [DASHBOARD_HTML, DASHBOARD_CSS, DASHBOARD_JS, DASHBOARD_I18N] {
            assert!(!asset.contains("innerHTML"), "DOM injection sink");
            for snippet in [
                "src=\"http",
                "fetch(\"http",
                "EventSource(\"http",
                "url(http",
                "@import",
                "XMLHttpRequest",
            ] {
                assert!(!asset.contains(snippet), "remote reference: {snippet}");
            }
        }
        // The deliberate exception: support/star anchors in the keys dialog
        // navigate on user click — they fetch nothing by themselves, so the
        // shell stays self-contained. Anything else remote fails closed.
        let mut search = DASHBOARD_HTML;
        let mut anchors = 0;
        while let Some(pos) = search.find("href=\"http") {
            let tag_start = search[..pos].rfind('<').expect("href lives in a tag");
            let tag = &search[tag_start..pos];
            assert!(
                tag.starts_with("<a ")
                    && search[pos..].starts_with("href=\"https://github.com/411A/V2RayDAR"),
                "remote reference must be a repository navigation anchor, got: {tag}"
            );
            anchors += 1;
            search = &search[pos + 1..];
        }
        assert_eq!(anchors, 2, "exactly the support + star anchors link out");
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
    async fn dashboard_flag_serves_whitelisted_svgs_and_404s_the_rest() {
        use super::dashboard_flag;
        let state = http_state(RuntimeState::default(), false);
        for file in ["GB.svg", "IR.svg", "CN.svg", "FR.svg", "RU.svg"] {
            let (headers, query, connect) = no_auth();
            let response = dashboard_flag(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
                axum::extract::Path(file.to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{file} must serve");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE),
                Some(&HeaderValue::from_static("image/svg+xml"))
            );
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body reads");
            let text = std::str::from_utf8(&body).expect("body is utf-8");
            assert!(text.contains("<svg"), "{file} must be an SVG file");
        }
        for file in ["EVIL.svg", "../app.js", "GB.svg/"] {
            let (headers, query, connect) = no_auth();
            let response = dashboard_flag(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
                axum::extract::Path(file.to_string()),
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{file} must not serve"
            );
        }
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
    async fn dashboard_flag_serves_power_icon() {
        use super::dashboard_flag;
        let state = http_state(RuntimeState::default(), false);
        let (headers, query, connect) = no_auth();
        let response = dashboard_flag(
            axum::extract::State(state),
            headers,
            query,
            connect,
            axum::extract::Path("power-off-svgrepo-com.svg".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("image/svg+xml"))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = std::str::from_utf8(&body).expect("body is utf-8");
        assert!(text.contains("<svg"), "power icon must be an SVG file");
    }

    #[tokio::test]
    async fn dashboard_flag_serves_vendored_vazirmatn() {
        use super::dashboard_flag;
        let state = http_state(RuntimeState::default(), false);
        for file in ["vazirmatn-arabic.woff2", "vazirmatn-latin.woff2"] {
            let (headers, query, connect) = no_auth();
            let response = dashboard_flag(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
                axum::extract::Path(file.to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{file} must serve");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE),
                Some(&HeaderValue::from_static("font/woff2"))
            );
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body reads");
            assert!(
                body.starts_with(b"wOF2"),
                "{file} must be a woff2 font file"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_confirms_without_exiting_tests() {
        // cfg!(test) skips the process::exit spawn: the runner survives.
        let state = http_state(RuntimeState::default(), false);
        let (headers, query, connect) = no_auth();
        let response =
            super::api_shutdown(axum::extract::State(state), headers, query, connect).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = std::str::from_utf8(&body).expect("body is utf-8");
        assert!(text.contains("stopping"), "confirmation names the stop");
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
        let mut seen_download_url = false;
        let mut seen_sing_box_path = false;
        let mut seen_process_concurrency = false;
        let mut seen_rotating_proxy = false;
        let mut seen_emergency = false;
        let mut seen_advanced = false;
        for group in groups {
            assert!(group["title"].is_string(), "every group has a string title");
            assert!(
                group["id"].is_string(),
                "every group has a stable client id"
            );
            let keys = group["keys"].as_array().expect("keys is an array");
            for row in keys {
                assert!(row["key"].is_string(), "every row has a string key");
                assert!(row["value"].is_string(), "every row has a string value");
                assert!(row["guide"].is_string(), "every row has a string guide");
                assert!(row["kind"].is_string(), "every row has a control kind");
                assert!(row["options"].is_array(), "every row has an options array");
                rows += 1;
                if row["key"] == "probe.mode" {
                    assert_eq!(
                        row["options"],
                        serde_json::json!(["active", "tcp"]),
                        "probe.mode offers its two choices"
                    );
                }
                if row["key"] == "sharing.token" {
                    seen_token = true;
                    assert_eq!(row["value"], serde_json::Value::from("set"));
                }
                if row["key"] == "probe.accepted_statuses" {
                    seen_statuses = true;
                    assert_eq!(row["value"], serde_json::Value::from("[204, 200]"));
                }
                if row["key"] == "probe.download_url" {
                    seen_download_url = true;
                    assert_eq!(row["value"], serde_json::Value::from("off"));
                    assert_eq!(row["kind"], serde_json::Value::from("text"));
                }
                if row["key"] == "probe.sing_box_path" {
                    seen_sing_box_path = true;
                    assert_eq!(row["kind"], serde_json::Value::from("text"));
                }
                if row["key"] == "probe.process_concurrency" {
                    seen_process_concurrency = true;
                    assert_eq!(row["value"], serde_json::Value::from("null"));
                }
                if row["key"] == "proxy.rotating_proxy" {
                    seen_rotating_proxy = true;
                    assert_eq!(row["kind"], serde_json::Value::from("bool"));
                }
                if row["key"] == "emergency_config" {
                    seen_emergency = true;
                    assert_eq!(row["value"], serde_json::Value::from(""));
                }
                if group["id"] == "advanced" {
                    seen_advanced = true;
                }
            }
        }
        assert!(rows > 0, "groups must carry key rows");
        assert!(seen_token, "sharing.token presence marker is served");
        assert!(seen_statuses, "probe.accepted_statuses list is served");
        assert!(seen_download_url, "probe.download_url switch is served");
        assert!(seen_sing_box_path, "probe.sing_box_path is served");
        assert!(
            seen_process_concurrency,
            "probe.process_concurrency is served"
        );
        assert!(seen_rotating_proxy, "proxy.rotating_proxy is served");
        assert!(seen_emergency, "emergency_config is served");
        assert!(seen_advanced, "advanced group is served");
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
    async fn firewall_elevation_flag_rides_along_for_admin_guide() {
        // The dashboard pops the run-as-admin guide from `code` + `os`; the
        // setting itself is already saved, so status stays 200 + ok.
        let response = super::mutation_firewall_elevation(
            "Sharing on (firewall update failed: boom)".to_string(),
        );
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("elevation is json");
        assert_eq!(value["ok"], serde_json::Value::Bool(true));
        assert_eq!(
            value["code"],
            serde_json::Value::String(super::FIREWALL_ELEVATION_CODE.to_string())
        );
        assert_eq!(
            value["os"],
            serde_json::Value::String(std::env::consts::OS.to_string())
        );
        assert!(
            value["status"]
                .as_str()
                .is_some_and(|status| status.contains("firewall update failed")),
            "the TUI-style reason stays human-readable: {value}"
        );
    }

    #[tokio::test]
    async fn plain_mutations_carry_no_admin_flag() {
        for response in [
            super::mutation_ok("done"),
            super::mutation_error(StatusCode::BAD_REQUEST, "nope"),
        ] {
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body reads");
            let value: serde_json::Value = serde_json::from_slice(&body).expect("mutation is json");
            assert!(
                value.get("code").is_none(),
                "plain mutation must not flag: {value}"
            );
            assert!(
                value.get("os").is_none(),
                "plain mutation must not flag: {value}"
            );
        }
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

    /// Test-only [`HttpState`] with live trigger channels and a real seeded
    /// database, so mutation handlers run end to end.
    fn mutation_state(
        runtime: RuntimeState,
    ) -> (
        HttpState,
        tokio::sync::mpsc::UnboundedReceiver<()>,
        tokio::sync::mpsc::UnboundedReceiver<()>,
        tokio::sync::watch::Receiver<crate::config::AppConfig>,
    ) {
        let mut state = http_state(runtime, false);
        let seed =
            crate::settings::load_app_config(state.database.as_ref().expect("test db present"))
                .expect("seed loads");
        let (config_tx, config_rx) = tokio::sync::watch::channel(seed);
        let (refresh_tx, refresh_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ping_tx, ping_rx) = tokio::sync::mpsc::unbounded_channel();
        state.config_tx = Some(config_tx);
        state.refresh_tx = Some(refresh_tx);
        state.ping_tx = Some(ping_tx);
        (state, refresh_rx, ping_rx, config_rx)
    }

    fn stored_config(state: &HttpState) -> crate::config::AppConfig {
        crate::settings::load_app_config(state.database.as_ref().expect("test db present"))
            .expect("settings reload")
    }

    fn no_auth() -> (
        axum::http::HeaderMap,
        axum::extract::Query<crate::server::AuthQuery>,
        axum::extract::ConnectInfo<SocketAddr>,
    ) {
        (
            axum::http::HeaderMap::new(),
            axum::extract::Query(crate::server::AuthQuery { token: None }),
            axum::extract::ConnectInfo(loopback()),
        )
    }

    #[tokio::test]
    async fn refresh_conflicts_while_refresh_running_and_never_sends() {
        let (state, mut refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState {
            refreshing: true,
            ..RuntimeState::default()
        });
        let (headers, query, connect) = no_auth();
        let response =
            super::api_refresh(axum::extract::State(state), headers, query, connect).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(
            refresh_rx.try_recv().is_err(),
            "a refused refresh must not reach the backend loop"
        );
    }

    #[tokio::test]
    async fn refresh_sends_exactly_one_trigger_when_idle() {
        let (state, mut refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response =
            super::api_refresh(axum::extract::State(state), headers, query, connect).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(refresh_rx.try_recv().is_ok(), "one trigger fires");
        assert!(
            refresh_rx.try_recv().is_err(),
            "no duplicate trigger is queued"
        );
    }

    #[tokio::test]
    async fn refresh_unavailable_without_trigger_channel() {
        let state = http_state(RuntimeState::default(), false);
        let (headers, query, connect) = no_auth();
        let response =
            super::api_refresh(axum::extract::State(state), headers, query, connect).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn ping_conflicts_while_any_cycle_runs_and_never_sends() {
        for runtime in [
            RuntimeState {
                refreshing: true,
                ..RuntimeState::default()
            },
            RuntimeState {
                pinging: true,
                ..RuntimeState::default()
            },
        ] {
            let (state, _refresh_rx, mut ping_rx, _config_rx) = mutation_state(runtime);
            let (headers, query, connect) = no_auth();
            let response =
                super::api_ping(axum::extract::State(state), headers, query, connect).await;

            assert_eq!(response.status(), StatusCode::CONFLICT);
            assert!(
                ping_rx.try_recv().is_err(),
                "a refused ping must not reach the backend loop"
            );
        }
    }

    #[tokio::test]
    async fn ping_sends_exactly_one_trigger_when_idle() {
        let (state, _refresh_rx, mut ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response = super::api_ping(axum::extract::State(state), headers, query, connect).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(ping_rx.try_recv().is_ok(), "one trigger fires");
        assert!(
            ping_rx.try_recv().is_err(),
            "no duplicate trigger is queued"
        );
    }

    #[tokio::test]
    async fn subscriptions_add_persists_and_pushes_live() {
        let (state, _refresh_rx, _ping_rx, mut config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_add(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::SubscriptionAdd {
                url: "https://example.com/new.txt".to_string(),
                name: "new".to_string(),
                priority: 5,
                enabled: true,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        assert!(
            stored
                .subscriptions
                .iter()
                .any(|source| source.name == "new" && source.priority == 5),
            "new subscription persists to the database"
        );
        assert!(
            state
                .subscriptions
                .read()
                .await
                .iter()
                .any(|source| source.name == "new"),
            "shared snapshot updates immediately"
        );
        config_rx.changed().await.expect("live broadcast fires");
        assert!(
            config_rx
                .borrow()
                .subscriptions
                .iter()
                .any(|source| source.name == "new"),
            "refresh/ping loops see the new subscription"
        );
    }

    #[tokio::test]
    async fn subscriptions_reject_unsupported_url_scheme() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_add(
            axum::extract::State(state),
            headers,
            query,
            connect,
            axum::Json(super::SubscriptionAdd {
                url: "gopher://example.com/sub".to_string(),
                name: "bad".to_string(),
                priority: 5,
                enabled: true,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn subscriptions_add_rejects_duplicate_url_with_its_index() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        seed_subscriptions(&state);
        // "b" already lives at the second row: re-adding its URL is refused
        // with a 409 that names the twin's 1-based index.
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_add(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::SubscriptionAdd {
                url: "https://example.com/b.txt".to_string(),
                name: "b-again".to_string(),
                priority: 1,
                enabled: true,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("index 2"),
            "rejection names the twin's index: {text}"
        );
        let stored = stored_config(&state);
        assert_eq!(stored.subscriptions.len(), 3, "no twin row persists");
    }

    #[tokio::test]
    async fn subscriptions_patch_rejects_sibling_url_but_allows_echo_save() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        seed_subscriptions(&state);
        // Pointing row 0 at row 1's URL is refused with the sibling's index.
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::extract::Path(0_usize),
            axum::Json(super::SubscriptionPatch {
                url: Some("https://example.com/b.txt".to_string()),
                ..Default::default()
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert!(
            String::from_utf8_lossy(&body).contains("index 2"),
            "rejection names the sibling's index"
        );

        // Re-saving a row with its own URL is an echo, not a twin.
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::extract::Path(1_usize),
            axum::Json(super::SubscriptionPatch {
                url: Some("https://example.com/b.txt".to_string()),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    fn seed_subscriptions(state: &HttpState) {
        let sources = ["a", "b", "c"]
            .into_iter()
            .map(|name| crate::config::SubscriptionSource {
                name: name.to_string(),
                url: format!("https://example.com/{name}.txt"),
                enabled: name != "c",
                priority: if name == "c" { 9 } else { 5 },
            })
            .collect::<Vec<_>>();
        state
            .database
            .as_ref()
            .expect("test db present")
            .save_subscriptions(&sources)
            .expect("seed subscriptions save");
    }

    #[tokio::test]
    async fn subscriptions_reorder_permutes_renumbers_and_persists() {
        let (state, _refresh_rx, _ping_rx, mut config_rx) = mutation_state(RuntimeState::default());
        seed_subscriptions(&state);
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_reorder(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::SubscriptionsReorder {
                order: vec![2, 0, 1],
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        let names: Vec<&str> = stored
            .subscriptions
            .iter()
            .map(|source| source.name.as_str())
            .collect();
        assert_eq!(names, ["c", "a", "b"], "stored order follows the drop");
        let priorities: Vec<u32> = stored
            .subscriptions
            .iter()
            .map(|source| source.priority)
            .collect();
        assert_eq!(priorities, [1, 2, 3], "priorities renumber in visual order");
        let live = state.subscriptions.read().await;
        assert_eq!(live[0].name, "c", "shared snapshot updates immediately");
        assert_eq!(live[0].priority, 1);
        config_rx.changed().await.expect("live broadcast fires");
    }

    #[tokio::test]
    async fn subscriptions_patch_moves_row_to_edited_rank() {
        let (state, _refresh_rx, _ping_rx, mut config_rx) = mutation_state(RuntimeState::default());
        seed_subscriptions(&state);
        // "c" sits last; retitling its rank to the top must move the row,
        // not just relabel the number in place.
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::extract::Path(2_usize),
            axum::Json(super::SubscriptionPatch {
                priority: Some(0),
                ..Default::default()
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        let names: Vec<&str> = stored
            .subscriptions
            .iter()
            .map(|source| source.name.as_str())
            .collect();
        assert_eq!(names, ["c", "a", "b"], "edited rank moves the row");
        let priorities: Vec<u32> = stored
            .subscriptions
            .iter()
            .map(|source| source.priority)
            .collect();
        assert_eq!(priorities, [1, 2, 3], "ranks stay dense after the move");
        let live = state.subscriptions.read().await;
        assert_eq!(live[0].name, "c", "shared snapshot updates immediately");
        config_rx.changed().await.expect("live broadcast fires");
    }

    #[tokio::test]
    async fn subscriptions_reorder_rejects_non_permutations() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        seed_subscriptions(&state);
        // Duplicate, out-of-range, and wrong-length orders are all 400 and
        // must leave the file untouched.
        for order in [vec![0, 0, 1], vec![0, 1, 5], vec![0, 1], vec![0, 1, 2, 0]] {
            let (headers, query, connect) = no_auth();
            let response = super::api_subscriptions_reorder(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
                axum::Json(super::SubscriptionsReorder { order }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        let stored = stored_config(&state);
        let names: Vec<&str> = stored
            .subscriptions
            .iter()
            .map(|source| source.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c"], "rejected reorder writes nothing");
        let priorities: Vec<u32> = stored
            .subscriptions
            .iter()
            .map(|source| source.priority)
            .collect();
        assert_eq!(priorities, [1, 2, 3], "loads converge to dense order");
    }

    #[tokio::test]
    async fn config_patch_rejects_unknown_and_read_only_keys() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        for key in [
            "nope.not_a_key",
            "subscription_count",
            "probe.speedtest_enabled",
        ] {
            let (headers, query, connect) = no_auth();
            let response = super::api_config_patch(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
                axum::Json(super::ConfigPatch {
                    key: key.to_string(),
                    value: "1".to_string(),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "key: {key}");
        }
    }

    #[tokio::test]
    async fn config_patch_applies_validated_value_live() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response = super::api_config_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ConfigPatch {
                key: "top_n".to_string(),
                value: "25".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        assert_eq!(stored.top_n, 25);
        assert_eq!(state.config.read().await.top_n, 25);
    }

    #[tokio::test]
    async fn config_patch_flags_next_cycle_only_for_refresh_relevant_edits() {
        // A settings tweak never re-fetches at once, so the dashboard must be
        // told when the edit lands on the next cycle instead.
        async fn patch_code(state: &HttpState, key: &str, value: &str) -> serde_json::Value {
            let (headers, query, connect) = no_auth();
            let response = super::api_config_patch(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
                axum::Json(super::ConfigPatch {
                    key: key.to_string(),
                    value: value.to_string(),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "key: {key}");
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body reads");
            serde_json::from_slice(&body).expect("mutation is json")
        }
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let changed = patch_code(&state, "top_n", "25").await;
        assert_eq!(
            changed["code"],
            serde_json::Value::String(super::APPLIES_NEXT_CYCLE_CODE.to_string()),
            "refresh-relevant edit flags next-cycle: {changed}"
        );
        let plain = patch_code(&state, "bind", "127.0.0.1:27142").await;
        assert!(
            plain.get("code").is_none(),
            "restart-only edit stays flag-free: {plain}"
        );
    }

    #[tokio::test]
    async fn config_reset_flags_next_cycle_only_when_refresh_data_moved() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let db = state.database.clone().expect("test db present");
        let reset_once = || async {
            let (headers, query, connect) = no_auth();
            let response = super::api_config_reset(
                axum::extract::State(state.clone()),
                headers,
                query,
                connect,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body reads");
            serde_json::from_slice::<serde_json::Value>(&body).expect("mutation is json")
        };
        let pristine = reset_once().await;
        assert!(
            pristine.get("code").is_none(),
            "reset over defaults stays flag-free: {pristine}"
        );
        {
            let mut cfg = crate::settings::load_app_config(&db).expect("seed loads");
            cfg.top_n = cfg.top_n.saturating_add(5);
            crate::settings::save_app_config(&db, &cfg).expect("customizes");
        }
        let moved = reset_once().await;
        assert_eq!(
            moved["code"],
            serde_json::Value::String(super::APPLIES_NEXT_CYCLE_CODE.to_string()),
            "reset restoring refresh settings flags next-cycle: {moved}"
        );
    }

    #[test]
    fn web_setting_validators_mirror_tui_rules() {
        let mut cfg = crate::config::AppConfig::default_for_first_run();
        assert!(super::apply_web_setting(&mut cfg, "top_n", "10").is_ok());
        assert_eq!(cfg.top_n, 10);
        assert!(super::apply_web_setting(&mut cfg, "top_n", "0").is_err());
        assert!(super::apply_web_setting(&mut cfg, "probe.mode", "tcp").is_ok());
        assert!(super::apply_web_setting(&mut cfg, "probe.mode", "carrier-pigeon").is_err());
        assert!(
            super::apply_web_setting(&mut cfg, "probe.accepted_statuses", "[204, 200]").is_ok()
        );
        assert_eq!(cfg.probe.accepted_statuses, vec![204, 200]);
        assert!(super::apply_web_setting(&mut cfg, "probe.accepted_statuses", "204,200").is_ok());
        assert!(super::apply_web_setting(&mut cfg, "probe.accepted_statuses", "999").is_err());
        assert!(super::apply_web_setting(&mut cfg, "probe.download_url", "off").is_ok());
        assert_eq!(cfg.probe.download_url, None);
        assert!(
            super::apply_web_setting(&mut cfg, "probe.download_url", "https://example.com/1gb")
                .is_ok()
        );
        assert_eq!(
            cfg.probe.download_url.as_deref(),
            Some("https://example.com/1gb")
        );
        // New keys mirror the TUI validators exactly.
        assert!(super::apply_web_setting(&mut cfg, "use_cache_only", "yes").is_ok());
        assert!(cfg.use_cache_only);
        assert!(super::apply_web_setting(&mut cfg, "use_cache_only", "maybe").is_err());
        assert!(super::apply_web_setting(&mut cfg, "emergency_config", "").is_ok());
        assert_eq!(cfg.emergency_config, None);
        assert!(
            super::apply_web_setting(&mut cfg, "emergency_config", "vless://u@h:443#e").is_ok()
        );
        assert_eq!(cfg.emergency_config.as_deref(), Some("vless://u@h:443#e"));
        assert!(
            super::apply_web_setting(&mut cfg, "probe.sing_box_path", "/usr/bin/sing-box").is_ok()
        );
        assert_eq!(cfg.probe.sing_box_path, "/usr/bin/sing-box");
        assert!(!cfg.probe.sing_box_path_auto);
        assert!(super::apply_web_setting(&mut cfg, "probe.connect_timeout_ms", "3000").is_ok());
        assert_eq!(cfg.probe.connect_timeout_ms, 3000);
        assert!(super::apply_web_setting(&mut cfg, "probe.connect_timeout_ms", "0").is_err());
        assert!(super::apply_web_setting(&mut cfg, "probe.process_concurrency", "auto").is_ok());
        assert_eq!(cfg.probe.process_concurrency, None);
        assert!(super::apply_web_setting(&mut cfg, "probe.process_concurrency", "3").is_ok());
        assert_eq!(cfg.probe.process_concurrency, Some(3));
        assert!(super::apply_web_setting(&mut cfg, "probe.process_concurrency", "0").is_err());
        assert!(super::apply_web_setting(&mut cfg, "proxy.rotating_proxy", "0").is_ok());
        assert!(!cfg.proxy.rotating_proxy);
        assert!(super::apply_web_setting(&mut cfg, "proxy.health_check_url", "").is_err());
        assert!(
            super::apply_web_setting(&mut cfg, "proxy.health_check_url", "https://1.1.1.1").is_ok()
        );
        assert_eq!(cfg.proxy.health_check_url, "https://1.1.1.1");
        assert!(
            super::apply_web_setting(&mut cfg, "proxy.health_check_interval_seconds", "30").is_ok()
        );
        assert_eq!(cfg.proxy.health_check_interval_seconds, 30);
        assert!(
            super::apply_web_setting(&mut cfg, "proxy.health_check_interval_seconds", "0").is_err()
        );
        assert!(super::apply_web_setting(&mut cfg, "clean_offlines_after_days", "14").is_ok());
        assert_eq!(cfg.clean_offlines_after_days, 14);
        assert!(super::apply_web_setting(&mut cfg, "clean_offlines_after_days", "0").is_err());
        assert!(super::apply_web_setting(&mut cfg, "geoip_db_path", "").is_ok());
        assert_eq!(cfg.geoip_db_path, None);
        assert!(super::apply_web_setting(&mut cfg, "geoip_db_path", "/data/geo.mmdb").is_ok());
        assert_eq!(cfg.geoip_db_path.as_deref(), Some("/data/geo.mmdb"));
        assert!(super::apply_web_setting(&mut cfg, "unknown.key", "1").is_err());
    }

    #[test]
    fn web_setting_accepts_human_input_from_any_keyboard() {
        // Regression: Persian-digit input rejected with
        // "refresh_seconds must be a number" on every OS.
        let mut cfg = crate::config::AppConfig::default_for_first_run();
        assert!(super::apply_web_setting(&mut cfg, "refresh_seconds", "۹۰۰").is_ok());
        assert_eq!(cfg.refresh_seconds, 900);
        assert!(super::apply_web_setting(&mut cfg, "ping_seconds", "٣٠٠").is_ok());
        assert_eq!(cfg.ping_seconds, 300);
        assert!(super::apply_web_setting(&mut cfg, "top_n", "１０").is_ok());
        assert_eq!(cfg.top_n, 10);
        // BIDI isolates hitchhiking from RTL paste or isolated rendering.
        assert!(super::apply_web_setting(&mut cfg, "refresh_seconds", " 900 ").is_ok());
        assert_eq!(cfg.refresh_seconds, 900);
        assert!(
            super::apply_web_setting(&mut cfg, "refresh_seconds", "\u{2066}900\u{2069}").is_ok()
        );
        assert_eq!(cfg.refresh_seconds, 900);
        assert!(
            super::apply_web_setting(&mut cfg, "probe.accepted_statuses", "[۲۰۴, ۲۰۰]").is_ok()
        );
        assert_eq!(cfg.probe.accepted_statuses, vec![204, 200]);
        assert_eq!(cfg.probe.accepted_statuses, vec![204, 200]);
        // Text keeps its characters: isolates stripped, content untouched.
        assert!(
            super::apply_web_setting(&mut cfg, "emergency_config", "vless://u@h:443#e").is_ok()
        );
        assert_eq!(cfg.emergency_config.as_deref(), Some("vless://u@h:443#e"));
        // Genuine garbage still refuses.
        assert!(super::apply_web_setting(&mut cfg, "refresh_seconds", "abc").is_err());
        assert!(super::apply_web_setting(&mut cfg, "top_n", "0").is_err());
    }

    fn test_sub(name: &str, url: &str, priority: u32) -> crate::config::SubscriptionSource {
        crate::config::SubscriptionSource {
            name: name.to_string(),
            url: url.to_string(),
            enabled: true,
            priority,
        }
    }

    fn stored_names(state: &HttpState) -> Vec<String> {
        let cfg = stored_config(state);
        cfg.subscriptions.iter().map(|s| s.name.clone()).collect()
    }

    #[tokio::test]
    async fn patch_echo_never_moves_a_row() {
        // Regression: on a legacy-order database the dashboard showed vec
        // order while PATCH indexed database order, so saving row N edited a
        // stranger (and duplicated names). Loads converge now, so an echo of
        // the prefilled values is a no-op move on every surface.
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        {
            let db = state.database.as_ref().expect("test db present");
            let mut cfg = crate::settings::load_app_config(db).expect("seed loads");
            cfg.subscriptions = vec![
                test_sub("src-1", "https://x.com/1.txt", 30),
                test_sub("src-2", "https://x.com/2.txt", 10),
                test_sub("src-3", "https://x.com/3.txt", 20),
            ];
            // Bypass validation to store the legacy shape verbatim.
            crate::settings::save_app_config(db, &cfg).expect("legacy seeds");
        }
        // The mutation load converges to priority order, dense: this is the
        // order the dashboard shows, so position 0 addresses src-2.
        let loaded =
            crate::settings::load_app_config(state.database.as_ref().expect("test db present"))
                .expect("loads converge");
        let names: Vec<&str> = loaded
            .subscriptions
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["src-2", "src-3", "src-1"]);
        // Echo-save row 0 (only the URL changes, priority echoed back):
        // order and names preserved, nothing teleports, nothing duplicates.
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::extract::Path(0_usize),
            axum::Json(super::SubscriptionPatch {
                url: Some("https://x.com/2b.txt".to_string()),
                name: Some("src-2".to_string()),
                priority: Some(1),
                enabled: Some(true),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(stored_names(&state), vec!["src-2", "src-3", "src-1"]);
        let stored = stored_config(&state);
        assert_eq!(stored.subscriptions[0].url, "https://x.com/2b.txt");
        // An explicit rank change still moves the row to its slot.
        let (headers, query, connect) = no_auth();
        let response = super::api_subscriptions_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::extract::Path(2_usize),
            axum::Json(super::SubscriptionPatch {
                url: None,
                name: None,
                priority: Some(1),
                enabled: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(stored_names(&state), vec!["src-1", "src-2", "src-3"]);
    }

    #[tokio::test]
    async fn require_token_without_a_token_mints_one() {
        // The reported brick: the switch persisted protection with no token
        // and the next settings load failed. Flipping it on must mint a
        // random editable token that survives a reload.
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response = super::api_config_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ConfigPatch {
                key: "sharing.require_token".to_string(),
                value: "true".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        assert!(stored.sharing.require_token);
        assert!(
            !stored.sharing.token.trim().is_empty(),
            "a token is minted and the reload validates"
        );
        // Clearing the token while protection is on re-mints instead of
        // storing the trap.
        let (headers, query, connect) = no_auth();
        let response = super::api_config_patch(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ConfigPatch {
                key: "sharing.token".to_string(),
                value: String::new(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        assert!(!stored.sharing.token.trim().is_empty());
    }

    #[test]
    fn share_urls_tokenize_protected_links() {
        // The dashboard masks the raw token, so the Share tab could never
        // offer a working protected link. These server-built URLs carry
        // ?token= like the QR sheet does.
        let mut cfg = crate::config::AppConfig::default_for_first_run();
        cfg.sharing.enabled = true;
        cfg.sharing.require_token = true;
        cfg.sharing.token = "secret".to_string();
        let runtime = crate::model::RuntimeConfig::from(&cfg);
        let urls = super::share_urls(&runtime, "192.0.2.2");
        assert_eq!(urls.len(), 3);
        let bodies: Vec<&str> = urls.iter().map(|u| u.url.as_str()).collect();
        assert!(
            bodies
                .iter()
                .all(|u| u.starts_with("http://192.0.2.2:27141/"))
        );
        assert!(bodies.iter().all(|u| u.contains("?token=secret")));
        assert!(bodies.iter().any(|u| u.contains("/subscription?token=")));
        assert!(
            bodies
                .iter()
                .any(|u| u.contains("/subscription.txt?token="))
        );
        assert!(bodies.iter().any(|u| u.contains("/mihomo.yaml?token=")));

        // Unprotected sharing serves bare URLs.
        let mut cfg = crate::config::AppConfig::default_for_first_run();
        cfg.sharing.enabled = true;
        cfg.sharing.require_token = false;
        cfg.sharing.token.clear();
        let runtime = crate::model::RuntimeConfig::from(&cfg);
        let urls = super::share_urls(&runtime, "192.0.2.2");
        assert!(urls.iter().all(|u| !u.url.contains("?token=")));
    }

    #[tokio::test]
    async fn share_urls_empty_while_sharing_off() {
        // Deterministic without touching the network: discovery runs only
        // when sharing is on.
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let (headers, query, connect) = no_auth();
        let response =
            super::api_share_urls(axum::extract::State(state), headers, query, connect).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = String::from_utf8(body.to_vec()).expect("body is text");
        assert!(
            text.contains("\"urls\":[]"),
            "no links while sharing is off"
        );
    }

    #[tokio::test]
    async fn config_token_reveals_to_authorized_viewer() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let db = state.database.clone().expect("test db present");
        {
            let mut cfg = crate::settings::load_app_config(&db).expect("seed loads");
            cfg.sharing.require_token = true;
            cfg.sharing.token = "viewer-token".to_string();
            crate::settings::save_app_config(&db, &cfg).expect("customizes");
        }
        // Push the customized config live like persist_config would.
        {
            let cfg = crate::settings::load_app_config(&db).expect("reloads");
            *state.config.write().await = crate::model::RuntimeConfig::from(&cfg);
        }
        let (headers, query, connect) = no_auth();
        let response =
            super::api_config_token(axum::extract::State(state.clone()), headers, query, connect)
                .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let text = String::from_utf8(body.to_vec()).expect("body is text");
        assert!(
            text.contains("viewer-token"),
            "explicit Show reveals the token"
        );
    }

    #[tokio::test]
    async fn config_reset_restores_defaults_and_keeps_subscriptions() {
        let (state, _refresh_rx, _ping_rx, _config_rx) = mutation_state(RuntimeState::default());
        let db = state.database.clone().expect("test db present");
        let embedded = crate::settings::embedded_defaults();
        {
            let mut cfg = crate::settings::load_app_config(&db).expect("seed loads");
            cfg.top_n = embedded.top_n.saturating_add(5);
            cfg.emergency_config = Some("vless://uuid@example.com:443#bridge".to_string());
            cfg.probe.sing_box_path = "/tmp/sing-box".to_string();
            cfg.subscriptions
                .push(test_sub("mine", "https://test.invalid/mine.txt", 99));
            crate::settings::save_app_config(&db, &cfg).expect("customizes");
        }
        let (headers, query, connect) = no_auth();
        let response =
            super::api_config_reset(axum::extract::State(state.clone()), headers, query, connect)
                .await;
        assert_eq!(response.status(), StatusCode::OK);
        let stored = stored_config(&state);
        assert_eq!(stored.top_n, embedded.top_n);
        assert_eq!(
            stored.emergency_config.as_deref(),
            Some("vless://uuid@example.com:443#bridge"),
            "reset keeps the user-filled emergency bridge"
        );
        assert_eq!(
            stored.probe.sing_box_path, "/tmp/sing-box",
            "reset keeps the manual sing-box path"
        );
        let names = stored_names(&state);
        assert!(names.contains(&"mine".to_string()), "custom sub kept");
        let snapshot = db.load_defaults_snapshot().expect("snapshot reads");
        assert!(snapshot.is_some(), "reset re-anchors the baseline");
        {
            let live = state.subscriptions.read().await;
            assert!(live.iter().any(|s| s.name == "mine"));
            drop(live);
        }
    }

    #[tokio::test]
    async fn proxy_select_pins_live_without_touching_disk() {
        let (state, _refresh_rx, _ping_rx, mut config_rx) = mutation_state(RuntimeState::default());
        let db = state.database.as_ref().expect("test db present").clone();
        let before = db.load_settings().expect("settings read");
        let (headers, query, connect) = no_auth();
        let uri = "vless://uuid@example.com:443?security=tls#Node".to_string();
        let response = super::api_proxy_select(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ProxySelect {
                uri: Some(uri.clone()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        config_rx.changed().await.expect("live broadcast fires");
        assert_eq!(
            config_rx.borrow().proxy.manual_proxy_uri.as_deref(),
            Some(uri.as_str())
        );
        let after = db.load_settings().expect("settings read");
        assert_eq!(before, after, "proxy select never writes the database");
    }

    #[tokio::test]
    async fn proxy_mode_and_sharing_preserve_live_pin() {
        // A manual pin is live-only (never written). Toggling proxy mode or
        // sharing rebuilt the config from disk and silently dropped it from
        // both the file and the runtime; both toggles now build on live.
        let (state, _refresh_rx, _ping_rx, mut config_rx) = mutation_state(RuntimeState::default());
        let uri = "vless://uuid@example.com:443?security=tls#Node".to_string();
        let (headers, query, connect) = no_auth();
        let response = super::api_proxy_select(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ProxySelect {
                uri: Some(uri.clone()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        config_rx.changed().await.expect("pin broadcast fires");

        let (headers, query, connect) = no_auth();
        let response = super::api_proxy_mode(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ProxyMode { mode: None }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        config_rx.changed().await.expect("mode broadcast fires");
        assert_eq!(
            config_rx.borrow().proxy.manual_proxy_uri.as_deref(),
            Some(uri.as_str()),
            "proxy mode toggle keeps the live pin"
        );
        assert!(config_rx.borrow().proxy.enabled);

        let (headers, query, connect) = no_auth();
        let response =
            super::api_sharing(axum::extract::State(state.clone()), headers, query, connect).await;
        assert_eq!(response.status(), StatusCode::OK);
        config_rx.changed().await.expect("sharing broadcast fires");
        assert_eq!(
            config_rx.borrow().proxy.manual_proxy_uri.as_deref(),
            Some(uri.as_str()),
            "sharing toggle keeps the live pin"
        );
        assert!(config_rx.borrow().sharing.enabled);
    }

    async fn call_proxy_mode(
        state: &HttpState,
        config_rx: &mut tokio::sync::watch::Receiver<crate::config::AppConfig>,
        mode: Option<&str>,
    ) -> (axum::http::StatusCode, crate::config::AppConfig) {
        let (headers, query, connect) = no_auth();
        let response = super::api_proxy_mode(
            axum::extract::State(state.clone()),
            headers,
            query,
            connect,
            axum::Json(super::ProxyMode {
                mode: mode.map(str::to_string),
            }),
        )
        .await;
        let status = response.status();
        if status == StatusCode::OK {
            // persist_config broadcasts every accepted change.
            config_rx.changed().await.expect("mode broadcast fires");
        }
        (status, config_rx.borrow().clone())
    }

    #[tokio::test]
    async fn proxy_mode_sets_directly_rejects_unknown_and_cycles_legacy() {
        let (state, _refresh_rx, _ping_rx, mut config_rx) = mutation_state(RuntimeState::default());

        let (status, config) = call_proxy_mode(&state, &mut config_rx, Some("lan")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(config.proxy.enabled && config.proxy.discoverable);

        let (status, config) = call_proxy_mode(&state, &mut config_rx, Some("local")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(config.proxy.enabled && !config.proxy.discoverable);

        let (status, config) = call_proxy_mode(&state, &mut config_rx, Some("  OFF ")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!config.proxy.enabled && !config.proxy.discoverable);

        let (status, _) = call_proxy_mode(&state, &mut config_rx, Some("turbo")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Omitted mode keeps the legacy Off → Local → LAN cycle.
        let (status, config) = call_proxy_mode(&state, &mut config_rx, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(config.proxy.enabled && !config.proxy.discoverable);
    }

    #[tokio::test]
    async fn save_is_a_compatible_no_op() {
        let state = http_state(RuntimeState::default(), false);
        let (headers, query, connect) = no_auth();
        let response = super::api_save(axum::extract::State(state), headers, query, connect).await;

        assert_eq!(response.status(), StatusCode::OK);
    }
}
