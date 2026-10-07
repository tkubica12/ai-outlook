import { AlertTriangle, CheckCircle2, Clock3, LoaderCircle, Sparkles } from "lucide-react";
import type { CalendarEvent } from "../types";

export function BriefingsView({
  events,
  onSelect,
}: {
  events: CalendarEvent[];
  onSelect: (event: CalendarEvent) => void;
}) {
  const sorted = [...events].sort(
    (left, right) => new Date(left.start).getTime() - new Date(right.start).getTime(),
  );
  return (
    <section className="briefings-workspace" aria-labelledby="briefings-title">
      <div className="briefings-heading">
        <div><span><Sparkles size={17} /></span><div><h2 id="briefings-title">Meeting briefings</h2><p>Preparation status across your loaded calendar.</p></div></div>
        <strong>{sorted.filter((event) => event.briefing_status === "ready").length}/{sorted.length} ready</strong>
      </div>
      <div className="briefing-cards">
        {sorted.length === 0 && (
          <p className="empty-note" role="status">
            No meetings match the current filters.
          </p>
        )}
        {sorted.map((event) => {
          const date = new Date(event.start);
          const status = event.briefing_status;
          return (
            <button key={event.id} onClick={() => onSelect(event)} className="briefing-card">
              <span className={`briefing-card-status ${status}`}>
                {status === "ready" ? <CheckCircle2 /> : status === "error" ? <AlertTriangle /> : <LoaderCircle className={status === "analyzing" ? "spin" : ""} />}
              </span>
              <span className="briefing-card-copy">
                <strong>{event.title}</strong>
                <small><Clock3 size={13} /> {date.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" })} · {date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</small>
              </span>
              <span className={`briefing-state-label ${status}`}>{status.replace("_", " ")}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
