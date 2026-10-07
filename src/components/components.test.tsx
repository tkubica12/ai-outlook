import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AnalysisProgress } from "./AnalysisProgress";
import { CalendarSyncStatus } from "./CalendarSyncStatus";
import { MeetingPanel } from "./MeetingPanel";
import { briefing, detail, sources } from "../test-fixtures";
import type { CalendarStatus, MeetingDetail } from "../types";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const status = (overrides: Partial<CalendarStatus> = {}): CalendarStatus => ({
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
  ...overrides,
});

describe("AnalysisProgress", () => {
  const base = {
    total: 3,
    ready: 1,
    queued: 1,
    running: 0,
    failed: 0,
    not_started: 0,
    percent: 50,
  };

  it("excludes meetings that need no briefing from the tracked total", () => {
    render(
      <AnalysisProgress
        state={{
          ...base,
          meetings: [
            { meeting_id: "a", status: "ready" },
            { meeting_id: "b", status: "queued" },
            { meeting_id: "c", status: "not_required" },
          ],
        }}
      />,
    );
    expect(screen.getByText(/1 of 2 ready/)).toBeInTheDocument();
    expect(screen.getByText(/1 need no briefing/)).toBeInTheDocument();
  });

  it("names the model and worker count the backend actually runs", () => {
    render(<AnalysisProgress state={{ ...base, model: "terra-low", concurrency: 4 }} />);
    expect(screen.getByText("Model terra-low")).toBeInTheDocument();
    expect(screen.getByText("4 parallel workers")).toBeInTheDocument();
  });

  it("surfaces the first failure detail instead of a generic message", () => {
    render(
      <AnalysisProgress
        state={{
          ...base,
          queued: 0,
          failed: 1,
          meetings: [{ meeting_id: "a", status: "failed", detail: "Model call timed out" }],
        }}
      />,
    );
    expect(screen.getByText("Model call timed out")).toBeInTheDocument();
    expect(screen.getByText(/finished with failures/i)).toBeInTheDocument();
  });

  it("stays silent about completion while work is outstanding", () => {
    render(<AnalysisProgress state={{ ...base, queued: 0, not_started: 2 }} />);
    expect(screen.queryByText(/preparation complete/i)).toBeNull();
    expect(screen.getByText(/2 not started/)).toBeInTheDocument();
  });

  it("marks the numbers as stale when progress updates stop arriving", () => {
    const { rerender } = render(<AnalysisProgress state={base} />);
    expect(screen.queryByRole("note")).toBeNull();

    rerender(<AnalysisProgress state={base} statusUnavailable />);
    const note = screen.getByRole("note");
    expect(note).toHaveTextContent(/Progress updates unavailable/);
    // The last known counts stay on screen; only their freshness is disclaimed.
    expect(screen.getByText(/1 of 3 ready/)).toBeInTheDocument();

    rerender(<AnalysisProgress state={base} />);
    expect(screen.queryByRole("note")).toBeNull();
  });
});

describe("CalendarSyncStatus", () => {
  const props = {
    failures: [],
    failuresElsewhere: 0,
    staleInRange: 0,
    backgroundSync: false,
    outstanding: 0,
    settled: 0,
    onRetry: () => {},
    retrying: false,
  };

  it("renders nothing when the visible range is fully settled", () => {
    const { container } = render(
      <CalendarSyncStatus {...props} status={status()} settled={7} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing when the backend never answered the status call", () => {
    const { container } = render(<CalendarSyncStatus {...props} status={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("counts loaded days and says pending days are not empty days", () => {
    render(
      <CalendarSyncStatus
        {...props}
        status={status({ syncing: true })}
        outstanding={4}
        settled={3}
      />,
    );
    expect(screen.getByText("Syncing calendar — 3 of 7 days loaded")).toBeInTheDocument();
    expect(screen.getByText("43%")).toBeInTheDocument();
    expect(screen.getByText(/show as pending, not as empty/i)).toBeInTheDocument();
  });

  it("lists failed days with the backend message and offers a retry", async () => {
    const onRetry = vi.fn();
    render(
      <CalendarSyncStatus
        {...props}
        status={status()}
        failures={[{ date: "2026-03-02", message: "Graph timeout" }]}
        settled={7}
        onRetry={onRetry}
      />,
    );
    const banner = screen.getByTestId("calendar-sync");
    expect(banner).toHaveTextContent("2026-03-02");
    expect(banner).toHaveTextContent("Graph timeout");
    fireEvent.click(within(banner).getByRole("button", { name: /Retry failed days/i }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("disables the retry button while a retry is running", () => {
    render(
      <CalendarSyncStatus
        {...props}
        status={status()}
        failures={[{ date: "2026-03-02", message: "Graph timeout" }]}
        settled={7}
        retrying
      />,
    );
    expect(screen.getByRole("button", { name: /Retrying/i })).toBeDisabled();
  });

  it("shows a cache error even when nothing is pending", () => {
    render(
      <CalendarSyncStatus
        {...props}
        status={status({ cache_error: "Cache file is read-only" })}
        settled={7}
      />,
    );
    expect(screen.getByText("Cache file is read-only")).toBeInTheDocument();
  });

  it("counts failures outside the visible range instead of naming them", () => {
    render(
      <CalendarSyncStatus
        {...props}
        status={status()}
        failures={[{ date: "2026-03-02", message: "Graph timeout" }]}
        failuresElsewhere={3}
        settled={7}
      />,
    );
    const banner = screen.getByTestId("calendar-sync");
    expect(banner).toHaveTextContent("1 day in view could not be loaded");
    expect(banner).toHaveTextContent("and 3 outside this range");
  });

  it("still offers a retry when every failure is outside the visible range", () => {
    const onRetry = vi.fn();
    render(
      <CalendarSyncStatus {...props} status={status()} failuresElsewhere={2} settled={7} onRetry={onRetry} />,
    );
    const banner = screen.getByTestId("calendar-sync");
    expect(banner).toHaveTextContent("2 days outside this range could not be loaded");
    expect(banner).not.toHaveTextContent("in view could not be loaded");
    fireEvent.click(within(banner).getByRole("button", { name: /Retry failed days/i }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("keeps the reason for the retry button visible while the range is still loading", () => {
    render(
      <CalendarSyncStatus {...props} status={status()} failuresElsewhere={2} outstanding={3} settled={1} />,
    );
    const banner = screen.getByTestId("calendar-sync");
    expect(banner).toHaveTextContent("Syncing calendar — 1 of 4 days loaded");
    expect(banner).toHaveTextContent("2 days outside this range could not be loaded");
    expect(within(banner).getByRole("button", { name: /Retry failed days/i })).toBeInTheDocument();
  });

  it("does not present a global background sync as this range still loading", () => {
    render(
      <CalendarSyncStatus {...props} status={status({ syncing: true })} backgroundSync settled={7} />,
    );
    const banner = screen.getByTestId("calendar-sync");
    expect(banner).toHaveTextContent("Syncing other dates in the background");
    expect(banner).toHaveTextContent("Every day in this view has already answered");
    expect(screen.queryByText(/of 7 days loaded/)).toBeNull();
    expect(screen.queryByText("100%")).toBeNull();
  });

  it("reports only the stale days the current view actually paints", () => {
    render(
      <CalendarSyncStatus
        {...props}
        status={status({ stale_dates: ["2026-03-02", "2025-01-01", "2025-01-02"] })}
        staleInRange={1}
        failuresElsewhere={1}
        settled={7}
      />,
    );
    expect(screen.getByText("1 day in view may be out of date")).toBeInTheDocument();
  });
});

describe("MeetingPanel evidence honesty", () => {
  const withDetail = (overrides: Partial<MeetingDetail>): MeetingDetail => ({
    ...detail,
    ...overrides,
  });

  const renderPanel = (value: MeetingDetail = detail) =>
    render(<MeetingPanel detail={value} onClose={() => {}} onUpdated={() => {}} />);

  it("labels predicted consumption as not billed actuals", () => {
    renderPanel();
    expect(screen.getAllByText("Predicted").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Actual").length).toBeGreaterThan(0);
    expect(screen.getByText(/are not billed actuals/i)).toBeInTheDocument();
  });

  it("treats legacy unspecified points as unverified rather than actual", () => {
    renderPanel(
      withDetail({
        briefing: {
          ...briefing,
          consumption: [{ label: "Aug", value: 12, kind: "unspecified" }],
        },
      }),
    );
    expect(screen.getAllByText("Unverified").length).toBeGreaterThan(0);
    expect(screen.queryByText("Actual")).toBeNull();
  });

  it("shows metadata warnings and a safe source link next to the attendees", () => {
    renderPanel(
      withDetail({
        event: {
          ...detail.event,
          source_url: "https://outlook.office.com/item",
          metadata_warnings: ["Organizer could not be resolved"],
        },
      }),
    );
    const link = screen.getByRole("link", { name: /Open in the source calendar/i });
    expect(link).toHaveAttribute("href", "https://outlook.office.com/item");
    expect(link).toHaveAttribute("rel", expect.stringContaining("noopener"));
    expect(screen.getByText("Organizer could not be resolved")).toBeInTheDocument();
  });

  it("omits the source link when the backend withheld one", () => {
    renderPanel();
    expect(screen.queryByRole("link", { name: /Open in the source calendar/i })).toBeNull();
  });

  it("says participants could not be read when the list was dropped", () => {
    renderPanel(
      withDetail({
        event: {
          ...detail.event,
          attendees: [],
          metadata_warnings: ["Participant information was incomplete in the calendar response."],
        },
      }),
    );
    expect(screen.getByText("Participants could not be read")).toBeInTheDocument();
    expect(screen.getByText(/check the original invitation/i)).toBeInTheDocument();
    expect(screen.queryByText("No participants listed")).toBeNull();
  });

  it("warns that a partial participant list may be missing people", () => {
    renderPanel(
      withDetail({
        event: {
          ...detail.event,
          metadata_warnings: ["Some participant information was missing."],
        },
      }),
    );
    expect(screen.getByText(/may be missing people/i)).toBeInTheDocument();
    expect(screen.queryByText("Participants could not be read")).toBeNull();
  });

  it("does not invent a participant warning for an unrelated metadata gap", () => {
    renderPanel(
      withDetail({
        event: {
          ...detail.event,
          attendees: [],
          metadata_warnings: ["Organizer could not be resolved"],
        },
      }),
    );
    expect(screen.getByText("No participants listed")).toBeInTheDocument();
    expect(screen.queryByText(/participant list/i)).toBeNull();
  });

  it("renders without crashing when the connector returned no participant list at all", () => {
    renderPanel(
      withDetail({
        event: {
          ...detail.event,
          attendees: null,
          metadata_warnings: ["Participant information was incomplete in the calendar response."],
        },
      }),
    );
    expect(screen.getByText("Participants could not be read")).toBeInTheDocument();
  });

  it("links milestones back to the record they came from", () => {
    renderPanel(
      withDetail({
        briefing: {
          ...briefing,
          milestones: [
            {
              ...(briefing.milestones ?? [])[0],
              url: "https://crm.invalid/milestone-1",
              source_ids: ["mail-1"],
            },
          ],
        },
      }),
    );
    const link = screen.getByRole("link", { name: /Open original record/i });
    expect(link).toHaveAttribute("href", "https://crm.invalid/milestone-1");
    expect(screen.getByText(new RegExp(`From ${sources[0].connector}`))).toBeInTheDocument();
  });
});

describe("MeetingPanel chat history", () => {
  const chatResponse = (answer: string) =>
    new Response(JSON.stringify({ answer, sources: [] }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });

  const ask = async (text: string) => {
    fireEvent.change(screen.getByPlaceholderText(/Ask a follow-up/i), {
      target: { value: text },
    });
    fireEvent.submit(screen.getByPlaceholderText(/Ask a follow-up/i).closest("form")!);
  };

  it("replays earlier turns without duplicating the question being asked", async () => {
    const bodies: { history: { role: string; text: string }[]; message: string }[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((async (
      _input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      bodies.push(JSON.parse(String(init?.body)));
      return chatResponse(`answer ${bodies.length}`);
    }) as typeof fetch);

    render(<MeetingPanel detail={detail} onClose={() => {}} onUpdated={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: /Ask Copilot/i }));

    await ask("first question");
    await screen.findByText("answer 1");
    await ask("second question");
    await screen.findByText("answer 2");

    expect(bodies).toHaveLength(2);
    expect(bodies[0].history).toEqual([]);
    expect(bodies[1].message).toBe("second question");
    expect(bodies[1].history).toEqual([
      { role: "user", text: "first question" },
      { role: "assistant", text: "answer 1" },
    ]);
    expect(bodies[1].history.some((turn) => turn.text === "second question")).toBe(false);
  });

  it("never replays a failed turn", async () => {
    const bodies: { history: { role: string; text: string }[] }[] = [];
    let call = 0;
    vi.spyOn(globalThis, "fetch").mockImplementation((async (
      _input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      bodies.push(JSON.parse(String(init?.body)));
      call += 1;
      return call === 1
        ? new Response(JSON.stringify({ detail: "Model unavailable" }), { status: 503 })
        : chatResponse("recovered");
    }) as typeof fetch);

    render(<MeetingPanel detail={detail} onClose={() => {}} onUpdated={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: /Ask Copilot/i }));

    await ask("first question");
    await screen.findByText(/Model unavailable/);
    await ask("second question");
    await screen.findByText("recovered");

    await waitFor(() => expect(bodies).toHaveLength(2));
    expect(bodies[1].history).toEqual([{ role: "user", text: "first question" }]);
  });
});
