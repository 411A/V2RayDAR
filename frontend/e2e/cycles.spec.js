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

test.beforeEach(() => {
  stub.refreshPosts = 0;
  stub.pingPosts = 0;
  stub.busyRefreshing = false;
  stub.busyPinging = false;
});

test("rapid Refresh triggers collapse to a single POST (frontend pre-guard)", async ({ page }) => {
  // Slow the response so repeats land while the POST is still in flight;
  // keyboard repeats bypass `disabled`, so only the in-browser guard saves us.
  stub.refreshDelayMs = 400;
  await page.goto(base + "/overview");
  await expect(page.locator("#btn-refresh")).toBeEnabled();
  for (let i = 0; i < 5; i += 1) {
    await page.keyboard.press("r");
  }
  await expect(page.locator("#toasts .toast.good")).toContainText(["Manual refresh started"]);
  await expect(page.locator("#toasts .toast.bad")).toContainText(["Refresh already running"]);
  await page.waitForTimeout(600);
  expect(stub.refreshPosts).toBe(1);
  stub.refreshDelayMs = 0;
});

test("rapid Ping triggers collapse to a single POST", async ({ page }) => {
  stub.pingDelayMs = 400;
  await page.goto(base + "/overview");
  await expect(page.locator("#btn-ping")).toBeEnabled();
  for (let i = 0; i < 5; i += 1) {
    await page.keyboard.press("p");
  }
  await expect(page.locator("#toasts .toast.good")).toContainText(["Manual ping started"]);
  await page.waitForTimeout(600);
  expect(stub.pingPosts).toBe(1);
  stub.pingDelayMs = 0;
});

test("buttons disable while a system cycle runs; clicks send nothing", async ({ page }) => {
  stub.busyRefreshing = true;
  await page.goto(base + "/overview");
  // SSE hello carries refreshing:true, so both buttons lock on render.
  await expect(page.locator("#btn-refresh")).toBeDisabled();
  await expect(page.locator("#btn-ping")).toBeDisabled();
  await expect(page.locator("#btn-refresh")).toHaveAttribute(
    "title",
    "A refresh is already running — wait for it to finish",
  );
  // Disabled buttons cannot be clicked at all — counters stay at zero.
  await expect(stub.refreshPosts).toBe(0);
  await expect(stub.pingPosts).toBe(0);
  // Like the TUI, Last scan reads — until the running cycle finishes.
  await expect(page.locator("#stat-cards .card").nth(2).locator(".stat-value")).toHaveText("—");
});

test("keyboard trigger while busy is refused in-browser with TUI wording", async ({ page }) => {
  stub.busyRefreshing = true;
  await page.goto(base + "/overview");
  await expect(page.locator("#btn-refresh")).toBeDisabled();
  // The `r` shortcut calls triggerCycle directly (bypasses `disabled`);
  // the guard must still refuse without any network request.
  await page.keyboard.press("r");
  await expect(page.locator("#toasts .toast.bad")).toContainText(["Refresh already running"]);
  await expect(stub.refreshPosts).toBe(0);
});

test("keyboard shortcuts r/p trigger exactly once each", async ({ page }) => {
  await page.goto(base + "/overview");
  await page.keyboard.press("r");
  await expect.poll(() => stub.refreshPosts).toBe(1);
  // The successful refresh optimistically marks the snapshot refreshing
  // (resynced ~1.2 s later): pressing `p` before that is correctly refused,
  // so wait for the resync before the ping shortcut.
  await page.waitForTimeout(1600);
  await page.keyboard.press("p");
  await expect.poll(() => stub.pingPosts).toBe(1);
});

test("Fetched badge follows live probe-delta totals without a reload", async ({ page }) => {
  await page.goto(base + "/overview");
  // Stub hello reports 8 fetched; a fetch-phase delta lands while
  // tested/working hold still — the badge must follow it live.
  await expect(page.locator("#stat-fetched .stat-value")).toHaveText("8");
  stub.emit("probe-delta", { tested: 8, working: 2, total: 42, refreshing: false, pinging: false });
  await expect(page.locator("#stat-fetched .stat-value")).toHaveText("42");
});
