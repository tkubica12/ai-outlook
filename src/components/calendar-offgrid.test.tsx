import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { CalendarView } from "./CalendarView";
import type { CalendarEvent } from "../types";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const iso = (hour: number, minute = 0) =>
  new Date(2026, 8, 9, hour, minute, 0, 0).toISOString();

const event = (id: string, from: number, to: number): CalendarEvent => ({
  id,
  title: `Meeting ${id}`,
  start: iso(from),
  end: iso(to),
  category: "Calendar",
  attendees: [],
  organizer: "Organizer",
  location: "",
  status: "confirmed",
  briefing_status: "not_ready",
  is_all_day: false,
  has_changes: false,
});

const anchor = new Date(2026, 8, 9, 12, 0, 0, 0);

/**
 * The timed grid is capped to a readable hour window, so a 03:00 call is not
 * drawn. It must still be counted and reachable rather than silently dropped.
 */
describe("meetings outside the drawn hour window", () => {
  const events = [event("in-1", 9, 10), event("in-2", 14, 15), event("early", 3, 4)];

  it("never draws fewer cards than it admits to", () => {
    render(
      <CalendarView events={events} loading={false} view="day" anchor={anchor} onSelect={() => {}} />,
    );
    const cards = document.querySelectorAll(".day-column .event-card");
    const offGrid = screen.getByRole("button", { name: /fall outside the .* grid/i });
    expect(cards).toHaveLength(2);
    expect(offGrid).toHaveTextContent("+1 off-grid");
    expect(cards.length + 1).toBe(events.length);
  });

  it("opens the full day list so the hidden meeting stays reachable", () => {
    render(
      <CalendarView events={events} loading={false} view="day" anchor={anchor} onSelect={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /falls? outside the .* grid/i }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Meeting early")).toBeInTheDocument();
  });

  it("stays silent when every meeting is drawn", () => {
    render(
      <CalendarView
        events={[event("in-1", 9, 10)]}
        loading={false}
        view="day"
        anchor={anchor}
        onSelect={() => {}}
      />,
    );
    expect(screen.queryByText(/off-grid/)).not.toBeInTheDocument();
  });
});
