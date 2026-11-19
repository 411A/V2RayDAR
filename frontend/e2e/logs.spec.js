import { test, expect } from "@playwright/test";
import { createStub } from "./stub-server.mjs";

let stub;
let server;
let base;

test.beforeAll(async () => {
  ({ stub, server } = createStub());
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  base = `http://127.0.0.1:${server.address().port}`;
  // A session longer than both the live tail and the client render cap:
  // the oldest lines live only in the archive the "Load older" button pages.
  const lines = (count, start = 1) =>
    Array.from({ length: count }, (_, i) => {
      const seq = start + i;
      return {
        seq,
        line: `2026/10/25 13:20:59.324 [INFO] archived line ${String(seq).padStart(6, "0")}`,
      };
    });
  stub.logSeq = 1000;
  stub.archiveLogs = lines(1000);
  stub.liveLogs = stub.archiveLogs.slice(-2).map((row) => row.line);
});

test.afterAll(async () => {
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
});

test("load older reaches the first line of the session", async ({ page }) => {
  await page.goto(base + "/logs");
  const rows = page.locator("#log-list li");
  await expect(rows).toHaveCount(2);
  const older = page.getByRole("button", { name: "Load older" });
  await expect(older).toBeEnabled();

  await older.click();
  await expect(rows).toHaveCount(502);
  await expect(page.locator("#log-list")).toContainText("archived line 000499");
  await expect(page.locator("#log-list")).toContainText("archived line 001000");
  await expect(older).toBeEnabled();

  await older.click();
  await expect(rows).toHaveCount(1000);
  await expect(page.locator("#log-list")).toContainText("archived line 000001");
  await expect(older).toBeDisabled();
});

test("download offers the complete session log", async ({ page }) => {
  await page.goto(base + "/logs");
  const downloadPromise = page.waitForEvent("download");
  await page.locator("#btn-log-download").click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe("v2raydar-session.log");
});
