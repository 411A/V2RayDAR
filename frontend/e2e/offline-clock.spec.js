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
