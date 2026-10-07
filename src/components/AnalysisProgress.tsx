import { AlertTriangle, CheckCircle2, LoaderCircle, Sparkles } from "lucide-react";

export interface AnalysisMeetingState {
  meeting_id: string;
  status: string;
  source?: string | null;
  detail?: string | null;
}

export interface AnalysisState {
  total: number;
  ready: number;
  queued: number;
  running: number;
  failed: number;
  not_started: number;
  percent: number;
  meetings?: AnalysisMeetingState[];
  /** Model actually used by the backend runtime. */
  model?: string;
  /** Number of analysis workers the backend runs in parallel. */
  concurrency?: number;
}

export function AnalysisProgress({
  state,
  statusUnavailable = false,
}: {
  state: AnalysisState;
  /** The progress endpoint stopped answering, so these numbers are stale. */
  statusUnavailable?: boolean;
}) {
  const active = state.queued + state.running > 0;
  // Meetings that need no briefing are not work, so they must not inflate the
  // denominator or make an unfinished run look complete.
  const notRequired = (state.meetings ?? []).filter(
    (meeting) => meeting.status === "not_required",
  ).length;
  const tracked = Math.max(state.total - notRequired, 0);
  const outstanding = state.queued + state.running + state.not_started;
  const complete = !active && outstanding === 0;
  const failureDetail = (state.meetings ?? []).find(
    (meeting) => meeting.status === "failed" && meeting.detail,
  )?.detail;
  return (
    <section
      className={`analysis-progress ${active ? "active" : complete ? "complete" : "idle"}`}
      aria-live="polite"
    >
      <div className="analysis-progress-icon">
        {active ? <LoaderCircle className="spin" size={19} /> : <CheckCircle2 size={19} />}
      </div>
      <div className="analysis-progress-copy">
        <div>
          <strong>
            {active
              ? "Preparing meeting briefings"
              : outstanding > 0
                ? `${outstanding} briefing${outstanding === 1 ? "" : "s"} not started yet`
                : state.failed > 0
                  ? "Background preparation finished with failures"
                  : "Background preparation complete"}
          </strong>
          <span>{state.percent}%</span>
        </div>
        <div className="progress-track" aria-label={`${state.percent}% complete`}>
          <span style={{ width: `${state.percent}%` }} />
        </div>
        <p>
          <span><CheckCircle2 size={12} /> {state.ready} of {tracked} ready</span>
          {state.running > 0 && <span><Sparkles size={12} /> {state.running} analyzing</span>}
          {state.queued > 0 && <span>{state.queued} queued</span>}
          {state.not_started > 0 && <span>{state.not_started} not started</span>}
          {state.failed > 0 && <span className="failed"><AlertTriangle size={12} /> {state.failed} failed</span>}
          {notRequired > 0 && <span>{notRequired} need no briefing</span>}
        </p>
        {(state.model || state.concurrency) && (
          <p className="analysis-progress-runtime">
            {state.model && <span>Model {state.model}</span>}
            {state.concurrency ? <span>{state.concurrency} parallel workers</span> : null}
          </p>
        )}
        {failureDetail && <p className="analysis-progress-failure">{failureDetail}</p>}
        {statusUnavailable && (
          <p className="analysis-progress-stale" role="note">
            <AlertTriangle size={12} aria-hidden="true" /> Progress updates unavailable — the
            numbers above are the last known values.
          </p>
        )}
      </div>
    </section>
  );
}
