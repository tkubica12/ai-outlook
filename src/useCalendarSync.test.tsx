import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { isoDate } from "./dates";
import { useCalendarSync } from "./useCalendarSync";
import type { CalendarEvent, CalendarStatus } from "./types";

const DAY = (iso: string) => new Date(`${iso}T09:00:00`);

const status = (over: Partial<CalendarStatus> = {}): CalendarStatus => ({
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
  ...over,
});

const event = (id: string, iso: string): CalendarEvent =>
  ({
    id,
    title: "Meeting",
    start: `${iso}T09:00:00`,
    end: `${iso}T10:00:00`,
    attendees: [],
    location: "",
    category: "Customer",
    analysis_status: "ready",
  }) as unknown as CalendarEvent;

const options = (over: Partial<Parameters<typeof useCalendarSync>[0]> = {}) => ({
  startDate: "2026-03-02",
  endDate: "2026-03-09",
  days: [DAY("2026-03-02"), DAY("2026-03-03")],
  enabled: true,
  ...over,
});

describe("useCalendarSync", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("requests exactly the visible range", async () => {
    const calendar = vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(status());
    renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(calendar).toHaveBeenCalled());
    expect(calendar.mock.calls[0][0]).toBe("2026-03-02");
    expect(calendar.mock.calls[0][1]).toBe("2026-03-09");
  });

  it("does not fetch at all until enabled", async () => {
    const calendar = vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(status());
    const { result, rerender } = renderHook(
      (props: { enabled: boolean }) => useCalendarSync(options({ enabled: props.enabled })),
      { initialProps: { enabled: false } },
    );
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(calendar).not.toHaveBeenCalled();
    // A sync that was never asked to run is not "loading": reporting otherwise
    // pins the shell on a skeleton and hides the connection setup for good.
    expect(result.current.initialLoading).toBe(false);

    rerender({ enabled: true });
    expect(result.current.initialLoading).toBe(true);
    await waitFor(() => expect(result.current.initialLoading).toBe(false));
    expect(calendar).toHaveBeenCalled();
  });

  it("keeps a cold empty range loading rather than calling it empty", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(
      status({ syncing: true, pending_dates: ["2026-03-02", "2026-03-03"] }),
    );
    const { result } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.initialLoading).toBe(false));
    expect(result.current.events).toEqual([]);
    expect(result.current.dayState(DAY("2026-03-02"))).toBe("pending");
    expect(result.current.outstanding).toBe(2);
    expect(result.current.settled).toBe(0);
  });

  it("separates covered, syncing, pending and failed days", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([event("a", "2026-03-02")]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(
      status({
        syncing: true,
        covered_dates: ["2026-03-02"],
        syncing_dates: ["2026-03-03"],
        pending_dates: ["2026-03-04"],
        failures: { "2026-03-05": "Graph timeout" },
      }),
    );
    const { result } = renderHook(() =>
      useCalendarSync(
        options({
          days: [DAY("2026-03-02"), DAY("2026-03-03"), DAY("2026-03-04"), DAY("2026-03-05")],
        }),
      ),
    );
    await waitFor(() => expect(result.current.status).not.toBeNull());
    expect(result.current.dayState(DAY("2026-03-02"))).toBe("loaded");
    expect(result.current.dayState(DAY("2026-03-03"))).toBe("syncing");
    expect(result.current.dayState(DAY("2026-03-04"))).toBe("pending");
    expect(result.current.dayState(DAY("2026-03-05"))).toBe("failed");
    expect(result.current.failures).toEqual([{ date: "2026-03-05", message: "Graph timeout" }]);
  });

  it("reports a day the backend never claimed as unqueried", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(status({ covered_dates: ["2026-03-02"] }));
    const { result } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.status).not.toBeNull());
    expect(result.current.dayState(DAY("2026-03-03"))).toBe("unqueried");
  });

  it("falls back to loaded when the status endpoint is unavailable", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockRejectedValue(new Error("404"));
    const { result } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.initialLoading).toBe(false));
    expect(result.current.error).toBe("");
    expect(result.current.dayState(DAY("2026-03-02"))).toBe("loaded");
  });

  it("discards a stale response that resolves after the range moved", async () => {
    const first = event("stale", "2026-03-02");
    const second = event("fresh", "2026-04-02");
    let releaseFirst: (value: CalendarEvent[]) => void = () => {};
    vi.spyOn(api, "calendarStatus").mockResolvedValue(status());
    vi.spyOn(api, "calendar").mockImplementation((start?: string) =>
      start === "2026-03-02"
        ? new Promise<CalendarEvent[]>((resolve) => {
            releaseFirst = resolve;
          })
        : Promise.resolve([second]),
    );

    const { result, rerender } = renderHook((props: Parameters<typeof useCalendarSync>[0]) =>
      useCalendarSync(props),
    { initialProps: options() });

    rerender(options({ startDate: "2026-04-01", endDate: "2026-04-08" }));
    await waitFor(() => expect(result.current.events).toEqual([second]));

    await act(async () => {
      releaseFirst([first]);
      await Promise.resolve();
    });
    expect(result.current.events).toEqual([second]);
  });

  it("runs one request cycle at a time", async () => {
    let inFlight = 0;
    let overlaps = 0;
    vi.spyOn(api, "calendarStatus").mockResolvedValue(status({ syncing: true }));
    vi.spyOn(api, "calendar").mockImplementation(async () => {
      inFlight += 1;
      if (inFlight > 1) overlaps += 1;
      await new Promise((resolve) => setTimeout(resolve, 5));
      inFlight -= 1;
      return [];
    });
    const { result, unmount } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.initialLoading).toBe(false));
    await new Promise((resolve) => setTimeout(resolve, 60));
    unmount();
    expect(overlaps).toBe(0);
  });

  it("keeps polling a range that returned zero events", async () => {
    const calendar = vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(
      status({ syncing: true, pending_dates: ["2026-03-02"] }),
    );
    const { result, unmount } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.initialLoading).toBe(false));
    await waitFor(() => expect(calendar.mock.calls.length).toBeGreaterThan(1), { timeout: 4000 });
    unmount();
  });

  it("resets the failed budget and refetches on retry", async () => {
    const calendar = vi.spyOn(api, "calendar").mockResolvedValue([]);
    const statusMock = vi
      .spyOn(api, "calendarStatus")
      .mockResolvedValue(status({ failures: { "2026-03-02": "boom" } }));
    const retry = vi
      .spyOn(api, "retryCalendar")
      .mockResolvedValue(status({ syncing: true, pending_dates: ["2026-03-02"] }));
    const { result } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.failures).toHaveLength(1));
    const before = calendar.mock.calls.length;
    statusMock.mockResolvedValue(status({ syncing: true, pending_dates: ["2026-03-02"] }));

    await act(async () => {
      await result.current.retryFailed();
    });
    expect(retry).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(calendar.mock.calls.length).toBeGreaterThan(before));
    await waitFor(() => expect(result.current.failures).toHaveLength(0));
  });

  it("surfaces a fetch failure without wiping the loaded range", async () => {
    const first = event("keep", "2026-03-02");
    let call = 0;
    vi.spyOn(api, "calendarStatus").mockResolvedValue(status({ syncing: true }));
    vi.spyOn(api, "calendar").mockImplementation(async () => {
      call += 1;
      if (call === 1) return [first];
      throw new Error("Cannot reach the local API. Check that the backend is running.");
    });
    const { result, unmount } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.events).toEqual([first]));
    await waitFor(() => expect(result.current.error).toMatch(/backend is running/), {
      timeout: 4000,
    });
    expect(result.current.events).toEqual([first]);
    unmount();
  });

  it("derives day keys in local time", () => {
    expect(isoDate(new Date(2026, 2, 2, 23, 30))).toBe("2026-03-02");
    expect(isoDate(new Date(2026, 2, 2, 0, 15))).toBe("2026-03-02");
  });

  it("scopes globally reported failures and stale days to the visible range", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(
      status({
        covered_dates: ["2026-03-02", "2026-03-03"],
        stale_dates: ["2026-03-03", "2024-11-01", "2024-11-02"],
        failures: {
          "2026-03-02": "Graph timeout",
          "2024-11-01": "Graph timeout",
          "2024-11-02": "Graph timeout",
        },
      }),
    );
    const { result, unmount } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.status).not.toBeNull());
    expect(result.current.failures).toEqual([{ date: "2026-03-02", message: "Graph timeout" }]);
    expect(result.current.failuresElsewhere).toBe(2);
    expect(result.current.staleInRange).toBe(1);
    unmount();
  });

  it("treats a global sync as background work once the visible range answered", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(
      status({ syncing: true, covered_dates: ["2026-03-02", "2026-03-03"] }),
    );
    const { result, unmount } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.settled).toBe(2));
    expect(result.current.outstanding).toBe(0);
    expect(result.current.backgroundSync).toBe(true);
    unmount();
  });

  it("does not call a range still loading a background sync", async () => {
    vi.spyOn(api, "calendar").mockResolvedValue([]);
    vi.spyOn(api, "calendarStatus").mockResolvedValue(
      status({ syncing: true, pending_dates: ["2026-03-03"] }),
    );
    const { result, unmount } = renderHook(() => useCalendarSync(options()));
    await waitFor(() => expect(result.current.outstanding).toBeGreaterThan(0));
    expect(result.current.backgroundSync).toBe(false);
    unmount();
  });
});
