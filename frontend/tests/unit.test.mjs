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

  it("duplicate URL on add pops up the twin index; nothing is sent", async () => {
    api.state.subs = {
      list: [
        { url: "https://a.example/s", name: "a", priority: 1, enabled: true },
        { url: "https://b.example/s", name: "b", priority: 2, enabled: true },
      ],
      dirty: false,
    };
    api.openSubDialog(null, null);
    sandbox.__elements.get("dlg-sub-url").value = "https://b.example/s";
    sandbox.__elements.get("dlg-sub-name").value = "b-again";
    const alerts = [];
    sandbox.window.alert = (msg) => alerts.push(String(msg));
    let posts = 0;
    sandbox.fetch = async () => {
      posts += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    await api.submitSubDialog();
    assert.equal(posts, 0, "twin never leaves the browser");
    assert.equal(alerts.length, 1);
    assert.match(alerts[0], /2/, "popup names the twin's 1-based index");
    assert.equal(api.state.editingSub, null, "dialog stays open for a fix");
  });

  it("editing a row onto a sibling URL warns; echo-save still PATCHes", async () => {
    api.state.subs = {
      list: [
        { url: "https://a.example/s", name: "a", priority: 1, enabled: true },
        { url: "https://b.example/s", name: "b", priority: 2, enabled: true },
      ],
      dirty: false,
    };
    // Onto the sibling: popup, no PATCH.
    api.openSubDialog(api.state.subs.list[0], 0);
    sandbox.__elements.get("dlg-sub-url").value = "https://b.example/s";
    const alerts = [];
    sandbox.window.alert = (msg) => alerts.push(String(msg));
    let patches = 0;
    sandbox.fetch = async (url, init) => {
      if (init && init.method === "PATCH") {
        patches += 1;
      }
      return { status: 200, async text() { return '{"ok":true,"list":[],"dirty":false}'; } };
    };
    await api.submitSubDialog();
    assert.equal(patches, 0);
    assert.equal(alerts.length, 1);
    assert.match(alerts[0], /2/);

    // Echo (own URL): PATCH fires.
    api.openSubDialog(api.state.subs.list[1], 1);
    sandbox.__elements.get("dlg-sub-url").value = "https://b.example/s";
    sandbox.__elements.get("dlg-sub-name").value = "b";
    await api.submitSubDialog();
    assert.equal(patches, 1, "echo-save of the same row is not a twin");
  });

  it("server 409 surfaces the twin index as a popup, dialog stays open", async () => {
    api.state.subs = { list: [], dirty: false };
    api.openSubDialog(null, null);
    sandbox.__elements.get("dlg-sub-url").value = "https://c.example/s";
    sandbox.__elements.get("dlg-sub-name").value = "c";
    const alerts = [];
    sandbox.window.alert = (msg) => alerts.push(String(msg));
    sandbox.fetch = async () => ({
      status: 409,
      async text() { return '{"ok":false,"status":"Subscription URL already exists at index 2","dirty":false}'; },
    });
    await api.submitSubDialog();
    assert.equal(alerts.length, 1);
    assert.match(alerts[0], /index 2/);
    assert.equal(api.state.editingSub, null, "dialog stays open for a fix");
  });
});

describe("preserveViewport keeps the viewport and focus across repaints", () => {
  it("restores scroll offsets across a collapsing paint", () => {
    const scrolled = [];
    sandbox.window.scrollX = 0;
    sandbox.window.scrollY = 231;
    sandbox.window.scrollTo = (x, y) => scrolled.push([x, y]);
    let painted = 0;
    const out = api.preserveViewport(() => {
      painted += 1;
      return "painted";
    });
    assert.equal(painted, 1);
    assert.equal(out, "painted", "return value passes through");
    assert.deepEqual(scrolled, [[0, 231]]);
  });

  it("refocuses the rebuilt twin by data-key when the control is destroyed", () => {
    const scrolled = [];
    sandbox.window.scrollX = 0;
    sandbox.window.scrollY = 500;
    sandbox.window.scrollTo = (x, y) => scrolled.push([x, y]);
    const dead = sandbox.document.createElement("button");
    dead.setAttribute("data-key", "top_n");
    let focused = 0;
    const twin = sandbox.document.createElement("button");
    twin.focus = () => { focused += 1; };
    let seenSelector = "";
    sandbox.document.querySelector = (sel) => {
      seenSelector = String(sel);
      return twin;
    };
    sandbox.document.activeElement = dead;
    api.preserveViewport(() => {
      // The repaint destroys the focused control: focus falls to body.
      sandbox.document.activeElement = sandbox.document.body;
    });
    assert.equal(seenSelector, '[data-key="top_n"]');
    assert.equal(focused, 1, "rebuilt twin regains focus");
    assert.deepEqual(scrolled, [[0, 500]], "scroll still restored");
  });

  it("keeps focus alone when nothing is destroyed; survives a missing twin", () => {
    sandbox.window.scrollTo = () => {};
    const kept = sandbox.document.createElement("button");
    sandbox.document.activeElement = kept;
    let queried = 0;
    sandbox.document.querySelector = () => {
      queried += 1;
      return null;
    };
    api.preserveViewport(() => {});
    assert.equal(queried, 0, "no twin lookup when focus survives");
    sandbox.document.activeElement = sandbox.document.body;
    const stray = sandbox.document.createElement("button");
    stray.setAttribute("data-key", "bind");
    sandbox.document.activeElement = stray;
    api.preserveViewport(() => {
      sandbox.document.activeElement = sandbox.document.body;
    });
    assert.equal(queried, 1, "lookup runs on focus loss");
  });

  it("async paints restore once settled", async () => {
    const scrolled = [];
    sandbox.window.scrollX = 0;
    sandbox.window.scrollY = 77;
    sandbox.window.scrollTo = (x, y) => scrolled.push([x, y]);
    const out = await api.preserveViewport(async () => "async-painted");
    assert.equal(out, "async-painted");
    assert.deepEqual(scrolled, [[0, 77]]);
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

describe("subscriptions bulk import/export text format", () => {
  it("subsExportText writes name, priority, url with a format header", () => {
    const { api } = loadApp();
    assert.equal(
      api.subsExportText([
        { name: "demo", url: "https://example.com/sub.txt", priority: 100, enabled: true },
        { name: "Second, with comma", url: "https://example.com/s.txt", priority: 3, enabled: false },
      ]),
      "# name, priority, url\ndemo, 100, https://example.com/sub.txt\nSecond, with comma, 3, https://example.com/s.txt",
    );
    assert.equal(api.subsExportText([]), "# name, priority, url");
  });

  it("parseSubsImport splits on the last two commas; skips blanks and # lines", () => {
    const { api } = loadApp();
    const parsed = api.parseSubsImport(
      "# exported backup\n\ndemo, 100, https://example.com/sub.txt\nSecond, with comma, 3, https://example.com/s.txt\nplain, https://example.com/p.txt\n",
    );
    assert.equal(parsed.badLine, null);
    // Entries are built inside the vm sandbox (different Object prototype),
    // so compare through JSON rather than deepStrictEqual.
    assert.equal(
      JSON.stringify(parsed.entries),
      JSON.stringify([
        { name: "demo", priority: 100, url: "https://example.com/sub.txt" },
        { name: "Second, with comma", priority: 3, url: "https://example.com/s.txt" },
        { name: "plain", priority: null, url: "https://example.com/p.txt" },
      ]),
    );
  });

  it("parseSubsImport fails fast on the first malformed line", () => {
    const { api } = loadApp();
    assert.equal(api.parseSubsImport("ok, 1, https://example.com/a.txt\nno-commas-here\nx, 2, https://example.com/b.txt").badLine, 2);
    assert.equal(api.parseSubsImport(", 5, https://example.com/a.txt").badLine, 1, "empty name");
    assert.equal(api.parseSubsImport("nameless, 5, ").badLine, 1, "empty url");
  });

  it("partitionSubsImport dedupes against the server list and inside the paste", () => {
    const { api } = loadApp();
    const part = api.partitionSubsImport(
      [
        { name: "fresh", priority: 1, url: "https://example.com/fresh.txt" },
        { name: "twin", priority: 2, url: " https://example.com/demo.txt " },
        { name: "repeat", priority: 3, url: "https://example.com/fresh.txt" },
      ],
      ["https://example.com/demo.txt"],
    );
    assert.equal(part.duplicates, 2);
    assert.equal(
      JSON.stringify(part.fresh),
      JSON.stringify([{ name: "fresh", priority: 1, url: "https://example.com/fresh.txt" }]),
    );
  });

  it("submitSubsImport POSTs fresh rows and reports n/m added (x duplicates)", async () => {
    const { api, sandbox } = loadApp();
    const posts = [];
    sandbox.fetch = async (url, init) => {
      posts.push({ url: String(url), method: init && init.method, body: init && init.body ? JSON.parse(init.body) : null });
      if (init && init.method === "POST" && String(url).endsWith("/api/subscriptions")) {
        return { status: 200, async text() { return '{"ok":true,"status":"Added.","dirty":false}'; } };
      }
      return { status: 200, async text() { return '{"list":[],"dirty":false}'; } };
    };
    api.state.subs = {
      list: [{ name: "demo", url: "https://example.com/sub.txt", priority: 100, enabled: true }],
      dirty: false,
    };
    sandbox.__elements.get("sub-import-text").value =
      "fresh one, 5, https://example.com/fresh.txt\n" +
      "twin, 6, https://example.com/sub.txt\n" +
      "appended, https://example.com/appended.txt";
    await api.submitSubsImport();
    const adds = posts.filter((p) => p.method === "POST" && p.url.endsWith("/api/subscriptions"));
    assert.equal(adds.length, 2);
    assert.deepEqual(adds[0].body, { url: "https://example.com/fresh.txt", name: "fresh one", priority: 5, enabled: true });
    assert.deepEqual(adds[1].body, { url: "https://example.com/appended.txt", name: "appended", priority: 101, enabled: true });
    const toasts = sandbox.__elements.get("toasts");
    // Interpolated counts ride FSI...PDI isolates, so match loosely around them.
    assert.match(toasts.children[toasts.children.length - 1].textContent, /2[\s\S]*\/[\s\S]*3[\s\S]*subscriptions added \([\s\S]*1[\s\S]*duplicates\)/);
  });

  it("submitSubsImport keeps the dialog open on a malformed line", async () => {
    const { api, sandbox } = loadApp();
    let calls = 0;
    sandbox.fetch = async () => {
      calls += 1;
      return { status: 200, async text() { return "{}"; } };
    };
    api.state.subs = { list: [], dirty: false };
    sandbox.__elements.get("sub-import-text").value = "ok, 1, https://example.com/a.txt\nbroken line";
    await api.submitSubsImport();
    assert.equal(calls, 0, "fail fast sends nothing");
    const toasts = sandbox.__elements.get("toasts");
    assert.match(toasts.children[toasts.children.length - 1].textContent, /Line[\s\S]*2[\s\S]*not a valid/);
  });

  it("pasteIntoImport falls back to manual paste without a clipboard API", async () => {
    const { api, sandbox } = loadApp();
    await api.pasteIntoImport();
    const toasts = sandbox.__elements.get("toasts");
    assert.match(toasts.children[toasts.children.length - 1].textContent, /Ctrl\+V/);
    assert.equal(sandbox.__elements.get("sub-import-text").value, "");
  });

  it("pasteIntoImport appends clipboard text after existing lines", async () => {
    const sb = makeSandbox();
    sb.navigator = { clipboard: { readText: async () => "pasted, 2, https://example.com/p.txt" } };
    const { api, sandbox } = loadApp(sb);
    sandbox.__elements.get("sub-import-text").value = "existing, 9, https://example.com/e.txt\n";
    await api.pasteIntoImport();
    assert.equal(
      sandbox.__elements.get("sub-import-text").value,
      "existing, 9, https://example.com/e.txt\npasted, 2, https://example.com/p.txt",
    );
  });

  it("pasteIntoImport replaces an empty box and survives a denied read", async () => {
    const sb = makeSandbox();
    sb.navigator = { clipboard: { readText: async () => { throw new Error("denied"); } } };
    const denied = loadApp(sb);
    await denied.api.pasteIntoImport();
    const toasts = denied.sandbox.__elements.get("toasts");
    assert.match(toasts.children[toasts.children.length - 1].textContent, /Ctrl\+V/);
    const sb2 = makeSandbox();
    sb2.navigator = { clipboard: { readText: async () => "fresh, 1, https://example.com/f.txt" } };
    const { api, sandbox } = loadApp(sb2);
    await api.pasteIntoImport();
    assert.equal(sandbox.__elements.get("sub-import-text").value, "fresh, 1, https://example.com/f.txt");
  });
});

describe("power-off stops quietly and stays stopped", () => {
  it("shutdownServer pins the stopped screen on 200", async () => {
    const { api, sandbox } = loadApp();
    sandbox.fetch = async () => ({ status: 200, async text() { return '{"ok":true,"status":"Server stopping"}'; } });
    await api.shutdownServer();
    assert.equal(api.state.serverStopped, true);
    assert.equal(api.state.sse, null);
    assert.equal(sandbox.__elements.get("banner-title").textContent, "Server stopped");
    assert.equal(sandbox.__elements.get("status-text").textContent, "Stopped.");
  });

  it("shutdownServer refuses old servers and keeps running", async () => {
    const { api, sandbox } = loadApp();
    sandbox.fetch = async () => ({ status: 404, async text() { return ""; } });
    await api.shutdownServer();
    assert.equal(api.state.serverStopped, false);
    const toasts = sandbox.__elements.get("toasts");
    assert.match(toasts.children[toasts.children.length - 1].textContent, /Shutdown API/);
  });

  it("loadResults keeps the stopped screen instead of offline", async () => {
    const { api, sandbox } = loadApp();
    sandbox.fetch = async () => ({ status: 0, data: null });
    api.state.serverStopped = true;
    assert.equal(await api.loadResults(), false);
    assert.equal(sandbox.__elements.get("banner-title").textContent, "Server stopped");
  });
});

describe("firewall elevation pops the run-as-admin guide", () => {
  const elevation = (os) => ({
    status: 200,
    async text() {
      return JSON.stringify({
        ok: true,
        status: "Sharing on (firewall update failed: boom)",
        dirty: false,
        code: "firewall_elevation",
        os,
      });
    },
  });

  it("firewallElevation reads code+os, ignores plain replies", () => {
    const { api } = loadApp();
    assert.equal(
      api.firewallElevation({ status: 200, data: { ok: true, code: "firewall_elevation", os: "windows" } }),
      "windows",
    );
    assert.equal(
      api.firewallElevation({ status: 200, data: { ok: true, code: "firewall_elevation" } }),
      "other",
    );
    assert.equal(
      api.firewallElevation({ status: 200, data: { ok: true, status: "Sharing on" } }),
      null,
    );
    assert.equal(api.firewallElevation({ status: 500, data: null }), null);
  });

  it("showAdminGuide renders per-OS text in the user's language", () => {
    const { api, sandbox } = loadApp();
    api.showAdminGuide("windows");
    assert.equal(sandbox.__elements.get("dlg-admin").open, true);
    assert.match(sandbox.__elements.get("dlg-admin-body").textContent, /Run as administrator/);
    api.showAdminGuide("linux");
    assert.match(sandbox.__elements.get("dlg-admin-body").textContent, /sudo/);
    api.showAdminGuide("android");
    assert.match(sandbox.__elements.get("dlg-admin-body").textContent, /Termux/);
    api.setLanguage("fa");
    api.showAdminGuide("windows");
    // The Windows menu label stays in English: that is what the OS shows.
    assert.match(sandbox.__elements.get("dlg-admin-body").textContent, /Run as administrator/);
  });

  it("proxy mode LAN failure opens the guide instead of the success toast", async () => {
    const { api, sandbox } = loadApp();
    sandbox.fetch = async () => elevation("windows");
    await api.setProxyMode("lan");
    assert.equal(sandbox.__elements.get("dlg-admin").open, true);
    assert.match(sandbox.__elements.get("dlg-admin-body").textContent, /Run as administrator/);
    assert.equal(sandbox.__elements.get("toasts").children.length, 0, "no success toast on elevation");
  });

  it("plain proxy replies keep the toast path", async () => {
    const { api, sandbox } = loadApp();
    sandbox.fetch = async () => ({
      status: 200,
      async text() { return '{"ok":true,"status":"Proxy LAN (rule ok)","dirty":false}'; },
    });
    await api.setProxyMode("lan");
    assert.equal(sandbox.__elements.get("dlg-admin").open, false);
    const toasts = sandbox.__elements.get("toasts");
    assert.match(toasts.children[toasts.children.length - 1].textContent, /Proxy LAN/);
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

  it("Last scan stacks age above duration on two sub-lines", () => {
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
    const scan = sandbox.__elements.get("stat-cards").children[2];
    const subs = scan.children.filter((c) => c.className === "stat-sub");
    assert.equal(subs.length, 2);
    assert.equal(subs[0].id, "stat-scan-age");
    assert.equal(subs[1].id, "stat-scan-took");
    assert.match(subs[1].textContent, /took/);
  });

  it("tickClock refreshes the scan age every second without a render", () => {
    api.state.hasSummaryApi = true;
    api.state.refreshSeconds = 300;
    api.state.pingSeconds = 0;
    api.state.startedAt = new Date(Date.now() - 3_600_000).toISOString();
    api.state.feed = "live";
    api.state.snapshot = {
      refreshing: false,
      pinging: false,
      total_candidates: 9482,
      tested_candidates: 550,
      reachable_candidates: 41,
      fetch_bytes: 6081740,
      last_refresh: new Date(Date.now() - 90_000).toISOString(),
      refresh_duration_ms: 33400,
      ranked: [],
      fetch_errors: [],
      proxy_running: false,
    };
    api.renderStats();
    // Id lookup resolves to the rendered badge (stub registry stands in
    // for the browser's id index); the tick rewrites that node in place.
    const age = sandbox.document.getElementById("stat-scan-age");
    // The stamp moves forward; the next tick rewrites the age in place.
    api.state.snapshot.last_refresh = new Date(Date.now() - 20_000).toISOString();
    api.tickClock();
    assert.match(age.textContent, /\d+.+s ago/);
  });

  it("Fetched badge carries the phone-hide id (CSS drops it on portrait phones)", () => {
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
    assert.equal(cards[3].id, "stat-fetched");
    assert.match(cards[3].children[0].textContent, /Fetched/);
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
    // The popup shows the exact milliseconds even past one second.
    api.openDetail({ ...row, latency_ms: 2100 });
    const kvMs = sandbox.__elements.get("dlg-detail-kv").children.map((c) => c.textContent).join("|");
    assert.ok(kvMs.includes("Latency|\u20662100 ms\u2069"), "popup latency stays in ms: " + kvMs);
    // No QREncode in the sandbox: QR block hides instead of erroring.
    assert.equal(sandbox.__elements.get("dlg-detail-qr-wrap").hidden, true);
    const useBtn = sandbox.__elements.get("dlg-detail-use");
    assert.equal(useBtn.disabled, false);
    assert.equal(useBtn.textContent, "Use as proxy");
    dlg.close();
  });

  it("cell() labels every value for the portrait-phone card layout", () => {
    const td = api.cell("vless", "Protocol");
    assert.equal(td.tagName, "TD");
    assert.equal(td.dataset.th, "Protocol");
    assert.equal(td.children.length, 1);
    assert.equal(td.children[0].tagName, "SPAN");
    assert.equal(td.children[0].className, "cell-text");
    assert.equal(td.children[0].textContent, "vless");
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

  it("merges the fetched total so the badge updates without a reload", () => {
    api.state.snapshot = snap();
    api.state.proxyPendingUri = undefined;
    api.applyProbeDelta(JSON.stringify({ tested: 8, working: 2, total: 42 }));
    assert.equal(api.state.snapshot.total_candidates, 42);
    const fetched = sandbox.__elements.get("stat-cards").children[3];
    assert.equal(fetched.id, "stat-fetched");
    assert.equal(fetched.children[1].textContent, "42");
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

describe("country badge mirrors the server display-name rule", () => {
  // countryFlag objects are built inside the vm sandbox (different Object
  // prototype), so compare through JSON rather than deepStrictEqual.
  const flag = (api, name, cc) => JSON.stringify(api.countryFlag(name, cc));
  it("the name's own flag wins over a disagreeing GeoIP code", () => {
    const { api } = loadApp();
    assert.equal(flag(api, "🇳🇱 | @WhiteDNS", "US"), JSON.stringify({ flag: "🇳🇱", code: "NL" }));
    assert.equal(flag(api, "Server 🇫🇷 fast", "DE"), JSON.stringify({ flag: "🇫🇷", code: "FR" }));
  });

  it("the GeoIP code fills in only when the name carries no flag", () => {
    const { api } = loadApp();
    assert.equal(flag(api, "@ProxyChannel", "us"), JSON.stringify({ flag: "🇺🇸", code: "US" }));
    assert.equal(flag(api, "plain", null), JSON.stringify({ flag: "", code: "" }));
    assert.equal(flag(api, "plain", "USA"), JSON.stringify({ flag: "", code: "" }));
    assert.equal(flag(api, "plain", ""), JSON.stringify({ flag: "", code: "" }));
  });

  it("a bare two-letter name flags itself without dropping its text", () => {
    const { api } = loadApp();
    // Provider named the node "NL": the badge and the display name both
    // become flag + original text — never the flag alone.
    assert.equal(flag(api, "NL", null), JSON.stringify({ flag: "🇳🇱", code: "NL" }));
    assert.equal(flag(api, "GB", "DE"), JSON.stringify({ flag: "🇬🇧", code: "GB" }));
    assert.equal(api.displayName("NL"), "🇳🇱 NL");
    assert.equal(api.displayName("GB"), "🇬🇧 GB");
    assert.equal(api.displayName("🇳🇱 x"), "🇳🇱 x");
    assert.equal(api.displayName("plain name"), "plain name");
    assert.equal(api.displayName(""), "");
    assert.equal(api.displayName(null), null);
  });

  it("extractFlag finds a pair anywhere; flagCode inverts it", () => {
    const { api } = loadApp();
    assert.equal(api.extractFlag("a🇯🇵b"), "🇯🇵");
    assert.equal(api.extractFlag("plain"), "");
    assert.equal(api.extractFlag(null), "");
    assert.equal(api.flagCode("🇩🇪"), "DE");
    assert.equal(api.flagCode(""), "");
    assert.equal(api.flagCode("x"), "");
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

describe("overview updated pill tracks the freshest data touch", () => {
  const wrap = () => sandbox.document.getElementById("ov-updated-wrap");
  const text = () => sandbox.document.getElementById("ov-updated").textContent;
  it("follows refresh or ping (whichever is newer), hides mid-refresh and with no data", () => {
    const hourAgo = new Date(Date.now() - 3600_000).toISOString();
    const justNow = new Date().toISOString();
    // Refresh-only age.
    api.state.snapshot = { refreshing: false, last_refresh: hourAgo, last_ping_at: null };
    api.renderUpdated();
    assert.equal(wrap().hidden, false);
    assert.match(text(), /1.+h ago/);
    // A fresh ping moves the pill even though no scan ran: the ping
    // re-timed every ranked row, so "Updated" counts from the ping.
    api.state.snapshot = { refreshing: false, last_refresh: hourAgo, last_ping_at: justNow };
    api.renderUpdated();
    assert.equal(wrap().hidden, false);
    assert.match(text(), /just now/);
    // Newer refresh moves the pill too.
    api.state.snapshot = { refreshing: false, last_refresh: justNow, last_ping_at: hourAgo };
    api.renderUpdated();
    assert.match(text(), /just now/);
    // Unparseable stamps behave like no data.
    api.state.snapshot = { refreshing: false, last_refresh: "not-a-date", last_ping_at: null };
    api.renderUpdated();
    assert.equal(wrap().hidden, true);
    assert.equal(text(), "");
    // Mid-refresh: dot only, no stale stamp.
    api.state.snapshot = { refreshing: true, last_refresh: hourAgo, last_ping_at: justNow };
    api.renderUpdated();
    assert.equal(wrap().hidden, false);
    assert.equal(text(), "");
    // No data yet: fully hidden.
    api.state.snapshot = null;
    api.renderUpdated();
    assert.equal(wrap().hidden, true);
  });

  it("pill runs ahead of the Last scan age right after a ping", () => {
    api.state.hasSummaryApi = true;
    api.state.refreshSeconds = 300;
    api.state.pingSeconds = 60;
    api.state.startedAt = new Date(Date.now() - 3_600_000).toISOString();
    api.state.snapshot = {
      refreshing: false,
      pinging: false,
      total_candidates: 8,
      tested_candidates: 8,
      reachable_candidates: 2,
      fetch_bytes: 1234,
      last_refresh: new Date(Date.now() - 90_000).toISOString(),
      refresh_duration_ms: 6100,
      last_ping_at: new Date().toISOString(),
      ranked: [],
      fetch_errors: [],
      proxy_running: false,
    };
    api.renderStats();
    const scan = sandbox.__elements.get("stat-cards").children[2];
    const age = scan.children.find((c) => c.id === "stat-scan-age").textContent;
    assert.match(age, /1.+m ago/);
    // renderStats re-renders the pill too: it counts from the fresh ping
    // while the Last scan age line keeps the refresh anchor.
    assert.match(text(), /just now/, `pill ${JSON.stringify(text())} follows the ping`);
    assert.ok(!text().endsWith(age), `pill ${JSON.stringify(text())} runs ahead of scan age ${JSON.stringify(age)}`);
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

  it("nextCycleMessage flags deferred settings edits in every locale", () => {
    // A refresh-relevant PATCH/reset no longer re-fetches at once: the toast
    // must promise the next cycle (naming the real Refresh button) instead
    // of a plain saved note. Anything else falls through to subMessage.
    for (const locale of ["en", "ir", "cn", "fr", "ru"]) {
      api.selectLang(locale);
      const msg = api.nextCycleMessage({ data: { status: "Updated top_n", code: "applies_next_cycle" } }, "fb");
      assert.ok(!msg.includes("applies_next_cycle"), `${locale}: no machine flag leaks`);
      assert.ok(msg.includes(api.t("btnRefresh")), `${locale}: names the Refresh button`);
      assert.equal(
        api.nextCycleMessage({ data: { status: "Updated bind" } }, "fb"),
        "Updated bind",
        `${locale}: plain saves keep server status`,
      );
      assert.equal(api.nextCycleMessage({ data: null }, "fb"), "fb", `${locale}: fallback`);
    }
    api.selectLang("en");
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

describe("polling fallback recovers capabilities after transient failure", () => {
  it("status 0 on /api/summary stays unknown + retries, 200 promotes", async () => {
    const timers = [];
    const sb = makeSandbox();
    let summaryCalls = 0;
    sb.fetch = async (url) => {
      if (String(url).includes("/api/summary")) {
        summaryCalls += 1;
        if (summaryCalls === 1) {
          return { status: 0, async text() { return ""; } };
        }
        return { status: 200, async text() { return '{"refresh_seconds":900,"ping_seconds":300}'; } };
      }
      return { status: 0, async text() { return ""; } };
    };
    sb.window.setTimeout = (fn, ms) => { timers.push({ fn, ms }); return timers.length; };
    sb.window.clearTimeout = () => {};
    sb.window.setInterval = () => 0;
    sb.window.clearInterval = () => {};
    sb.setTimeout = (fn, ms) => { timers.push({ fn, ms }); return timers.length; };
    const { api: fresh } = loadApp(sb);
    fresh.startPolling(false);
    await new Promise((r) => setTimeout(r, 100));
    assert.equal(fresh.state.hasSummaryApi, null, "transient summary failure stays unknown, never false");
    const retry = timers.find((t) => t.ms === 30000);
    assert.ok(retry, "schedules one capabilities retry");
    retry.fn();
    await new Promise((r) => setTimeout(r, 100));
    assert.equal(summaryCalls, 2, "retry re-requests capabilities");
    assert.equal(fresh.state.hasSummaryApi, true, "retry promotes capabilities on 200");
  });

  it("404 on /api/summary cements false with no retry (old server)", async () => {
    const timers = [];
    const sb = makeSandbox();
    sb.fetch = async () => ({ status: 404, async text() { return ""; } });
    sb.window.setTimeout = (fn, ms) => { timers.push({ fn, ms }); return timers.length; };
    sb.window.clearTimeout = () => {};
    sb.window.setInterval = () => 0;
    sb.window.clearInterval = () => {};
    const { api: fresh } = loadApp(sb);
    fresh.startPolling(false);
    await new Promise((r) => setTimeout(r, 100));
    assert.equal(fresh.state.hasSummaryApi, false, "old server stays on full polling");
    assert.ok(!timers.some((t) => t.ms === 30000), "no retry scheduled for a proven-absent API");
  });
});

describe("settings tab: typed controls + translated rows", () => {
  const payload = () => ({
    dirty: false,
    groups: [{
      id: "connection",
      title: "Connection",
      keys: [
        { key: "bind", value: "127.0.0.1:27141", guide: "Local bind.", kind: "text", options: [] },
        { key: "top_n", value: "8", guide: "Kept.", kind: "int", options: [] },
        { key: "encoded_subscription", value: "false", guide: "Feed kind.", kind: "bool", options: [] },
        { key: "probe.mode", value: "active", guide: "Mode.", kind: "choice", options: ["active", "tcp"] },
        { key: "sharing.token", value: "empty", guide: "Secret.", kind: "secret", options: [] },
        { key: "probe.speedtest_enabled", value: "false", guide: "Derived.", kind: "readonly", options: [] },
      ],
    }],
  });

  it("settingKind prefers server kind, infers old-server rows", () => {
    assert.equal(api.settingKind({ kind: "bool" }), "bool");
    assert.equal(api.settingKind({ key: "x", value: "true" }), "bool");
    assert.equal(api.settingKind({ key: "sharing.token", value: "set" }), "secret");
    assert.equal(api.settingKind({ key: "probe.mode", value: "active" }), "choice");
    assert.equal(api.settingKind({ key: "subscription_count", value: "2" }), "readonly");
    assert.equal(api.settingKind({ key: "bind", value: "127.0.0.1:27141" }), "text");
    assert.equal(api.settingKind(null), "text");
  });

  it("names/guides translate, unknown keys fall back to server strings", () => {
    assert.equal(api.settingName("bind"), "Bind address");
    assert.match(api.settingGuide("bind", "srv"), /dashboard listens/);
    assert.match(api.settingGuide("bind", "srv"), /Type:/);
    assert.match(api.settingGuide("bind", "srv"), /Example:/);
    assert.equal(api.settingName("future.key"), "future.key");
    assert.equal(api.settingGuide("future.key", "srv"), "srv");
    assert.equal(api.settingGroupTitle({ id: "probe", title: "Probe" }), "Probe");
    assert.equal(api.settingGroupTitle({ title: "Legacy" }), "Legacy");
    assert.equal(api.setLanguage("fa"), true);
    assert.equal(api.settingName("bind"), "آدرس گوش دادن");
    assert.equal(api.setLanguage("en"), true);
  });

  it("renderSettings builds name/value/description rows with typed controls", () => {
    api.state.settings = payload();
    api.state.settingsStatus = 200;
    api.renderSettings();
    const box = sandbox.__elements.get("settings-groups");
    assert.equal(box.children.length, 1);
    const card = box.children[0];
    assert.equal(card.children[0].textContent, "Connection");
    const rows = card.children.slice(1);
    assert.equal(rows.length, 6);
    // Name | control | description columns.
    assert.equal(rows[0].children[0].textContent, "Bind address");
    assert.equal(rows[0].children[0].className, "set-name");
    assert.equal(rows[0].children[1].className, "val editable");
    assert.match(rows[0].children[2].textContent, /dashboard listens/);
    assert.match(rows[0].children[2].textContent, /Example:/);
    // Bool row renders an off switch.
    const sw = rows[2].children[1].children[0];
    assert.equal(sw.getAttribute("role"), "switch");
    assert.equal(sw.getAttribute("aria-checked"), "false");
    assert.equal(sw.textContent, "Off");
    // Choice row renders a select; secret row a presence + setter; readonly is static.
    assert.equal(rows[3].children[1].children[0].tagName, "SELECT");
    assert.equal(rows[4].children[1].children[0].textContent, "empty");
    assert.equal(rows[5].children[1].children.length, 0);
  });

  it("switch click PATCHes the flipped value", async () => {
    const bodies = [];
    sandbox.fetch = async (url, init) => {
      const body = init && init.body ? JSON.parse(init.body) : null;
      bodies.push({ url: String(url), body });
      if (String(url).includes("/api/config") && init && init.method === "PATCH") {
        return { status: 200, async text() { return '{"ok":true,"status":"Updated.","dirty":false}'; } };
      }
      return { status: 200, async text() { return JSON.stringify(payload()); } };
    };
    api.state.settings = payload();
    api.state.settingsStatus = 200;
    api.renderSettings();
    const box = sandbox.__elements.get("settings-groups");
    const sw = box.children[0].children[3].children[1].children[0];
    sw.__listeners.get("click")[0]();
    await new Promise((r) => setTimeout(r, 100));
    const patch = bodies.find((b) => b.body && b.body.key === "encoded_subscription");
    assert.deepEqual(patch.body, { key: "encoded_subscription", value: "true" });
  });

  it("normalizeSettingInput folds keyboard digits and drops bidi controls", () => {
    // Persian, Arabic-Indic, and fullwidth digits from real keyboards.
    assert.equal(api.normalizeSettingInput("۹۰۰", true), "900");
    assert.equal(api.normalizeSettingInput("٣٠٠", true), "300");
    assert.equal(api.normalizeSettingInput("９００", true), "900");
    // BIDI isolates hitchhiking from RTL paste.
    assert.equal(api.normalizeSettingInput(" 900 ", true), "900");
    // Text fields keep their characters (no digit folding).
    assert.equal(api.normalizeSettingInput("vless://u@h:443#e", false), "vless://u@h:443#e");
    assert.equal(api.normalizeSettingInput("۱۲۳", false), "۱۲۳");
    // Genuine garbage passes through for the server to refuse.
    assert.equal(api.normalizeSettingInput("abc", true), "abc");
  });

  it("shareUrlLabel reuses the endpoint labels, honoring base64 mode", () => {
    api.state.ovConfig = new Map([["encoded_subscription", "true"]]);
    assert.equal(api.shareUrlLabel("subscription"), "Subscription (base64)");
    assert.equal(api.shareUrlLabel("subscription_txt"), "Subscription (plain text)");
    assert.equal(api.shareUrlLabel("mihomo"), "Mihomo YAML");
    api.state.ovConfig = new Map([["encoded_subscription", "false"]]);
    assert.equal(api.shareUrlLabel("subscription"), "Subscription (plain text)");
  });

  it("secret row Shows the token on demand, copies, and hides again", async () => {
    const seen = [];
    sandbox.fetch = async (url, init) => {
      seen.push(String(url));
      return { status: 200, async text() { return '{"token":"unit-secret"}'; } };
    };
    const withToken = payload();
    withToken.groups[0].keys[4].value = "set";
    api.state.settings = withToken;
    api.state.settingsStatus = 200;
    api.renderSettings();
    const box = sandbox.__elements.get("settings-groups");
    const wrap = box.children[0].children[5].children[1];
    const show = wrap.children[1];
    assert.equal(show.textContent, "Show");
    show.__listeners.get("click")[0]();
    await new Promise((r) => setTimeout(r, 50));
    assert.ok(seen.some((u) => u.includes("/api/config/token")), "fetches only on Show");
    const code = wrap.children[0];
    assert.equal(code.textContent, "unit-secret");
    const hide = wrap.children[2];
    assert.equal(hide.textContent, "Hide");
    hide.__listeners.get("click")[0]();
    assert.equal(box.children[0].children[5].children[1].children[0].textContent, "set");
  });
});

describe("live-logs severity filter contract", () => {
  it("LOG_LEVELS/LOG_LEVEL_ORDER rank DEBUG < INFO < WARN < ERROR", () => {
    // Arrays/objects are built inside the vm sandbox (different prototypes),
    // so compare through JSON/fields rather than deepStrictEqual.
    assert.equal(JSON.stringify(api.LOG_LEVELS), JSON.stringify(["DEBUG", "INFO", "WARN", "ERROR"]));
    assert.equal(api.LOG_LEVEL_ORDER.DEBUG, 0);
    assert.equal(api.LOG_LEVEL_ORDER.INFO, 1);
    assert.equal(api.LOG_LEVEL_ORDER.WARN, 2);
    assert.equal(api.LOG_LEVEL_ORDER.ERROR, 3);
  });

  it("parseLogLine splits ts/level/msg on tagged lines", () => {
    const info = api.parseLogLine("20:20:02.792 [INFO] hello world");
    assert.equal(info.ts, "20:20:02.792");
    assert.equal(info.level, "INFO");
    assert.equal(info.msg, "hello world");

    const warn = api.parseLogLine("20:20:03.001 [WARN] slow peer");
    assert.equal(warn.ts, "20:20:03.001");
    assert.equal(warn.level, "WARN");
    assert.equal(warn.msg, "slow peer");

    const err = api.parseLogLine("20:20:04.120 [ERROR] dial failed");
    assert.equal(err.ts, "20:20:04.120");
    assert.equal(err.level, "ERROR");
    assert.equal(err.msg, "dial failed");

    const dbg = api.parseLogLine("20:20:05.000 [DEBUG] probe tick");
    assert.equal(dbg.ts, "20:20:05.000");
    assert.equal(dbg.level, "DEBUG");
    assert.equal(dbg.msg, "probe tick");
  });

  it("parseLogLine folds unknown levels to INFO, keeps legacy/non-string", () => {
    const unknown = api.parseLogLine("20:20:02.792 [VERBOSE] chatter");
    assert.equal(unknown.ts, "20:20:02.792");
    assert.equal(unknown.level, "INFO");
    assert.equal(unknown.msg, "chatter");

    const legacy = api.parseLogLine("boot ok");
    assert.equal(legacy.ts, "");
    assert.equal(legacy.level, "INFO");
    assert.equal(legacy.msg, "boot ok");

    const empty = api.parseLogLine("");
    assert.equal(empty.ts, "");
    assert.equal(empty.level, "INFO");
    assert.equal(empty.msg, "");

    for (const bad of [null, undefined, 42, {}]) {
      const p = api.parseLogLine(bad);
      assert.equal(p.ts, "");
      assert.equal(p.level, "INFO");
      assert.equal(p.msg, String(bad));
    }
    // A tagged line with no message renders once, not doubled.
    const bare = api.parseLogLine("20:20:02.792 [INFO]");
    assert.equal(bare.ts, "");
    assert.equal(bare.msg, "20:20:02.792 [INFO]");
  });

  it("logLineVisible gates rows by severity threshold", () => {
    const at = (level) => ({ ts: "", level, msg: "x" });
    for (const level of ["DEBUG", "INFO", "WARN", "ERROR"]) {
      assert.equal(api.logLineVisible(at(level), "ALL", ""), true, `ALL shows ${level}`);
    }
    assert.equal(api.logLineVisible(at("INFO"), "INFO", ""), true);
    assert.equal(api.logLineVisible(at("WARN"), "INFO", ""), true);
    assert.equal(api.logLineVisible(at("ERROR"), "INFO", ""), true);
    assert.equal(api.logLineVisible(at("DEBUG"), "INFO", ""), false, "INFO hides DEBUG");
    assert.equal(api.logLineVisible(at("WARN"), "WARN", ""), true);
    assert.equal(api.logLineVisible(at("ERROR"), "WARN", ""), true);
    assert.equal(api.logLineVisible(at("INFO"), "WARN", ""), false, "WARN hides INFO");
    assert.equal(api.logLineVisible(at("DEBUG"), "WARN", ""), false, "WARN hides DEBUG");
    assert.equal(api.logLineVisible(at("ERROR"), "ERROR", ""), true);
    assert.equal(api.logLineVisible(at("WARN"), "ERROR", ""), false, "ERROR hides WARN");
    assert.equal(api.logLineVisible(at("INFO"), "ERROR", ""), false, "ERROR hides INFO");
    assert.equal(api.logLineVisible(at("DEBUG"), "ERROR", ""), false, "ERROR hides DEBUG");
  });

  it("logLineVisible matches query against the raw line, like the old text filter", () => {
    const raw = "20:20:02.792 [INFO] Hello World";
    const row = api.parseLogLine(raw);
    assert.equal(api.logLineVisible(row, "ALL", "", raw), true, "empty query passes");
    assert.equal(api.logLineVisible(row, "ALL", "hello", raw), true, "lowercase query hits mixed-case msg");
    assert.equal(api.logLineVisible(row, "ALL", "missing", raw), false);
    // Timestamp fragments and level tokens match: same scope as before.
    assert.equal(api.logLineVisible(row, "ALL", "20:20", raw), true, "timestamp fragment matches");
    assert.equal(api.logLineVisible(row, "ALL", "info", raw), true, "level token matches");
    const errRaw = "20:20:04.120 [ERROR] all good";
    const err = api.parseLogLine(errRaw);
    assert.equal(api.logLineVisible(err, "ALL", "good", errRaw), true);
    // Severity and text combine: both must pass.
    assert.equal(api.logLineVisible(err, "ERROR", "good", errRaw), true);
    assert.equal(api.logLineVisible(err, "ERROR", "missing", errRaw), false);
    assert.equal(api.logLineVisible(api.parseLogLine("20:20:05.000 [DEBUG] good"), "ERROR", "good", "20:20:05.000 [DEBUG] good"), false);
  });
});
