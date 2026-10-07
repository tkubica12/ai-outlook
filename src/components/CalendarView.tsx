import { AlertTriangle, BrainCircuit, Clock3, RefreshCw, Sparkles } from "lucide-react";
import readyIcon from "../assets/generated/event-ready-v2.png";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  hourWindow,
  hoursFromWindowStart,
  isAllDayLike,
  isSameDay,
  isSameMonth,
  laneMetrics,
  layoutDayEvents,
  monthMatrix,
  packAllDayEvents,
  startOfDay,
  visibleDays,
  type HourWindow,
  type ViewMode,
} from "../dates";
import { DayPeek } from "./DayPeek";
import type { CalendarEvent } from "../types";
import type { DaySyncState } from "../useCalendarSync";

interface Props {
  events: CalendarEvent[];
  loading: boolean;
  view: ViewMode;
  anchor: Date;
  onSelect: (event: CalendarEvent) => void;
  onDateSelect?: (date: Date) => void;
  categoryColors?: Record<string, string>;
  selectedId?: string;
  emptyHint?: string;
  /** Per-day sync state, so unanswered days are not painted as empty. */
  dayState?: (date: Date) => DaySyncState;
}

/**
 * A day is only allowed to read as "no meetings" when the backend has actually
 * confirmed coverage for it; every other state says so explicitly.
 */
const DAY_STATE_NOTE: Record<Exclude<DaySyncState, "loaded">, string> = {
  loading: "Loading…",
  syncing: "Syncing…",
  pending: "Not synced yet",
  failed: "Sync failed",
  unqueried: "Not loaded",
};

/**
 * Copy for a day that already shows cached meetings while its refresh is
 * unfinished or failed. A partial day looks identical to a complete one, so it
 * has to say that what is on screen may not be the whole day.
 */
const DAY_STATE_PARTIAL_NOTE: Record<Exclude<DaySyncState, "loaded">, string> = {
  loading: "Refreshing…",
  syncing: "Syncing — may be incomplete",
  pending: "May be incomplete",
  failed: "Sync failed — may be incomplete",
  unqueried: "May be incomplete",
};

const HOUR_HEIGHT = 72;
const CARD_GAP = 6;
const ALL_DAY_ROW = 25;
const ALL_DAY_COLLAPSED_ROWS = 2;
const MONTH_CHIP = 20;

const categoryClass = (category: string) => category.toLowerCase();

const hourLabel = (hour: number) => `${(hour % 24).toString().padStart(2, "0")}:00`;

const STATUS_TEXT: Record<CalendarEvent["briefing_status"], string> = {
  ready: "Briefing ready",
  analyzing: "Briefing in progress",
  error: "Briefing failed",
  not_ready: "Briefing queued",
  not_required: "No briefing needed",
};

function StatusIcon({ event }: { event: CalendarEvent }) {
  if (event.briefing_status === "analyzing") return <RefreshCw className="spin" size={13} />;
  if (event.briefing_status === "error") return <AlertTriangle size={13} />;
  if (event.briefing_status === "ready")
    return event.has_changes
      ? <Sparkles size={13} />
      : <img className="event-ready-generated" src={readyIcon} alt="" />;
  return null;
}

const time = (value: Date) => value.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** Live width of an element, so lane packing reacts to the real column size. */
function useElementWidth<T extends HTMLElement>(): [(node: T | null) => void, number] {
  const [node, setNode] = useState<T | null>(null);
  const [width, setWidth] = useState(0);

  useLayoutEffect(() => {
    if (!node) {
      setWidth(0);
      return;
    }
    const measure = () => setWidth(node.getBoundingClientRect().width);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [node]);

  return [setNode, width];
}

function EventCard({
  event,
  onSelect,
  selected,
  variant = "timed",
  style,
  color,
  overlapped = false,
}: {
  event: CalendarEvent;
  onSelect: (event: CalendarEvent) => void;
  selected?: boolean;
  variant?: "timed" | "chip" | "allday";
  style?: React.CSSProperties;
  color?: string;
  overlapped?: boolean;
}) {
  const start = new Date(event.start);
  const end = new Date(event.end);
  const status = STATUS_TEXT[event.briefing_status];
  const changes = event.has_changes ? ", has updates" : "";
  // A dropped participant list is invisible on a card unless it is marked, and
  // current participant details may be unavailable even for a saved briefing.
  const incomplete = (event.metadata_warnings ?? []).length > 0;
  const variantClass = variant === "chip" ? "compact-event" : variant === "allday" ? "allday-chip" : "";
  const classes = ["event-card", categoryClass(event.category), variantClass]
    .concat(overlapped ? "overlapped" : [], selected ? "selected" : [])
    .filter(Boolean)
    .join(" ");
  return (
    <button
      className={classes}
      style={{ "--event-color": color ?? "#6554d9", ...style } as React.CSSProperties}
      onClick={(clickEvent) => {
        // Safari and Firefox do not focus a button on click, which would break
        // returning focus here after the detail panel closes.
        clickEvent.currentTarget.focus();
        onSelect(event);
      }}
      disabled={event.briefing_status === "not_required"}
      aria-current={selected ? "true" : undefined}
      data-event-id={event.id}
      data-overlapped={overlapped ? "true" : undefined}
      aria-label={`${event.title}, ${time(start)} to ${time(end)}${event.location ? `, ${event.location}` : ""}. ${status}${changes}${incomplete ? ", calendar details incomplete" : ""}`}
    >
      <span className="event-accent" aria-hidden="true" />
      <span className="event-content">
        <strong className="event-title">
          {incomplete && (
            <AlertTriangle
              className="event-incomplete"
              size={11}
              aria-hidden="true"
            />
          )}
          {event.title}
        </strong>
        <span className="event-time">
          {time(start)}
          <span className="event-until">–{time(end)}</span>
        </span>
        <span className="event-meta">{event.location}</span>
      </span>
      <span className={`briefing-indicator ${event.briefing_status}`} aria-hidden="true">
        <StatusIcon event={event} />
      </span>
    </button>
  );
}

function NowLine({ window }: { window: HourWindow }) {
  const now = new Date();
  const offset = hoursFromWindowStart(now, window);
  if (offset < 0 || now.getHours() >= window.end) return null;
  return (
    <div className="now-line" style={{ top: `${offset * HOUR_HEIGHT}px` }}>
      <span>
        <Clock3 size={10} />
      </span>
    </div>
  );
}

interface PeekTarget {
  date: Date;
  origin: DOMRect | null;
}

export function CalendarView({
  events,
  loading,
  view,
  anchor,
  onSelect,
  onDateSelect,
  categoryColors = {},
  selectedId,
  emptyHint = "No meetings in this period.",
  dayState,
}: Props) {
  const noteFor = useCallback(
    (date: Date, hasEvents = false) => {
      const state = dayState?.(date) ?? "loaded";
      if (state === "loaded") return null;
      return hasEvents ? DAY_STATE_PARTIAL_NOTE[state] : DAY_STATE_NOTE[state];
    },
    [dayState],
  );
  const scroller = useRef<HTMLDivElement>(null);
  const [grid, gridWidth] = useElementWidth<HTMLDivElement>();
  const [monthGrid, setMonthGrid] = useState<HTMLDivElement | null>(null);
  const [monthCapacity, setMonthCapacity] = useState(3);
  const [peek, setPeek] = useState<PeekTarget | null>(null);
  const [allDayExpanded, setAllDayExpanded] = useState(false);

  const days = useMemo(() => (view === "month" ? [] : visibleDays(anchor, view)), [anchor, view]);
  const timedEvents = useMemo(() => events.filter((event) => !isAllDayLike(event)), [events]);
  const allDayEvents = useMemo(() => events.filter(isAllDayLike), [events]);
  const window = useMemo(() => hourWindow(timedEvents), [timedEvents]);
  const hours = useMemo(
    () => Array.from({ length: window.end - window.start }, (_, i) => window.start + i),
    [window],
  );
  const allDay = useMemo(() => packAllDayEvents(allDayEvents, days), [allDayEvents, days]);
  const allDayRows = allDay.reduce((max, item) => Math.max(max, item.row + 1), 0);
  const visibleAllDayRows = allDayExpanded
    ? allDayRows
    : Math.min(allDayRows, ALL_DAY_COLLAPSED_ROWS);
  const hiddenAllDay = allDay.filter((item) => item.row >= visibleAllDayRows).length;
  const allDayToggle = hiddenAllDay > 0 || (allDayExpanded && allDayRows > ALL_DAY_COLLAPSED_ROWS);
  const dayWidth = days.length > 0 ? Math.max(0, (gridWidth - 55) / days.length) : 0;

  /** Every event touching `date`, including the middle of a multi-day booking. */
  const eventsOn = useCallback(
    (date: Date) => {
      const from = startOfDay(date).getTime();
      const to = from + 86_400_000;
      return events.filter((event) => {
        const start = new Date(event.start).getTime();
        const end = new Date(event.end).getTime();
        return start < to && Math.max(end, start + 1) > from;
      });
    },
    [events],
  );

  const openPeek = (date: Date, element: HTMLElement) =>
    setPeek({ date, origin: element.getBoundingClientRect() });
  const closePeek = useCallback(() => setPeek(null), []);

  useEffect(() => {
    setPeek(null);
    setAllDayExpanded(false);
  }, [view, anchor]);

  // Bring today (or the first meeting) into view. On narrow screens the current
  // day otherwise sits outside the horizontal scroll area on first paint.
  useEffect(() => {
    if (loading) return;
    const node = scroller.current?.querySelector<HTMLElement>(".day-column.current, .event-card");
    // Guarded: scrollIntoView is missing in non-browser DOM implementations.
    node?.scrollIntoView?.({ block: "nearest", inline: "center" });
  }, [view, anchor, loading]);

  // How many month chips fit depends on the row height, which follows the
  // viewport. Measuring beats hard-coding three and wasting the rest.
  useLayoutEffect(() => {
    if (!monthGrid) return;
    const measure = () => {
      const cell = monthGrid.querySelector<HTMLElement>(".month-cell");
      if (!cell || cell.clientHeight === 0) return;
      // Reserved: cell padding, the day number and the "+N more" chip.
      setMonthCapacity(Math.max(1, Math.floor((cell.clientHeight - 33) / MONTH_CHIP)));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(monthGrid);
    return () => observer.disconnect();
  }, [monthGrid]);

  const peekLayer = peek ? (
    <DayPeek
      date={peek.date}
      events={eventsOn(peek.date)}
      origin={peek.origin}
      categoryColors={categoryColors}
      selectedId={selectedId}
      onSelect={onSelect}
      onOpenDay={(date) => onDateSelect?.(date)}
      onClose={closePeek}
    />
  ) : null;

  if (loading) {
    return (
      <div className="calendar-skeleton" role="status" aria-label="Loading calendar" aria-busy="true">
        {Array.from({ length: 8 }, (_, i) => (
          <div key={i} />
        ))}
      </div>
    );
  }

  if (view === "month") {
    const cells = monthMatrix(anchor);
    return (
      <div className="month-scroll" ref={scroller}>
        <div className="month-grid" ref={setMonthGrid}>
          {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((day) => (
            <div className="month-day-name" key={day}>
              {day}
            </div>
          ))}
          {cells.map((date) => {
            const dayEvents = eventsOn(date).sort(
              (a, b) => new Date(a.start).getTime() - new Date(b.start).getTime(),
            );
            const overflowing = dayEvents.length > monthCapacity;
            const shown = overflowing ? dayEvents.slice(0, Math.max(1, monthCapacity - 1)) : dayEvents;
            const label = date.toLocaleDateString("en", { month: "long", day: "numeric" });
            const note = noteFor(date, dayEvents.length > 0);
            return (
              <div
                className={`month-cell ${isSameDay(date, new Date()) ? "current" : ""} ${isSameMonth(date, anchor) ? "in-range" : "outside"} ${isSameDay(date, anchor) ? "anchor" : ""}`}
                key={date.toISOString()}
                data-day-state={dayState?.(date) ?? "loaded"}
              >
                <button
                  className="day-number"
                  onClick={() => onDateSelect?.(date)}
                  aria-label={`Open ${label} in day view`}
                >
                  {date.getDate()}
                </button>
                {note && (
                  <span
                    className={`day-sync-note ${dayEvents.length > 0 ? "partial" : ""}`}
                    data-day-partial={dayEvents.length > 0 ? "true" : "false"}
                    title={note}
                  >
                    {note}
                  </span>
                )}
                {shown.map((event) => (
                  <EventCard
                    key={event.id}
                    event={event}
                    onSelect={onSelect}
                    selected={event.id === selectedId}
                    color={categoryColors[event.category]}
                    variant="chip"
                  />
                ))}
                {overflowing && (
                  <button
                    className="more-events"
                    aria-haspopup="dialog"
                    aria-label={`Show all ${dayEvents.length} meetings on ${label}`}
                    onClick={(clickEvent) => openPeek(date, clickEvent.currentTarget)}
                  >
                    +{dayEvents.length - shown.length} more
                  </button>
                )}
              </div>
            );
          })}
        </div>
        {peekLayer}
      </div>
    );
  }

  // Must match `eventsOn`: a booking that spans the period without starting in
  // it still renders in the all-day lane, so day-start equality would show an
  // "empty" note on top of visible cards.
  const periodFrom = days.length > 0 ? startOfDay(days[0]).getTime() : 0;
  const periodTo = days.length > 0 ? startOfDay(days[days.length - 1]).getTime() + 86_400_000 : 0;
  const periodEvents = events.filter((event) => {
    const start = new Date(event.start).getTime();
    const end = new Date(event.end).getTime();
    return start < periodTo && Math.max(end, start + 1) > periodFrom;
  });

  // The hour window is deliberately capped so one 03:00 call does not squash the
  // working day. Anything that falls outside it is therefore not drawn — which
  // must be said out loud instead of letting meetings disappear from the column.
  const dayLayouts = days.map((date) => {
    const dayStart = startOfDay(date);
    const dayEnd = new Date(dayStart);
    dayEnd.setDate(dayEnd.getDate() + 1);
    const dayEvents = timedEvents.filter((event) => {
      const start = new Date(event.start);
      const end = new Date(event.end);
      return start < dayEnd && end > dayStart;
    });
    const laid = layoutDayEvents(dayEvents, window, HOUR_HEIGHT, 26, date);
    return { laid, offGrid: Math.max(0, dayEvents.length - laid.length) };
  });

  return (
    <>
      {periodEvents.length === 0 && (
        <div className="calendar-empty" role="status">
          <BrainCircuit size={22} />
          <p>{emptyHint}</p>
        </div>
      )}
      <div className="week-scroll" ref={scroller}>
        <div
          className="week-view"
          ref={grid}
          style={
            {
              "--day-count": days.length,
              "--hour-count": hours.length,
              "--allday-height": `${visibleAllDayRows === 0 ? 0 : visibleAllDayRows * ALL_DAY_ROW + 12 + (allDayToggle ? 20 : 0)}px`,
            } as React.CSSProperties
          }
        >
          <div className="time-header" />
          {days.map((date, index) => {
            const count = eventsOn(date).length;
            const note = noteFor(date, count > 0);
            const long = date.toLocaleDateString("en", {
              weekday: "long",
              month: "long",
              day: "numeric",
            });
            return (
              <div
                className={`day-header ${isSameDay(date, new Date()) ? "current" : ""}`}
                key={date.toISOString()}
                style={{ gridColumn: index + 2 }}
                data-day-state={dayState?.(date) ?? "loaded"}
              >
                <button
                  className="day-header-date"
                  onClick={() => onDateSelect?.(date)}
                  aria-label={`Open ${long} in day view`}
                >
                  <span>{date.toLocaleDateString("en", { weekday: "short" })}</span>
                  <strong>{date.getDate()}</strong>
                </button>
                {note && (
                  <span
                    className={`day-sync-note ${count > 0 ? "partial" : ""}`}
                    data-day-partial={count > 0 ? "true" : "false"}
                    title={note}
                  >
                    {note}
                  </span>
                )}
                {count > 0 && (
                  <button
                    className="day-count"
                    aria-haspopup="dialog"
                    aria-label={`List all ${count} meeting${count === 1 ? "" : "s"} on ${long}`}
                    onClick={(clickEvent) => openPeek(date, clickEvent.currentTarget)}
                  >
                    {count}
                  </button>
                )}
                {dayLayouts[index].offGrid > 0 && (
                  <button
                    className="day-offgrid"
                    aria-haspopup="dialog"
                    title={`Outside the ${hourLabel(window.start)}–${hourLabel(window.end)} grid`}
                    aria-label={`Show ${dayLayouts[index].offGrid} meeting${dayLayouts[index].offGrid === 1 ? "" : "s"} on ${long} that fall outside the ${hourLabel(window.start)} to ${hourLabel(window.end)} grid`}
                    onClick={(clickEvent) => openPeek(date, clickEvent.currentTarget)}
                  >
                    +{dayLayouts[index].offGrid} off-grid
                  </button>
                )}
              </div>
            );
          })}

          <div className={`allday-gutter ${allDayRows === 0 ? "empty" : ""}`}>
            {allDayRows > 0 && <span>All day</span>}
          </div>
          <div className={`allday-lane ${allDayRows === 0 ? "empty" : ""}`}>
            {allDay.map((item) =>
              item.row >= visibleAllDayRows ? null : (
                <EventCard
                  key={item.event.id}
                  event={item.event}
                  onSelect={onSelect}
                  selected={item.event.id === selectedId}
                  color={categoryColors[item.event.category]}
                  variant="allday"
                  style={{
                    top: `${item.row * ALL_DAY_ROW + 6}px`,
                    left: `calc(${(item.startIndex / days.length) * 100}% + 3px)`,
                    width: `calc(${(item.span / days.length) * 100}% - 6px)`,
                  }}
                />
              ),
            )}
            {hiddenAllDay > 0 && (
              <button className="allday-more" onClick={() => setAllDayExpanded(true)}>
                +{hiddenAllDay} more all-day
              </button>
            )}
            {allDayExpanded && allDayRows > ALL_DAY_COLLAPSED_ROWS && (
              <button className="allday-more collapse" onClick={() => setAllDayExpanded(false)}>
                Show fewer all-day
              </button>
            )}
          </div>

          <div className="time-column" aria-hidden="true">
            {hours.map((hour) => (
              <span key={hour} style={{ top: `${(hour - window.start) * HOUR_HEIGHT}px` }}>
                {`${hour.toString().padStart(2, "0")}:00`}
              </span>
            ))}
          </div>
          {days.map((date, index) => {
            const { laid } = dayLayouts[index];
            const current = isSameDay(date, new Date());
            return (
              <div
                className={`day-column ${current ? "current" : ""}`}
                key={date.toISOString()}
                style={{ gridColumn: index + 2 }}
                role="group"
                aria-label={date.toLocaleDateString("en", {
                  weekday: "long",
                  day: "numeric",
                  month: "long",
                })}
              >
                {hours.map((hour) => (
                  <div className="hour-line" key={hour} />
                ))}
                {laid.map(({ event, top, height, lane, lanes, span }) => {
                  const metrics = laneMetrics(lanes, dayWidth, CARD_GAP);
                  const overlapped = dayWidth > 0 && metrics.overlapped;
                  const geometry = overlapped
                    ? {
                        left: `${3 + lane * metrics.step}px`,
                        width: `${metrics.cardWidth + (span - 1) * metrics.step}px`,
                        // Hover or focus reveals a covered card in full without
                        // displacing anything around it.
                        "--card-open-width": `${dayWidth - CARD_GAP - lane * metrics.step}px`,
                      }
                    : {
                        left: `calc(${(lane / lanes) * 100}% + 3px)`,
                        width: `calc(${(span / lanes) * 100}% - 6px)`,
                      };
                  return (
                    <EventCard
                      key={event.id}
                      event={event}
                      onSelect={onSelect}
                      selected={event.id === selectedId}
                      color={categoryColors[event.category]}
                      overlapped={overlapped}
                      style={
                        {
                          top: `${top}px`,
                          height: `${height}px`,
                          zIndex: lane + 1,
                          ...geometry,
                        } as React.CSSProperties
                      }
                    />
                  );
                })}
                {current && <NowLine window={window} />}
              </div>
            );
          })}
        </div>
      </div>
      {peekLayer}
    </>
  );
}
