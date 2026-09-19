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

test("country cell shows the config's own flag, not a disagreeing GeoIP code", async ({ page }) => {
  // Server rule (geoip.rs format_display_name): the name's own flag always
  // wins — the Country badge must agree with the Name cell, not the code.
  // A bare two-letter name ("NL") is a flagless country code: flag up front,
  // provider text kept.
  const row = (name, cc) => ({
    rank: 1,
    stability_count: 0,
    id: `vless://flagged-${cc || "none"}.example.com:443`,
    dedup_key: `vless://flagged-${cc || "none"}.example.com:443`,
    source: "e2e",
    priority: 100,
    protocol: "vless",
    name,
    endpoint: { host: "flagged.example.com", port: 443 },
    uri: "vless://uuid@flagged.example.com:443#flagged-node",
    reachable: true,
    validation: "active_http",
    latency_ms: 10,
    http_status: 204,
    download_mbps: null,
    download_bytes: null,
    error: null,
    country_code: cc,
  });
  stub.rankedPush = [row("🇳🇱 flagged-node", "DE"), row("NL", null)];
  await page.goto(base + "/configs");
  // The pushed `ranked` event replaces the two seed rows (~300 ms after
  // hello, like the overview test above).
  await expect(page.locator("#cfg-body tr")).toHaveCount(2, { timeout: 5000 });
  const flagged = page.locator("#cfg-body tr").nth(0).locator("td[data-th='Country']");
  await expect(flagged).toHaveText("🇳🇱");
  await expect(flagged.locator("span")).toHaveAttribute("title", "NL");
  const bare = page.locator("#cfg-body tr").nth(1);
  await expect(bare.locator("td.cell-main strong")).toHaveText("🇳🇱 NL");
  const bareCountry = bare.locator("td[data-th='Country']");
  await expect(bareCountry).toHaveText("🇳🇱");
  await expect(bareCountry.locator("span")).toHaveAttribute("title", "NL");
  stub.rankedPush = null;
});
