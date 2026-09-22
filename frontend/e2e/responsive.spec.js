import { test, expect, devices } from "@playwright/test";
import { createStub } from "./stub-server.mjs";

// Aspect-ratio sweep: every route on phones (portrait + landscape), tablets,
// and desktops. Portrait phones get the bottom-tab app layout with labeled
// table-cards; everything else keeps real tables. Every combination asserts
// zero page-level horizontal overflow plus the chrome its form factor owns.
//
// The Termux-on-Android case (a phone browser on the server bundle) is
// covered faithfully by the Pixel 7 descriptor: real Android UA, touch,
// mobile viewport.

let stub;
let server;
let base;

// Chromium refuses to navigate to its blocklisted "unsafe" ports (5061,
// 6000, 6665-6669, …). A port-0 stub can draw one, so re-roll until the
// port is navigable instead of flaking the whole sweep.
const UNSAFE_PORTS = new Set([
  1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77,
  79, 87, 95, 101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123,
  135, 137, 138, 139, 143, 161, 179, 389, 427, 465, 512, 513, 514, 515, 526,
  530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 991, 992,
  993, 995, 999, 1000, 1022, 1023, 1024, 1720, 1723, 2049, 2121, 2375, 2376,
  3659, 4045, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6697,
  10080,
]);

test.beforeAll(async () => {
  ({ stub, server } = createStub());
  for (let i = 0; i < 25; i++) {
    await new Promise((r) => server.listen(0, "127.0.0.1", r));
    const port = server.address().port;
    if (!UNSAFE_PORTS.has(port)) {
      break;
    }
    await new Promise((r) => server.close(r));
  }
  base = `http://127.0.0.1:${server.address().port}`;
});

test.afterAll(async () => {
  await new Promise((r) => server.close(r));
});

const ROUTES = [
  { name: "overview", settle: "#stat-cards .card", count: 7 },
  { name: "configs", settle: "#cfg-body tr", count: 2 },
  { name: "subscriptions", settle: "#sub-body tr", count: 1 },
  { name: "settings", settle: ".set-row", count: 1 },
  { name: "proxy", settle: null, count: 0 },
  { name: "logs", settle: null, count: 0 },
  { name: "share", settle: null, count: 0 },
];

async function gotoRoute(page, route, errors) {
  page.on("console", (m) => {
    if (m.type() === "error") errors.push("console: " + m.text());
  });
  page.on("pageerror", (e) => errors.push("page: " + e.message));
  await page.goto(base + "/" + route.name);
  await expect(page.locator("#panel-" + route.name)).toBeVisible();
  if (route.settle) {
    const rows = page.locator(route.settle);
    if (route.count > 1) {
      await expect(rows).toHaveCount(route.count);
    } else {
      await expect.poll(async () => rows.count()).toBeGreaterThan(0);
    }
  }
  await page.screenshot({ path: test.info().outputPath(`shot-${route.name}.png`) });
}

/// No page-level horizontal scrolling: wide content must live inside its own
/// scroll container (table-wrap, tab bar), never the page.
async function expectNoPageOverflow(page) {
  const r = await page.evaluate(() => ({
    doc: document.documentElement.scrollWidth,
    body: document.body ? document.body.scrollWidth : 0,
    vw: window.innerWidth,
  }));
  expect(r.doc, "documentElement scrollWidth vs viewport").toBeLessThanOrEqual(r.vw + 2);
  expect(r.body, "body scrollWidth vs viewport").toBeLessThanOrEqual(r.vw + 2);
}

/// Every top-bar control stays inside the viewport (nothing clipped off the
/// right edge, in LTR or RTL).
async function expectTopActionsInside(page) {
  const vw = await page.evaluate(() => window.innerWidth);
  const boxes = await page.$$eval(
    "#btn-refresh, #btn-ping, .theme-switch, #btn-lang, #btn-keys, #btn-power",
    (els) => els.map((e) => {
      const r = e.getBoundingClientRect();
      return { x: r.x, w: r.width };
    }),
  );
  expect(boxes.length).toBe(6);
  for (const b of boxes) {
    expect(b.x, "control inside left edge").toBeGreaterThanOrEqual(-1);
    expect(b.x + b.w, "control inside right edge").toBeLessThanOrEqual(vw + 1);
  }
}

/// Small screens: every scrollable region shows its bar (never an
/// invisible swipe-only area) — tab bar, wide tables, live logs.
async function expectVisibleScrollbars(page, route) {
  const tabs = await page.$eval(".tabs", (el) => [
    getComputedStyle(el).scrollbarWidth,
    getComputedStyle(el, "::-webkit-scrollbar").height,
  ]);
  expect(tabs, "tab bar indicator").toEqual(["thin", "5px"]);
  const regions = route.name === "overview"
    ? ["#stat-cards", ".table-wrap"]
    : route.name === "logs"
      ? ["#log-list"]
      : (route.name === "configs" || route.name === "subscriptions")
        ? [".table-wrap"]
        : [];
  for (const sel of regions) {
    const r = await page.$eval(sel, (el) => [
      getComputedStyle(el).scrollbarWidth,
      getComputedStyle(el, "::-webkit-scrollbar").height,
      getComputedStyle(el, "::-webkit-scrollbar").width,
    ]);
    expect(r[0], sel + " thin").toBe("thin");
    expect([r[1], r[2]].includes("5px"), sel + " 5px indicator").toBe(true);
  }
}

function sweep(suiteName, use, mode) {
  test.describe(suiteName, () => {
    test.use(use);
    for (const route of ROUTES) {
      test(`${route.name}: fits the viewport`, async ({ page }) => {
        const errors = [];
        await gotoRoute(page, route, errors);
        await expectNoPageOverflow(page);
        await expectTopActionsInside(page);
        if (mode !== "desktop") {
          await expectVisibleScrollbars(page, route);
        }
        if (mode === "phone-portrait") {
          // The always-overflowing tab bar fades at the trailing edge (the
          // at-rest cue overlay scrollbars cannot give); the stat badges
          // are a fixed grid now, so only the tabs keep the swipe cue.
          const mask = await page.$eval(".tabs", (el) => getComputedStyle(el).maskImage);
          expect(mask, ".tabs swipe cue").toContain("linear-gradient");
        }
        const tabsPos = await page.evaluate(
          () => getComputedStyle(document.querySelector(".tabs")).position,
        );
        if (mode === "phone-portrait") {
          // Bottom tab bar docked at the thumb, real tables gone.
          expect(tabsPos).toBe("fixed");
          const bar = await page.locator(".tabs").boundingBox();
          const vh = await page.evaluate(() => window.innerHeight);
          expect(bar.y + bar.height, "tab bar docked at bottom").toBeGreaterThanOrEqual(vh - 2);
          if (route.name === "configs" || route.name === "subscriptions" || route.name === "overview") {
            const table = route.name === "overview" ? "#ov-cfg-table" : route.name === "configs" ? "#cfg-table" : "#sub-table";
            await expect(page.locator(table + " thead")).toBeHidden();
            // Every value cell carries its card label (the grip handle and
            // the label-less title/action cells are hidden or
            // self-evident by design).
            const unlabeled = await page.$$eval(
              table + " tbody td",
              (cells) => cells.filter((td) => !td.dataset.th
                && !td.classList.contains("cell-main")
                && !td.classList.contains("cell-actions")
                && !td.classList.contains("cell-grip")).length,
            );
            expect(unlabeled).toBe(0);
          }
          if (route.name === "overview") {
            // Six badges (Fetched hides on phones) in a fixed 3x2 grid —
            // nothing scrolls horizontally.
            const fetchedDisplay = await page.$eval("#stat-fetched", (el) => getComputedStyle(el).display);
            expect(fetchedDisplay, "Fetched badge hidden on phones").toBe("none");
            const workingSubDisplay = await page.$eval("#stat-working-sub", (el) => getComputedStyle(el).display);
            expect(workingSubDisplay, "working sub carries the fetched fallback on phones").not.toBe("none");
            const scrollable = await page.$eval("#stat-cards", (el) => el.scrollWidth > el.clientWidth + 1);
            expect(scrollable, "stat grid does not scroll").toBe(false);
            const cols = await page.$eval("#stat-cards", (el) => getComputedStyle(el).gridTemplateColumns.split(" ").length);
            expect(cols, "stat grid columns").toBe(3);
            // The stacked row shows Endpoints above Top configs.
            const stacked = await page.evaluate(() => {
              const ep = document.getElementById("ov-endpoints-card").getBoundingClientRect();
              const cfg = document.getElementById("ov-cfg-table").closest(".card").getBoundingClientRect();
              return ep.bottom <= cfg.top;
            });
            expect(stacked, "Endpoints above Top configs on phones").toBe(true);
          }
        } else {
          // Top tabs + real tables everywhere else.
          expect(tabsPos).not.toBe("fixed");
          if (route.name === "configs" || route.name === "subscriptions") {
            const table = route.name === "configs" ? "#cfg-table" : "#sub-table";
            await expect(page.locator(table + " thead")).toBeVisible();
          }
        }
        if (mode === "tablet-sticky" && (route.name === "configs" || route.name === "subscriptions")) {
          // The name column pins while the table scrolls sideways.
          const sel = route.name === "configs"
            ? "#cfg-table thead th:nth-child(2)"
            : "#sub-table thead th:nth-child(4)";
          const pos = await page.$eval(sel, (th) => getComputedStyle(th).position);
          expect(pos, "sticky name column").toBe("sticky");
        }
        expect(errors).toEqual([]);
      });
    }

    // Thumb-size controls on touch form factors (overview is enough: the
    // top bar and tabs are identical on every route).
    if (use.hasTouch) {
      test("touch targets are thumb-size", async ({ page }) => {
        const errors = [];
        await gotoRoute(page, ROUTES[0], errors);
        const heights = await page.$$eval(
          "#btn-refresh, #btn-ping, .theme-switch, #btn-lang, #btn-keys, #btn-power, .tab",
          (els) => els.map((e) => Math.round(e.getBoundingClientRect().height)),
        );
        expect(heights.length).toBeGreaterThan(0);
        for (const h of heights) {
          expect(h, "touch target height").toBeGreaterThanOrEqual(40);
        }
        expect(errors).toEqual([]);
      });
    }
  });
}

const touch = { hasTouch: true };
const mouse = { hasTouch: false };
// The Termux-on-Android client is a phone browser on the server bundle:
// exercise it with the real Android Chrome UA + phone pixel density
// (`isMobile` is worker-bound and cannot vary per suite, so the descriptor
// contributes everything describe-safe instead).
const pixel7 = {
  viewport: { width: 412, height: 915 },
  hasTouch: true,
  userAgent: devices["Pixel 7"].userAgent,
  deviceScaleFactor: devices["Pixel 7"].deviceScaleFactor,
};

sweep("phone 360x740 portrait", { viewport: { width: 360, height: 740 }, ...touch }, "phone-portrait");
sweep("phone 390x844 portrait", { viewport: { width: 390, height: 844 }, ...touch }, "phone-portrait");
sweep("pixel7 termux android", pixel7, "phone-portrait");
sweep("phone 740x360 landscape", { viewport: { width: 740, height: 360 }, ...touch }, "phone-landscape");
sweep("phone 844x390 landscape", { viewport: { width: 844, height: 390 }, ...touch }, "phone-landscape");
sweep("tablet 768x1024 portrait", { viewport: { width: 768, height: 1024 }, ...touch }, "tablet-sticky");
sweep("tablet 820x1180 portrait", { viewport: { width: 820, height: 1180 }, ...touch }, "tablet-sticky");
sweep("tablet 1024x768 landscape", { viewport: { width: 1024, height: 768 }, ...touch }, "tablet-sticky");
sweep("tablet 1180x820 landscape", { viewport: { width: 1180, height: 820 }, ...touch }, "desktop");
sweep("laptop 1280x800", { viewport: { width: 1280, height: 800 }, ...mouse }, "desktop");
sweep("desktop 1536x864", { viewport: { width: 1536, height: 864 }, ...mouse }, "desktop");

// Persian RTL on a narrow phone: the longest strings must still fit —
// bottom bar, icon actions, and labeled cards in RTL.
test.describe("rtl phone 390x844", () => {
  test.use({ viewport: { width: 390, height: 844 }, ...touch });
  for (const route of ROUTES.slice(0, 3)) {
    test(`${route.name}: fits in RTL`, async ({ page }) => {
      const errors = [];
      await page.addInitScript(() => {
        try {
          window.localStorage.setItem("v2raydar-lang", "fa");
        } catch (err) {
          /* private mode: default language still exercises layout */
        }
      });
      await gotoRoute(page, route, errors);
      expect(await page.evaluate(() => document.documentElement.dir)).toBe("rtl");
      await expectNoPageOverflow(page);
      await expectTopActionsInside(page);
      await expectVisibleScrollbars(page, route);
      const rtlMask = await page.$eval(".tabs", (el) => getComputedStyle(el).maskImage);
      expect(rtlMask, "RTL swipe cue").toContain("linear-gradient");
      expect(errors).toEqual([]);
    });
  }
});

// Narrowest portrait phone in Persian: overview sub-lines must show fully —
// single-line ellipsis used to cut them mid-word ("..."), so assert content
// fits in both dimensions instead of only page-level scrollWidth.
test.describe("rtl phone 360x740", () => {
  test.use({ viewport: { width: 360, height: 740 }, ...touch });
  test("overview stat subs fit without clipping", async ({ page }) => {
    const errors = [];
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem("v2raydar-lang", "fa");
      } catch (err) {
        /* private mode: default language still exercises layout */
      }
    });
    await gotoRoute(page, ROUTES[0], errors);
    expect(await page.evaluate(() => document.documentElement.dir)).toBe("rtl");
    const clipped = await page.$$eval("#stat-cards .stat-sub", (els) =>
      els.filter((el) => el.scrollWidth > el.clientWidth + 1
        || el.scrollHeight > el.clientHeight + 1).length);
    expect(clipped, "stat subs fit without clipping").toBe(0);
    expect(errors).toEqual([]);
  });
});

// Large screens show the Fetched badge, so the Working card's fetched
// fallback sub must stay hidden there (mirrors the phone assertion above).
test.describe("desktop overview", () => {
  test.use({ viewport: { width: 1536, height: 864 }, ...mouse });
  test("fetched badge shows, working sub hides", async ({ page }) => {
    const errors = [];
    await gotoRoute(page, ROUTES[0], errors);
    await expect(page.locator("#stat-fetched")).toBeVisible();
    await expect(page.locator("#stat-working-sub")).toBeHidden();
    expect(errors).toEqual([]);
  });
});
