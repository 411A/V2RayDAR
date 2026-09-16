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

test("legacy #/tab bookmarks replace-redirect with token preserved", async ({ page }) => {
  await page.goto(base + "/overview?token=abc#/configs");
  // Boot reads the hash and replace-redirects to the clean path (token kept).
  await expect.poll(() => page.url()).toContain("/configs?token=abc");
  await expect(page.locator("#panel-configs")).toBeVisible();
});
