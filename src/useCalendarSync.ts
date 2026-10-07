import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import { isoDate } from "./dates";
import type { CalendarEvent, CalendarStatus } from "./types";

/**
 * What is honestly known about one visible day.
 *
 * `loaded` is the only state that allows an empty day to be reported as "no
 * meetings". Everything else means the answer is not in yet, which the grid
 * must say instead of silently drawing an empty column.
 */
export type DaySyncState = "loaded" | "syncing" | "pending" | "failed" | "loading" | "unqueried";

/** Cadence while shards are still moving, and once everything settled. */
export const ACTIVE_POLL_MS = 2_000;
export const IDLE_POLL_MS = 30_000;

export interface CalendarSync {
  events: CalendarEvent[];
  status: CalendarStatus | null;
  /** True only until the first response of the very first range arrives. */
  initialLoading: boolean;
  /** True while the events on screen belong to a previous range. */
  rangeLoading: boolean;
  error: string;
  dayState: (date: Date) => DaySyncState;
  /** Failed days that the current view actually paints. */
  failures: { date: string; message: string }[];
  /** Failed days the backend reports for dates outside the visible range. */
  failuresElsewhere: number;
  /** Days of the visible range the backend flagged as possibly out of date. */
  staleInRange: number;
  /** The backend is working on dates this view does not show. */
  backgroundSync: boolean;
  /** Days the backend has queued or is fetching inside the visible range. */
  outstanding: number;
  /** Days of the visible range whose data has arrived. */
  settled: number;
  reload: () => void;
  retryFailed: () => Promise<void>;
  retrying: boolean;
}

interface Options {
  /** Inclusive first day, `YYYY-MM-DD`. */
  startDate: string;
  /** Exclusive last day, `YYYY-MM-DD`. */
  endDate: string;
  /** Days painted by the current view, used for progress and per-day state. */
  days: Date[];
  enabled: boolean;
}

const isAbort = (reason: unknown) =>
  reason instanceof DOMException && reason.name === "AbortError";

/**
 * Owns every calendar read: the range fetch, the status poll and the retry.
 *
 * One self-rescheduling cycle does both requests together, so a slow backend
 * can never stack overlapping request loops, and a monotonic sequence number
 * discards any response that arrives after the user has already moved on.
 */
export function useCalendarSync({ startDate, endDate, days, enabled }: Options): CalendarSync {
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [status, setStatus] = useState<CalendarStatus | null>(null);
  const [error, setError] = useState("");
  const [initialLoading, setInitialLoading] = useState(true);
  const [loadedRange, setLoadedRange] = useState("");
  const [reloadToken, setReloadToken] = useState(0);
  const [retrying, setRetrying] = useState(false);
  const sequence = useRef(0);
  const statusRef = useRef<CalendarStatus | null>(null);
  const everLoaded = useRef(false);
  statusRef.current = status;

  const rangeKey = `${startDate}:${endDate}`;

  const reload = useCallback(() => setReloadToken((value) => value + 1), []);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let timer = 0;
    // The status a cycle actually observed. A ref updated during render lags a
    // render behind, which made the first cold-start cycle reschedule 30s out.
    let latest: CalendarStatus | null = statusRef.current;
    let consecutiveFailures = 0;
    const controller = new AbortController();

    // Poll fast only while something is genuinely moving; a settled calendar
    // must not hammer the backend, and a zero-event range still polls so a
    // shard that finishes later is picked up.
    const delay = () => {
      if (consecutiveFailures > 0) return consecutiveFailures >= 3 ? IDLE_POLL_MS : ACTIVE_POLL_MS;
      const active =
        latest &&
        (latest.syncing ||
          latest.pending_dates.length > 0 ||
          (latest.syncing_dates?.length ?? 0) > 0);
      return active ? ACTIVE_POLL_MS : IDLE_POLL_MS;
    };

    const cycle = async () => {
      if (disposed) return;
      const id = sequence.current + 1;
      sequence.current = id;
      try {
        const [nextEvents, nextStatus] = await Promise.all([
          api.calendar(startDate, endDate, { signal: controller.signal }),
          // Status is supporting detail: an older backend without the endpoint
          // must not take the calendar down with it.
          api
            .calendarStatus({ signal: controller.signal })
            .catch((reason) => (isAbort(reason) ? Promise.reject(reason) : null)),
        ]);
        if (disposed || id !== sequence.current) return;
        setEvents(nextEvents);
        if (nextStatus) {
          latest = nextStatus;
          setStatus(nextStatus);
        }
        setLoadedRange(rangeKey);
        setError("");
        consecutiveFailures = 0;
        everLoaded.current = true;
      } catch (reason) {
        if (disposed || isAbort(reason) || id !== sequence.current) return;
        consecutiveFailures += 1;
        // The previously loaded range stays on screen; the banner explains why.
        setError(reason instanceof Error ? reason.message : "Could not load calendar");
      } finally {
        if (!disposed && id === sequence.current) setInitialLoading(false);
        if (!disposed) timer = window.setTimeout(() => void cycle(), delay());
      }
    };

    void cycle();
    return () => {
      disposed = true;
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [enabled, startDate, endDate, rangeKey, reloadToken]);

  const retryFailed = useCallback(async () => {
    setRetrying(true);
    try {
      const next = await api.retryCalendar();
      setStatus(next);
      setError("");
      reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Retry failed");
    } finally {
      setRetrying(false);
    }
  }, [reload]);

  const rangeLoading = loadedRange !== rangeKey;

  const dayState = useCallback(
    (date: Date): DaySyncState => {
      const key = isoDate(date);
      if (status?.failures?.[key]) return "failed";
      if (status?.syncing_dates?.includes(key)) return "syncing";
      if (status?.pending_dates?.includes(key)) return "pending";
      // Events on screen belong to another range until this one answers.
      if (rangeLoading) return "loading";
      if (status?.covered_dates?.includes(key)) return "loaded";
      // No status at all: the fetch succeeded, so treat it as answered rather
      // than inventing an unknown state the backend never reported.
      return status ? "unqueried" : "loaded";
    },
    [status, rangeLoading],
  );

  const dayKeys = useMemo(() => new Set(days.map(isoDate)), [days]);

  // The backend reports cache state for every date it has ever visited, not
  // just the requested window. Naming a failure from another month inside a
  // banner about this week would be noise, so in-range days are listed and the
  // rest are only counted.
  const allFailures = useMemo(
    () =>
      Object.entries(status?.failures ?? {})
        .map(([date, message]) => ({ date, message }))
        .sort((a, b) => a.date.localeCompare(b.date)),
    [status],
  );

  const failures = useMemo(
    () => allFailures.filter((failure) => dayKeys.has(failure.date)),
    [allFailures, dayKeys],
  );

  const staleInRange = useMemo(
    () => (status?.stale_dates ?? []).filter((date) => dayKeys.has(date)).length,
    [status, dayKeys],
  );

  const { outstanding, settled } = useMemo(() => {
    let pending = 0;
    let done = 0;
    for (const day of days) {
      const state = dayState(day);
      if (state === "loaded" || state === "failed") done += 1;
      else pending += 1;
    }
    return { outstanding: pending, settled: done };
  }, [days, dayState]);

  return {
    events,
    status,
    // A disabled sync never asks anything, so it must not report a load in
    // flight: that would leave the shell stuck on a skeleton and hide the
    // connection setup the user needs to see.
    initialLoading: enabled && initialLoading && !everLoaded.current,
    rangeLoading,
    error,
    dayState,
    failures,
    failuresElsewhere: allFailures.length - failures.length,
    staleInRange,
    // `syncing` is global, so it only means "elsewhere" once every visible day
    // has answered. Reporting it as this range's progress would show a full
    // bar that never completes.
    backgroundSync: Boolean(status?.syncing) && outstanding === 0,
    outstanding,
    settled,
    reload,
    retryFailed,
    retrying,
  };
}
