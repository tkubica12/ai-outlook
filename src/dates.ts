export type ViewMode = "day" | "workweek" | "week" | "month";

export const MS_PER_HOUR = 3_600_000;

/** Human noun for the range a view selects, used in accessible descriptions. */
export const VIEW_RANGE_NOUN: Record<ViewMode, string> = {
  day: "day",
  workweek: "work week",
  week: "week",
  month: "month",
};

export function startOfDay(date: Date): Date {
  const copy = new Date(date);
  copy.setHours(0, 0, 0, 0);
  return copy;
}

export function startOfMonth(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), 1);
}

export function endOfMonth(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth() + 1, 0);
}

export function addDays(date: Date, amount: number): Date {
  const copy = new Date(date);
  copy.setDate(copy.getDate() + amount);
  return copy;
}

/** Adds months without the "31 Jan + 1 month = 3 Mar" overflow. */
export function addMonths(date: Date, amount: number): Date {
  const copy = new Date(date);
  const targetDay = copy.getDate();
  copy.setDate(1);
  copy.setMonth(copy.getMonth() + amount);
  const lastDay = new Date(copy.getFullYear(), copy.getMonth() + 1, 0).getDate();
  copy.setDate(Math.min(targetDay, lastDay));
  return copy;
}

/** Monday-first start of the week containing `date`. */
export function startOfWeek(date: Date): Date {
  const copy = startOfDay(date);
  const mondayOffset = (copy.getDay() + 6) % 7;
  return addDays(copy, -mondayOffset);
}

export function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** Stable per-day identity for lookups (local time, not UTC). */
export const dayKey = (date: Date) =>
  `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;

export function isSameMonth(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth();
}

/** Moves the anchor by one unit of the active view. */
export function shiftAnchor(anchor: Date, view: ViewMode, direction: number): Date {
  if (view === "day") return addDays(anchor, direction);
  if (view === "month") return addMonths(anchor, direction);
  return addDays(anchor, direction * 7);
}

/** The consecutive days rendered by the day/work week/week grids. */
export function visibleDays(anchor: Date, view: ViewMode): Date[] {
  if (view === "day") return [startOfDay(anchor)];
  const monday = startOfWeek(anchor);
  const length = view === "workweek" ? 5 : 7;
  return Array.from({ length }, (_, index) => addDays(monday, index));
}

/**
 * Six weeks of Monday-first cells covering the anchor's month, so every month
 * fits and cells always line up with their weekday column.
 */
export function monthMatrix(anchor: Date): Date[] {
  const firstOfMonth = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const gridStart = startOfWeek(firstOfMonth);
  return Array.from({ length: 42 }, (_, index) => addDays(gridStart, index));
}

export interface DateRange {
  start: Date;
  end: Date;
}

/**
 * The inclusive day range a view keeps selected for a given anchor. Every
 * surface (mini calendar, grid, heading) derives its highlight from this, so a
 * date picked in one place cannot mean something different in another.
 */
export function selectionRange(anchor: Date, view: ViewMode): DateRange {
  if (view === "day") return { start: startOfDay(anchor), end: startOfDay(anchor) };
  if (view === "month") {
    return { start: startOfMonth(anchor), end: startOfDay(endOfMonth(anchor)) };
  }
  const days = visibleDays(anchor, view);
  return { start: days[0], end: days[days.length - 1] };
}

export function isInRange(date: Date, range: DateRange): boolean {
  const value = startOfDay(date).getTime();
  return value >= startOfDay(range.start).getTime() && value <= startOfDay(range.end).getTime();
}

/** `YYYY-MM-DD` in local time — the calendar day the backend keys shards by. */
export function isoDate(date: Date): string {
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** Largest window the calendar API accepts, and exactly one month grid. */
export const MAX_FETCH_DAYS = 42;

export interface FetchRange {
  start: Date;
  /** Exclusive: the first day *after* the window, matching the API contract. */
  end: Date;
  /** Every day the request covers, in order. */
  days: Date[];
  start_date: string;
  end_date: string;
}

/**
 * The days the API must be asked for so the visible grid is never rendered from
 * a window the backend was never told about. Month deliberately covers the full
 * six-week matrix, including the leading and trailing days of adjacent months
 * that the grid actually paints, which is exactly the 42-day API ceiling.
 */
export function fetchRange(anchor: Date, view: ViewMode): FetchRange {
  const days = view === "month" ? monthMatrix(anchor) : visibleDays(anchor, view);
  const bounded = days.slice(0, MAX_FETCH_DAYS);
  const start = bounded[0];
  const end = addDays(bounded[bounded.length - 1], 1);
  return {
    start,
    end,
    days: bounded,
    start_date: isoDate(start),
    end_date: isoDate(end),
  };
}

/**
 * Whether `range` is the period the given moment falls into for this view.
 * Point-in-range is the wrong test for a period-level badge: in Work week the
 * grid legitimately excludes Saturday and Sunday, so a weekend "today" would
 * read as off-period the instant the user pressed Today.
 */
export function isCurrentPeriod(range: DateRange, view: ViewMode, now: Date = new Date()): boolean {
  const current = selectionRange(now, view);
  return (
    startOfDay(current.start).getTime() === startOfDay(range.start).getTime() &&
    startOfDay(current.end).getTime() === startOfDay(range.end).getTime()
  );
}

export function formatRangeLabel(anchor: Date, view: ViewMode, locale = "en"): string {
  if (view === "month") {
    return new Intl.DateTimeFormat(locale, { month: "long", year: "numeric" }).format(anchor);
  }
  if (view === "day") {
    return new Intl.DateTimeFormat(locale, {
      weekday: "long",
      month: "long",
      day: "numeric",
      year: "numeric",
    }).format(anchor);
  }
  const days = visibleDays(anchor, view);
  const first = days[0];
  const last = days[days.length - 1];
  const year = new Intl.DateTimeFormat(locale, { year: "numeric" }).format(last);
  const sameMonth = isSameMonth(first, last);
  const startPart = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
  }).format(first);
  const endPart = new Intl.DateTimeFormat(locale, {
    ...(sameMonth ? {} : { month: "short" }),
    day: "numeric",
  }).format(last);
  return `${startPart} – ${endPart}, ${year}`;
}

export interface HourWindow {
  start: number;
  end: number;
}

const DEFAULT_WINDOW: HourWindow = { start: 8, end: 18 };

/**
 * Meetings at or beyond this length, and anything crossing midnight, are shown
 * in the all-day band instead of the timed grid. A 12h+ block occupies a whole
 * lane for the entire day, which used to push every ordinary meeting on that
 * day into an unreadable sliver.
 */
export const ALL_DAY_MIN_HOURS = 12;

export function isAllDayLike(event: { start: string; end: string }): boolean {
  const from = new Date(event.start);
  const to = new Date(event.end);
  if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) return false;
  if (to.getTime() - from.getTime() >= ALL_DAY_MIN_HOURS * MS_PER_HOUR) return true;
  // An event ending exactly at midnight still belongs to the day before it.
  return !isSameDay(from, new Date(Math.max(from.getTime(), to.getTime() - 1)));
}

/**
 * Widens the default 08:00–18:00 grid so events outside business hours stay
 * visible. Without this, positions are negative or overflow and events vanish —
 * which happens for any viewer whose timezone differs from the data's.
 */
export function hourWindow(
  events: { start: string; end: string }[],
  fallback: HourWindow = DEFAULT_WINDOW,
): HourWindow {
  const sameDay = events.filter((event) => {
    const from = new Date(event.start);
    const to = new Date(event.end);
    return isSameDay(from, to) && !isAllDayLike(event);
  });
  let start = fallback.start;
  let end = fallback.end;
  for (const event of sameDay) {
    const from = new Date(event.start);
    const to = new Date(event.end);
    if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) continue;
    start = Math.min(start, from.getHours());
    // A 09:00–10:00 event needs the grid to reach 10:00; round partial hours up.
    const endHour = to.getHours() + (to.getMinutes() > 0 ? 1 : 0);
    end = Math.max(end, endHour);
  }
  return { start: Math.max(6, start), end: Math.min(22, Math.max(end, start + 1)) };
}

/** Fractional hours between the window start and `date`. */
export function hoursFromWindowStart(date: Date, window: HourWindow): number {
  return date.getHours() + date.getMinutes() / 60 - window.start;
}

export interface PositionedEvent<T> {
  event: T;
  top: number;
  height: number;
  /** Zero-based packing lane inside the overlap cluster. */
  lane: number;
  /** Total lanes the cluster needs. */
  lanes: number;
  /** How many consecutive lanes the event may widen into. */
  span: number;
}

interface Slot<T> {
  item: PositionedEvent<T>;
  from: number;
  to: number;
}

/** Greedy first-fit lane packing plus rightward expansion into free lanes. */
function packCluster<T>(cluster: Slot<T>[]): void {
  const laneEnds: number[] = [];
  for (const slot of cluster) {
    let lane = laneEnds.findIndex((end) => end <= slot.from + 1e-9);
    if (lane === -1) {
      lane = laneEnds.length;
      laneEnds.push(slot.to);
    } else {
      laneEnds[lane] = slot.to;
    }
    slot.item.lane = lane;
  }
  for (const slot of cluster) {
    let span = 1;
    while (slot.item.lane + span < laneEnds.length) {
      const blocked = cluster.some(
        (other) =>
          other !== slot &&
          other.item.lane === slot.item.lane + span &&
          other.from < slot.to &&
          other.to > slot.from,
      );
      if (blocked) break;
      span += 1;
    }
    slot.item.lanes = laneEnds.length;
    slot.item.span = span;
  }
}

/**
 * Lays out concurrent events side by side instead of stacking them on top of
 * each other. Events are packed into the fewest lanes that keep them apart —
 * a cluster of six meetings that never all overlap at once needs three lanes,
 * not six — and each event then widens into any neighbouring lane that stays
 * free for its whole duration.
 */
export function layoutDayEvents<T extends { start: string; end: string }>(
  events: T[],
  window: HourWindow,
  hourHeight: number,
  minHeight = 26,
  day?: Date,
): PositionedEvent<T>[] {
  const slots: Slot<T>[] = [];
  for (const event of events) {
    const sourceFrom = new Date(event.start);
    const sourceTo = new Date(event.end);
    if (Number.isNaN(sourceFrom.getTime()) || Number.isNaN(sourceTo.getTime())) continue;
    const dayStart = day ? startOfDay(day) : startOfDay(sourceFrom);
    const windowStart = new Date(dayStart);
    windowStart.setHours(window.start, 0, 0, 0);
    const windowEnd = new Date(dayStart);
    windowEnd.setHours(window.end, 0, 0, 0);
    const from = new Date(Math.max(sourceFrom.getTime(), windowStart.getTime()));
    const to = new Date(Math.min(sourceTo.getTime(), windowEnd.getTime()));
    if (to <= from) continue;
    const startHours = hoursFromWindowStart(from, window);
    const endHours = hoursFromWindowStart(to, window);
    slots.push({
      from: startHours,
      to: endHours,
      item: {
        event,
        top: startHours * hourHeight,
        height: Math.max(minHeight, (endHours - startHours) * hourHeight - 4),
        lane: 0,
        lanes: 1,
        span: 1,
      },
    });
  }
  // Longer events first on a tie so they take the leftmost lane and the shorter
  // ones can widen over them, which reads the way people scan a day.
  slots.sort((a, b) => a.from - b.from || b.to - a.to);

  const positioned: PositionedEvent<T>[] = [];
  let cluster: Slot<T>[] = [];
  let clusterEnd = -Infinity;
  const flush = () => {
    if (cluster.length > 0) {
      packCluster(cluster);
      positioned.push(...cluster.map((slot) => slot.item));
    }
    cluster = [];
    clusterEnd = -Infinity;
  };
  for (const slot of slots) {
    if (slot.from >= clusterEnd) flush();
    clusterEnd = Math.max(clusterEnd, slot.to);
    cluster.push(slot);
  }
  flush();
  return positioned;
}

export interface LaneMetrics {
  /** Width of a single-lane card in px, or 0 when the column is unmeasured. */
  cardWidth: number;
  /** Horizontal distance between consecutive lanes in px. */
  step: number;
  /** True when lanes deliberately overlap to stay readable. */
  overlapped: boolean;
}

/** Narrowest card that can still show an ellipsised title plus its accent. */
const MIN_CARD_WIDTH = 56;
/** Narrowest strip of a covered card that must stay visible and clickable. */
const MIN_PEEK = 18;

/**
 * Chooses between tiling and controlled overlap for one cluster.
 *
 * Tiling equally is only readable while each lane stays above a legible width.
 * Below that, splitting further produces two-character columns, so lanes are
 * overlapped instead: the front card keeps a readable width and every card
 * behind it still exposes a strip — its coloured accent plus background — that
 * remains visible and clickable. Nothing is hidden behind a "+N" either way,
 * and the per-day peek is the equivalent full-size route to every meeting.
 */
export function laneMetrics(lanes: number, dayWidth: number, gap = 6): LaneMetrics {
  const usable = dayWidth - gap;
  if (lanes <= 1 || usable <= 0) {
    return { cardWidth: Math.max(0, usable), step: Math.max(0, usable), overlapped: false };
  }
  const readable = Math.min(96, Math.max(72, usable * 0.58));
  const slot = usable / lanes;
  if (slot >= readable) return { cardWidth: slot, step: slot, overlapped: false };
  const cardWidth = Math.max(
    MIN_CARD_WIDTH,
    Math.min(readable, usable - MIN_PEEK * (lanes - 1)),
  );
  return { cardWidth, step: (usable - cardWidth) / (lanes - 1), overlapped: true };
}

export interface AllDayPlacement<T> {
  event: T;
  /** Index into the visible days where the bar starts. */
  startIndex: number;
  /** Number of visible days the bar covers. */
  span: number;
  /** Stacking row inside the all-day band. */
  row: number;
  continuesBefore: boolean;
  continuesAfter: boolean;
}

/** Stacks multi-day bars into the fewest rows without overlapping horizontally. */
export function packAllDayEvents<T extends { start: string; end: string }>(
  events: T[],
  days: Date[],
): AllDayPlacement<T>[] {
  if (days.length === 0) return [];
  const first = startOfDay(days[0]).getTime();
  const last = startOfDay(days[days.length - 1]).getTime();
  const dayIndex = (time: number) =>
    Math.round((startOfDay(new Date(time)).getTime() - first) / 86_400_000);

  const placements = events
    .map((event) => {
      const from = new Date(event.start);
      const to = new Date(event.end);
      if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) return null;
      const lastCovered = new Date(Math.max(from.getTime(), to.getTime() - 1));
      if (startOfDay(lastCovered).getTime() < first || startOfDay(from).getTime() > last) {
        return null;
      }
      const rawStart = dayIndex(from.getTime());
      const rawEnd = dayIndex(lastCovered.getTime());
      const startIndex = Math.max(0, rawStart);
      const endIndex = Math.min(days.length - 1, rawEnd);
      return {
        event,
        startIndex,
        span: endIndex - startIndex + 1,
        row: 0,
        continuesBefore: rawStart < 0,
        continuesAfter: rawEnd > days.length - 1,
      } satisfies AllDayPlacement<T>;
    })
    .filter((item): item is AllDayPlacement<T> => item !== null)
    .sort((a, b) => a.startIndex - b.startIndex || b.span - a.span);

  const rows: number[] = [];
  for (const placement of placements) {
    let row = rows.findIndex((end) => end <= placement.startIndex);
    if (row === -1) {
      row = rows.length;
      rows.push(0);
    }
    rows[row] = placement.startIndex + placement.span;
    placement.row = row;
  }
  return placements;
}

