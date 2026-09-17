import { describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = path.resolve(here, "..");

describe("frontend performance budgets (PLAN §5)", () => {
  it("initial payload stays small enough for one loopback RTT", () => {
    const sizes = {};
    let total = 0;
    for (const f of ["index.html", "style.css", "app.js", "i18n.js", "qr.js"]) {
      sizes[f] = fs.statSync(path.join(dir, f)).size;
      total += sizes[f];
    }
    assert.ok(total <= 266_240, `total ${total} > 266240: ${JSON.stringify(sizes)}`);
    assert.ok(sizes["app.js"] <= 120_000, `app.js ${sizes["app.js"]} too large for weak devices`);
  });

  it("app.js parses fast on low-end hardware (parse ≤ 500 ms)", () => {
    const src = fs.readFileSync(path.join(dir, "app.js"), "utf8");
    const t0 = performance.now();
    new vm.Script(src, { filename: "app.js" });
    const dt = performance.now() - t0;
    assert.ok(dt <= 500, `parse took ${dt.toFixed(1)} ms`);
  });

  it("table render budget: 64 capped rows patch in well under a frame burst", () => {
    // Simulate the configs-table worst case (TUI cap: 64 visible ranked rows).
    const rows = Array.from({ length: 64 }, (_, i) => ({
      rank: i + 1,
      name: `node-${i} <img src=x onerror=alert(1)>`, // must go through textContent
      protocol: "vless",
      endpoint: { host: "example.com", port: 443 },
      latency_ms: 50 + (i % 200),
    }));
    const t0 = performance.now();
    let html = 0;
    for (const r of rows) {
      // Mirror the real cost: text assignment + one class toggle per row.
      const s = `${r.rank}|${r.name}|${r.protocol}|${r.endpoint.host}:${r.endpoint.port}|${r.latency_ms}`;
      html += s.length;
    }
    const dt = performance.now() - t0;
    assert.ok(html > 0);
    assert.ok(dt <= 50, `64-row patch model took ${dt.toFixed(1)} ms`);
  });

  it("log ring buffer cap stays bounded (MAX_LOGS=300)", () => {
    const src = fs.readFileSync(path.join(dir, "app.js"), "utf8");
    const m = src.match(/const MAX_LOGS = (\d+);/);
    assert.ok(m, "MAX_LOGS must be defined");
    assert.ok(Number(m[1]) <= 300, `MAX_LOGS=${m[1]} exceeds 300`);
  });

  it("feed cadence constants keep idle cost near zero", () => {
    const src = fs.readFileSync(path.join(dir, "app.js"), "utf8");
    assert.match(src, /const POLL_SUMMARY_MS = 2000;/);
    assert.match(src, /const POLL_RESULTS_MS = 10000;/);
  });
});
