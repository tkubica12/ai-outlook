import { expect, test, type Page, type Route } from "@playwright/test";
import path from "node:path";
import { detail } from "../src/test-fixtures";

// Every request in this file is fulfilled in the browser, so the suite never
// reaches the local backend: no cache is written, no connector is called and no
// Copilot CLI job is queued. Assertions stay structural — the fixtures below
// carry no real customer, attendee or meeting content.

const ARTIFACTS = path.join(
  "C:",
  "Users",
  "tokubica",
  ".copilot",
  "session-state",
  "eb2acacb-20a7-418f-8b00-c477c24d6914",
  "files",
);

const shot = (page: Page, name: string, project: string) =>
  page.screenshot({ path: path.join(ARTIFACTS, `e2e-${name}-${project}.png`), fullPage: false });

interface CalendarStatusFixture {
  syncing: boolean;
  covered_dates: string[];
  requested_dates: string[];
  pending_dates: string[];
  syncing_dates: string[];
  stale_dates: string[];
  failures: Record<string, string>;
  last_synced_at: string | null;
  cache_warning: string | null;
  cache_error: string | null;
}

const emptyStatus = (overrides: Partial<CalendarStatusFixture> = {}): CalendarStatusFixture => ({
  syncing: false,
  covered_dates: [],
  requested_dates: [],
  pending_dates: [],
  syncing_dates: [],
  stale_dates: [],
  failures: {},
  last_synced_at: null,
  cache_warning: null,
  cache_error: null,
  ...overrides,
});

interface Harness {
  /** Every `start_date`/`end_date` pair the app asked for, in order. */
  ranges: { start: string | null; end: string | null }[];
  /** Any non-GET request the app made — the retry POST is the only legal one. */
  writes: { method: string; path: string }[];
  retries: number;
  status: CalendarStatusFixture;
  events: unknown[];
  /** Served for `/api/meetings/:id` when a test opens the detail panel. */
  meeting?: unknown;
}

const json = (route: Route, body: unknown) =>
  route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(body),
  });

async function install(page: Page, initial: Partial<Harness> = {}): Promise<Harness> {
  const harness: Harness = {
    ranges: [],
    writes: [],
    retries: 0,
    status: initial.status ?? emptyStatus(),
    events: initial.events ?? [],
    meeting: initial.meeting,
  };

  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const at = url.pathname;
    if (request.method() !== "GET") harness.writes.push({ method: request.method(), path: at });

    if (at === "/api/setup")
      return json(route, {
        ready: true,
        missing: [],
        configured_connectors: ["outlook"],
        instructions: [],
      });
    if (at === "/api/calendar/retry") {
      harness.retries += 1;
      harness.status = { ...harness.status, failures: {}, syncing: true, pending_dates: [] };
      return json(route, harness.status);
    }
    if (at === "/api/calendar") {
      harness.ranges.push({
        start: url.searchParams.get("start_date"),
        end: url.searchParams.get("end_date"),
      });
      return json(route, harness.events);
    }
    if (at === "/api/calendar-status") return json(route, harness.status);
    if (at.startsWith("/api/meetings/") && harness.meeting) return json(route, harness.meeting);
    if (at === "/api/analysis-status")
      return json(route, {
        total: 0,
        ready: 0,
        queued: 0,
        running: 0,
        failed: 0,
        not_started: 0,
        percent: 100,
        model: "test-model",
        concurrency: 4,
        meetings: [],
      });
    if (at === "/api/health")
      return json(route, { status: "ok", database: "ok", copilot_cli: "disabled" });
    if (at === "/api/connectors") return json(route, []);
    return json(route, {});
  });

  return harness;
}

const open = async (page: Page) => {
  await page.goto("/");
  await page.waitForSelector(".week-view, .month-grid, .calendar-sync", { timeout: 30_000 });
};

const days = (range: { start: string | null; end: string | null }) => {
  if (!range.start || !range.end) return 0;
  const start = Date.parse(`${range.start}T00:00:00`);
  const end = Date.parse(`${range.end}T00:00:00`);
  return Math.round((end - start) / 86_400_000);
};

const pick = (page: Page, name: string) => page.getByRole("button", { name, exact: true });

test.describe("calendar sync honesty", () => {
  test("a cold calendar reports syncing instead of an empty week", async ({ page }, info) => {
    const harness = await install(page, {
      status: emptyStatus({ syncing: true, pending_dates: ["2026-01-01"] }),
    });
    await open(page);

    const banner = page.getByTestId("calendar-sync");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(/Syncing calendar/i);
    await expect(page.getByText(/No meetings in this period/i)).toHaveCount(0);
    await expect(page.locator("[data-day-state='pending'], [data-day-state='unqueried']").first()).toBeVisible();
    await shot(page, "cold-sync", info.project.name);
    expect(harness.writes).toHaveLength(0);
  });

  test("a range that answered with zero events keeps being polled", async ({ page }) => {
    const harness = await install(page, { status: emptyStatus({ syncing: true }) });
    await open(page);
    const before = harness.ranges.length;
    await expect(async () => {
      expect(harness.ranges.length).toBeGreaterThan(before);
    }).toPass({ timeout: 15_000 });
    expect(harness.writes).toHaveLength(0);
  });

  test("failed days are named and retried only on request", async ({ page }, info) => {
    const today = new Date();
    const iso = `${today.getFullYear()}-${`${today.getMonth() + 1}`.padStart(2, "0")}-${`${today.getDate()}`.padStart(2, "0")}`;
    const harness = await install(page, {
      status: emptyStatus({ failures: { [iso]: "Graph request timed out" } }),
    });
    await open(page);

    const banner = page.getByTestId("calendar-sync");
    await expect(banner).toContainText("Graph request timed out");
    expect(harness.retries).toBe(0);
    await shot(page, "sync-failure", info.project.name);

    await banner.getByRole("button", { name: /Retry failed days/i }).click();
    await expect(async () => {
      expect(harness.retries).toBe(1);
    }).toPass({ timeout: 10_000 });
    // The retry is the only write this app is allowed to make.
    expect(harness.writes).toEqual([{ method: "POST", path: "/api/calendar/retry" }]);
  });

  test("a day that already shows meetings still says its sync failed", async ({ page }, info) => {
    // The dangerous case: cached meetings make a half-synced day look complete.
    const today = new Date();
    const iso = `${today.getFullYear()}-${`${today.getMonth() + 1}`.padStart(2, "0")}-${`${today.getDate()}`.padStart(2, "0")}`;
    const at = (hour: number) => {
      const stamp = new Date(today);
      stamp.setHours(hour, 0, 0, 0);
      return stamp.toISOString();
    };
    const harness = await install(page, {
      // Structural fixture only: no real meeting, customer or attendee content.
      events: [
        {
          id: "fixture-1",
          title: "Fixture meeting",
          start: at(9),
          end: at(10),
          category: "Internal",
          attendees: [],
          organizer: "",
          location: "",
          status: "accepted",
          briefing_status: "not_started",
          is_all_day: false,
          has_changes: false,
        },
      ],
      status: emptyStatus({
        covered_dates: [iso],
        requested_dates: [iso],
        failures: { [iso]: "Graph request timed out" },
      }),
    });
    await open(page);

    await expect(page.getByText("Fixture meeting").first()).toBeVisible();
    const note = page.locator('.day-sync-note[data-day-partial="true"]').first();
    await expect(note).toBeVisible();
    await expect(note).toContainText("may be incomplete");
    await expect(page.getByTestId("calendar-sync")).toContainText("Graph request timed out");
    await shot(page, "sync-partial-day", info.project.name);
    expect(harness.writes).toHaveLength(0);
  });

  test("failures the backend reports for other dates are counted, not named", async ({ page }) => {    // `/api/calendar-status` is global: it describes every date the backend has
    // ever visited, so the banner must not attribute a far-away failure to the
    // week on screen.
    const harness = await install(page, {
      status: emptyStatus({
        failures: {
          "2019-01-07": "Graph request timed out",
          "2019-01-08": "Graph request timed out",
        },
      }),
    });
    await open(page);

    const banner = page.getByTestId("calendar-sync");
    await expect(banner).toContainText("outside this range could not be loaded");
    await expect(banner).not.toContainText("2019-01-07");
    await expect(banner).toContainText("Retry failed days");
    expect(harness.writes).toHaveLength(0);
  });

  test("an event whose participant list was dropped is marked on the card", async ({
    page,
  }, info) => {
    // The connector can return a complete event with no participants at all, and
    // a card that looks normal would hide that the briefing missed those people.
    const today = new Date();
    const iso = `${today.getFullYear()}-${`${today.getMonth() + 1}`.padStart(2, "0")}-${`${today.getDate()}`.padStart(2, "0")}`;
    const at = (hour: number) => {
      const stamp = new Date(today);
      stamp.setHours(hour, 0, 0, 0);
      return stamp.toISOString();
    };
    const incomplete = {
      ...detail.event,
      id: "fixture-2",
      title: "Fixture meeting",
      start: at(11),
      end: at(12),
      organizer: "",
      location: "",
      attendees: [],
      briefing_status: "ready",
      metadata_warnings: ["Participant information was incomplete in the calendar response."],
    };
    const harness = await install(page, {
      // Structural fixture only: no real meeting, customer or attendee content.
      events: [incomplete],
      meeting: { ...detail, event: incomplete },
      status: emptyStatus({ covered_dates: [iso], requested_dates: [iso] }),
    });
    await open(page);

    const card = page.locator(".event-card", { hasText: "Fixture meeting" }).first();
    await expect(card).toBeVisible();
    await expect(card).toHaveAttribute("aria-label", /calendar details incomplete/);
    await expect(card.locator(".event-incomplete")).toBeVisible();

    await card.click();
    const panel = page.locator(".meeting-panel");
    await expect(panel.getByText("Participants could not be read")).toBeVisible();
    await expect(panel.locator(".attendee-warning")).toContainText("Check the original invitation");
    await shot(page, "participants-incomplete", info.project.name);
    expect(harness.writes).toHaveLength(0);
  });
});

test.describe("calendar range requests", () => {
  test("each view asks the backend for exactly its own visible range", async ({ page }, info) => {
    const harness = await install(page);
    await open(page);
    await expect(async () => {
      expect(harness.ranges.length).toBeGreaterThan(0);
    }).toPass({ timeout: 15_000 });

    const rangeAfter = async (label: string, expected: number) => {
      const button = pick(page, label);
      // Mobile boots into Day view, so selecting the active view is a no-op
      // and must not be mistaken for a missing request.
      const alreadyActive = (await button.getAttribute("aria-pressed")) === "true";
      const seen = harness.ranges.length;
      await button.click();
      if (!alreadyActive) {
        await expect(async () => {
          expect(harness.ranges.length).toBeGreaterThan(seen);
        }).toPass({ timeout: 15_000 });
      }
      const latest = harness.ranges[harness.ranges.length - 1];
      expect(latest.start).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      expect(latest.end).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      expect(days(latest)).toBe(expected);
    };

    await rangeAfter("Day", 1);
    await rangeAfter("Work week", 5);
    await rangeAfter("Week", 7);
    // The month grid is a fixed 6×7 matrix, which is also the backend ceiling.
    await rangeAfter("Month", 42);
    await shot(page, "month-range", info.project.name);
    expect(harness.writes).toHaveLength(0);
  });

  test("navigating periods moves the requested range without piling up loops", async ({ page }) => {
    const harness = await install(page);
    await open(page);
    await pick(page, "Week").click();
    await expect(async () => {
      expect(harness.ranges.length).toBeGreaterThan(0);
    }).toPass({ timeout: 15_000 });

    const before = harness.ranges[harness.ranges.length - 1];
    await page.getByRole("button", { name: "Next period" }).click();
    await expect(async () => {
      const latest = harness.ranges[harness.ranges.length - 1];
      expect(latest.start).not.toBe(before.start);
    }).toPass({ timeout: 15_000 });

    const moved = harness.ranges[harness.ranges.length - 1];
    expect(days(moved)).toBe(7);
    expect(Date.parse(`${moved.start}T00:00:00`)).toBeGreaterThan(
      Date.parse(`${before.start}T00:00:00`),
    );

    // A single self-rescheduling poll: two seconds of idling cannot produce a
    // burst of overlapping range requests.
    const settled = harness.ranges.length;
    await page.waitForTimeout(2500);
    expect(harness.ranges.length - settled).toBeLessThanOrEqual(3);
    expect(harness.writes).toHaveLength(0);
  });
});
