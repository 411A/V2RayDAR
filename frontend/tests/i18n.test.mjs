import { describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";
import { FRONTEND_DIR, loadApp } from "./helpers/load-app.mjs";

const read = (f) => fs.readFileSync(path.join(FRONTEND_DIR, f), "utf8");

/** Execute only i18n.js in an empty context and return the `en` table. */
function enTable() {
  const box = {};
  vm.runInNewContext(`${read("i18n.js")};globalThis.__en = I18N_STRINGS.en;`, box, {
    filename: "i18n.js",
  });
  return box.__en;
}

describe("frontend i18n: one unified strings file, English default", () => {
  it("every t(\"key\") in app.js exists in the en table", () => {
    const en = enTable();
    const used = new Set([...read("app.js").matchAll(/\bt\(\s*"([^"]+)"\s*[,)]/g)].map((m) => m[1]));
    assert.ok(used.size > 100, `expected 100+ string keys, found ${used.size}`);
    const missing = [...used].filter((k) => !Object.prototype.hasOwnProperty.call(en, k));
    assert.deepEqual(missing, [], `t() keys missing from en: ${missing.join(", ")}`);
  });

  it("every data-i18n* binding in index.html exists in the en table", () => {
    const en = enTable();
    const html = read("index.html");
    const bound = new Set();
    for (const attr of ["data-i18n", "data-i18n-ph", "data-i18n-aria", "data-i18n-title", "data-i18n-alt", "data-i18n-content"]) {
      for (const m of html.matchAll(new RegExp(`${attr}="([^"]+)"`, "g"))) {
        bound.add(m[1]);
      }
    }
    assert.ok(bound.size > 100, `expected 100+ HTML bindings, found ${bound.size}`);
    const missing = [...bound].filter((k) => !Object.prototype.hasOwnProperty.call(en, k));
    assert.deepEqual(missing, [], `HTML bindings missing from en: ${missing.join(", ")}`);
  });

  it("no hardcoded user sentence remains in app.js string literals", () => {
    // i18n keys are camelCase without spaces; any single-line double-quoted
    // literal WITH a space must be glue, a symbol/unit, or a code fragment.
    const code = read("app.js")
      .replace(/\/\/.*$/gm, "")
      .replace(/\/\*[\s\S]*?\*\//g, "");
    // Single-pass tokenizer: real string opens consume the whole literal, so
    // a match can never start at a closing quote and span two neighbours.
    const spaced = [];
    for (const m of code.matchAll(/"((?:[^"\\\n]|\\.)*)"|[\s\S]/g)) {
      if (m[1] !== undefined && m[1].includes(" ")) {
        spaced.push(m[1]);
      }
    }
    const legit = new Set([
      "use strict",
      "(prefers-color-scheme: dark)",
      "stat card",
      "btn small",
      " ",
      " · ",
      " http://",
      "; ",
      "m ",
      "h ",
      " ms",
      " s",
      " B",
      " KB",
      " MB",
      " GB",
      " Mbps",
    ]);
    const bad = [...new Set(spaced)].filter((s) => !legit.has(s));
    assert.deepEqual(bad, [], `hardcoded user strings left in app.js: ${bad.join(" | ")}`);
  });

  it("t() substitutes placeholders and falls back to the key", () => {
    const { api } = loadApp();
    assert.equal(api.t("btnRefresh"), "Refresh");
    assert.equal(api.t("cfgCount", { n: 3, total: 9, limit: "" }), "Showing 3 of 9 ranked configs.");
    assert.equal(
      api.t("cfgCount", { n: 3, total: 9, limit: api.t("cfgLimit", { limit: 25 }) }),
      "Showing 3 of 9 ranked configs (limit 25).",
    );
    assert.equal(api.t("definitely-not-a-key"), "definitely-not-a-key");
  });

  it("default language is English; unknown languages are refused", () => {
    const { api } = loadApp();
    assert.equal(api.t("langAria"), "Language: English");
    assert.equal(api.setLanguage("xx"), false);
  });

  it("language menu toggles, EN sticks, others toast coming-soon", () => {
    const { api, sandbox } = loadApp();
    const menu = sandbox.__elements.get("lang-menu");
    assert.equal(menu.hidden, false); // stub default; wire() owns the real state
    api.toggleLangMenu();
    assert.equal(menu.hidden, true);
    api.toggleLangMenu();
    assert.equal(menu.hidden, false);
    assert.equal(api.closeLangMenu(), true);
    assert.equal(menu.hidden, true);
    assert.equal(api.closeLangMenu(), false);
    // Non-English: toast names the language, English stays.
    api.selectLang("ir");
    const toasts = sandbox.__elements.get("toasts");
    const last = toasts.children[toasts.children.length - 1];
    assert.match(last.textContent, /is coming soon/);
    assert.equal(api.t("btnRefresh"), "Refresh");
    // Unknown codes are ignored silently.
    api.selectLang("xx");
    // Menu rows cover every LANG in order (EN, IR, CN, FR, RU).
    const html = fs.readFileSync(path.join(FRONTEND_DIR, "index.html"), "utf8");
    const rows = [...html.matchAll(/data-lang="([a-z]+)"/g)].map((m) => m[1]);
    assert.deepEqual(rows, ["en", "ir", "cn", "fr", "ru"]);
  });

  it("every ./assets/*.svg referenced in index.html exists on disk", () => {
    const html = read("index.html");
    const refs = new Set([...html.matchAll(/\.\/assets\/([A-Za-z0-9._-]+)/g)].map((m) => m[1]));
    assert.ok(refs.size >= 5, `expected 5+ flag refs, found ${refs.size}`);
    for (const f of refs) {
      assert.ok(
        fs.existsSync(path.join(FRONTEND_DIR, "assets", f)),
        `missing frontend/assets/${f}`,
      );
    }
  });
});
