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

// Regression: a two-digit rank wrapped mid-number ("1"/"4" on separate
// lines) on narrow screens — `.cell-text` allows breaks anywhere, which is
// right for long values but must never split a number.
test("rank digits never split across lines, even on a 360px phone", async ({ page }) => {
  const row = (i) => ({
    rank: i + 1,
    stability_count: 12,
    id: `vless://wrap${i}.example.com:443`,
    dedup_key: `vless://wrap${i}.example.com:443`,
    source: "src-16",
    priority: 100,
    protocol: "vless",
    // The exact provider remark from the report: pipes and emoji are data.
    name: i === 13 ? "SE Sweden Stockholm |\u23F1395ms |\u26A1826KB/s | @NamazVPN" : `wrap-node-${i}`,
    endpoint: { host: "wrap.example.com", port: 443 + i },
    uri: `vless://uuid@wrap.example.com:${443 + i}?security=tls#wrap-node-${i}`,
    reachable: true,
    validation: "active_http",
    latency_ms: 395,
    http_status: 204,
    download_mbps: null,
    download_bytes: null,
    error: null,
    country_code: "SE",
  });
  stub.rankedPush = Array.from({ length: 14 }, (_, i) => row(i));
  await page.setViewportSize({ width: 360, height: 740 });
  await page.goto(base + "/configs");
  await expect(page.locator("#cfg-body tr")).toHaveCount(14, { timeout: 5000 });
  // Every numeric cell is pinned to one line by construction.
  const nums = await page.$$eval("#cfg-body tr td.cell-num .cell-text", (els) =>
    els.map((el) => ({
      text: el.textContent,
      ws: getComputedStyle(el).whiteSpace,
      single: el.scrollHeight <= el.clientHeight + 1,
    })),
  );
  expect(nums.length).toBeGreaterThan(0);
  for (const n of nums) {
    expect(n.ws, `white-space of ${n.text}`).toBe("nowrap");
    expect(n.single, `single line: ${n.text}`).toBe(true);
  }
  // The reported row itself: rank reads "14" on exactly one line.
  const rank14 = page.locator("#cfg-body tr").nth(13).locator("td.cell-num .cell-text").first();
  await expect(rank14).toHaveText("14");
  stub.rankedPush = null;
});
