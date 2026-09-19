import { describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";
import { FRONTEND_DIR, loadApp } from "./helpers/load-app.mjs";

const read = (f) => fs.readFileSync(path.join(FRONTEND_DIR, f), "utf8");

const LOCALES = ["en", "fa", "zh", "fr", "ru"];

/** Execute only i18n.js in an empty context and return all locale tables. */
function allTables() {
  const box = {};
  vm.runInNewContext(`${read("i18n.js")};globalThis.__all = I18N_STRINGS;`, box, {
    filename: "i18n.js",
  });
  return box.__all;
}

describe("frontend i18n: one unified strings file, English default", () => {
  it("every t(\"key\") in app.js exists in every locale table", () => {
    const all = allTables();
    const used = new Set([...read("app.js").matchAll(/\bt\(\s*"([^"]+)"\s*[,)]/g)].map((m) => m[1]));
    assert.ok(used.size > 100, `expected 100+ string keys, found ${used.size}`);
    for (const locale of LOCALES) {
      const missing = [...used].filter((k) => !Object.prototype.hasOwnProperty.call(all[locale], k));
      assert.deepEqual(missing, [], `t() keys missing from ${locale}: ${missing.join(", ")}`);
    }
  });

  it("every data-i18n* binding in index.html exists in every locale table", () => {
    const all = allTables();
    const html = read("index.html");
    const bound = new Set();
    for (const attr of ["data-i18n", "data-i18n-ph", "data-i18n-aria", "data-i18n-title", "data-i18n-alt", "data-i18n-content", "data-i18n-href"]) {
      for (const m of html.matchAll(new RegExp(`${attr}="([^"]+)"`, "g"))) {
        bound.add(m[1]);
      }
    }
    assert.ok(bound.size > 100, `expected 100+ HTML bindings, found ${bound.size}`);
    for (const locale of LOCALES) {
      const missing = [...bound].filter((k) => !Object.prototype.hasOwnProperty.call(all[locale], k));
      assert.deepEqual(missing, [], `HTML bindings missing from ${locale}: ${missing.join(", ")}`);
    }
  });

  it("all locales share en's exact key set and placeholder names", () => {
    const all = allTables();
    const enKeys = Object.keys(all.en).sort();
    const ph = (s) => [...s.matchAll(/\{([a-zA-Z]+)\}/g)].map((m) => m[1]).sort().join(",");
    for (const locale of LOCALES.filter((l) => l !== "en")) {
      const keys = Object.keys(all[locale]).sort();
      assert.deepEqual(keys, enKeys, `${locale} key set differs from en`);
      for (const k of enKeys) {
        assert.equal(typeof all[locale][k], "string", `${locale}.${k} is not a string`);
        assert.ok(all[locale][k].length > 0, `${locale}.${k} is empty`);
        assert.equal(ph(all[locale][k]), ph(all.en[k]), `${locale}.${k} placeholders differ from en`);
      }
    }
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
      "val editable",
      "drag-cell cell-grip",
      // Code fragments (the scanner pairs quotes naively, so the selector
      // glue below surfaces with its neighbouring single quotes attached).
      " + key + ",
      "' + key + '",
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
    // Substituted values are bidi-isolated (U+2068 FSI … U+2069 PDI) so
    // Latin numbers/URLs keep their place inside RTL sentences, while a
    // nested localized phrase keeps its own word order (FSI takes the
    // direction from the value's first strong character, never forced LTR).
    const FSI = "\u2068";
    const PDI = "\u2069";
    assert.equal(api.t("btnRefresh"), "Refresh");
    assert.equal(
      api.t("cfgCount", { n: 3, total: 9, limit: "" }),
      `Showing ${FSI}3${PDI} of ${FSI}9${PDI} ranked configs.`,
    );
    assert.equal(
      api.t("cfgCount", { n: 3, total: 9, limit: api.t("cfgLimit", { limit: 25 }) }),
      `Showing ${FSI}3${PDI} of ${FSI}9${PDI} ranked configs${FSI} (limit ${FSI}25${PDI})${PDI}.`,
    );
    assert.equal(api.setLanguage("fa"), true);
    assert.equal(api.t("btnRefresh"), "به‌روزرسانی");
    assert.equal(
      api.t("cfgCount", { n: 3, total: 9, limit: "" }),
      `نمایش ${FSI}3${PDI} از ${FSI}9${PDI} کانفیگ رتبه‌بندی‌شده.`,
    );
    // The reported case: the count stays glued to the sentence start.
    assert.ok(api.t("fetchErrSubN", { n: 3 }).startsWith(`${FSI}3${PDI} منبع`));
    assert.equal(api.setLanguage("en"), true);
    assert.equal(api.t("definitely-not-a-key"), "definitely-not-a-key");
  });

  it("nested offline-banner phrase keeps RTL number order (2 دقیقه پیش)", () => {
    // Reported BiDi bug: the offline banner nests an already-localized
    // phrase — t("bannerConnLostBody", { ago: t("minAgo", { m }) }) — and
    // the old U+2066 LRI isolate forced that phrase left-to-right, so fa
    // rendered "… دقیقه پیش 2" instead of "… 2 دقیقه پیش". FSI takes the
    // direction from the phrase's first strong character (RTL here), so the
    // logical order "2 دقیقه پیش" renders as written.
    const { api } = loadApp();
    assert.equal(api.setLanguage("fa"), true);
    const s = api.t("bannerConnLostBody", { ago: api.t("minAgo", { m: 2 }) });
    assert.ok(!s.includes("\u2066"), `no forced-LTR isolate in: ${JSON.stringify(s)}`);
    assert.ok(s.indexOf("2") < s.indexOf("دقیقه"), `number precedes noun in: ${JSON.stringify(s)}`);
    assert.ok(
      s.includes("نمایش داده‌های \u2068\u20682\u2069 دقیقه پیش\u2069."),
      `phrase intact in: ${JSON.stringify(s)}`,
    );
    assert.equal(api.setLanguage("en"), true);
  });

  it("every settings key has a translated name/guide in every locale", () => {
    // Contract with the server's /api/config groups (see web.rs
    // connection/fetch/probe/sharing/proxy/maintenance groups): adding a key
    // there must add setName_<key> + setGuide_<key> here, or the Settings tab
    // falls back to raw English.
    const all = allTables();
    const keys = [
      "bind", "top_n", "refresh_seconds", "ping_seconds",
      "encoded_subscription", "prioritize_stability", "return_configs_asap",
      "scan_all_configs", "use_cache_only", "emergency_config",
      "fetch_timeout_ms", "fetch_concurrency",
      "max_subscription_bytes", "probe.mode", "probe.sing_box_path",
      "probe.connect_timeout_ms", "probe.concurrency",
      "probe.batch_size", "probe.process_concurrency",
      "probe.active_timeout_ms",
      "probe.startup_timeout_ms", "probe.test_url",
      "probe.accepted_statuses", "probe.download_bytes_limit",
      "probe.download_url", "probe.speedtest_enabled", "sharing.enabled",
      "sharing.require_token", "sharing.token", "proxy.enabled",
      "proxy.port", "proxy.discoverable", "proxy.rotating_proxy",
      "proxy.health_check_url", "proxy.health_check_interval_seconds",
      "clean_offlines_after_days", "geoip_db_path",
    ];
    const groups = ["connection", "fetch", "probe", "sharing", "proxy", "advanced"];
    for (const locale of LOCALES) {
      for (const key of keys) {
        // Dotted API keys (probe.mode) map to underscores in i18n keys.
        const flat = key.replace(".", "_");
        assert.ok(all[locale]["setName_" + flat], `${locale}.setName_${flat} missing`);
        assert.ok(all[locale]["setGuide_" + flat], `${locale}.setGuide_${flat} missing`);
      }
      for (const id of groups) {
        assert.ok(all[locale]["setGroup_" + id], `${locale}.setGroup_${id} missing`);
      }
    }
  });

  it("every settings guide states the type and shows an example", () => {
    // Enrichment contract: each setGuide_<key> names the accepted type
    // (or is marked read-only) and is long enough to carry an example —
    // one-liner guides regress to unexplained fields.
    const markers = {
      en: ["Type:", "Read-only:"],
      fa: ["نوع:", "فقط‌خواندنی:"],
      zh: ["类型", "只读"],
      fr: ["Type :", "Lecture seule"],
      ru: ["Тип:", "Только чтение"],
    };
    const all = allTables();
    for (const locale of LOCALES) {
      const guideKeys = Object.keys(all[locale]).filter((k) => k.startsWith("setGuide_"));
      assert.ok(guideKeys.length >= 37, `${locale} lost settings guides`);
      for (const k of guideKeys) {
        const text = all[locale][k];
        assert.ok(
          markers[locale].some((m) => text.includes(m)),
          `${locale}.${k} states no type`,
        );
        assert.ok(text.length >= 30, `${locale}.${k} carries no example`);
      }
    }
  });

  it("default language is English; unknown languages are refused", () => {
    const { api } = loadApp();
    assert.equal(api.t("langAria"), "Language: English");
    assert.equal(api.setLanguage("xx"), false);
  });

  it("measurements stay LTR-glued for RTL (2.1 s, 5.8 MB, x3)", () => {
    const { api } = loadApp();
    const LRI = "\u2066";
    const PDI = "\u2069";
    assert.equal(api.fmtLatency(2100), `${LRI}2.1 s${PDI}`);
    assert.equal(api.fmtLatency(42), `${LRI}42 ms${PDI}`);
    assert.equal(api.fmtLatencyMs(2100), `${LRI}2100 ms${PDI}`);
    assert.equal(api.fmtLatencyMs(42), `${LRI}42 ms${PDI}`);
    assert.equal(api.fmtLatencyMs(null), "—");
    assert.equal(api.fmtBytes(6080000), `${LRI}5.8 MB${PDI}`);
    assert.equal(api.fmtDuration(130000), `${LRI}2m 10s${PDI}`);
    assert.equal(api.fmtLatency(null), "—");
    assert.equal(api.ltr("×3"), `${LRI}×3${PDI}`);
  });

  it("navigation-link arrows match reading direction (fa <-, rest ->)", () => {
    const all = allTables();
    for (const k of ["ovOpenList", "ovOpenSettings", "ovOpenLogs"]) {
      assert.ok(all.fa[k].endsWith("←"), `fa.${k}`);
      for (const locale of ["en", "zh", "fr", "ru"]) {
        assert.ok(all[locale][k].endsWith("→"), `${locale}.${k}`);
      }
    }
  });

  it("RTL locales mirror <html> lang + dir (fa -> rtl, rest -> ltr)", () => {
    const { api, sandbox } = loadApp();
    const el = () => sandbox.document.documentElement;
    api.selectLang("ir");
    assert.equal(el().lang, "fa");
    assert.equal(el().dir, "rtl");
    for (const code of ["cn", "fr", "ru", "en"]) {
      api.selectLang(code);
      assert.equal(el().dir, "ltr", `selectLang(${code})`);
    }
    assert.equal(el().lang, "en");
  });

  it("language menu toggles and every row switches for real", () => {
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
    // Every menu code switches language, persists it, and closes the menu.
    const expectLang = { en: "Refresh", ir: "به‌روزرسانی", cn: "刷新", fr: "Actualiser", ru: "Обновить" };
    for (const [code, label] of Object.entries(expectLang)) {
      api.toggleLangMenu();
      api.selectLang(code);
      assert.equal(api.t("btnRefresh"), label, `selectLang(${code})`);
      assert.equal(menu.hidden, true);
    }
    assert.equal(sandbox.localStorage.getItem("v2raydar-lang"), "ru");
    // Switching repaints dynamic sections from cache (share rows, status).
    api.selectLang("ir");
    const text = (id) => sandbox.__elements.get(id).textContent;
    assert.equal(text("share-hint"), api.t("shareHintLocal"));
    assert.equal(text("status-text"), "در حال اتصال…"); // boot feed re-derived
    api.selectLang("en");
    assert.equal(text("share-hint"), api.t("shareHintLocal"));
    assert.equal(text("status-text"), "Connecting…");
    api.selectLang("en");
    // Unknown codes are ignored silently.
    api.selectLang("xx");
    assert.equal(api.t("btnRefresh"), "Refresh");
    // Menu rows cover every LANG in order (EN, IR, CN, FR, RU).
    const html = fs.readFileSync(path.join(FRONTEND_DIR, "index.html"), "utf8");
    const rows = [...html.matchAll(/<button[^>]*data-lang="([a-z]+)"/g)].map((m) => m[1]);
    assert.deepEqual(rows, ["en", "ir", "cn", "fr", "ru"]);
    // The badge pre-loads one flag per LANG (offline-proof switch).
    const badge = [...html.matchAll(/<img[^>]*data-lang="([a-z]+)"/g)].map((m) => m[1]);
    assert.deepEqual(badge, ["en", "ir", "cn", "fr", "ru"]);
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
