import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { CalendarDays, X } from "lucide-react";
import { useDialog } from "../useDialog";
import type { CalendarEvent } from "../types";

interface Props {
  date: Date;
  events: CalendarEvent[];
  origin: DOMRect | null;
  categoryColors: Record<string, string>;
  selectedId?: string;
  onSelect: (event: CalendarEvent) => void;
  onOpenDay: (date: Date) => void;
  onClose: () => void;
}

const PEEK_WIDTH = 312;
const SHEET_BREAKPOINT = 560;

const time = (value: Date) => value.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/**
 * Everything scheduled on one day, in full, without leaving the current view.
 * This is the guaranteed path to every meeting: however tightly the grid packs
 * a busy day, the peek still lists each one with its complete title.
 */
export function DayPeek({
  date,
  events,
  origin,
  categoryColors,
  selectedId,
  onSelect,
  onOpenDay,
  onClose,
}: Props) {
  const ref = useDialog<HTMLDivElement>(onClose, true);
  const [style, setStyle] = useState<React.CSSProperties>({ visibility: "hidden" });
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useLayoutEffect(() => {
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    if (viewportWidth <= SHEET_BREAKPOINT) {
      setStyle({ left: 0, right: 0, bottom: 0, top: "auto" });
      return;
    }
    const height = ref.current?.offsetHeight || 320;
    const centre = origin ? origin.left + origin.width / 2 : viewportWidth / 2;
    const left = Math.min(
      Math.max(12, centre - PEEK_WIDTH / 2),
      Math.max(12, viewportWidth - PEEK_WIDTH - 12),
    );
    const below = (origin?.bottom ?? viewportHeight / 3) + 8;
    const top =
      below + height > viewportHeight - 12
        ? Math.max(12, (origin?.top ?? viewportHeight / 3) - height - 8)
        : below;
    setStyle({ left, top, width: PEEK_WIDTH });
  }, [origin, ref]);

  // A popover anchored to a grid cell must not linger once that cell moves.
  // Armed a frame late so the focus move on open cannot dismiss it.
  useEffect(() => {
    let armed = false;
    const arm = requestAnimationFrame(() => {
      armed = true;
    });
    const dismiss = (event: Event) => {
      if (!armed) return;
      if (event.target instanceof Node && ref.current?.contains(event.target)) return;
      closeRef.current();
    };
    window.addEventListener("resize", dismiss);
    document.addEventListener("scroll", dismiss, true);
    document.addEventListener("pointerdown", dismiss, true);
    return () => {
      cancelAnimationFrame(arm);
      window.removeEventListener("resize", dismiss);
      document.removeEventListener("scroll", dismiss, true);
      document.removeEventListener("pointerdown", dismiss, true);
    };
  }, [ref]);

  const sorted = useMemo(
    () => [...events].sort((a, b) => new Date(a.start).getTime() - new Date(b.start).getTime()),
    [events],
  );

  const heading = date.toLocaleDateString("en", {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
  const count = `${sorted.length} meeting${sorted.length === 1 ? "" : "s"}`;

  return (
    <div
      ref={ref}
      className="day-peek"
      role="dialog"
      aria-modal="true"
      aria-label={`${count} on ${heading}`}
      style={style}
    >
      <header className="day-peek-header">
        <div>
          <strong>{heading}</strong>
          <span>{count}</span>
        </div>
        <button className="icon-button compact" aria-label="Close day overview" onClick={onClose}>
          <X size={16} />
        </button>
      </header>
      <ul className="day-peek-list">
        {sorted.map((event) => {
          const start = new Date(event.start);
          const end = new Date(event.end);
          return (
            <li key={event.id}>
              <button
                className={`day-peek-item ${event.id === selectedId ? "selected" : ""}`}
                style={
                  {
                    "--event-color": categoryColors[event.category] ?? "#6554d9",
                  } as React.CSSProperties
                }
                disabled={event.briefing_status === "not_required"}
                data-event-id={event.id}
                onClick={() => {
                  onSelect(event);
                  onClose();
                }}
              >
                <span className="day-peek-accent" aria-hidden="true" />
                <span className="day-peek-body">
                  <strong>{event.title}</strong>
                  <span>
                    {time(start)}–{time(end)}
                    {event.location ? ` · ${event.location}` : ""}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <footer className="day-peek-footer">
        <button
          onClick={() => {
            onOpenDay(date);
            onClose();
          }}
        >
          <CalendarDays size={14} aria-hidden="true" /> Open day view
        </button>
      </footer>
    </div>
  );
}
