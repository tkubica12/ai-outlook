import { describe, expect, it, vi } from "vitest";
import { api } from "./api";

const jsonResponse = (body: unknown, status: number) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

describe("api error handling", () => {
  it("surfaces a FastAPI string detail", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(jsonResponse({ detail: "Meeting not found" }, 404));
    await expect(api.meeting("nope")).rejects.toThrow("Meeting not found");
    vi.restoreAllMocks();
  });

  it("flattens a 422 validation detail array instead of rendering [object Object]", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      jsonResponse(
        { detail: [{ loc: ["body", "message"], msg: "field required", type: "missing" }] },
        422,
      ),
    );
    await expect(api.chat("id", "")).rejects.toThrow("field required");
    vi.restoreAllMocks();
  });

  it("falls back to the status code when the body is not JSON", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("boom", { status: 500 }));
    await expect(api.calendar()).rejects.toThrow("Request failed (500)");
    vi.restoreAllMocks();
  });

  it("explains an unreachable backend instead of leaking 'Failed to fetch'", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("Failed to fetch"));
    await expect(api.calendar()).rejects.toThrow(/backend is running/);
    vi.restoreAllMocks();
  });

  it("returns parsed JSON on success", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(jsonResponse([{ id: "a" }], 200));
    await expect(api.calendar()).resolves.toEqual([{ id: "a" }]);
    vi.restoreAllMocks();
  });

  it("rethrows an abort instead of blaming the backend", async () => {
    const abort = new DOMException("aborted", "AbortError");
    vi.spyOn(globalThis, "fetch").mockRejectedValue(abort);
    await expect(api.calendar("2026-03-01", "2026-03-08")).rejects.toBe(abort);
    vi.restoreAllMocks();
  });
});

describe("api calendar range", () => {
  const capture = () => {
    const fetchMock = vi.fn<(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>>(
      async () => jsonResponse([], 200),
    );
    vi.spyOn(globalThis, "fetch").mockImplementation(fetchMock as unknown as typeof fetch);
    return fetchMock;
  };

  it("sends both range bounds as query parameters", async () => {
    const fetchMock = capture();
    await api.calendar("2026-03-01", "2026-03-09");
    const url = String(fetchMock.mock.calls[0][0]);
    expect(url).toContain("/api/calendar?");
    const params = new URL(url, "http://localhost").searchParams;
    expect(params.get("start_date")).toBe("2026-03-01");
    expect(params.get("end_date")).toBe("2026-03-09");
    vi.restoreAllMocks();
  });

  it("omits the query entirely when no range is given", async () => {
    const fetchMock = capture();
    await api.calendar();
    expect(String(fetchMock.mock.calls[0][0])).not.toContain("?");
    vi.restoreAllMocks();
  });

  it("never sends a half range the backend would reject", async () => {
    const fetchMock = capture();
    await api.calendar("2026-03-01");
    expect(String(fetchMock.mock.calls[0][0])).not.toContain("start_date");
    vi.restoreAllMocks();
  });

  it("posts the retry request without a body", async () => {
    const fetchMock = capture();
    await api.retryCalendar();
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(String(url)).toContain("/api/calendar/retry");
    expect(init.method).toBe("POST");
    vi.restoreAllMocks();
  });

  it("reads the status endpoint", async () => {
    const fetchMock = capture();
    await api.calendarStatus();
    expect(String(fetchMock.mock.calls[0][0])).toContain("/api/calendar-status");
    vi.restoreAllMocks();
  });
});

describe("api chat history", () => {
  const capture = () => {
    const fetchMock = vi.fn<(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>>(
      async () => jsonResponse({ answer: "ok", sources: [] }, 200),
    );
    vi.spyOn(globalThis, "fetch").mockImplementation(fetchMock as unknown as typeof fetch);
    return fetchMock;
  };

  it("sends an empty history when none is supplied", async () => {
    const fetchMock = capture();
    await api.chat("m1", "hello");
    const init = fetchMock.mock.calls[0][1] as unknown as RequestInit;
    expect(JSON.parse(String(init.body))).toEqual({ message: "hello", history: [] });
    vi.restoreAllMocks();
  });

  it("forwards prior turns in order", async () => {
    const fetchMock = capture();
    await api.chat("m1", "and then?", [
      { role: "user", text: "first" },
      { role: "assistant", text: "answer" },
    ]);
    const init = fetchMock.mock.calls[0][1] as unknown as RequestInit;
    expect(JSON.parse(String(init.body)).history).toEqual([
      { role: "user", text: "first" },
      { role: "assistant", text: "answer" },
    ]);
    vi.restoreAllMocks();
  });
});
