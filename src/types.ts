export type EventStatus = "ready" | "analyzing" | "error" | "not_ready" | "not_required";

export interface CalendarEvent {
  id: string;
  title: string;
  start: string;
  end: string;
  category: string;
  attendees: string[] | null;
  organizer: string;
  location: string;
  status: string;
  briefing_status: EventStatus;
  is_all_day: boolean;
  has_changes: boolean;
  /** Backend-validated http(s) link to the event in its source system. */
  source_url?: string | null;
  /** Fields the connector could not read; shown so gaps are not read as facts. */
  metadata_warnings?: string[];
}

/**
 * Per-date sync state reported by `/api/calendar-status`. Dates are ISO
 * `YYYY-MM-DD` strings in the backend's local calendar day.
 */
export interface CalendarStatus {
  syncing: boolean;
  covered_dates: string[];
  requested_dates: string[];
  pending_dates: string[];
  /** Dates whose shard is executing right now; absent on older backends. */
  syncing_dates?: string[];
  stale_dates: string[];
  failures: Record<string, string>;
  last_synced_at: string | null;
  cache_warning?: string | null;
  cache_error?: string | null;
}

export interface AnalysisMeetingState {
  meeting_id: string;
  status: string;
  source?: string | null;
  /** Failure reason for a failed analysis job. */
  detail?: string | null;
}

export interface AnalysisStatus {
  total: number;
  ready: number;
  queued: number;
  running: number;
  failed: number;
  not_started: number;
  percent: number;
  meetings: AnalysisMeetingState[];
  model?: string;
  concurrency?: number;
}

export interface ChatTurn {
  role: "user" | "assistant";
  text: string;
}

/**
 * `unspecified` is the legacy default and explicitly does NOT mean actual —
 * only `actual` may be presented as measured consumption.
 */
export type ConsumptionKind = "actual" | "partial_actual" | "prediction" | "unspecified";

export interface ConsumptionPoint {
  label: string;
  value: number;
  kind?: ConsumptionKind;
}

export interface Source {
  id: string;
  connector: string;
  title: string;
  timestamp: string;
  evidence_type: "fact" | "synthesis" | "inference" | "recommendation" | "missing";
  url?: string;
  is_mock: boolean;
}

export interface Briefing {
  meeting_id: string;
  version: number;
  created_at: string;
  status: string;
  summary: string;
  objective: string;
  why_now: string;
  role: {
    role: string;
    confidence: number;
    explanation: string;
    evidence_source_ids: string[];
  };
  preparation: string[];
  open_questions: string[];
  risks: string[];
  decisions: string[];
  talking_points: string[];
  communication_context: string[];
  business_context: Record<string, string>;
  consumption: ConsumptionPoint[];
  consumption_unit: string;
  consumption_period: string;
  public_context: string[];
  changes: string[];
  sources: Source[];
  warnings: string[];
  claim_sources?: Record<string, string[]>;
  claim_confidence?: Record<string, number>;
  milestones?: {
    id: string;
    name: string;
    due_date: string;
    status: string;
    opportunity?: string;
    confidence: number;
    association_reason: string;
    has_user_task?: boolean;
    existing_task_subject?: string;
    /** Ids of the briefing sources this association was derived from. */
    source_ids?: string[];
    /** Backend-validated http(s) link to the milestone record. */
    url?: string | null;
    suggested_task?: {
      subject: string;
      due_date: string;
      duration_hours: number;
      activity_type: string;
      milestone_id: string;
      reason: string;
      status: string;
    };
  }[];
  analysis_profile?: string;
}

export interface MeetingDetail {
  event: CalendarEvent;
  invitation: string;
  response_status: string;
  briefing: Briefing;
}

export interface SkillProposal {
  id: string;
  title: string;
  reason: string;
  status: string;
  old_content: string;
  new_content: string;
  created_at: string;
}
