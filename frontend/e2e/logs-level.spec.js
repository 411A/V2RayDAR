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

// The Logs tab severity filter: a `<select id="log-level">` (ALL/INFO/WARN/
// ERROR) gates rendered rows by parsed level. The stub hello carries the
// legacy plain line "boot ok", which has no level tag and must render as an
// INFO row — visible under ALL, hidden under ERROR.
test("logs severity filter hides legacy INFO line under ERROR, restores under ALL", async ({ page }) => {
  await page.goto(base + "/logs");

  const level = page.locator("#log-level");
  await expect(level).toBeVisible();
  await expect(level.locator("option")).toHaveCount(4);
  const values = await level.locator("option").evaluateAll((opts) => opts.map((o) => o.value));
  expect(values).toEqual(["ALL", "INFO", "WARN", "ERROR"]);
  // Normalize: the shipped default is INFO — ALL shows the same legacy row.
  await level.selectOption("ALL");

  const row = page.locator("#log-list li", { hasText: "boot ok" });
  await expect(row).toBeVisible();
  const badge = row.locator(".log-lvl.lvl-info");
  await expect(badge).toBeVisible();
  await expect(badge).toHaveText("INFO");

  await level.selectOption("ERROR");
  await expect(row).toBeHidden();
  await expect(page.locator("#log-empty")).toBeVisible();

  // Leave the shipped default behind so no localStorage value leaks out.
  await level.selectOption("INFO");
  await expect(row).toBeVisible();
  await expect(page.locator("#log-empty")).toBeHidden();
});
