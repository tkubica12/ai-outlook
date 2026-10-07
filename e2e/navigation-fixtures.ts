import type { Page, Route } from "@playwright/test";

/**
 * Deterministic calendar fixtures for the navigation suite.
 *
 * Every response is fulfilled in the browser, so these tests never reach the
 * local backend: no cache is written, no connector is called and no Copilot CLI
 * job is queued. The events are synthetic placeholders — no real meeting,
 * customer or attendee content — and they are generated for whatever range the
 * app asks for, so the assertions hold on any calendar date.
 */

export const dayKey = (date: Date) =>
  `${date.getFullYear()}-${`${date.getMonth() + 1}`.padStart(2, "0")}-${`${date.getDate()}`.padStart(2, "0")}`;

/** Local-time ISO stamp: `toISOString()` would shift the day in most zones. */
const at = (date: Date, hour: number, minute: number) => {
  const stamp = new Date(date);
  stamp.setHours(hour, minute, 0, 0);
  const offset = -stamp.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  const pad = (value: number) => `${Math.floor(Math.abs(value))}`.padStart(2, "0");
  return `${dayKey(stamp)}T${pad(stamp.getHours())}:${pad(stamp.getMinutes())}:00${sign}${pad(offset / 60)}:${pad(offset % 60)}`;
};

/** Start/end pairs in minutes-from-midnight, applied to every day in range. */
const NORMAL: [number, number][] = [
  [9 * 60, 9 * 60 + 30],
  [9 * 60 + 15, 10 * 60 + 15],
  [13 * 60, 14 * 60],
];

/**
 * Four concurrent meetings, so lane packing has to collapse the cards on both
 * the desktop week grid and the mobile day column.
 */
const STACKED: [number, number][] = [
  [9 * 60, 10 * 60],
  [9 * 60, 10 * 60],
  [9 * 60, 10 * 60],
  [9 * 60, 10 * 60],
  [13 * 60, 14 * 60],
];

/** Enough meetings per day to overflow a month cell on any viewport. */
const DENSE: [number, number][] = [
  [8 * 60, 8 * 60 + 30],
  [9 * 60, 9 * 60 + 30],
  [9 * 60 + 15, 10 * 60 + 15],
  [11 * 60, 11 * 60 + 30],
  [12 * 60, 12 * 60 + 30],
  [13 * 60, 14 * 60],
  [15 * 60, 15 * 60 + 30],
  [16 * 60, 17 * 60],
];

export type Density = "normal" | "stacked" | "dense" | "offgrid";

/**
 * Two meetings the readable 06:00–22:00 grid cannot draw, alongside ones it
 * can, so the day header has to admit the difference.
 */
const OFFGRID: [number, number][] = [
  [2 * 60, 3 * 60],
  [9 * 60, 10 * 60],
  [13 * 60, 14 * 60],
  [23 * 60, 23 * 60 + 45],
];

/** Meetings in an `offgrid` day that fall outside the drawn hour window. */
export const OFFGRID_HIDDEN = 2;

const SLOTS: Record<Density, [number, number][]> = {
  normal: NORMAL,
  stacked: STACKED,
  dense: DENSE,
  offgrid: OFFGRID,
};

/** Meetings the fixture puts on every single day, whatever the range. */
export const perDay = (density: Density) => SLOTS[density].length;

const CATEGORIES = ["Internal", "Customer", "Focus"];

export function eventsForRange(start: string, end: string, density: Density = "normal") {
  const slots = SLOTS[density];
  const events: Record<string, unknown>[] = [];
  const cursor = new Date(`${start}T00:00:00`);
  const last = new Date(`${end}T00:00:00`);
  // `end` is exclusive, and the app never asks for more than 42 days.
  for (let guard = 0; cursor < last && guard < 45; guard += 1) {
    const key = dayKey(cursor);
    slots.forEach(([from, to], index) => {
      events.push({
        id: `${key}-${index}`,
        title: `Test event ${index + 1}`,
        start: at(cursor, Math.floor(from / 60), from % 60),
        end: at(cursor, Math.floor(to / 60), to % 60),
        category: CATEGORIES[index % CATEGORIES.length],
        attendees: [],
        organizer: "Test organizer",
        location: "Test room",
        status: "accepted",
        briefing_status: "ready",
        is_all_day: false,
        has_changes: false,
      });
    });
    cursor.setDate(cursor.getDate() + 1);
  }
  return events;
}

export interface NavigationHarness {
  /** Any non-GET request the app made; navigation must never write. */
  writes: { method: string; path: string }[];
  ranges: { start: string | null; end: string | null }[];
}

const json = (route: Route, body: unknown) =>
  route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });

export async function installCalendar(
  page: Page,
  density: Density = "normal",
): Promise<NavigationHarness> {
  const harness: NavigationHarness = { writes: [], ranges: [] };

  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const endpoint = url.pathname;
    if (request.method() !== "GET") harness.writes.push({ method: request.method(), path: endpoint });

    if (endpoint === "/api/setup")
      return json(route, {
        ready: true,
        missing: [],
        configured_connectors: ["Outlook Calendar"],
        instructions: [],
      });
    if (endpoint === "/api/calendar") {
      const start = url.searchParams.get("start_date");
      const end = url.searchParams.get("end_date");
      harness.ranges.push({ start, end });
      return json(route, start && end ? eventsForRange(start, end, density) : []);
    }
    if (endpoint === "/api/calendar-status") {
      // Every date the app has asked for is reported as fully synced, so no day
      // is painted as pending and the navigation assertions stay stable.
      const covered = new Set<string>();
      for (const range of harness.ranges) {
        if (!range.start || !range.end) continue;
        const cursor = new Date(`${range.start}T00:00:00`);
        const last = new Date(`${range.end}T00:00:00`);
        for (let guard = 0; cursor < last && guard < 45; guard += 1) {
          covered.add(dayKey(cursor));
          cursor.setDate(cursor.getDate() + 1);
        }
      }
      const dates = [...covered];
      return json(route, {
        syncing: false,
        covered_dates: dates,
        requested_dates: dates,
        pending_dates: [],
        syncing_dates: [],
        stale_dates: [],
        failures: {},
        last_synced_at: new Date().toISOString(),
        cache_warning: null,
        cache_error: null,
      });
    }
    if (endpoint === "/api/analysis-status")
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
    if (endpoint === "/api/health")
      return json(route, { status: "ok", database: "ok", copilot_cli: "disabled" });
    if (endpoint === "/api/connectors") return json(route, []);
    return json(route, {});
  });

  return harness;
}
