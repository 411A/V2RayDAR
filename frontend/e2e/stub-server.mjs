import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const FRONTEND = path.resolve(here, "..");
const TAB_PATHS = new Set(["overview", "configs", "subscriptions", "settings", "proxy", "logs", "share"]);

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
};

function ranked(i) {
  return {
    rank: i + 1,
    stability_count: 3,
    id: `vless://test${i}.example.com:443`,
    dedup_key: `vless://test${i}.example.com:443`,
    source: "e2e",
    priority: 100,
    protocol: "vless",
    name: `e2e-node-${i}`,
    endpoint: { host: "test.example.com", port: 443 + i },
    uri: `vless://uuid@test.example.com:${443 + i}?security=tls#e2e-node-${i}`,
    reachable: true,
    validation: "active_http",
    latency_ms: 42 + i,
    http_status: 204,
    download_mbps: null,
    download_bytes: null,
    error: null,
    country_code: "DE",
  };
}

function snapshot(extra = {}) {
  return {
    refreshing: false,
    pinging: false,
    total_candidates: 8,
    tested_candidates: 8,
    reachable_candidates: 2,
    fetch_bytes: 1234,
    speedtest_bytes: 0,
    ranked: [ranked(0), ranked(1)],
    logs: ["boot ok"],
    live_logs: ["boot ok"],
    last_refresh: "2026-09-16T10:00:00+00:00",
    refresh_started_at: null,
    refresh_finished_at: "2026-09-16T10:00:06+00:00",
    refresh_duration_ms: 6100,
    last_ping_at: null,
    fetch_errors: [],
    proxy_running: false,
    proxy_active_config: null,
    proxy_active_uri: null,
    proxy_port: 27910,
    proxy_discoverable: false,
    ...extra,
  };
}

/**
 * Stub origin server emulating the Rust backend for E2E.
 * Mutable `stub` handle lets specs flip busy flags and inspect POST counters.
 */
function moveSub(subs, index, rank) {
  if (index < 0 || index >= subs.length) {
    return;
  }
  const [entry] = subs.splice(index, 1);
  const slot = Math.min(Math.max(1, rank), subs.length + 1) - 1;
  subs.splice(slot, 0, entry);
  subs.forEach((s, k) => { s.priority = k + 1; });
}

export function createStub() {
  const sseClients = new Set();
  const stub = {
    refreshPosts: 0,
    pingPosts: 0,
    resultsGets: 0,
    refreshDelayMs: 0,
    pingDelayMs: 0,
    rankedPush: null,
    fetchErrors: [],
    proxyModePosts: 0,
    proxyModeBodies: [],
    proxyModeDelayMs: 0,
    proxyEnabled: false,
    proxyDiscoverable: false,
    sharingPosts: 0,
    proxySelectBodies: [],
    proxyActiveUri: null,
    applyProxyOnSelect: true,
    emit: (event, data) => {
      const payload = typeof data === "string" ? data : JSON.stringify(data);
      for (const res of [...sseClients]) {
        try {
          res.write(`event: ${event}\ndata: ${payload}\n\n`);
        } catch {
          sseClients.delete(res);
        }
      }
    },
    subAddBodies: [],
    subPatch: [],
    subToggle: [],
    subDelete: [],
    subReorder: [],
    configPatch: [],
    configReset: [],
    tokenReveals: [],
    shareUrls: [],
    savePosts: 0,
    cacheClean: [],
    shutdownPosts: 0,
    busyRefreshing: false,
    busyPinging: false,
    subs: [
      { url: "https://example.com/sub.txt", name: "demo", priority: 100, enabled: true },
    ],
    // Mutable settings store: PATCH persists like the server's
    // immediate-save, so a resync re-renders the flipped value.
    configValues: {
      bind: "127.0.0.1:27141",
      top_n: "8",
      encoded_subscription: "true",
      "probe.download_url": "off",
      "probe.speedtest_enabled": "false",
      "sharing.token": "set",
    },
  };

  const json = (res, status, obj) => {
    const body = JSON.stringify(obj);
    res.writeHead(status, { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) });
    res.end(body);
  };
  const readBody = (req) =>
    new Promise((resolve) => {
      let s = "";
      req.on("data", (c) => { s += c; });
      req.on("end", () => {
        try {
          resolve(s ? JSON.parse(s) : {});
        } catch {
          resolve({});
        }
      });
    });

  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const p = url.pathname;

    if (req.method === "GET" && (p === "/" || TAB_PATHS.has(p.slice(1)))) {
      const html = fs.readFileSync(path.join(FRONTEND, "index.html"));
      res.writeHead(200, { "Content-Type": MIME[".html"] });
      res.end(html);
      return;
    }
    if (req.method === "GET" && p === "/app.js") {
      res.writeHead(200, { "Content-Type": MIME[".js"] });
      res.end(fs.readFileSync(path.join(FRONTEND, "app.js")));
      return;
    }
    if (req.method === "GET" && p === "/style.css") {
      res.writeHead(200, { "Content-Type": MIME[".css"] });
      res.end(fs.readFileSync(path.join(FRONTEND, "style.css")));
      return;
    }
    if (req.method === "GET" && p === "/qr.js") {
      res.writeHead(200, { "Content-Type": MIME[".js"] });
      res.end(fs.readFileSync(path.join(FRONTEND, "qr.js")));
      return;
    }
    if (req.method === "GET" && p === "/i18n.js") {
      res.writeHead(200, { "Content-Type": MIME[".js"] });
      res.end(fs.readFileSync(path.join(FRONTEND, "i18n.js")));
      return;
    }
    if (req.method === "GET" && p.startsWith("/assets/")) {
      // Mirror the backend whitelist: flag SVGs + power icon + vendored
      // Vazirmatn, no traversal.
      const name = p.slice("/assets/".length);
      if (/^(GB|IR|CN|FR|RU)\.svg$/.test(name) || name === "power-off-svgrepo-com.svg") {
        // Mirror the real backend: asset bytes ride with the binary, so
        // fixed filenames are served `no-store` (see web.rs).
        res.writeHead(200, { "Content-Type": MIME[".svg"], "Cache-Control": "no-store" });
        res.end(fs.readFileSync(path.join(FRONTEND, "assets", name)));
      } else if (/^vazirmatn-(arabic|latin)\.woff2$/.test(name)) {
        res.writeHead(200, { "Content-Type": MIME[".woff2"], "Cache-Control": "no-store" });
        res.end(fs.readFileSync(path.join(FRONTEND, "assets", name)));
      } else {
        res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
        res.end("unknown asset");
      }
      return;
    }
    if (req.method === "GET" && p === "/favicon.ico") {
      res.writeHead(200, { "Content-Type": MIME[".svg"] });
      res.end("<svg xmlns='http://www.w3.org/2000/svg'></svg>");
      return;
    }
    if (req.method === "GET" && p === "/results") {
      stub.resultsGets += 1;
      json(res, 200, snapshot({ refreshing: stub.busyRefreshing, pinging: stub.busyPinging, fetch_errors: stub.fetchErrors, proxy_active_uri: stub.proxyActiveUri, proxy_running: stub.proxyEnabled, proxy_discoverable: stub.proxyDiscoverable }));
      return;
    }
    if (req.method === "GET" && p === "/api/summary") {
      const s = snapshot({ refreshing: stub.busyRefreshing, pinging: stub.busyPinging, fetch_errors: stub.fetchErrors, proxy_active_uri: stub.proxyActiveUri, proxy_running: stub.proxyEnabled, proxy_discoverable: stub.proxyDiscoverable });
      json(res, 200, { ...s, qr_available: false, refresh_seconds: 60, ping_seconds: 300, started_at: "2026-09-16T10:00:00+00:00" });
      return;
    }
    if (req.method === "GET" && p === "/api/events") {
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      });
      sseClients.add(res);
      const s = snapshot({ refreshing: stub.busyRefreshing, pinging: stub.busyPinging, fetch_errors: stub.fetchErrors, proxy_active_uri: stub.proxyActiveUri, proxy_running: stub.proxyEnabled, proxy_discoverable: stub.proxyDiscoverable });
      res.write(`event: hello\ndata: ${JSON.stringify({ snapshot: s })}\n\n`);
      if (stub.rankedPush) {
        const rows = stub.rankedPush;
        setTimeout(() => {
          try {
            res.write(`event: ranked\ndata: ${JSON.stringify(rows)}\n\n`);
          } catch {
            /* client gone */
          }
        }, 300);
      }
      const timer = setInterval(() => {
        try {
          res.write(": heartbeat\n\n");
        } catch {
          clearInterval(timer);
        }
      }, 15000);
      req.on("close", () => {
        clearInterval(timer);
        sseClients.delete(res);
      });
      return;
    }
    if (req.method === "POST" && p === "/api/refresh") {
      stub.refreshPosts += 1;
      if (stub.refreshDelayMs) {
        await new Promise((r) => setTimeout(r, stub.refreshDelayMs));
      }
      if (stub.busyRefreshing) {
        json(res, 409, { ok: false, status: "Refresh already running", dirty: false });
        return;
      }
      json(res, 200, { ok: true, status: "Manual refresh started", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/ping") {
      stub.pingPosts += 1;
      if (stub.pingDelayMs) {
        await new Promise((r) => setTimeout(r, stub.pingDelayMs));
      }
      if (stub.busyRefreshing || stub.busyPinging) {
        json(res, 409, { ok: false, status: "A cycle is already running", dirty: false });
        return;
      }
      json(res, 200, { ok: true, status: "Manual ping started", dirty: false });
      return;
    }
    if (req.method === "GET" && p === "/api/subscriptions") {
      json(res, 200, { list: stub.subs, dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/subscriptions") {
      const body = await readBody(req);
      stub.subAddBodies.push(body);
      if (!body.url || !body.name) {
        json(res, 400, { ok: false, status: "URL and name are required", dirty: false });
        return;
      }
      stub.subs.push({ url: body.url, name: body.name, priority: body.priority ?? 100, enabled: body.enabled ?? true });
      // Mirror the server: priority is the list position (insert on rank).
      moveSub(stub.subs, stub.subs.length - 1, stub.subs[stub.subs.length - 1].priority);
      json(res, 200, { ok: true, status: "Added.", dirty: false });
      return;
    }
    let m = p.match(/^\/api\/subscriptions\/(\d+)$/);
    if (m && req.method === "PATCH") {
      const body = await readBody(req);
      stub.subPatch.push({ index: Number(m[1]), body });
      Object.assign(stub.subs[Number(m[1])], body);
      // Mirror the server: an edited rank moves the row to that exact slot.
      if (body.priority !== undefined) {
        moveSub(stub.subs, Number(m[1]), body.priority);
      }
      json(res, 200, { ok: true, status: "Saved.", dirty: false });
      return;
    }
    if (m && req.method === "DELETE") {
      stub.subDelete.push(Number(m[1]));
      stub.subs.splice(Number(m[1]), 1);
      json(res, 200, { ok: true, status: "Deleted.", dirty: false });
      return;
    }
    m = p.match(/^\/api\/subscriptions\/(\d+)\/toggle$/);
    if (m && req.method === "POST") {
      await readBody(req);
      stub.subToggle.push(Number(m[1]));
      const s = stub.subs[Number(m[1])];
      s.enabled = !s.enabled;
      json(res, 200, { ok: true, status: "Toggled.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/subscriptions/reorder") {
      const body = await readBody(req);
      stub.subReorder.push(body.order);
      const next = body.order.map((i) => stub.subs[i]);
      next.forEach((s, k) => { s.priority = k + 1; });
      stub.subs.length = 0;
      stub.subs.push(...next);
      json(res, 200, { ok: true, status: "Reordered.", dirty: false });
      return;
    }
    if (req.method === "GET" && p === "/api/config") {
      const v = stub.configValues;
      json(res, 200, {
        dirty: false,
        groups: [
          {
            id: "connection",
            title: "Connection",
            keys: [
              { key: "bind", value: v.bind, guide: "Local bind address.", kind: "text", options: [] },
              { key: "top_n", value: v.top_n, guide: "Configs to publish.", kind: "int", options: [] },
              { key: "encoded_subscription", value: v.encoded_subscription, guide: "Base64 or raw list.", kind: "bool", options: [] },
              { key: "probe.download_url", value: v["probe.download_url"], guide: "Speedtest link.", kind: "text", options: [] },
              { key: "probe.speedtest_enabled", value: v["probe.speedtest_enabled"], guide: "Follows the link.", kind: "readonly", options: [] },
              { key: "sharing.token", value: v["sharing.token"], guide: "Secret.", kind: "secret", options: [] },
            ],
          },
        ],
      });
      return;
    }
    if (req.method === "PATCH" && p === "/api/config") {
      const body = await readBody(req);
      stub.configPatch.push(body);
      if (body.key === "bogus_key") {
        json(res, 400, { ok: false, status: "unknown key", dirty: false });
        return;
      }
      if (Object.prototype.hasOwnProperty.call(stub.configValues, body.key)) {
        stub.configValues[body.key] = String(body.value);
      }
      // Mirror the server's applies_next_cycle flag: refresh-relevant keys
      // (everything here but bind/sharing.token) defer to the next cycle.
      const nextCycle = body.key !== "bind" && body.key !== "sharing.token";
      json(res, 200, {
        ok: true,
        status: "Saved.",
        dirty: false,
        ...(nextCycle ? { code: "applies_next_cycle" } : {}),
      });
      return;
    }
    if (req.method === "POST" && p === "/api/config/reset") {
      await readBody(req);
      stub.configReset.push(Date.now());
      json(res, 200, { ok: true, status: "Defaults restored.", dirty: false, code: "applies_next_cycle" });
      return;
    }
    if (req.method === "GET" && p === "/api/config/token") {
      stub.tokenReveals.push(Date.now());
      json(res, 200, { token: "stub-token" });
      return;
    }
    if (req.method === "GET" && p === "/api/share-urls") {
      stub.shareUrls.push(Date.now());
      json(res, 200, {
        urls: [
          { key: "subscription", url: "http://192.0.2.2:27141/subscription?token=stub-token" },
          { key: "subscription_txt", url: "http://192.0.2.2:27141/subscription.txt?token=stub-token" },
          { key: "mihomo", url: "http://192.0.2.2:27141/mihomo.yaml?token=stub-token" },
        ],
        sharing_enabled: true,
      });
      return;
    }
    if (req.method === "POST" && p === "/api/save") {
      stub.savePosts += 1;
      json(res, 200, { ok: true, status: "Saved.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/proxy/select") {
      const body = await readBody(req);
      stub.proxySelectBodies.push(body);
      // Emulate the server applying the switch: the next snapshot carries
      // it as proxy_active_uri, confirming the client's pending state.
      // Tests can disable this to simulate a slow switch confirmed later
      // via an explicit probe-delta push.
      if (stub.applyProxyOnSelect) {
        stub.proxyActiveUri = body && body.uri !== undefined ? body.uri : stub.proxyActiveUri;
      }
      json(res, 200, { ok: true, status: "Proxy switch requested.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/proxy/mode") {
      const body = await readBody(req);
      stub.proxyModePosts += 1;
      if (stub.proxyModeDelayMs) {
        await new Promise((r) => setTimeout(r, stub.proxyModeDelayMs));
      }
      stub.proxyModeBodies.push(body);
      // Direct set when the client names a mode, legacy cycle otherwise —
      // the next snapshot reflects it, pressing the live segment.
      const mode = body && typeof body.mode === "string" ? body.mode.trim().toLowerCase() : "";
      if (mode === "off") {
        stub.proxyEnabled = false;
        stub.proxyDiscoverable = false;
      } else if (mode === "local") {
        stub.proxyEnabled = true;
        stub.proxyDiscoverable = false;
      } else if (mode === "lan") {
        stub.proxyEnabled = true;
        stub.proxyDiscoverable = true;
      } else if (mode !== "") {
        json(res, 400, { ok: false, status: `Unknown proxy mode '${body.mode}' (use off, local, or lan)`, dirty: false });
        return;
      } else if (!stub.proxyEnabled) {
        stub.proxyEnabled = true;
        stub.proxyDiscoverable = false;
      } else if (!stub.proxyDiscoverable) {
        stub.proxyDiscoverable = true;
      } else {
        stub.proxyEnabled = false;
        stub.proxyDiscoverable = false;
      }
      json(res, 200, { ok: true, status: "Proxy mode set.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/sharing") {
      stub.sharingPosts += 1;
      json(res, 200, { ok: true, status: "Sharing toggled.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/cache/clean") {
      stub.cacheClean.push(await readBody(req));
      json(res, 200, { ok: true, status: "Cache cleaned.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/qr/generate") {
      json(res, 200, { ok: true, status: "QR generated.", dirty: false });
      return;
    }
    if (req.method === "POST" && p === "/api/shutdown") {
      stub.shutdownPosts += 1;
      json(res, 200, { ok: true, status: "Server stopping", dirty: false });
      return;
    }
    if (req.method === "GET" && p === "/api/qr.jpg") {
      res.writeHead(404, { "Content-Type": "text/plain" });
      res.end("generate first");
      return;
    }
    res.writeHead(404, { "Content-Type": "text/plain" });
    res.end("not found");
  });

  return { stub, server };
}



