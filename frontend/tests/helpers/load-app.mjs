import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const FRONTEND_DIR = path.resolve(here, "..", "..");
export const APP_JS = path.join(FRONTEND_DIR, "app.js");
export const I18N_JS = path.join(FRONTEND_DIR, "i18n.js");

function makeElement(tag = "div") {
  const listeners = new Map();
  const el = {
    tagName: String(tag).toUpperCase(),
    children: [],
    textContent: "",
    value: "",
    checked: false,
    disabled: false,
    hidden: false,
    title: "",
    className: "",
    style: {},
    dataset: {},
    parentNode: null,
    firstChild: null,
    open: false,
    classList: {
      _s: new Set(),
      add(...ks) {
        for (const k of ks) this._s.add(k);
      },
      remove(...ks) {
        for (const k of ks) this._s.delete(k);
      },
      toggle(k, f) {
        if (f) this._s.add(k);
        else this._s.delete(k);
      },
      contains(k) {
        return this._s.has(k);
      },
    },
    appendChild(c) {
      c.parentNode = el;
      el.children.push(c);
      el.firstChild = el.children[0] || null;
      return c;
    },
    removeChild(c) {
      const i = el.children.indexOf(c);
      if (i >= 0) el.children.splice(i, 1);
      el.firstChild = el.children[0] || null;
      if (c) c.parentNode = null;
      return c;
    },
    addEventListener(t, fn) {
      if (!listeners.has(t)) listeners.set(t, []);
      listeners.get(t).push(fn);
    },
    removeEventListener() {},
    setAttribute(k, v) {
      el.__attrs.set(String(k), String(v));
    },
    getAttribute(k) {
      return el.__attrs.has(String(k)) ? el.__attrs.get(String(k)) : null;
    },
    __attrs: new Map(),
    querySelector() {
      return null;
    },
    closest() {
      return null;
    },
    focus() {},
    select() {},
    showModal() {
      el.open = true;
    },
    close() {
      el.open = false;
    },
    __listeners: listeners,
    __fire(type, ev = {}) {
      for (const fn of listeners.get(type) || []) fn(ev);
    },
  };
  return el;
}

/** Minimal browser stub sufficient to evaluate app.js and drive pure UI logic. */
export function makeSandbox(overrides = {}) {
  const elements = new Map();
  const getOrCreate = (id) => {
    if (!elements.has(id)) elements.set(id, makeElement("div"));
    return elements.get(id);
  };
  // Pre-create every id the app wires or guards on.
  for (const id of [
    "toasts", "status-text", "status-dirty", "btn-save", "btn-refresh", "btn-ping",
    "banner", "banner-title", "banner-text", "banner-retry", "conn-dot", "conn-bind",
    "conn-state", "stat-cards", "ov-cfg-body", "ov-cfg-empty", "endpoint-list",
    "btn-qr-generate-ov", "ov-qr-img", "ov-qr-note", "ov-svc", "ov-net",
    "ov-config-note", "ov-logs", "fetch-errors-card", "fetch-errors-sub",
    "fetch-errors", "cfg-search", "cfg-reachable-only", "cfg-limit", "cfg-count",
    "cfg-body", "cfg-empty", "btn-sub-add", "sub-note", "sub-body", "sub-empty",
    "settings-note", "settings-groups", "btn-settings-reload", "proxy-off",
    "proxy-local", "proxy-lan",
    "btn-proxy-unpin", "proxy-pill", "proxy-kv", "proxy-manual", "log-filter",
    "log-follow", "btn-log-clear", "log-list", "log-empty", "btn-sharing",
    "share-list", "share-hint", "qr-img", "qr-note", "btn-qr-generate",
    "dlg-sub", "dlg-sub-form", "dlg-sub-title", "dlg-sub-url", "dlg-sub-name",
    "dlg-sub-priority", "dlg-sub-enabled", "dlg-sub-ok", "dlg-detail",
    "dlg-detail-title", "dlg-detail-kv", "dlg-detail-copy", "dlg-detail-close",
    "dlg-qr", "dlg-qr-title", "qr-canvas", "qr-hint", "dlg-qr-close",
    "dlg-keys", "dlg-keys-title", "dlg-keys-close", "btn-keys", "btn-lang",
    "theme-system", "theme-light", "theme-dark",
  ]) {
    getOrCreate(id);
  }
  // Tab/panel ids.
  for (const t of ["overview", "configs", "subscriptions", "settings", "proxy", "logs", "share"]) {
    getOrCreate("panel-" + t);
    getOrCreate("tab-" + t);
  }

  const fetchCalls = [];
  const sandbox = {
    console,
    URLSearchParams,
    document: {
      getElementById: (id) => getOrCreate(id),
      createElement: (tag) => makeElement(tag),
      querySelector: () => makeElement("div"),
      addEventListener: () => {},
      body: makeElement("body"),
      activeElement: null,
    },
    window: null,
    localStorage: (() => {
      const m = new Map();
      return {
        getItem: (k) => (m.has(k) ? m.get(k) : null),
        setItem: (k, v) => void m.set(k, String(v)),
        removeItem: (k) => void m.delete(k),
      };
    })(),
    navigator: {},
    location: { pathname: "/overview", search: "", host: "127.0.0.1:27141" },
    history: { pushState: () => {} },
    fetch: async (url) => {
      fetchCalls.push(String(url));
      return { status: 404, async text() { return ""; } };
    },
    EventSource: undefined,
    setTimeout: (fn) => 0,
    clearTimeout: () => {},
    setInterval: () => 0,
    clearInterval: () => {},
    requestAnimationFrame: () => 0,
    matchMedia: undefined,
    confirm: () => true,
    __fetchCalls: fetchCalls,
    __elements: elements,
  };
  sandbox.window = {
    location: sandbox.location,
    localStorage: sandbox.localStorage,
    history: sandbox.history,
    matchMedia: undefined,
    addEventListener: () => {},
    setTimeout: sandbox.setTimeout,
    clearTimeout: () => {},
    confirm: sandbox.confirm,
  };
  Object.assign(sandbox, overrides);
  sandbox.window.fetch = sandbox.fetch;
  return sandbox;
}

const EXPORT_HOOK = `;globalThis.__v2 = {
  state, t, applyI18nStatic, setLanguage, loadLanguage,
  refreshBusy, pingBusy, updateCycleButtons, triggerCycle, wire,
  openSubDialog, submitSubDialog, selectProxy, subToggle, subDelete, subEdit,
  flagFor, apiPath, subMessage, maskedHost, currentTab, showTab, goTab,
  saveNow, fetchJson, toast, setStatus, setDirty, renderStats, loadResults,
  applyRanked, renderOvConfigs, renderConfigs, pingSub, refreshStatus,
  renderStats, fmtClock, fmtStamp, fmtDuration, fmtAgo,
  openDetail, wireRowDialog, selectProxy, toggleProxy,
  proxyRowState, syncProxyPending, applyProbeDelta, settleProxyPending,
  setProxyMode, proxyMode, updateProxyModeButtons,
  tickClock, uptimeText,
};`;

/** Evaluate the real frontend (i18n.js first, then app.js) in a stub DOM. */
export function loadApp(sandbox = makeSandbox()) {
  const i18n = fs.readFileSync(I18N_JS, "utf8");
  const src = fs.readFileSync(APP_JS, "utf8");
  const ctx = vm.createContext(sandbox);
  vm.runInContext(i18n + src + EXPORT_HOOK, ctx, { filename: "app+i18n.js" });
  return { ctx, api: ctx.__v2, sandbox };
}
