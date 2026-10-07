import { AlertTriangle, Cloud, LoaderCircle, RefreshCw } from "lucide-react";
import type { CalendarStatus } from "../types";

interface Props {
  status: CalendarStatus | null;
  /** Failed days inside the visible range, listed by date. */
  failures: { date: string; message: string }[];
  /** Failed days outside the visible range, counted only. */
  failuresElsewhere: number;
  /** Visible days the backend flagged as possibly out of date. */
  staleInRange: number;
  /** The backend is busy with dates this view does not show. */
  backgroundSync: boolean;
  /** Days of the visible range still waiting for an answer. */
  outstanding: number;
  /** Days of the visible range that have answered. */
  settled: number;
  onRetry: () => void;
  retrying: boolean;
}

const formatTime = (value: string | null) => {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toLocaleTimeString([], { timeStyle: "short" });
};

/**
 * Reports what the calendar cache actually knows about the visible range.
 *
 * Nothing here is inferred: pending days come from the backend's own queue and
 * failures carry the backend's message, so an empty grid is never dressed up as
 * a synced-and-empty calendar.
 */
export function CalendarSyncStatus({
  status,
  failures,
  failuresElsewhere,
  staleInRange,
  backgroundSync,
  outstanding,
  settled,
  onRetry,
  retrying,
}: Props) {
  if (!status) return null;
  const total = outstanding + settled;
  const busy = outstanding > 0;
  const hasFailures = failures.length > 0;
  const anyFailures = hasFailures || failuresElsewhere > 0;
  const notice = status.cache_error ?? status.cache_warning ?? null;
  if (!busy && !anyFailures && !notice && !backgroundSync) return null;

  const percent = total > 0 ? Math.round((settled / total) * 100) : 0;
  const syncedAt = formatTime(status.last_synced_at);

  return (
    <section
      className={`calendar-sync ${anyFailures || status.cache_error ? "has-failures" : busy || backgroundSync ? "busy" : "notice"}`}
      aria-live="polite"
      data-testid="calendar-sync"
    >
      <div className="calendar-sync-icon">
        {busy || backgroundSync ? (
          <LoaderCircle className="spin" size={18} />
        ) : anyFailures || status.cache_error ? (
          <AlertTriangle size={18} />
        ) : (
          <Cloud size={18} />
        )}
      </div>
      <div className="calendar-sync-copy">
        <div className="calendar-sync-headline">
          <strong>
            {busy
              ? `Syncing calendar — ${settled} of ${total} day${total === 1 ? "" : "s"} loaded`
              : hasFailures
                ? `${failures.length} day${failures.length === 1 ? "" : "s"} in view could not be loaded`
                : failuresElsewhere > 0
                  ? `${failuresElsewhere} day${failuresElsewhere === 1 ? "" : "s"} outside this range could not be loaded`
                  : backgroundSync
                    ? "Syncing other dates in the background"
                    : "Calendar cache notice"}
          </strong>
          {busy && <span>{percent}%</span>}
        </div>
        {busy && (
          <div className="progress-track" aria-label={`${percent}% of the visible range loaded`}>
            <span style={{ width: `${percent}%` }} />
          </div>
        )}
        <p className="calendar-sync-detail">
          {busy && <span>Days still loading show as pending, not as empty.</span>}
          {!busy && backgroundSync && <span>Every day in this view has already answered.</span>}
          {/* The retry button is global, so its reason must stay visible even
              while this range is still loading. */}
          {failuresElsewhere > 0 && !hasFailures && busy && (
            <span>
              {failuresElsewhere} day{failuresElsewhere === 1 ? "" : "s"} outside this range could
              not be loaded
            </span>
          )}
          {syncedAt && <span>Last synced {syncedAt}</span>}
          {staleInRange > 0 && !busy && (
            <span>
              {staleInRange} day{staleInRange === 1 ? "" : "s"} in view may be out of date
            </span>
          )}
        </p>
        {notice && <p className="calendar-sync-notice">{notice}</p>}
        {hasFailures && (
          <ul className="calendar-sync-failures">
            {failures.slice(0, 4).map((failure) => (
              <li key={failure.date}>
                <strong>{failure.date}</strong> {failure.message}
              </li>
            ))}
            {failures.length > 4 && <li>and {failures.length - 4} more in view</li>}
            {failuresElsewhere > 0 && <li>and {failuresElsewhere} outside this range</li>}
          </ul>
        )}
      </div>
      {anyFailures && (
        <button className="ghost-button" onClick={onRetry} disabled={retrying}>
          <RefreshCw size={14} className={retrying ? "spin" : undefined} />
          {retrying ? "Retrying…" : "Retry failed days"}
        </button>
      )}
    </section>
  );
}
