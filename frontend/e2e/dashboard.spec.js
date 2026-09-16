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
  // Every sub-line sits on the same bottom baseline (flex-pinned).
  const bottoms = await page.$$eval("#stat-cards .stat-sub", (subs) =>
    subs.map((s) => Math.round(s.getBoundingClientRect().bottom)),
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

test("language flag sits left of ? and announces English-only", async ({ page }) => {
  await page.goto(base + "/overview");
  const flag = page.locator("#btn-lang");
  const keys = page.locator("#btn-keys");
  await expect(flag).toBeVisible();
  // Inlined GB mark (vector, no external asset) with an English label.
  expect(await flag.locator("svg").count()).toBe(1);
  await expect(flag).toHaveAttribute("aria-label", "Language: English");
  // Immediately left of the ? button in the same action row.
  const order = await page.evaluate(() => {
    const f = document.getElementById("btn-lang").getBoundingClientRect();
    const k = document.getElementById("btn-keys").getBoundingClientRect();
    return { flagRight: f.right, keysLeft: k.left, sameRow: Math.abs(f.top - k.top) < 4 };
  });
  expect(order.sameRow).toBe(true);
  expect(order.flagRight).toBeLessThanOrEqual(order.keysLeft);
  // Single language for now: clicking says so instead of switching.
  await flag.click();
  await expect(page.locator("#toasts .toast").last()).toContainText("English is the only language");
});

test("legacy #/tab bookmarks replace-redirect with token preserved", async ({ page }) => {
  await page.goto(base + "/overview?token=abc#/configs");
  // Boot reads the hash and replace-redirects to the clean path (token kept).
  await expect.poll(() => page.url()).toContain("/configs?token=abc");
  await expect(page.locator("#panel-configs")).toBeVisible();
});
