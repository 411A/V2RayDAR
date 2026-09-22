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
const LOG_LEVEL_KEY = "v2raydar.logLevel";
const MAX_LOGS = 300;
const MAX_TOASTS = 4;
const TOAST_MS = 4500;
const THEME_KEY = "v2raydar-theme";
const TAB_KEY = "v2raydar-tab";

const TABS = ["overview", "configs", "subscriptions", "settings", "proxy", "logs", "share"];

const LOG_LEVELS = ["DEBUG", "INFO", "WARN", "ERROR"];
const LOG_LEVEL_ORDER = { DEBUG: 0, INFO: 1, WARN: 2, ERROR: 3 };

const state = {
  snapshot: null,
  feed: "boot", // boot | live | polling | offline | locked
  serverStopped: false, // set after a confirmed shutdown; boot clears it
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
  logLevel: "INFO",
  follow: true,
  wired: false, // wire() runs once; boot() may re-enter via Retry
  cycleInflight: { refresh: false, ping: false }, // POST in flight (buttons locked)
  proxyPendingUri: undefined, // proxy switch requested but not yet confirmed (undefined = none; null = unpin in flight)
  proxyModeInflight: false, // mode POST in flight (segmented buttons locked)
  editingSub: null, // subscription index being edited, null = adding
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

/// Table cell with a mobile-card label: on portrait phones the wide tables
/// collapse into labeled cards, and each value reads its `data-th` (the
/// already-localized column header) as the card-line label. Desktop and
/// tablets render plain tables and ignore the attribute entirely.
function cell(text, label, cls) {
  const td = document.createElement("td");
  td.dataset.th = label;
  if (cls) {
    td.className = cls;
  }
  td.appendChild(el("span", text, "cell-text"));
  return td;
}

/// Re-render without moving the viewport: tearing a tall container down to
/// empty collapses the document, so the browser clamps window.scrollY upward
/// and the rebuilt content appears under a jumped viewport (the settings-tab
/// jump after every save). Snapshot the offsets, run the paint, then put them
/// back. When the paint destroys the focused control, focus its rebuilt twin
/// (matched by the row's data-key) so keyboard users keep their place too.
/// Promise-transparent: an async paint restores once it settles.
function preserveViewport(paint) {
  const x = typeof window.scrollX === "number" ? window.scrollX : 0;
  const y = typeof window.scrollY === "number" ? window.scrollY : 0;
  const active = document.activeElement;
  const key = active && typeof active.getAttribute === "function"
    ? active.getAttribute("data-key")
    : null;
  const restore = () => {
    if (key && document.activeElement !== active) {
      const twin = typeof document.querySelector === "function"
        ? document.querySelector('[data-key="' + key + '"]')
        : null;
      if (twin && twin !== active && typeof twin.focus === "function") {
        twin.focus({ preventScroll: true });
      }
    }
    if (typeof window.scrollTo === "function") {
      window.scrollTo(x, y);
    }
  };
  const out = paint();
  if (out && typeof out.then === "function") {
    return out.then(
      (value) => { restore(); return value; },
      (error) => { restore(); throw error; },
    );
  }
  restore();
  return out;
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

/// Fail-fast timeout for API calls: a stalled server must drop into the
/// offline/retry path, not wedge boot on "Connecting…" forever. Zero
/// network/resource overhead — purely client-side, the request is aborted.
/// Override via `state.fetchTimeoutMs` (tests).
const FETCH_TIMEOUT_MS = 15000;

async function fetchJson(path, options) {
  const ctrl = typeof AbortController !== "undefined" ? new AbortController() : null;  const ms = state.fetchTimeoutMs || FETCH_TIMEOUT_MS;
  const timer = ctrl ? window.setTimeout(() => ctrl.abort(), ms) : 0;
  const init = { headers: { Accept: "application/json" } };
  if (ctrl) {
    init.signal = ctrl.signal;
  }
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
  } finally {
    if (timer) {
      window.clearTimeout(timer);
    }
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
  const node = el("div", message, "toast" + (kind ? " " + kind : ""));
  box.appendChild(node);
  window.setTimeout(() => {
    if (node.parentNode === box) {
      box.removeChild(node);
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
    pill.textContent = dirty ? t("dirtyUnsaved") : t("dirtySaved");
    pill.classList.toggle("warn", dirty);
    pill.classList.toggle("good", !dirty);
  }
  const btn = $("btn-save");
  if (btn) {
    btn.disabled = !dirty;
    btn.textContent = dirty ? t("btnSaveNow") : t("btnSaveClean");
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

/* ---------- language menu ---------- */

/// Language menu codes in menu order (EN, IR, CN, FR, RU) mapped to the
/// i18n.js locale tables; the choice persists via setLanguage().
const LANGS = ["en", "ir", "cn", "fr", "ru"];
const LANG_LOCALE = { en: "en", ir: "fa", cn: "zh", fr: "fr", ru: "ru" };

function syncLangMenu() {
  let code = "en";
  for (const c of LANGS) {
    if (LANG_LOCALE[c] === i18nLang) {
      code = c;
      break;
    }
  }
  const menu = $("lang-menu");
  if (menu && menu.querySelectorAll) {
    const items = menu.querySelectorAll("button[data-lang]");
    for (let i = 0; i < items.length; i += 1) {
      const itemCode = items[i].getAttribute("data-lang");
      const locale = LANG_LOCALE[itemCode] || "en";
      items[i].setAttribute("aria-checked", String(locale === i18nLang));
    }
  }
  // The button shows the current language's flag — it must follow the
  // switch (and the persisted language at boot), not stay stuck on GB.
  // One <img> per language lives in the button and only the current one
  // is shown: flag assets are served `no-store`, so retargeting `src`
  // refetches over the network and breaks while offline (and moving nodes
  // scrambles the menu rows), while toggling preloaded nodes needs zero
  // network and leaves every menu row untouched.
  const btn = $("btn-lang");
  const kids = btn && btn.children ? btn.children : [];
  for (let i = 0; i < kids.length; i += 1) {
    const el = kids[i];
    if (el && el.tagName === "IMG") {
      el.hidden = (el.getAttribute("data-lang") || "en") !== code;
    }
  }
}

function toggleLangMenu() {
  const menu = $("lang-menu");
  if (!menu) {
    return;
  }
  menu.hidden = !menu.hidden;
  $("btn-lang").setAttribute("aria-expanded", String(!menu.hidden));
}

function closeLangMenu() {
  const menu = $("lang-menu");
  if (!menu || menu.hidden) {
    return false;
  }
  menu.hidden = true;
  $("btn-lang").setAttribute("aria-expanded", "false");
  return true;
}

function selectLang(code) {
  const locale = LANG_LOCALE[code];
  if (!locale || !LANGS.includes(code)) {
    return;
  }
  setLanguage(locale);
  // Static shell re-applies inside setLanguage(); dynamic sections render
  // only on data arrival, so repaint everything from cached state —
  // otherwise stale-language rows (share list, tables, pill, banner)
  // survive the switch.
  renderAll();
  renderSubs();
  renderSettings();
  setDirty(!!((state.subs && state.subs.dirty) || (state.settings && state.settings.dirty)));
  updateCycleButtons();
  reapplyFeedStatus();
  syncLangMenu();
  closeLangMenu();
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

/// Log-level floor persisted under LOG_LEVEL_KEY; unknown stored values
/// fall back to "INFO" (same validate-or-default shape as loadRowLimit).
function loadLogLevel() {
  try {
    const v = window.localStorage.getItem(LOG_LEVEL_KEY);
    if (v === "ALL" || v === "INFO" || v === "WARN" || v === "ERROR") {
      return v;
    }
  } catch (err) {
    /* private mode etc. — fall through to INFO */
  }
  return "INFO";
}

function saveLogLevel(level) {
  try {
    window.localStorage.setItem(LOG_LEVEL_KEY, level);
  } catch (err) {
    /* private mode etc. — level still applies for the session */
  }
}

/* ---------- formatting ---------- */

/// Isolate a Latin technical run (measurements, "x3" counts) as one
/// left-to-right chunk so RTL layout keeps "2.1 s" instead of flipping it
/// to "s 2.1". Written as backslash-u escapes on purpose: invisible bidi
/// control chars must never be pasted literally into source.
function ltr(s) {
  return "\u2066" + s + "\u2069";
}

function fmtBytes(n) {
  if (n === null || n === undefined) {
    return "—";
  }
  const v = Number(n);
  if (!Number.isFinite(v)) {
    return "—";
  }
  if (v < 1024) {
    return ltr(v + " B");
  }
  if (v < 1048576) {
    return ltr((v / 1024).toFixed(1) + " KB");
  }
  if (v < 1073741824) {
    return ltr((v / 1048576).toFixed(1) + " MB");
  }
  return ltr((v / 1073741824).toFixed(2) + " GB");
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
    return ltr(Math.round(v) + " ms");
  }
  return ltr((v / 1000).toFixed(1) + " s");
}

/// Milliseconds, always (`842 ms`, never `0.8 s`): the config detail popup
/// shows the exact figure while tables stay compact with [`fmtLatency`].
function fmtLatencyMs(ms) {
  if (ms === null || ms === undefined) {
    return "—";
  }
  const v = Number(ms);
  if (!Number.isFinite(v)) {
    return "—";
  }
  return ltr(Math.round(v) + " ms");
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
    " " + fmtClock(iso);
}

/// Clock time only (`HH:MM:SS`): one-line badge text where the full stamp
/// would wrap mid-value. Pair with a `fmtStamp` tooltip carrying the date.
function fmtClock(iso) {
  if (!iso) {
    return "—";
  }
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) {
    return "—";
  }
  const pad = (n) => String(n).padStart(2, "0");
  return pad(d.getHours()) + ":" + pad(d.getMinutes()) + ":" + pad(d.getSeconds());
}

function fmtAgo(iso) {
  if (!iso) {
    return "—";
  }
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) {
    return "—";
  }
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 10) {
    return t("justNow");
  }
  if (s < 60) {
    return t("secAgo", { s });
  }
  const m = Math.floor(s / 60);
  if (m < 60) {
    return t("minAgo", { m });
  }
  const h = Math.floor(m / 60);
  if (h < 48) {
    return t("hrAgo", { h });
  }
  return t("dayAgo", { d: Math.floor(h / 24) });
}

/// Human-friendly elapsed time for "took …": whole units, never fractional
/// minutes (`2m 48s`, not `2.8 min`).
function fmtDuration(ms) {
  if (ms === null || ms === undefined) {
    return "—";
  }
  const n = Number(ms);
  if (!Number.isFinite(n)) {
    return "—";
  }
  const v = Math.max(0, n);
  if (v < 1000) {
    return ltr(Math.round(v) + " ms");
  }
  const totalSeconds = Math.round(v / 1000);
  if (totalSeconds < 60) {
    return ltr((v / 1000).toFixed(1) + " s");
  }
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) {
    return ltr(minutes + "m " + seconds + "s");
  }
  return ltr(Math.floor(minutes / 60) + "h " + (minutes % 60) + "m");
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
  const ms = Date.parse(state.startedAt);
  if (Number.isNaN(ms)) {
    return "—";
  }
  return fmtHMS((Date.now() - ms) / 1000);
}

/// Second line of the Refresh card: live ping countdown under `fetch in`,
/// same shape as the fetch line above it. Anchored at the server's last
/// deadline (re)arm, so it ticks down in real time via the 1s ticker and
/// survives idle stretches with no feed traffic — exactly like `refreshVal`.
/// Hidden while refreshing (a fetch already probes everything, so there is
/// no ping countdown to show) and when the ping cadence equals the refresh
/// cadence (a fetch on the same timer revalidates everything, so a separate
/// ping line would only duplicate it — same as the TUI's top bar).
function pingSub() {
  const s = state.snapshot;
  if (s && s.pinging) {
    return t("pingRunning");
  }
  if (!state.hasSummaryApi) {
    return "";
  }
  if (state.pingSeconds <= 0) {
    return t("pingOff");
  }
  if (s && s.refreshing) {
    return "";
  }
  if (state.refreshSeconds > 0 && state.pingSeconds === state.refreshSeconds) {
    return "";
  }
  const anchor = s && s.last_ping_at ? Date.parse(s.last_ping_at) : NaN;
  if (Number.isNaN(anchor)) {
    // Old server (or no arm yet): static interval until the anchor arrives.
    return t("pingEvery", { s: state.pingSeconds });
  }
  return t("pingIn", { countdown: fmtCountdown(Math.max(0, anchor + state.pingSeconds * 1000 - Date.now())) });
}

/// First line of the Refresh card (mirrors the TUI's fetch line).
function refreshVal() {
  const s = state.snapshot;
  if (!s) {
    return "—";
  }
  if (s.refreshing) {
    const start = s.refresh_started_at ? Date.parse(s.refresh_started_at) : NaN;
    return t("cycleRunning") + (Number.isNaN(start) ? "" : " " + fmtHMS((Date.now() - start) / 1000));
  }
  if (!state.hasSummaryApi || state.refreshSeconds <= 0) {
    return state.hasSummaryApi ? t("refreshManual") : "—";
  }
  const finSrc = s.refresh_finished_at || s.last_refresh;
  const fin = finSrc ? Date.parse(finSrc) : NaN;
  if (Number.isNaN(fin)) {
    return "—";
  }
  return t("fetchIn", { countdown: fmtCountdown(Math.max(0, fin + state.refreshSeconds * 1000 - Date.now())) });
}

function refreshStatus() {
  return { val: refreshVal(), sub: pingSub() };
}

/// True while a refresh cycle runs (system or manual): the next manual
/// refresh must not be sent at all (same refusal as the TUI's
/// "Refresh already running").
function refreshBusy() {
  const s = state.snapshot;
  return !!((s && s.refreshing) || state.cycleInflight.refresh);
}

/// True while any cycle runs: the next manual ping must not be sent at all
/// (same refusal as the TUI's "A cycle is already running").
function pingBusy() {
  const s = state.snapshot;
  return !!((s && (s.refreshing || s.pinging)) || state.cycleInflight.ping);
}

/// Lock the Refresh/Ping buttons to match backend busy state. Called on every
/// render path so a system-started cycle disables manual triggers even though
/// the user never clicked: no click while busy may reach the network.
function updateCycleButtons() {
  const rb = $("btn-refresh");
  const pb = $("btn-ping");
  const rBusy = refreshBusy();
  const pBusy = pingBusy();
  if (rb) {
    rb.disabled = rBusy;
    rb.title = rBusy ? t("btnRefreshBusy") : t("btnRefreshTitle");
  }
  if (pb) {
    pb.disabled = pBusy;
    pb.title = pBusy ? t("btnPingBusy") : t("btnPingTitle");
  }
}

function lastUpdateAgo() {
  if (state.snapshot && state.snapshot.last_refresh) {
    return fmtAgo(state.snapshot.last_refresh);
  }
  return t("lastUpdateFallback");
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

/// First Regional Indicator flag pair anywhere in `s` ("" when none) —
/// the config's own flag, mirroring the server's `extract_any_flag`.
function extractFlag(s) {
  if (typeof s !== "string") {
    return "";
  }
  const chars = [...s];
  for (let i = 0; i + 1 < chars.length; i += 1) {
    const a = chars[i].codePointAt(0);
    const b = chars[i + 1].codePointAt(0);
    if (a >= 0x1F1E6 && a <= 0x1F1FF && b >= 0x1F1E6 && b <= 0x1F1FF) {
      return chars[i] + chars[i + 1];
    }
  }
  return "";
}

/// ISO code behind an RI flag pair ("🇳🇱" -> "NL"); "" when malformed.
function flagCode(flag) {
  const chars = [...String(flag)];
  if (chars.length !== 2) {
    return "";
  }
  const a = chars[0].codePointAt(0) - 0x1F1E6 + 0x41;
  const b = chars[1].codePointAt(0) - 0x1F1E6 + 0x41;
  if (a < 0x41 || a > 0x5A || b < 0x41 || b > 0x5A) {
    return "";
  }
  return String.fromCharCode(a, b);
}

/// Country badge for a config row, mirroring the server display-name rule
/// (`format_display_name` in geoip.rs): the name's own flag always wins —
/// found anywhere, even when the GeoIP code disagrees; a bare two-letter
/// name ("NL") is a flagless country code and flags itself; the GeoIP code
/// only fills in when the name carries neither. Returns `{ flag, code }`
/// (`code` feeds the tooltip/aria-label). Both empty when no source has one.
function countryFlag(name, cc) {
  const own = extractFlag(name);
  if (own) {
    return { flag: own, code: flagCode(own) };
  }
  if (typeof name === "string") {
    const bare = name.trim();
    if (/^[A-Za-z]{2}$/.test(bare)) {
      const up = bare.toUpperCase();
      return { flag: flagFor(up), code: up };
    }
  }
  const flag = flagFor(cc);
  return { flag, code: flag ? String(cc).toUpperCase() : "" };
}

/// Display name for a config row, mirroring the server rule: a bare
/// two-letter remark ("NL") is a flagless country code — prepend its flag
/// but keep the provider's text ("🇳🇱 NL"), never reduce it to the flag
/// alone. Anything else renders verbatim.
function displayName(name) {
  if (typeof name === "string") {
    const bare = name.trim();
    if (/^[A-Za-z]{2}$/.test(bare)) {
      const flag = flagFor(bare);
      if (flag) {
        return flag + " " + bare;
      }
    }
  }
  return name;
}

/* ---------- connection / feed ---------- */

function setConn(connState, detail) {
  const box = document.querySelector(".conn");
  if (box) {
    box.setAttribute("data-state", connState);
  }
  const label = connState === "online" ? t("connConnected") : connState === "locked" ? t("connLocked") : t("connOffline");
  $("conn-state").textContent = detail ? label + " · " + detail : label;
  $("conn-bind").textContent = window.location.host || t("connLocal");
}

/* Single atomic feed-state transition: state.feed + pill + banner + footer.
 * All feed code must go through here; setConn/showBanner/hideBanner/setStatus
 * below are low-level helpers owned by this function (transient action
 * footers in triggerCycle/selectProxy/saveNow never touch pill or banner).
 */
function setFeedStatus(feed, opts) {
  const o = opts || {};
  state.feed = feed;
  // Stash the variant selectors (never translated strings) so a language
  // switch can re-derive this exact state via reapplyFeedStatus().
  if (feed === "polling") {
    state.feedPollFast = (o.detail || "") === t("feedPoll2");
  } else if (feed === "locked") {
    state.feedLockKind = o.lockKind;
  } else if (feed === "offline") {
    state.feedOfflineCustom = o.bannerTitle !== undefined || o.bannerText !== undefined
      || o.status !== undefined || (!!o.connDetail && o.connDetail !== t("connUnreachable"));
  }
  if (feed === "live") {
    setConn("online", t("feedLive"));
    hideBanner();
    setStatus(o.status || t("statusLive"));
    return;
  }
  if (feed === "polling") {
    const detail = o.detail || t("feedPoll10");
    setConn("online", detail);
    hideBanner();
    if (o.status) {
      setStatus(o.status);
    } else if (detail === t("feedPoll2")) {
      setStatus(t("statusPoll2"));
    } else {
      setStatus(t("statusPoll10", { ago: lastUpdateAgo() }));
    }
    return;
  }
  if (feed === "locked") {
    const sharingOff = o.lockKind === "sharing";
    setConn("locked", sharingOff ? t("lockSharingOff") : t("lockTokenRequired"));
    showBanner(
      t("lockTitle"),
      sharingOff ? t("lockSharingBody") : t("lockTokenBody"),
      false
    );
    setStatus(o.status || t("statusLocked"));
    return;
  }
  if (feed === "boot") {
    setConn("offline", t("connConnecting"));
    hideBanner();
    setStatus(t("statusConnecting"));
    return;
  }
  setConn("offline", o.connDetail || t("connUnreachable"));
  if (o.bannerTitle !== undefined || o.bannerText !== undefined) {
    showBanner(o.bannerTitle || t("bannerConnLost"), o.bannerText || "", o.retry !== false);
  } else if (state.snapshot) {
    showBanner(
      t("bannerConnLost"),
      t("bannerConnLostBody", { ago: lastUpdateAgo() }),
      true
    );
  } else {
    showBanner(t("bannerUnreachable"), t("bannerUnreachableBody"), true);
  }
  setStatus(o.status || t("statusOffline"));
}

/// Re-derive pill + banner + status line in the current language (language
/// switch). Custom offline banners (unexpected HTTP, feed-lost) are left
/// alone — the retry/reconnect timers refire them with fresh strings.
function reapplyFeedStatus() {
  if (state.feed === "live" || state.feed === "boot") {
    setFeedStatus(state.feed);
  } else if (state.feed === "polling") {
    setFeedStatus("polling", { detail: t(state.feedPollFast ? "feedPoll2" : "feedPoll10") });
  } else if (state.feed === "locked") {
    setFeedStatus("locked", { lockKind: state.feedLockKind });
  } else if (state.feed === "offline" && !state.feedOfflineCustom) {
    setFeedStatus("offline", {});
  }
}

/// Sticky "server is off" screen: no retry loop may overwrite it — only a
/// manual boot (Retry now) clears `serverStopped` and reconnects.
function showStopped() {
  showBanner(t("powerStoppedTitle"), t("powerStoppedBody"), false);
  setStatus(t("statusStopped"));
}

async function loadResults() {
  const r = await fetchJson("/results");
  if (r.status === 0) {
    if (state.serverStopped) {
      showStopped();
      return false;
    }
    setFeedStatus("offline", { connDetail: t("connUnreachable") });
    return false;
  }
  if (r.status === 401 || r.status === 403) {
    setFeedStatus("locked", { lockKind: r.status === 401 ? "token" : "sharing" });
    return false;
  }
  if (r.status !== 200 || !r.data) {
    if (state.serverStopped) {
      showStopped();
      return false;
    }
    setFeedStatus("offline", {
      connDetail: t("httpStatus", { status: r.status }),
      bannerTitle: t("bannerUnexpected"),
      bannerText: t("bannerUnexpectedBody", { status: r.status }),
      status: t("statusUnexpected"),
    });
    return false;
  }
  state.snapshot = r.data;
  syncProxyPending();
  ingestLogs(r.data);
  renderAll();
  if (state.feed !== "live" && state.feed !== "polling") {
    // Fresh data but no transport yet — the caller (boot/retry/reconnect)
    // upgrades to live or polling immediately; never leave a stale banner.
    setFeedStatus("polling", { detail: t("feedPoll10") });
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
    toast(t("configChanged"), "good");
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
        connDetail: t("connFeedLost"),
        bannerTitle: t("bannerFeedLost"),
        bannerText: t("bannerFeedLostBody"),
        status: t("statusReconnecting"),
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
        setFeedStatus("polling", { detail: t("feedPoll2") });
        applySummary(r.data);
        return;
      }
    }
    const ok = await loadResults();
    if (ok) {
      setFeedStatus("polling", { detail: t("feedPoll10") });
    }
  };
  const checkSummary = async () => {
    stopPolling();
    const r = await fetchJson("/api/summary");
    if (r.status === 200 && r.data) {
      state.hasSummaryApi = true;
      setFeedStatus("polling", { detail: t("feedPoll2") });
      applySummary(r.data);
      state.pollTimer = window.setInterval(tickSummary, POLL_SUMMARY_MS);
    } else if (r.status === 404) {
      // Old server without the summary API: stay on full polling.
      state.hasSummaryApi = false;
      void tick();
      state.pollTimer = window.setInterval(tick, POLL_RESULTS_MS);
    } else {
      // Transient (server restarting, blip): poll full results now, but do
      // not cement `false` — retry capabilities once so the Refresh
      // countdown and ping line recover instead of showing "—" forever.
      void tick();
      state.pollTimer = window.setInterval(tick, POLL_RESULTS_MS);
      window.setTimeout(() => {
        if (state.hasSummaryApi === null && !state.sse) {
          void checkSummary();
        }
      }, 30000);
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
        setFeedStatus("polling", { detail: t("feedPoll10") });
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
    syncProxyPending();
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
  syncProxyPending();
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
  // Fetched count rides the delta like tested/working: the fetch phase
  // lands before any probe result, so without this the badge sticks at the
  // hello value until a manual reload.
  if (typeof d.total === "number") {
    state.snapshot.total_candidates = d.total;
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
  // Live proxy state (pins from either UI, failovers, mode changes): merge
  // and refresh the proxy surfaces only when something actually moved.
  let proxyChanged = false;
  for (const k of ["proxy_running", "proxy_active_config", "proxy_active_uri",
    "proxy_port", "proxy_discoverable"]) {
    if (typeof d[k] !== "undefined" && state.snapshot[k] !== d[k]) {
      state.snapshot[k] = d[k];
      proxyChanged = true;
    }
  }
  const wasPending = state.proxyPendingUri;
  syncProxyPending();
  if (proxyChanged || (wasPending !== undefined && state.proxyPendingUri === undefined)) {
    renderOvConfigs();
    renderConfigs();
    renderProxyTab();
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
  renderOvConfigs();
  renderConfigs();
  void renderShare();
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
  void renderShare();
  void renderOvQr();
}

/// Stat badge: title on top, value in the middle, sub-line at the bottom.
/// The card is `min-inline-size: 0` with one-line ellipsis subs — nothing
/// may stick out of the border at any viewport width.
function statCard(label, value, sub, valueCls, valueId, subId, hint) {
  const p = el("p", null, "stat card");
  p.appendChild(el("span", label, "stat-label"));
  const v = el("span", value, "stat-value" + (valueCls ? " " + valueCls : ""));
  if (valueId) {
    v.id = valueId;
  }
  if (hint) {
    v.title = hint;
  }
  p.appendChild(v);
  if (sub) {
    const s = el("span", sub, "stat-sub");
    if (subId) {
      s.id = subId;
    }
    if (hint) {
      s.title = hint;
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
  preserveViewport(paintStats);
}

/// Overview cards: skeleton vs numbers split. The skeleton (labels, clock
/// nodes, structure) builds once per shape — language, snapshot presence,
/// refresh running — while probe deltas only refresh the number badges in
/// place. Clock nodes (uptime, refresh countdown, scan age, pill) are written
/// solely by tickClock: recomputing them mid-second on every delta made the
/// seconds visibly jump whenever Failed/Working updated (Math.round ages
/// disagree with the aligned ticker by a second either way), and rebuilding
/// seven cards per delta thrashed the DOM for no new information.
let statCardsShape = "";
function statCardsShapeOf(s) {
  return (document.documentElement.lang || "") + ":" + (!!s) + ":" + (!!s && !!s.refreshing);
}

function paintStats() {
  const box = $("stat-cards");
  if (!box) {
    return;
  }
  const s = state.snapshot;
  const shape = statCardsShapeOf(s);
  if (statCardsShape !== shape) {
    buildStatCards(box, s);
    statCardsShape = shape;
  } else if (s) {
    updateStatNumbers(s);
  }
  updateCycleButtons();
}

/// Number badges only (probe deltas): clock nodes are owned by tickClock.
function updateStatNumbers(s) {
  setText($("stat-fetched-val"), String(s.total_candidates || 0));
  const failed = Math.max(0, (s.tested_candidates || 0) - (s.reachable_candidates || 0));
  setText($("stat-failed-val"), String(failed));
  setText($("stat-failed-sub"), t("failedOfFetched", { fetched: s.total_candidates || 0 }));
  setText($("stat-working-val"), String(s.reachable_candidates || 0));
  setText($("stat-working-sub"), t("workingOfTested", { tested: s.tested_candidates || 0 }));
  setText($("stat-subusage-val"), fmtBytes(s.fetch_bytes));
  const tookNode = $("stat-scan-took");
  if (tookNode) {
    const took = s.refresh_duration_ms !== null && s.refresh_duration_ms !== undefined
      ? t("tookMs", { dur: fmtDuration(s.refresh_duration_ms) })
      : "";
    if (took) {
      setText(tookNode, took);
    }
  }
}

/// Full card skeleton (first paint, language switch, refresh start/stop,
/// snapshot arrival). Clock texts are painted once here and owned by
/// tickClock afterwards; see updateStatNumbers.
function buildStatCards(box, s) {
  while (box.firstChild) {
    box.removeChild(box.firstChild);
  }
  if (!s) {
    box.appendChild(statCard("Status", t("statusNoData")));
    renderUpdated();
    return;
  }
  const failed = Math.max(0, (s.tested_candidates || 0) - (s.reachable_candidates || 0));
  const tested = s.tested_candidates || 0;
  const rs = refreshStatus();
  const ago = fmtAgo(s.last_refresh);
  const took = s.refresh_duration_ms !== null && s.refresh_duration_ms !== undefined
    ? t("tookMs", { dur: fmtDuration(s.refresh_duration_ms) })
    : "";
  // Like the TUI (which clears the duration when a cycle starts), the Last
  // scan badge reads "—" while a refresh runs — the stamp underneath is
  // stale until the cycle finishes.
  const scanRunning = !!s.refreshing;
  const scanVal = scanRunning ? "—" : fmtClock(s.last_refresh);
  const scanHint = scanRunning ? "" : fmtStamp(s.last_refresh);
  // Age above duration on two stacked sub-lines (no "·" joiner): each line
  // ellipsizes on its own, and tickClock refreshes the age every second
  // like the green pill.
  const scanCard = statCard(t("cardLastScan"), scanVal, null, null, null, null, scanHint);
  if (!scanRunning) {
    const ageLine = el("span", ago, "stat-sub");
    ageLine.id = "stat-scan-age";
    ageLine.title = scanHint;
    scanCard.appendChild(ageLine);
    if (took) {
      const tookLine = el("span", took, "stat-sub");
      tookLine.id = "stat-scan-took";
      tookLine.title = scanHint;
      scanCard.appendChild(tookLine);
    }
  }
  // TUI top-strip parity: Running For / Refresh / Last Scan / Fetched /
  // Failed / Working / Sub Usage. Seven tight badges share one row (narrow
  // landscape viewports scroll horizontally instead of wrapping or
  // overflowing); portrait phones hide Fetched (`#stat-fetched`) and show
  // the remaining six in a fixed 3x2 grid — see style.css.
  box.appendChild(statCard(t("cardRunningFor"), uptimeText(), state.startedAt ? t("startedAt", { time: fmtClock(state.startedAt) }) : "", null, "stat-running", null, state.startedAt ? t("startedAt", { time: fmtStamp(state.startedAt) }) : ""));
  box.appendChild(statCard(t("cardRefresh"), rs.val, rs.sub, null, "stat-refresh-val", "stat-refresh-sub"));
  box.appendChild(scanCard);
  const fetched = statCard(t("cardFetched"), String(s.total_candidates || 0), null, null, "stat-fetched-val");
  fetched.id = "stat-fetched";
  box.appendChild(fetched);
  box.appendChild(statCard(t("cardFailed"), String(failed), t("failedOfFetched", { fetched: s.total_candidates || 0 }), null, "stat-failed-val", "stat-failed-sub"));
  box.appendChild(statCard(t("cardWorking"), String(s.reachable_candidates || 0), t("workingOfTested", { tested }), null, "stat-working-val", "stat-working-sub"));
  box.appendChild(statCard(t("cardSubUsage"), fmtBytes(s.fetch_bytes), null, null, "stat-subusage-val"));
  renderUpdated();
}

/// Overview "updated" pill: hidden with no data yet, green dot only while
/// a refresh runs (the stamp underneath is stale then, like the Last scan
/// card which reads "—" mid-cycle), dot + age once the refresh finished.
/// The pill counts from the freshest data touch — refresh OR ping — so
/// after a ping it honestly runs ahead of the Last scan card, which keeps
/// the refresh anchor only. The age phrase from `fmtAgo` is already a
/// complete localized string, so it is concatenated — never nested inside
/// another `t()` substitution (nesting is direction-safe since `t()` uses
/// FSI isolates, but concatenation keeps the pill's two runs independently
/// wrappable).
function renderUpdated() {
  const s = state.snapshot;
  const stamp = s && !s.refreshing ? pillStamp(s) : null;
  const wrap = $("ov-updated-wrap");
  if (wrap) {
    wrap.hidden = !s || (!s.refreshing && !stamp);
  }
  setText($("ov-updated"), stamp ? t("ovUpdated") + " " + fmtAgo(stamp) : "");
}

/// Freshest data touch for the green pill: a ping re-times every ranked
/// row, so the pill counts from the newer of the last refresh and the last
/// ping (either anchor missing or unparseable is skipped; null when neither
/// parses). Old servers without `last_ping_at` degrade to refresh-only.
function pillStamp(s) {
  let best = null;
  let bestMs = NaN;
  for (const iso of [s.last_refresh, s.last_ping_at]) {
    if (typeof iso !== "string") {
      continue;
    }
    const ms = Date.parse(iso);
    if (Number.isNaN(ms)) {
      continue;
    }
    if (best === null || ms > bestMs) {
      best = iso;
      bestMs = ms;
    }
  }
  return best;
}

/// Last completed refresh stamp — the Last scan card's anchor only. A ping
/// re-times rows but is not a scan, so it must not move this clock (see
/// pillStamp for the pill's fresher anchor).
function lastRefreshStamp(s) {
  const iso = s.last_refresh;
  if (!iso || Number.isNaN(Date.parse(iso))) {
    return null;
  }
  return iso;
}

/// Wall-clock-aligned live ticker: uptime + countdowns only. A plain
/// `setInterval(1000)` drifts (late wakeups, background throttling), so the
/// displayed second would lag real time and then visibly jump whenever a data
/// render recomputed it — exactly the "seconds jump when Failed updates"
/// bug. Instead each tick schedules the next one for just past the upcoming
/// second boundary, so ticker and renders always agree to the second.
/// Writes are change-guarded: identical text never touches the DOM.
function tickClock() {
  // While offline the server is gone: freeze the clock on the last known
  // values instead of counting up from a stale anchor. Reconnecting resumes
  // from the fresh snapshot (catching up honestly in one step).
  if (state.snapshot && state.feed !== "offline") {
    setText($("stat-running"), uptimeText());
    const rs = refreshStatus();
    setText($("stat-refresh-val"), rs.val);
    setText($("stat-refresh-sub"), rs.sub);
    // Last-scan age ticks every second like the pill; the took line only
    // changes when a cycle finishes (renderStats). Mid-refresh the badge
    // reads "—" with no age element, so the write is a safe no-op then.
    if (!state.snapshot.refreshing) {
      setText($("stat-scan-age"), fmtAgo(state.snapshot.last_refresh));
    }
    renderUpdated();
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
    pill.textContent = t("pillRunning");
    pill.classList.add("good");
  } else if (running) {
    pill.textContent = t("pillRunningBare");
    pill.classList.add("warn");
  } else {
    pill.textContent = t("pillOff");
  }
  return pill;
}

/// Overview "Recent logs": last 5 lines, newest at the bottom (TUI order).
function renderOvLogs() {
  preserveViewport(paintOvLogs);
}

/// Synchronous DOM rebuild for renderOvLogs (viewport-preserving wrapper above).
function paintOvLogs() {
  const list = $("ov-logs");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  const visible = [];
  for (const line of state.logLines) {
    const parsed = parseLogLine(line);
    if (!logLineVisible(parsed, state.logLevel, "", line)) {
      continue;
    }
    visible.push({ line, parsed });
  }
  const tail = visible.slice(-5);
  if (tail.length === 0) {
    const li = el("li", t("ovLogsEmpty"), "muted");
    list.appendChild(li);
    return;
  }
  for (const { line, parsed } of tail) {
    const li = document.createElement("li");
    li.title = line;
    if (parsed.ts) {
      li.appendChild(el("span", parsed.ts, "log-ts"));
      li.appendChild(document.createTextNode(" "));
    }
    const badge = el("span", parsed.level, "log-lvl");
    badge.classList.add("lvl-" + parsed.level.toLowerCase());
    li.appendChild(badge);
    li.appendChild(document.createTextNode(" "));
    li.appendChild(el("span", parsed.msg, "log-msg"));
    list.appendChild(li);
  }
}

/// True while the overview QR image is actually on screen (loaded for the
/// current sheet and unhidden). The Endpoints column then grows tall, so
/// Top configs stretches to match instead of leaving a gap underneath.
function ovQrShown() {
  return !!(state.ovQrLoaded && !$("ov-qr-img").hidden);
}

/// Overview "Top configs": first 8 reachable, compact (TUI found-panel
/// slice) — up to 15 while the overview QR image is shown, filling the
/// taller Endpoints column instead of leaving a gap below the table.
function renderOvConfigs() {
  preserveViewport(paintOvConfigs);
}

/// Synchronous DOM rebuild for renderOvConfigs (viewport-preserving wrapper above).
function paintOvConfigs() {
  const body = $("ov-cfg-body");
  while (body.firstChild) {
    body.removeChild(body.firstChild);
  }
  const s = state.snapshot;
  const rows = s && Array.isArray(s.ranked)
    ? s.ranked.filter((c) => c.reachable).slice(0, ovQrShown() ? 15 : 8)
    : [];
  $("ov-cfg-empty").hidden = rows.length !== 0;
  const activeUri = s && s.proxy_active_uri ? s.proxy_active_uri : "";
  for (const c of rows) {
    const tr = document.createElement("tr");
    if (activeUri && c.uri && c.uri === activeUri) {
      tr.className = "is-proxy";
    } else if (proxyRowState(c.uri) === "pending") {
      tr.className = "is-pending";
    }
    tr.appendChild(cell(c.rank !== undefined ? String(c.rank) : "—", t("thRank"), "cell-num"));
    const nameTd = document.createElement("td");
    nameTd.className = "cell-main";
    nameTd.appendChild(el("strong", displayName(c.name) || t("unnamed")));
    tr.appendChild(nameTd);
    tr.appendChild(cell(fmtLatency(c.latency_ms), t("thLatency"), "cell-num"));
    wireRowDialog(tr, c);
    body.appendChild(tr);
  }
}

/// Make a config row open the detail popup on click (or Enter/Space when
/// focused). Clicks on in-row buttons (Use/Copy/QR) keep their own action
/// and never open the dialog.
function wireRowDialog(tr, c) {
  tr.tabIndex = 0;
  tr.title = t("rowOpenTip");
  tr.addEventListener("click", (ev) => {
    if (ev.target && ev.target.closest && ev.target.closest("button")) {
      return;
    }
    openDetail(c);
  });
  tr.addEventListener("keydown", (ev) => {
    if (ev.target && ev.target !== tr) {
      return;
    }
    if (ev.key === "Enter" || ev.key === " ") {
      ev.preventDefault();
      openDetail(c);
    }
  });
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
  preserveViewport(paintOvConfig);
}

/// Synchronous DOM rebuild for renderOvConfig (viewport-preserving wrapper above).
function paintOvConfig() {
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
    note.textContent = state.hasSummaryApi === false ? t("ovCfgOld") : t("ovCfgLoading");
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
    proxyText = t("pillOff");
  } else if (proxyPort) {
    proxyText = t("yes") + " http://" + window.location.hostname + ":" + proxyPort + (proxyLan ? t("ovProxyLan") : t("ovProxyLocal"));
  } else {
    proxyText = t("yes");
  }
  ovConfigRow(net, "proxy", proxyText);
  if (svc.children.length === 0 && net.children.length === 0) {
    note.hidden = false;
    note.textContent = t("ovCfgNone");
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
  // /subscription honors the encoded_subscription setting (base64 unless
  // turned off); /subscription.txt is always plain readable text.
  const enc = state.ovConfig ? state.ovConfig.get("encoded_subscription") : undefined;
  const base64 = enc !== "false" && enc !== false;
  const items = [
    [base64 ? t("epBase64") : t("epPlainText"), origin + "/subscription"],
    [t("epPlainText"), origin + "/subscription.txt"],
    [t("epMihomo"), origin + "/mihomo.yaml"],
  ];
  // Same link the QR sheet encodes (qr.rs `telegram_proxy_url`): shown under
  // the same conditions as the sheet's proxy card (proxy LAN-enabled).
  // Host follows the dashboard URL, so opening via the LAN IP yields the
  // exact QR link; the firewall check stays server-side with the sheet.
  const tg = telegramProxyUrl();
  if (tg) {
    items.push([t("epTelegram"), tg]);
  }
  items.push([t("epHealth"), origin + "/health"]);
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
  preserveViewport(paintEndpoints);
}

/// Synchronous DOM rebuild for renderEndpoints (viewport-preserving wrapper above).
function paintEndpoints() {
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
    const btn = el("button", t("btnCopy"), "btn small");
    btn.type = "button";
    btn.addEventListener("click", () => void copyText(url, t("epCopied")));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

function renderFetchErrors() {
  preserveViewport(paintFetchErrors);
}

/// Synchronous DOM rebuild for renderFetchErrors (viewport-preserving wrapper above).
function paintFetchErrors() {
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
  $("fetch-errors-sub").textContent = errors.length === 1
    ? t("fetchErrSub1", { n: errors.length })
    : t("fetchErrSubN", { n: errors.length });
  const groups = new Map();
  for (const e of errors.slice(0, 60)) {
    groups.set(e, (groups.get(e) || 0) + 1);
  }
  const lines = [];
  for (const [msg, n] of groups) {
    lines.push(n > 1 ? ltr("\u00d7" + n) + " " + msg : msg);
  }
  const SHOWN = 6;
  for (const line of lines.slice(0, SHOWN)) {
    list.appendChild(el("li", line));
  }
  if (lines.length > SHOWN) {
    const details = document.createElement("details");
    details.className = "more";
    const summary = document.createElement("summary");
    summary.textContent = t("showMore", { n: lines.length - SHOWN });
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
  preserveViewport(paintConfigs);
}

/// Synchronous DOM rebuild for renderConfigs (viewport-preserving wrapper above).
function paintConfigs() {
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
    : t("cfgCount", {
      n: rows.length,
      total,
      limit: limited ? t("cfgLimit", { limit: state.rowLimit }) : "",
    });
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
    } else if (proxyRowState(c.uri) === "pending") {
      tr.className = "is-pending";
    }
    tr.appendChild(cell(c.rank !== undefined ? String(c.rank) : "—", t("thRank"), "cell-num"));

    const nameTd = document.createElement("td");
    nameTd.className = "cell-main";
    nameTd.appendChild(el("strong", displayName(c.name) || t("unnamed")));
    if (c.source) {
      nameTd.appendChild(el("div", c.source, "muted"));
    }
    tr.appendChild(nameTd);

    const protoTd = document.createElement("td");
    protoTd.dataset.th = t("thProtocol");
    protoTd.appendChild(el("span", c.protocol || "?", "proto"));
    tr.appendChild(protoTd);

    tr.appendChild(cell(maskedHost(c.endpoint), t("thEndpoint")));

    const latTd = document.createElement("td");
    latTd.className = "cell-lat";
    latTd.dataset.th = t("thLatency");
    latTd.textContent = fmtLatency(c.latency_ms);
    const bar = el("span", null, "lat-bar");
    const fillSpan = document.createElement("span");
    const pct = typeof c.latency_ms === "number" ? Math.max(4, Math.min(100, Math.round((c.latency_ms / maxLat) * 100))) : 0;
    fillSpan.style.inlineSize = pct + "%";
    bar.appendChild(fillSpan);
    latTd.appendChild(bar);
    tr.appendChild(latTd);

    tr.appendChild(cell(c.stability_count ? ltr("\u00d7" + c.stability_count) : "—", t("thStability"), "cell-num"));
    // Flag glyph only (+ code in the tooltip): regional indicators render as
    // the two letters on platforms without flag emoji (notably Windows), so
    // showing both would duplicate ("DE DE"). The flag mirrors the server's
    // display-name rule: the config's own flag (found anywhere in the name)
    // wins over the GeoIP country code.
    const cc = countryFlag(c.name, c.country_code);
    const ccTd = document.createElement("td");
    ccTd.dataset.th = t("thCountry");
    if (cc.flag) {
      const glyph = el("span", cc.flag);
      glyph.setAttribute("role", "img");
      glyph.setAttribute("aria-label", cc.code);
      glyph.title = cc.code;
      ccTd.appendChild(glyph);
    } else {
      ccTd.textContent = "—";
    }
    tr.appendChild(ccTd);

    const actTd = document.createElement("td");
    actTd.className = "cell-actions";
    const wrap = el("span", null, "row-actions");
    const rowState = proxyRowState(c.uri);
    const useBtn = el("button", rowState === "active" ? t("useActive") : rowState === "pending" ? t("usePending") : t("useIdle"), "btn small");
    useBtn.type = "button";
    useBtn.disabled = rowState === "pending";
    useBtn.title = rowState === "active"
      ? t("tipActiveProxy")
      : rowState === "pending"
        ? t("tipPendingProxy")
        : t("tipPinProxy");
    useBtn.addEventListener("click", () => void toggleProxy(c.uri));
    const copyBtn = el("button", t("btnCopy"), "btn small");
    copyBtn.type = "button";
    copyBtn.addEventListener("click", () => void copyText(c.uri || "", t("linkCopied")));
    const qrBtn = el("button", t("btnQR"), "btn small");
    qrBtn.type = "button";
    qrBtn.title = t("tipQrRow");
    qrBtn.addEventListener("click", () => openQr(c));
    wrap.appendChild(useBtn);
    wrap.appendChild(copyBtn);
    wrap.appendChild(qrBtn);
    actTd.appendChild(wrap);
    tr.appendChild(actTd);

    // The row itself opens the detail popup; the Detail button is gone.
    wireRowDialog(tr, c);
    body.appendChild(tr);
  }
}

function openDetail(c) {
  state.detailUri = c.uri || "";
  $("dlg-detail-title").textContent = t("dlgDetail");
  const kv = $("dlg-detail-kv");
  while (kv.firstChild) {
    kv.removeChild(kv.firstChild);
  }
  const cc = countryFlag(c.name, c.country_code);
  const pairs = [
    [t("fName"), displayName(c.name) || t("unnamed")],
    [t("fRank"), c.rank !== undefined ? String(c.rank) : "—"],
    [t("fProtocol"), c.protocol || "—"],
    [t("fEndpoint"), maskedHost(c.endpoint)],
    [t("fSource"), c.source || "—"],
    [t("fReachable"), c.reachable ? t("yes") : t("no")],
    [t("fStability"), c.stability_count ? ltr("\u00d7" + c.stability_count) : "—"],
    [t("fValidation"), c.validation || "—"],
    [t("fLatency"), fmtLatencyMs(c.latency_ms)],
    [t("fHttp"), c.http_status !== null && c.http_status !== undefined ? String(c.http_status) : "—"],
    [t("fSpeed"), c.download_mbps !== null && c.download_mbps !== undefined ? ltr(Number(c.download_mbps).toFixed(2) + t("speedUnit")) : "—"],
    [t("fCountry"), cc.flag ? cc.flag : "—"],
    [t("fError"), c.error || "—"],
  ];
  for (const [k, v] of pairs) {
    kv.appendChild(el("dt", k));
    const dd = el("dd", v);
    if (k === t("fCountry") && cc.flag) {
      dd.title = cc.code;
      dd.setAttribute("role", "img");
      dd.setAttribute("aria-label", cc.code);
    }
    kv.appendChild(dd);
  }
  // QR alongside the facts: same client-side encoder as the per-row QR
  // button (link stays in JS, painted to canvas, never in the DOM).
  // Quiet: a missing encoder just hides the QR block instead of toasting
  // on every dialog open.
  const qrWrap = $("dlg-detail-qr-wrap");
  if (c.uri && drawQr(c.uri, "dlg-detail-qr", true)) {
    qrWrap.hidden = false;
  } else {
    qrWrap.hidden = true;
  }
  const useBtn = $("dlg-detail-use");
  const detailState = proxyRowState(c.uri);
  useBtn.disabled = detailState !== "idle";
  useBtn.textContent = detailState === "active" ? t("btnActiveProxy") : detailState === "pending" ? t("usePending") : t("btnUseAsProxy");
  useBtn.title = detailState === "active" ? t("tipActiveProxy") : "";
  const dlg = $("dlg-detail");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

function openQr(c) {
  const uri = c.uri || "";
  if (!uri) {
    toast(t("qrNoLink"), "bad");
    return;
  }
  // Kept in a JS variable only — the full link is passed to the encoder
  // and painted to canvas, never written into the DOM as text.
  state.qrUri = uri;
  $("dlg-qr-title").textContent = t("qrTitlePrefix", { name: displayName(c.name) || t("unnamed") });
  if (!drawQr(uri)) {
    return;
  }
  const dlg = $("dlg-qr");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

function drawQr(text, canvasId, silent) {
  const canvas = $(canvasId || "qr-canvas");
  if (typeof QREncode === "undefined" || !QREncode) {
    if (!silent) {
      toast(t("qrEncoderMissing"), "bad");
    }
    return false;
  }
  let model = null;
  try {
    model = QREncode.encode(text, QREncode.CorrectLevel.M);
  } catch (err) {
    if (!silent) {
      toast(t("qrTooLong"), "bad");
    }
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

/// Per-row proxy state for button labels: `active` (server-confirmed),
/// `pending` (requested, confirmation in flight), or `idle`. Mirrors the
/// TUI's `proxy_pending_uri`: a pending row only flips to active once
/// `proxy_active_uri` catches up in a fresh snapshot.
function proxyRowState(uri) {
  const s = state.snapshot;
  const activeUri = s && s.proxy_active_uri ? s.proxy_active_uri : null;
  if (state.proxyPendingUri !== undefined && uri === state.proxyPendingUri) {
    return activeUri === uri ? "active" : "pending";
  }
  return activeUri && uri === activeUri ? "active" : "idle";
}

/// Clear a pending switch once the server confirms it. Called on every path
/// that installs a fresh snapshot; the list re-renders right after, so the
/// Pending… button flips to Active without another round trip.
function syncProxyPending() {
  if (state.proxyPendingUri !== undefined && state.snapshot &&
    state.snapshot.proxy_active_uri === state.proxyPendingUri) {
    state.proxyPendingUri = undefined;
  }
}

/// Drop the optimistic lock and repaint the proxy surfaces (failures,
/// settle timeouts, stale locks).
function clearProxyPending() {
  state.proxyPendingUri = undefined;
  renderConfigs();
  renderOvConfigs();
  renderProxyTab();
}

/// Row/dialog proxy action with TUI `Enter` toggle semantics: clicking the
/// confirmed-active config unpins back to auto-select instead of re-firing
/// a no-op pin. The optimistic lock is set synchronously (like
/// `cycleInflight`), so same-tick double clicks collapse to one POST and
/// further clicks while a switch runs are refused with a toast instead of
/// stacking duplicate pins.
async function toggleProxy(uri) {
  if (!uri) {
    await selectProxy(uri);
    return;
  }
  if (state.proxyPendingUri !== undefined) {
    toast(t("proxyInflight"), "bad");
    return;
  }
  const s = state.snapshot;
  const activeUri = s && s.proxy_active_uri;
  const unpin = !!(activeUri && uri === activeUri);
  state.proxyPendingUri = unpin ? null : uri;
  renderConfigs();
  renderOvConfigs();
  renderProxyTab();
  await selectProxy(unpin ? null : uri);
}

async function selectProxy(uri) {
  // Explicit `null` unpins back to auto-select (same as the TUI toggle-off).
  if (uri === null) {
    /* unpin path continues below */
  } else if (!uri) {
    toast(t("noLink"), "bad");
    return;
  }
  const body = uri === null ? { uri: null } : { uri };
  const r = await fetchJson("/api/proxy/select", { method: "POST", body });
  if (r.status === 404) {
    toast(t("proxySelectOld"), "bad");
    clearProxyPending();
    return;
  }
  if (r.status === 0) {
    toast(t("unreachable"), "bad");
    clearProxyPending();
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    // Re-affirm the lock set synchronously by the caller (idempotent):
    // the button already shows Pending… so clicks during the 1.5 s
    // confirmation window are refused instead of duplicating the pin.
    state.proxyPendingUri = uri;
    renderConfigs();
    renderOvConfigs();
    renderProxyTab();
    toast(subMessage(r, t("proxyRequested")), "good");
    setStatus(subMessage(r, t("proxyRequestedShort")));
    window.setTimeout(() => void loadResults(), 1500);
    // Safety net: a switch can take a while (proxy restart) or never land
    // (proxy off, start failure). If this exact request is still unconfirmed
    // after 10 s, resync once more and say so instead of Pending… forever.
    const waiting = uri;
    window.setTimeout(() => {
      if (state.proxyPendingUri === waiting) {
        void settleProxyPending();
      }
    }, 10000);
    return;
  }
  clearProxyPending();
  toast(subMessage(r, t("proxyFailed", { status: r.status })), "bad");
}

/// Settle an unconfirmed proxy switch: resync once, and if the server still
/// has not applied it, drop the Pending… lock and say why it may be stuck
/// (proxy off, switch failure) instead of leaving the button wedged.
async function settleProxyPending() {
  if (state.proxyPendingUri === undefined) {
    return;
  }
  const ok = await loadResults();
  if (!ok || state.proxyPendingUri === undefined) {
    return;
  }
  clearProxyPending();
  toast(t("proxyUnconfirmed"), "bad");
  setStatus(t("proxyUnconfirmedShort"));
}

async function copyText(text, okMsg) {
  if (!text) {
    toast(t("copyNothing"), "bad");
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
    toast(done ? okMsg : t("copyManual"), done ? "good" : "bad");
  } catch (err) {
    toast(t("copyFailed"), "bad");
  }
}

/* ---------- logs ---------- */

/// Split a raw log line into `{ ts, level, msg }`. Tagged lines look like
/// `20:20:02.792 [INFO] message`; unknown level tokens fall back to INFO
/// (keeping the message), and legacy lines without a prefix — plus any
/// non-string input (shown in string form, like before) — yield an empty
/// ts with level INFO.
function parseLogLine(line) {
  if (typeof line !== "string") {
    return { ts: "", level: "INFO", msg: String(line) };
  }
  const m = line.match(/^(\d{2}:\d{2}:\d{2}\.\d{3}) \[([A-Z]+)\] ([\s\S]+)$/);
  if (m) {
    return {
      ts: m[1],
      level: LOG_LEVELS.includes(m[2]) ? m[2] : "INFO",
      msg: m[3],
    };
  }
  return { ts: "", level: "INFO", msg: line };
}

/// True when the parsed line passes the severity floor (`"ALL"` shows
/// everything) and the lowercased text query matches the raw line —
/// timestamp, level tag and message, exactly like the text filter always
/// has. Severity itself is what the Level dropdown is for.
function logLineVisible(parsed, minLevel, query, rawLine) {
  const levelOk = minLevel === "ALL"
    || LOG_LEVEL_ORDER[parsed.level] >= LOG_LEVEL_ORDER[minLevel];
  if (!levelOk) {
    return false;
  }
  return !query || String(rawLine).toLowerCase().includes(query);
}

function renderLogs() {
  preserveViewport(paintLogs);
}

/// Synchronous DOM rebuild for renderLogs (viewport-preserving wrapper above).
function paintLogs() {
  const list = $("log-list");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  const q = state.logFilter.trim().toLowerCase();
  let shown = 0;
  for (const line of state.logLines) {
    const parsed = parseLogLine(line);
    if (!logLineVisible(parsed, state.logLevel, q, line)) {
      continue;
    }
    const li = document.createElement("li");
    if (parsed.ts) {
      li.appendChild(el("span", parsed.ts, "log-ts"));
      li.appendChild(document.createTextNode(" "));
    }
    const badge = el("span", parsed.level, "log-lvl");
    badge.classList.add("lvl-" + parsed.level.toLowerCase());
    li.appendChild(badge);
    li.appendChild(document.createTextNode(" "));
    li.appendChild(el("span", parsed.msg, "log-msg"));
    list.appendChild(li);
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

/// Set the proxy mode directly (Off/Local/LAN). One switch in flight at a
/// time — rapid clicks collapse like every other manual trigger.
async function setProxyMode(mode) {
  if (state.proxyModeInflight) {
    toast(t("proxyInflight"), "bad");
    return;
  }
  state.proxyModeInflight = true;
  updateProxyModeButtons();
  try {
    const r = await fetchJson("/api/proxy/mode", { method: "POST", body: { mode } });
    if (r.status === 404) {
      toast(t("proxyModeOld"), "bad");
      return;
    }
    if (r.status === 0) {
      toast(t("unreachable"), "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300) {
      const elevated = firewallElevation(r);
      if (elevated) {
        showAdminGuide(elevated);
        window.setTimeout(() => void loadResults(), 1200);
        return;
      }
      toast(subMessage(r, t("proxyModeSet")), "good");
      window.setTimeout(() => void loadResults(), 1200);
      return;
    }
    toast(subMessage(r, t("proxyFailed", { status: r.status })), "bad");
  } finally {
    state.proxyModeInflight = false;
    updateProxyModeButtons();
  }
}

/// Current proxy mode from the live snapshot (`off` when not running —
/// same reading as the status pill), pressed into the segmented control.
function proxyMode() {
  const s = state.snapshot;
  if (!s || !s.proxy_running) {
    return "off";
  }
  return s.proxy_discoverable ? "lan" : "local";
}

/// Lock the mode segments in flight, press the live mode otherwise. Called
/// on every proxy-tab render so a change from any UI (or the resync after
/// our own POST) reflects within one render.
function updateProxyModeButtons() {
  const mode = proxyMode();
  for (const m of ["off", "local", "lan"]) {
    const b = $("proxy-" + m);
    if (!b) {
      continue;
    }
    b.disabled = state.proxyModeInflight;
    b.setAttribute("aria-pressed", String(m === mode));
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
  preserveViewport(paintProxyTab);
}

/// Synchronous DOM rebuild for renderProxyTab (viewport-preserving wrapper above).
function paintProxyTab() {
  const s = state.snapshot;
  const pillSlot = $("proxy-pill");
  while (pillSlot.firstChild) {
    pillSlot.removeChild(pillSlot.firstChild);
  }
  if (!s) {
    pillSlot.textContent = t("proxyUnknown");
    kvFill($("proxy-kv"), []);
    updateProxyModeButtons();
    return;
  }
  pillSlot.appendChild(proxyPill(s.proxy_running, s.proxy_active_uri));
  kvFill($("proxy-kv"), [
    [t("kvRunning"), s.proxy_running ? t("yes") : t("no")],
    [t("kvActiveConfig"), s.proxy_active_config || "—"],
    [t("kvPort"), s.proxy_port !== null && s.proxy_port !== undefined ? String(s.proxy_port) : "—"],
    [t("kvLan"), s.proxy_discoverable ? t("yes") : t("no")],
    [t("kvPool"), String(s.reachable_candidates || 0)],
  ]);
  $("proxy-manual").textContent = s.proxy_active_uri ? t("proxyManualHeld") : t("proxyNonePinned");
  // Locked while an unpin is in flight (snapshot still shows the old pin
  // until the server confirms), so rapid clicks cannot stack duplicate
  // unpin POSTs; a pin in flight keeps it clickable to cancel the pin.
  $("btn-proxy-unpin").disabled = !s.proxy_active_uri || state.proxyPendingUri === null;
  updateProxyModeButtons();
}

/// Label for a server share-URL key: the `/subscription` label follows the
/// encoded_subscription setting like the local list does.
function shareUrlLabel(key) {
  if (key === "mihomo") {
    return t("epMihomo");
  }
  if (key === "subscription_txt") {
    return t("epPlainText");
  }
  const enc = state.ovConfig ? state.ovConfig.get("encoded_subscription") : undefined;
  return enc !== "false" && enc !== false ? t("epBase64") : t("epPlainText");
}

async function renderShare() {
  return preserveViewport(paintShare);
}

/// DOM rebuild for renderShare (viewport-preserving wrapper above).
async function paintShare() {
  const list = $("share-list");
  while (list.firstChild) {
    list.removeChild(list.firstChild);
  }
  // The hint never depends on the server: paint it synchronously so language
  // switches re-tint it in the same tick.
  $("share-hint").textContent = getToken() ? t("shareHintToken") : t("shareHintLocal");
  // Prefer the server's tokenized LAN URLs (the raw token stays masked, so
  // the client could never build these); fall back to the origin-based list
  // on old servers or while sharing is off.
  let items = null;
  const r = await fetchJson("/api/share-urls");
  if (r.status === 200 && r.data && Array.isArray(r.data.urls) && r.data.urls.length > 0) {
    items = r.data.urls.map((u) => [shareUrlLabel(u.key), u.url]);
  }
  for (const [label, url] of items || endpointItems().slice(0, 3)) {
    const li = document.createElement("li");
    li.appendChild(el("strong", label));
    const code = el("code", url);
    code.title = url;
    li.appendChild(code);
    const btn = el("button", t("btnCopy"), "btn small");
    btn.type = "button";
    btn.addEventListener("click", () => void copyText(url, t("subUrlCopied")));
    li.appendChild(btn);
    list.appendChild(li);
  }
  const img = $("qr-img");
  const note = $("qr-note");
  img.hidden = true;
  note.textContent = t("qrLoading");
  loadQrImage();
}

async function loadQrImage() {
  const img = $("qr-img");
  const note = $("qr-note");
  img.hidden = true;
  if ($("panel-share").hidden) {
    // Cheap path: don't probe a file nobody is looking at.
    note.textContent = t("qrOpenTab");
    return;
  }
  if (state.hasSummaryApi === false) {
    note.textContent = t("qrNeedsServer");
    return;
  }
  if (state.qrKnown === null) {
    // Exactly one metadata answer per session; a missing file then costs
    // zero failed requests (failed fetches log console errors).
    const r = await fetchJson("/api/summary");
    if (r.status === 200 && r.data && typeof r.data.qr_available === "boolean") {
      state.hasSummaryApi = true;
      storeCapabilities(r.data);
    } else if (r.status === 404) {
      // Old server without the summary API.
      state.hasSummaryApi = false;
      state.qrKnown = false;
      note.textContent = t("qrNeedsServer");
      return;
    } else {
      // Transient (server restarting, blip): leave capabilities unknown so
      // a later probe recovers instead of cementing "no summary API".
      return;
    }
  }
  if (!state.qrKnown) {
    note.textContent = t("qrNoneYet");
    return;
  }
  note.textContent = t("qrLoading");
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
      note.textContent = t("qrScanHint");
    }
  ).catch(() => {
    note.textContent = t("qrNoneYet");
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
    note.textContent = t("qrNeedsServer");
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
      } else if (r.status === 404) {
        // Old server without the summary API.
        state.hasSummaryApi = false;
        state.qrKnown = false;
      }
      // Transient failures leave both unknown: the next render retries.
    } finally {
      state.ovQrProbing = false;
    }
    if (state.qrKnown === null) {
      return;
    }
  }
  if (!state.qrKnown) {
    const wasShown = ovQrShown();
    state.ovQrLoaded = false;
    img.hidden = true;
    note.textContent = t("qrNoneYetOv");
    if (wasShown) {
      renderOvConfigs();
    }
    return;
  }
  if (state.ovQrLoaded && !img.hidden) {
    return;
  }
  note.textContent = t("qrLoading");
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
      note.textContent = t("qrScanHint");
      renderOvConfigs();
    }
  ).catch(() => {
    const wasShown = ovQrShown();
    state.ovQrLoaded = false;
    img.hidden = true;
    note.textContent = t("qrNoneYetOv");
    if (wasShown) {
      renderOvConfigs();
    }
  });
}

async function generateQr() {
  const btn = $("btn-qr-generate");
  const btnOv = $("btn-qr-generate-ov");
  btn.disabled = true;
  if (btnOv) {
    btnOv.disabled = true;
  }
  try {
    const r = await fetchJson("/api/qr/generate", { method: "POST", body: {} });
    if (r.status === 404) {
      toast(t("qrApiOld"), "bad");
      return;
    }
    if (r.status === 0) {
      toast(t("unreachable"), "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300 && r.data) {
      const skipped = Array.isArray(r.data.skipped) ? r.data.skipped : [];
      const base = r.data.message || (r.data.ok ? t("qrGenerated") : t("qrUnavailable"));
      const shown = skipped.length > 0 ? base + t("qrSkipped", { list: skipped.join("; ") }) : base;
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
    toast(t("qrFailed", { status: r.status }), "bad");
  } finally {
    btn.disabled = false;
    if (btnOv) {
      btnOv.disabled = false;
    }
  }
}

/* ---------- subscriptions tab (progressive) ---------- */

async function loadSubscriptions() {
  const note = $("sub-note");
  const r = await fetchJson("/api/subscriptions");
  if (r.status === 200 && r.data && Array.isArray(r.data.list)) {
    state.subs = { list: r.data.list, dirty: !!r.data.dirty };
    setDirty(!!r.data.dirty);
    note.textContent = t("subCount", { n: r.data.list.length });
    renderSubs();
    // Spotlight a just-added/edited row at its new rank (see submitSubDialog).
    if (state.flashSub) {
      const at = r.data.list.findIndex(
        (s) => s && s.name === state.flashSub.name && s.url === state.flashSub.url
      );
      state.flashSub = null;
      if (at >= 0) {
        flashSubRow(at);
      }
    }
    return;
  }
  state.subs = null;
  setDirty(false);
  renderSubs();
  note.textContent = r.status === 404
    ? t("subApiOld")
    : t("subLoadFailed", { status: r.status });
}

function renderSubs() {
  preserveViewport(paintSubs);
}

/// Synchronous DOM rebuild for renderSubs (viewport-preserving wrapper above).
function paintSubs() {
  const body = $("sub-body");
  while (body.firstChild) {
    body.removeChild(body.firstChild);
  }
  const list = state.subs ? state.subs.list : [];
  $("sub-empty").hidden = list.length !== 0;
  list.forEach((sub, i) => {
    const tr = document.createElement("tr");
    tr.dataset.index = String(i);
    tr.addEventListener("dragover", (ev) => subRowDragOver(ev, tr, i));
    tr.addEventListener("drop", (ev) => void subRowDrop(ev, tr, i));

    const gripTd = document.createElement("td");
    gripTd.className = "drag-cell cell-grip";
    const grip = document.createElement("button");
    grip.type = "button";
    grip.className = "drag-handle";
    grip.draggable = true;
    grip.setAttribute("aria-label", t("subDragHandle"));
    grip.title = t("subDragHandle");
    grip.textContent = "⋮⋮";
    grip.addEventListener("dragstart", (ev) => subDragStart(ev, tr, i));
    grip.addEventListener("dragend", subDragCleanup);
    gripTd.appendChild(grip);
    tr.appendChild(gripTd);

    const onTd = document.createElement("td");
    onTd.dataset.th = t("thOn");
    const tgl = document.createElement("button");
    tgl.type = "button";
    tgl.className = "btn small";
    tgl.textContent = sub.enabled ? "✅" : "❌";
    tgl.setAttribute("aria-pressed", sub.enabled ? "true" : "false");
    tgl.setAttribute("aria-label", t("toggleSub", { name: sub.name || t("subFallback", { i: i + 1 }) }));
    tgl.addEventListener("click", () => void subToggle(i));
    onTd.appendChild(tgl);
    tr.appendChild(onTd);

    tr.appendChild(cell(sub.priority !== undefined ? String(sub.priority) : "—", t("thPriority"), "cell-num"));
    tr.appendChild(el("td", sub.name || t("subFallback", { i: i + 1 }), "cell-main"));

    const urlTd = document.createElement("td");
    urlTd.dataset.th = t("thUrl");
    urlTd.appendChild(el("code", redactUrl(sub.url || "")));
    tr.appendChild(urlTd);

    const actTd = document.createElement("td");
    actTd.className = "cell-actions";
    const wrap = el("span", null, "row-actions");
    const editBtn = el("button", t("btnEdit"), "btn small");
    editBtn.type = "button";
    editBtn.addEventListener("click", () => void subEdit(i));
    const delBtn = el("button", t("btnDelete"), "btn small");
    delBtn.type = "button";
    delBtn.addEventListener("click", () => void subDelete(i));
    wrap.appendChild(editBtn);
    wrap.appendChild(delBtn);
    actTd.appendChild(wrap);
    tr.appendChild(actTd);

    body.appendChild(tr);
  });
}

/// Drag-and-drop reorder state: source row while a drag is in flight.
let subDragFrom = null;

/// Drop target in post-removal coordinates: dropping after row `i` lands at
/// `i + 1`, then removing `from` shifts everything at/after it down by one.
function dropIndex(from, i, after) {
  let to = after ? i + 1 : i;
  if (from < to) {
    to -= 1;
  }
  return to;
}

function subDragStart(ev, tr, i) {
  if (state.subReorderInflight) {
    ev.preventDefault();
    return;
  }
  subDragFrom = i;
  if (ev.dataTransfer) {
    ev.dataTransfer.effectAllowed = "move";
    try {
      // Firefox requires payload or dragstart is cancelled.
      ev.dataTransfer.setData("text/plain", String(i));
    } catch (err) {
      /* dataTransfer without setData still drags elsewhere */
    }
  }
  tr.classList.add("dragging");
}

function subRowDragOver(ev, tr, i) {
  if (subDragFrom === null || subDragFrom === undefined) {
    return;
  }
  // Allow the drop + show before/after insertion line from the pointer.
  ev.preventDefault();
  if (ev.dataTransfer) {
    ev.dataTransfer.dropEffect = "move";
  }
  const rect = tr.getBoundingClientRect ? tr.getBoundingClientRect() : null;
  const after = rect ? ev.clientY - rect.top > rect.height / 2 : false;
  tr.dataset.dropAfter = after ? "1" : "";
  const rows = $("sub-body").querySelectorAll("tr");
  for (const r of rows) {
    r.classList.remove("drop-before", "drop-after");
  }
  tr.classList.add(after ? "drop-after" : "drop-before");
}

function subDragCleanup() {
  subDragFrom = null;
  const body = $("sub-body");
  const rows = body && body.querySelectorAll ? body.querySelectorAll("tr") : [];
  for (const r of rows) {
    r.classList.remove("dragging", "drop-before", "drop-after");
    delete r.dataset.dropAfter;
  }
}

async function subRowDrop(ev, tr, i) {
  ev.preventDefault();
  const from = subDragFrom;
  const after = tr.dataset.dropAfter === "1";
  subDragCleanup();
  if (from === null || from === undefined) {
    return;
  }
  await subReorder(from, dropIndex(from, i, after));
}

async function subReorder(from, to) {
  const list = state.subs ? state.subs.list : [];
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) {
    return;
  }
  if (state.subReorderInflight) {
    return;
  }
  // Permutation of current indices; the server reorders + renumbers 1..N.
  const order = list.map((_, k) => k);
  const moved = order.splice(from, 1)[0];
  order.splice(to, 0, moved);
  state.subReorderInflight = true;
  try {
    const r = await fetchJson("/api/subscriptions/reorder", { method: "POST", body: { order } });
    if (r.status === 404) {
      toast(t("subApiOldShort"), "bad");
    } else if (r.status >= 200 && r.status < 300) {
      toast(subMessage(r, t("subReordered")), "good");
    } else {
      toast(subMessage(r, t("subReorderFailed", { status: r.status })), "bad");
    }
  } finally {
    state.subReorderInflight = false;
  }
  await loadSubscriptions();
  flashSubRow(to);
}

/// Briefly spotlight the row at `index` after an add/edit/reorder lands, so
/// the eye catches where the entry moved without hunting the table.
function flashSubRow(index) {
  const body = $("sub-body");
  const row = body && body.querySelector
    ? body.querySelector("tr[data-index=" + index + "]")
    : null;
  if (!row || !row.classList) {
    return;
  }
  row.classList.add("flash");
  if (typeof row.scrollIntoView === "function") {
    try {
      row.scrollIntoView({ block: "nearest" });
    } catch (err) {
      /* older browsers take no options */
    }
  }
  window.setTimeout(() => row.classList.remove("flash"), 1600);
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

function subMessage(r, fallback) {
  return (r.data && r.data.status) || fallback;
}

/// Next-cycle flag from settings PATCH/reset: the value is saved but the
/// loop deliberately did NOT re-fetch, so the toast must say the edit lands
/// on the next refresh — or on a manual Refresh — instead of a plain saved
/// note that would read as "already applied".
function nextCycleMessage(r, fallback) {
  if (r && r.data && r.data.code === "applies_next_cycle") {
    return t("setNextCycle", { refresh: t("btnRefresh") });
  }
  return subMessage(r, fallback);
}

/// Firewall-elevation flag from proxy/sharing mutations: the setting is
/// saved but the rule change needs admin/root, so the caller pops the guide
/// instead of the success toast. Returns the server OS (the browser may sit
/// on another LAN device, so the client must not guess from its own UA).
function firewallElevation(r) {
  if (r.data && r.data.code === "firewall_elevation") {
    return typeof r.data.os === "string" && r.data.os ? r.data.os : "other";
  }
  return null;
}

function showAdminGuide(os) {
  const key =
    os === "windows" ? "adminGuideWindows"
    : os === "linux" ? "adminGuideLinux"
    : os === "macos" ? "adminGuideMac"
    : os === "android" ? "adminGuideAndroid"
    : "adminGuideOther";
  $("dlg-admin-body").textContent = t(key) + " " + t("adminGuideRetry");
  const d = $("dlg-admin");
  if (typeof d.showModal === "function" && !d.open) {
    d.showModal();
  }
}

async function subToggle(i) {
  const r = await fetchJson("/api/subscriptions/" + i + "/toggle", { method: "POST", body: {} });
  if (r.status === 404) {
    toast(t("subApiOldShort"), "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast(subMessage(r, t("toggled")), "good");
    setDirty(false);
    void loadSubscriptions();
    return;
  }
  toast(subMessage(r, t("toggleFailed", { status: r.status })), "bad");
}

async function subDelete(i) {
  const sub = state.subs && state.subs.list[i];
  const label = (sub && sub.name) || t("subFallback", { i: i + 1 });
  if (!window.confirm(t("confirmDelete", { name: label }))) {
    return;
  }
  const r = await fetchJson("/api/subscriptions/" + i, { method: "DELETE" });
  if (r.status === 404) {
    toast(t("subApiOldShort"), "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast(subMessage(r, t("deleted")), "good");
    setDirty(false);
    void loadSubscriptions();
    return;
  }
  toast(subMessage(r, t("deleteFailed", { status: r.status })), "bad");
}

function subEdit(i) {
  const sub = state.subs && state.subs.list[i];
  if (!sub) {
    return;
  }
  openSubDialog(sub, i);
}

function openSubDialog(preset, index) {
  state.editingSub = (index === undefined || index === null) ? null : index;
  $("dlg-sub-title").textContent = preset ? t("dlgEditSub") : t("dlgAddSub");
  $("dlg-sub-ok").textContent = preset ? t("btnSave") : t("btnAdd");
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
    toast(t("subRequired"), "bad");
    return;
  }
  // URLs are unique: warn with the twin's 1-based index before anything
  // leaves the browser (an echo-save of the row being edited is fine).
  // The dialog stays open so the URL can be fixed in place.
  const editing = state.editingSub;
  const known = state.subs && Array.isArray(state.subs.list) ? state.subs.list : [];
  const twin = known.findIndex(
    (s, i) => i !== editing && s && typeof s.url === "string" && s.url.trim() === payload.url
  );
  if (twin >= 0) {
    window.alert(t("subDuplicateUrl", { n: twin + 1 }));
    return;
  }
  const r = editing === null
    ? await fetchJson("/api/subscriptions", { method: "POST", body: payload })
    : await fetchJson("/api/subscriptions/" + editing, { method: "PATCH", body: payload });
  if (r.status === 404) {
    toast(t("subApiOldShort"), "bad");
    return;
  }
  // A twin that slipped past the local check (stale list, direct API race):
  // the server names the index — pop it up the same way, dialog stays open.
  if (r.status === 409) {
    window.alert(subMessage(r, editing === null ? t("addFailed", { status: r.status }) : t("saveFailed", { status: r.status })));
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    state.editingSub = null;
    const dlg = $("dlg-sub");
    if (dlg && typeof dlg.close === "function" && dlg.open) {
      dlg.close();
    }
    toast(subMessage(r, editing === null ? t("added") : t("saved")), "good");
    setDirty(false);
    // The server resorts on priority, so the row may land elsewhere: remember
    // its new identity and spotlight it after the refetch.
    state.flashSub = { name: payload.name, url: payload.url };
    await loadSubscriptions();
    return;
  }
  toast(subMessage(r, editing === null ? t("addFailed", { status: r.status }) : t("saveFailed", { status: r.status })), "bad");
}

/* ---------- subscriptions bulk import / export (plain text) ---------- */

/// Shared bulk format, one subscription per line: `name, priority, url`.
/// The split takes the last field as the URL and the second-to-last as the
/// priority, so names may contain commas freely with no quoting rules.
/// Blank lines and `#` comment lines are skipped. Export always starts with
/// a `#` header documenting the rule, so an export pastes straight back
/// into the import box — on this or another instance.
const SUBS_BULK_HEADER = "# name, priority, url";

/// Build the export text for `list` (server order is priority order).
function subsExportText(list) {
  const lines = [SUBS_BULK_HEADER];
  for (const s of list || []) {
    if (!s || typeof s.url !== "string") {
      continue;
    }
    const name = typeof s.name === "string" ? s.name.trim() : "";
    const url = s.url.trim();
    if (!url) {
      continue;
    }
    const prio = Number(s && s.priority);
    lines.push(name + ", " + (Number.isFinite(prio) ? String(Math.max(0, Math.floor(prio))) : "0") + ", " + url);
  }
  return lines.join("\n");
}

/// Parse pasted bulk text. Returns `{ entries, badLine }` — `badLine` is
/// the 1-based line number of the first entry with an empty name/URL (or no
/// comma at all), null when every line parses. Callers fail fast on it so a
/// typo never half-imports. Three fields (`name, priority, url`) take the
/// last field as the URL and the second-to-last as the priority; two fields
/// (`name, url`) mean "append at the end". `priority` is a number, or null
/// for "append after the highest known priority" (missing/invalid).
function parseSubsImport(text) {
  const entries = [];
  const lines = String(text === undefined || text === null ? "" : text).split("\n");
  for (let li = 0; li < lines.length; li += 1) {
    const raw = lines[li].trim();
    if (!raw || raw.startsWith("#")) {
      continue;
    }
    const parts = raw.split(",");
    if (parts.length < 2) {
      return { entries, badLine: li + 1 };
    }
    const url = parts.pop().trim();
    const prioRaw = parts.length > 1 ? parts.pop().trim() : "";
    const name = parts.join(",").trim();
    if (!name || !url) {
      return { entries, badLine: li + 1 };
    }
    const prioNum = prioRaw === "" ? NaN : Number(prioRaw);
    entries.push({
      name,
      priority: prioRaw !== "" && Number.isFinite(prioNum) && prioNum >= 0 ? Math.floor(prioNum) : null,
      url,
    });
  }
  return { entries, badLine: null };
}

/// Split parsed `entries` into fresh rows vs duplicates. Duplicate = URL
/// (trimmed) already in `knownUrls` or repeated inside the paste itself —
/// the same trimmed-URL rule the server enforces with 409, checked up front
/// so the result toast can report "n/m added (x duplicates)".
function partitionSubsImport(entries, knownUrls) {
  const seen = new Set();
  for (const u of knownUrls || []) {
    if (typeof u === "string" && u.trim()) {
      seen.add(u.trim());
    }
  }
  const fresh = [];
  let duplicates = 0;
  for (const e of entries) {
    const url = e && typeof e.url === "string" ? e.url.trim() : "";
    if (!url || seen.has(url)) {
      duplicates += 1;
    } else {
      seen.add(url);
      fresh.push(e);
    }
  }
  return { fresh, duplicates };
}

function openSubsExport() {
  if (!state.subs) {
    toast(t("subApiOldShort"), "bad");
    return;
  }
  $("sub-export-text").value = subsExportText(state.subs.list);
  const dlg = $("dlg-sub-export");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

function openSubsImport() {
  if (!state.subs) {
    toast(t("subApiOldShort"), "bad");
    return;
  }
  $("sub-import-text").value = "";
  const dlg = $("dlg-sub-import");
  if (typeof dlg.showModal === "function") {
    dlg.showModal();
  }
}

/// Fill the import box from the clipboard: insert at the cursor when it
/// already holds text, otherwise replace it. Clipboard read needs a secure
/// context and may ask permission — on any failure (or plain-HTTP LAN use,
/// where the API is absent) fall back to focusing the box so the user
/// pastes with Ctrl+V instead.
async function pasteIntoImport() {
  const box = $("sub-import-text");
  let text = null;
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard && navigator.clipboard.readText) {
      text = await navigator.clipboard.readText();
    }
  } catch (err) {
    text = null;
  }
  if (typeof text !== "string") {
    toast(t("pasteManual"), "bad");
    if (box && typeof box.focus === "function") {
      box.focus();
    }
    return;
  }
  if (box) {
    try {
      if (typeof box.setRangeText === "function" && typeof box.selectionStart === "number") {
        const at = box.selectionStart;
        box.setRangeText(text, at, typeof box.selectionEnd === "number" ? box.selectionEnd : at, "end");
      } else {
        box.value = (box.value ? box.value.replace(/\s+$/, "") + "\n" : "") + text;
      }
    } catch (err) {
      box.value = text;
    }
    if (typeof box.focus === "function") {
      box.focus();
    }
  }
}

/// Import the pasted bulk text: fail fast on the first malformed line (the
/// dialog stays open so the text isn't lost), skip duplicates, POST the rest
/// one by one through the validated single-add endpoint, then report
/// "n/m added (x duplicates)" and resync.
async function submitSubsImport() {
  const parsed = parseSubsImport($("sub-import-text").value);
  if (parsed.badLine !== null) {
    toast(t("importBadLine", { l: parsed.badLine }), "bad");
    return;
  }
  const known = state.subs && Array.isArray(state.subs.list) ? state.subs.list : [];
  const part = partitionSubsImport(
    parsed.entries,
    known.map((s) => s && s.url),
  );
  let top = known.reduce((m, s) => {
    const p = Number(s && s.priority);
    return Number.isFinite(p) && p > m ? p : m;
  }, 0);
  let added = 0;
  let lastAdded = null;
  for (const e of part.fresh) {
    const priority = e.priority === null ? top + 1 : e.priority;
    const r = await fetchJson("/api/subscriptions", {
      method: "POST",
      body: { url: e.url, name: e.name, priority, enabled: true },
    });
    if (r.status === 404) {
      toast(t("subApiOldShort"), "bad");
      return;
    }
    if (r.status === 409) {
      // Lost a race with another writer: count it as a duplicate, not a
      // failure, and keep going with the rest of the batch.
      part.duplicates += 1;
      continue;
    }
    if (!(r.status >= 200 && r.status < 300)) {
      toast(subMessage(r, t("addFailed", { status: r.status })), "bad");
      return;
    }
    added += 1;
    lastAdded = e;
    if (priority > top) {
      top = priority;
    }
  }
  const dlg = $("dlg-sub-import");
  if (dlg && typeof dlg.close === "function" && dlg.open) {
    dlg.close();
  }
  toast(t("importResult", { n: added, m: parsed.entries.length, x: part.duplicates }), "good");
  setDirty(false);
  if (lastAdded) {
    state.flashSub = { name: lastAdded.name, url: lastAdded.url };
  }
  await loadSubscriptions();
}

/* ---------- settings tab (progressive) ---------- */

async function loadSettings() {
  const r = await fetchJson("/api/config");
  if (r.status === 200 && r.data && Array.isArray(r.data.groups)) {
    state.settings = r.data;
    state.settingsStatus = 200;
  } else {
    state.settings = null;
    state.settingsStatus = r.status;
  }
  renderSettings();
}

/// Settings tab: name | value | description columns with a control that fits
/// the row — on/off switch for booleans, dropdown for choices, inline editor
/// for numbers/text/lists, a setter for the secret, plain text for read-only
/// rows. Names/guides come from the locale tables (`setName_<key>`,
/// `setGuide_<key>`, `setGroup_<id>`), falling back to the server strings so
/// older servers stay readable.
const READONLY_SETTING_KEYS = [
  "subscription_count",
  "enabled_subscription_count",
  "probe.speedtest_enabled",
];

/// Control kind for one settings row: the server's `kind` wins; older
/// servers (no kinds) fall back to inference from key/value.
function settingKind(k) {
  const known = ["bool", "int", "text", "choice", "list", "secret", "readonly"];
  if (k && known.includes(k.kind)) {
    return k.kind;
  }
  if (!k) {
    return "text";
  }
  if (k.key === "sharing.token") {
    return "secret";
  }
  if (k.key === "probe.mode") {
    return "choice";
  }
  if (READONLY_SETTING_KEYS.includes(k.key)) {
    return "readonly";
  }
  if (k.value === "true" || k.value === "false") {
    return "bool";
  }
  return "text";
}

function settingName(key) {
  const k = "setName_" + String(key).split(".").join("_");
  return thas(k) ? t(k) : key;
}

function settingGuide(key, fallback) {
  const k = "setGuide_" + String(key).split(".").join("_");
  if (thas(k)) {
    return t(k);
  }
  return fallback || "";
}

function settingGroupTitle(g) {
  if (g && g.id && thas("setGroup_" + g.id)) {
    return t("setGroup_" + g.id);
  }
  return (g && g.title) || t("setFallback");
}

/// Normalize a typed setting before PATCH: trim, drop invisible bidi
/// controls, and (for numeric fields) fold non-ASCII digits to ASCII.
/// Keyboards and IMEs on every OS can emit Persian/Arabic/fullwidth digits
/// that no server parse accepts — normalize here so the field never rejects
/// what the user typed, with the server sanitizer as the backstop for pastes
/// and other clients. Written as backslash-u escapes on purpose: invisible
/// bidi control chars must never be pasted literally into source.
function normalizeSettingInput(raw, numeric) {
  let s = String(raw === undefined || raw === null ? "" : raw).trim();
  s = s.replace(/[\u2066-\u2069\u200E\u200F\u202A-\u202E\u061C\uFEFF]/g, "");
  if (numeric) {
    s = s
      .replace(/[\u06F0-\u06F9]/g, (d) => String(d.charCodeAt(0) - 0x06f0))
      .replace(/[\u0660-\u0669]/g, (d) => String(d.charCodeAt(0) - 0x0660))
      .replace(/[\uFF10-\uFF19]/g, (d) => String(d.charCodeAt(0) - 0xff10));
  }
  return s;
}

/// PATCH one setting, toast the outcome, and resync the tab + overview.
async function patchSetting(key, value) {
  const r = await fetchJson("/api/config", { method: "PATCH", body: { key, value } });
  if (r.status === 404) {
    toast(t("setApiOldShort"), "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast(nextCycleMessage(r, t("saved")), "good");
    setDirty(!!(r.data && r.data.dirty));
    await loadSettings();
    void loadOvConfig();
    return;
  }
  const msg = (r.data && (r.data.status || r.data.message)) || t("httpStatus", { status: r.status });
  toast(t("rejected", { msg }), "bad");
}

/// Reset non-subscription settings to the embedded defaults (subscriptions
/// kept): confirm first, then POST and resync the tab + overview.
async function resetSettings() {
  const r = await fetchJson("/api/config/reset", { method: "POST", body: {} });
  if (r.status === 404) {
    toast(t("resetApiOld"), "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    toast(nextCycleMessage(r, t("resetDone")), "good");
    setDirty(!!(r.data && r.data.dirty));
    await loadSettings();
    void loadOvConfig();
    return;
  }
  toast(subMessage(r, t("resetFailed", { status: r.status })), "bad");
  void loadSettings();
}

/// Paint the settings tab from the cached payload (no fetch). Skips while an
/// inline editor is open (blur auto-commits, so rebuilding the DOM could
/// PATCH a half-typed value) and when nothing ever loaded.
function renderSettings() {
  if (!state.settings && state.settingsStatus === undefined) {
    return;
  }
  if (state.settingsEditing) {
    return;
  }
  preserveViewport(paintSettings);
}

/// Synchronous DOM rebuild for renderSettings (viewport-preserving wrapper
/// above). Skipped while an inline editor is open (see renderSettings).
function paintSettings() {
  const box = $("settings-groups");
  const note = $("settings-note");
  while (box.firstChild) {
    box.removeChild(box.firstChild);
  }
  if (state.settings) {
    setDirty(!!state.settings.dirty);
    note.textContent = t("setHint");
    for (const g of state.settings.groups) {
      const card = el("section", null, "set-group");
      card.appendChild(el("h3", settingGroupTitle(g)));
      for (const k of g.keys || []) {
        card.appendChild(settingRow(k));
      }
      box.appendChild(card);
    }
    return;
  }
  note.textContent = state.settingsStatus === 404
    ? t("setApiOld")
    : t("setLoadFailed", { status: state.settingsStatus });
}

function settingRow(k) {
  const key = k.key || "";
  const name = settingName(key);
  const row = document.createElement("div");
  row.className = "set-row";
  row.appendChild(el("span", name, "set-name"));
  row.appendChild(settingControl(k, key, name));
  const guide = settingGuide(key, k.guide);
  if (guide) {
    row.appendChild(el("span", guide, "guide"));
  }
  return row;
}

function settingControl(k, key, name) {
  const kind = settingKind(k);
  const value = k.value !== undefined ? String(k.value) : "—";
  if (kind === "bool") {
    const on = value === "true";
    const sw = el("button", on ? t("setOn") : t("setOff"), "switch");
    sw.type = "button";
    sw.setAttribute("role", "switch");
    sw.setAttribute("aria-checked", on ? "true" : "false");
    sw.setAttribute("aria-label", name);
    sw.setAttribute("data-key", key);
    sw.addEventListener("click", () => void patchSetting(key, on ? "false" : "true"));
    const wrap = el("span", null, "val");
    wrap.appendChild(sw);
    return wrap;
  }
  if (kind === "choice") {
    const options = (k.options && k.options.length) ? k.options : ["active", "tcp"];
    const sel = document.createElement("select");
    sel.setAttribute("aria-label", name);
    sel.setAttribute("data-key", key);
    for (const opt of options) {
      const o = document.createElement("option");
      o.value = opt;
      o.textContent = opt;
      if (opt === value) {
        o.selected = true;
      }
      sel.appendChild(o);
    }
    sel.addEventListener("change", () => void patchSetting(key, sel.value));
    const wrap = el("span", null, "val");
    wrap.appendChild(sel);
    return wrap;
  }
  if (kind === "secret") {
    return settingSecretControl(k, key, name);
  }
  if (kind === "readonly") {
    return el("span", value, "val");
  }
  // Click-to-edit values wear the editable field look (same box as the
  // input they turn into) so the tab reads as a form, not a report.
  const val = el("span", value, "val editable");
  val.tabIndex = 0;
  val.setAttribute("role", "button");
  val.setAttribute("data-key", key);
  val.title = t("tipEditSetting");
  val.addEventListener("click", () => editSettingText(key, name, val, kind === "int"));
  val.addEventListener("keydown", (ev) => {
    // Keystrokes from the inline editor bubble up here: ignore them, or
    // Enter/Space would open a second (empty) editor on top of the commit.
    if (ev.target !== val) {
      return;
    }
    if (ev.key === "Enter" || ev.key === " ") {
      ev.preventDefault();
      editSettingText(key, name, val, kind === "int");
    }
  });
  return val;
}

function settingSecretControl(k, key, name) {
  const present = k.value !== "empty" && k.value !== "" && k.value !== "—";
  const wrap = el("span", null, "val");
  wrap.classList.add("set-secret");
  wrap.appendChild(el("span", present ? t("setPresentSet") : t("setPresentEmpty"), "set-presence"));
  if (present && key === "sharing.token") {
    const show = el("button", t("setShow"), "btn small");
    show.type = "button";
    show.addEventListener("click", () => void revealToken(key, wrap));
    wrap.appendChild(show);
  }
  const change = el("button", t("setChange"), "btn small");
  change.type = "button";
  change.setAttribute("data-key", key);
  change.addEventListener("click", () => editSecret(key, name, wrap));
  wrap.appendChild(change);
  if (present) {
    const clear = el("button", t("setClear"), "btn small");
    clear.type = "button";
    clear.addEventListener("click", () => void patchSetting(key, ""));
    wrap.appendChild(clear);
  }
  return wrap;
}

/// Explicit token reveal: the Settings table masks the secret, so fetch the
/// real value once for display + copy. Old servers 404 (no such route).
async function revealToken(key, wrap) {
  const r = await fetchJson("/api/config/token");
  if (r.status === 404) {
    toast(t("tokenApiOld"), "bad");
    return;
  }
  const token = r.status >= 200 && r.status < 300 && r.data ? String(r.data.token || "") : "";
  if (!token) {
    toast(subMessage(r, t("tokenRevealFailed", { status: r.status })), "bad");
    return;
  }
  while (wrap.firstChild) {
    wrap.removeChild(wrap.firstChild);
  }
  const code = el("code", token, "token-revealed");
  wrap.appendChild(code);
  const copy = el("button", t("btnCopy"), "btn small");
  copy.type = "button";
  copy.addEventListener("click", () => void copyText(token, t("subUrlCopied")));
  wrap.appendChild(copy);
  const hide = el("button", t("setHide"), "btn small");
  hide.type = "button";
  hide.addEventListener("click", () => renderSettings());
  wrap.appendChild(hide);
}

function editSecret(key, name, wrap) {
  state.settingsEditing = true;
  while (wrap.firstChild) {
    wrap.removeChild(wrap.firstChild);
  }
  const input = document.createElement("input");
  input.type = "password";
  input.placeholder = t("setSecretPh");
  input.setAttribute("aria-label", name);
  input.autocomplete = "off";
  wrap.appendChild(input);
  const save = el("button", t("btnSave"), "btn small");
  save.type = "button";
  save.classList.add("primary");
  const cancel = el("button", t("btnCancel"), "btn small");
  cancel.type = "button";
  wrap.appendChild(save);
  wrap.appendChild(cancel);
  input.focus();
  let done = false;
  const close = async (store) => {
    if (done) {
      return;
    }
    done = true;
    state.settingsEditing = false;
    if (!store) {
      renderSettings();
      return;
    }
    await patchSetting(key, input.value);
  };
  save.addEventListener("click", () => void close(true));
  cancel.addEventListener("click", () => void close(false));
  input.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") {
      ev.preventDefault();
      void close(true);
    } else if (ev.key === "Escape") {
      ev.preventDefault();
      void close(false);
    }
  });
}

function editSettingText(key, name, valNode, numeric) {
  state.settingsEditing = true;
  const current = valNode.textContent;
  const input = document.createElement("input");
  input.type = "text";
  if (numeric) {
    input.inputMode = "numeric";
  }
  input.value = current === "—" ? "" : current;
  input.setAttribute("aria-label", t("ariaNewValue", { key: name }));
  valNode.textContent = "";
  valNode.appendChild(input);
  input.focus();
  input.select();
  let done = false;
  const commit = async (save) => {
    if (done) {
      return;
    }
    done = true;
    state.settingsEditing = false;
    if (!save) {
      renderSettings();
      return;
    }
    await patchSetting(key, normalizeSettingInput(input.value, numeric));
  };
  input.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") {
      ev.preventDefault();
      ev.stopPropagation();
      void commit(true);
    } else if (ev.key === "Escape") {
      ev.preventDefault();
      ev.stopPropagation();
      void commit(false);
    }
  });
  input.addEventListener("blur", () => void commit(true));
}

/* ---------- actions ---------- */

/* ---------- actions ---------- */

/// Confirmed power-off: ask the server to stop the whole instance, then go
/// quiet — close the feed, stop polling, and pin the stopped screen. A 404
/// means an old server without the route; anything else failed still ends
/// quiet only when the server is actually unreachable (status 0).
async function shutdownServer() {
  const r = await fetchJson("/api/shutdown", { method: "POST", body: {} });
  if (r.status === 404) {
    toast(t("powerApiOld"), "bad");
    return;
  }
  if (r.status !== 200 && r.status !== 0) {
    toast(subMessage(r, t("trigFailed", { status: r.status })), "bad");
    return;
  }
  state.serverStopped = true;
  stopPolling();
  if (state.sse) {
    try {
      state.sse.close();
    } catch (err) {
      /* connection already gone */
    }
    state.sse = null;
  }
  setConn("offline", t("connOffline"));
  showStopped();
}

async function triggerCycle(kind) {
  // Frontend guard first: while any relevant cycle runs (system or manual)
  // the request must never leave the browser — same wording as the TUI so
  // rapid clicks, keyboard repeats, and phone double-taps collapse to one.
  if (kind === "refresh" ? refreshBusy() : pingBusy()) {
    toast(
      kind === "refresh" ? t("trigRefreshBusy") : t("trigCycleBusy"),
      "bad"
    );
    updateCycleButtons();
    return;
  }
  const path = kind === "refresh" ? "/api/refresh" : "/api/ping";
  state.cycleInflight[kind] = true;
  updateCycleButtons();
  try {
    const r = await fetchJson(path, { method: "POST", body: {} });
    if (r.status === 404) {
      toast(t("trigNeedsServer", { kind, path }), "bad");
      setStatus(t("trigUnavailable", { kind }));
      return;
    }
    if (r.status === 409) {
      // Backend confirms busy (system cycle won the race after our guard):
      // resync so buttons/countdown reflect the running cycle immediately.
      toast(
        (r.data && r.data.status) || t("trigCycleBusy"),
        "bad"
      );
      void loadResults();
      return;
    }
    if (r.status === 503) {
      toast(
        (r.data && r.data.status) || t("trigUnavailable", { kind }),
        "bad"
      );
      return;
    }
    if (r.status === 0) {
      toast(t("unreachable"), "bad");
      return;
    }
    if (r.status >= 200 && r.status < 300) {
      // Optimistic lock: mark the cycle running now so the very next click
      // (before SSE/probe-delta arrives) still refuses without a request.
      if (state.snapshot) {
        if (kind === "refresh") {
          state.snapshot.refreshing = true;
        } else {
          state.snapshot.pinging = true;
        }
        renderStats();
      }
      toast((r.data && r.data.status) || (kind === "refresh" ? t("trigRefreshDone") : t("trigPingDone")), "good");
      setStatus(t("trigWatch", { kind }));
      window.setTimeout(() => void loadResults(), 1200);
      return;
    }
    toast(t("trigFailed", { status: r.status }), "bad");
  } finally {
    state.cycleInflight[kind] = false;
    updateCycleButtons();
  }
}

async function saveNow() {
  const r = await fetchJson("/api/save", { method: "POST", body: {} });
  if (r.status === 404) {
    toast(t("saveApiOld"), "bad");
    return;
  }
  if (r.status >= 200 && r.status < 300) {
    setDirty(false);
    toast(t("saved"), "good");
    setStatus(t("saved"));
    void loadOvConfig();
    return;
  }
  toast(t("saveFailed", { status: r.status }), "bad");
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
  // Tab switches land on top on purpose: same-tab resyncs preserve the
  // viewport (see preserveViewport), so the switch itself must set it.
  if (typeof window.scrollTo === "function") {
    window.scrollTo(0, 0);
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
    void renderShare();
  }
}

function goTab(name) {
  if (!TABS.includes(name)) {
    name = "overview";
  }
  const url = "/" + name + window.location.search;
  if (window.location.pathname + window.location.search !== url) {
    if (window.history && typeof window.history.pushState === "function") {
      window.history.pushState({ tab: name }, "", url);
    } else {
      // Pre-History-API browser: full load still lands on the right tab
      // because the server serves the shell at every tab path.
      window.location.assign(url);
      return;
    }
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
    if (closeLangMenu()) {
      return;
    }
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
  // boot() re-enters via "Retry now": wiring twice would double every
  // listener and fire duplicate POSTs per click — wire exactly once.
  if (state.wired) {
    return;
  }
  state.wired = true;
  loadLanguage();
  applyI18nStatic(document);
  syncLangMenu();
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

  $("btn-power").addEventListener("click", () => {
    const d = $("dlg-power");
    if (typeof d.showModal === "function") {
      d.showModal();
    }
  });
  $("dlg-power-cancel").addEventListener("click", () => $("dlg-power").close());
  $("dlg-power-ok").addEventListener("click", () => {
    $("dlg-power").close();
    void shutdownServer();
  });
  $("btn-settings-reset").addEventListener("click", () => {
    const d = $("dlg-reset");
    if (typeof d.showModal === "function") {
      d.showModal();
    }
  });
  $("dlg-reset-cancel").addEventListener("click", () => $("dlg-reset").close());
  $("dlg-reset-ok").addEventListener("click", () => {
    $("dlg-reset").close();
    void resetSettings();
  });
  $("dlg-admin-ok").addEventListener("click", () => $("dlg-admin").close());

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
  const logLevelSel = $("log-level");
  if (logLevelSel) {
    logLevelSel.value = state.logLevel;
    logLevelSel.addEventListener("change", (ev) => {
      const v = ev.target.value;
      if (v !== "ALL" && v !== "INFO" && v !== "WARN" && v !== "ERROR") {
        ev.target.value = state.logLevel;
        return;
      }
      state.logLevel = v;
      saveLogLevel(v);
      renderLogs();
      renderOvLogs();
    });
  }
  $("log-follow").addEventListener("change", (ev) => {
    state.follow = ev.target.checked;
    if (state.follow) {
      renderLogs();
    }
  });
  $("btn-log-clear").addEventListener("click", () => {
    state.logLines = [];
    renderLogs();
    toast(t("logCleared"), "good");
  });

  $("btn-lang").addEventListener("click", () => toggleLangMenu());
  const menu = $("lang-menu");
  if (menu) {
    menu.addEventListener("click", (ev) => {
      const btn = ev.target && ev.target.closest ? ev.target.closest("button[data-lang]") : null;
      if (btn) {
        selectLang(btn.getAttribute("data-lang"));
      }
    });
  }
  // Light-dismiss like the dialogs: anything outside menu + button closes.
  document.addEventListener("click", (ev) => {
    const m = $("lang-menu");
    if (!m || m.hidden) {
      return;
    }
    const tgt = ev.target;
    if (tgt && tgt.closest && (tgt.closest("#lang-menu") || tgt.closest("#btn-lang"))) {
      return;
    }
    closeLangMenu();
  });

  $("btn-sub-add").addEventListener("click", () => {
    if (!state.subs) {
      toast(t("subApiOldShort"), "bad");
      return;
    }
    openSubDialog(null, null);
  });
  $("dlg-sub-form").addEventListener("submit", (ev) => {
    if (ev.submitter && ev.submitter.value === "ok") {
      // method="dialog" would close the dialog on every OK click — even a
      // rejected one. Keep it open until the save succeeds so a warned URL
      // can be fixed in place; Cancel still auto-closes.
      ev.preventDefault();
      void submitSubDialog();
    }
  });
  $("btn-sub-import").addEventListener("click", () => void openSubsImport());
  $("btn-sub-export").addEventListener("click", () => void openSubsExport());
  $("dlg-sub-import-form").addEventListener("submit", (ev) => {
    if (ev.submitter && ev.submitter.value === "ok") {
      // Same as the single-add dialog: a malformed pasted line must not
      // close the box and lose the text — keep it open, toast the line.
      ev.preventDefault();
      void submitSubsImport();
    }
  });
  $("dlg-sub-import-paste").addEventListener("click", () => void pasteIntoImport());
  $("dlg-sub-export-copy").addEventListener("click", () => {
    const box = $("sub-export-text");
    void copyText(box ? box.value : "", t("exportCopied"));
  });
  $("btn-save").addEventListener("click", () => void saveNow());
  $("btn-settings-reload").addEventListener("click", () => {
    state.settings = null;
    void loadSettings();
  });

  for (const mode of ["off", "local", "lan"]) {
    $("proxy-" + mode).addEventListener("click", () => void setProxyMode(mode));
  }
  $("btn-proxy-unpin").addEventListener("click", () => {
    const s = state.snapshot;
    if (s && s.proxy_active_uri && state.proxyPendingUri === undefined) {
      // Synchronous lock first: the button disables on re-render, so a
      // second click can never stack another unpin POST behind this one.
      state.proxyPendingUri = null;
      renderProxyTab();
      void selectProxy(null);
    } else if (state.proxyPendingUri !== undefined) {
      toast(t("proxyInflight"), "bad");
    } else {
      toast(t("noPin"), "bad");
    }
  });
  $("btn-sharing").addEventListener("click", async (ev) => {
    const btn = ev.currentTarget;
    btn.disabled = true;
    try {
      const r = await fetchJson("/api/sharing", { method: "POST", body: {} });
      if (r.status === 404) {
        toast(t("sharingOld"), "bad");
        return;
      }
      if (r.status >= 200 && r.status < 300) {
        const elevated = firewallElevation(r);
        if (elevated) {
          showAdminGuide(elevated);
          window.setTimeout(() => void loadResults(), 1200);
          return;
        }
        toast(subMessage(r, t("sharingToggled")), "good");
        window.setTimeout(() => void loadResults(), 1200);
        return;
      }
      toast(subMessage(r, t("sharingFailed", { status: r.status })), "bad");
    } finally {
      btn.disabled = false;
    }
  });
  $("btn-qr-generate").addEventListener("click", () => void generateQr());
  $("btn-qr-generate-ov").addEventListener("click", () => void generateQr());

  $("dlg-detail-close").addEventListener("click", () => $("dlg-detail").close());
  // Light-dismiss: a click/tap outside the dialog box (on the backdrop,
  // which targets the dialog element itself) closes it, same as Close/Esc.
  // Inner clicks target children, so form buttons keep working.
  for (const id of ["dlg-sub", "dlg-detail", "dlg-qr", "dlg-keys", "dlg-power", "dlg-admin"]) {
    const d = $(id);
    d.addEventListener("click", (ev) => {
      if (ev.target === d) {
        d.close();
      }
    });
  }
  $("dlg-detail-copy").addEventListener("click", () => void copyText(state.detailUri, t("linkCopied")));
  $("dlg-detail-use").addEventListener("click", () => {
    if (!state.detailUri) {
      toast(t("noLink"), "bad");
      return;
    }
    $("dlg-detail").close();
    void toggleProxy(state.detailUri);
  });
  $("dlg-qr-close").addEventListener("click", () => $("dlg-qr").close());

  window.addEventListener("popstate", () => showTab(currentTab()));
  // Minimized/background windows get throttled timers and may sit on a dead
  // SSE connection for hours: resync the moment the page is visible again
  // instead of waiting for the next throttled tick, so every badge jumps to
  // current truth in one repaint.
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      void loadResults();
    }
  });
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
  state.logLevel = loadLogLevel();
  state.serverStopped = false;
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
