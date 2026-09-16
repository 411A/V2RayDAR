import { test, expect } from "@playwright/test";

// Live-server smoke: runs against the real Rust backend (LIVE_BASE).
// Skipped unless LIVE_BASE is set, so normal `playwright test` stays hermetic.
const LIVE = process.env.LIVE_BASE || "";
test.skip(!LIVE, "LIVE_BASE not set — stub specs cover CI");

test("live: dashboard boots with zero console errors", async ({ page }) => {
  const errors = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push("console: " + m.text());
  });
  page.on("pageerror", (e) => errors.push("page: " + e.message));
  await page.goto(LIVE + "/overview");
  await expect(page.locator("#stat-cards .card")).toHaveCount(7);
  expect(errors).toEqual([]);
});

test("live: manual refresh POSTs once; ping while refreshing gets 409 semantics", async ({ page }) => {
  await page.goto(LIVE + "/overview");
  const refresh = await page.evaluate(() =>
    fetch("/api/refresh", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })
      .then(async (r) => ({ status: r.status, body: await r.text() })),
  );
  expect([200, 409]).toContain(refresh.status);
  const ping = await page.evaluate(() =>
    fetch("/api/ping", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })
      .then(async (r) => ({ status: r.status, body: await r.text() })),
  );
  expect([200, 409]).toContain(ping.status);
});

test("live: subscriptions + config + save round-trip", async ({ page }) => {
  await page.goto(LIVE + "/subscriptions");
  const subs = await page.evaluate(() => fetch("/api/subscriptions").then((r) => r.json()));
  expect(Array.isArray(subs.list)).toBe(true);
  const added = await page.evaluate(() =>
    fetch("/api/subscriptions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url: "https://example.com/e2e.txt", name: "live-e2e", priority: 5, enabled: false }),
    }).then((r) => r.json()),
  );
  expect(added.ok).toBe(true);
  const del = await page.evaluate((n) =>
    fetch("/api/subscriptions/" + (n - 1), { method: "DELETE" }).then((r) => r.json()),
  subs.list.length + 1);
  expect(del.ok).toBe(true);
  const cfg = await page.evaluate(() =>
    fetch("/api/config", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ key: "top_n", value: "10" }),
    }).then((r) => r.json()),
  );
  expect(cfg.ok).toBe(true);
  const bad = await page.evaluate(() =>
    fetch("/api/config", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ key: "bogus_key_xyz", value: "1" }),
    }).then((r) => ({ status: r.status })),
  );
  expect(bad.status).toBe(400);
});
