import {
  Bell,
  ChevronLeft,
  ChevronRight,
  Menu,
  Moon,
  Search,
  Settings2,
  Sun,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import briefingsIcon from "./assets/generated/briefings-icon.png";
import calendarIcon from "./assets/generated/calendar-icon.png";
import { CalendarView } from "./components/CalendarView";
import { AnalysisProgress, type AnalysisState } from "./components/AnalysisProgress";
import { CalendarSyncStatus } from "./components/CalendarSyncStatus";
import { BriefingsView } from "./components/BriefingsView";
import { ConnectionSetup } from "./components/ConnectionSetup";
import { Diagnostics } from "./components/Diagnostics";
import { DuckPilotLogo } from "./components/DuckPilotLogo";
import { MeetingPanel } from "./components/MeetingPanel";
import { MiniCalendar } from "./components/MiniCalendar";
import { useCalendarSync } from "./useCalendarSync";
import { useDialog } from "./useDialog";
import {
  dayKey,
  fetchRange,
  formatRangeLabel,
  isCurrentPeriod,
  isInRange,
  selectionRange,
  shiftAnchor,
  startOfDay,
  startOfMonth,
  VIEW_RANGE_NOUN,
  type ViewMode,
} from "./dates";
import type { CalendarEvent, MeetingDetail } from "./types";

export const PRODUCT_NAME = "Tomlook";

const VIEWS: ViewMode[] = ["day", "workweek", "week", "month"];
const VIEW_LABEL: Record<ViewMode, string> = {
  day: "Day",
  workweek: "Work week",
  week: "Week",
  month: "Month",
};
const CATEGORY_PALETTE = ["#6554d9", "#2188c7", "#159577", "#d47a24", "#c04a87", "#7957d5", "#5b8c31", "#b4513e"];

const NOT_IN_MVP = "Not available until the write-capable Microsoft 365 flow is approved";

function App() {
  const [selected, setSelected] = useState<MeetingDetail | null>(null);
  const [view, setView] = useState<ViewMode>(() =>
    typeof window !== "undefined" && window.matchMedia?.("(max-width: 820px)").matches
      ? "day"
      : "week",
  );
  const [anchor, setAnchor] = useState(() => startOfDay(new Date()));
  const [miniMonth, setMiniMonth] = useState(() => startOfMonth(new Date()));
  const [setupLoading, setSetupLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState("");
  const [dark, setDark] = useState(() => localStorage.getItem("theme") === "dark");
  const [diagnostics, setDiagnostics] = useState(false);
  const [navOpen, setNavOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [hidden, setHidden] = useState<string[]>([]);
  const [briefingsOnly, setBriefingsOnly] = useState(false);
  const [setup, setSetup] = useState<Awaited<ReturnType<typeof api.setup>> | null>(null);
  const [analysis, setAnalysis] = useState<AnalysisState | null>(null);
  const [analysisStale, setAnalysisStale] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const triggerRef = useRef<HTMLElement | null>(null);
  const triggerEventIdRef = useRef<string | null>(null);
  const selectedRef = useRef<MeetingDetail | null>(null);
  const openRequestRef = useRef(0);
  const openAbortRef = useRef<AbortController | null>(null);
  const detailLoadingRef = useRef(false);
  const navOpenRef = useRef(false);
  const navRef = useDialog<HTMLElement>(() => setNavOpen(false), navOpen);
  selectedRef.current = selected;
  navOpenRef.current = navOpen;
  detailLoadingRef.current = detailLoading;

  // The exact window the grid paints, so the backend is never asked for a
  // different range than the one on screen. `end` is exclusive and the month
  // matrix is six full weeks, which is also the API's 42-day ceiling.
  const window_ = useMemo(() => fetchRange(anchor, view), [anchor, view]);
  const sync = useCalendarSync({
    startDate: window_.start_date,
    endDate: window_.end_date,
    days: window_.days,
    enabled: setup?.ready === true,
  });

  // Returns focus to the event card that opened the panel instead of dropping
  // keyboard users back on <body>.
  const closeMeeting = useCallback(() => {
    // Invalidate any detail request still in flight so it cannot open a panel
    // the user has already dismissed, and stop the request itself: a slow
    // meeting read would otherwise keep the connector busy for nothing.
    openRequestRef.current += 1;
    openAbortRef.current?.abort();
    openAbortRef.current = null;
    setDetailLoading(false);
    setSelected(null);
    const trigger = triggerRef.current;
    const eventId = triggerEventIdRef.current;
    triggerRef.current = null;
    triggerEventIdRef.current = null;
    const restore = () => {
      if (trigger?.isConnected) trigger.focus();
      else if (eventId) {
        Array.from(document.querySelectorAll<HTMLElement>("[data-event-id]"))
          .find((element) => element.dataset.eventId === eventId)
          ?.focus();
      }
    };
    restore();
    queueMicrotask(restore);
  }, []);

  /**
   * The single way the calendar changes date. Moving the main view always pulls
   * the mini calendar to the same month; browsing the mini calendar on its own
   * does not call this, so it never drags the main view along.
   */
  const goToDate = useCallback((date: Date) => {
    setAnchor(startOfDay(date));
    setMiniMonth(startOfMonth(date));
  }, []);

  const loadSetup = useCallback(async () => {
    setSetupLoading(true);
    try {
      setError("");
      setSetup(await api.setup());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not load calendar");
    } finally {
      setSetupLoading(false);
    }
  }, []);

  useEffect(() => void loadSetup(), [loadSetup]);

  const reloadCalendar = sync.reload;
  const loadCalendar = useCallback(async () => {
    await loadSetup();
    reloadCalendar();
  }, [loadSetup, reloadCalendar]);

  // Analysis status runs on its own self-rescheduling loop so a slow response
  // cannot stack requests, and it keeps polling with zero events because a
  // calendar shard that lands later still needs its briefings tracked.
  useEffect(() => {
    if (!setup?.ready) return;
    let disposed = false;
    let timer = 0;
    const update = async () => {
      try {
        const current = await api.analysisStatus();
        if (!disposed) {
          setAnalysis(current);
          setAnalysisStale(false);
        }
      } catch {
        // The calendar stays usable, but the last numbers must not keep passing
        // for live progress: say so instead of silently freezing the bar.
        if (!disposed) setAnalysisStale(true);
      } finally {
        if (!disposed) timer = window.setTimeout(() => void update(), 3000);
      }
    };
    void update();
    return () => {
      disposed = true;
      window.clearTimeout(timer);
    };
  }, [setup?.ready]);

  /**
   * Analysis state is overlaid rather than written back into the calendar, so a
   * late status response can never resurrect events from a range the user has
   * already navigated away from.
   */
  const events = useMemo<CalendarEvent[]>(() => {
    const states = new Map((analysis?.meetings ?? []).map((item) => [item.meeting_id, item.status]));
    return sync.events.map((item) => {
      // A meeting that needs no briefing must stay inert; the analysis feed
      // reports every id and would otherwise mark it ready.
      if (item.briefing_status === "not_required") return item;
      const status = states.get(item.id);
      const briefingStatus =
        status === "ready"
          ? "ready"
          : status === "failed"
            ? "error"
            : status === "queued" || status === "running"
              ? "analyzing"
              : item.briefing_status;
      return briefingStatus === item.briefing_status
        ? item
        : { ...item, briefing_status: briefingStatus };
    });
  }, [sync.events, analysis]);

  const loading = setupLoading || sync.initialLoading;

  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    localStorage.setItem("theme", dark ? "dark" : "light");
  }, [dark]);

  useEffect(() => {
    if (!navOpen) return;
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, [navOpen]);

  // Escape closes the top-most surface; Cmd/Ctrl+K focuses search.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        searchRef.current?.focus();
        return;
      }
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (navOpenRef.current) setNavOpen(false);
      else if (selectedRef.current || detailLoadingRef.current) closeMeeting();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [closeMeeting]);

  /**
   * Rapidly clicking between meetings, or closing the panel mid-flight, must
   * never land an older detail response on screen. Each open takes a ticket and
   * only the newest ticket is allowed to publish; closing bumps the ticket too.
   */
  const openMeeting = async (event: CalendarEvent) => {
    if (event.briefing_status === "not_required") return;
    const ticket = openRequestRef.current + 1;
    openRequestRef.current = ticket;
    openAbortRef.current?.abort();
    const controller = new AbortController();
    openAbortRef.current = controller;
    triggerRef.current = document.activeElement as HTMLElement | null;
    triggerEventIdRef.current = event.id;
    setDetailLoading(true);
    setError("");
    try {
      const detail = await api.meeting(event.id, { signal: controller.signal });
      if (openRequestRef.current !== ticket) return;
      setSelected(detail);
    } catch (reason) {
      if (openRequestRef.current !== ticket) return;
      setSelected(null);
      setError(reason instanceof Error ? reason.message : "Could not open meeting");
    } finally {
      if (openRequestRef.current === ticket) {
        openAbortRef.current = null;
        setDetailLoading(false);
      }
    }
  };

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return events.filter((event) => {
      if (hidden.includes(event.category)) return false;
      if (briefingsOnly && event.briefing_status === "not_required") return false;
      if (!needle) return true;
      return [event.title, event.location, event.organizer, ...(event.attendees ?? [])]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [events, query, hidden, briefingsOnly]);
  const categories = useMemo(
    () => [...new Set(events.map((event) => event.category || "Calendar"))].sort(),
    [events],
  );
  const categoryColors = useMemo(
    () => Object.fromEntries(categories.map((category, index) => [category, CATEGORY_PALETTE[index % CATEGORY_PALETTE.length]])),
    [categories],
  );

  const busyDays = useMemo(
    () => new Set(visible.map((event) => dayKey(new Date(event.start)))),
    [visible],
  );
  const briefingCount = useMemo(
    () => events.filter((event) => event.briefing_status === "ready").length,
    [events],
  );
  const dateLabel = useMemo(() => formatRangeLabel(anchor, view), [anchor, view]);
  const range = useMemo(() => selectionRange(anchor, view), [anchor, view]);
  // The ring must never sit outside the highlighted period. Work week keeps a
  // weekend anchor so switching back to Day returns the user's actual day, so
  // the mini calendar gets a clamped copy instead of the raw anchor.
  const selectedDate = useMemo(
    () => (isInRange(anchor, range) ? anchor : range.start),
    [anchor, range],
  );
  const filtered = query.trim() !== "" || hidden.length > 0 || briefingsOnly;

  const toggleCategory = (category: string) =>
    setHidden((current) =>
      current.includes(category)
        ? current.filter((item) => item !== category)
        : [...current, category],
    );

  return (
    <div className="app-shell">
      <a className="skip-link" href="#calendar">
        Skip to calendar
      </a>
      <header className="topbar">
        <button
          className="icon-button mobile-only"
          aria-label="Open navigation"
          aria-expanded={navOpen}
          onClick={() => setNavOpen(true)}
        >
          <Menu size={20} />
        </button>
        <a className="brand" href="#calendar" aria-label={`${PRODUCT_NAME} home`}>
          <span className="brand-mark">
            <DuckPilotLogo compact />
          </span>
          <span>{PRODUCT_NAME}</span>
          <span className="preview-pill">Preview</span>
        </a>
        <div className="global-search">
          <Search size={17} aria-hidden="true" />
          <input
            ref={searchRef}
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search meetings"
            placeholder="Search meetings, people, context…"
          />
          {query && (
            <button className="clear-search" aria-label="Clear search" onClick={() => setQuery("")}>
              <X size={14} />
            </button>
          )}
          <kbd aria-hidden="true">⌘ K</kbd>
        </div>
        <div className="top-actions">
          <button className="icon-button" aria-label="Notifications" title={NOT_IN_MVP} disabled>
            <Bell size={19} />
          </button>
          <button
            className="icon-button"
            aria-label={dark ? "Use light theme" : "Use dark theme"}
            aria-pressed={dark}
            onClick={() => setDark((value) => !value)}
          >
            {dark ? <Sun size={19} /> : <Moon size={19} />}
          </button>
          <button className="avatar" aria-label="User profile" title={NOT_IN_MVP} disabled>
            TK
          </button>
        </div>
      </header>

      <div className="workspace">
        <aside
          ref={navRef}
          className={`sidebar ${navOpen ? "open" : ""}`}
          role={navOpen ? "dialog" : undefined}
          aria-modal={navOpen ? "true" : undefined}
          aria-label={navOpen ? "Navigation" : undefined}
        >
          <div className="sidebar-mobile-header">
            <span>Navigation</span>
            <button
              className="icon-button"
              onClick={() => setNavOpen(false)}
              aria-label="Close navigation"
            >
              <X size={18} />
            </button>
          </div>
          <button className="new-event" title={NOT_IN_MVP} disabled>
            <span aria-hidden="true">+</span> New event
          </button>
          <div className="sidebar-search">
            <Search size={16} aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search meetings…"
              aria-label="Search meetings in navigation"
            />
          </div>
          <nav aria-label="Main navigation">
            <button
              className={`nav-item ${briefingsOnly ? "" : "active"}`}
              aria-current={briefingsOnly ? undefined : "page"}
              onClick={() => {
                setBriefingsOnly(false);
                setNavOpen(false);
              }}
            >
              <img className="nav-generated-icon" src={calendarIcon} alt="" /> Calendar
            </button>
            <button
              className={`nav-item ${briefingsOnly ? "active" : ""}`}
              aria-current={briefingsOnly ? "page" : undefined}
              onClick={() => {
                setBriefingsOnly(true);
                setNavOpen(false);
              }}
            >
              <img className="nav-generated-icon" src={briefingsIcon} alt="" /> Briefings
              <span className="nav-count">{briefingCount}</span>
            </button>
          </nav>
          <MiniCalendar
            anchor={miniMonth}
            selectedDate={selectedDate}
            range={range}
            rangeNoun={VIEW_RANGE_NOUN[view]}
            busyDays={busyDays}
            onNavigateMonth={(date) => setMiniMonth(startOfMonth(date))}
            onPick={(date) => {
              // Picking a date moves the selection inside the active view; it
              // never switches to Day behind the user's back.
              goToDate(date);
              setBriefingsOnly(false);
              setNavOpen(false);
            }}
          />
          <fieldset className="calendar-list">
            <legend>My calendars</legend>
            {categories.map((category) => (
              <label key={category}>
                <input
                  type="checkbox"
                  checked={!hidden.includes(category)}
                  onChange={() => toggleCategory(category)}
                />
                <span
                  className="cal-dot"
                  style={{ background: categoryColors[category] }}
                  aria-hidden="true"
                />
                {category}
              </label>
            ))}
          </fieldset>
          <button className="diagnostic-link" onClick={() => setDiagnostics(true)}>
            <Settings2 size={16} aria-hidden="true" /> System diagnostics
          </button>
        </aside>
        {navOpen && (
          // Redundant pointer affordance only: the drawer's close button and
          // Escape are the accessible routes, so this must not duplicate their
          // accessible name or add a second tab stop.
          <div className="scrim" aria-hidden="true" onClick={() => setNavOpen(false)} />
        )}

        <main className="calendar-area" id="calendar">
          <div className="calendar-toolbar">
            <div className="date-controls">
              <button className="today-button" onClick={() => goToDate(new Date())}>
                Today
              </button>
              <button
                className="icon-button compact"
                aria-label="Previous period"
                onClick={() => goToDate(shiftAnchor(anchor, view, -1))}
              >
                <ChevronLeft size={18} />
              </button>
              <button
                className="icon-button compact"
                aria-label="Next period"
                onClick={() => goToDate(shiftAnchor(anchor, view, 1))}
              >
                <ChevronRight size={18} />
              </button>
              <h1 aria-live="polite">{dateLabel}</h1>
              {/* Compare periods, not days: in Work week a weekend "today"
                  still belongs to the displayed Mon–Fri period, so pressing
                  Today must not leave an "off-today" flag behind. */}
              {!isCurrentPeriod(range, view) && <span className="off-today">Not today</span>}
            </div>
            <div className="view-switch" role="group" aria-label="Calendar view">
              {VIEWS.map((mode) => (
                <button
                  key={mode}
                  className={view === mode ? "active" : ""}
                  aria-pressed={view === mode}
                  onClick={() => setView(mode)}
                >
                  {VIEW_LABEL[mode]}
                </button>
              ))}
            </div>
          </div>

          {error && (
            <div className="error-banner" role="alert">
              <span>{error}</span>
              <button onClick={loadCalendar}>Retry</button>
            </div>
          )}
          {!error && sync.error && (
            <div className="error-banner" role="alert">
              <span>{sync.error}</span>
              <button onClick={loadCalendar}>Retry</button>
            </div>
          )}
          <CalendarSyncStatus
            status={sync.status}
            failures={sync.failures}
            failuresElsewhere={sync.failuresElsewhere}
            staleInRange={sync.staleInRange}
            backgroundSync={sync.backgroundSync}
            outstanding={sync.outstanding}
            settled={sync.settled}
            onRetry={() => void sync.retryFailed()}
            retrying={sync.retrying}
          />
          {filtered && !loading && (
            <div className="filter-bar" role="status">
              <span>
                Showing {visible.length} of {events.length} meetings
              </span>
              <button
                onClick={() => {
                  setQuery("");
                  setHidden([]);
                  setBriefingsOnly(false);
                }}
              >
                Clear filters
              </button>
            </div>
          )}
          {analysis && analysis.total > 0 && (
            <AnalysisProgress state={analysis} statusUnavailable={analysisStale} />
          )}
          {!loading && setup && !setup.ready ? (
            <ConnectionSetup
              missing={setup.missing}
              configured={setup.configured_connectors}
              onRetry={loadCalendar}
            />
          ) : briefingsOnly ? (
            <BriefingsView events={visible} onSelect={openMeeting} />
          ) : (
            <CalendarView
              events={visible}
              loading={loading}
              view={view}
              anchor={anchor}
              onSelect={openMeeting}
              onDateSelect={(date) => {
                // Day headers and month cells are an explicit drill-in, so they
                // do change the view — unlike the mini calendar.
                goToDate(date);
                setView("day");
              }}
              categoryColors={categoryColors}
              selectedId={selected?.event.id}
              dayState={sync.dayState}
              emptyHint={
                filtered
                  ? "No meetings match the current filters."
                  : sync.outstanding > 0
                    ? "Still loading this period — days that have not answered yet are marked as pending."
                    : "No meetings in this period. Use Today to return to the current week."
              }
            />
          )}
        </main>

        {(selected || detailLoading) && (
          <aside className="detail-shell" aria-label="Meeting details">
            {detailLoading ? (
              <div
                className="panel-skeleton"
                role="status"
                aria-busy="true"
                aria-label="Loading meeting"
              >
                <button
                  className="icon-button panel-skeleton-close"
                  onClick={closeMeeting}
                  aria-label="Cancel loading meeting"
                >
                  <X size={16} aria-hidden="true" />
                </button>
                <div />
                <div />
                <div />
                <div />
              </div>
            ) : selected ? (
              <MeetingPanel
                detail={selected}
                onClose={closeMeeting}
                onUpdated={setSelected}
              />
            ) : null}
          </aside>
        )}
      </div>
      {diagnostics && <Diagnostics onClose={() => setDiagnostics(false)} />}
    </div>
  );
}

export default App;
