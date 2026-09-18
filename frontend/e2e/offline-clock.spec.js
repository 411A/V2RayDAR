import { test, expect } from "@playwright/test";
import { createStub } from "./stub-server.mjs";

// Own origin server: this file kills it mid-test to prove the Running For
// clock freezes while offline (other specs share one server per file and
// must stay up).
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

test("running-for pauses while the server is unreachable", async ({ page }) => {
  await page.goto(base + "/overview");
  await expect(page.locator("#stat-cards .card")).toHaveCount(7);
  const before = await page.locator("#stat-running").textContent();
  expect(before).toMatch(/^\d\d:\d\d:\d\d$/);

  // Hard-drop the origin (plain close() would leave the SSE socket hanging):
  // EventSource errors, the reconnect probe fails, feed goes offline.
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
  await expect(page.locator("#banner")).toBeVisible({ timeout: 15000 });

  // Two-plus wall-clock ticks pass with the banner up; the badge must not move.
  await page.waitForTimeout(2300);
  const after = await page.locator("#stat-running").textContent();
  expect(after).toBe(before);
});

test("language badge follows the switch while offline", async ({ page }) => {
  await page.goto(base + "/overview");
  const badge = page.locator("#btn-lang img:not([hidden])");
  await expect(badge).toHaveAttribute("src", "./assets/GB.svg");
  // Hard-drop the origin like the clock test above.
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
  await expect(page.locator("#banner")).toBeVisible({ timeout: 15000 });
  // Switch offline: the badge follows with zero network (decoded bitmap,
  // real size) and the menu rows keep their own flags.
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="ir"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "fa");
  await expect(badge).toHaveAttribute("src", "./assets/IR.svg");
  const ok = await badge.evaluate((img) => img.complete && img.naturalWidth > 0);
  expect(ok).toBe(true);
  await expect(page.locator('#lang-menu button[data-lang="en"] img')).toHaveAttribute("src", "./assets/GB.svg");
  await expect(page.locator('#lang-menu button[data-lang="ir"] img')).toHaveAttribute("src", "./assets/IR.svg");
  // The reported scramble: Persian on, pick Chinese — every row must show
  // its own flag, badge included.
  await page.locator("#btn-lang").click();
  await page.locator('#lang-menu button[data-lang="cn"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh");
  await expect(badge).toHaveAttribute("src", "./assets/CN.svg");
  for (const [code, file] of [["en", "GB"], ["ir", "IR"], ["cn", "CN"], ["fr", "FR"], ["ru", "RU"]]) {
    await expect(page.locator(`#lang-menu button[data-lang="${code}"] img`)).toHaveAttribute("src", `./assets/${file}.svg`);
  }
});
