import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { formatRangeLabel } from "./dates";
import { calendarEvents, coveredStatus, detail, internalEvent, mockApi } from "./test-fixtures";

const useApi = (options: Parameters<typeof mockApi>[0] = {}) =>
  vi.spyOn(globalThis, "fetch").mockImplementation(mockApi(options) as typeof fetch);

/** Accessible name of a mini-calendar day cell. */
const miniLabel = (date: Date) =>
  date.toLocaleDateString("en", {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
  });

/** Four meetings on one day, so density affordances have something to show. */
const busyDay = () => {
  const at = (hour: number, minute = 0) => {
    const date = new Date();
    date.setHours(hour, minute, 0, 0);
    return date.toISOString();
  };
  return [
    { ...calendarEvents[0], id: "a", title: "Alpha review", start: at(9), end: at(10) },
    { ...calendarEvents[0], id: "b", title: "Beta review", start: at(9, 30), end: at(10, 30) },
    { ...calendarEvents[0], id: "c", title: "Gamma review", start: at(9, 45), end: at(11) },
    { ...calendarEvents[0], id: "d", title: "Delta review", start: at(10), end: at(11, 30) },
  ];
};

const openContoso = async () => {
  const meeting = await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
  fireEvent.click(meeting);
  await waitFor(() => expect(screen.getByText("What matters")).toBeInTheDocument());
  return meeting;
};

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});
afterEach(() => vi.restoreAllMocks());

describe("calendar shell", () => {
  it("loads the connected calendar without demo labelling", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    expect(screen.getByRole("link", { name: "Tomlook home" })).toBeInTheDocument();
    expect(screen.getByText("Tomlook", { exact: true })).toBeInTheDocument();
    expect(screen.queryByText("Demonstration workspace")).toBeNull();
  });

  it("shows a retryable error banner when the calendar cannot load", async () => {
    const onCalendar = vi
      .fn<() => Response | undefined>()
      .mockImplementationOnce(() => new Response("nope", { status: 503 }))
      .mockImplementation(() => undefined);
    useApi({ onCalendar });
    render(<App />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Request failed \(503\)/);

    fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("changes the heading when navigating periods and returns with Today", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const heading = screen.getByRole("heading", { level: 1 });
    const initial = heading.textContent;

    fireEvent.click(screen.getByRole("button", { name: "Next period" }));
    expect(heading.textContent).not.toBe(initial);

    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    expect(heading.textContent).toBe(initial);
  });

  it("switches between all four views", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    for (const view of ["Day", "Work week", "Week", "Month"]) {
      fireEvent.click(screen.getByRole("button", { name: view }));
      expect(screen.getByRole("button", { name: view })).toHaveAttribute("aria-pressed", "true");
    }
  });

  it("keeps the active week when a date is picked in the mini calendar", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Week" }));

    const target = new Date();
    target.setDate(target.getDate() + 1);
    fireEvent.click(screen.getByRole("gridcell", { name: miniLabel(target) }));

    expect(screen.getByRole("button", { name: "Week" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Day" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(
      formatRangeLabel(target, "week"),
    );
  });

  it("highlights the whole selected range in the mini calendar", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const selectedDays = () => document.querySelectorAll('.mini-days [aria-selected="true"]').length;

    fireEvent.click(screen.getByRole("button", { name: "Week" }));
    expect(selectedDays()).toBe(7);

    fireEvent.click(screen.getByRole("button", { name: "Work week" }));
    expect(selectedDays()).toBe(5);

    fireEvent.click(screen.getByRole("button", { name: "Day" }));
    expect(selectedDays()).toBe(1);

    fireEvent.click(screen.getByRole("button", { name: "Month" }));
    const daysInMonth = new Date(
      new Date().getFullYear(),
      new Date().getMonth() + 1,
      0,
    ).getDate();
    expect(selectedDays()).toBe(daysInMonth);
  });

  it("keeps the mini-calendar ring inside the highlighted period", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const ring = () => document.querySelector(".mini-days .selected");
    // Column 5 and 6 of the Monday-first grid are Saturday and Sunday: the only
    // picks that can fall outside a Work week highlight.
    const weekendCells = () =>
      [...document.querySelectorAll<HTMLElement>(".mini-week")].slice(1, 3).flatMap((row) =>
        [...row.querySelectorAll<HTMLElement>("button")].slice(5),
      );

    for (const view of ["Day", "Work week", "Week", "Month"]) {
      fireEvent.click(screen.getByRole("button", { name: view }));
      for (const cell of weekendCells()) {
        const picked = cell.getAttribute("aria-label");
        fireEvent.click(cell);
        const selected = ring();
        expect(selected, `${view}: no ring after picking ${picked}`).not.toBeNull();
        expect(
          selected?.getAttribute("aria-selected"),
          `${view}: ring outside the highlight after picking ${picked}`,
        ).toBe("true");
      }
      fireEvent.click(screen.getByRole("button", { name: "Today" }));
    }
  });

  it("drops the off-period flag as soon as Today is pressed", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });

    for (const view of ["Day", "Work week", "Week", "Month"]) {
      fireEvent.click(screen.getByRole("button", { name: view }));
      fireEvent.click(screen.getByRole("button", { name: "Today" }));
      expect(screen.queryByText("Not today"), `${view} flagged as off-period`).toBeNull();

      fireEvent.click(screen.getByRole("button", { name: "Next period" }));
      expect(screen.getByText("Not today")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Today" }));
    }
  });

  it("browses the mini month without moving the main view or losing the tab stop", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const heading = screen.getByRole("heading", { level: 1 }).textContent;

    fireEvent.click(screen.getByRole("button", { name: "Next month" }));
    fireEvent.click(screen.getByRole("button", { name: "Next month" }));

    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(heading);
    expect(document.querySelectorAll('.mini-days button[tabindex="0"]')).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: /Back to selected/ }));
    expect(document.querySelectorAll('.mini-days button[tabindex="0"]')).toHaveLength(1);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(heading);
  });

  it("moves mini-calendar focus with the arrow keys without committing", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Week" }));
    const heading = screen.getByRole("heading", { level: 1 }).textContent;
    const tabStop = () => document.querySelector('.mini-days button[tabindex="0"]') as HTMLElement;

    fireEvent.keyDown(tabStop(), { key: "ArrowDown" });
    fireEvent.keyDown(tabStop(), { key: "ArrowRight" });

    const target = new Date();
    target.setDate(target.getDate() + 8);
    expect(tabStop()).toHaveAccessibleName(miniLabel(target));
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(heading);
  });

  it("drills into day view from a day header, which is an explicit request", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Week" }));

    const today = new Date();
    const label = today.toLocaleDateString("en", {
      weekday: "long",
      month: "long",
      day: "numeric",
    });
    fireEvent.click(screen.getByRole("button", { name: `Open ${label} in day view` }));
    expect(screen.getByRole("button", { name: "Day" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("opens a dedicated briefings workspace", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: /Briefings/ }));
    expect(screen.getByRole("heading", { name: "Meeting briefings" })).toBeInTheDocument();
    expect(screen.getByText(/Preparation status across your loaded calendar/)).toBeInTheDocument();
  });

  it("explains an empty briefings list instead of rendering a blank region", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: /Briefings/ }));
    fireEvent.change(screen.getByRole("searchbox", { name: "Search meetings" }), {
      target: { value: "zzzz-no-such-meeting" },
    });

    const workspace = screen.getByRole("region", { name: "Meeting briefings" });
    expect(within(workspace).getByRole("status")).toBeInTheDocument();
    expect(within(workspace).queryByRole("button")).toBeNull();
  });

  it("does not open a meeting that needs no briefing", async () => {
    useApi();
    render(<App />);
    const focus = await screen.findByRole("button", { name: /Focus time/i });
    expect(focus).toBeDisabled();
  });

  it("filters the calendar by search text", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });

    fireEvent.change(screen.getByRole("searchbox", { name: "Search meetings" }), {
      target: { value: "contoso" },
    });
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /Digital sales team sync/i })).toBeNull(),
    );
    expect(
      screen.getByText(`Showing 1 of ${calendarEvents.length} meetings`),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    await screen.findByRole("button", { name: /Digital sales team sync/i });
  });

  it("filters by calendar category", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("checkbox", { name: "Customer" }));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /Contoso cloud adoption review/i })).toBeNull(),
    );
    expect(screen.getByRole("button", { name: /Digital sales team sync/i })).toBeInTheDocument();
  });

  it("toggles the theme and persists it", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Use dark theme" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem("theme")).toBe("dark");
    fireEvent.click(screen.getByRole("button", { name: "Use light theme" }));
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("keeps demonstration-only chrome disabled rather than silently inert", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    expect(screen.getByRole("button", { name: "Notifications" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /New event/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "User profile" })).toBeDisabled();
  });
});

describe("event density", () => {
  it("renders every concurrent meeting rather than hiding some behind a counter", async () => {
    useApi({ events: busyDay() });
    render(<App />);
    await screen.findByRole("button", { name: /Alpha review/i });
    fireEvent.click(screen.getByRole("button", { name: "Week" }));

    for (const title of ["Alpha review", "Beta review", "Gamma review", "Delta review"]) {
      expect(screen.getByRole("button", { name: new RegExp(title, "i") })).toBeInTheDocument();
    }
    expect(document.querySelector(".overlap-summary")).toBeNull();
  });

  it("lists a whole day in a peek without leaving the week", async () => {
    useApi({ events: busyDay() });
    render(<App />);
    await screen.findByRole("button", { name: /Alpha review/i });
    fireEvent.click(screen.getByRole("button", { name: "Week" }));

    fireEvent.click(screen.getByRole("button", { name: /List all 4 meetings on/ }));
    const peek = await screen.findByRole("dialog");
    expect(within(peek).getAllByRole("button", { name: /review/i })).toHaveLength(4);
    expect(screen.getByRole("button", { name: "Week" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Week" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("opens a meeting straight from the peek", async () => {
    useApi({ events: busyDay() });
    render(<App />);
    await screen.findByRole("button", { name: /Alpha review/i });
    fireEvent.click(screen.getByRole("button", { name: /List all 4 meetings on/ }));
    const peek = await screen.findByRole("dialog");
    fireEvent.click(within(peek).getByRole("button", { name: /Gamma review/i }));
    await waitFor(() => expect(screen.getByText("What matters")).toBeInTheDocument());
  });

  it("keeps a very short meeting reachable and correctly labelled", async () => {
    const start = new Date();
    start.setHours(9, 0, 0, 0);
    const end = new Date(start.getTime() + 5 * 60_000);
    useApi({
      events: [
        {
          ...calendarEvents[0],
          id: "tiny",
          title: "Five minute check",
          start: start.toISOString(),
          end: end.toISOString(),
        },
      ],
    });
    render(<App />);
    const card = await screen.findByRole("button", { name: /Five minute check/i });
    expect(card).toHaveAttribute("aria-label", expect.stringContaining("Five minute check"));
    expect(Number.parseFloat(card.style.height)).toBeGreaterThanOrEqual(26);
  });

  it("lifts an all-day booking out of the timed grid into its own band", async () => {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    const end = new Date(start);
    end.setDate(end.getDate() + 2);
    useApi({
      events: [
        ...calendarEvents,
        {
          ...calendarEvents[0],
          id: "offsite",
          title: "Team offsite",
          start: start.toISOString(),
          end: end.toISOString(),
        },
      ],
    });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Week" }));

    const chip = screen.getByRole("button", { name: /Team offsite/i });
    expect(chip.closest(".allday-lane")).not.toBeNull();
    expect(document.querySelectorAll(".day-column .event-card")).not.toHaveLength(0);
    expect(chip.closest(".day-column")).toBeNull();
  });

  it("shows the overflow of a month cell as a peek instead of a view change", async () => {
    useApi({ events: busyDay() });
    render(<App />);
    await screen.findByRole("button", { name: /Alpha review/i });
    fireEvent.click(screen.getByRole("button", { name: "Month" }));

    const more = screen.getByRole("button", { name: /Show all 4 meetings on/ });
    expect(more).toHaveAttribute("aria-haspopup", "dialog");
    fireEvent.click(more);
    const peek = await screen.findByRole("dialog");
    expect(within(peek).getAllByRole("button", { name: /review/i })).toHaveLength(4);
    expect(screen.getByRole("button", { name: "Month" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });
});

describe("meeting detail", () => {  it("opens the briefing with its evidence", async () => {
    useApi();
    render(<App />);
    await openContoso();
    expect(screen.getByText("Key briefing summary")).toBeInTheDocument();
    expect(screen.getByText("Expert contributor")).toBeInTheDocument();
    expect(screen.getAllByText("Inference").length).toBeGreaterThan(0);
  });

  it("closes on Escape and returns focus to the originating event", async () => {
    useApi();
    render(<App />);
    const trigger = await openContoso();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByText("What matters")).toBeNull());
    expect(document.activeElement).toBe(trigger);
  });

  it("reports a failed meeting load without leaving an empty panel", async () => {
    useApi({
      onMeeting: () =>
        new Response(JSON.stringify({ detail: "Meeting not found" }), {
          status: 404,
          headers: { "Content-Type": "application/json" },
        }),
    });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Contoso cloud adoption review/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Meeting not found");
    expect(screen.queryByText("What matters")).toBeNull();
  });

  it("renders traceable sources and never links a source without a URL", async () => {
    useApi();
    render(<App />);
    await openContoso();
    fireEvent.click(screen.getByRole("tab", { name: "Sources & details" }));
    expect(await screen.findByText("RE: Contoso adoption plan")).toBeInTheDocument();
    expect(document.querySelectorAll('a[href="#"]')).toHaveLength(0);
    const powerBi = screen.getByText("Azure consumption").closest(".source-item");
    expect(powerBi?.tagName).toBe("DIV");
  });

  it("shows future milestones and opens a review-only task draft", async () => {
    useApi();
    render(<App />);
    await openContoso();
    expect(screen.getByText("Future milestones")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Review suggested task" }));
    const dialog = screen.getByRole("dialog", { name: "Prepare production readiness review" });
    expect(dialog).toHaveTextContent("Draft only · no Dataverse write");
    expect(dialog).toHaveTextContent("Architecture Review");
  });

  it("answers in chat with rendered emphasis and cited sources", async () => {
    useApi();
    render(<App />);
    await openContoso();
    fireEvent.click(screen.getByRole("tab", { name: "Ask Copilot" }));
    fireEvent.change(screen.getByLabelText("Chat message"), {
      target: { value: "What should I present?" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));

    const reply = await screen.findByText(/Your likely role is/);
    expect(reply.textContent).not.toContain("**");
    expect(within(reply.closest(".message") as HTMLElement).getByText(/Outlook Mail/)).toBeInTheDocument();
  });

  it("shows chat failures as errors rather than as an answer", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
      if (String(input).includes("/chat")) throw new TypeError("Failed to fetch");
      return mockApi()(input);
    }) as typeof fetch);
    render(<App />);
    await openContoso();
    fireEvent.click(screen.getByRole("tab", { name: "Ask Copilot" }));
    fireEvent.change(screen.getByLabelText("Chat message"), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(document.querySelector(".message.failed")).toBeTruthy());
  });
});

describe("feedback and skill governance", () => {
  const openFeedback = async () => {
    fireEvent.click(screen.getByRole("button", { name: "Improve this briefing" }));
    return screen.findByRole("dialog");
  };

  it("saves meeting-scoped feedback", async () => {
    useApi();
    render(<App />);
    await openContoso();
    await openFeedback();
    fireEvent.change(screen.getByRole("textbox", { name: "Feedback" }), {
      target: { value: "Add the renewal date." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save feedback" }));
    expect(await screen.findByText("Feedback saved.")).toBeInTheDocument();
  });

  it("closes the feedback dialog on Escape without closing the panel", async () => {
    useApi();
    render(<App />);
    await openContoso();
    await openFeedback();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByText("What matters")).toBeInTheDocument();
  });

  it("reports a feedback failure inline instead of failing silently", async () => {
    useApi({ onFeedback: () => new Response("no", { status: 500 }) });
    render(<App />);
    await openContoso();
    await openFeedback();
    fireEvent.change(screen.getByRole("textbox", { name: "Feedback" }), {
      target: { value: "boom" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save feedback" }));
    expect(await screen.findByText(/Request failed \(500\)/)).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("requires explicit approval before a skill change is accepted", async () => {
    useApi({
      onFeedback: () =>
        new Response(
          JSON.stringify({
            saved: true,
            scope: "skill",
            proposal: {
              id: "p1",
              title: "Clarify role evidence",
              reason: "Needs two sources",
              status: "pending",
              old_content: "old rule",
              new_content: "new rule",
              created_at: new Date().toISOString(),
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
    });
    render(<App />);
    await openContoso();
    await openFeedback();
    fireEvent.change(screen.getByRole("textbox", { name: "Feedback" }), {
      target: { value: "Require two sources." },
    });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "skill" } });
    fireEvent.click(screen.getByRole("button", { name: "Save feedback" }));

    expect(await screen.findByText("Clarify role evidence")).toBeInTheDocument();
    expect(screen.getByText(/− old rule/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Record approval" }));
    expect(await screen.findByText(/Decision recorded as approved/)).toBeInTheDocument();
    expect(screen.getByText(/skill used at runtime is unchanged/i)).toBeInTheDocument();
  });

  it("marks a rejected proposal distinctly from an approved one", async () => {
    useApi({
      onFeedback: () =>
        new Response(
          JSON.stringify({
            saved: true,
            scope: "skill",
            proposal: {
              id: "p1",
              title: "Clarify role evidence",
              reason: "Needs two sources",
              status: "pending",
              old_content: "old rule",
              new_content: "new rule",
              created_at: new Date().toISOString(),
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
    });
    render(<App />);
    await openContoso();
    await openFeedback();
    fireEvent.change(screen.getByRole("textbox", { name: "Feedback" }), {
      target: { value: "Require two sources." },
    });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "skill" } });
    fireEvent.click(screen.getByRole("button", { name: "Save feedback" }));
    fireEvent.click(await screen.findByRole("button", { name: "Reject" }));
    await waitFor(() => expect(document.querySelector(".decision.rejected")).toBeTruthy());
    expect(document.querySelector(".decision.approved")).toBeNull();
  });
});

describe("diagnostics", () => {
  it("shows runtime and connector provenance and closes on Escape", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: /System diagnostics/ }));
    expect(await screen.findByText("Not configured")).toBeInTheDocument();
    expect(screen.getByText(/Live MCP mode/)).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("calendar range fetching", () => {
  const rangeOf = (url: URL) => `${url.searchParams.get("start_date")}..${url.searchParams.get("end_date")}`;

  it("requests only the visible range and refetches when the view changes", async () => {
    const seen: string[] = [];
    useApi({ onCalendar: (url) => { seen.push(rangeOf(url)); return undefined; } });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    await waitFor(() => expect(seen.length).toBeGreaterThan(0));
    expect(seen[0]).toMatch(/^\d{4}-\d{2}-\d{2}\.\.\d{4}-\d{2}-\d{2}$/);

    const spanOf = (value: string) => {
      const [start, end] = value.split("..");
      return Math.round((Date.parse(end) - Date.parse(start)) / 86_400_000);
    };

    for (const [view, days] of [["Day", 1], ["Work week", 5], ["Week", 7], ["Month", 42]] as const) {
      const before = seen.length;
      fireEvent.click(screen.getByRole("button", { name: view }));
      await waitFor(() => expect(seen.length).toBeGreaterThan(before));
      expect(spanOf(seen[seen.length - 1])).toBe(days);
    }
  });

  it("never exceeds the backend 42 day ceiling", async () => {
    const seen: string[] = [];
    useApi({ onCalendar: (url) => { seen.push(rangeOf(url)); return undefined; } });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Month" }));
    await waitFor(() => expect(seen.length).toBeGreaterThan(1));
    for (const range of seen) {
      const [start, end] = range.split("..");
      const days = Math.round((Date.parse(end) - Date.parse(start)) / 86_400_000);
      expect(days).toBeGreaterThan(0);
      expect(days).toBeLessThanOrEqual(42);
    }
  });

  it("refetches when navigating periods", async () => {
    const seen: string[] = [];
    useApi({ onCalendar: (url) => { seen.push(rangeOf(url)); return undefined; } });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const before = seen.length;
    fireEvent.click(screen.getByRole("button", { name: "Next period" }));
    await waitFor(() => expect(seen.length).toBeGreaterThan(before));
    expect(seen[seen.length - 1]).not.toBe(seen[0]);
  });
});

describe("cold calendar honesty", () => {
  const syncing = {
    syncing: true,
    covered_dates: [],
    requested_dates: [],
    pending_dates: [new Date().toISOString().slice(0, 10)],
    syncing_dates: [],
    stale_dates: [],
    failures: {},
    last_synced_at: null,
    cache_warning: null,
    cache_error: null,
  };

  it("shows sync progress instead of an empty calendar while shards run", async () => {
    useApi({ events: [], calendarStatus: syncing });
    render(<App />);
    const banner = await screen.findByTestId("calendar-sync");
    expect(banner).toHaveTextContent(/Syncing calendar/i);
    expect(screen.queryByText(/No meetings in this period/i)).toBeNull();
  });

  it("keeps polling a range that returned zero events", async () => {
    let calls = 0;
    useApi({ events: [], calendarStatus: syncing, onCalendar: () => { calls += 1; return undefined; } });
    render(<App />);
    await screen.findByTestId("calendar-sync");
    const before = calls;
    await waitFor(() => expect(calls).toBeGreaterThan(before), { timeout: 5000 });
  });

  it("reports failed days and retries them on request", async () => {
    const today = new Date().toISOString().slice(0, 10);
    let retried = 0;
    useApi({
      events: [],
      calendarStatus: { ...syncing, syncing: false, pending_dates: [], failures: { [today]: "Graph timeout" } },
      onRetry: () => { retried += 1; return undefined; },
    });
    render(<App />);
    const banner = await screen.findByTestId("calendar-sync");
    expect(banner).toHaveTextContent(/Graph timeout/);
    fireEvent.click(within(banner).getByRole("button", { name: /Retry failed days/i }));
    await waitFor(() => expect(retried).toBe(1));
  });

  it("hides the sync banner once every visible day is covered", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    await waitFor(() => expect(screen.queryByTestId("calendar-sync")).toBeNull());
  });

  // A partly-refreshed day looks exactly like a complete one, so a failure has
  // to stay visible even though cached meetings are already on screen.
  const localToday = () => {
    const now = new Date();
    const month = `${now.getMonth() + 1}`.padStart(2, "0");
    const day = `${now.getDate()}`.padStart(2, "0");
    return `${now.getFullYear()}-${month}-${day}`;
  };

  it("still reports failed days when cached events are already showing", async () => {
    useApi({
      calendarStatus: {
        ...coveredStatus(),
        failures: { [localToday()]: "Graph timeout" },
      },
    });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const banner = await screen.findByTestId("calendar-sync");
    expect(banner).toHaveTextContent(/could not be loaded/i);
    expect(banner).toHaveTextContent(/Graph timeout/);
    expect(within(banner).getByRole("button", { name: /Retry failed days/i })).toBeInTheDocument();
  });

  it("marks a day that has meetings but did not finish syncing as incomplete", async () => {
    useApi({
      calendarStatus: {
        ...coveredStatus(),
        failures: { [localToday()]: "Graph timeout" },
      },
    });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    const note = await screen.findByText(/Sync failed — may be incomplete/i);
    expect(note).toHaveAttribute("data-day-partial", "true");
  });

  // The month cell used to drop the note entirely as soon as a day had events.
  it("keeps the incomplete marker on a month cell that already lists meetings", async () => {
    useApi({
      calendarStatus: {
        ...coveredStatus(),
        failures: { [localToday()]: "Graph timeout" },
      },
    });
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    fireEvent.click(screen.getByRole("button", { name: "Month" }));

    const note = await screen.findByText(/Sync failed — may be incomplete/i);
    expect(note.closest(".month-cell")).toHaveAttribute("data-day-state", "failed");
  });

  it("leaves a fully synced day unannotated", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    expect(document.querySelector(".day-sync-note")).toBeNull();
  });

  // A dropped participant list is otherwise invisible until the panel is opened.
  it("marks an event card whose calendar metadata came back incomplete", async () => {
    useApi({
      events: [
        {
          ...calendarEvents[0],
          attendees: [],
          metadata_warnings: ["Participant information was incomplete in the calendar response."],
        },
      ],
    });
    render(<App />);
    const card = await screen.findByRole("button", {
      name: /calendar details incomplete/i,
    });
    expect(card.querySelector(".event-incomplete")).not.toBeNull();
  });

  it("does not mark event cards whose metadata was complete", async () => {
    useApi();
    render(<App />);
    await screen.findByRole("button", { name: /Contoso cloud adoption review/i });
    expect(document.querySelector(".event-incomplete")).toBeNull();
  });

  // A disabled sync used to report a load in flight forever, which pinned the
  // shell on the skeleton and made the connection setup unreachable.
  it("shows the connection setup instead of a skeleton when the workspace is not ready", async () => {
    const calls: string[] = [];
    useApi({
      setup: {
        ready: false,
        missing: ["Outlook Calendar"],
        configured_connectors: [],
        instructions: ["Sign in to the calendar connector."],
      },
      onCalendar: (url) => {
        calls.push(url.pathname);
        return undefined;
      },
    });
    render(<App />);
    await screen.findByRole("heading", { name: /Connect your Microsoft 365 workspace/i });
    expect(screen.queryByLabelText("Loading calendar")).toBeNull();
    // Nothing is asked of a backend that has told us it is not ready.
    expect(calls).toEqual([]);
  });
});

describe("meeting open race", () => {
  it("does not open a panel for a meeting that was closed before it resolved", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const routed = mockApi({});
    vi.spyOn(globalThis, "fetch").mockImplementation((async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const path = new URL(
        String(typeof input === "object" && "url" in input ? input.url : input),
        "http://localhost",
      ).pathname;
      // Only the second meeting is slow, so the panel is already open and
      // closeable while its request is still in flight.
      if (path === "/api/meetings/team-sync") await gate;
      return routed(input, init);
    }) as typeof fetch);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Contoso cloud adoption review/i }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(
        "Contoso cloud adoption review",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: /Digital sales team sync/i }));
    // Escape dismisses the panel while the second detail request is still open.
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("heading", { level: 2 })).toBeNull());

    await act(async () => {
      release();
      await Promise.resolve();
    });
    await new Promise((resolve) => setTimeout(resolve, 30));
    // The in-flight open was invalidated by the close: nothing may reappear.
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(screen.queryByLabelText("Meeting details")).toBeNull();
  });

  it("cancels a still-loading meeting from the panel and aborts its request", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const signals: AbortSignal[] = [];
    const routed = mockApi({});
    vi.spyOn(globalThis, "fetch").mockImplementation((async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const path = new URL(
        String(typeof input === "object" && "url" in input ? input.url : input),
        "http://localhost",
      ).pathname;
      if (path.includes("/api/meetings/")) {
        if (init?.signal) signals.push(init.signal);
        await gate;
      }
      return routed(input, init);
    }) as typeof fetch);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Contoso cloud adoption review/i }));
    // The skeleton is the only thing on screen while the detail is loading, so
    // it has to carry its own way out.
    const cancel = await screen.findByRole("button", { name: "Cancel loading meeting" });
    fireEvent.click(cancel);

    await waitFor(() => expect(screen.queryByLabelText("Meeting details")).toBeNull());
    expect(signals.at(-1)?.aborted).toBe(true);

    await act(async () => {
      release();
      await Promise.resolve();
    });
    await new Promise((resolve) => setTimeout(resolve, 30));
    // A cancelled read must not reopen the panel or raise a backend error.
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("closes a loading meeting on Escape before any panel has opened", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const routed = mockApi({});
    vi.spyOn(globalThis, "fetch").mockImplementation((async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const path = new URL(
        String(typeof input === "object" && "url" in input ? input.url : input),
        "http://localhost",
      ).pathname;
      if (path.includes("/api/meetings/")) await gate;
      return routed(input, init);
    }) as typeof fetch);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Contoso cloud adoption review/i }));
    await screen.findByLabelText("Loading meeting");
    fireEvent.keyDown(document, { key: "Escape" });

    await waitFor(() => expect(screen.queryByLabelText("Loading meeting")).toBeNull());
    await act(async () => {
      release();
      await Promise.resolve();
    });
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
  });

  it("shows the meeting clicked last when two open requests overlap", async () => {
    let releaseFirst: () => void = () => {};
    const first = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    const routed = mockApi({
      meeting: detail,
      onMeeting: () => undefined,
    });
    vi.spyOn(globalThis, "fetch").mockImplementation((async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const url = String(typeof input === "object" && "url" in input ? input.url : input);
      const path = new URL(url, "http://localhost").pathname;
      if (path === "/api/meetings/contoso-qbr") {
        await first;
        return new Response(JSON.stringify({ ...detail, briefing: { ...detail.briefing, headline: "STALE HEADLINE" } }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      if (path === "/api/meetings/team-sync") {
        return new Response(JSON.stringify({ ...detail, event: internalEvent }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return routed(input, init);
    }) as typeof fetch);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Contoso cloud adoption review/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Digital sales team sync/i }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(
        "Digital sales team sync",
      ),
    );

    await act(async () => {
      releaseFirst();
      await Promise.resolve();
    });
    await new Promise((resolve) => setTimeout(resolve, 30));
    // The slower first request must not overwrite the panel the user is reading.
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Digital sales team sync");
    expect(screen.queryByText("STALE HEADLINE")).toBeNull();
  });
});

describe("analysis progress honesty", () => {
  it(
    "flags progress as unavailable when the status poll fails and clears it on recovery",
    async () => {
      let failing = false;
      useApi({
        analysis: { total: 3, ready: 1, queued: 1, running: 1, not_started: 0, percent: 33 },
        onAnalysis: () => (failing ? new Response("nope", { status: 503 }) : undefined),
      });
      render(<App />);
      await screen.findByText(/Preparing meeting briefings/);
      expect(screen.queryByRole("note")).toBeNull();

      failing = true;
      // The bar keeps its last numbers, but it must say they stopped updating.
      await screen.findByText(/Progress updates unavailable/, undefined, { timeout: 12_000 });
      expect(screen.getByText(/Preparing meeting briefings/)).toBeInTheDocument();

      failing = false;
      await waitFor(() => expect(screen.queryByRole("note")).toBeNull(), { timeout: 12_000 });
    },
    30_000,
  );
});