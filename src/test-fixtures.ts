import type { Briefing, CalendarEvent, CalendarStatus, MeetingDetail, Source } from "./types";

const hoursFromNow = (hours: number) => {
  const date = new Date();
  date.setHours(hours, 0, 0, 0);
  return date.toISOString();
};

export const sources: Source[] = [
  {
    id: "mail-1",
    connector: "Outlook Mail",
    title: "RE: Contoso adoption plan",
    timestamp: new Date().toISOString(),
    evidence_type: "fact",
    url: "https://example.invalid/mail",
    is_mock: false,
  },
  {
    id: "usage-1",
    connector: "Power BI",
    title: "Azure consumption",
    timestamp: new Date().toISOString(),
    evidence_type: "fact",
    is_mock: false,
  },
];

export const customerEvent: CalendarEvent = {
  id: "contoso-qbr",
  title: "Contoso cloud adoption review",
  start: hoursFromNow(9),
  end: hoursFromNow(10),
  category: "Customer",
  attendees: ["Avery Stone", "You"],
  organizer: "Avery Stone",
  location: "Microsoft Teams",
  status: "accepted",
  briefing_status: "ready",
  is_all_day: false,
  has_changes: true,
};

export const internalEvent: CalendarEvent = {
  ...customerEvent,
  id: "team-sync",
  title: "Digital sales team sync",
  start: hoursFromNow(11),
  end: hoursFromNow(12),
  category: "Internal",
  organizer: "Nora Hill",
  attendees: ["Nora Hill", "You"],
  has_changes: false,
};

export const focusEvent: CalendarEvent = {
  ...customerEvent,
  id: "focus",
  title: "Focus time — proposal draft",
  start: hoursFromNow(13),
  end: hoursFromNow(14),
  category: "Focus",
  briefing_status: "not_required",
  attendees: ["You"],
  location: "",
  has_changes: false,
};

export const briefing: Briefing = {
  meeting_id: customerEvent.id,
  version: 2,
  created_at: new Date().toISOString(),
  status: "ready",
  summary: "Key briefing summary",
  objective: "Agree next steps",
  why_now: "Checkpoint this week",
  role: {
    role: "Expert contributor",
    confidence: 0.82,
    explanation: "You own the technical workstream.",
    evidence_source_ids: ["mail-1"],
  },
  preparation: ["Confirm the architecture recommendation"],
  open_questions: ["Is the landing zone approved?"],
  risks: ["Association is inferred"],
  decisions: [],
  talking_points: ["Lead with progress"],
  communication_context: ["Avery asked for a recommendation"],
  business_context: { customer: "Contoso (suggested)" },
  consumption: [
    { label: "Aug", value: 77, kind: "actual" },
    { label: "Sep", value: 91, kind: "prediction" },
  ],
  consumption_unit: "k EUR / month",
  consumption_period: "Aug–Sep 2026",
  public_context: [],
  changes: ["New email: security approval expected Friday"],
  sources,
  warnings: ["Test fixture data."],
  milestones: [
    {
      id: "milestone-1",
      name: "Production readiness",
      due_date: "2026-10-15",
      status: "On Track",
      opportunity: "Cloud expansion",
      confidence: 0.84,
      association_reason: "Confirmed customer and opportunity context.",
      has_user_task: false,
      suggested_task: {
        subject: "Prepare production readiness review",
        due_date: "2026-10-10",
        duration_hours: 1,
        activity_type: "Architecture Review",
        milestone_id: "milestone-1",
        reason: "No owned task exists for the upcoming milestone.",
        status: "draft",
      },
    },
  ],
  analysis_profile: "business-v2",
};

export const detail: MeetingDetail = {
  event: customerEvent,
  invitation: "Test invitation.",
  response_status: "accepted",
  briefing,
};

export const calendarEvents = [customerEvent, internalEvent, focusEvent];

/** Every day of the fixture window reported as synced, so nothing is pending. */
/**
 * A backend that has already synced everything the UI can ask for: coverage is
 * reported for the whole ±45 day window so no view reports unqueried days.
 */
export function coveredStatus(events: CalendarEvent[] = calendarEvents): CalendarStatus {
  void events;
  const dates: string[] = [];
  const cursor = new Date();
  cursor.setDate(cursor.getDate() - 45);
  for (let index = 0; index < 91; index += 1) {
    const year = cursor.getFullYear();
    const month = `${cursor.getMonth() + 1}`.padStart(2, "0");
    const day = `${cursor.getDate()}`.padStart(2, "0");
    dates.push(`${year}-${month}-${day}`);
    cursor.setDate(cursor.getDate() + 1);
  }
  return {
    syncing: false,
    covered_dates: dates,
    requested_dates: dates,
    pending_dates: [],
    syncing_dates: [],
    stale_dates: [],
    failures: {},
    last_synced_at: new Date().toISOString(),
    cache_warning: null,
    cache_error: null,
  };
}

interface RouteOptions {
  events?: CalendarEvent[];
  meeting?: MeetingDetail;
  calendarStatus?: CalendarStatus;
  setup?: Record<string, unknown>;
  onCalendar?: (url: URL) => Response | undefined;
  onCalendarStatus?: () => Response | undefined;
  onRetry?: () => Response | undefined;
  onMeeting?: () => Response | undefined;
  onAnalysis?: () => Response | undefined;
  onFeedback?: () => Response | undefined;
  onChat?: (body: unknown) => Response | undefined;
  analysis?: Record<string, unknown>;
}

/** Routes the handful of endpoints the UI touches, so tests never hit the network. */
export function mockApi(options: RouteOptions = {}) {
  const events = options.events ?? calendarEvents;
  const meeting = options.meeting ?? detail;
  const ok = (body: unknown) =>
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });

  return async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = String(typeof input === "object" && "url" in input ? input.url : input);
    // Match on the path only: the calendar range is carried in the query string.
    const url = new URL(raw, "http://localhost");
    const path = url.pathname;
    if (path === "/api/setup")
      return ok({
        ready: true,
        missing: [],
        configured_connectors: ["Outlook Calendar"],
        instructions: [],
        ...options.setup,
      });
    if (path === "/api/calendar") return options.onCalendar?.(url) ?? ok(events);
    if (path === "/api/calendar-status")
      return options.onCalendarStatus?.() ?? ok(options.calendarStatus ?? coveredStatus(events));
    if (path === "/api/calendar/retry")
      return options.onRetry?.() ?? ok(options.calendarStatus ?? coveredStatus(events));
    if (path === "/api/analysis-status")
      return (
        options.onAnalysis?.() ??
        ok({
          total: events.length,
          ready: events.filter((event) => event.briefing_status === "ready").length,
          queued: 0,
          running: 0,
          failed: 0,
          not_started: 0,
          percent: 100,
          model: "test-model",
          concurrency: 4,
          meetings: events.map((event) => ({
            meeting_id: event.id,
            status: event.briefing_status === "not_required" ? "not_required" : "ready",
            detail: null,
          })),
          ...options.analysis,
        })
      );
    if (path.includes("/refresh")) return ok({ id: "job-1", status: "running" });
    if (path.includes("/api/jobs/")) return ok({ id: "job-1", status: "completed" });
    if (path.includes("/chat")) {
      const body = init?.body ? JSON.parse(String(init.body)) : {};
      return (
        options.onChat?.(body) ??
        ok({ id: "c1", answer: "Your likely role is **Expert contributor**.", sources })
      );
    }
    if (path.includes("/feedback"))
      return options.onFeedback?.() ?? ok({ saved: true, scope: "meeting" });
    if (path.includes("/api/skill-proposals/"))
      return ok({
        id: "p1",
        title: "Clarify role evidence",
        reason: "Needs two sources",
        status: path.endsWith("approve") ? "approved" : "rejected",
        old_content: "old",
        new_content: "new",
        created_at: new Date().toISOString(),
      });
    if (path === "/api/health")
      return ok({
        status: "healthy",
        runtime: "Not configured",
        database: "SQLite",
        mode: "live",
        active_jobs: 0,
      });
    if (path === "/api/connectors")
      return ok([
        {
          id: "calendar",
          label: "Outlook Calendar",
          capabilities: ["list_events"],
          status: "available",
          mode: "live",
          last_checked: new Date().toISOString(),
        },
      ]);
    if (path.includes("/api/meetings/")) return options.onMeeting?.() ?? ok(meeting);
    return ok({});
  };
}
