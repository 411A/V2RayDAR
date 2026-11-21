import { test, expect } from "@playwright/test";

// Live password lifecycle against the real Rust backend (LIVE_BASE):
// enable, sessions, wrong password, update, recovery reset, disable, and the
// real reset-lock countdown. Skipped unless LIVE_BASE is set, so the default
// Playwright run stays hermetic. The spec refuses to touch an instance that
// already has a password and restores protection to off in afterAll.
const LIVE = (process.env.LIVE_BASE || "").replace(/\/+$/, "");
test.skip(!LIVE, "LIVE_BASE not set — stub specs cover CI");
test.setTimeout(120_000);

const suffix = Math.random().toString(36).slice(2, 10);
const P1 = `Live-Enable-${suffix}-2026!`;
const R1 = `live phrase one ${suffix}`;
const P2 = `Live-Updated-${suffix}-2026!`;
const R2 = `live phrase two ${suffix}`;
const P3 = `Live-Recovered-${suffix}-2026!`;
const P4 = `Live-Lock-${suffix}-2026!`;
const R4 = `live phrase four ${suffix}`;
const known = { password: P1 };
let armed = false;

const pathOf = (response) => new URL(response.url()).pathname;

async function status(request) {
  const response = await request.get(LIVE + "/api/dashboard/status");
  return { code: response.status(), body: await response.json() };
}

async function waitPost(page, action, path) {
  const [response] = await Promise.all([
    page.waitForResponse((r) => pathOf(r) === path && r.request().method() === "POST"),
    action(),
  ]);
  return response;
}

async function fillAuth(page, { current, next, phrase }) {
  if (current !== undefined) await page.fill("#auth-current", current);
  await page.fill("#auth-new", next);
  await page.fill("#auth-confirm", next);
  await page.fill("#auth-phrase", phrase);
  await page.fill("#auth-phrase-confirm", phrase);
}

async function login(page, password) {
  await expect(page.locator("#login-password")).toBeVisible();
  await page.fill("#login-password", password);
  const response = await waitPost(page, () => page.click("#login-submit"), "/api/dashboard/login");
  expect(response.status()).toBe(200);
  await expect(page.locator("#login-password")).toHaveCount(0);
}

async function signOut(page) {
  const response = await waitPost(
    page,
    () => page.getByRole("button", { name: "Sign out" }).click(),
    "/api/dashboard/logout",
  );
  expect(response.status()).toBe(200);
  await expect(page.locator("#login-password")).toBeVisible();
}

async function settingsPage(page) {
  await page.goto(LIVE + "/settings", { waitUntil: "networkidle" });
  await expect(page.locator("#dashboard-auth-card")).toBeVisible();
}

test.afterAll(async ({ browser }) => {
  if (!LIVE || !armed) return;
  const context = await browser.newContext();
  try {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const current = await status(context.request);
      if (!current.body.enabled) return;
      const login = await context.request.post(LIVE + "/api/dashboard/login", {
        data: { password: known.password },
      });
      if (login.status() === 429) {
        await new Promise((resolve) => setTimeout(resolve, 6000));
        continue;
      }
      if (!login.ok()) {
        console.warn("live-auth cleanup: could not sign in; disable the password manually");
        return;
      }
      const disabled = await context.request.post(LIVE + "/api/dashboard/password", {
        data: { disable: true, current_password: known.password },
      });
      if (!disabled.ok()) {
        console.warn("live-auth cleanup: disable failed with", disabled.status());
      }
      return;
    }
  } finally {
    await context.close();
  }
});

test("live: password lifecycle, sessions, recovery, and reset lock", async ({ page, browser }) => {
  page.setDefaultTimeout(15_000);
  const initial = await status(page.request);
  test.skip(initial.body.enabled, "live instance already has a dashboard password");

  await test.step("enable from the settings card", async () => {
    await settingsPage(page);
    await expect(page.locator("#auth-submit")).toBeDisabled();
    await page.fill("#auth-new", "short");
    await page.fill("#auth-confirm", "different");
    await page.fill("#auth-phrase", "tiny");
    await expect(page.locator("#auth-new-meta")).toHaveText("5/12");
    await expect(page.locator("#auth-confirm-meta")).toHaveText("Different");
    await expect(page.locator("#auth-submit")).toBeDisabled();
    await page.check("#auth-show");
    await expect(page.locator("#auth-new")).toHaveAttribute("type", "text");
    await page.uncheck("#auth-show");
    await expect(page.locator("#auth-new")).toHaveAttribute("type", "password");
    await fillAuth(page, { next: P1, phrase: R1 });
    await expect(page.locator("#auth-confirm-meta")).toHaveText("Matches");
    const response = await waitPost(page, () => page.click("#auth-submit"), "/api/dashboard/password");
    expect(response.status()).toBe(200);
    armed = true;
    await expect(page.locator("#toasts")).toContainText("Dashboard password updated.");
    await expect(page.locator("#dash-auth-state")).toContainText("On.");
    await expect(page.locator("#auth-current")).toBeVisible();
    const current = await status(page.request);
    expect(current.body).toMatchObject({ enabled: true, has_phrase: true, authenticated: true });
  });

  await test.step("session survives a reload", async () => {
    await settingsPage(page);
    await expect(page.locator("#dash-auth-state")).toContainText("On.");
  });

  await test.step("anonymous browser sees the sign-in page and cannot change settings", async () => {
    const anon = await browser.newContext();
    try {
      const anonPage = await anon.newPage();
      await anonPage.goto(LIVE + "/settings", { waitUntil: "networkidle" });
      await expect(anonPage.locator("#login-password")).toBeVisible();
      const read = await anon.request.get(LIVE + "/api/config");
      expect(read.status()).toBe(200);
      const write = await anon.request.patch(LIVE + "/api/config", {
        data: { key: "__not_a_setting__", value: "x" },
      });
      expect(write.status()).toBe(401);
      expect((await write.json()).code).toBe("dashboard_locked");
    } finally {
      await anon.close();
    }
  });

  await test.step("wrong password is rejected, then the right one signs in", async () => {
    await signOut(page);
    await page.fill("#login-password", "definitely-wrong");
    const rejected = await waitPost(page, () => page.click("#login-submit"), "/api/dashboard/login");
    expect(rejected.status()).toBe(401);
    await expect(page.locator("#banner")).toContainText("That password is not correct.");
    await page.waitForTimeout(5600);
    await login(page, P1);
    await settingsPage(page);
  });

  await test.step("update the password with the current one", async () => {
    await fillAuth(page, { current: P1, next: P2, phrase: R2 });
    const response = await waitPost(page, () => page.click("#auth-submit"), "/api/dashboard/password");
    expect(response.status()).toBe(200);
    known.password = P2;
    await expect(page.locator("#toasts")).toContainText("Dashboard password updated.");
    await expect(page.locator("#auth-current")).toHaveValue("");
    await expect(page.locator("#dash-auth-state")).toContainText("On.");
  });

  await test.step("the updated password is the one that signs in", async () => {
    await signOut(page);
    await login(page, P2);
    await settingsPage(page);
  });

  await test.step("recovery phrase resets the password", async () => {
    await signOut(page);
    await page.click("#show-reset");
    await expect(page.locator("#reset-phrase")).toBeVisible();
    await page.fill("#reset-phrase", R2);
    await page.fill("#reset-password", P3);
    await page.fill("#reset-confirm", P3);
    const response = await waitPost(page, () => page.click("#reset-submit"), "/api/dashboard/reset");
    expect(response.status()).toBe(200);
    known.password = P3;
    await expect(page.locator("#login-password")).toHaveCount(0);
    await settingsPage(page);
    expect((await status(page.request)).body.authenticated).toBe(true);
  });

  await test.step("the recovered password survives a fresh sign-in", async () => {
    await signOut(page);
    await login(page, P3);
    await settingsPage(page);
  });

  await test.step("disable protection with the current password", async () => {
    await page.fill("#auth-current", P3);
    page.once("dialog", (dialog) => void dialog.accept());
    const response = await waitPost(
      page,
      () => page.getByRole("button", { name: "Disable password" }).click(),
      "/api/dashboard/password",
    );
    expect(response.status()).toBe(200);
    armed = false;
    await expect(page.locator("#dash-auth-state")).toContainText("Off.");
    const current = await status(page.request);
    expect(current.body).toMatchObject({ enabled: false, has_phrase: false });
  });

  await test.step("dashboard is open again for a new browser", async () => {
    const anon = await browser.newContext();
    try {
      const anonPage = await anon.newPage();
      await anonPage.goto(LIVE + "/settings", { waitUntil: "networkidle" });
      await expect(anonPage.locator("#dashboard-auth-card")).toBeVisible();
      await expect(anonPage.locator("#login-password")).toHaveCount(0);
      const read = await anon.request.get(LIVE + "/api/config");
      expect(read.status()).toBe(200);
      const write = await anon.request.patch(LIVE + "/api/config", {
        data: { key: "__not_a_setting__", value: "x" },
      });
      expect(write.status()).toBe(400);
    } finally {
      await anon.close();
    }
  });

  await test.step("a wrong phrase pauses recovery and the countdown ticks", async () => {
    await settingsPage(page);
    await fillAuth(page, { next: P4, phrase: R4 });
    const enabled = await waitPost(page, () => page.click("#auth-submit"), "/api/dashboard/password");
    expect(enabled.status()).toBe(200);
    armed = true;
    known.password = P4;
    await expect(page.locator("#dash-auth-state")).toContainText("On.");

    await signOut(page);
    await page.click("#show-reset");
    await page.fill("#reset-phrase", "this phrase is wrong");
    await page.fill("#reset-password", P4);
    await page.fill("#reset-confirm", P4);
    const rejected = await waitPost(page, () => page.click("#reset-submit"), "/api/dashboard/reset");
    expect(rejected.status()).toBe(429);
    await expect(page.locator("#banner")).toContainText("Wrong phrase or too many attempts.");
    expect((await status(page.request)).body.reset_wait_seconds).toBeGreaterThan(0);

    await page.reload({ waitUntil: "networkidle" });
    await expect(page.locator("#show-reset")).toContainText("Recovery paused for");
    await expect(page.locator("#show-reset")).toBeDisabled();
    await login(page, P4);
    await settingsPage(page);
    const lock = page.locator("#dash-auth-lock");
    await expect(lock).toContainText("Reset paused");
    const first = await lock.innerText();
    await page.waitForTimeout(1200);
    await expect.poll(async () => lock.innerText()).not.toBe(first);
    expect((await status(page.request)).body.reset_wait_seconds).toBeGreaterThan(0);
  });

  await test.step("disable again, clearing the reset lock", async () => {
    await page.fill("#auth-current", P4);
    page.once("dialog", (dialog) => void dialog.accept());
    const response = await waitPost(
      page,
      () => page.getByRole("button", { name: "Disable password" }).click(),
      "/api/dashboard/password",
    );
    expect(response.status()).toBe(200);
    armed = false;
    await expect(page.locator("#dash-auth-state")).toContainText("Off.");
    const current = await status(page.request);
    expect(current.body).toMatchObject({ enabled: false, has_phrase: false, reset_wait_seconds: null });
  });
});
