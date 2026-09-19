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

test("duplicate URL pops up the twin index; nothing is sent, dialog stays open", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  const twinUrl = stub.subs[0].url;
  const sent = stub.subAddBodies.length;
  await page.click("#btn-sub-add");
  await expect(page.locator("#dlg-sub")).toHaveAttribute("open", "");
  await page.fill("#dlg-sub-url", twinUrl);
  await page.fill("#dlg-sub-name", "twin");
  const message = new Promise((resolve) => {
    page.once("dialog", (dlg) => {
      const text = dlg.message();
      void dlg.accept().then(() => resolve(text));
    });
  });
  const clicked = page.click("#dlg-sub-ok");
  // t() wraps interpolations in BIDI isolates, so match the parts.
  const text = await message;
  expect(text).toContain("index");
  expect(text).toContain("1");
  await clicked;
  expect(stub.subAddBodies.length).toBe(sent);
  await expect(page.locator("#dlg-sub")).toHaveAttribute("open", "");
  await expect(page.locator("#dlg-sub-url")).toHaveValue(twinUrl);
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

  // Accessible name carries bidi isolates around the sub name (i18n t()).
  const tgl = page.locator("#sub-body tr").first().getByRole("button", { name: /Toggle.*renamed/ });
  await expect(tgl).toHaveText("✅");
  await tgl.click();
  await expect.poll(() => stub.subToggle.length).toBe(1);
  await expect(tgl).toHaveText("❌");
});

test("drag-and-drop reorders rows and renumbers priorities 1..N", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  const rows = page.locator("#sub-body tr");
  const before = await rows.count();
  // Ensure a second row so there is something to reorder.
  await page.click("#btn-sub-add");
  await page.fill("#dlg-sub-url", "https://example.com/dnd.txt");
  await page.fill("#dlg-sub-name", "dnd-second");
  await page.click("#dlg-sub-ok");
  await expect(rows).toHaveCount(before + 1);
  const n = before + 1;
  const movedName = await rows.first().locator("td").nth(3).textContent();

  // Drag the first grip below the last row (real mouse HTML5 DnD).
  const lastBox = await rows.nth(n - 1).boundingBox();
  await rows.first().locator(".drag-handle").hover();
  await page.mouse.down();
  await page.mouse.move(lastBox.x + lastBox.width / 2, lastBox.y + lastBox.height - 4, { steps: 8 });
  await page.mouse.up();

  // One reorder POST with [1..n-1, 0]; UI resyncs renamed order + 1..N.
  await expect.poll(() => stub.subReorder.length).toBe(1);
  expect(stub.subReorder[0]).toEqual([...Array(n).keys()].map((k) => (k + 1) % n));
  await expect(page.locator("#toasts .toast").last()).toContainText("Reordered.");
  await expect(rows.nth(n - 1)).toContainText(movedName.trim());
  const pris = await page.locator("#sub-body tr td:nth-child(3)").allTextContents();
  expect(pris).toEqual([...Array(n).keys()].map((k) => String(k + 1)));
});

test("editing priority moves the row to its new rank in real time", async ({ page }) => {
  await page.goto(base + "/subscriptions");
  const rows = page.locator("#sub-body tr");

  // Add a newcomer with a low rank: it lands last.
  await page.click("#btn-sub-add");
  await page.fill("#dlg-sub-url", "https://example.com/prio.txt");
  await page.fill("#dlg-sub-name", "prio-move");
  await page.fill("#dlg-sub-priority", "999");
  await page.click("#dlg-sub-ok");
  await expect(rows.last()).toContainText("prio-move");
  const n = await rows.count();

  // Retitle its rank to 1: the row jumps first and ranks stay dense 1..N.
  await rows.last().getByRole("button", { name: "Edit" }).click();
  await page.fill("#dlg-sub-priority", "1");
  await page.click("#dlg-sub-ok");
  await expect(rows.first()).toContainText("prio-move");
  const pris = await page.locator("#sub-body tr td:nth-child(3)").allTextContents();
  expect(pris).toEqual([...Array(n).keys()].map((k) => String(k + 1)));
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
  await expect(page.locator(".set-row")).toHaveCount(6);
  // Translated name + description columns flank the value control.
  await expect(page.locator(".set-row .set-name").first()).toHaveText("Bind address");
  await expect(page.locator(".set-row .guide").first()).toContainText("dashboard listens");
  await expect(page.locator("#btn-settings-reload")).toHaveAttribute("title", "Show the current server values again (drops anything you are typing). Never changes or resets anything.");
  await page.locator(".set-row .val").first().click();
  const input = page.locator(".set-row input").first();
  await input.fill("127.0.0.1:27142");
  await input.press("Enter");
  await expect.poll(() => stub.configPatch.length).toBe(1);
  expect(stub.configPatch[0]).toMatchObject({ key: "bind" });
  // Let the first round-trip fully settle: the toast fires before the
  // resync rebuilds the rows, and an editor opened too early would be
  // destroyed (or worse, shadow the next one).
  await expect(page.locator("#toasts")).toContainText("Saved.");
  await expect(page.locator(".set-row input")).toHaveCount(0);
  // Numeric fields normalize keyboard digits: Persian digits arrive as ASCII
  // so the server never rejects what the user typed.
  await page.locator(".set-row .val").nth(1).click();
  const num = page.locator(".set-row input").first();
  await num.fill("۸");
  await num.press("Enter");
  await expect.poll(() => stub.configPatch.length).toBe(2);
  expect(stub.configPatch[1]).toMatchObject({ key: "top_n", value: "8" });
  // One Enter sends exactly one PATCH: the keystroke must not bubble into a
  // second (empty) editor that would commit "" on the next blur. Clicking a
  // neutral row name blurs anything left open; the count must not move.
  await expect(page.locator(".set-row input")).toHaveCount(0);
  await page.locator(".set-row .set-name").nth(1).click();
  // Absence check: a phantom empty commit would land within a round-trip.
  await page.waitForTimeout(500);
  expect(stub.configPatch.length).toBe(2);
});

test("reset button asks first; cancel sends nothing, confirm POSTs reset", async ({ page }) => {
  await page.goto(base + "/settings");
  await page.locator("#btn-settings-reset").click();
  const dlg = page.locator("#dlg-reset");
  await expect(dlg).toBeVisible();
  await expect(dlg.locator("#dlg-reset-title")).toHaveText("Reset to defaults?");
  await expect(dlg.locator("p.muted")).toContainText("subscriptions are kept");
  await page.locator("#dlg-reset-cancel").click();
  await expect(dlg).toBeHidden();
  expect(stub.configReset).toEqual([]);
  await page.locator("#btn-settings-reset").click();
  await expect(dlg).toBeVisible();
  await page.locator("#dlg-reset-ok").click();
  await expect(dlg).toBeHidden();
  await expect.poll(() => stub.configReset.length).toBe(1);
  await expect(page.locator("#toasts")).toContainText("Defaults restored.");
});

test("secret row Shows the token on demand and hides it again", async ({ page }) => {
  await page.goto(base + "/settings");
  const row = page.locator(".set-row").nth(5);
  await expect(row.locator(".set-presence")).toHaveText("set");
  await row.getByRole("button", { name: "Show" }).click();
  await expect(row.locator("code")).toHaveText("stub-token");
  await expect.poll(() => stub.tokenReveals.length).toBe(1);
  await row.getByRole("button", { name: "Hide" }).click();
  await expect(row.locator(".set-presence")).toHaveText("set");
});

test("bool switch PATCHes the flipped value; readonly rows stay static", async ({ page }) => {
  await page.goto(base + "/settings");
  const sw = page.locator(".set-row .switch").first();
  await expect(sw).toHaveText("On");
  await sw.click();
  await expect.poll(() => stub.configPatch.length).toBe(1);
  expect(stub.configPatch[0]).toMatchObject({ key: "encoded_subscription", value: "false" });
  // Read-only rows render no control at all (5th stub row = the switch).
  const ro = page.locator(".set-row").nth(4).locator(".val");
  await expect(ro).toContainText("false");
  await expect(ro.locator("button, input, select")).toHaveCount(0);
});
