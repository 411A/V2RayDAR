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

test("returning from a hidden tab reconnects the live feed instead of replaying it", async ({ page }) => {
  await page.goto(base + "/overview");
  await expect(page.locator("#stat-cards .card")).toHaveCount(7);
  stub.resultsGets = 0;
  stub.eventsGets = 0;
  // Simulate minimize-then-restore: hidden dispatches must not fetch,
  // the return to visible opens a fresh feed whose hello snapshot carries
  // the latest state, so the dashboard never replays the backlog through
  // /results deltas.
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { value: true, configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(300);
  expect(stub.resultsGets).toBe(0);
  expect(stub.eventsGets).toBe(0);
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { value: false, configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect.poll(() => stub.eventsGets, { timeout: 5000 }).toBeGreaterThanOrEqual(1);
  expect(stub.resultsGets).toBe(0);
  await expect(page.locator("#stat-cards .card")).toHaveCount(7);
});
