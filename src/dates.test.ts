import { describe, expect, it } from "vitest";
import {
  addMonths,
  dayKey,
  formatRangeLabel,
  hourWindow,
  isAllDayLike,
  isCurrentPeriod,
  isInRange,
  isSameDay,
  laneMetrics,
  layoutDayEvents,
  monthMatrix,
  packAllDayEvents,
  selectionRange,
  shiftAnchor,
  startOfWeek,
  visibleDays,
} from "./dates";

const at = (y: number, m: number, d: number, h = 0, min = 0) =>
  new Date(y, m - 1, d, h, min).toISOString();

describe("startOfWeek", () => {
  it("returns Monday for every weekday", () => {
    // 2026-09-02 is a Wednesday; the whole week must collapse to Aug 31.
    for (let day = 31; day <= 36; day += 1) {
      const date = new Date(2026, 7, day);
      expect(startOfWeek(date).toDateString()).toBe(new Date(2026, 7, 31).toDateString());
    }
  });

  it("treats Sunday as the last day of the week", () => {
    expect(startOfWeek(new Date(2026, 8, 6)).getDate()).toBe(31);
  });
});

describe("addMonths", () => {
  it("does not overflow into the next month", () => {
    const result = addMonths(new Date(2026, 0, 31), 1);
    expect(result.getMonth()).toBe(1);
    expect(result.getDate()).toBe(28);
  });

  it("moves backwards across a year boundary", () => {
    const result = addMonths(new Date(2026, 0, 15), -1);
    expect(result.getFullYear()).toBe(2025);
    expect(result.getMonth()).toBe(11);
  });
});

describe("shiftAnchor", () => {
  const anchor = new Date(2026, 8, 2);
  it("moves one day in day view", () => {
    expect(shiftAnchor(anchor, "day", 1).getDate()).toBe(3);
  });
  it("moves one week in week views", () => {
    expect(shiftAnchor(anchor, "week", 1).getDate()).toBe(9);
    expect(shiftAnchor(anchor, "workweek", -1).getDate()).toBe(26);
  });
  it("moves one calendar month in month view, not 30 days", () => {
    const next = shiftAnchor(anchor, "month", 1);
    expect(next.getMonth()).toBe(9);
    expect(next.getDate()).toBe(2);
  });
});

describe("visibleDays", () => {
  it("returns 1, 5 and 7 days and always starts on Monday", () => {
    const anchor = new Date(2026, 8, 2);
    expect(visibleDays(anchor, "day")).toHaveLength(1);
    expect(visibleDays(anchor, "workweek")).toHaveLength(5);
    expect(visibleDays(anchor, "week")).toHaveLength(7);
    expect(visibleDays(anchor, "week")[0].getDay()).toBe(1);
    expect(visibleDays(anchor, "workweek")[0].getDate()).toBe(31);
  });
});

describe("monthMatrix", () => {
  it("emits 6 aligned weeks so every month fits", () => {
    const cells = monthMatrix(new Date(2026, 8, 2));
    expect(cells).toHaveLength(42);
    expect(cells[0].getDay()).toBe(1);
    // September 2026 starts on a Tuesday, so the 1st must sit in column 2.
    expect(cells[1].getDate()).toBe(1);
    expect(cells[1].getMonth()).toBe(8);
  });

  it("covers months that span six weeks", () => {
    const cells = monthMatrix(new Date(2026, 4, 1));
    const inMonth = cells.filter((d) => d.getMonth() === 4);
    expect(inMonth).toHaveLength(31);
  });
});

describe("formatRangeLabel", () => {
  it("shows a date range for week views so navigation is visible", () => {
    expect(formatRangeLabel(new Date(2026, 8, 2), "workweek")).toBe("Aug 31 – Sep 4, 2026");
    expect(formatRangeLabel(new Date(2026, 8, 9), "workweek")).toBe("Sep 7 – 11, 2026");
  });
  it("shows the full day in day view and month plus year in month view", () => {
    expect(formatRangeLabel(new Date(2026, 8, 2), "day")).toBe("Wednesday, September 2, 2026");
    expect(formatRangeLabel(new Date(2026, 8, 2), "month")).toBe("September 2026");
  });
  it("produces a different label for adjacent periods", () => {
    const a = formatRangeLabel(new Date(2026, 8, 2), "week");
    const b = formatRangeLabel(shiftAnchor(new Date(2026, 8, 2), "week", 1), "week");
    expect(a).not.toBe(b);
  });
});

describe("hourWindow", () => {
  it("falls back to business hours when nothing is scheduled", () => {
    expect(hourWindow([])).toEqual({ start: 8, end: 18 });
  });

  it("bounds extreme meetings so the working grid stays usable", () => {
    const window = hourWindow([
      { start: at(2026, 9, 2, 2, 30), end: at(2026, 9, 2, 3, 30) },
      { start: at(2026, 9, 2, 20, 0), end: at(2026, 9, 2, 21, 30) },
    ]);
    expect(window.start).toBe(6);
    expect(window.end).toBe(22);
  });

  it("does not let an isolated overnight item expand the working grid", () => {
    const events = [{ start: at(2026, 9, 2, 2, 30), end: at(2026, 9, 2, 3, 30) }];
    expect(hourWindow(events)).toEqual({ start: 6, end: 18 });
  });

  it("ignores unparseable timestamps", () => {
    expect(hourWindow([{ start: "nope", end: "nope" }])).toEqual({ start: 8, end: 18 });
  });
});

describe("layoutDayEvents", () => {
  const window = { start: 8, end: 18 };

  it("puts non-overlapping events in a single full-width lane", () => {
    const laid = layoutDayEvents(
      [
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 10) },
        { start: at(2026, 9, 2, 11), end: at(2026, 9, 2, 12) },
      ],
      window,
      72,
    );
    expect(laid.every((item) => item.lanes === 1)).toBe(true);
  });

  it("splits overlapping events into side-by-side lanes", () => {
    const laid = layoutDayEvents(
      [
        { start: at(2026, 9, 2, 13), end: at(2026, 9, 2, 14, 30) },
        { start: at(2026, 9, 2, 14), end: at(2026, 9, 2, 15) },
      ],
      window,
      72,
    );
    expect(laid.map((item) => item.lanes)).toEqual([2, 2]);
    expect(laid.map((item) => item.lane)).toEqual([0, 1]);
  });

  it("reuses a lane once it is free instead of one lane per meeting", () => {
    // Six meetings in one chain, but never more than three at the same moment.
    const laid = layoutDayEvents(
      [
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 12) },
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 10) },
        { start: at(2026, 9, 2, 9, 30), end: at(2026, 9, 2, 10, 30) },
        { start: at(2026, 9, 2, 10), end: at(2026, 9, 2, 11) },
        { start: at(2026, 9, 2, 11), end: at(2026, 9, 2, 12) },
        { start: at(2026, 9, 2, 11, 30), end: at(2026, 9, 2, 13) },
      ],
      window,
      72,
    );
    expect(Math.max(...laid.map((item) => item.lanes))).toBe(3);
  });

  it("widens an event into neighbouring lanes that stay free", () => {
    const laid = layoutDayEvents(
      [
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 13) },
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 10) },
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 10) },
        { start: at(2026, 9, 2, 11), end: at(2026, 9, 2, 12) },
      ],
      window,
      72,
    );
    const late = laid.find((item) => new Date(item.event.start).getHours() === 11);
    expect(late?.lanes).toBe(3);
    expect(late?.span).toBe(2);
  });

  it("never lets two events widen onto the same lane at the same time", () => {
    const laid = layoutDayEvents(
      [
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 11) },
        { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 10) },
        { start: at(2026, 9, 2, 9, 30), end: at(2026, 9, 2, 11) },
        { start: at(2026, 9, 2, 10), end: at(2026, 9, 2, 11) },
      ],
      window,
      72,
    );
    for (const a of laid) {
      for (const b of laid) {
        if (a === b) continue;
        const overlapsInTime = a.top < b.top + b.height && b.top < a.top + a.height;
        const overlapsInLane = a.lane < b.lane + b.span && b.lane < a.lane + a.span;
        expect(overlapsInTime && overlapsInLane).toBe(false);
      }
    }
  });

  it("positions and sizes events against the window", () => {
    const [item] = layoutDayEvents(
      [{ start: at(2026, 9, 2, 9, 30), end: at(2026, 9, 2, 10, 30) }],
      window,
      72,
    );
    expect(item.top).toBe(108);
    expect(item.height).toBe(68);
  });

  it("enforces a minimum height for very short meetings", () => {
    const [item] = layoutDayEvents(
      [{ start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 9, 5) }],
      window,
      72,
    );
    expect(item.height).toBeGreaterThanOrEqual(26);
  });

  it("clamps a multi-day event to the visible portion of each day", () => {
    const event = { start: at(2026, 9, 2, 16), end: at(2026, 9, 4, 10) };
    const middle = layoutDayEvents([event], window, 72, 26, new Date(2026, 8, 3));
    expect(middle[0].top).toBe(0);
    expect(middle[0].height).toBe(716);
  });
});

describe("laneMetrics", () => {
  it("tiles evenly while each lane stays readable", () => {
    const metrics = laneMetrics(3, 600);
    expect(metrics.overlapped).toBe(false);
    expect(metrics.cardWidth).toBeCloseTo(198, 0);
  });

  it("overlaps instead of splitting into unreadable slivers", () => {
    const metrics = laneMetrics(4, 154);
    expect(metrics.overlapped).toBe(true);
    expect(metrics.cardWidth).toBeGreaterThan(154 / 4);
  });

  it("always leaves a clickable strip of every covered card", () => {
    for (const lanes of [2, 3, 4, 5]) {
      for (const width of [145, 154, 216, 320]) {
        const metrics = laneMetrics(lanes, width);
        if (!metrics.overlapped) continue;
        expect(metrics.step).toBeGreaterThanOrEqual(14);
        // The rightmost lane still ends inside the column.
        expect(metrics.cardWidth + metrics.step * (lanes - 1)).toBeLessThanOrEqual(width - 5);
      }
    }
  });

  it("reports nothing to lay out when the column has not been measured", () => {
    expect(laneMetrics(3, 0).overlapped).toBe(false);
  });
});

describe("all-day handling", () => {
  it("treats long and midnight-crossing bookings as all-day", () => {
    expect(isAllDayLike({ start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 17) })).toBe(false);
    expect(isAllDayLike({ start: at(2026, 9, 2, 6), end: at(2026, 9, 2, 20) })).toBe(true);
    expect(isAllDayLike({ start: at(2026, 9, 2, 16), end: at(2026, 9, 3, 10) })).toBe(true);
  });

  it("does not treat a meeting ending exactly at midnight as multi-day", () => {
    expect(isAllDayLike({ start: at(2026, 9, 2, 22), end: at(2026, 9, 3, 0) })).toBe(false);
  });

  it("keeps an all-day booking out of the timed grid's hour window", () => {
    const events = [
      { start: at(2026, 9, 2, 0), end: at(2026, 9, 2, 23, 59) },
      { start: at(2026, 9, 2, 9), end: at(2026, 9, 2, 10) },
    ];
    expect(hourWindow(events)).toEqual({ start: 8, end: 18 });
  });

  it("spans a multi-day bar across the days it covers", () => {
    const days = Array.from({ length: 7 }, (_, i) => new Date(2026, 8, 7 + i));
    const [bar] = packAllDayEvents([{ start: at(2026, 9, 8, 9), end: at(2026, 9, 10, 17) }], days);
    expect(bar.startIndex).toBe(1);
    expect(bar.span).toBe(3);
    expect(bar.continuesBefore).toBe(false);
  });

  it("clips a bar that starts before the visible week and flags it", () => {
    const days = Array.from({ length: 5 }, (_, i) => new Date(2026, 8, 7 + i));
    const [bar] = packAllDayEvents([{ start: at(2026, 9, 4, 9), end: at(2026, 9, 9, 17) }], days);
    expect(bar.startIndex).toBe(0);
    expect(bar.continuesBefore).toBe(true);
  });

  it("stacks bars that share days onto separate rows", () => {
    const days = Array.from({ length: 7 }, (_, i) => new Date(2026, 8, 7 + i));
    const bars = packAllDayEvents(
      [
        { start: at(2026, 9, 7, 0), end: at(2026, 9, 9, 23) },
        { start: at(2026, 9, 8, 0), end: at(2026, 9, 10, 23) },
        { start: at(2026, 9, 11, 0), end: at(2026, 9, 12, 23) },
      ],
      days,
    );
    expect(bars.map((bar) => bar.row)).toEqual([0, 1, 0]);
  });
});

describe("selectionRange", () => {
  const anchor = new Date(2026, 8, 9); // Wednesday

  it("selects only the anchor in day view", () => {
    const range = selectionRange(anchor, "day");
    expect(range.start.getDate()).toBe(9);
    expect(range.end.getDate()).toBe(9);
  });

  it("selects the whole week containing the anchor", () => {
    const range = selectionRange(anchor, "week");
    expect(range.start.getDate()).toBe(7);
    expect(range.end.getDate()).toBe(13);
  });

  it("selects Monday to Friday in work week view", () => {
    const range = selectionRange(anchor, "workweek");
    expect(range.start.getDate()).toBe(7);
    expect(range.end.getDate()).toBe(11);
  });

  it("selects the whole calendar month in month view", () => {
    const range = selectionRange(anchor, "month");
    expect(range.start.getDate()).toBe(1);
    expect(range.end.getDate()).toBe(30);
  });

  it("marks a weekend day picked in work week view as outside the range", () => {
    const range = selectionRange(new Date(2026, 8, 13), "workweek");
    expect(isInRange(new Date(2026, 8, 13), range)).toBe(false);
    expect(isInRange(new Date(2026, 8, 11), range)).toBe(true);
  });

  it("keeps the range and the rendered days in step", () => {
    for (const view of ["day", "workweek", "week"] as const) {
      const range = selectionRange(anchor, view);
      const days = visibleDays(anchor, view);
      expect(days.every((day) => isInRange(day, range))).toBe(true);
      expect(days).toHaveLength(
        Math.round((range.end.getTime() - range.start.getTime()) / 86_400_000) + 1,
      );
    }
  });
});

describe("isCurrentPeriod", () => {
  // 2026-09-05 is a Saturday, 2026-09-06 a Sunday.
  const saturday = new Date(2026, 8, 5, 14, 30);
  const sunday = new Date(2026, 8, 6, 9, 0);

  it("treats a weekend today as inside the displayed work week", () => {
    for (const now of [saturday, sunday]) {
      const range = selectionRange(now, "workweek");
      expect(isInRange(now, range)).toBe(false); // the grid really does exclude it
      expect(isCurrentPeriod(range, "workweek", now)).toBe(true); // ...but it is still this period
    }
  });

  it("flags a neighbouring work week as off-period", () => {
    const previous = selectionRange(new Date(2026, 7, 26), "workweek");
    const next = selectionRange(new Date(2026, 8, 9), "workweek");
    expect(isCurrentPeriod(previous, "workweek", saturday)).toBe(false);
    expect(isCurrentPeriod(next, "workweek", saturday)).toBe(false);
  });

  it("agrees with the rendered period for every view", () => {
    for (const view of ["day", "workweek", "week", "month"] as const) {
      expect(isCurrentPeriod(selectionRange(saturday, view), view, saturday)).toBe(true);
      expect(isCurrentPeriod(selectionRange(new Date(2026, 10, 17), view), view, saturday)).toBe(
        false,
      );
    }
  });

  it("ignores the time of day", () => {
    const range = selectionRange(new Date(2026, 8, 5, 0, 0), "day");
    expect(isCurrentPeriod(range, "day", new Date(2026, 8, 5, 23, 59))).toBe(true);
  });
});

describe("mini calendar selection clamping", () => {
  // App renders the ring at `isInRange(anchor, range) ? anchor : range.start`;
  // the invariant is that the ring can never land outside the highlight.
  const clamp = (anchor: Date, view: "day" | "workweek" | "week" | "month") => {
    const range = selectionRange(anchor, view);
    return isInRange(anchor, range) ? anchor : range.start;
  };

  it("pulls a weekend anchor onto the Monday of the shown work week", () => {
    for (const day of [5, 6]) {
      const anchor = new Date(2026, 8, day);
      const range = selectionRange(anchor, "workweek");
      expect(isInRange(clamp(anchor, "workweek"), range)).toBe(true);
      expect(clamp(anchor, "workweek").getDate()).toBe(range.start.getDate());
    }
  });

  it("leaves an in-range anchor untouched in every view", () => {
    for (const view of ["day", "workweek", "week", "month"] as const) {
      const anchor = new Date(2026, 8, 9); // a Wednesday
      expect(clamp(anchor, view).getTime()).toBe(anchor.getTime());
    }
  });

  it("never lands outside the highlight for any day of the year", () => {
    for (const view of ["day", "workweek", "week", "month"] as const) {
      for (let offset = 0; offset < 366; offset += 1) {
        const anchor = new Date(2026, 0, 1 + offset);
        expect(isInRange(clamp(anchor, view), selectionRange(anchor, view))).toBe(true);
      }
    }
  });
});

describe("day identity helpers", () => {
  it("compares calendar days, not just the day number", () => {
    expect(isSameDay(new Date(2026, 8, 2), new Date(2026, 9, 2))).toBe(false);
    expect(isSameDay(new Date(2026, 8, 2, 23), new Date(2026, 8, 2, 1))).toBe(true);
  });

  it("produces distinct keys across months", () => {
    expect(dayKey(new Date(2026, 8, 2))).not.toBe(dayKey(new Date(2026, 9, 2)));
  });
});
