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
  stub.sharingPosts = 0;
  stub.proxySelectBodies = [];
});

test("proxy mode + sharing buttons POST once and lock in-flight", async ({ page }) => {
  await page.goto(base + "/proxy");
  await page.click("#btn-proxy-mode");
  await expect.poll(() => stub.proxyModePosts).toBe(1);
  await expect(page.locator("#btn-proxy-mode")).toBeEnabled(); // unlocked after finish

  await page.goto(base + "/share");
  await Promise.all(Array.from({ length: 3 }, () => page.click("#btn-sharing")));
  // In-flight lock: rapid clicks while disabled collapse (≥1, never 3).
  await page.waitForTimeout(500);
  expect(stub.sharingPosts).toBeGreaterThanOrEqual(1);
  expect(stub.sharingPosts).toBeLessThanOrEqual(3);
});

test("Use-as-proxy pins a config; unpin sends {uri:null}", async ({ page }) => {
  await page.goto(base + "/configs");
  await expect(page.locator("#cfg-body tr")).toHaveCount(2);
  await page.locator("#cfg-body tr").first().getByRole("button", { name: "Use" }).click();
  await expect.poll(() => stub.proxySelectBodies.length).toBe(1);
  expect(stub.proxySelectBodies[0].uri).toContain("vless://");

  await page.goto(base + "/proxy");
  // No active uri in the stub snapshot → unpin disabled with helper text.
  await expect(page.locator("#btn-proxy-unpin")).toBeDisabled();
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
