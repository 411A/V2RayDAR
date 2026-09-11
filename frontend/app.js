"use strict";

/* V2RayDAR dashboard — vanilla JS, zero dependencies.
 * Data strategy (capability chain, cheapest first):
 *   1. GET /results          — exists on every server, full RuntimeState JSON.
 *   2. GET /api/events (SSE) — live deltas when the server provides them.
 *   3. GET /api/summary      — 2 s poll fallback when SSE is missing.
 *   4. Poll /results every 10 s when neither new endpoint exists.
 * Reads use the same auth model as the subscription endpoints (loopback is
 * trusted; LAN needs sharing + optional token, passed as ?token= like the
 * official clients do). Full credential URIs are never written into the DOM;
 * they travel in JS variables only and leave via explicit Copy clicks.
 */

const $ = (id) => document.getElementById(id);

const POLL_SUMMARY_MS = 2000;
const POLL_RESULTS_MS = 10000;
const ROW_LIMIT_KEY = "v2raydar-row-limit";
const MAX_LOGS = 300;
const MAX_TOASTS = 4;
const TOAST_MS = 4500;
const THEME_KEY = "v2raydar-theme";
const TAB_KEY = "v2raydar-tab";

const TABS = ["overview", "configs", "subscriptions", "settings", "proxy", "logs", "share"];

const state = {
  snapshot: null,
  feed: "boot", // boot | live | polling | offline | locked
  sse: null,
  pollTimer: 0,
  subs: null, // { list, dirty } when /api/subscriptions exists
  settings: null, // server settings payload when /api/config exists
  logLines: [],
  detailUri: "",
  qrUri: "",
  qrObjectUrl: "",
  ovQrObjectUrl: "",
  qrKnown: null, // null = unknown; mirrors /api/summary qr_available
  ovQrLoaded: false, // overview QR image already shown for current sheet
  ovQrProbing: false, // capability probe in flight (do not stack them)
  hasSummaryApi: null, // null = unknown; true once any summary-shaped answer arrives
  startedAt: null, // server start (RFC3339) — "Running For" clock
  refreshSeconds: 0, // countdown cadence; 0 = unknown/manual
  pingSeconds: 0,
  clockTimer: 0, // wall-clock-aligned ticker timeout (uptime + countdowns)
  ovConfig: null, // cached /api/config payload for the overview card
  search: "",
  reachableOnly: true,
  rowLimit: 0, // 0 = All; otherwise 25 | 50 | 100
  logFilter: "",
  follow: true,
};

function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined && text !== null) {
    node.textContent = String(text);
  }
  if (className) {
    node.className = className;
  }
  return node;
}

function getToken() {
  try {
    const q = new URLSearchParams(window.location.search);
    const t = q.get("token");
    return t ? t : "";
  } catch (err) {
    return "";
  }
}

function apiPath(path) {
  const token = getToken();
  if (!token) {
    return path;
  }
  const sep = path.includes("?") ? "&" : "?";
  return path + sep + "token=" + encodeURIComponent(token);
}

async function fetchJson(path, options) {
  const init = { headers: { Accept: "application/json" } };
  if (options && options.method) {
    init.method = options.method;
    init.headers["Content-Type"] = "application/json";
    if (options.body !== undefined) {
      init.body = JSON.stringify(options.body);
    }
  }
  let res;
  try {
    res = await fetch(apiPath(path), init);
  } catch (err) {
    return { status: 0, data: null };
  }
  let data = null;
  try {
    const text = await res.text();
    data = text ? JSON.parse(text) : null;
  } catch (err) {
    data = null;
  }
  return { status: res.status, data };
}

/* ---------- toasts / status / banner ---------- */

function toast(message, kind) {
  const box = $("toasts");
  while (box.children.length >= MAX_TOASTS) {
    box.removeChild(box.firstChild);
  }
  const t = el("div", message, "toast" + (kind ? " " + kind : ""));
  box.appendChild(t);
  window.setTimeout(() => {
    if (t.parentNode === box) {
      box.removeChild(t);
    }
  }, TOAST_MS);
}

function setStatus(message) {
  const node = $("status-text");
  if (node) {
    node.textContent = message;
  }
}

function setDirty(dirty) {
  const pill = $("status-dirty");
  if (pill) {
    pill.textContent = dirty ? "unsaved changes" : "saved";
    pill.classList.toggle("warn", dirty);
    pill.classList.toggle("good", !dirty);
  }
  const btn = $("btn-save");
  if (btn) {
    btn.disabled = !dirty;
    btn.textContent = dirty ? "Save now" : "Save (no changes)";
  }
}

function showBanner(title, text, showRetry) {
  $("banner-title").textContent = title;
  $("banner-text").textContent = text;
  $("banner-retry").hidden = !showRetry;
  $("banner").hidden = false;
}

function hideBanner() {
  $("banner").hidden = true;
}

/* ---------- theme ---------- */

function applyTheme(mode) {
  const root = document.documentElement;
  root.setAttribute("data-theme", mode);
  let dark = mode === "dark";
  if (mode === "system" && window.matchMedia) {
    try {
      dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    } catch (err) {
      dark = false;
    }
  }
  try {
    root.style.colorScheme = dark ? "dark" : "light";
  } catch (err) {
    /* older browsers ignore colorScheme; CSS still applies */
  }
  for (const m of ["system", "light", "dark"]) {
    $("theme-" + m).setAttribute("aria-pressed", m === mode ? "true" : "false");
  }
}

function loadTheme() {
  let saved = "system";
  try {
    const v = window.localStorage.getItem(THEME_KEY);
    if (v === "light" || v === "dark" || v === "system") {
      saved = v;
    }
  } catch (err) {
    saved = "system";
  }
  applyTheme(saved);
}

function saveTheme(mode) {
  try {
    window.localStorage.setItem(THEME_KEY, mode);
  } catch (err) {
    /* private mode etc. — theme still applies for the session */
  }
  applyTheme(mode);
}

function loadRowLimit() {
  try {
    const v = window.localStorage.getItem(ROW_LIMIT_KEY);
    if (v === "25" || v === "50" || v === "100") {
      return Number(v);
    }
  } catch (err) {
    /* private mode etc. — fall through to All */
  }
  return 0;
}

function saveRowLimit(limit) {
  try {
    window.localStorage.setItem(ROW_LIMIT_KEY, limit > 0 ? String(limit) : "all");
  } catch (err) {
    /* private mode etc. — limit still applies for the session */
  }
}

/* ---------- formatting ---------- */

function fmtBytes(n) {
  if (n === null || n === undefined) {
    return "—";
  }
  const v = Number(n);
  if (!Number.isFinite(v)) {
    return "—";
  }
  if (v < 1024) {
    return v + " B";
  }
  if (v < 1048576) {
    return (v / 1024).toFixed(1) + " KB";
  }
  if (v < 1073741824) {
    return (v / 1048576).toFixed(1) + " MB";
  }
  return (v / 1073741824).toFixed(2) + " GB";
}

function fmtLatency(ms) {
  if (ms === null || ms === undefined) {
    return "—";
  }
  const v = Number(ms);
  if (!Number.isFinite(v)) {
    return "—";
  }
  if (v < 1000) {
    return Math.round(v) + " ms";
  }
  return (v / 1000).toFixed(1) + " s";
}

function fmtStamp(iso) {
  if (!iso) {
    return "—";
  }
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) {
    return "—";
  }
  const pad = (n) => String(n).padStart(2, "0");
  return d.getFullYear() + "/" + pad(d.getMonth() + 1) + "/" + pad(d.getDate()) +
    " " + pad(d.getHours()) + ":" + pad(d.getMinutes()) + ":" + pad(d.getSeconds());
}

function fmtAgo(iso) {
  if (!iso) {
    return "—";
  }
  const t = Date.parse(iso);
  if (Number.isNaN(t)) {
    return "—";
  }
  const s = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (s < 10) {
    return "just now";
  }
  if (s < 60) {
    return s + "s ago";
  }
  const m = Math.floor(s / 60);
  if (m < 60) {
    return m + "m ago";
  }
  const h = Math.floor(m / 60);
  if (h < 48) {
    return h + "h ago";
  }
  return Math.floor(h / 24) + "d ago";
}

function fmtDuration(ms) {
  if (ms === null || ms === undefined) {
    return "—";
  }
  const v = Number(ms);
  if (!Number.isFinite(v)) {
    return "—";
  }
  if (v < 1000) {
    return Math.round(v) + " ms";
  }
  if (v < 60000) {
    return (v / 1000).toFixed(1) + " s";
  }
  return (v / 60000).toFixed(1) + " min";
}

function truncate(text, max) {
  const s = String(text);
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

/// TUI-style clock `00:02:08` for the "Running For" card.
function fmtHMS(totalSeconds) {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) {
    return "—";
  }
  const s = Math.floor(totalSeconds);
  const h = String(Math.floor(s / 3600)).padStart(2, "0");
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, "0");
  const sec = String(s % 60).padStart(2, "0");
  return h + ":" + m + ":" + sec;
}

/// Compact countdown for the Refresh card (`14:18`, or `1:02:03` past an hour).
function fmtCountdown(ms) {
  if (!Number.isFinite(ms) || ms < 0) {
    return "—";
  }
  const s = Math.ceil(ms / 1000);
  if (s >= 3600) {
    return fmtHMS(s);
  }
  return String(Math.floor(s / 60)).padStart(2, "0") + ":" + String(s % 60).padStart(2, "0");
}

function uptimeText() {
  if (!state.startedAt) {
    return "—";
  }
  const t = Date.parse(state.startedAt);
  if (Number.isNaN(t)) {
    return "—";
  }
  return fmtHMS((Date.now() - t) / 1000);
}

/// Second line of the Refresh card: live ping countdown, same shape as the
/// fetch line above it. Anchored at the server's last deadline (re)arm, so it
/// ticks down in real time via the 1s ticker and survives idle stretches with
/// no feed traffic — exactly like `refreshVal`.
function pingSub() {
  const s = state.snapshot;
  if (s && s.pinging) {
    return "ping running…";
  }
  if (!state.hasSummaryApi) {
    return "";
  }
  if (state.pingSeconds <= 0) {
    return "ping off";
  }
  if (state.refreshSeconds > 0 && state.pingSeconds === state.refreshSeconds) {
    return "ping with refresh";
  }
  const anchor = s && s.last_ping_at ? Date.parse(s.last_ping_at) : NaN;
  if (Number.isNaN(anchor)) {
    // Old server (or no arm yet): static interval until the anchor arrives.
    return "ping every " + state.pingSeconds + "s";
  }
  return "ping in " + fmtCountdown(Math.max(0, anchor + state.pingSeconds * 1000 - Date.now()));
}

/// First line of the Refresh card (mirrors the TUI's fetch line).
function refreshVal() {
  const s = state.snapshot;
  if (!s) {
    return "—";
  }
  if (s.refreshing) {
    const start = s.refresh_started_at ? Date.parse(s.refresh_started_at) : NaN;
    return "running" + (Number.isNaN(start) ? "" : " " + fmtHMS((Date.now() - start) / 1000));
  }
  if (!state.hasSummaryApi || state.refreshSeconds <= 0) {
    return state.hasSummaryApi ? "manual" : "—";
  }
  const finSrc = s.refresh_finished_at || s.last_refresh;
  const fin = finSrc ? Date.parse(finSrc) : NaN;
  if (Number.isNaN(fin)) {
    return "—";
  }
  return "fetch in " + fmtCountdown(Math.max(0, fin + state.refreshSeconds * 1000 - Date.now()));
}

function refreshStatus() {
  return { val: refreshVal(), sub: pingSub() };
}

function lastUpdateAgo() {
  if (state.snapshot && state.snapshot.last_refresh) {
    return fmtAgo(state.snapshot.last_refresh);
  }
  return "the last update";
}

function maskedHost(endpoint) {
  if (!endpoint || !endpoint.host) {
    return "—";
  }
  return endpoint.host + ":" + endpoint.port;
}

function flagFor(cc) {
  if (cc === null || cc === undefined) {
    return "";
  }
  const up = String(cc).toUpperCase();
  if (!/^[A-Z]{2}$/.test(up)) {
    return "";
  }
  return String.fromCodePoint(up.charCodeAt(0) + 0x1F1A5, up.charCodeAt(1) + 0x1F1A5);
}

/* ---------- connection / feed ---------- */

function setConn(connState, detail) {
  const box = document.querySelector(".conn");
  if (box) {
    box.setAttribute("data-state", connState);
  }
  const label = connState === "online" ? "connected" : connState === "locked" ? "locked" : "offline";
  $("conn-state").textContent = detail ? label + " · " + detail : label;
  $("conn-bind").textContent = window.location.host || "local";
}

/* Single atomic feed-state transition: state.feed + pill + banner + footer.
 * All feed code must go through here; setConn/showBanner/hideBanner/setStatus
 * below are low-level helpers owned by this function (transient action
 * footers in triggerCycle/selectProxy/saveNow never touch pill or banner).
 */
function setFeedStatus(feed, opts) {
  const o = opts || {};
  state.feed = feed;
  if (feed === "live") {
    setConn("online", "live feed");
    hideBanner();
    setStatus(o.status || "Live — receiving real-time updates.");
    return;
  }
  if (feed === "polling") {
    const detail = o.detail || "poll 10 s";
    setConn("online", detail);
    hideBanner();
    if (o.status) {
      setStatus(o.status);
    } else if (detail === "poll 2 s") {
      setStatus("Polling summary every 2 s.");
    } else {
      setStatus("Last update " + lastUpdateAgo() + " · polling every 10 s (no live feed on this server).");
    }
    return;
  }
  if (feed === "locked") {
    const sharingOff = o.lockKind === "sharing";
    setConn("locked", sharingOff ? "LAN sharing off" : "token required");
    showBanner(
      "Access denied",
      sharingOff
        ? "LAN sharing is disabled on this instance. Enable sharing in the TUI or open the dashboard on the device itself."
        : "This instance requires a token. Open the dashboard URL that includes ?token=… (same token as the subscription URL).",
      false
    );
    setStatus(o.status || "Locked.");
    return;
  }
  if (feed === "boot") {
    setConn("offline", "connecting");
    hideBanner();
    setStatus("Connecting…");
    return;
  }
  setConn("offline", o.connDetail || "server unreachable");
  if (o.bannerTitle !== undefined || o.bannerText !== undefined) {
    showBanner(o.bannerTitle || "Connection lost", o.bannerText || "", o.retry !== false);
  } else if (state.snapshot) {
    showBanner(
      "Connection lost",
      "Lost contact with the instance. Showing data from " + lastUpdateAgo() + ". Retrying automatically.",
      true
    );
  } else {
    showBanner("Server unreachable", "Could not reach the V2RayDAR instance. Is it running?", true);
  }
  setStatus(o.status || "Offline — retrying automatically.");
}

async function loadResults() {
  const r = await fetchJson("/results");
  if (r.status === 0) {
    setFeedStatus("offline", { connDetail: "server unreachable" });
    return false;
  }
  if (r.status === 401 || r.status === 403) {
    setFeedStatus("locked", { lockKind: r.status === 401 ? "token" : "sharing" });
    return false;
  }
  if (r.status !== 200 || !r.data) {
    setFeedStatus("offline", {
      connDetail: "HTTP " + r.status,
      bannerTitle: "Unexpected response",
      bannerText: "GET /results returned HTTP " + r.status + ".",
      status: "Unexpected response.",
    });
    return false;
  }
  state.snapshot = r.data;
  ingestLogs(r.data);
  renderAll();
  if (state.feed !== "live" && state.feed !== "polling") {
    // Fresh data but no transport yet — the caller (boot/retry/reconnect)
    // upgrades to live or polling immediately; never leave a stale banner.
    setFeedStatus("polling", { detail: "poll 10 s" });
  }
  return true;
}

function connectFeed() {
  // Prefer SSE; fall back to summary polling, then full polling.
  if (!("EventSource" in window)) {
    startPolling(false);
    return;
  }
  let sse;
  try {
    sse = new EventSource(apiPath("/api/events"));
  } catch (err) {
    startPolling(false);
    return;
  }
  state.sse = sse;
  let opened = false;

  sse.addEventListener("hello", (ev) => {
    opened = true;
    setFeedStatus("live");
    applyHello(ev.data);
    // Proves the modern API surface AND stores capabilities (uptime origin,
    // cadences, QR flag). Must run unblocked: it no-ops once flags are set.
    void syncCapabilitiesOnce();
  });
  sse.addEventListener("log", (ev) => pushLogLine(ev.data));
  sse.addEventListener("probe-delta", (ev) => applyProbeDelta(ev.data));
  sse.addEventListener("ranked", (ev) => applyRanked(ev.data));
  sse.addEventListener("config-changed", () => {
    toast("Configuration changed on the server — resyncing.", "good");
    void loadResults();
    void loadOvConfig();
  });
  sse.onerror = () => {
    try {
      sse.close();
    } catch (err) {
      /* already closed */
    }
    state.sse = null;
    if (!opened) {
      // No live feed on this server version — probe the fallback chain.
      startPolling(false);
    } else {
      setFeedStatus("offline", {
        connDetail: "feed lost",
        bannerTitle: "Live feed lost",
        bannerText: "Reconnecting automatically…",
        status: "Reconnecting…",
      });
      window.setTimeout(() => {
        void loadResults().then((ok) => {
          if (ok) {
            connectFeed();
          } else {
            startPolling(false);
          }
        });
      }, 3000);
    }
  };
}

function startPolling(summaryAvailable) {
  stopPolling();
  const useSummary = summaryAvailable;
  const tick = async () => {
    if (useSummary) {
      const r = await fetchJson("/api/summary");
      if (r.status === 200 && r.data) {
        setFeedStatus("polling", { detail: "poll 2 s" });
        applySummary(r.data);
        return;
      }
    }
    const ok = await loadResults();
    if (ok) {
      setFeedStatus("polling", { detail: "poll 10 s" });
    }
  };
  const checkSummary = async () => {
    const r = await fetchJson("/api/summary");
    if (r.status === 200 && r.data) {
      state.hasSummaryApi = true;
      setFeedStatus("polling", { detail: "poll 2 s" });
      applySummary(r.data);
      state.pollTimer = window.setInterval(tickSummary, POLL_SUMMARY_MS);
    } else {
      state.hasSummaryApi = false;
      void tick();
      state.pollTimer = window.setInterval(tick, POLL_RESULTS_MS);
    }
  };
  const tickSummary = async () => {
    const r = await fetchJson("/api/summary");
    if (r.status === 200 && r.data) {
      applySummary(r.data);
    } else {
      stopPolling();
      const ok = await loadResults();
      if (ok) {
        setFeedStatus("polling", { detail: "poll 10 s" });
      }
      state.pollTimer = window.setInterval(tick, ok ? POLL_RESULTS_MS : POLL_RESULTS_MS);
    }
  };
  void checkSummary();
}

function stopPolling() {
  if (state.pollTimer) {
    window.clearInterval(state.pollTimer);
    state.pollTimer = 0;
  }
}

/* ---------- live-event application ---------- */

function safeParse(text) {
  try {
    return JSON.parse(text);
  } catch (err) {
    return null;
  }
}

function applyHello(text) {
  const msg = safeParse(text);
  if (msg && msg.snapshot) {
    state.snapshot = msg.snapshot;
    ingestLogs(msg.snapshot);
    renderAll();
  } else {
    void loadResults();
  }
}

function applySummary(sum) {
  if (!state.snapshot) {
    void loadResults();
    return;
  }
  const s = state.snapshot;
  for (const k of ["refreshing", "pinging", "total_candidates", "tested_candidates",
    "reachable_candidates", "fetch_bytes", "speedtest_bytes"]) {
    if (sum[k] !== undefined) {
      s[k] = sum[k];
    }
  }
  if (sum.proxy) {
    for (const k of ["proxy_running", "proxy_active_config", "proxy_active_uri",
      "proxy_port", "proxy_discoverable"]) {
      if (sum.proxy[k] !== undefined) {
        s[k] = sum.proxy[k];
      }
    }
  }
  if (Array.isArray(sum.ranked)) {
    s.ranked = sum.ranked;
  }
  if (typeof sum.last_error !== "undefined") {
    s.last_error = sum.last_error;
  }
  // Cycle timestamps: without these the cards stick on "running" and Last
  // scan never resolves after a poll-only update.
  for (const k of ["last_refresh", "refresh_started_at", "refresh_finished_at",
    "refresh_duration_ms", "last_ping_at"]) {
    if (typeof sum[k] !== "undefined") {
      s[k] = sum[k];
    }
  }
  if (Array.isArray(sum.fetch_errors)) {
    s.fetch_errors = sum.fetch_errors;
  }
  if (typeof sum.qr_available === "boolean") {
    state.qrKnown = sum.qr_available;
  }
  storeCapabilities(sum);
  renderAll();
}

/// Copies slow-moving server facts out of a summary payload (works for
/// `/api/summary` answers and the `hello` snapshot alike: unknown keys are
/// simply skipped, so old servers degrade to "—" instead of breaking).
function storeCapabilities(sum) {
  if (!sum || typeof sum !== "object") {
    return;
  }
  if (typeof sum.qr_available === "boolean") {
    state.qrKnown = sum.qr_available;
  }
  if (typeof sum.started_at === "string") {
    state.startedAt = sum.started_at;
  }
  if (typeof sum.refresh_seconds === "number") {
    state.refreshSeconds = sum.refresh_seconds;
  }
  if (typeof sum.ping_seconds === "number") {
    state.pingSeconds = sum.ping_seconds;
  }
}

/// One-shot capability sync (uptime origin, cadences, QR flag). Only called
/// where the server already proved modern (SSE hello) — never blindly at
/// boot, so old servers don't pay a 404 console error just for loading.
async function syncCapabilitiesOnce() {
  if (state.hasSummaryApi !== null) {
    return;
  }
  const r = await fetchJson("/api/summary");
  if (r.status === 200 && r.data) {
    state.hasSummaryApi = true;
    storeCapabilities(r.data);
  } else if (r.status === 404) {
    state.hasSummaryApi = false;
  }
  // status 0 (network loss): stay unknown so a later visit retries.
}

function applyProbeDelta(text) {
  const d = safeParse(text);
  if (!d || !state.snapshot) {
    return;
  }
  if (typeof d.tested === "number") {
    state.snapshot.tested_candidates = d.tested;
  }
  if (typeof d.working === "number") {
    state.snapshot.reachable_candidates = d.working;
  }
  if (typeof d.bytes === "number") {
    state.snapshot.fetch_bytes = (state.snapshot.fetch_bytes || 0) + d.bytes;
  }
  // Cycle state rides every delta (see web.rs feed_task) so the UI leaves
  // "running" the moment the backend finishes, even with identical counts.
  for (const k of ["refreshing", "pinging"]) {
    if (typeof d[k] === "boolean") {
      state.snapshot[k] = d[k];
    }
  }
  for (const k of ["last_refresh", "refresh_started_at", "refresh_finished_at",
    "refresh_duration_ms", "last_ping_at"]) {
    if (typeof d[k] !== "undefined") {
      state.snapshot[k] = d[k];
    }
  }
  if (Array.isArray(d.fetch_errors)) {
    state.snapshot.fetch_errors = d.fetch_errors;
  }
  renderStats();
  renderFetchErrors();
}

function applyRanked(text) {
  const d = safeParse(text);
  if (!d || !state.snapshot) {
    return;
  }
  state.snapshot.ranked = Array.isArray(d) ? d : d.ranked || state.snapshot.ranked;
  renderConfigs();
  renderShare();
}

function pushLogLine(text) {
  if (text === null || text === undefined) {
    return;
  }
  state.logLines.push(String(text));
  if (state.logLines.length > MAX_LOGS) {
    state.logLines.splice(0, state.logLines.length - MAX_LOGS);
  }
  renderLogs();
}

function ingestLogs(snap) {
  const lines = [];
  if (Array.isArray(snap.live_logs)) {
    for (const l of snap.live_logs) {
      lines.push(l);
    }
  } else if (Array.isArray(snap.logs)) {
    for (const l of snap.logs) {
      lines.push(l);
    }
  }
  state.logLines = lines.slice(-MAX_LOGS);
}

/* ---------- rendering ---------- */

function renderAll() {
  renderStats();
  renderOvLogs();
  renderOvConfigs();
  renderOvConfig();
  renderEndpoints();
  renderFetchErrors();
  renderConfigs();
  renderLogs();
  renderProxyTab();
  renderShare();
  void renderOvQr();
}

/// Stat badge: title on top, value in the middle, sub-line at the bottom.
/// The card is `min-inline-size: 0` with wrapping values — nothing may stick
/// out of the border at any viewport width.
function statCard(label, value, sub, valueCls, valueId, subId) {
  const p = el("p", null, "stat card");
  p.appendChild(el("span", label, "stat-label"));
  const v = el("span", value, "stat-value" + (valueCls ? " " + valueCls : ""));
  if (valueId) {
    v.id = valueId;
  }
  p.appendChild(v);
  if (sub) {
    const s = el("span", sub, "stat-sub");
    if (subId) {
      s.id = subId;
    }
    p.appendChild(s);
  } else if (subId) {
    const s = el("span", "", "stat-sub");
    s.id = subId;
    p.appendChild(s);
  }
  return p;
}

function renderStats() {
  const box = $("stat-cards");
  while (box.firstChild) {
    box.removeChild(box.firstChild);
  }
  const s = state.snapshot;
  if (!s) {
    box.appendChild(statCard("Status", "no data"));
    return;
  }
  const failed = Math.max(0, (s.tested_candidates || 0) - (s.reachable_candidates || 0));
  const tested = s.tested_candidates || 0;
  const rs = refreshStatus();
  const ago = fmtAgo(s.last_refresh);
  const took = s.refresh_duration_ms !== null && s.refresh_duration_ms !== undefined
    ? " · took " + fmtDuration(s.refresh_duration_ms)
    : "";
  // TUI top-strip parity: Running For / Refresh / Last Scan / Fetched /
  // Failed / Working / Sub Usage. Seven tight badges share one row (narrow
  // viewports scroll horizontally instead of wrapping or overflowing).
  box.appendChild(statCard("Running For", uptimeText(), state.startedAt ? "Started: " + fmtStamp(state.startedAt) : "", null, "stat-running"));
  box.appendChild(statCard("Refresh", rs.val, rs.sub, null, "stat-refresh-val", "stat-refresh-sub"));
  box.appendChild(statCard("Last scan", fmtStamp(s.last_refresh), ago + took, "stamp"));
  box.appendChild(statCard("Fetched", String(s.total_candidates || 0)));
  box.appendChild(statCard("Failed", String(failed), "of " + tested + " tested"));
  box.appendChild(statCard("Working", String(s.reachable_candidates || 0)));
  box.appendChild(statCard("Sub usage", fmtBytes(s.fetch_bytes)));
}

/// Wall-clock-aligned live ticker: uptime + countdowns only. A plain
/// `setInterval(1000)` drifts (late wakeups, background throttling), so the
/// displayed second would lag real time and then visibly jump whenever a data
/// render recomputed it — exactly the "seconds jump when Failed updates"
/// bug. Instead each tick schedules the next one for just past the upcoming
/// second boundary, so ticker and renders always agree to the second.
/// Writes are change-guarded: identical text never touches the DOM.
function tickClock() {
  if (state.snapshot) {
    setText($("stat-running"), uptimeText());
    const rs = refreshStatus();
    setText($("stat-refresh-val"), rs.val);
    setText($("stat-refresh-sub"), rs.sub);
  }
  scheduleClock();
}

/// Write only on change (null-safe): avoids redundant DOM writes that can
/// flicker or reset text selection in assistive tech.
function setText(node, text) {
  if (node && node.textContent !== text) {
    node.textContent = text;
  }
}

function scheduleClock() {
  if (state.clockTimer) {
    window.clearTimeout(state.clockTimer);
    state.clockTimer = 0;
  }
  // Just past the next second boundary (the +10 ms keeps us off the edge so
  // the new second is always the one displayed).
  const delay = 1000 - (Date.now() % 1000) + 10;
  state.clockTimer = window.setTimeout(tickClock, delay);
}

function startClock() {
  scheduleClock();
}

function proxyPill(running, uri) {
  const pill = el("span", null, "pill");
  if (running && uri) {
    pill.textContent = "running";
    pill.classList.add("good");
  } else if (running) {
    pill.textContent = "running (no config)";
    pill.classList.add("warn");
  } else {
    pill.textContent = "off";
  }
  return pill;
}

/// Overview "Recent logs": last 5 lines, newest at the bottom (TUI order).
function renderOvLogs() {
  const list = $("ov-logs");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  const lines = state.logLines.slice(-5);
  if (lines.length === 0) {
    const li = el("li", "No log lines yet — they appear while a cycle runs.", "muted");
    list.appendChild(li);
    return;
  }
  for (const line of lines) {
    const li = el("li", line);
    li.title = line;
    list.appendChild(li);
  }
}

/// Overview "Top configs": first 8 reachable, compact (TUI found-panel slice).
function renderOvConfigs() {
  const body = $("ov-cfg-body");
  while (body.firstChild) {
    body.removeChild(body.firstChild);
  }
  const s = state.snapshot;
  const rows = s && Array.isArray(s.ranked)
    ? s.ranked.filter((c) => c.reachable).slice(0, 8)
    : [];
  $("ov-cfg-empty").hidden = rows.length !== 0;
  const activeUri = s && s.proxy_active_uri ? s.proxy_active_uri : "";
  for (const c of rows) {
    const tr = document.createElement("tr");
    if (activeUri && c.uri && c.uri === activeUri) {
      tr.className = "is-proxy";
    }
    tr.appendChild(el("td", c.rank !== undefined ? String(c.rank) : "—"));
    const nameTd = document.createElement("td");
    nameTd.appendChild(el("strong", c.name || "(unnamed)"));
    tr.appendChild(nameTd);
    tr.appendChild(el("td", fmtLatency(c.latency_ms)));
    const actTd = document.createElement("td");
    const isActive = !!(c.uri && activeUri && c.uri === activeUri);
    const useBtn = el("button", isActive ? "Active" : "Use", "btn small");
    useBtn.type = "button";
    useBtn.title = isActive ? "This config is the active proxy" : "Pin this config as the proxy (same as Enter in the TUI)";
    useBtn.addEventListener("click", () => void selectProxy(c.uri));
    actTd.appendChild(useBtn);
    tr.appendChild(actTd);
    body.appendChild(tr);
  }
}

function ovConfigRow(box, key, value) {
  if (value === undefined || value === null || value === "") {
    return;
  }
  box.appendChild(el("dt", key));
  box.appendChild(el("dd", String(value)));
}

/// Overview "Configuration": Service/Network mirror of the TUI panel, sourced
/// from the cached `/api/config` payload (keys absent from the API are
/// skipped, never faked).
function renderOvConfig() {
  const svc = $("ov-svc");
  const net = $("ov-net");
  while (svc.firstChild) {
    svc.removeChild(svc.firstChild);
  }
  while (net.firstChild) {
    net.removeChild(net.firstChild);
  }
  const note = $("ov-config-note");
  const cfg = state.ovConfig;
  if (!cfg) {
    note.hidden = false;
    note.textContent = state.hasSummaryApi === false
      ? "Configuration needs a newer server — see the Settings tab."
      : "Loading configuration…";
    return;
  }
  note.hidden = true;
  const get = (key) => cfg.get(key);
  const secs = (key) => {
    const v = get(key);
    return v === undefined ? undefined : v + "s";
  };
  ovConfigRow(svc, "bind", get("bind") ? "http://" + get("bind") : undefined);
  ovConfigRow(svc, "top_n", get("top_n"));
  ovConfigRow(svc, "refresh", secs("refresh_seconds"));
  ovConfigRow(svc, "ping", secs("ping_seconds"));
  ovConfigRow(svc, "probe", get("probe.mode"));
  ovConfigRow(svc, "batch", get("probe.batch_size") === "null" ? "auto" : get("probe.batch_size"));
  ovConfigRow(svc, "stability", get("prioritize_stability"));
  ovConfigRow(svc, "asap", get("return_configs_asap"));
  ovConfigRow(svc, "scan_all", get("scan_all_configs"));
  const subs = get("enabled_subscription_count");
  const total = get("subscription_count");
  ovConfigRow(svc, "subscriptions", subs !== undefined && total !== undefined ? subs + "/" + total : subs);
  const maxSub = Number(get("max_subscription_bytes"));
  ovConfigRow(svc, "max_sub", Number.isFinite(maxSub) && get("max_subscription_bytes") !== undefined ? fmtBytes(maxSub) : undefined);
  ovConfigRow(net, "sharing", get("sharing.enabled"));
  ovConfigRow(net, "token", get("sharing.token"));
  const proxyOn = get("proxy.enabled") === "true";
  const proxyPort = get("proxy.port");
  const proxyLan = get("proxy.discoverable") === "true";
  let proxyText;
  if (!proxyOn) {
    proxyText = "off";
  } else if (proxyPort) {
    proxyText = "yes http://" + window.location.hostname + ":" + proxyPort + (proxyLan ? " (LAN)" : " (local)");
  } else {
    proxyText = "yes";
  }
  ovConfigRow(net, "proxy", proxyText);
  if (svc.children.length === 0 && net.children.length === 0) {
    note.hidden = false;
    note.textContent = "No configuration rows served — see the Settings tab.";
  }
}

/// Refreshes the overview configuration cache (visible tab, SSE change, save).
async function loadOvConfig() {
  if ($("panel-overview").hidden) {
    return;
  }
  const r = await fetchJson("/api/config");
  if (r.status === 200 && r.data && Array.isArray(r.data.groups)) {
    const map = new Map();
    for (const g of r.data.groups) {
      for (const k of g.keys || []) {
        if (k && k.key !== undefined) {
          map.set(k.key, k.value);
        }
      }
    }
    state.ovConfig = map;
  } else {
    state.ovConfig = null;
  }
  renderOvConfig();
  // The Telegram proxy endpoint row depends on this same payload.
  renderEndpoints();
}

function endpointItems() {
  const origin = window.location.origin;
  const items = [
    ["Subscription (auto)", origin + "/subscription"],
    ["Subscription (plain)", origin + "/subscription.txt"],
    ["Mihomo YAML", origin + "/mihomo.yaml"],
  ];
  // Same link the QR sheet encodes (qr.rs `telegram_proxy_url`): shown under
  // the same conditions as the sheet's proxy card (proxy LAN-enabled).
  // Host follows the dashboard URL, so opening via the LAN IP yields the
  // exact QR link; the firewall check stays server-side with the sheet.
  const tg = telegramProxyUrl();
  if (tg) {
    items.push(["Telegram proxy", tg]);
  }
  items.push(["Health", origin + "/health"]);
  return items;
}

/// `https://t.me/socks?server={host}&port={port}` or "" when the proxy is
/// not LAN-enabled (mirrors the QR planner's proxy gate).
function telegramProxyUrl() {
  const cfg = state.ovConfig;
  if (!cfg) {
    return "";
  }
  if (cfg.get("proxy.enabled") !== "true" || cfg.get("proxy.discoverable") !== "true") {
    return "";
  }
  const port = cfg.get("proxy.port");
  if (!port) {
    return "";
  }
  return "https://t.me/socks?server=" + window.location.hostname + "&port=" + port;
}

function renderEndpoints() {
  const list = $("endpoint-list");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  for (const [label, url] of endpointItems()) {
    const li = document.createElement("li");
    li.appendChild(el("strong", label));
    const code = el("code", url);
    code.title = url;
    li.appendChild(code);
    const btn = el("button", "Copy", "btn small");
    btn.type = "button";
    btn.addEventListener("click", () => void copyText(url, "Endpoint URL copied."));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

function renderFetchErrors() {
  const s = state.snapshot;
  const card = $("fetch-errors-card");
  const wrap = $("fetch-errors-wrap");
  const list = $("fetch-errors");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  const extra = wrap.querySelector(".more");
  if (extra) {
    wrap.removeChild(extra);
  }
  const errors = s && Array.isArray(s.fetch_errors) ? s.fetch_errors : [];
  if (errors.length === 0) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  $("fetch-errors-sub").textContent =
    errors.length + (errors.length === 1 ? " source" : " sources") +
    " failed to download (fetch problems — different from probe failures above)";
  const groups = new Map();
  for (const e of errors.slice(0, 60)) {
    groups.set(e, (groups.get(e) || 0) + 1);
  }
  const lines = [];
  for (const [msg, n] of groups) {
    lines.push(n > 1 ? "×" + n + " " + msg : msg);
  }
  const SHOWN = 6;
  for (const line of lines.slice(0, SHOWN)) {
    list.appendChild(el("li", line));
  }
  if (lines.length > SHOWN) {
    const details = document.createElement("details");
    details.className = "more";
    const summary = document.createElement("summary");
    summary.textContent = "Show " + (lines.length - SHOWN) + " more";
    details.appendChild(summary);
    const rest = document.createElement("ul");
    rest.className = "mono-list";
    for (const line of lines.slice(SHOWN)) {
      rest.appendChild(el("li", line));
    }
    details.appendChild(rest);
    wrap.appendChild(details);
  }
}

function filteredRanked() {
  const s = state.snapshot;
  if (!s || !Array.isArray(s.ranked)) {
    return [];
  }
  const q = state.search.trim().toLowerCase();
  const out = [];
  for (const c of s.ranked) {
    if (state.reachableOnly && !c.reachable) {
      continue;
    }
    if (q) {
      const hay = ((c.name || "") + " " + (c.protocol || "") + " " + maskedHost(c.endpoint) + " " + (c.source || "")).toLowerCase();
      if (!hay.includes(q)) {
        continue;
      }
    }
    out.push(c);
  }
  return out;
}

function visibleRanked() {
  const out = filteredRanked();
  if (state.rowLimit > 0 && out.length > state.rowLimit) {
    out.length = state.rowLimit;
  }
  return out;
}

function renderConfigs() {
  const rows = visibleRanked();
  const body = $("cfg-body");
  while (body.firstChild) {
    body.removeChild(body.firstChild);
  }
  const s = state.snapshot;
  const total = s && Array.isArray(s.ranked) ? s.ranked.length : 0;
  const matched = filteredRanked().length;
  const limited = state.rowLimit > 0 && matched > state.rowLimit;
  $("cfg-count").textContent = total === 0
    ? ""
    : "Showing " + rows.length + " of " + total + " ranked configs" +
      (limited ? " (limit " + state.rowLimit + ")" : "") + ".";
  $("cfg-empty").hidden = rows.length !== 0;

  let maxLat = 1;
  for (const c of rows) {
    if (typeof c.latency_ms === "number" && c.latency_ms > maxLat) {
      maxLat = c.latency_ms;
    }
  }
  const activeUri = s && s.proxy_active_uri ? s.proxy_active_uri : "";

  for (const c of rows) {
    const tr = document.createElement("tr");
    if (activeUri && c.uri && c.uri === activeUri) {
      tr.className = "is-proxy";
    }
    tr.appendChild(el("td", c.rank !== undefined ? String(c.rank) : "—"));

    const nameTd = document.createElement("td");
    nameTd.appendChild(el("strong", c.name || "(unnamed)"));
    if (c.source) {
      nameTd.appendChild(el("div", c.source, "muted"));
    }
    tr.appendChild(nameTd);

    const protoTd = document.createElement("td");
    protoTd.appendChild(el("span", c.protocol || "?", "proto"));
    tr.appendChild(protoTd);

    tr.appendChild(el("td", maskedHost(c.endpoint)));

    const latTd = document.createElement("td");
    latTd.textContent = fmtLatency(c.latency_ms);
    const bar = el("span", null, "lat-bar");
    const fillSpan = document.createElement("span");
    const pct = typeof c.latency_ms === "number" ? Math.max(4, Math.min(100, Math.round((c.latency_ms / maxLat) * 100))) : 0;
    fillSpan.style.inlineSize = pct + "%";
    bar.appendChild(fillSpan);
    latTd.appendChild(bar);
    tr.appendChild(latTd);

    tr.appendChild(el("td", c.stability_count ? "×" + c.stability_count : "—"));
    // Flag glyph only (+ code in the tooltip): regional indicators render as
    // the two letters on platforms without flag emoji (notably Windows), so
    // showing both would duplicate ("DE DE").
    const ccFlag = flagFor(c.country_code);
    const ccTd = document.createElement("td");
    if (ccFlag) {
      const ccUp = String(c.country_code).toUpperCase();
      const glyph = el("span", ccFlag);
      glyph.setAttribute("role", "img");
      glyph.setAttribute("aria-label", ccUp);
      glyph.title = ccUp;
      ccTd.appendChild(glyph);
    } else {
      ccTd.textContent = "—";
    }
    tr.appendChild(ccTd);

    const actTd = document.createElement("td");
    const wrap = el("span", null, "row-actions");
    const isActive = !!(c.uri && activeUri && c.uri === activeUri);
    const useBtn = el("button", isActive ? "Active" : "Use", "btn small");
    useBtn.type = "button";
    useBtn.title = isActive ? "This config is the active proxy" : "Pin this config as the proxy (same as Enter in the TUI)";
    useBtn.addEventListener("click", () => void selectProxy(c.uri));
    const detBtn = el("button", "Detail", "btn small");
    detBtn.type = "button";
    detBtn.addEventListener("click", () => openDetail(c));
    const copyBtn = el("button", "Copy", "btn small");
    copyBtn.type = "button";
    copyBtn.addEventListener("click", () => void copyText(c.uri || "", "Config link copied."));
    const qrBtn = el("button", "QR", "btn small");
    qrBtn.type = "button";
    qrBtn.title = "Show the QR code for this config (scan with a phone)";
    qrBtn.addEventListener("click", () => openQr(c));
    wrap.appendChild(useBtn);
    wrap.appendChild(detBtn);
    wrap.appendChild(copyBtn);
    wrap.appendChild(qrBtn);
    actTd.appendChild(wrap);
    tr.appendChild(actTd);

    body.appendChild(tr);
  }
}

function openDetail(c) {
  state.detailUri = c.uri || "";
  $("dlg-detail-title").textContent = "Config detail — " + (c.name || "(unnamed)");
  const kv = $("dlg-detail-kv");
  while (kv.firstChild) {
    kv.removeChild(kv.firstChild);
  }
  const ccFlag = flagFor(c.country_code);
  const ccUp = ccFlag ? String(c.country_code).toUpperCase() : "";
  const pairs = [
    ["Rank", c.rank !== undefined ? String(c.rank) : "—"],
    ["Protocol", c.protocol || "—"],
    ["Endpoint", maskedHost(c.endpoint)],
    ["Source", c.source || "—"],
    ["Reachable", c.reachable ? "yes" : "no"],
    ["Validation", c.validation || "—"],
    ["Latency", fmtLatency(c.latency_ms)],
    ["HTTP status", c.http_status !== null && c.http_status !== undefined ? String(c.http_status) : "—"],
    ["Speed", c.download_mbps !== null && c.download_mbps !== undefined ? Number(c.download_mbps).toFixed(2) + " Mbps" : "—"],
    ["Country", ccFlag ? ccFlag : "—"],
    ["Error", c.error || "—"],
  ];
  for (const [k, v] of pairs) {
    kv.appendChild(el("dt", k));
    const dd = el("dd", v);
    if (k === "Country" && ccFlag) {
      dd.title = ccUp;
      dd.setAttribute("role", "img");
      dd.setAttribute("aria-label", ccUp);
    }
    kv.appendChild(dd);
  }
  const dlg = $("dlg-detail");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

function openQr(c) {
  const uri = c.uri || "";
  if (!uri) {
    toast("No link to encode for this row.", "bad");
    return;
  }
  // Kept in a JS variable only — the full link is passed to the encoder
  // and painted to canvas, never written into the DOM as text.
  state.qrUri = uri;
  $("dlg-qr-title").textContent = "QR — " + (c.name || "(unnamed)");
  if (!drawQr(uri)) {
    return;
  }
  const dlg = $("dlg-qr");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

function drawQr(text) {
  const canvas = $("qr-canvas");
  if (typeof QREncode === "undefined" || !QREncode) {
    toast("QR encoder failed to load (qr.js missing).", "bad");
    return false;
  }
  let model = null;
  try {
    model = QREncode.encode(text, QREncode.CorrectLevel.M);
  } catch (err) {
    toast("Link is too long for a QR code.", "bad");
    return false;
  }
  const n = model.getModuleCount();
  const ctx = canvas.getContext("2d");
  const px = canvas.width;
  const quiet = 4;
  const scale = px / (n + quiet * 2);
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, px, canvas.height);
  ctx.fillStyle = "#000000";
  for (let r = 0; r < n; r += 1) {
    for (let col = 0; col < n; col += 1) {
      if (model.isDark(r, col)) {
        ctx.fillRect(Math.round((col + quiet) * scale), Math.round((r + quiet) * scale),
          Math.ceil(scale), Math.ceil(scale));
      }
    }
  }
  return true;
}

async function selectProxy(uri) {
  if (!uri) {
    toast("No link to pin for this row.", "bad");
    return;
  }
  const r = await fetchJson("/api/proxy/select", { method: "POST", body: { uri } });
  if (r.status === 404) {
    toast("Proxy-select API is not on this server version yet — use the TUI.", "bad");
    return;
  }
  if (r.status === 0) {
    toast("Server unreachable.", "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast("Proxy switch requested — confirming…", "good");
    setStatus("Proxy switch requested.");
    window.setTimeout(() => void loadResults(), 1500);
    return;
  }
  toast("Proxy switch failed (HTTP " + r.status + ").", "bad");
}

async function copyText(text, okMsg) {
  if (!text) {
    toast("Nothing to copy.", "bad");
    return;
  }
  try {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      await navigator.clipboard.writeText(text);
      toast(okMsg, "good");
      return;
    }
  } catch (err) {
    /* fall through to the legacy path */
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const done = document.execCommand("copy");
    document.body.removeChild(ta);
    toast(done ? okMsg : "Copy failed — select the text manually.", done ? "good" : "bad");
  } catch (err) {
    toast("Copy failed in this browser.", "bad");
  }
}

/* ---------- logs ---------- */

function renderLogs() {
  const list = $("log-list");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  const q = state.logFilter.trim().toLowerCase();
  let shown = 0;
  for (const line of state.logLines) {
    if (q && !line.toLowerCase().includes(q)) {
      continue;
    }
    list.appendChild(el("li", line));
    shown += 1;
  }
  $("log-empty").hidden = shown !== 0;
  if (state.follow) {
    try {
      list.scrollTop = list.scrollHeight;
    } catch (err) {
      /* non-scrollable container */
    }
  }
}

/* ---------- proxy / share tabs ---------- */

function kvFill(box, pairs) {
  while (box.firstChild) {
    box.removeChild(box.firstChild);
  }
  for (const [k, v] of pairs) {
    box.appendChild(el("dt", k));
    box.appendChild(el("dd", v));
  }
}

function renderProxyTab() {
  const s = state.snapshot;
  const pillSlot = $("proxy-pill");
  while (pillSlot.firstChild) {
    pillSlot.removeChild(pillSlot.firstChild);
  }
  if (!s) {
    pillSlot.textContent = "unknown";
    kvFill($("proxy-kv"), []);
    return;
  }
  pillSlot.appendChild(proxyPill(s.proxy_running, s.proxy_active_uri));
  kvFill($("proxy-kv"), [
    ["Running", s.proxy_running ? "yes" : "no"],
    ["Active config", s.proxy_active_config || "—"],
    ["Port", s.proxy_port !== null && s.proxy_port !== undefined ? String(s.proxy_port) : "—"],
    ["LAN discoverable", s.proxy_discoverable ? "yes" : "no"],
    ["Working pool", String(s.reachable_candidates || 0)],
  ]);
  $("proxy-manual").textContent = s.proxy_active_uri ? "Active link held by the server (full URI never rendered — use Copy on its row to export)." : "none pinned";
  $("btn-proxy-unpin").disabled = !s.proxy_active_uri;
}

function renderShare() {
  const list = $("share-list");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  for (const [label, url] of endpointItems().slice(0, 3)) {
    const li = document.createElement("li");
    li.appendChild(el("strong", label));
    const code = el("code", url);
    code.title = url;
    li.appendChild(code);
    const btn = el("button", "Copy", "btn small");
    btn.type = "button";
    btn.addEventListener("click", () => void copyText(url, "Subscription URL copied."));
    li.appendChild(btn);
    list.appendChild(li);
  }
  $("share-hint").textContent = getToken()
    ? "This page URL carries a token, so these links work for LAN clients too (same token model as the TUI subscription URL)."
    : "On this device no token is needed (loopback bypass). LAN clients need sharing enabled — and the token appended if the server requires one.";
  const img = $("qr-img");
  const note = $("qr-note");
  img.hidden = true;
  note.textContent = "Loading QR…";
  loadQrImage();
}

async function loadQrImage() {
  const img = $("qr-img");
  const note = $("qr-note");
  img.hidden = true;
  if ($("panel-share").hidden) {
    // Cheap path: don't probe a file nobody is looking at.
    note.textContent = "Open this tab to view the QR sheet.";
    return;
  }
  if (state.hasSummaryApi === false) {
    note.textContent = "The QR sheet needs a newer server — per-config QR buttons still work.";
    return;
  }
  if (state.qrKnown === null) {
    // Exactly one metadata answer per session; a missing file then costs
    // zero failed requests (failed fetches log console errors).
    const r = await fetchJson("/api/summary");
    if (r.status === 200 && r.data && typeof r.data.qr_available === "boolean") {
      state.hasSummaryApi = true;
      storeCapabilities(r.data);
    } else {
      state.hasSummaryApi = false;
      state.qrKnown = false;
      note.textContent = "The QR sheet needs a newer server — per-config QR buttons still work.";
      return;
    }
  }
  if (!state.qrKnown) {
    note.textContent = "No QR image yet — press Generate QR (requires LAN sharing).";
    return;
  }
  note.textContent = "Loading QR…";
  // fetch (not `new Image()`): failures stay silent promise rejections.
  // The bytes travel once: the blob URL below reuses this same response.
  fetch(apiPath("/api/qr.jpg"), { headers: { Accept: "image/jpeg" } }).then(
    (res) => {
      if (!res || !res.ok) {
        throw new Error("missing");
      }
      return res.blob();
    }
  ).then(
    (blob) => {
      if (!blob || blob.size === 0) {
        throw new Error("empty");
      }
      if (state.qrObjectUrl) {
        try {
          URL.revokeObjectURL(state.qrObjectUrl);
        } catch (err) {
          /* already revoked */
        }
        state.qrObjectUrl = "";
      }
      let url = "";
      try {
        url = URL.createObjectURL(blob);
      } catch (err) {
        url = "";
      }
      if (!url) {
        throw new Error("object-url");
      }
      state.qrObjectUrl = url;
      img.src = url;
      img.hidden = false;
      note.textContent = "Scan with your phone to add the LAN subscription.";
    }
  ).catch(() => {
    note.textContent = "No QR image yet — press Generate QR (requires LAN sharing).";
  });
}

/// Overview QR block under Endpoints: same sheet as the Share tab, loaded
/// lazily (tab visible only) and once per sheet — renderAll calls this every
/// render, so every path after the loaded check must be cheap and the single
/// capability probe must never stack.
async function renderOvQr() {
  const img = $("ov-qr-img");
  const note = $("ov-qr-note");
  if (!img || !note || $("panel-overview").hidden) {
    return;
  }
  if (state.hasSummaryApi === false) {
    note.textContent = "The QR sheet needs a newer server — per-config QR buttons still work.";
    return;
  }
  if (state.qrKnown === null) {
    if (state.ovQrProbing) {
      return;
    }
    state.ovQrProbing = true;
    try {
      const r = await fetchJson("/api/summary");
      if (r.status === 200 && r.data && typeof r.data.qr_available === "boolean") {
        state.hasSummaryApi = true;
        storeCapabilities(r.data);
      } else {
        state.hasSummaryApi = false;
        state.qrKnown = false;
      }
    } finally {
      state.ovQrProbing = false;
    }
    if (state.qrKnown === null) {
      return;
    }
  }
  if (!state.qrKnown) {
    state.ovQrLoaded = false;
    img.hidden = true;
    note.textContent = "No QR image yet — press Generate QR Code (requires LAN sharing).";
    return;
  }
  if (state.ovQrLoaded && !img.hidden) {
    return;
  }
  note.textContent = "Loading QR…";
  // fetch (not `new Image()`): failures stay silent promise rejections.
  fetch(apiPath("/api/qr.jpg"), { headers: { Accept: "image/jpeg" } }).then(
    (res) => {
      if (!res || !res.ok) {
        throw new Error("missing");
      }
      return res.blob();
    }
  ).then(
    (blob) => {
      if (!blob || blob.size === 0) {
        throw new Error("empty");
      }
      let url = "";
      try {
        url = URL.createObjectURL(blob);
      } catch (err) {
        url = "";
      }
      if (!url) {
        throw new Error("object-url");
      }
      // Own object URL: the Share tab has its own (`state.qrObjectUrl`) and
      // revokes on every render — sharing one field would break the hidden
      // image each time tabs switch.
      if (state.ovQrObjectUrl) {
        try {
          URL.revokeObjectURL(state.ovQrObjectUrl);
        } catch (err) {
          /* already revoked */
        }
      }
      state.ovQrObjectUrl = url;
      img.src = url;
      img.hidden = false;
      state.ovQrLoaded = true;
      note.textContent = "Scan with your phone to add the LAN subscription.";
    }
  ).catch(() => {
    state.ovQrLoaded = false;
    img.hidden = true;
    note.textContent = "No QR image yet — press Generate QR Code (requires LAN sharing).";
  });
}

async function generateQr() {
  const btn = $("btn-qr-generate");
  btn.disabled = true;
  try {
    const r = await fetchJson("/api/qr/generate", { method: "POST", body: {} });
    if (r.status === 404) {
      toast("QR API is not on this server version yet.", "bad");
      return;
    }
    if (r.status === 0) {
      toast("Server unreachable.", "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300 && r.data) {
      const skipped = Array.isArray(r.data.skipped) ? r.data.skipped : [];
      const base = r.data.message || (r.data.ok ? "QR generated." : "QR unavailable.");
      const shown = skipped.length > 0 ? base + " Skipped: " + skipped.join("; ") : base;
      toast(shown, r.data.ok ? "good" : "bad");
      state.hasSummaryApi = true;
      state.qrKnown = !!r.data.ok;
      if (r.data.ok) {
        state.ovQrLoaded = false;
        loadQrImage();
        void renderOvQr();
      }
      return;
    }
    toast("QR generation failed (HTTP " + r.status + ").", "bad");
  } finally {
    btn.disabled = false;
  }
}

/* ---------- subscriptions tab (progressive) ---------- */

async function loadSubscriptions() {
  const note = $("sub-note");
  const r = await fetchJson("/api/subscriptions");
  if (r.status === 200 && r.data && Array.isArray(r.data.list)) {
    state.subs = { list: r.data.list, dirty: !!r.data.dirty };
    setDirty(!!r.data.dirty);
    note.textContent = r.data.list.length + " subscription(s) on the server.";
    renderSubs();
    return;
  }
  state.subs = null;
  setDirty(false);
  renderSubs();
  note.textContent = r.status === 404
    ? "Subscription management API is not on this server version yet — manage subscriptions from the TUI or configs.yaml."
    : "Could not load subscriptions (HTTP " + r.status + ").";
}

function renderSubs() {
  const body = $("sub-body");
  while (body.firstChild) {
    body.removeChild(body.firstChild);
  }
  const list = state.subs ? state.subs.list : [];
  $("sub-empty").hidden = list.length !== 0;
  list.forEach((sub, i) => {
    const tr = document.createElement("tr");

    const onTd = document.createElement("td");
    const tgl = document.createElement("button");
    tgl.type = "button";
    tgl.className = "btn small";
    tgl.textContent = sub.enabled ? "On" : "Off";
    tgl.setAttribute("aria-pressed", sub.enabled ? "true" : "false");
    tgl.setAttribute("aria-label", "Toggle " + (sub.name || ("subscription " + (i + 1))));
    tgl.addEventListener("click", () => void subToggle(i));
    onTd.appendChild(tgl);
    tr.appendChild(onTd);

    tr.appendChild(el("td", sub.priority !== undefined ? String(sub.priority) : "—"));
    tr.appendChild(el("td", sub.name || ("subscription " + (i + 1))));

    const urlTd = document.createElement("td");
    urlTd.appendChild(el("code", redactUrl(sub.url || "")));
    tr.appendChild(urlTd);

    const actTd = document.createElement("td");
    const wrap = el("span", null, "row-actions");
    const editBtn = el("button", "Edit", "btn small");
    editBtn.type = "button";
    editBtn.addEventListener("click", () => void subEdit(i));
    const delBtn = el("button", "Delete", "btn small");
    delBtn.type = "button";
    delBtn.addEventListener("click", () => void subDelete(i));
    wrap.appendChild(editBtn);
    wrap.appendChild(delBtn);
    actTd.appendChild(wrap);
    tr.appendChild(actTd);

    body.appendChild(tr);
  });
}

function redactUrl(url) {
  // Mirror the server's redact convention: keep host/path visible,
  // strip credentials, token query and fragments.
  try {
    const u = new URL(url, window.location.origin);
    u.username = "";
    u.password = "";
    u.hash = "";
    u.searchParams.delete("token");
    let s = u.toString();
    if (s.length > 120) {
      s = s.slice(0, 117) + "…";
    }
    return s;
  } catch (err) {
    return url.length > 120 ? url.slice(0, 117) + "…" : url;
  }
}

async function subToggle(i) {
  const r = await fetchJson("/api/subscriptions/" + i + "/toggle", { method: "POST", body: {} });
  if (r.status === 404) {
    toast("Subscription API is not on this server version yet.", "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast("Toggled. Remember to Save.", "good");
    void loadSubscriptions();
    return;
  }
  toast("Toggle failed (HTTP " + r.status + ").", "bad");
}

async function subDelete(i) {
  const r = await fetchJson("/api/subscriptions/" + i, { method: "DELETE" });
  if (r.status === 404) {
    toast("Subscription API is not on this server version yet.", "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast("Deleted. Remember to Save.", "good");
    void loadSubscriptions();
    return;
  }
  toast("Delete failed (HTTP " + r.status + ").", "bad");
}

function subEdit(i) {
  const sub = state.subs && state.subs.list[i];
  if (!sub) {
    return;
  }
  openSubDialog(sub);
}

function openSubDialog(preset) {
  $("dlg-sub-title").textContent = preset ? "Edit subscription" : "Add subscription";
  $("dlg-sub-url").value = (preset && preset.url) || "";
  $("dlg-sub-name").value = (preset && preset.name) || "";
  $("dlg-sub-priority").value = (preset && preset.priority !== undefined) ? String(preset.priority) : "100";
  $("dlg-sub-enabled").checked = preset ? !!preset.enabled : true;
  const dlg = $("dlg-sub");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

async function submitSubDialog() {
  const payload = {
    url: $("dlg-sub-url").value.trim(),
    name: $("dlg-sub-name").value.trim(),
    priority: Math.max(0, Number($("dlg-sub-priority").value) || 0),
    enabled: $("dlg-sub-enabled").checked,
  };
  if (!payload.url || !payload.name) {
    toast("URL and name are required.", "bad");
    return;
  }
  const r = await fetchJson("/api/subscriptions", { method: "POST", body: payload });
  if (r.status === 404) {
    toast("Subscription API is not on this server version yet.", "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast("Added. Remember to Save.", "good");
    void loadSubscriptions();
    return;
  }
  toast("Add failed (HTTP " + r.status + ").", "bad");
}

/* ---------- settings tab (progressive) ---------- */

async function loadSettings() {
  const note = $("settings-note");
  const r = await fetchJson("/api/config");
  const box = $("settings-groups");
  while (box.firstChild) {
    box.removeChild(box.firstChild);
  }
  if (r.status === 200 && r.data && Array.isArray(r.data.groups)) {
    state.settings = r.data;
    setDirty(!!r.data.dirty);
    note.textContent = "Click a value to edit it (same validators as the TUI). Changes stay in memory until saved.";
    for (const g of r.data.groups) {
      const card = el("section", null, "set-group");
      card.appendChild(el("h3", g.title || "Settings"));
      for (const k of g.keys || []) {
        const row = document.createElement("div");
        row.className = "set-row";
        row.appendChild(el("span", k.key || "", "key"));
        const val = el("span", k.value !== undefined ? String(k.value) : "—", "val");
        val.tabIndex = 0;
        val.setAttribute("role", "button");
        val.title = "Click to edit";
        val.addEventListener("click", () => editSetting(k.key, val));
        val.addEventListener("keydown", (ev) => {
          if (ev.key === "Enter" || ev.key === " ") {
            ev.preventDefault();
            editSetting(k.key, val);
          }
        });
        row.appendChild(val);
        if (k.guide) {
          row.appendChild(el("span", k.guide, "guide"));
        }
        card.appendChild(row);
      }
      box.appendChild(card);
    }
    return;
  }
  state.settings = null;
  note.textContent = r.status === 404
    ? "Settings API is not on this server version yet — edit settings from the TUI Configurations screen or configs.yaml."
    : "Could not load settings (HTTP " + r.status + ").";
}

function editSetting(key, valNode) {
  const current = valNode.textContent;
  const input = document.createElement("input");
  input.type = "text";
  input.value = current === "—" ? "" : current;
  input.setAttribute("aria-label", "New value for " + key);
  valNode.textContent = "";
  valNode.appendChild(input);
  input.focus();
  input.select();
  const commit = async (save) => {
    const v = input.value;
    valNode.removeChild(input);
    valNode.textContent = v === "" ? "—" : v;
    if (!save) {
      return;
    }
    const r = await fetchJson("/api/config", { method: "PATCH", body: { key, value: v } });
    if (r.status === 404) {
      toast("Settings API is not on this server version yet.", "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300) {
      toast("Saved in memory. Remember to Save.", "good");
      setDirty(true);
      return;
    }
    const msg = r.data && r.data.message ? r.data.message : "HTTP " + r.status;
    toast("Rejected: " + msg, "bad");
  };
  input.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") {
      ev.preventDefault();
      void commit(true);
    } else if (ev.key === "Escape") {
      ev.preventDefault();
      void commit(false);
    }
  });
  input.addEventListener("blur", () => void commit(true));
}

/* ---------- actions ---------- */

async function triggerCycle(kind) {
  const path = kind === "refresh" ? "/api/refresh" : "/api/ping";
  const btn = kind === "refresh" ? $("btn-refresh") : $("btn-ping");
  btn.disabled = true;
  try {
    // Preferred: dedicated trigger endpoint (lands with the backend phases).
    const r = await fetchJson(path, { method: "POST", body: {} });
    if (r.status === 404) {
      toast("Manual " + kind + " needs a newer server (no " + path + " route yet).", "bad");
      setStatus("Manual " + kind + " unavailable on this server.");
      return;
    }
    if (r.status === 409) {
      toast("A cycle is already running — try again when it finishes.", "bad");
      return;
    }
    if (r.status === 0) {
      toast("Server unreachable.", "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300) {
      toast(kind === "refresh" ? "Refresh triggered." : "Ping triggered.", "good");
      setStatus("Manual " + kind + " triggered — watch Overview.");
      window.setTimeout(() => void loadResults(), 1200);
      return;
    }
    toast("Trigger failed (HTTP " + r.status + ").", "bad");
  } finally {
    btn.disabled = false;
  }
}

async function saveNow() {
  const r = await fetchJson("/api/save", { method: "POST", body: {} });
  if (r.status === 404) {
    toast("Save API is not on this server version yet.", "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    setDirty(false);
    toast("Saved.", "good");
    setStatus("Saved.");
    void loadOvConfig();
    return;
  }
  toast("Save failed (HTTP " + r.status + ").", "bad");
}

/* ---------- tabs / routing (clean paths, History API) ---------- */

/// Legacy `#/tab` bookmarks redirect to the clean path on boot (rewrite, not
/// push: the hash URL never enters back-button history). Query (`?token=`)
/// is preserved so LAN token links keep working across the redirect.
function redirectLegacyHash() {
  const h = window.location.hash || "";
  const m = h.match(/^#\/([a-z]+)/);
  if (m && TABS.includes(m[1])) {
    window.location.replace("/" + m[1] + window.location.search);
    return true;
  }
  return false;
}

function currentTab() {
  const m = window.location.pathname.match(/^\/([a-z]+)/);
  if (m && TABS.includes(m[1])) {
    return m[1];
  }
  // `/` (and unknown paths, which the server 404s — this only runs for the
  // shell) reopen the last tab, overview on first visit.
  try {
    const v = window.localStorage.getItem(TAB_KEY);
    if (v && TABS.includes(v)) {
      return v;
    }
  } catch (err) {
    /* ignore */
  }
  return "overview";
}

function showTab(name) {
  if (!TABS.includes(name)) {
    name = "overview";
  }
  for (const t of TABS) {
    const panel = $("panel-" + t);
    const tab = $("tab-" + t);
    const active = t === name;
    panel.hidden = !active;
    tab.setAttribute("aria-selected", active ? "true" : "false");
  }
  try {
    window.localStorage.setItem(TAB_KEY, name);
  } catch (err) {
    /* ignore */
  }
  if (name === "subscriptions" && !state.subs) {
    void loadSubscriptions();
  }
  if (name === "settings" && !state.settings) {
    void loadSettings();
  }
  if (name === "overview") {
    void loadOvConfig();
    void renderOvQr();
  }
  if (name === "share") {
    renderShare();
  }
}

function goTab(name) {
  if (!TABS.includes(name)) {
    name = "overview";
  }
  const url = "/" + name + window.location.search;
  if (window.location.pathname + window.location.search !== url) {
    window.history.pushState({ tab: name }, "", url);
  }
  showTab(name);
}

/* ---------- keyboard ---------- */

function typingTarget() {
  const a = document.activeElement;
  return !!a && (a.tagName === "INPUT" || a.tagName === "TEXTAREA" || a.tagName === "SELECT");
}

function onKey(ev) {
  if (ev.key === "Escape") {
    for (const id of ["dlg-sub", "dlg-detail", "dlg-qr", "dlg-keys"]) {
      const d = $(id);
      if (d.open) {
        d.close();
        return;
      }
    }
  }
  if (ev.ctrlKey || ev.altKey || ev.metaKey || typingTarget()) {
    return;
  }
  if (ev.key === "?") {
    ev.preventDefault();
    const d = $("dlg-keys");
    if (typeof d.showModal === "function") {
      d.showModal();
    }
  } else if (ev.key === "r" || ev.key === "R") {
    ev.preventDefault();
    void triggerCycle("refresh");
  } else if (ev.key === "p" || ev.key === "P") {
    ev.preventDefault();
    void triggerCycle("ping");
  } else if (ev.key === "/") {
    ev.preventDefault();
    goTab("configs");
    $("cfg-search").focus();
  } else if (ev.key >= "1" && ev.key <= "7") {
    ev.preventDefault();
    goTab(TABS[Number(ev.key) - 1]);
  }
}

/* ---------- boot ---------- */

function wire() {
  $("btn-refresh").addEventListener("click", () => void triggerCycle("refresh"));
  $("btn-ping").addEventListener("click", () => void triggerCycle("ping"));
  $("banner-retry").addEventListener("click", () => {
    void boot();
  });

  for (const m of ["system", "light", "dark"]) {
    $("theme-" + m).addEventListener("click", () => saveTheme(m));
  }
  if (window.matchMedia) {
    try {
      window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
        let cur = "system";
        try {
          cur = window.localStorage.getItem(THEME_KEY) || "system";
        } catch (err) {
          cur = "system";
        }
        if (cur === "system") {
          applyTheme("system");
        }
      });
    } catch (err) {
      /* older browsers without addEventListener on MediaQueryList */
    }
  }

  $("btn-keys").addEventListener("click", () => {
    const d = $("dlg-keys");
    if (typeof d.showModal === "function") {
      d.showModal();
    }
  });
  $("dlg-keys-close").addEventListener("click", () => $("dlg-keys").close());

  $("cfg-search").addEventListener("input", (ev) => {
    state.search = ev.target.value;
    renderConfigs();
  });
  $("cfg-reachable-only").addEventListener("change", (ev) => {
    state.reachableOnly = ev.target.checked;
    renderConfigs();
  });
  const limitSel = $("cfg-limit");
  limitSel.value = state.rowLimit > 0 ? String(state.rowLimit) : "all";
  limitSel.addEventListener("change", (ev) => {
    const v = ev.target.value;
    state.rowLimit = (v === "25" || v === "50" || v === "100") ? Number(v) : 0;
    saveRowLimit(state.rowLimit);
    renderConfigs();
  });

  $("log-filter").addEventListener("input", (ev) => {
    state.logFilter = ev.target.value;
    renderLogs();
  });
  $("log-follow").addEventListener("change", (ev) => {
    state.follow = ev.target.checked;
    if (state.follow) {
      renderLogs();
    }
  });
  $("btn-log-clear").addEventListener("click", () => {
    state.logLines = [];
    renderLogs();
    toast("Log view cleared (server logs untouched).", "good");
  });

  $("btn-sub-add").addEventListener("click", () => {
    if (!state.subs) {
      toast("Subscription API is not on this server version yet.", "bad");
      return;
    }
    openSubDialog(null);
  });
  $("dlg-sub-form").addEventListener("submit", (ev) => {
    if (ev.submitter && ev.submitter.value === "ok") {
      void submitSubDialog();
    }
  });
  $("btn-save").addEventListener("click", () => void saveNow());
  $("btn-settings-reload").addEventListener("click", () => {
    state.settings = null;
    void loadSettings();
  });

  $("btn-proxy-mode").addEventListener("click", async () => {
    const r = await fetchJson("/api/proxy/mode", { method: "POST", body: {} });
    if (r.status === 404) {
      toast("Proxy API is not on this server version yet — use the TUI.", "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300) {
      const msg = r.data && r.data.message ? r.data.message : "Proxy mode cycled.";
      toast(msg, "good");
      window.setTimeout(() => void loadResults(), 1200);
      return;
    }
    toast("Proxy switch failed (HTTP " + r.status + ").", "bad");
  });
  $("btn-proxy-unpin").addEventListener("click", () => {
    const s = state.snapshot;
    if (s && s.proxy_active_uri) {
      void selectProxy(s.proxy_active_uri);
    }
  });
  $("btn-sharing").addEventListener("click", async () => {
    const r = await fetchJson("/api/sharing", { method: "POST", body: {} });
    if (r.status === 404) {
      toast("Sharing API is not on this server version yet — use the TUI.", "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300) {
      const msg = r.data && r.data.message ? r.data.message : "Sharing toggled.";
      toast(msg, "good");
      window.setTimeout(() => void loadResults(), 1200);
      return;
    }
    toast("Sharing toggle failed (HTTP " + r.status + ").", "bad");
  });
  $("btn-qr-generate").addEventListener("click", () => void generateQr());
  $("btn-qr-generate-ov").addEventListener("click", () => void generateQr());

  $("dlg-detail-close").addEventListener("click", () => $("dlg-detail").close());
  $("dlg-detail-copy").addEventListener("click", () => void copyText(state.detailUri, "Config link copied."));
  $("dlg-qr-close").addEventListener("click", () => $("dlg-qr").close());

  window.addEventListener("popstate", () => showTab(currentTab()));
  // In-app tab links navigate instantly (no reload); anything else (skip
  // link, dialogs, external URLs) keeps native behavior.
  document.addEventListener("click", (ev) => {
    if (ev.ctrlKey || ev.metaKey || ev.shiftKey || ev.altKey || ev.button !== 0) {
      return;
    }
    const a = ev.target && ev.target.closest ? ev.target.closest('a[href^="/"]') : null;
    if (!a) {
      return;
    }
    const m = a.getAttribute("href").match(/^\/([a-z]+)$/);
    if (m && TABS.includes(m[1])) {
      ev.preventDefault();
      goTab(m[1]);
    }
  });
  document.addEventListener("keydown", onKey);
}

async function boot() {
  // Tear down any previous loops: "Retry now" re-enters boot and must not
  // leave a duplicate poller or SSE connection behind.
  stopPolling();
  if (state.sse) {
    try {
      state.sse.close();
    } catch (err) {
      /* connection already gone */
    }
    state.sse = null;
  }
  loadTheme();
  state.rowLimit = loadRowLimit();
  if (redirectLegacyHash()) {
    return;
  }
  wire();
  setDirty(false);
  setFeedStatus("boot");
  startClock();
  showTab(currentTab());
  const ok = await loadResults();
  if (ok) {
    connectFeed();
  } else if (state.feed === "offline") {
    // Retry loop until the server answers; then wire the feed once.
    stopPolling();
    const retry = async () => {
      const good = await loadResults();
      if (good) {
        connectFeed();
      } else {
        window.setTimeout(retry, 5000);
      }
    };
    window.setTimeout(retry, 5000);
  }
}

document.addEventListener("DOMContentLoaded", () => void boot());
