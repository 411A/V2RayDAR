import { describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = path.resolve(here, "..");
const repo = path.resolve(dir, "..");
const read = (f) => fs.readFileSync(path.join(dir, f), "utf8");
const readRepo = (f) => fs.readFileSync(path.join(repo, f), "utf8");

// PLAN.md §5 - "Performance & efficiency budgets (hard gates)".
// Every budget below is a number the plan calls a hard gate, so every one of
// them is asserted here. The plan says "test asserts"; this file is that
// test. A budget nobody checks is a budget that silently stops being true.
const INITIAL_PAYLOAD_CAP = 404 * 1024; // PLAN §5, "Initial payload"
const MAX_LOG_RING = 300; // PLAN §5, "ring buffers: logs 300"
const MAX_RANKED_ROWS = 64; // PLAN §5, "table capped at 64 rows"
const MAX_DOM_PATCH_HZ = 4; // PLAN §5, "DOM patch <= 4 Hz"
const MAX_SSE_KEEPALIVE_S = 15; // PLAN §5, "heartbeat <= 1/15 s"

describe("PLAN §5 hard gates", () => {
  it("initial payload (HTML+CSS+JS+i18n) stays inside the budget", () => {
    const files = ["index.html", "style.css", "app.js", "i18n.js", "qr.js"];
    let total = 0;
    for (const f of files) {
      total += fs.statSync(path.join(dir, f)).size;
    }
    assert.ok(
      total <= INITIAL_PAYLOAD_CAP,
      `initial payload is ${(total / 1024).toFixed(1)} KiB, over the ${INITIAL_PAYLOAD_CAP / 1024} KiB budget`,
    );
  });

  it("the log ring buffer stays bounded", () => {
    const m = read("app.js").match(/const MAX_LOGS = (\d+);/);
    assert.ok(m, "MAX_LOGS must be defined");
    assert.ok(
      Number(m[1]) <= MAX_LOG_RING,
      `MAX_LOGS=${m[1]} exceeds the ${MAX_LOG_RING}-line ring buffer`,
    );
  });

  it("idle polling stays cheaper than the 4 Hz DOM patch budget", () => {
    const src = read("app.js");
    const summary = src.match(/const POLL_SUMMARY_MS = (\d+);/);
    const results = src.match(/const POLL_RESULTS_MS = (\d+);/);
    assert.ok(summary && results, "the poll cadences must be defined");
    const period = Math.max(Number(summary[1]), Number(results[1]));
    assert.ok(
      1000 / period <= MAX_DOM_PATCH_HZ,
      `a ${period} ms poll is ${(1000 / period).toFixed(2)} Hz, over the ${MAX_DOM_PATCH_HZ} Hz budget`,
    );
  });

  it("no local state is cached beyond the UI preference keys", () => {
    const src = read("app.js");
    const keys = [...src.matchAll(/localStorage\.(?:get|set)Item\((\w+)/g)].map((m) => m[1]);
    assert.ok(keys.length > 0, "the theme/tab preferences must persist");
    // Every key must be a UI preference. A subscription URL, a token or a
    // config cache in localStorage would be stale state the plan forbids.
    for (const key of new Set(keys)) {
      assert.match(
        key,
        /^(THEME|TAB|ROW_LIMIT|LOG_LEVEL)_KEY$/,
        `localStorage key ${key} is not a UI preference`,
      );
    }
  });

  it("no service worker and no WebSocket (SSE only)", () => {
    const src = read("app.js");
    assert.ok(!/serviceWorker/.test(src), "no service worker is allowed");
    assert.ok(!/new WebSocket/.test(src), "the feed must be SSE, not WebSocket");
    assert.ok(/new EventSource/.test(src), "the feed must use EventSource");
  });

  it("the ranked table is capped at 64 rows", () => {
    const m = readRepo("src/constants.rs").match(
      /pub const TUI_MAX_VISIBLE_RANKED: usize = (\d+);/,
    );
    assert.ok(m, "TUI_MAX_VISIBLE_RANKED must be defined");
    assert.ok(
      Number(m[1]) <= MAX_RANKED_ROWS,
      `the table cap is ${m[1]}, over the ${MAX_RANKED_ROWS}-row budget`,
    );
  });

  it("the server heartbeats the feed at least every 15 s and caps the clients", () => {
    const src = readRepo("src/web.rs");
    const keepalive = src.match(/const FEED_KEEP_ALIVE: Duration = Duration::from_secs\((\d+)\);/);
    assert.ok(keepalive, "FEED_KEEP_ALIVE must be defined");
    assert.ok(
      Number(keepalive[1]) <= MAX_SSE_KEEPALIVE_S,
      `the heartbeat is ${keepalive[1]} s, over the ${MAX_SSE_KEEPALIVE_S} s budget`,
    );

    // A dashboard tab farm must never stall the probe loops: the cap exists
    // and the extra client is turned away instead of being served.
    const cap = src.match(/const FEED_MAX_CONNECTIONS: usize = (\d+);/);
    assert.ok(cap, "FEED_MAX_CONNECTIONS must be defined");
    assert.ok(
      Number(cap[1]) > 0 && Number(cap[1]) <= 32,
      `the SSE client cap is ${cap[1]}, which does not protect the probe loops`,
    );
    assert.match(
      src,
      /ACTIVE_FEEDS\.fetch_add\([^)]*\)[^;]*>=\s*FEED_MAX_CONNECTIONS/,
      "the cap has to be enforced before a client is served",
    );
  });

  it("long tables use content-visibility so weak CPUs stay smooth", () => {
    assert.match(read("style.css"), /content-visibility:\s*auto/);
  });
});
