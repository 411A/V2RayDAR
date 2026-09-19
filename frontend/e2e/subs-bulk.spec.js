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

test("export dialog shows name, priority, url text with a format header", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  await expect(page.locator("#sub-body tr")).toHaveCount(1);
  await page.click("#btn-sub-export");
  await expect(page.locator("#dlg-sub-export")).toHaveAttribute("open", "");
  await expect(page.locator("#dlg-sub-export-title")).toHaveText("Export subscriptions");
  const text = await page.locator("#sub-export-text").inputValue();
  expect(text).toContain("# name, priority, url");
  expect(text).toContain("demo, 100, https://example.com/sub.txt");
  await page.click("#dlg-sub-export button[value='close']");
  await expect(page.locator("#dlg-sub-export")).not.toHaveAttribute("open", "");
});

test("import pastes text, skips the twin, and reports n/m added (x duplicates)", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  await expect(page.locator("#sub-body tr")).toHaveCount(1);
  await page.click("#btn-sub-import");
  await expect(page.locator("#dlg-sub-import")).toHaveAttribute("open", "");
  await page.fill(
    "#sub-import-text",
    "fresh one, 5, https://example.com/fresh.txt\n" +
      "twin, 6, https://example.com/sub.txt\n" +
      "appended, https://example.com/appended.txt",
  );
  await page.click("#dlg-sub-import-ok");
  await expect.poll(() => stub.subAddBodies.length).toBe(2);
  expect(stub.subAddBodies[0]).toMatchObject({ url: "https://example.com/fresh.txt", name: "fresh one", priority: 5 });
  expect(stub.subAddBodies[1]).toMatchObject({ url: "https://example.com/appended.txt", name: "appended", priority: 101 });
  // t() wraps interpolated counts in BIDI isolates, so match the parts.
  const toast = page.locator("#toasts .toast").last();
  await expect(toast).toContainText("subscriptions added");
  await expect(toast).toContainText("duplicates");
  await expect(page.locator("#dlg-sub-import")).not.toHaveAttribute("open", "");
  await expect(page.locator("#sub-body tr")).toHaveCount(3);
});

test("malformed pasted line toasts the line and keeps the dialog open", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  await expect(page.locator("#sub-body tr")).toHaveCount(1);
  const sent = stub.subAddBodies.length;
  await page.click("#btn-sub-import");
  await expect(page.locator("#dlg-sub-import")).toHaveAttribute("open", "");
  await page.fill("#sub-import-text", "ok, 1, https://example.com/a.txt\nbroken line");
  await page.click("#dlg-sub-import-ok");
  expect(stub.subAddBodies.length).toBe(sent);
  await expect(page.locator("#dlg-sub-import")).toHaveAttribute("open", "");
  await expect(page.locator("#toasts .toast").last()).toContainText("not a valid");
});

test("paste button fills the box from the clipboard", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(base + "/subscriptions");
  await expect(page.locator("#sub-body tr")).toHaveCount(1);
  await page.evaluate(() => navigator.clipboard.writeText("pasted one, 7, https://example.com/pasted.txt"));
  await page.click("#btn-sub-import");
  await expect(page.locator("#dlg-sub-import")).toHaveAttribute("open", "");
  await page.click("#dlg-sub-import-paste");
  await expect(page.locator("#sub-import-text")).toHaveValue("pasted one, 7, https://example.com/pasted.txt");
  await page.click("#dlg-sub-import-ok");
  await expect.poll(() => stub.subAddBodies.length).toBe(1);
  expect(stub.subAddBodies[0]).toMatchObject({ url: "https://example.com/pasted.txt", name: "pasted one", priority: 7 });
});

test("copy button puts the export text on the clipboard", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(base + "/subscriptions");
  await expect(page.locator("#sub-body tr")).toHaveCount(1);
  await page.click("#btn-sub-export");
  await expect(page.locator("#dlg-sub-export")).toHaveAttribute("open", "");
  await page.click("#dlg-sub-export-copy");
  await expect(page.locator("#toasts .toast").last()).toContainText("copied");
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  expect(clip).toContain("# name, priority, url");
  expect(clip).toContain("demo, 100, https://example.com/sub.txt");
});
