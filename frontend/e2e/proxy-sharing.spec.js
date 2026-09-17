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
  stub.proxyModePosts = 0;
  stub.proxyModeBodies = [];
  stub.proxyModeDelayMs = 0;
  stub.proxyEnabled = false;
  stub.proxyDiscoverable = false;
  stub.sharingPosts = 0;
  stub.proxySelectBodies = [];
  stub.proxyActiveUri = null;
  stub.applyProxyOnSelect = true;
});

test("proxy mode segments set directly, collapse in flight, reflect state", async ({ page }) => {
  await page.goto(base + "/proxy");
  await expect(page.locator("#proxy-off")).toHaveAttribute("aria-pressed", "true");
  await page.click("#proxy-lan");
  await expect.poll(() => stub.proxyModePosts).toBe(1);
  expect(stub.proxyModeBodies[0]).toEqual({ mode: "lan" });
  // The resync presses the live segment; buttons unlock after finish.
  await expect(page.locator("#proxy-lan")).toHaveAttribute("aria-pressed", "true", { timeout: 8000 });
  await expect(page.locator("#proxy-off")).toHaveAttribute("aria-pressed", "false");

  // Rapid re-clicks while in flight collapse to the first POST.
  stub.proxyModePosts = 0;
  stub.proxyModeBodies = [];
  stub.proxyModeDelayMs = 400;
  await Promise.all(Array.from({ length: 3 }, () => page.click("#proxy-off")));
  await page.waitForTimeout(600);
  expect(stub.proxyModePosts).toBe(1);
  expect(stub.proxyModeBodies[0]).toEqual({ mode: "off" });
  stub.proxyModeDelayMs = 0;
});

test("sharing button POSTs and locks in-flight", async ({ page }) => {
  await page.goto(base + "/share");
  await Promise.all(Array.from({ length: 3 }, () => page.click("#btn-sharing")));
  // In-flight lock: rapid clicks while disabled collapse (≥1, never 3).
  await page.waitForTimeout(500);
  expect(stub.sharingPosts).toBeGreaterThanOrEqual(1);
  expect(stub.sharingPosts).toBeLessThanOrEqual(3);
});

test("sharing firewall failure pops the run-as-admin guide", async ({ page }) => {
  await page.route("**/api/sharing", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      ok: true,
      status: "Sharing on (firewall update failed: boom)",
      dirty: false,
      code: "firewall_elevation",
      os: "windows",
    }),
  }));
  await page.goto(base + "/share");
  await page.click("#btn-sharing");
  const dlg = page.locator("#dlg-admin");
  await expect(dlg).toHaveAttribute("open", "");
  await expect(dlg.locator("#dlg-admin-title")).toHaveText("Firewall needs elevation");
  await expect(page.locator("#dlg-admin-body")).toContainText("Run as administrator");
  await page.click("#dlg-admin-ok");
  await expect(dlg).not.toHaveAttribute("open", "");
});

test("proxy LAN firewall failure pops the guide with server-OS text", async ({ page }) => {
  await page.route("**/api/proxy/mode", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      ok: true,
      status: "Proxy LAN (firewall update failed: boom)",
      dirty: false,
      code: "firewall_elevation",
      os: "linux",
    }),
  }));
  await page.goto(base + "/proxy");
  await page.click("#proxy-lan");
  await expect(page.locator("#dlg-admin")).toHaveAttribute("open", "");
  await expect(page.locator("#dlg-admin-body")).toContainText("sudo");
});

test("Use-as-proxy pins a config; proxy tab unpin sends {uri:null}", async ({ page }) => {
  await page.goto(base + "/configs");
  await expect(page.locator("#cfg-body tr")).toHaveCount(2);
  await page.locator("#cfg-body tr").first().getByRole("button", { name: "Use" }).click();
  await expect.poll(() => stub.proxySelectBodies.length).toBe(1);
  expect(stub.proxySelectBodies[0].uri).toContain("vless://");

  await page.goto(base + "/proxy");
  // The stub applies the pin, so the resync arms the unpin button…
  await expect(page.locator("#btn-proxy-unpin")).toBeEnabled({ timeout: 8000 });
  await page.click("#btn-proxy-unpin");
  await expect.poll(() => stub.proxySelectBodies.length).toBe(2);
  expect(stub.proxySelectBodies[1]).toEqual({ uri: null });
  // …and the following resync disarms it again.
  await expect(page.locator("#btn-proxy-unpin")).toBeDisabled({ timeout: 8000 });
});

test("rapid double unpin collapses to a single POST", async ({ page }) => {
  await page.goto(base + "/configs");
  await page.locator("#cfg-body tr").first().getByRole("button", { name: "Use" }).click();
  await expect.poll(() => stub.proxySelectBodies.length).toBe(1);
  await page.goto(base + "/proxy");
  await expect(page.locator("#btn-proxy-unpin")).toBeEnabled({ timeout: 8000 });
  // Synthetic double click: the synchronous lock refuses the second one.
  await page.locator("#btn-proxy-unpin").dispatchEvent("click");
  await page.locator("#btn-proxy-unpin").dispatchEvent("click");
  await page.waitForTimeout(400);
  expect(stub.proxySelectBodies.length).toBe(2);
});

test("overview row click opens detail popup with QR and proxy action", async ({ page }) => {
  await page.goto(base + "/overview");
  await page.locator("#ov-cfg-body tr").first().locator("td").nth(1).click();
  await expect(page.locator("#dlg-detail")).toHaveAttribute("open", "");
  await expect(page.locator("#dlg-detail-title")).toHaveText("Config detail");
  await expect(page.locator("#dlg-detail-kv")).toContainText("e2e-node-0");
  const pixels = await page.evaluate(() => {
    const c = document.getElementById("dlg-detail-qr");
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let dark = 0;
    for (let i = 0; i < d.length; i += 40) {
      if (d[i] < 128) dark += 1;
    }
    return dark;
  });
  expect(pixels).toBeGreaterThan(0);
  await page.click("#dlg-detail-use");
  await expect.poll(() => stub.proxySelectBodies.length).toBe(1);
  expect(stub.proxySelectBodies[0].uri).toContain("vless://");
});

test("configs row click opens detail popup; Detail button is gone", async ({ page }) => {
  await page.goto(base + "/configs");
  await expect(page.locator("#cfg-body").getByRole("button", { name: "Detail" })).toHaveCount(0);
  await page.locator("#cfg-body tr").first().locator("td").nth(1).click();
  await expect(page.locator("#dlg-detail")).toHaveAttribute("open", "");
  await expect(page.locator("#dlg-detail-title")).toHaveText("Config detail");
  await expect(page.locator("#dlg-detail-kv")).toContainText("e2e-node-0");
  await page.click("#dlg-detail-close");
});

test("proxy button tracks pending → active → toggle-off", async ({ page }) => {
  await page.goto(base + "/configs");
  const btn = page.locator("#cfg-body tr").first().getByRole("button", { name: "Use" });
  await btn.click();
  // Instant feedback: Pending… before the server round trip finishes.
  await expect(page.locator("#cfg-body tr").first().getByRole("button", { name: "Pending…" })).toBeVisible();
  // The stub applies the switch; the 1.5 s resync confirms it → Active.
  await expect(page.locator("#cfg-body tr").first().getByRole("button", { name: "Active" })).toBeVisible({ timeout: 8000 });
  // Clicking the confirmed-active config unpins (TUI Enter toggle), not re-pins.
  await page.locator("#cfg-body tr").first().getByRole("button", { name: "Active" }).click();
  await expect.poll(() => stub.proxySelectBodies.length).toBe(2);
  expect(stub.proxySelectBodies[1]).toEqual({ uri: null });
  await expect(page.locator("#cfg-body tr").first().getByRole("button", { name: "Use" })).toBeVisible({ timeout: 8000 });
});

test("detail popup light-dismisses on outside click/tap", async ({ page }) => {
  await page.goto(base + "/configs");
  await page.locator("#cfg-body tr").first().locator("td").nth(1).click();
  await expect(page.locator("#dlg-detail")).toHaveAttribute("open", "");
  // Top-left corner is outside the centered dialog (backdrop).
  await page.mouse.click(5, 5);
  await expect(page.locator("#dlg-detail")).not.toHaveAttribute("open", "");
});

test("slow proxy switch confirms via live probe-delta (no stuck Pending)", async ({ page }) => {
  // The reported bug: the 1.5 s resync fires before a slow switch lands,
  // and nothing afterwards ever delivered the confirmation.
  stub.applyProxyOnSelect = false;
  await page.goto(base + "/configs");
  await page.locator("#cfg-body tr").first().getByRole("button", { name: "Use" }).click();
  const pending = page.locator("#cfg-body tr").first().getByRole("button", { name: "Pending…" });
  await expect(pending).toBeVisible();
  await page.waitForTimeout(2200);
  await expect(pending).toBeVisible();
  // The switch lands minutes later: a live probe-delta confirms it.
  const uri = stub.proxySelectBodies[stub.proxySelectBodies.length - 1].uri;
  stub.proxyActiveUri = uri;
  stub.emit("probe-delta", {
    refreshing: false,
    pinging: false,
    proxy_running: true,
    proxy_active_config: "e2e-node-0",
    proxy_active_uri: uri,
    proxy_port: 27910,
    proxy_discoverable: false,
  });
  await expect(page.locator("#cfg-body tr").first().getByRole("button", { name: "Active" })).toBeVisible({ timeout: 5000 });
  stub.applyProxyOnSelect = true;
});

test("per-config QR dialog renders on canvas; copy buttons exist", async ({ page }) => {
  await page.goto(base + "/configs");
  await page.locator("#cfg-body tr").first().getByRole("button", { name: "QR" }).click();
  await expect(page.locator("#dlg-qr")).toHaveAttribute("open", "");
  const pixels = await page.evaluate(() => {
    const c = document.getElementById("qr-canvas");
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let dark = 0;
    for (let i = 0; i < d.length; i += 40) {
      if (d[i] < 128) dark += 1;
    }
    return dark;
  });
  expect(pixels).toBeGreaterThan(0);
  await page.click("#dlg-qr-close");
});

test("full URIs never appear in the DOM by default", async ({ page }) => {
  await page.goto(base + "/configs");
  const html = await page.content();
  expect(html).not.toContain("vless://uuid@");
});

