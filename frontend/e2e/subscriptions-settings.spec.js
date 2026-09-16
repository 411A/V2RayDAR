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

test("subscriptions table renders; add dialog POSTs and resyncs", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  await expect(page.locator("#sub-body tr")).toHaveCount(1);
  await expect(page.locator("#sub-body")).toContainText("demo");

  await page.click("#btn-sub-add");
  await expect(page.locator("#dlg-sub")).toHaveAttribute("open", "");
  await expect(page.locator("#dlg-sub-title")).toHaveText("Add subscription");
  await page.fill("#dlg-sub-url", "https://example.com/other.txt");
  await page.fill("#dlg-sub-name", "other");
  await page.click("#dlg-sub-ok");
  await expect.poll(() => stub.subAddBodies.length).toBe(1);
  expect(stub.subAddBodies[0]).toMatchObject({ url: "https://example.com/other.txt", name: "other" });
  await expect(page.locator("#sub-body tr")).toHaveCount(2);
});

test("edit dialog PATCHes /:index; toggle flips enabled", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  await page.locator("#sub-body tr").first().getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#dlg-sub-title")).toHaveText("Edit subscription");
  await page.fill("#dlg-sub-name", "renamed");
  await page.click("#dlg-sub-ok");
  await expect.poll(() => stub.subPatch.length).toBe(1);
  expect(stub.subPatch[0]).toMatchObject({ index: 0, body: { name: "renamed" } });
  await expect(page.locator("#sub-body")).toContainText("renamed");

  const tgl = page.locator("#sub-body tr").first().getByRole("button", { name: "Toggle renamed" });
  await expect(tgl).toHaveText("✅");
  await tgl.click();
  await expect.poll(() => stub.subToggle.length).toBe(1);
  await expect(tgl).toHaveText("❌");
});

test("delete asks for confirm; cancel sends nothing", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  page.once("dialog", (d) => void d.dismiss());
  await page.locator("#sub-body tr").first().getByRole("button", { name: "Delete" }).click();
  expect(stub.subDelete).toEqual([]);
  page.once("dialog", (d) => void d.accept());
  await page.locator("#sub-body tr").first().getByRole("button", { name: "Delete" }).click();
  await expect.poll(() => stub.subDelete.length).toBe(1);
});

test("settings editor PATCHes a key; unknown key shows rejection toast", async ({ page }) => {
  await page.goto(base + "/settings");
  await expect(page.locator(".set-row")).toHaveCount(2);
  await page.locator(".set-row .val").first().click();
  const input = page.locator(".set-row input").first();
  await input.fill("127.0.0.1:27142");
  await input.press("Enter");
  await expect.poll(() => stub.configPatch.length).toBe(1);
  expect(stub.configPatch[0]).toMatchObject({ key: "bind" });
});
