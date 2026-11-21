import { test, expect } from "@playwright/test";
import { createStub } from "./stub-server.mjs";

let stub;
let server;
let base;

test.beforeEach(async () => {
  ({ stub, server } = createStub());
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  base = `http://127.0.0.1:${server.address().port}`;
});

test.afterEach(async () => {
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
});

test("dashboard password card enables protection with masked fields", async ({ page }) => {
  await page.goto(base + "/settings");
  const card = page.locator("#dashboard-auth-card");
  await expect(card).toBeVisible();
  await expect(page.locator("#dash-auth-state")).toContainText("Off.");

  const newPassword = page.locator("#auth-new");
  await expect(newPassword).toHaveAttribute("type", "password");
  await page.check("#auth-show");
  await expect(newPassword).toHaveAttribute("type", "text");
  await page.uncheck("#auth-show");
  await expect(newPassword).toHaveAttribute("type", "password");

  await newPassword.fill("correct horse battery staple");
  await page.fill("#auth-confirm", "correct horse battery staple");
  await page.fill("#auth-phrase", "John Smith");
  await page.fill("#auth-phrase-confirm", "John Smith");
  await page.getByRole("button", { name: "Enable password" }).click();

  await expect.poll(() => stub.dashboardPasswordBodies.length).toBe(1);
  expect(stub.dashboardPasswordBodies[0]).toMatchObject({
    new_password: "correct horse battery staple",
    confirm_password: "correct horse battery staple",
    reset_phrase: "John Smith",
    confirm_phrase: "John Smith",
  });
  await expect(page.locator("#dash-auth-state")).toContainText("On.");
  await expect(page.locator("#auth-current")).toBeVisible();
});

test("disabling requires the current password field when protection is on", async ({ page }) => {
  stub.dashAuth.enabled = true;
  stub.dashAuth.has_phrase = true;
  await page.goto(base + "/settings");
  await expect(page.locator("#dashboard-auth-card")).toBeVisible();
  await expect(page.locator("#dash-auth-state")).toContainText("On.");
  await page.check("#auth-show");
  await page.fill("#auth-current", "correct horse battery staple");
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("button", { name: "Disable password" }).click();
  await expect.poll(() => stub.dashboardPasswordBodies.length).toBe(1);
  expect(stub.dashboardPasswordBodies[0]).toMatchObject({
    disable: true,
    current_password: "correct horse battery staple",
  });
  await expect(page.locator("#dash-auth-state")).toContainText("Off.");
});

test("live status warns about a missing phrase and ticks the reset lock", async ({ page }) => {
  stub.dashAuth.enabled = true;
  stub.dashAuth.has_phrase = false;
  stub.dashAuth.reset_wait_seconds = 120;
  await page.goto(base + "/settings");
  await expect(page.locator("#dash-auth-state")).toContainText("On.");
  await expect(page.locator(".auth-warn")).toContainText("No recovery phrase");
  await expect(page.locator("#dash-auth-recommended")).toBeHidden();
  const lock = page.locator("#dash-auth-lock");
  await expect(lock).toContainText("Reset paused");
  const first = await lock.innerText();
  await expect.poll(async () => (await lock.innerText()) !== first, { timeout: 3000 }).toBe(true);
});
