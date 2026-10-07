import { ChevronLeft, ChevronRight, CornerUpLeft } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  addDays,
  addMonths,
  dayKey,
  isInRange,
  isSameDay,
  isSameMonth,
  monthMatrix,
  startOfDay,
  startOfMonth,
  startOfWeek,
  type DateRange,
} from "../dates";

interface Props {
  /** Month the grid is showing; browsing it never moves the main view. */
  anchor: Date;
  onPick: (date: Date) => void;
  busyDays: Set<string>;
  onNavigateMonth: (date: Date) => void;
  /** The day the user last picked. */
  selectedDate: Date;
  /** Inclusive day range the active view keeps selected. */
  range: DateRange;
  /** "day", "week", "work week" or "month" for the accessible description. */
  rangeNoun: string;
}

const ARROW_STEP: Record<string, number> = {
  ArrowLeft: -1,
  ArrowRight: 1,
  ArrowUp: -7,
  ArrowDown: 7,
};

export function MiniCalendar({
  anchor,
  selectedDate,
  range,
  rangeNoun,
  onPick,
  onNavigateMonth,
  busyDays,
}: Props) {
  const cells = monthMatrix(anchor);
  const today = new Date();
  const gridRef = useRef<HTMLDivElement>(null);
  const shouldFocus = useRef(false);
  const [cursor, setCursor] = useState(() => startOfDay(selectedDate));
  const monthLabel = new Intl.DateTimeFormat("en", { month: "long", year: "numeric" }).format(anchor);
  const offSelection = !isSameMonth(anchor, range.start) && !isSameMonth(anchor, range.end);

  // The roving tab stop follows the selection, but has to stay inside the month
  // on screen: browsing to another month used to leave the grid with no tab stop
  // at all, which made the whole mini calendar unreachable by keyboard.
  useEffect(() => setCursor(startOfDay(selectedDate)), [selectedDate]);
  useEffect(() => {
    setCursor((current) => (isSameMonth(current, anchor) ? current : startOfMonth(anchor)));
  }, [anchor]);

  useEffect(() => {
    if (!shouldFocus.current) return;
    shouldFocus.current = false;
    gridRef.current?.querySelector<HTMLElement>('button[tabindex="0"]')?.focus();
  }, [cursor, anchor]);

  const moveCursor = (date: Date) => {
    const next = startOfDay(date);
    shouldFocus.current = true;
    setCursor(next);
    if (!isSameMonth(next, anchor)) onNavigateMonth(startOfMonth(next));
  };

  // Arrow keys move focus only; Enter and Space commit through the button's own
  // click handler. Browsing with the keyboard therefore never moves the grid.
  const onKeyDown = (event: React.KeyboardEvent, date: Date) => {
    const step = ARROW_STEP[event.key];
    if (step !== undefined) {
      event.preventDefault();
      moveCursor(addDays(date, step));
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const monday = startOfWeek(date);
      moveCursor(event.key === "Home" ? monday : addDays(monday, 6));
      return;
    }
    if (event.key === "PageUp" || event.key === "PageDown") {
      event.preventDefault();
      const amount = event.key === "PageUp" ? -1 : 1;
      moveCursor(addMonths(date, event.shiftKey ? amount * 12 : amount));
    }
  };

  const tabStop = cells.some((date) => isSameDay(date, cursor)) ? cursor : startOfMonth(anchor);

  return (
    <div className="mini-calendar">
      <div className="mini-title">
        <strong>{monthLabel}</strong>
        <span>
          <button
            className="mini-nav"
            aria-label="Previous month"
            onClick={() => onNavigateMonth(addMonths(anchor, -1))}
          >
            <ChevronLeft size={14} />
          </button>
          <button
            className="mini-nav"
            aria-label="Next month"
            onClick={() => onNavigateMonth(addMonths(anchor, 1))}
          >
            <ChevronRight size={14} />
          </button>
        </span>
      </div>
      {offSelection && (
        <button className="mini-return" onClick={() => onNavigateMonth(startOfMonth(range.start))}>
          <CornerUpLeft size={12} aria-hidden="true" />
          Back to selected {rangeNoun}
        </button>
      )}
      <div className="mini-weekdays" aria-hidden="true">
        {["M", "T", "W", "T", "F", "S", "S"].map((day, index) => (
          <span key={`${day}-${index}`}>{day}</span>
        ))}
      </div>
      <div
        className="mini-days"
        role="grid"
        aria-multiselectable={!isSameDay(range.start, range.end)}
        aria-label={`${monthLabel}, arrow keys move, Enter selects the ${rangeNoun}`}
        ref={gridRef}
      >
        {Array.from({ length: 6 }, (_, week) => (
          <div className="mini-week" role="row" key={week}>
            {cells.slice(week * 7, week * 7 + 7).map((date) => {
              const inRange = isInRange(date, range);
              const weekday = date.getDay();
              return (
                <button
                  key={date.toISOString()}
                  role="gridcell"
                  tabIndex={isSameDay(date, tabStop) ? 0 : -1}
                  className={[
                    isSameDay(date, today) ? "today" : "",
                    isSameMonth(date, anchor) ? "" : "outside",
                    isSameDay(date, selectedDate) ? "selected" : "",
                    inRange ? "in-range" : "",
                    inRange && (isSameDay(date, range.start) || weekday === 1) ? "range-start" : "",
                    inRange && (isSameDay(date, range.end) || weekday === 0) ? "range-end" : "",
                    busyDays.has(dayKey(date)) ? "has-events" : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  aria-selected={inRange}
                  aria-current={isSameDay(date, today) ? "date" : undefined}
                  aria-label={date.toLocaleDateString("en", {
                    weekday: "long",
                    day: "numeric",
                    month: "long",
                    year: "numeric",
                  })}
                  onKeyDown={(event) => onKeyDown(event, date)}
                  onClick={() => onPick(date)}
                >
                  {date.getDate()}
                  {busyDays.has(dayKey(date)) && <span className="mini-dot" aria-hidden="true" />}
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
