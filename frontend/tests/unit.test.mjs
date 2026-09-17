import { describe, it, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { loadApp, makeSandbox } from "./helpers/load-app.mjs";

let api;
let sandbox;

beforeEach(() => {
  const loaded = loadApp(makeSandbox());
  api = loaded.api;
  sandbox = loaded.sandbox;
});

describe("busy guards mirror TUI trigger_refresh/trigger_ping", () => {
  it("refreshBusy is true only while refreshing or a refresh POST is in flight", () => {
    api.state.snapshot = null;
    api.state.cycleInflight = { refresh: false, ping: false };
    assert.equal(api.refreshBusy(), false);

    api.state.snapshot = { refreshing: true, pinging: false };
    assert.equal(api.refreshBusy(), true);

    api.state.snapshot = { refreshing: false, pinging: true };
    assert.equal(api.refreshBusy(), false);

    api.state.snapshot = null;
    api.state.cycleInflight.refresh = true;
    assert.equal(api.refreshBusy(), true);
  });

  it("pingBusy is true while any cycle runs or a ping POST is in flight", () => {
    api.state.snapshot = null;
    api.state.cycleInflight = { refresh: false, ping: false };
    assert.equal(api.pingBusy(), false);

    api.state.snapshot = { refreshing: true, pinging: false };
    assert.equal(api.pingBusy(), true);

    api.state.snapshot = { refreshing: false, pinging: true };
    assert.equal(api.pingBusy(), true);

    api.state.snapshot = { refreshing: false, pinging: false };
    assert.equal(api.pingBusy(), false);

    api.state.cycleInflight.ping = true;
    assert.equal(api.pingBusy(), true);
  });

  it("updateCycleButtons locks Refresh/Ping with TUI wording on every render", () => {
    api.state.snapshot = { refreshing: true, pinging: false };
    api.state.cycleInflight = { refresh: false, ping: false };
    api.updateCycleButtons();
    assert.equal(sandbox.__elements.get("btn-refresh").disabled, true);
    assert.equal(sandbox.__elements.get("btn-ping").disabled, true);
    assert.match(sandbox.__elements.get("btn-refresh").title, /already running/);
    assert.match(sandbox.__elements.get("btn-ping").title, /already running/);

    api.state.snapshot = { refreshing: false, pinging: false };
    api.updateCycleButtons();
    assert.equal(sandbox.__elements.get("btn-refresh").disabled, false);
    assert.equal(sandbox.__elements.get("btn-ping").disabled, false);
  });
});

describe("triggerCycle never sends while a cycle runs (no duplicate triggers)", () => {
  it("refuses refresh without fetch when snapshot.refreshing", async () => {
    api.state.snapshot = { refreshing: true, pinging: false };
    api.state.cycleInflight = { refresh: false, ping: false };
    let posts = 0;
    sandbox.fetch = async () => {
      posts += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    await api.triggerCycle("refresh");
    assert.equal(posts, 0);
    assert.equal(sandbox.__elements.get("btn-refresh").disabled, true);
  });

  it("refuses ping without fetch when snapshot.pinging", async () => {
    api.state.snapshot = { refreshing: false, pinging: true };
    let posts = 0;
    sandbox.fetch = async () => {
      posts += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    await api.triggerCycle("ping");
    assert.equal(posts, 0);
  });

  it("refuses ping while refreshing (TUI parity) without fetch", async () => {
    api.state.snapshot = { refreshing: true, pinging: false };
    let posts = 0;
    sandbox.fetch = async () => {
      posts += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    await api.triggerCycle("ping");
    assert.equal(posts, 0);
  });

  it("sends exactly one POST when idle and sets optimistic snapshot flags", async () => {
    api.state.snapshot = { refreshing: false, pinging: false };
    api.state.cycleInflight = { refresh: false, ping: false };
    const posts = [];
    sandbox.fetch = async (url, init) => {
      posts.push({ url: String(url), method: init && init.method });
      return { status: 200, async text() { return '{"ok":true,"status":"Manual refresh started","dirty":false}'; } };
    };
    await api.triggerCycle("refresh");
    assert.equal(posts.length, 1);
    assert.equal(posts[0].method, "POST");
    assert.match(posts[0].url, /\/api\/refresh/);
    assert.equal(api.state.snapshot.refreshing, true);
    assert.equal(api.state.cycleInflight.refresh, false);
  });

  it("concurrent triggerCycle calls collapse to one POST via cycleInflight", async () => {
    api.state.snapshot = { refreshing: false, pinging: false };
    api.state.cycleInflight = { refresh: false, ping: false };
    let posts = 0;
    let release;
    const gate = new Promise((r) => { release = r; });
    sandbox.fetch = async () => {
      posts += 1;
      await gate;
      return { status: 200, async text() { return "{}"; } };
    };
    const a = api.triggerCycle("refresh");
    const b = api.triggerCycle("refresh");
    release();
    await Promise.all([a, b]);
    assert.equal(posts, 1);
  });

  it("409 resyncs via loadResults (buttons reflect the running cycle)", async () => {
    api.state.snapshot = { refreshing: false, pinging: false };
    api.state.cycleInflight = { refresh: false, ping: false };
    const urls = [];
    sandbox.fetch = async (url) => {
      urls.push(String(url));
      if (String(url).includes("/api/refresh")) {
        return { status: 409, async text() { return '{"ok":false,"status":"Refresh already running","dirty":false}'; } };
      }
      return { status: 200, async text() { return '{"refreshing":true}'; } };
    };
    await api.triggerCycle("refresh");
    assert.ok(urls.some((u) => u.includes("/api/refresh")));
    assert.ok(urls.some((u) => u.includes("/results")));
  });
});

describe("wire() runs exactly once (no duplicate listeners / POSTs)", () => {
  it("second wire() call is a no-op", () => {
    api.wire();
    assert.equal(api.state.wired, true);
    const before = sandbox.__elements.get("btn-refresh").__listeners.get("click").length;
    api.wire();
    const after = sandbox.__elements.get("btn-refresh").__listeners.get("click").length;
    assert.equal(before, 1);
    assert.equal(after, 1);
  });
});

describe("subscription dialog distinguishes add vs PATCH", () => {
  it("openSubDialog(null) arms an add; submitSubDialog POSTs", async () => {
    api.openSubDialog(null, null);
    assert.equal(api.state.editingSub, null);
    assert.equal(sandbox.__elements.get("dlg-sub-title").textContent, "Add subscription");
    sandbox.__elements.get("dlg-sub-url").value = "https://example.com/sub.txt";
    sandbox.__elements.get("dlg-sub-name").value = "demo";
    const posts = [];
    sandbox.fetch = async (url, init) => {
      posts.push({ url: String(url), method: init && init.method });
      return { status: 200, async text() { return '{"ok":true,"status":"Added.","dirty":false}'; } };
    };
    await api.submitSubDialog();
    const mutations = posts.filter((p) => p.method === "POST");
    assert.equal(mutations.length, 1);
    assert.match(mutations[0].url, /\/api\/subscriptions$/);
    assert.equal(mutations[0].method, "POST");
  });

  it("openSubDialog(preset, i) arms an edit; submitSubDialog PATCHes /:index", async () => {
    api.state.subs = { list: [{ url: "https://a.example/s", name: "a", priority: 1, enabled: true }], dirty: false };
    api.openSubDialog(api.state.subs.list[0], 0);
    assert.equal(api.state.editingSub, 0);
    assert.equal(sandbox.__elements.get("dlg-sub-title").textContent, "Edit subscription");
    const posts = [];
    sandbox.fetch = async (url, init) => {
      posts.push({ url: String(url), method: init && init.method });
      return { status: 200, async text() { return '{"ok":true,"status":"Saved.","dirty":false}'; } };
    };
    await api.submitSubDialog();
    const mutations = posts.filter((p) => p.method === "PATCH");
    assert.equal(mutations.length, 1);
    assert.match(mutations[0].url, /\/api\/subscriptions\/0/);
    assert.equal(mutations[0].method, "PATCH");
  });

  it("submitSubDialog validates URL and name locally (no request)", async () => {
    api.openSubDialog(null, null);
    sandbox.__elements.get("dlg-sub-url").value = "";
    sandbox.__elements.get("dlg-sub-name").value = "";
    let posts = 0;
    sandbox.fetch = async () => {
      posts += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    await api.submitSubDialog();
    assert.equal(posts, 0);
  });
});

describe("subscription drag-and-drop reorder", () => {
  it("dropIndex lands after/before in post-removal coordinates", () => {
    const { api } = loadApp();
    assert.equal(api.dropIndex(0, 2, true), 2); // drag row 0 below row 2
    assert.equal(api.dropIndex(0, 2, false), 1); // drag row 0 above row 2
    assert.equal(api.dropIndex(2, 0, false), 0); // drag row 2 above row 0
    assert.equal(api.dropIndex(1, 1, false), 1); // no-op
    assert.equal(api.dropIndex(1, 2, true), 2); // drag row 1 below row 2
    assert.equal(api.dropIndex(2, 2, true), 2); // below itself = same spot
  });

  it("subReorder POSTs the permutation and resyncs", async () => {
    const { api, sandbox } = loadApp();
    const posts = [];
    sandbox.fetch = async (url, init) => {
      posts.push({ url: String(url), body: init && init.body ? JSON.parse(init.body) : null });
      if (String(url).includes("/reorder")) {
        return { status: 200, async text() { return '{"ok":true,"status":"Reordered.","dirty":false}'; } };
      }
      return { status: 200, async text() { return '{"list":[],"dirty":false}'; } };
    };
    api.state.subs = {
      list: [
        { name: "a", url: "https://example.com/a.txt", priority: 1, enabled: true },
        { name: "b", url: "https://example.com/b.txt", priority: 2, enabled: true },
        { name: "c", url: "https://example.com/c.txt", priority: 3, enabled: true },
      ],
      dirty: false,
    };
    await api.subReorder(0, 2);
    const reorder = posts.find((p) => p.url.includes("/reorder"));
    assert.deepEqual(reorder.body, { order: [1, 2, 0] });
    const toasts = sandbox.__elements.get("toasts");
    assert.match(toasts.children[toasts.children.length - 1].textContent, /Reordered/);
  });

  it("subReorder ignores no-ops and reports old servers", async () => {
    const { api, sandbox } = loadApp();
    let calls = 0;
    sandbox.fetch = async () => {
      calls += 1;
      return { status: 404, async text() { return ""; } };
    };
    api.state.subs = { list: [{ name: "a", priority: 1, enabled: true }], dirty: false };
    await api.subReorder(0, 0);
    assert.equal(calls, 0, "same-spot drop sends nothing");
    await api.subReorder(0, 5);
    assert.equal(calls, 0, "out-of-range sends nothing");
  });
});

describe("live ranked events refresh overview top-configs too", () => {
  const row = {
    rank: 1,
    name: "fresh-node",
    protocol: "vless",
    endpoint: { host: "fresh.example.com", port: 443 },
    uri: "vless://uuid@fresh.example.com:443?security=tls#fresh-node",
    reachable: true,
    latency_ms: 37,
  };

  it("applyRanked updates ov-cfg-body, not just the configs tab", () => {
    api.state.snapshot = { ranked: [], proxy_active_uri: null };
    api.applyRanked(JSON.stringify([row]));
    const body = sandbox.__elements.get("ov-cfg-body");
    assert.equal(body.children.length, 1);
    assert.equal(sandbox.__elements.get("ov-cfg-empty").hidden, true);
    assert.equal(sandbox.__elements.get("cfg-body").children.length, 1);
  });

  it("applyRanked keeps both lists in sync when results empty out", () => {
    api.state.snapshot = { ranked: [row], proxy_active_uri: null };
    api.applyRanked(JSON.stringify([]));
    assert.equal(sandbox.__elements.get("ov-cfg-body").children.length, 0);
    assert.equal(sandbox.__elements.get("ov-cfg-empty").hidden, false);
  });
});

describe("refresh badge second line: ping countdown under fetch", () => {
  it("counts down when cadences differ, hides only when they match", () => {
    api.state.hasSummaryApi = true;
    api.state.refreshSeconds = 300;

    // Own cadence + anchor: live `ping in MM:SS` under `fetch in`.
    api.state.pingSeconds = 60;
    api.state.snapshot = { refreshing: false, pinging: false, last_ping_at: new Date(Date.now() - 10_000).toISOString() };
    assert.match(api.pingSub(), /^ping in .*?\d\d:\d\d.*$/);

    // Same cadence (fetch 300s, ping 300s): fetch covers it, line hidden.
    api.state.pingSeconds = 300;
    assert.equal(api.pingSub(), "");

    // Running ping always shows, even on the refresh cadence.
    api.state.snapshot = { refreshing: false, pinging: true };
    assert.equal(api.pingSub(), "ping running…");

    // Refreshing: a fetch already probes everything (TUI hides the line).
    api.state.snapshot = { refreshing: true, pinging: false };
    assert.equal(api.pingSub(), "");

    // Disabled ping reports off (no countdown to show).
    api.state.pingSeconds = 0;
    api.state.snapshot = { refreshing: false, pinging: false };
    assert.equal(api.pingSub(), "ping off");
  });
});

describe("overview top-configs stretches while the QR is shown", () => {
  const rows = (n) =>
    Array.from({ length: n }, (_, i) => ({
      rank: i + 1,
      name: "n" + i,
      reachable: true,
      uri: "vless://" + i,
      latency_ms: 10,
      endpoint: { host: "h", port: 443 },
    }));

  it("shows 8 normally, up to 15 with the QR image on screen", () => {
    api.state.snapshot = { ranked: rows(20), proxy_active_uri: null };
    const img = sandbox.__elements.get("ov-qr-img");
    img.hidden = true;
    api.state.ovQrLoaded = false;
    api.renderOvConfigs();
    assert.equal(sandbox.__elements.get("ov-cfg-body").children.length, 8);
    img.hidden = false;
    api.state.ovQrLoaded = true;
    api.renderOvConfigs();
    assert.equal(sandbox.__elements.get("ov-cfg-body").children.length, 15);
  });

  it("caps at the available rows either way", () => {
    api.state.snapshot = { ranked: rows(5), proxy_active_uri: null };
    const img = sandbox.__elements.get("ov-qr-img");
    img.hidden = false;
    api.state.ovQrLoaded = true;
    api.renderOvConfigs();
    assert.equal(sandbox.__elements.get("ov-cfg-body").children.length, 5);
  });
});

describe("stat badges stay one-line with full stamps in tooltips", () => {
  it("fmtClock renders HH:MM:SS, fmtStamp keeps the date", () => {
    assert.match(api.fmtClock("2026-09-16T13:03:47+00:00"), /^\d\d:\d\d:\d\d$/);
    assert.equal(api.fmtClock(""), "—");
    assert.equal(api.fmtClock("bogus"), "—");
    assert.match(api.fmtStamp("2026-09-16T13:03:47+00:00"), /^2026\/09\/16 \d\d:\d\d:\d\d$/);
  });

  it("renderStats uses clock-only badge text with dated tooltips", () => {
    api.state.hasSummaryApi = true;
    api.state.refreshSeconds = 300;
    api.state.pingSeconds = 0;
    api.state.startedAt = "2026-09-16T13:03:47+00:00";
    api.state.snapshot = {
      refreshing: false,
      pinging: false,
      total_candidates: 9482,
      tested_candidates: 550,
      reachable_candidates: 41,
      fetch_bytes: 6081740,
      last_refresh: "2026-09-16T13:03:40+00:00",
      refresh_duration_ms: 33400,
      ranked: [],
      fetch_errors: [],
      proxy_running: false,
    };
    api.renderStats();
    const cards = sandbox.__elements.get("stat-cards").children;
    assert.equal(cards.length, 7);
    const text = (i) => cards[i].children.map((c) => c.textContent).join("|");
    // Running For: uptime value + short "Started: HH:MM:SS" sub, no date inline.
    assert.match(text(0), /^Running For\|.*\|Started: .*?\d\d:\d\d:\d\d.*$/);
    assert.ok(!text(0).includes("/"));
    // Last scan: clock-only value (never wraps mid-stamp), date in the title.
    assert.match(text(2), /^Last scan\|\d\d:\d\d:\d\d\|/);
    const valueNode = cards[2].children[1];
    assert.match(valueNode.title, /^2026\/09\/16 \d\d:\d\d:\d\d$/);
  });

  it("Last scan reads — while a refresh runs (TUI parity)", () => {
    api.state.hasSummaryApi = true;
    api.state.refreshSeconds = 300;
    api.state.pingSeconds = 0;
    api.state.startedAt = "2026-09-16T13:03:47+00:00";
    api.state.snapshot = {
      refreshing: true,
      pinging: false,
      total_candidates: 9482,
      tested_candidates: 550,
      reachable_candidates: 41,
      fetch_bytes: 6081740,
      last_refresh: "2026-09-16T13:03:40+00:00",
      refresh_duration_ms: 33400,
      ranked: [],
      fetch_errors: [],
      proxy_running: false,
    };
    api.renderStats();
    const cards = sandbox.__elements.get("stat-cards").children;
    const text = cards[2].children.map((c) => c.textContent).join("|");
    assert.equal(text, "Last scan|—");
  });
});

describe("fmtDuration uses whole units, never fractional minutes", () => {
  it("renders ms, seconds, minutes+seconds, hours+minutes", () => {
    // Measurements are bidi-isolated (U+2066 LRI … U+2069 PDI) so RTL
    // layout keeps "30.6 s" instead of flipping it to "s 30.6".
    const LRI = "\u2066";
    const PDI = "\u2069";
    assert.equal(api.fmtDuration(null), "—");
    assert.equal(api.fmtDuration("bogus"), "—");
    assert.equal(api.fmtDuration(850), `${LRI}850 ms${PDI}`);
    assert.equal(api.fmtDuration(30600), `${LRI}30.6 s${PDI}`);
    assert.equal(api.fmtDuration(168000), `${LRI}2m 48s${PDI}`);
    assert.equal(api.fmtDuration(59999), `${LRI}1m 0s${PDI}`);
    assert.equal(api.fmtDuration(60000), `${LRI}1m 0s${PDI}`);
    assert.equal(api.fmtDuration(3720000), `${LRI}1h 2m${PDI}`);
  });
});

describe("config detail popup (row click)", () => {
  const row = {
    rank: 1,
    name: "node-x",
    protocol: "vless",
    endpoint: { host: "x.example.com", port: 443 },
    uri: "vless://uuid@x.example.com:443#node-x",
    reachable: true,
    validation: "active_http",
    latency_ms: 11,
    http_status: 204,
    source: "e2e",
    country_code: "DE",
    error: null,
    download_mbps: null,
    stability_count: 3,
  };

  it("openDetail fills facts, stores the link, hides QR without encoder", () => {
    api.state.snapshot = { proxy_active_uri: null };
    api.openDetail(row);
    const dlg = sandbox.__elements.get("dlg-detail");
    assert.equal(dlg.open, true);
    assert.equal(api.state.detailUri, row.uri);
    assert.equal(sandbox.__elements.get("dlg-detail-title").textContent, "Config detail");
    const kvText = sandbox.__elements.get("dlg-detail-kv").children.map((c) => c.textContent).join("|");
    assert.ok(kvText.includes("Name|node-x"), "name renders inside the popup: " + kvText);
    assert.ok(kvText.includes("Reachable|yes|Stability|\u2066\u00d73\u2069"),
      "stability follows Reachable like the configs tab: " + kvText);
    // No QREncode in the sandbox: QR block hides instead of erroring.
    assert.equal(sandbox.__elements.get("dlg-detail-qr-wrap").hidden, true);
    const useBtn = sandbox.__elements.get("dlg-detail-use");
    assert.equal(useBtn.disabled, false);
    assert.equal(useBtn.textContent, "Use as proxy");
    dlg.close();
  });

  it("openDetail marks the active proxy row", () => {
    api.state.snapshot = { proxy_active_uri: row.uri };
    api.openDetail(row);
    const useBtn = sandbox.__elements.get("dlg-detail-use");
    assert.equal(useBtn.disabled, true);
    assert.equal(useBtn.textContent, "Active proxy");
    sandbox.__elements.get("dlg-detail").close();
  });

  it("backdrop clicks light-dismiss, inner clicks do not", () => {
    api.wire();
    for (const id of ["dlg-sub", "dlg-detail", "dlg-qr", "dlg-keys"]) {
      const d = sandbox.__elements.get(id);
      d.open = true;
      d.__fire("click", { target: d });
      assert.equal(d.open, false, id + " closes on backdrop click");
      d.open = true;
      d.__fire("click", { target: sandbox.document.createElement("button") });
      assert.equal(d.open, true, id + " stays open on inner click");
      d.close();
    }
  });

  it("row clicks open the popup, button clicks do not", () => {
    api.state.snapshot = { proxy_active_uri: null };
    const tr = sandbox.document.createElement("tr");
    api.wireRowDialog(tr, row);
    assert.equal(tr.tabIndex, 0);
    tr.__fire("click", { target: { closest: () => null } });
    assert.equal(sandbox.__elements.get("dlg-detail").open, true);
    sandbox.__elements.get("dlg-detail").close();
    tr.__fire("click", { target: { closest: (sel) => (sel === "button" ? {} : null) } });
    assert.equal(sandbox.__elements.get("dlg-detail").open, false);
  });
});

describe("proxy row state tracks pending → active (TUI parity)", () => {
  const URI = "vless://uuid@x.example.com:443#node-x";

  it("idle / active / pending matrix", () => {
    api.state.snapshot = { proxy_active_uri: null };
    api.state.proxyPendingUri = undefined;
    assert.equal(api.proxyRowState(URI), "idle");

    api.state.snapshot = { proxy_active_uri: URI };
    assert.equal(api.proxyRowState(URI), "active");
    assert.equal(api.proxyRowState("vless://other"), "idle");

    // Requested but not yet confirmed: pending…
    api.state.snapshot = { proxy_active_uri: null };
    api.state.proxyPendingUri = URI;
    assert.equal(api.proxyRowState(URI), "pending");

    // …and a fresh snapshot confirming it flips back to active.
    api.state.snapshot = { proxy_active_uri: URI };
    api.syncProxyPending();
    assert.equal(api.state.proxyPendingUri, undefined);
    assert.equal(api.proxyRowState(URI), "active");
  });

  it("toggleProxy unpins the confirmed-active config, refuses while in flight", async () => {
    const posts = [];
    sandbox.fetch = async (url, init) => {
      posts.push(JSON.parse(init.body));
      return { status: 200, async text() { return '{"ok":true,"status":"Proxy switch requested.","dirty":false}'; } };
    };
    // Confirmed active, no pending: toggle OFF (TUI Enter behavior).
    api.state.snapshot = { proxy_active_uri: URI, ranked: [], proxy_running: false };
    api.state.proxyPendingUri = undefined;
    await api.toggleProxy(URI);
    assert.deepEqual(posts[posts.length - 1], { uri: null });
    assert.equal(api.state.proxyPendingUri, null);

    // Unpin still in flight: further clicks are refused, never stacked.
    await api.toggleProxy(URI);
    assert.equal(posts.length, 1);

    // Server confirms the unpin (snapshot clears): pinning works again.
    api.state.snapshot = { proxy_active_uri: null, ranked: [], proxy_running: false };
    api.syncProxyPending();
    assert.equal(api.state.proxyPendingUri, undefined);
    await api.toggleProxy(URI);
    assert.deepEqual(posts[posts.length - 1], { uri: URI });
    assert.equal(posts.length, 2);
  });

  it("same-tick double toggleProxy sends exactly one POST", async () => {
    let posts = 0;
    let release;
    const gate = new Promise((r) => { release = r; });
    sandbox.fetch = async () => {
      posts += 1;
      await gate;
      return { status: 200, async text() { return '{"ok":true}'; } };
    };
    api.state.snapshot = { proxy_active_uri: null, ranked: [], proxy_running: false };
    api.state.proxyPendingUri = undefined;
    const uri = "vless://uuid@x.example.com:443#node-x";
    const a = api.toggleProxy(uri);
    const b = api.toggleProxy(uri);
    release();
    await Promise.all([a, b]);
    assert.equal(posts, 1);
  });

  it("selectProxy sets the optimistic pending lock immediately", async () => {
    sandbox.fetch = async () => (
      { status: 200, async text() { return '{"ok":true,"status":"Proxy switch requested.","dirty":false}'; } }
    );
    api.state.snapshot = { proxy_active_uri: null, ranked: [], proxy_running: false };
    api.state.proxyPendingUri = undefined;
    await api.selectProxy(URI);
    assert.equal(api.state.proxyPendingUri, URI);
    assert.equal(api.proxyRowState(URI), "pending");
  });
});

describe("live probe-delta confirms proxy switches", () => {
  const URI = "vless://uuid@x.example.com:443#node-x";
  const snap = () => ({
    refreshing: false,
    pinging: false,
    total_candidates: 0,
    tested_candidates: 0,
    reachable_candidates: 0,
    fetch_bytes: 0,
    ranked: [],
    fetch_errors: [],
    proxy_running: false,
    proxy_active_config: null,
    proxy_active_uri: null,
    proxy_port: 27910,
    proxy_discoverable: false,
  });

  it("merges proxy fields and clears the pending lock", () => {
    api.state.snapshot = snap();
    api.state.proxyPendingUri = URI;
    api.applyProbeDelta(JSON.stringify({
      proxy_running: true,
      proxy_active_uri: URI,
      proxy_active_config: "node-x",
      proxy_port: 27910,
      proxy_discoverable: false,
    }));
    assert.equal(api.state.snapshot.proxy_active_uri, URI);
    assert.equal(api.state.proxyPendingUri, undefined);
    assert.equal(api.proxyRowState(URI), "active");
  });

  it("leaves non-proxy rendering alone when proxy is unchanged", () => {
    api.state.snapshot = snap();
    api.state.proxyPendingUri = undefined;
    api.applyProbeDelta(JSON.stringify({ tested: 5, working: 1 }));
    assert.equal(api.state.snapshot.tested_candidates, 5);
    assert.equal(api.state.snapshot.proxy_active_uri, null);
  });

  it("settle clears a never-confirmed switch with a warning", async () => {
    api.state.snapshot = snap();
    api.state.proxyPendingUri = URI;
    sandbox.fetch = async () => (
      { status: 200, async text() { return JSON.stringify(api.state.snapshot); } }
    );
    await api.settleProxyPending();
    assert.equal(api.state.proxyPendingUri, undefined);
    const toasts = sandbox.__elements.get("toasts").children;
    assert.ok(toasts.some((t) => t.textContent.includes("not confirmed")));
  });
});

describe("proxy mode segmented control", () => {
  it("posts the chosen mode and locks in flight", async () => {
    const posts = [];
    let release;
    const gate = new Promise((r) => { release = r; });
    sandbox.fetch = async (url, init) => {
      posts.push(JSON.parse(init.body));
      await gate;
      return { status: 200, async text() { return '{"ok":true,"status":"Proxy local.","dirty":false}'; } };
    };
    api.state.snapshot = { proxy_running: false, proxy_discoverable: false };
    const p = api.setProxyMode("local");
    assert.equal(sandbox.__elements.get("proxy-off").disabled, true);
    assert.equal(sandbox.__elements.get("proxy-lan").disabled, true);
    release();
    await p;
    assert.deepEqual(posts, [{ mode: "local" }]);
    assert.equal(sandbox.__elements.get("proxy-off").disabled, false);
  });

  it("concurrent sets collapse to one POST", async () => {
    let posts = 0;
    let release;
    const gate = new Promise((r) => { release = r; });
    sandbox.fetch = async () => {
      posts += 1;
      await gate;
      return { status: 200, async text() { return '{"ok":true}'; } };
    };
    api.state.snapshot = { proxy_running: false, proxy_discoverable: false };
    const a = api.setProxyMode("lan");
    const b = api.setProxyMode("lan");
    release();
    await Promise.all([a, b]);
    assert.equal(posts, 1);
  });

  it("pressed follows the live snapshot (off when not running)", () => {
    const pressed = () => ["off", "local", "lan"].filter((m) =>
      sandbox.__elements.get("proxy-" + m).getAttribute("aria-pressed") === "true");
    api.state.proxyModeInflight = false;
    api.state.snapshot = { proxy_running: false, proxy_discoverable: false };
    api.updateProxyModeButtons();
    assert.deepEqual(pressed(), ["off"]);
    api.state.snapshot = { proxy_running: true, proxy_discoverable: false };
    api.updateProxyModeButtons();
    assert.deepEqual(pressed(), ["local"]);
    api.state.snapshot = { proxy_running: true, proxy_discoverable: true };
    api.updateProxyModeButtons();
    assert.deepEqual(pressed(), ["lan"]);
  });
});

describe("running-for clock pauses while offline", () => {
  it("tickClock freezes badges on offline, advances on live", () => {
    api.state.startedAt = new Date(Date.now() - 3600_000).toISOString();
    api.state.snapshot = { refreshing: false, pinging: false };
    api.state.hasSummaryApi = true;
    api.state.refreshSeconds = 0;
    api.state.pingSeconds = 0;
    const node = sandbox.document.getElementById("stat-running");

    api.state.feed = "offline";
    api.tickClock();
    assert.equal(node.textContent, "");

    api.state.feed = "live";
    api.tickClock();
    assert.match(node.textContent, /^\d\d:\d\d:\d\d$/);
  });
});

describe("proxy select: explicit null unpins (live-push only)", () => {
  it("selectProxy(null) POSTs {uri:null}", async () => {
    const bodies = [];
    sandbox.fetch = async (url, init) => {
      bodies.push({ url: String(url), body: init && init.body });
      return { status: 200, async text() { return '{"ok":true,"status":"Proxy: auto-select","dirty":false}'; } };
    };
    await api.selectProxy(null);
    assert.equal(bodies.length, 1);
    assert.match(bodies[0].url, /\/api\/proxy\/select/);
    assert.equal(JSON.parse(bodies[0].body).uri, null);
  });

  it("selectProxy('') toasts without a request", async () => {
    let posts = 0;
    sandbox.fetch = async () => {
      posts += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    await api.selectProxy("");
    assert.equal(posts, 0);
  });
});

describe("pure helpers", () => {
  it("flagFor maps ISO codes to regional indicators, rejects junk", () => {
    assert.equal(api.flagFor("de"), "\u{1F1E9}\u{1F1EA}");
    assert.equal(api.flagFor("US"), "\u{1F1FA}\u{1F1F8}");
    assert.equal(api.flagFor(null), "");
    assert.equal(api.flagFor("D"), "");
    assert.equal(api.flagFor("DEU"), "");
    assert.equal(api.flagFor("<img>"), "");
  });

  it("apiPath preserves LAN token semantics", () => {
    assert.equal(api.apiPath("/results"), "/results");
    sandbox.window.location.search = "?token=abc";
    sandbox.location.search = "?token=abc";
    assert.equal(api.apiPath("/results"), "/results?token=abc");
    assert.equal(api.apiPath("/results?x=1"), "/results?x=1&token=abc");
  });

  it("subMessage prefers server status, falls back locally", () => {
    assert.equal(api.subMessage({ data: { status: "Manual ping started" } }, "fb"), "Manual ping started");
    assert.equal(api.subMessage({ data: null }, "fb"), "fb");
  });

  it("fetchJson aborts a hung server into status 0 (fail fast)", async () => {
    const sb = makeSandbox();
    sb.setTimeout = setTimeout;
    sb.clearTimeout = clearTimeout;
    sb.AbortController = AbortController;
    sb.window.setTimeout = setTimeout;
    sb.window.clearTimeout = clearTimeout;
    sb.fetch = (url, init) =>
      new Promise((_, reject) => {
        const onAbort = () => reject(new Error("aborted"));
        if (init && init.signal) {
          if (init.signal.aborted) {
            onAbort();
          } else {
            init.signal.addEventListener("abort", onAbort);
          }
        }
      });
    const hung = loadApp(sb);
    hung.api.state.fetchTimeoutMs = 50;
    const r = await hung.api.fetchJson("/results");
    assert.equal(r.status, 0);
    assert.equal(r.data, null);
  });
});
