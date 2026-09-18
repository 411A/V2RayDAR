import { test, expect } from "@playwright/test";
import { createStub } from "./stub-server.mjs";

let stub;
let server;
let base;

test.beforeAll(async () => {
  ({ stub, server } = createStub());
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  base = `http://127.0.0.1:${server.address().port}`;
});

test.afterAll(async () => {
  await new Promise((r) => server.close(r));
});

test("dashboard boots with zero console/page/network errors", async ({ page }) => {
  const errors = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push("console: " + m.text());
  });
  page.on("pageerror", (e) => errors.push("page: " + e.message));
  page.on("response", (r) => {
    if (r.status() >= 500) errors.push(`net ${r.status()} ${r.url()}`);
  });
  await page.goto(base + "/overview");
  await expect(page.locator("#stat-cards .card")).toHaveCount(7);
  await expect(page.locator("#ov-cfg-body tr")).toHaveCount(2);
  // No badge text may stick out of its card border (subs are one-line
  // ellipsis, values are short clock/count strings).
  const overflows = await page.$$eval("#stat-cards .card", (cards) =>
    cards.map((c) => c.scrollWidth - c.clientWidth),
  );
  expect(overflows.every((d) => d <= 1)).toBe(true);
  // Last scan stacks age above duration on two sub-lines.
  await expect(page.locator('#stat-cards .card').nth(2).locator('.stat-sub')).toHaveCount(2);
  // Every card's last sub-line sits on the same bottom baseline
  // (flex-pinned; multi-line cards only add lines above it).
  const bottoms = await page.$$eval("#stat-cards .card", (cards) =>
    cards
      .map((c) => c.querySelectorAll(".stat-sub"))
      .filter((subs) => subs.length > 0)
      .map((subs) => Math.round(subs[subs.length - 1].getBoundingClientRect().bottom)),
  );
  expect(bottoms.length).toBeGreaterThan(0);
  expect(Math.max(...bottoms) - Math.min(...bottoms)).toBeLessThanOrEqual(1);
  expect(errors).toEqual([]);
});

test("all seven tabs switch instantly with no reload", async ({ page }) => {
  await page.goto(base + "/overview");
  await expect(page.locator("#panel-overview")).toBeVisible();
  const tabs = ["configs", "subscriptions", "settings", "proxy", "logs", "share"];
  for (const t of tabs) {
    await page.click(`#tab-${t}`);
    await expect(page.locator(`#panel-${t}`)).toBeVisible();
    expect(page.url()).toContain("/" + t);
  }
  // Only one history entry per click; panels never overlap.
  await expect(page.locator(".panel:visible")).toHaveCount(1);
});

test("deep link opens the right tab; back button returns", async ({ page }) => {
  await page.goto(base + "/proxy");
  await expect(page.locator("#panel-proxy")).toBeVisible();
  await page.click("#tab-logs");
  await expect(page.locator("#panel-logs")).toBeVisible();
  await page.goBack();
  await expect(page.locator("#panel-proxy")).toBeVisible();
});

test("fetch errors render headline + cause on two lines", async ({ page }) => {
  stub.fetchErrors = [
    "failed to fetch subscription 'src-7':\nHTTP status client error (404 Not Found) for url (https://example.com/gone.txt)",
  ];
  await page.goto(base + "/overview");
  const card = page.locator("#fetch-errors-card");
  await expect(card).toBeVisible();
  const item = page.locator("#fetch-errors li").first();
  await expect(item).toContainText("failed to fetch subscription 'src-7':");
  await expect(item).toContainText("HTTP status client error (404 Not Found)");
  // The embedded newline must survive HTML whitespace collapsing.
  expect(await item.evaluate((el) => getComputedStyle(el).whiteSpace)).toBe("pre-wrap");
  stub.fetchErrors = [];
});

test("language flag sits left of ? with a real file and a 5-flag menu", async ({ page }) => {
  await page.goto(base + "/overview");
  const flag = page.locator("#btn-lang");
  const keys = page.locator("#btn-keys");
  await expect(flag).toBeVisible();
  // Real file (replaceable in frontend/assets/), rendered wide, labelled.
  const img = flag.locator("img.flag-img:not([hidden])");
  expect(await img.getAttribute("src")).toBe("./assets/GB.svg");
  const box = await img.boundingBox();
  expect(box.width).toBeGreaterThan(box.height);
  await expect(flag).toHaveAttribute("aria-label", "Language: English");
  // Immediately left of the ? button in the same action row.
  const order = await page.evaluate(() => {
    const f = document.getElementById("btn-lang").getBoundingClientRect();
    const k = document.getElementById("btn-keys").getBoundingClientRect();
    return { flagRight: f.right, keysLeft: k.left, sameRow: Math.abs(f.top - k.top) < 4 };
  });
  expect(order.sameRow).toBe(true);
  expect(order.flagRight).toBeLessThanOrEqual(order.keysLeft);
  // Menu opens with EN, IR, CN, FR, RU in order, English pressed.
  await flag.click();
  const menu = page.locator("#lang-menu");
  await expect(menu).toBeVisible();
  // Dropdown hangs off the flag button itself (overlaps it horizontally).
  const underButton = () =>
    page.evaluate(() => {
      const b = document.getElementById("btn-lang").getBoundingClientRect();
      const m = document.getElementById("lang-menu").getBoundingClientRect();
      return m.left <= b.right && m.right >= b.left;
    });
  expect(await underButton()).toBe(true);
  const codes = await menu.locator("button[data-lang]").evaluateAll((els) =>
    els.map((el) => el.getAttribute("data-lang")),
  );
  expect(codes).toEqual(["en", "ir", "cn", "fr", "ru"]);
  await expect(menu.locator('button[data-lang="en"]')).toHaveAttribute("aria-checked", "true");
  // Every row loads its flag file with no distortion (cover-crop, not stretch).
  for (const c of ["GB", "IR", "CN", "FR", "RU"]) {
    const r = await page.request.get(`${base}/assets/${c}.svg`);
    expect(r.status()).toBe(200);
    expect(r.headers()["content-type"]).toContain("image/svg+xml");
  }
  const fit = await menu.locator("img.flag-img").first().evaluate((el) => getComputedStyle(el).objectFit);
  expect(fit).toBe("cover");
  // Native language names, no country codes.
  const names = await menu.locator(".lang-name").evaluateAll((els) => els.map((el) => el.textContent));
  expect(names).toEqual(["English", "فارسی", "中文", "Français", "Русский"]);
  expect(await menu.locator(".lang-code").count()).toBe(0);
  // Glassy but opaque: page content must not bleed through the menu.
  const alpha = await menu.evaluate((el) => {
    const bg = getComputedStyle(el).backgroundColor;
    const m = bg.match(/[\d.]+(?=\))/g);
    return m ? Number(m[m.length - 1]) : 0;
  });
  expect(alpha).toBeGreaterThanOrEqual(0.8);
  // Picking a language switches the whole shell for real and persists.
  await menu.locator('button[data-lang="ir"]').click();
  await expect(menu).toBeHidden();
  await expect(page.locator("html")).toHaveAttribute("lang", "fa");
  await expect(page.locator("html")).toHaveAttribute("dir", "rtl");
  await expect(img).toHaveAttribute("src", "./assets/IR.svg");
  await expect(page.locator("#btn-refresh")).toContainText("به‌روزرسانی");
  await expect(page.locator("#tab-configs")).toContainText("کانفیگ‌ها");
  await expect(menu.locator('button[data-lang="ir"]')).toHaveAttribute("aria-checked", "true");
  // Dynamic sections repaint immediately: share rows in Persian, no reload.
  await page.locator("#tab-share").click();
  const firstRow = page.locator("#share-list li").first();
  await expect(firstRow.locator("strong")).toHaveText("اشتراک (بیس64)");
  await expect(firstRow.locator("button")).toHaveText("کپی");
  await expect(page.locator("#share-hint")).not.toBeEmpty();
  // URLs stay left-to-right inside RTL text.
  await expect(firstRow.locator("code")).toHaveCSS("direction", "ltr");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("lang", "fa");
  await expect(img).toHaveAttribute("src", "./assets/IR.svg");
  await expect(page.locator("#btn-refresh")).toContainText("به‌روزرسانی");
  await expect(page.locator('#cfg-limit option[value="all"]')).toHaveText("همه");
  // Setting guides are translated: Persian rows read right-to-left now
  // (the server English fallback stays left-to-right).
  await page.locator("#tab-settings").click();
  await expect(page.locator("#settings-groups .guide").first()).toHaveCSS("direction", "rtl");
  await expect(page.locator("#settings-groups .set-name").first()).toHaveText("آدرس گوش دادن");
  // Back to English for the rest of the suite.
  await flag.click();
  await expect(menu).toBeVisible();
  // Still anchored under the button in RTL heartland.
  expect(await underButton()).toBe(true);
  await menu.locator('button[data-lang="en"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.locator("html")).toHaveAttribute("dir", "ltr");
  await expect(img).toHaveAttribute("src", "./assets/GB.svg");
  await expect(page.locator("#btn-refresh")).toContainText("Refresh");
  // Unknown asset names 404 (whitelist, no traversal).
  const bad = await page.request.get(`${base}/assets/EVIL.svg`);
  expect(bad.status()).toBe(404);
});

test("power button sits rightmost; confirm stops the server", async ({ page }) => {
  await page.goto(base + "/overview");
  const power = page.locator("#btn-power");
  await expect(power).toBeVisible();
  // Rightmost in the top bar: at/after the ? button, same row.
  const order = await page.evaluate(() => {
    const p = document.getElementById("btn-power").getBoundingClientRect();
    const k = document.getElementById("btn-keys").getBoundingClientRect();
    return { powerLeft: p.left, keysRight: k.right, sameRow: Math.abs(p.top - k.top) < 4 };
  });
  expect(order.sameRow).toBe(true);
  expect(order.powerLeft).toBeGreaterThanOrEqual(order.keysRight - 1);
  // Vendored svgrepo file, served square and theme-aware.
  expect(await power.locator("img.power-img").getAttribute("src")).toBe("./assets/power-off-svgrepo-com.svg");
  const r = await page.request.get(`${base}/assets/power-off-svgrepo-com.svg`);
  expect(r.status()).toBe(200);
  expect(r.headers()["content-type"]).toContain("image/svg+xml");
  // Confirm dialog warns with a red stop button; cancel keeps running.
  await power.click();
  const dlg = page.locator("#dlg-power");
  await expect(dlg).toHaveAttribute("open", "");
  await expect(dlg.locator("#dlg-power-title")).toHaveText("Stop the server?");
  await expect(dlg.locator("#dlg-power-ok")).toHaveText("Yes, stop");
  await expect(dlg.locator("#dlg-power-ok")).toHaveClass(/danger/);
  await dlg.locator("#dlg-power-cancel").click();
  await expect(dlg).not.toHaveAttribute("open", "");
  expect(stub.shutdownPosts).toBe(0);
  // Confirm stops: POST sent, stopped screen pins.
  await power.click();
  await dlg.locator("#dlg-power-ok").click();
  await expect.poll(() => stub.shutdownPosts).toBe(1);
  await expect(page.locator("#banner")).toBeVisible();
  await expect(page.locator("#banner-title")).toHaveText("Server stopped");
});

test("legacy #/tab bookmarks replace-redirect with token preserved", async ({ page }) => {
  await page.goto(base + "/overview?token=abc#/configs");
  // Boot reads the hash and replace-redirects to the clean path (token kept).
  await expect.poll(() => page.url()).toContain("/configs?token=abc");
  await expect(page.locator("#panel-configs")).toBeVisible();
});

test("top-bar controls share one height; overview sections share one gap", async ({ page }) => {
  await page.goto(base + "/overview");
  // Equal heights, natural widths: icon/segmented controls must not sit
  // shorter than the text buttons.
  const heights = await page.evaluate(() =>
    [...document.querySelectorAll("#btn-refresh, #btn-ping, .theme-switch, #btn-lang, #btn-keys, #btn-power")]
      .map((el) => el.getBoundingClientRect().height)
  );
  expect(heights).toHaveLength(6);
  expect(Math.max(...heights) - Math.min(...heights)).toBeLessThanOrEqual(1);
  // One rhythm below the stat cards, between the two-col rows, and above
  // the fetch-errors card (computed margins match even while hidden).
  const margins = await page.evaluate(() => {
    const css = (sel, prop) => getComputedStyle(document.querySelector(sel))[prop];
    return [
      css("#stat-cards", "marginBottom"),
      css("#panel-overview .two-col", "marginTop"),
      css("#fetch-errors-card", "marginTop"),
    ];
  });
  expect(new Set(margins).size).toBe(1);
});

test("keys dialog ends with the GitHub support line (localized, RTL-safe)", async ({ page }) => {
  await page.goto(base + "/overview");
  await page.locator("#btn-keys").click();
  const dlg = page.locator("#dlg-keys");
  await expect(dlg).toBeVisible();
  const support = dlg.locator(".keys-support");
  await expect(support).toContainText("More help on");
  await expect(support).toContainText("⭐️");
  const link = support.locator("a[data-i18n-href]");
  await expect(link).toHaveAttribute("href", "https://github.com/411A/V2RayDAR");
  await expect(link).toHaveAttribute("target", "_blank");
  await expect(link.locator("strong")).toHaveText("GitHub");
  // The star links the repo root in every language (starring happens there).
  const star = support.locator("a.keys-star");
  await expect(star).toHaveAttribute("href", "https://github.com/411A/V2RayDAR");
  await expect(star).toHaveAttribute("target", "_blank");
  await expect(star).toHaveText("⭐️");
  await expect(star).toHaveAttribute("aria-label", "Star V2RayDAR on GitHub");
  await expect(support).toHaveCSS("direction", "ltr");
  // Full sentence on one line: no lone star stranded below.
  const singleLine = (el) => {
    const cs = getComputedStyle(el);
    return el.getBoundingClientRect().height <= parseFloat(cs.lineHeight) + 1;
  };
  expect(await support.evaluate(singleLine)).toBe(true);
  // French (the reported wrap): same line, own help page.
  await page.keyboard.press("Escape");
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="fr"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  await page.locator("#btn-keys").click();
  await expect(support).toContainText("Offrez-lui");
  await expect(link).toHaveAttribute("href", "https://github.com/411A/V2RayDAR/blob/main/docs/README.fr.md");
  expect(await support.evaluate(singleLine)).toBe(true);
  // Persian: translated sentence, right-to-left, link intact.
  await page.keyboard.press("Escape");
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="ir"]').click();
  await expect(page.locator("html")).toHaveAttribute("dir", "rtl");
  await page.locator("#btn-keys").click();
  await expect(support).toContainText("حمایت");
  await expect(support).toContainText("حمایتش کنید");
  await expect(star).toHaveText("ستاره ⭐️");
  await expect(support).toHaveCSS("direction", "rtl");
  await expect(link).toHaveAttribute("href", "https://github.com/411A/V2RayDAR/blob/main/docs/README.fa.md");
  await expect(star).toHaveAttribute("href", "https://github.com/411A/V2RayDAR");
  await expect(star).toHaveAttribute("aria-label", "به V2RayDAR در گیت‌هاب ستاره بدهید");
  expect(await support.evaluate(singleLine)).toBe(true);
  // Chinese points at its own help page too.
  await page.keyboard.press("Escape");
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="cn"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh");
  await page.locator("#btn-keys").click();
  await expect(link).toHaveAttribute("href", "https://github.com/411A/V2RayDAR/blob/main/docs/README.zh-CN.md");
  expect(await support.evaluate(singleLine)).toBe(true);
  // Russian: same line, own help page.
  await page.keyboard.press("Escape");
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="ru"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "ru");
  await page.locator("#btn-keys").click();
  await expect(support).toContainText("Поставьте");
  await expect(link).toHaveAttribute("href", "https://github.com/411A/V2RayDAR/blob/main/docs/README.ru.md");
  expect(await support.evaluate(singleLine)).toBe(true);
  // Back to English for the rest of the suite.
  await page.keyboard.press("Escape");
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="en"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
});
