"use strict";

/* V2RayDAR dashboard strings — the single home of every user-facing text.
 *
 * HOW TO ADD A LANGUAGE (not yet — English only for now):
 *   1. Copy the whole `en` table to a new key (e.g. `fa`), translate values.
 *   2. Keep every key and every `{placeholder}` name identical.
 *   3. Set `i18nLang` (persisted under I18N_LANG_KEY) — `t()` reads it.
 *
 * RULES:
 * - app.js never hardcodes user text; it calls `t("key")` or `t("key", vars).
 * - index.html never hardcodes user text either; it carries `data-i18n="key"`
 *   (textContent), `data-i18n-ph` (placeholder), `data-i18n-aria` (aria-label),
 *   `data-i18n-title` (title), `data-i18n-alt` (alt), `data-i18n-content`
 *   (meta content) — applied by `applyI18nStatic()` at boot.
 * - Symbols ("—", "×", "…", units like "ms"/"KB") stay inline in app.js: they
 *   are not words and do not translate.
 * - Server-provided content (setting guides, fetch-error lines, log lines,
 *   config names) is NOT in this file — the backend owns those bytes.
 */

const I18N_LANG_KEY = "v2raydar-lang";

const I18N_STRINGS = {
  en: {
    // Shell / header
    docTitle: "V2RayDAR Dashboard",
    metaDesc: "V2RayDAR local dashboard — monitor and control the running instance from your browser.",
    skipLink: "Skip to main content",
    brandSub: "local dashboard",
    brandAria: "V2RayDAR — go to Overview",
    connAria: "Connection status",
    connActive: "connecting…",
    btnRefresh: "Refresh",
    btnRefreshTitle: "Trigger a manual refresh (same as Ctrl+R in the TUI)",
    btnRefreshBusy: "A refresh is already running — wait for it to finish",
    btnPing: "Ping",
    btnPingTitle: "Re-ping cached configs (same as Ctrl+P in the TUI)",
    btnPingBusy: "A cycle is already running — wait for it to finish",
    themeGroup: "Color theme",
    themeSystem: "System",
    themeSystemTitle: "Follow the operating system theme",
    themeLight: "Light",
    themeLightTitle: "Always use the light theme",
    themeDark: "Dark",
    themeDarkTitle: "Always use the dark theme",
    langAria: "Language: English",
    langTitle: "Change language",
    langMenuAria: "Choose language",
    langEN: "English",
    langIR: "فارسی",
    langCN: "中文",
    langFR: "Français",
    langRU: "Русский",
    langSoon: "{lang} is coming soon — English for now.",
    keysAria: "Show keyboard shortcuts",
    keysTitle: "Keyboard shortcuts (?)",
    bannerRetry: "Retry now",
    mainHeading: "V2RayDAR dashboard",
    tabsLabel: "Dashboard sections",
    tabOverview: "Overview",
    tabConfigs: "Configs",
    tabSubscriptions: "Subscriptions",
    tabSettings: "Settings",
    tabProxy: "Proxy",
    tabLogs: "Logs",
    tabShare: "Share",

    // Overview
    ovTop: "Top configs",
    thRank: "#",
    thName: "Name",
    thLatency: "Latency",
    ovTopEmpty: "No working configs yet.",
    ovOpenList: "Open full list →",
    ovEndpoints: "Endpoints",
    ovGenQr: "Generate QR Code",
    qrAlt: "QR code for the LAN subscription URL",
    qrSheetNote: "QR appears here when the server provides one.",
    ovConfiguration: "Configuration",
    ovService: "Service",
    ovNetwork: "Network",
    ovOpenSettings: "Open settings →",
    ovRecentLogs: "Recent logs",
    ovOpenLogs: "Open live logs →",
    ovFetchErrors: "Fetch errors",
    ovCfgOld: "Configuration needs a newer server — see the Settings tab.",
    ovCfgLoading: "Loading configuration…",
    ovCfgNone: "No configuration rows served — see the Settings tab.",
    ovProxyLan: " (LAN)",
    ovProxyLocal: " (local)",
    ovLogsEmpty: "No log lines yet — they appear while a cycle runs.",

    // Configs tab
    cfgTitle: "Working configs",
    cfgSearch: "Search",
    cfgSearchPh: "name, protocol, endpoint…",
    cfgReachable: "Reachable only",
    cfgRows: "Rows",
    cfgRowsAria: "Row limit",
    thProtocol: "Protocol",
    thEndpoint: "Endpoint",
    thStability: "Stability",
    thCountry: "Country",
    thActions: "Actions",
    cfgEmpty: "No working configs yet. They appear here while a refresh or ping cycle runs.",
    cfgCount: "Showing {n} of {total} ranked configs{limit}.",
    cfgLimit: " (limit {limit})",
    rowOpenTip: "Show config details, QR code and proxy actions",
    unnamed: "(unnamed)",
    useActive: "Active",
    usePending: "Pending…",
    useIdle: "Use",
    tipActiveProxy: "This config is the active proxy — click to unpin (auto-select)",
    tipPendingProxy: "Confirming the proxy switch — the server has not caught up yet",
    tipPinProxy: "Pin this config as the proxy (same as Enter in the TUI)",
    tipQrRow: "Show the QR code for this config (scan with a phone)",

    // Detail popup
    dlgDetail: "Config detail",
    fName: "Name",
    fRank: "Rank",
    fProtocol: "Protocol",
    fEndpoint: "Endpoint",
    fSource: "Source",
    fReachable: "Reachable",
    fStability: "Stability",
    fValidation: "Validation",
    fLatency: "Latency",
    fHttp: "HTTP status",
    fSpeed: "Speed",
    fCountry: "Country",
    fError: "Error",
    yes: "yes",
    no: "no",
    speedUnit: " Mbps",
    btnActiveProxy: "Active proxy",
    btnUseAsProxy: "Use as proxy",
    btnCopy: "Copy",
    btnQR: "QR",
    btnCopyLink: "Copy link",
    btnClose: "Close",
    dlgQrHint: "Scan with your phone camera.",
    dlgQrTitle: "QR code",
    qrTitlePrefix: "QR — {name}",

    // Subscriptions tab
    subTitle: "Subscriptions",
    btnSubAdd: "Add subscription",
    btnSaveNow: "Save now",
    btnSaveClean: "Save (no changes)",
    dirtyUnsaved: "unsaved changes",
    dirtySaved: "saved",
    subCount: "{n} subscription(s) on the server.",
    subApiOld: "Subscription management API is not on this server version yet — manage subscriptions from the TUI or configs.yaml.",
    subLoadFailed: "Could not load subscriptions (HTTP {status}).",
    subEmpty: "No subscription data from the server yet.",
    thOn: "On",
    thPriority: "Priority",
    thUrl: "URL",
    toggleSub: "Toggle {name}",
    subFallback: "subscription {i}",
    btnEdit: "Edit",
    btnDelete: "Delete",
    subApiOldShort: "Subscription API is not on this server version yet.",
    toggled: "Toggled.",
    toggleFailed: "Toggle failed (HTTP {status}).",
    confirmDelete: "Delete subscription \"{name}\"?",
    deleted: "Deleted.",
    deleteFailed: "Delete failed (HTTP {status}).",
    dlgEditSub: "Edit subscription",
    dlgAddSub: "Add subscription",
    subUrlLabel: "URL",
    subUrlPh: "https://example.com/sub.txt",
    subNameLabel: "Name",
    subPriorityLabel: "Priority (lower runs first)",
    subEnabledLabel: "Enabled",
    btnSave: "Save",
    btnAdd: "Add",
    btnCancel: "Cancel",
    subRequired: "URL and name are required.",
    added: "Added.",
    saved: "Saved.",
    addFailed: "Add failed (HTTP {status}).",
    saveFailed: "Save failed (HTTP {status}).",
    saveApiOld: "Save API is not on this server version yet.",

    // Settings tab
    setTitle: "Settings",
    btnSetReload: "Reload from server",
    setHint: "Click a value to edit it (same validators as the TUI). Changes stay in memory until saved.",
    setFallback: "Settings",
    setApiOld: "Settings API is not on this server version yet — edit settings from the TUI Configurations screen or configs.yaml.",
    setLoadFailed: "Could not load settings (HTTP {status}).",
    tipEditSetting: "Click to edit",
    ariaNewValue: "New value for {key}",
    setApiOldShort: "Settings API is not on this server version yet.",
    rejected: "Rejected: {msg}",

    // Proxy tab
    proxyTitle: "Persistent proxy",
    proxyModeGroup: "Proxy mode",
    proxyOff: "Off",
    proxyOffTitle: "Turn the proxy off (same as the TUI proxy row).",
    proxyLocal: "Local",
    proxyLocalTitle: "Serve the proxy on this device only — loopback, nothing exposed to the network.",
    proxyLan: "LAN",
    proxyLanTitle: "Share the proxy with your local network (binds the LAN IP, firewall rule applied).",
    btnUnpin: "Unpin manual config",
    btnUnpinTitle: "Clear the pinned config so the proxy auto-selects the best working config (same as Enter on the active row in the TUI).",
    proxyStatus: "Status",
    proxyUnknown: "unknown",
    proxyManual: "Manual config",
    proxyManualHint: "Pin one of the working configs so the proxy never rotates away from it (same as pressing Enter on a row in the TUI).",
    proxyManualHeld: "Active link held by the server (full URI never rendered — use Copy on its row to export).",
    proxyNonePinned: "none pinned",
    kvRunning: "Running",
    kvActiveConfig: "Active config",
    kvPort: "Port",
    kvLan: "LAN discoverable",
    kvPool: "Working pool",
    pillRunning: "running",
    pillRunningBare: "running (no config)",
    pillOff: "off",
    proxyInflight: "Proxy switch already in flight — try again in a moment.",
    noPin: "No manual config pinned.",
    noLink: "No link to pin for this row.",
    proxySelectOld: "Proxy-select API is not on this server version yet — use the TUI.",
    proxyModeOld: "Proxy API is not on this server version yet — use the TUI.",
    proxyRequested: "Proxy switch requested — confirming…",
    proxyRequestedShort: "Proxy switch requested.",
    proxyFailed: "Proxy switch failed (HTTP {status}).",
    proxyUnconfirmed: "Proxy switch not confirmed — the proxy may be off or the switch failed. See the Proxy tab.",
    proxyUnconfirmedShort: "Proxy switch unconfirmed — see the Proxy tab.",
    proxyModeSet: "Proxy mode set.",

    // Logs tab
    logsTitle: "Live logs",
    logFollow: "Follow",
    logFilter: "Filter",
    logFilterPh: "text…",
    btnLogClear: "Clear view",
    logsAria: "Application logs, newest at the bottom",
    logEmpty: "No log lines yet.",
    logCleared: "Log view cleared (server logs untouched).",

    // Share tab
    shareTitle: "Share on LAN",
    btnSharing: "Toggle sharing",
    shareUrls: "Subscription URLs",
    btnGenerateQr: "Generate QR",
    sharingOld: "Sharing API is not on this server version yet — use the TUI.",
    sharingToggled: "Sharing toggled.",
    sharingFailed: "Sharing toggle failed (HTTP {status}).",
    shareHintToken: "This page URL carries a token, so these links work for LAN clients too (same token model as the TUI subscription URL).",
    shareHintLocal: "On this device no token is needed (loopback bypass). LAN clients need sharing enabled — and the token appended if the server requires one.",
    epAuto: "Subscription (auto)",
    epPlain: "Subscription (plain)",
    epMihomo: "Mihomo YAML",
    epTelegram: "Telegram proxy",
    epHealth: "Health",
    epCopied: "Endpoint URL copied.",
    subUrlCopied: "Subscription URL copied.",
    linkCopied: "Config link copied.",
    qrOpenTab: "Open this tab to view the QR sheet.",
    qrNeedsServer: "The QR sheet needs a newer server — per-config QR buttons still work.",
    qrNoneYet: "No QR image yet — press Generate QR (requires LAN sharing).",
    qrNoneYetOv: "No QR image yet — press Generate QR Code (requires LAN sharing).",
    qrLoading: "Loading QR…",
    qrScanHint: "Scan with your phone to add the LAN subscription.",
    qrApiOld: "QR API is not on this server version yet.",
    qrGenerated: "QR generated.",
    qrUnavailable: "QR unavailable.",
    qrSkipped: " Skipped: {list}",
    qrFailed: "QR generation failed (HTTP {status}).",
    qrNoLink: "No link to encode for this row.",
    qrEncoderMissing: "QR encoder failed to load (qr.js missing).",
    qrTooLong: "Link is too long for a QR code.",

    // Fetch errors card
    fetchErrSub1: "{n} source failed to download (fetch problems — different from probe failures above)",
    fetchErrSubN: "{n} sources failed to download (fetch problems — different from probe failures above)",
    showMore: "Show {n} more",

    // Relative time
    justNow: "just now",
    secAgo: "{s}s ago",
    minAgo: "{m}m ago",
    hrAgo: "{h}h ago",
    dayAgo: "{d}d ago",

    // Overview stat cards
    cardRunningFor: "Running For",
    cardRefresh: "Refresh",
    cardLastScan: "Last scan",
    cardFetched: "Fetched",
    cardFailed: "Failed",
    cardWorking: "Working",
    cardSubUsage: "Sub usage",
    statusNoData: "no data",
    startedAt: "Started: {time}",
    failedOfTested: "of {tested} tested",
    tookMs: " · took {dur}",
    pingRunning: "ping running…",
    pingOff: "ping off",
    pingEvery: "ping every {s}s",
    pingIn: "ping in {countdown}",
    cycleRunning: "running",
    refreshManual: "manual",
    fetchIn: "fetch in {countdown}",

    // Connection / feed states
    connConnected: "connected",
    connLocked: "locked",
    connOffline: "offline",
    connLocal: "local",
    feedLive: "live feed",
    feedPoll2: "poll 2 s",
    feedPoll10: "poll 10 s",
    statusLive: "Live — receiving real-time updates.",
    statusPoll2: "Polling summary every 2 s.",
    statusPoll10: "Last update {ago} · polling every 10 s (no live feed on this server).",
    lastUpdateFallback: "the last update",
    lockSharingOff: "LAN sharing off",
    lockTokenRequired: "token required",
    lockTitle: "Access denied",
    lockSharingBody: "LAN sharing is disabled on this instance. Enable sharing in the TUI or open the dashboard on the device itself.",
    lockTokenBody: "This instance requires a token. Open the dashboard URL that includes ?token=… (same token as the subscription URL).",
    statusLocked: "Locked.",
    connConnecting: "connecting",
    statusConnecting: "Connecting…",
    connUnreachable: "server unreachable",
    bannerConnLost: "Connection lost",
    bannerConnLostBody: "Lost contact with the instance. Showing data from {ago}. Retrying automatically.",
    bannerUnreachable: "Server unreachable",
    bannerUnreachableBody: "Could not reach the V2RayDAR instance. Is it running?",
    statusOffline: "Offline — retrying automatically.",
    httpStatus: "HTTP {status}",
    bannerUnexpected: "Unexpected response",
    bannerUnexpectedBody: "GET /results returned HTTP {status}.",
    statusUnexpected: "Unexpected response.",
    configChanged: "Configuration changed on the server — resyncing.",
    connFeedLost: "feed lost",
    bannerFeedLost: "Live feed lost",
    bannerFeedLostBody: "Reconnecting automatically…",
    statusReconnecting: "Reconnecting…",
    unreachable: "Server unreachable.",

    // Manual triggers
    trigRefreshBusy: "Refresh already running — wait for it to finish.",
    trigCycleBusy: "A cycle is already running — try again when it finishes.",
    trigNeedsServer: "Manual {kind} needs a newer server (no {path} route yet).",
    trigUnavailable: "Manual {kind} unavailable on this server.",
    trigRefreshDone: "Refresh triggered.",
    trigPingDone: "Ping triggered.",
    trigWatch: "Manual {kind} triggered — watch Overview.",
    trigFailed: "Trigger failed (HTTP {status}).",

    // Copy helper
    copyNothing: "Nothing to copy.",
    copyManual: "Copy failed — select the text manually.",
    copyFailed: "Copy failed in this browser.",

    // Keyboard help dialog
    keysTitle: "Keyboard shortcuts",
    keysHelp: "Open this help",
    keysRefresh: "Manual refresh (same as TUI Ctrl+R)",
    keysPing: "Manual re-ping (same as TUI Ctrl+P)",
    keysSearch: "Focus search on the Configs tab",
    keysTabs: "Jump to a tab",
    keysEsc: "Close dialogs",

    // Misc
    toastsLabel: "Notifications",
    noscript: "The V2RayDAR dashboard needs JavaScript to render live state. The subscription endpoints below keep working without it:",
  },
};

let i18nLang = "en";

function i18nTable() {
  return I18N_STRINGS[i18nLang] || I18N_STRINGS.en;
}

/// Translate `key`, filling `{placeholders}` from `vars`. Unknown keys fall
/// back to the key itself (never blank) — the key-coverage test fails the
/// build long before that fallback can reach a user.
function t(key, vars) {
  const table = i18nTable();
  let s = Object.prototype.hasOwnProperty.call(table, key) ? table[key] : key;
  if (vars) {
    for (const k of Object.keys(vars)) {
      s = s.split("{" + k + "}").join(String(vars[k]));
    }
  }
  return s;
}

/// `<html lang>` mirror (null-safe: the node unit sandbox has no
/// `documentElement`, and the attribute is cosmetic there anyway).
function setDocLang(lang) {
  try {
    if (document.documentElement) {
      document.documentElement.lang = lang;
    }
  } catch (err) {
    /* ignore */
  }
}

function setLanguage(lang) {
  if (!I18N_STRINGS[lang]) {
    return false;
  }
  i18nLang = lang;
  try {
    window.localStorage.setItem(I18N_LANG_KEY, lang);
  } catch (err) {
    /* private mode etc. — language still applies for the session */
  }
  setDocLang(lang);
  applyI18nStatic(document);
  return true;
}

function loadLanguage() {
  let saved = "en";
  try {
    const v = window.localStorage.getItem(I18N_LANG_KEY);
    if (v && I18N_STRINGS[v]) {
      saved = v;
    }
  } catch (err) {
    saved = "en";
  }
  i18nLang = saved;
  setDocLang(saved);
}

/// Apply every `data-i18n*` binding in index.html (static shell text only —
/// dynamic renders call `t()` directly).
function applyI18nStatic(root) {
  if (!root || !root.querySelectorAll) {
    return;
  }
  const set = (attr, fn) => {
    const nodes = root.querySelectorAll("[" + attr + "]");
    for (let i = 0; i < nodes.length; i += 1) {
      fn(nodes[i], nodes[i].getAttribute(attr));
    }
  };
  set("data-i18n", (n, k) => {
    n.textContent = t(k);
  });
  set("data-i18n-ph", (n, k) => {
    n.setAttribute("placeholder", t(k));
  });
  set("data-i18n-aria", (n, k) => {
    n.setAttribute("aria-label", t(k));
  });
  set("data-i18n-title", (n, k) => {
    n.title = t(k);
  });
  set("data-i18n-alt", (n, k) => {
    n.setAttribute("alt", t(k));
  });
  set("data-i18n-content", (n, k) => {
    n.setAttribute("content", t(k));
  });
}
