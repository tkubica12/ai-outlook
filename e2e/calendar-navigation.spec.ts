import { expect, test, type Page } from "@playwright/test";
import { installCalendar, OFFGRID_HIDDEN, perDay, type Density } from "./navigation-fixtures";

// Every request is fulfilled in the browser from synthetic fixtures, so this
// suite never touches the backend, the cache or the Copilot CLI, and it carries
// no real meeting, customer or attendee content. Assertions stay structural:
// counts, geometry and roles only.

const openCalendar = async (page: Page, density: Density = "normal") => {
  const harness = await installCalendar(page, density);
  await page.goto("/");
  await page.waitForSelector(".week-view, .month-grid", { timeout: 30_000 });
  await expect(page.locator(".connection-setup")).toHaveCount(0);
  await page.waitForTimeout(300);
  return harness;
};

const view = (page: Page, name: string) => page.getByRole("button", { name, exact: true });

/**
 * The week grid is the desktop's dense case; on a phone the same density lands
 * in Day view, which is what a narrow viewport actually opens on.
 */
const denseView = (isMobile: boolean | undefined) => (isMobile ? "Day" : "Week");
const denseDays = (isMobile: boolean | undefined) => (isMobile ? 1 : 7);

/** A day of this month that is always at least ten days from today. */
const farDayOfMonth = () => (new Date().getDate() > 14 ? 3 : 25);

test.describe("calendar selection", () => {
  // The mini calendar lives in the drawer on narrow viewports; the mobile
  // describe below covers that path.
  test.skip(({ isMobile }) => !!isMobile, "covered by the mobile drawer suite");

  test("a mini-calendar date selects the week without switching to Day", async ({ page }) => {
    await openCalendar(page);
    await view(page, "Week").click();
    const heading = page.getByRole("heading", { level: 1 });
    const before = await heading.textContent();

    await page
      .locator(".mini-days button:not(.outside)")
      .nth(farDayOfMonth() - 1)
      .click();

    await expect(view(page, "Week")).toHaveAttribute("aria-pressed", "true");
    await expect(view(page, "Day")).toHaveAttribute("aria-pressed", "false");
    expect(await heading.textContent()).not.toBe(before);
    // Seven contiguous days stay highlighted, with rounded ends on the range.
    await expect(page.locator(".mini-days .in-range")).toHaveCount(7);
    await expect(page.locator(".mini-days .in-range.range-start").first()).toBeVisible();
  });

  test("each view highlights its own range in the mini calendar", async ({ page }) => {
    await openCalendar(page);
    for (const [name, count] of [
      ["Day", 1],
      ["Work week", 5],
      ["Week", 7],
    ] as const) {
      await view(page, name).click();
      await expect(page.locator(".mini-days .in-range")).toHaveCount(count);
    }
    await view(page, "Month").click();
    expect(await page.locator(".mini-days .in-range").count()).toBeGreaterThanOrEqual(28);
  });

  test("browsing the mini month leaves the main view alone and stays keyboard reachable", async ({
    page,
  }) => {
    await openCalendar(page);
    await view(page, "Week").click();
    const heading = await page.getByRole("heading", { level: 1 }).textContent();

    await page.getByRole("button", { name: "Next month" }).click();
    await page.getByRole("button", { name: "Next month" }).click();

    expect(await page.getByRole("heading", { level: 1 }).textContent()).toBe(heading);
    await expect(page.locator('.mini-days button[tabindex="0"]')).toHaveCount(1);

    const back = page.getByRole("button", { name: /Back to selected/ });
    await expect(back).toBeVisible();
    await back.click();
    await expect(page.locator(".mini-days .in-range")).toHaveCount(7);
    expect(await page.getByRole("heading", { level: 1 }).textContent()).toBe(heading);
  });

  test("arrow keys roam the mini calendar and Enter commits the range", async ({ page }) => {
    await openCalendar(page);
    await view(page, "Week").click();
    const heading = await page.getByRole("heading", { level: 1 }).textContent();

    await page.locator('.mini-days button[tabindex="0"]').focus();
    // Two rows down is a fortnight away, so the committed week always differs.
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    expect(await page.getByRole("heading", { level: 1 }).textContent()).toBe(heading);
    await expect(page.locator('.mini-days button[tabindex="0"]')).toBeFocused();

    await page.keyboard.press("Enter");
    expect(await page.getByRole("heading", { level: 1 }).textContent()).not.toBe(heading);
    await expect(view(page, "Week")).toHaveAttribute("aria-pressed", "true");
  });
});

test.describe("event density", () => {
  test("every meeting in the period is rendered and inside its column", async ({
    page,
    isMobile,
  }) => {
    const harness = await openCalendar(page, "stacked");
    await view(page, denseView(isMobile)).click();
    await page.getByRole("button", { name: "Next period" }).click();
    await page.waitForTimeout(400);

    // No "+N" placeholder may stand in for a meeting.
    await expect(page.locator(".overlap-summary")).toHaveCount(0);

    const report = await page.evaluate(() => {
      const cards = Array.from(document.querySelectorAll<HTMLElement>(".day-column .event-card"));
      let outside = 0;
      let tooNarrow = 0;
      let missingTime = 0;
      for (const card of cards) {
        const box = card.getBoundingClientRect();
        const column = card.closest(".day-column")!.getBoundingClientRect();
        if (box.right > column.right + 1 || box.left < column.left - 1) outside += 1;
        // A collapsed card still has to be wide enough to read and to click.
        if (box.width < Math.min(56, column.width - 6)) tooNarrow += 1;
        const timeLine = card.querySelector<HTMLElement>(".event-time");
        // The second line appears once the card is tall enough to hold it.
        if (box.height >= 30 && (!timeLine || getComputedStyle(timeLine).display === "none"))
          missingTime += 1;
      }
      return { cards: cards.length, outside, tooNarrow, missingTime };
    });

    expect(report.cards).toBe(perDay("stacked") * denseDays(isMobile));
    expect(report.outside).toBe(0);
    expect(report.tooNarrow).toBe(0);
    expect(report.missingTime).toBe(0);
    expect(harness.writes).toHaveLength(0);
  });

  test("a covered card keeps a clickable strip and opens in full on hover", async ({
    page,
    isMobile,
  }) => {
    await openCalendar(page, "stacked");
    await view(page, denseView(isMobile)).click();
    await page.waitForTimeout(400);

    const covered = page.locator('.event-card[data-overlapped="true"]');
    // The fixture stacks four concurrent meetings, so lanes always collapse.
    await expect(covered.first()).toBeVisible();

    const strips = await page.evaluate(() => {
      const cards = Array.from(
        document.querySelectorAll<HTMLElement>('.event-card[data-overlapped="true"]'),
      ).map((card) => ({ card, box: card.getBoundingClientRect() }));
      let worst = Infinity;
      for (const a of cards) {
        // Only a card that overlaps in time can cover another one.
        const covering = cards.filter(
          (b) =>
            b !== a &&
            b.card.closest(".day-column") === a.card.closest(".day-column") &&
            b.box.left > a.box.left &&
            b.box.top < a.box.bottom &&
            a.box.top < b.box.bottom,
        );
        for (const b of covering) worst = Math.min(worst, b.box.left - a.box.left);
      }
      return worst;
    });
    // Every covered card keeps a visible, clickable band of its own.
    expect(Number.isFinite(strips)).toBe(true);
    expect(strips).toBeGreaterThanOrEqual(14);

    const first = covered.first();
    const collapsed = (await first.boundingBox())!.width;
    // Hovering the exposed strip proves the covered card is still reachable;
    // its centre is deliberately under the next card.
    await first.hover({ position: { x: 6, y: 10 } });
    await page.waitForTimeout(250);
    expect((await first.boundingBox())!.width).toBeGreaterThanOrEqual(collapsed);
  });

  test("keyboard focus reaches every card and opens a covered one in full", async ({ page }) => {
    await openCalendar(page, "stacked");
    // Day view keeps the tab walk short and exercises the same packing code.
    await view(page, "Day").click();
    await page.waitForTimeout(400);

    const cards = page.locator(".day-column .event-card:not([disabled])");
    const total = await cards.count();
    expect(total).toBe(perDay("stacked"));

    await cards.first().focus();
    let reached = 1;
    for (let i = 1; i < total; i += 1) {
      await page.keyboard.press("Tab");
      const stillInGrid = await page.evaluate(
        () => !!document.activeElement?.closest(".day-column"),
      );
      if (!stillInGrid) break;
      reached += 1;
    }
    // Tab order walks the whole grid: nothing is unreachable by keyboard.
    expect(reached).toBe(total);

    const expansion = await page.evaluate(() => {
      const card = document.querySelector<HTMLElement>(
        '.day-column .event-card[data-overlapped="true"]',
      );
      if (!card) return null;
      const before = card.getBoundingClientRect().width;
      card.focus();
      return {
        before,
        after: card.getBoundingClientRect().width,
        raised: getComputedStyle(card).zIndex,
      };
    });
    if (expansion) {
      expect(expansion.after).toBeGreaterThanOrEqual(expansion.before);
      expect(Number(expansion.raised)).toBeGreaterThan(2);
    }
  });

  test("the day peek lists a whole day and returns focus on close", async ({ page, isMobile }) => {
    await openCalendar(page);
    await view(page, denseView(isMobile)).click();
    const chip = page.locator(".day-count").first();
    const expected = Number(await chip.textContent());
    expect(expected).toBe(perDay("normal"));
    await chip.click();

    const peek = page.getByRole("dialog");
    await expect(peek).toBeVisible();
    await expect(peek.locator(".day-peek-item")).toHaveCount(expected);
    await expect(view(page, denseView(isMobile))).toHaveAttribute("aria-pressed", "true");

    await page.keyboard.press("Escape");
    await expect(peek).toBeHidden();
    await expect(chip).toBeFocused();
  });

  test("month cells never hide a meeting without saying so", async ({ page }) => {
    await openCalendar(page, "dense");
    await view(page, "Month").click();
    await page.waitForTimeout(600);

    const audit = await page.evaluate(() => {
      const cell = document.querySelector<HTMLElement>(".month-cell.in-range");
      if (!cell) return null;
      const more = cell.querySelector<HTMLElement>(".more-events");
      return {
        chips: cell.querySelectorAll(".event-card").length,
        hidden: more ? Number((more.textContent ?? "").replace(/\D/g, "")) : 0,
        hasMore: !!more,
      };
    });
    expect(audit).not.toBeNull();
    // Whatever the cell can fit, the count of hidden meetings is declared.
    expect(audit!.chips + audit!.hidden).toBe(perDay("dense"));
  });

  test("a day column never hides a meeting the hour grid cannot draw", async ({ page }) => {
    await openCalendar(page, "offgrid");
    await view(page, "Day").click();
    await page.waitForTimeout(400);

    const column = page.locator(".day-column").first();
    const drawn = await column.locator(".event-card").count();
    const offGrid = page.locator(".day-offgrid").first();
    await expect(offGrid).toBeVisible();
    const hidden = Number((await offGrid.textContent())?.replace(/\D/g, "") ?? "0");

    expect(hidden).toBe(OFFGRID_HIDDEN);
    // Nothing is lost: what is drawn plus what is declared is the whole day.
    expect(drawn + hidden).toBe(perDay("offgrid"));

    await offGrid.click();
    const peek = page.getByRole("dialog");
    await expect(peek).toBeVisible();
    await expect(peek.locator(".day-peek-item")).toHaveCount(perDay("offgrid"));
    await page.keyboard.press("Escape");
    await expect(peek).toBeHidden();
  });

  test("month overflow opens the peek instead of changing the view", async ({ page, isMobile }) => {
    test.skip(!!isMobile, "narrow month cells grow to fit the day instead of truncating it");    await openCalendar(page, "dense");
    await view(page, "Month").click();
    await page.waitForTimeout(400);
    const more = page.locator(".more-events").first();
    // Eight meetings a day overflow a desktop month cell at any window height.
    await expect(more).toBeVisible();
    await expect(more).toHaveAttribute("aria-haspopup", "dialog");

    await more.click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(view(page, "Month")).toHaveAttribute("aria-pressed", "true");
  });

  test("the grid fits the viewport so its sticky header stays usable", async ({
    page,
    isMobile,
  }) => {
    test.skip(!!isMobile, "narrow viewports scroll the month grid with the page on purpose");
    await openCalendar(page);
    for (const name of ["Week", "Month"] as const) {
      await view(page, name).click();
      await page.waitForTimeout(300);
      const fits = await page.evaluate(() => {
        const node = document.querySelector<HTMLElement>(".week-scroll, .month-scroll");
        if (!node) return true;
        return node.getBoundingClientRect().bottom <= window.innerHeight + 1;
      });
      expect(fits, `${name} view overflows the viewport`).toBe(true);
    }
  });
});

test.describe("mobile drawer", () => {
  test.skip(({ isMobile }) => !isMobile, "drawer only exists on narrow viewports");

  test("picking a date in the drawer keeps the active view and closes the drawer", async ({
    page,
  }) => {
    await openCalendar(page);
    await view(page, "Work week").click();
    await page.getByRole("button", { name: "Open navigation" }).click();
    await expect(page.locator(".sidebar.open")).toBeVisible();

    await page
      .locator(".sidebar.open .mini-days button:not(.outside)")
      .nth(farDayOfMonth() - 1)
      .click();

    await expect(page.locator(".sidebar.open")).toHaveCount(0);
    await expect(view(page, "Work week")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator(".mini-days .in-range")).toHaveCount(5);
  });

  test("browsing the drawer's mini month leaves the main view alone", async ({ page }) => {
    await openCalendar(page);
    await view(page, "Week").click();
    const heading = await page.getByRole("heading", { level: 1 }).textContent();
    await page.getByRole("button", { name: "Open navigation" }).click();

    await page.getByRole("button", { name: "Next month" }).click();
    expect(await page.getByRole("heading", { level: 1 }).textContent()).toBe(heading);
    await expect(page.locator('.mini-days button[tabindex="0"]')).toHaveCount(1);
    await expect(page.locator(".sidebar.open")).toBeVisible();

    await page.getByRole("button", { name: /Back to selected/ }).click();
    await expect(page.locator(".mini-days .in-range")).toHaveCount(7);
  });

  test("the day peek becomes a full-width sheet", async ({ page }) => {
    await openCalendar(page);
    await view(page, "Week").click();
    await page.locator(".day-count").first().click();
    const peek = page.getByRole("dialog");
    await expect(peek).toBeVisible();
    const box = (await peek.boundingBox())!;
    expect(box.width).toBeGreaterThan(page.viewportSize()!.width - 4);
    // Touch targets in the sheet stay comfortably tappable.
    const item = (await peek.locator(".day-peek-item").first().boundingBox())!;
    expect(item.height).toBeGreaterThanOrEqual(44);
  });
});
