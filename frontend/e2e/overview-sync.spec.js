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

// Regression: a live `ranked` event used to refresh the Configs tab only,
// leaving Overview "Top configs" stuck on "No working configs yet" after a
// manual refresh/ping delivered new rows mid-cycle.
test("overview top-configs follows live ranked events like the configs tab", async ({ page }) => {
  stub.rankedPush = [0, 1, 2].map((i) => ({
    rank: i + 1,
    stability_count: 3,
    id: `vless://late${i}.example.com:443`,
    dedup_key: `vless://late${i}.example.com:443`,
    source: "e2e",
    priority: 100,
    protocol: "vless",
    name: `late-node-${i}`,
    endpoint: { host: "late.example.com", port: 443 + i },
    uri: `vless://uuid@late.example.com:${443 + i}?security=tls#late-node-${i}`,
    reachable: true,
    validation: "active_http",
    latency_ms: 50 + i,
    http_status: 204,
    download_mbps: null,
    download_bytes: null,
    error: null,
    country_code: "DE",
  }));
  await page.goto(base + "/overview");
  await expect(page.locator("#ov-cfg-body tr")).toHaveCount(2);
  // The pushed `ranked` event lands ~300 ms after hello: both lists move to 3.
  await expect(page.locator("#ov-cfg-body tr")).toHaveCount(3, { timeout: 5000 });
  await expect(page.locator("#ov-cfg-empty")).toBeHidden();
  await page.click("#tab-configs");
  await expect(page.locator("#cfg-body tr")).toHaveCount(3);
  stub.rankedPush = null;
});
