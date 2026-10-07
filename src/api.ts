import type {
  AnalysisStatus,
  CalendarEvent,
  CalendarStatus,
  ChatTurn,
  MeetingDetail,
  SkillProposal,
  Source,
} from "./types";

/**
 * FastAPI returns `detail` as a string for HTTPException but as a list of
 * objects for 422 validation errors. Rendering the raw value produced
 * "[object Object]" in the UI, so both shapes are normalised here.
 */
function describeError(payload: unknown, status: number): string {
  const fallback = `Request failed (${status})`;
  if (typeof payload === "string" && payload.trim()) return payload;
  if (!payload || typeof payload !== "object") return fallback;
  const detail = (payload as { detail?: unknown }).detail;
  if (typeof detail === "string" && detail.trim()) return detail;
  if (Array.isArray(detail)) {
    const messages = detail
      .map((item) =>
        item && typeof item === "object" && typeof (item as { msg?: unknown }).msg === "string"
          ? (item as { msg: string }).msg
          : null,
      )
      .filter((item): item is string => Boolean(item));
    if (messages.length) return messages.join("; ");
  }
  return fallback;
}

async function request<T>(url: string, options?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, {
      ...options,
      headers: { "Content-Type": "application/json", ...options?.headers },
    });
  } catch (reason) {
    // An aborted request is the caller superseding itself, not a dead backend.
    if (reason instanceof DOMException && reason.name === "AbortError") throw reason;
    throw new Error("Cannot reach the local API. Check that the backend is running.");
  }
  if (!response.ok) {
    const payload = await response.json().catch(() => null);
    throw new Error(describeError(payload, response.status));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export const api = {
  setup: () =>
    request<{
      ready: boolean;
      missing: string[];
      configured_connectors: string[];
      instructions: string[];
    }>("/api/setup"),
  /**
   * Both bounds or neither: the backend rejects a half-specified window with
   * 422, and `end_date` is exclusive.
   */
  calendar: (startDate?: string, endDate?: string, init?: RequestInit) => {
    const query =
      startDate && endDate
        ? `?${new URLSearchParams({ start_date: startDate, end_date: endDate })}`
        : "";
    return request<CalendarEvent[]>(`/api/calendar${query}`, init);
  },
  calendarStatus: (init?: RequestInit) =>
    request<CalendarStatus>("/api/calendar-status", init),
  /** Clears the per-day retry budget so exhausted shards are attempted again. */
  retryCalendar: () =>
    request<CalendarStatus>("/api/calendar/retry", { method: "POST" }),
  analysisStatus: (init?: RequestInit) => request<AnalysisStatus>("/api/analysis-status", init),
  meeting: (id: string, init?: RequestInit) => request<MeetingDetail>(`/api/meetings/${id}`, init),
  refresh: (id: string) =>
    request<{ id: string; status: string }>(`/api/meetings/${id}/refresh`, {
      method: "POST",
    }),
  job: (id: string) =>
    request<{ id: string; status: string; detail?: string }>(`/api/jobs/${id}`),
  chat: (id: string, message: string, history: ChatTurn[] = []) =>
    request<{ id: string; answer: string; sources: Source[] }>(
      `/api/meetings/${id}/chat`,
      { method: "POST", body: JSON.stringify({ message, history }) },
    ),
  feedback: (id: string, message: string, scope: string) =>
    request<{ saved: boolean; scope: string; proposal?: SkillProposal }>(
      `/api/meetings/${id}/feedback`,
      { method: "POST", body: JSON.stringify({ message, scope }) },
    ),
  decideProposal: (id: string, action: "approve" | "reject") =>
    request<SkillProposal>(`/api/skill-proposals/${id}/${action}`, {
      method: "POST",
    }),
  diagnostics: () =>
    Promise.all([
      request<{
        status: string;
        runtime: string;
        database: string;
        mode: string;
        active_jobs: number;
      }>("/api/health"),
      request<
        {
          id: string;
          label: string;
          capabilities: string[];
          status: string;
          mode: string;
        }[]
      >("/api/connectors"),
    ]),
};
