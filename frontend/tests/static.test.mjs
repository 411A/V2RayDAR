import { describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = path.resolve(here, "..");
const read = (f) => fs.readFileSync(path.join(dir, f), "utf8");

describe("frontend static gates (PLAN §5 + TODO global gates)", () => {
  it("node --check passes for app.js, i18n.js and qr.js", () => {
    execFileSync(process.execPath, ["--check", path.join(dir, "app.js")]);
    execFileSync(process.execPath, ["--check", path.join(dir, "i18n.js")]);
    execFileSync(process.execPath, ["--check", path.join(dir, "qr.js")]);
  });

  it("zero console.*/innerHTML/eval/Function/new-function sinks", () => {
    for (const f of ["app.js", "i18n.js", "qr.js"]) {
      const src = read(f);
      assert.ok(!/console\.(log|debug|info|warn|error)/.test(src), `${f} must not call console.*`);
      assert.ok(!/\.innerHTML\s*=/.test(src), `${f} must not use innerHTML`);
      assert.ok(!/\beval\s*\(/.test(src), `${f} must not use eval`);
      assert.ok(!/\bnew\s+Function\s*\(/.test(src), `${f} must not use new Function`);
    }
  });

  it("no remote refs, no url( in CSS, same-origin fetch only", () => {
    const html = read("index.html");
    const css = read("style.css");
    const js = read("app.js");
    assert.ok(!/https?:\/\//.test(css.replace(/semantics|prefixes/i, "")), "style.css must not fetch remote URLs");
    assert.ok(!/url\s*\(/.test(css), "style.css must not use url()");
    assert.ok(!/<script[^>]+src\s*=\s*["']https?:/i.test(html), "index.html must not load remote scripts");
    assert.ok(!/<link[^>]+href\s*=\s*["']https?:/i.test(html), "index.html must not load remote styles");
    assert.ok(!/fetch\s*\(\s*["']https?:/.test(js), "app.js must only call same-origin APIs");
  });

  it('every $("id") exists in index.html or is created/guarded at runtime', () => {
    const html = read("index.html");
    const js = read("app.js");
    const ids = new Set([...html.matchAll(/id="([^"]+)"/g)].map((m) => m[1]));
    const used = new Set([...js.matchAll(/\$\("([^"]+)"\)/g)].map((m) => m[1]));
    // Dynamically created via statCard(valueId, subId) (`v.id = valueId`):
    // any "stat-*" string literal in app.js is a runtime-created id.
    const dynamic = new Set(
      [...js.matchAll(/"(stat-[a-z-]+)"/g)].map((m) => m[1]),
    );
    // Footer ids removed with the bottom status bar (TODO #13); setStatus /
    // setDirty null-guard them, so absence from index.html is intentional.
    const nullGuarded = new Set(["status-text", "status-dirty"]);
    for (const id of nullGuarded) {
      assert.match(js, new RegExp(`\\$\\("${id}"\\)[\\s\\S]{0,120}if \\(`), `${id} must stay null-guarded`);
    }
    const missing = [...used].filter((id) => !ids.has(id) && !dynamic.has(id) && !nullGuarded.has(id));
    assert.deepEqual(missing, [], `missing ids: ${missing.join(", ")}`);
  });

  it("payload budget: HTML+CSS+JS+i18n+QR ≤ 266240 bytes (PLAN §5)", () => {
    const total =
      fs.statSync(path.join(dir, "index.html")).size +
      fs.statSync(path.join(dir, "style.css")).size +
      fs.statSync(path.join(dir, "app.js")).size +
      fs.statSync(path.join(dir, "i18n.js")).size +
      fs.statSync(path.join(dir, "qr.js")).size;
    assert.ok(total <= 266_240, `payload ${total} bytes exceeds 266240`);
  });
});
