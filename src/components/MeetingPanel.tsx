import {
  AlertTriangle,
  ArrowUp,
  BookOpenCheck,
  BriefcaseBusiness,
  CalendarCheck2,
  Check,
  ChevronDown,
  Clock3,
  ExternalLink,
  FileText,
  History,
  Lightbulb,
  LoaderCircle,
  MessageSquareText,
  ListTodo,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Target,
  Users,
  X,
} from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";
import { api } from "../api";
import { useDialog } from "../useDialog";
import type {
  Briefing,
  ChatTurn,
  ConsumptionPoint,
  MeetingDetail,
  SkillProposal,
  Source,
} from "../types";

interface Props {
  detail: MeetingDetail;
  onClose: () => void;
  onUpdated: (detail: MeetingDetail) => void;
}

type Tab = "briefing" | "context" | "chat";
type ChatEntry = { role: "user" | "assistant"; text: string; sources?: Source[]; failed?: boolean };

const TABS: { id: Tab; label: string }[] = [
  { id: "briefing", label: "AI briefing" },
  { id: "context", label: "Sources & details" },
  { id: "chat", label: "Ask Copilot" },
];

/** Matches the backend's `ChatRequest` limits; anything longer is rejected. */
const MAX_CHAT_HISTORY = 12;
const MAX_CHAT_TURN_CHARS = 4000;

const EVIDENCE_LABEL: Record<Source["evidence_type"], string> = {
  fact: "Verified fact",
  synthesis: "Synthesis",
  inference: "Inference",
  recommendation: "Recommendation",
  missing: "Missing data",
};

/** Poll ceiling so a stuck or failed job can never spin the UI forever. */
const MAX_JOB_POLLS = 900;
const JOB_POLL_MS = 1000;

const formatTime = (iso: string) =>
  new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

const message = (error: unknown, fallback: string) =>
  error instanceof Error ? error.message : fallback;

function EvidenceBadge({ type }: { type: Source["evidence_type"] }) {
  return <span className={`evidence ${type}`}>{EVIDENCE_LABEL[type]}</span>;
}

/**
 * Confidence and source count share one line. Joining filtered parts keeps a
 * claim that only has sources from rendering as a dangling "· 2 sources".
 */
function claimMetaLabel(confidence: number | undefined, sourceCount: number): string {
  return [
    confidence === undefined ? null : `${Math.round(confidence * 100)}% confidence`,
    sourceCount > 0 ? `${sourceCount} source${sourceCount > 1 ? "s" : ""}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

function ClaimMeta({
  claim,
  briefing,
}: {
  claim: string;
  briefing: Briefing;
}) {
  const sourceMap = new Map(briefing.sources.map((source) => [source.id, source]));
  const sources = (briefing.claim_sources?.[claim] ?? [])
    .map((id) => sourceMap.get(id))
    .filter((source): source is Source => Boolean(source));
  const confidence = briefing.claim_confidence?.[claim];
  if (confidence === undefined && sources.length === 0) return null;
  const linked = sources.find((source) => source.url);
  const label = claimMetaLabel(confidence, sources.length);
  const title = `${label}\n${sources.map((source) => source.title).join("\n")}`.trim();
  return linked ? (
    <a className="claim-meta" href={linked.url} target="_blank" rel="noreferrer noopener" title={title}>
      {label} <ExternalLink size={10} aria-hidden="true" />
    </a>
  ) : <span className="claim-meta" title={title}>{label}</span>;
}

function BulletList({
  items,
  tone = "",
  briefing,
}: {
  items: string[];
  tone?: string;
  briefing?: Briefing;
}) {
  if (items.length === 0) return <p className="empty-note">Nothing recorded for this meeting.</p>;
  const sourceMap = new Map(briefing?.sources.map((source) => [source.id, source]) ?? []);
  return (
    <ul className={`insight-list ${tone}`}>
      {items.map((item, index) => {
        const sources = (briefing?.claim_sources?.[item] ?? [])
          .map((id) => sourceMap.get(id))
          .filter((source): source is Source => Boolean(source));
        const confidence = briefing?.claim_confidence?.[item];
        const linked = sources.find((source) => source.url);
        const title = [
          confidence === undefined ? null : `Confidence: ${Math.round(confidence * 100)}%`,
          sources.length ? `Sources: ${sources.map((source) => source.title).join("; ")}` : "No linked source",
        ].filter(Boolean).join("\n");
        return (
          <li key={`${index}-${item}`} title={title}>
            {linked ? (
              <a href={linked.url} target="_blank" rel="noreferrer noopener">{item}</a>
            ) : item}
            {(confidence !== undefined || sources.length > 0) && (
              <span className="claim-meta" aria-label={title}>
                {claimMetaLabel(confidence, sources.length)}
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function RichText({ text }: { text: string }) {
  return (
    <>
      {text.split(/(\*\*[^*]+\*\*)/g).map((part, index) =>
        part.startsWith("**") && part.endsWith("**") && part.length > 4 ? (
          <strong key={index}>{part.slice(2, -2)}</strong>
        ) : (
          <span key={index}>{part}</span>
        ),
      )}
    </>
  );
}

const CONSUMPTION_KIND_LABEL: Record<string, string> = {
  actual: "Actual",
  partial_actual: "Partial actual",
  prediction: "Predicted",
  unspecified: "Unverified",
};

/**
 * Predictions and legacy `unspecified` points are never presented as measured
 * spend: each bar carries its own provenance so a forecast cannot be read as an
 * invoice.
 */
function ConsumptionChart({ values, unit }: { values: ConsumptionPoint[]; unit: string }) {
  if (values.length === 0) return <p className="empty-note">No consumption data available.</p>;
  const max = Math.max(...values.map((item) => item.value));
  const kindOf = (item: ConsumptionPoint) => item.kind ?? "unspecified";
  const summary = values
    .map((item) => `${item.label}: ${item.value} (${CONSUMPTION_KIND_LABEL[kindOf(item)]})`)
    .join(", ");
  const kinds = [...new Set(values.map(kindOf))];
  const inexact = kinds.filter((kind) => kind !== "actual");
  return (
    <>
      <div className="consumption-chart" role="img" aria-label={`Cloud consumption in ${unit}. ${summary}`}>
        {values.map((item) => (
          <div className="bar-column" key={item.label} data-kind={kindOf(item)}>
            <span className="bar-value">{item.value}</span>
            <div
              className={`bar kind-${kindOf(item)}`}
              style={{ height: max > 0 ? `${(item.value / max) * 100}%` : "5px" }}
            />
            <span>{item.label}</span>
            <small className="bar-kind">{CONSUMPTION_KIND_LABEL[kindOf(item)]}</small>
          </div>
        ))}
      </div>
      {inexact.length > 0 && (
        <p className="consumption-caveat">
          {inexact.map((kind) => CONSUMPTION_KIND_LABEL[kind]).join(" and ")} values are not billed
          actuals.
        </p>
      )}
      <table className="visually-hidden">
        <caption>Cloud consumption ({unit})</caption>
        <tbody>
          {values.map((item) => (
            <tr key={item.label}>
              <th scope="row">{item.label}</th>
              <td>{item.value}</td>
              <td>{CONSUMPTION_KIND_LABEL[kindOf(item)]}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

function SourceRow({ source }: { source: Source }) {
  const body = (
    <>
      <span className="source-icon" aria-hidden="true">
        {source.connector[0]}
      </span>
      <span>
        <strong>{source.title}</strong>
        <small>
          {source.connector} · {new Date(source.timestamp).toLocaleString()}
        </small>
      </span>
      <EvidenceBadge type={source.evidence_type} />
    </>
  );
  // A source without a URL must not become href="#": that navigates away.
  if (!source.url) {
    return (
      <div className="source-item no-link" title="This source has no link target">
        {body}
      </div>
    );
  }
  return (
    <a className="source-item linked" href={source.url} target="_blank" rel="noreferrer noopener">
      {body}
      <ExternalLink size={14} aria-hidden="true" />
    </a>
  );
}

export function MeetingPanel({ detail, onClose, onUpdated }: Props) {
  const [tab, setTab] = useState<Tab>("briefing");
  const [refreshing, setRefreshing] = useState(false);
  const [chatInput, setChatInput] = useState("");
  const [messages, setMessages] = useState<ChatEntry[]>([]);
  const [sending, setSending] = useState(false);
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [feedbackError, setFeedbackError] = useState("");
  const [savingFeedback, setSavingFeedback] = useState(false);
  const [scope, setScope] = useState("meeting");
  const [proposal, setProposal] = useState<SkillProposal | null>(null);
  const [proposalError, setProposalError] = useState("");
  const [attendeesExpanded, setAttendeesExpanded] = useState(false);
  const [taskDraft, setTaskDraft] = useState<NonNullable<NonNullable<Briefing["milestones"]>[number]["suggested_task"]> | null>(null);
  const [toast, setToast] = useState<{ text: string; tone: "ok" | "bad" } | null>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const chatEnd = useRef<HTMLDivElement>(null);
  const panelId = useId();
  const b = detail.briefing;
  const status = detail.event.briefing_status;
  const attendeeLimit = 4;
  // A large invitation list can be dropped by the connector while the rest of
  // the event survives, so an empty list is not evidence that nobody is invited.
  const attendees = detail.event.attendees ?? [];
  const warnings = detail.event.metadata_warnings ?? [];
  const participantWarning = warnings.find((warning) => /participant|attendee/i.test(warning));
  const attendeesUnknown = attendees.length === 0 && Boolean(participantWarning);
  const shownAttendees = attendeesExpanded ? attendees : attendees.slice(0, attendeeLimit);
  const remainingAttendees = Math.max(0, attendees.length - attendeeLimit);

  // Move focus into the panel so keyboard users are not dropped back on <body>.
  useEffect(() => {
    headingRef.current?.focus();
  }, [detail.event.id]);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 6000);
    return () => clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    chatEnd.current?.scrollIntoView?.({ block: "nearest" });
  }, [messages, sending]);

  const refresh = async () => {
    setRefreshing(true);
    setToast(null);
    try {
      const job = await api.refresh(detail.event.id);
      let status = job.status;
      let jobDetail: string | undefined;
      for (let attempt = 0; status === "running" && attempt < MAX_JOB_POLLS; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, JOB_POLL_MS));
        const current = await api.job(job.id);
        status = current.status;
        jobDetail = current.detail;
      }
      if (status === "running") {
        throw new Error("The refresh job is still running. Try again in a moment.");
      }
      if (status === "failed") {
        throw new Error(jobDetail || "The refresh job failed. The previous briefing is unchanged.");
      }
      setToast({ text: "Briefing updated with the latest connected sources.", tone: "ok" });
      onUpdated(await api.meeting(detail.event.id));
    } catch (error) {
      setToast({ text: message(error, "Refresh failed"), tone: "bad" });
    } finally {
      setRefreshing(false);
    }
  };

  const sendChat = async (event: FormEvent) => {
    event.preventDefault();
    const text = chatInput.trim();
    if (!text || sending) return;
    // Only successful turns of *this* meeting are replayed, newest 12, and the
    // question being asked is never duplicated into its own history.
    const history: ChatTurn[] = messages
      .filter((item) => !item.failed && item.text.trim().length > 0)
      .slice(-MAX_CHAT_HISTORY)
      .map((item) => ({ role: item.role, text: item.text.slice(0, MAX_CHAT_TURN_CHARS) }));
    setMessages((items) => [...items, { role: "user", text }]);
    setChatInput("");
    setSending(true);
    try {
      const response = await api.chat(detail.event.id, text, history);
      setMessages((items) => [
        ...items,
        { role: "assistant", text: response.answer, sources: response.sources },
      ]);
    } catch (error) {
      setMessages((items) => [
        ...items,
        { role: "assistant", text: message(error, "Chat failed"), failed: true },
      ]);
    } finally {
      setSending(false);
    }
  };

  const submitFeedback = async (event: FormEvent) => {
    event.preventDefault();
    if (!feedback.trim() || savingFeedback) return;
    setSavingFeedback(true);
    setFeedbackError("");
    try {
      const result = await api.feedback(detail.event.id, feedback.trim(), scope);
      setProposal(result.proposal ?? null);
      setFeedbackOpen(false);
      setFeedback("");
      setToast({
        text:
          scope === "skill"
            ? "Skill change proposal created. It has not been applied."
            : "Feedback saved.",
        tone: "ok",
      });
    } catch (error) {
      setFeedbackError(message(error, "Feedback could not be saved."));
    } finally {
      setSavingFeedback(false);
    }
  };

  const decide = async (action: "approve" | "reject") => {
    if (!proposal) return;
    setProposalError("");
    try {
      setProposal(await api.decideProposal(proposal.id, action));
      setToast({
        text:
          action === "approve"
            ? "Skill proposal approved for future runs."
            : "Skill proposal rejected. Nothing was changed.",
        tone: "ok",
      });
    } catch (error) {
      setProposalError(message(error, "The decision could not be recorded."));
    }
  };

  const sourceMap = useMemo(
    () => new Map(b.sources.map((source) => [source.id, source])),
    [b.sources],
  );
  const roleEvidence = b.role.evidence_source_ids
    .map((id) => sourceMap.get(id)?.connector)
    .filter(Boolean)
    .join(" and ");

  const feedbackRef = useDialog<HTMLFormElement>(() => setFeedbackOpen(false), feedbackOpen);
  const proposalRef = useDialog<HTMLDivElement>(() => setProposal(null), Boolean(proposal));
  const taskDraftRef = useDialog<HTMLDivElement>(() => setTaskDraft(null), Boolean(taskDraft));

  return (
    <div className="meeting-panel">
      <div className="panel-header">
        <div>
          <div className="panel-kicker">
            <span className={`category-chip ${detail.event.category.toLowerCase()}`}>
              {detail.event.category}
            </span>
            <span>Live data</span>
          </div>
          <h2 ref={headingRef} tabIndex={-1}>
            {detail.event.title}
          </h2>
          <p>
            <Clock3 size={15} aria-hidden="true" /> {formatTime(detail.event.start)}–
            {formatTime(detail.event.end)}
            {detail.event.location ? ` · ${detail.event.location}` : ""}
          </p>
          <p>
            <Users size={15} aria-hidden="true" />
            <span className={`attendee-summary ${attendeesExpanded ? "expanded" : ""}`}>
              {attendeesUnknown
                ? "Participants could not be read"
                : shownAttendees.join(", ") || "No participants listed"}
            </span>
            {remainingAttendees > 0 && (
              <button
                className="attendee-toggle"
                onClick={() => setAttendeesExpanded((value) => !value)}
                aria-expanded={attendeesExpanded}
              >
                {attendeesExpanded ? "Show less" : `+${remainingAttendees} more`}
              </button>
            )}
          </p>
          {participantWarning && (
            <p className="attendee-warning" role="note">
              <AlertTriangle size={13} aria-hidden="true" />
              {attendeesUnknown
                ? "Current participant details are unavailable. Check the original invitation before relying on the briefing's participant details."
                : "This participant list may be incomplete, so the briefing may be missing people."}
            </p>
          )}
          {detail.event.source_url && (
            <p className="event-source-link">
              <ExternalLink size={14} aria-hidden="true" />
              <a href={detail.event.source_url} target="_blank" rel="noreferrer noopener">
                Open in the source calendar
              </a>
            </p>
          )}
          {warnings.length > 0 && (
            <ul className="metadata-warnings" aria-label="Calendar metadata warnings">
              {warnings.map((warning) => (
                <li key={warning}>
                  <AlertTriangle size={13} aria-hidden="true" /> {warning}
                </li>
              ))}
            </ul>
          )}
        </div>
        <button className="icon-button" onClick={onClose} aria-label="Close meeting details">
          <X size={20} />
        </button>
      </div>

      {status === "analyzing" && (
        <div className="status-strip analyzing" role="status">
          <LoaderCircle className="spin" size={15} aria-hidden="true" />
          <span>
            Analysis is still running. The briefing below is the last completed version and may be
            incomplete.
          </span>
        </div>
      )}
      {status === "error" && (
        <div className="status-strip error" role="alert">
          <AlertTriangle size={15} aria-hidden="true" />
          <span>
            The last analysis failed. Showing the previous briefing — update it before you rely on
            it.
          </span>
        </div>
      )}

      <div className="panel-actions">
        <button className="primary-action" onClick={refresh} disabled={refreshing}>
          {refreshing ? (
            <LoaderCircle className="spin" size={16} aria-hidden="true" />
          ) : (
            <RefreshCw size={16} aria-hidden="true" />
          )}
          {refreshing ? "Checking sources…" : "Update briefing"}
        </button>
        <span>
          Version {b.version} ·{" "}
          {new Date(b.created_at).toLocaleString([], {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          })}
        </span>
      </div>

      {toast && (
        <div className={`toast ${toast.tone}`} role={toast.tone === "bad" ? "alert" : "status"}>
          {toast.tone === "bad" ? <AlertTriangle size={15} /> : <Check size={15} />} {toast.text}
        </div>
      )}

      <div className="panel-tabs" role="tablist" aria-label="Meeting information">
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            id={`${panelId}-tab-${id}`}
            role="tab"
            aria-selected={tab === id}
            aria-controls={`${panelId}-panel-${id}`}
            tabIndex={tab === id ? 0 : -1}
            className={tab === id ? "active" : ""}
            onClick={() => setTab(id)}
            onKeyDown={(event) => {
              const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
              if (!step) return;
              event.preventDefault();
              const index = TABS.findIndex((item) => item.id === tab);
              const next = TABS[(index + step + TABS.length) % TABS.length];
              setTab(next.id);
              document.getElementById(`${panelId}-tab-${next.id}`)?.focus();
            }}
          >
            {label}
          </button>
        ))}
      </div>

      <div className="panel-scroll">
        {tab === "briefing" && (
          <div role="tabpanel" id={`${panelId}-panel-briefing`} aria-labelledby={`${panelId}-tab-briefing`}>
            <section className="hero-brief">
              <div className="section-title">
                <Sparkles size={17} aria-hidden="true" />
                <h3>What matters</h3>
                <EvidenceBadge type="synthesis" />
              </div>
              <p>{b.summary}</p>
              <ClaimMeta claim={b.summary} briefing={b} />
              <div className="brief-meta">
                <div>
                  <Target size={16} aria-hidden="true" />
                  <span>
                    <small>Objective</small>
                    {b.objective}
                    <ClaimMeta claim={b.objective} briefing={b} />
                  </span>
                </div>
                <div>
                  <History size={16} aria-hidden="true" />
                  <span>
                    <small>Why now</small>
                    {b.why_now}
                    <ClaimMeta claim={b.why_now} briefing={b} />
                  </span>
                </div>
              </div>
            </section>

            <section>
              <div className="section-title">
                <Users size={17} aria-hidden="true" />
                <h3>Your likely role</h3>
                <EvidenceBadge type="inference" />
              </div>
              <div className="role-card">
                <div
                  className="confidence-ring"
                  style={{ "--confidence": `${b.role.confidence * 360}deg` } as React.CSSProperties}
                  aria-hidden="true"
                >
                  <span>{Math.round(b.role.confidence * 100)}%</span>
                </div>
                <div>
                  <strong>{b.role.role}</strong>
                  <p>{b.role.explanation}</p>
                  <small>
                    {Math.round(b.role.confidence * 100)}% confidence
                    {roleEvidence ? ` · based on ${roleEvidence}` : " · no linked source"}
                  </small>
                </div>
              </div>
            </section>

            <section className="changes-section">
              <div className="section-title">
                <Sparkles size={17} aria-hidden="true" />
                <h3>New since last briefing</h3>
                <span className="new-count">
                  {b.changes.length} {b.changes.length === 1 ? "change" : "changes"}
                </span>
              </div>
              <BulletList items={b.changes} tone="changes" briefing={b} />
            </section>

            <div className="section-grid">
              <section>
                <div className="section-title">
                  <BookOpenCheck size={17} aria-hidden="true" />
                  <h3>Prepare</h3>
                </div>
                <BulletList items={b.preparation} briefing={b} />
              </section>
              <section>
                <div className="section-title">
                  <Lightbulb size={17} aria-hidden="true" />
                  <h3>Talking points</h3>
                </div>
                <BulletList items={b.talking_points} briefing={b} />
              </section>
            </div>

            <section>
              <div className="section-title">
                <AlertTriangle size={17} aria-hidden="true" />
                <h3>Risks &amp; open questions</h3>
              </div>
              <BulletList items={[...b.risks, ...b.open_questions]} tone="risks" briefing={b} />
            </section>

            <section>
              <div className="section-title">
                <BriefcaseBusiness size={17} aria-hidden="true" />
                <h3>Business context</h3>
                <EvidenceBadge type="inference" />
              </div>
              <dl className="business-grid">
                {Object.entries(b.business_context).map(([key, value]) => (
                  <div key={key}>
                    <dt>{key.replace(/_/g, " ")}</dt>
                    <dd>{value}</dd>
                    <ClaimMeta claim={value} briefing={b} />
                  </div>
                ))}
              </dl>
            </section>

            {b.milestones && b.milestones.length > 0 && (
              <section>
                <div className="section-title">
                  <CalendarCheck2 size={17} aria-hidden="true" />
                  <h3>Future milestones</h3>
                  <span className="source-label">Dataverse / MSX</span>
                </div>
                <div className="milestone-list">
                  {b.milestones.map((milestone) => (
                    <article className="milestone-card" key={milestone.id}>
                      <div className="milestone-head">
                        <div><strong>{milestone.name}</strong><small>{milestone.opportunity || "Likely customer milestone"}</small></div>
                        <span>{Math.round(milestone.confidence * 100)}% confidence</span>
                      </div>
                      <p>{new Date(milestone.due_date).toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" })} · {milestone.status}</p>
                      <p className="milestone-reason">{milestone.association_reason}</p>
                      {(milestone.url || (milestone.source_ids ?? []).length > 0) && (
                        <p className="milestone-provenance">
                          {milestone.url && (
                            <a href={milestone.url} target="_blank" rel="noreferrer noopener">
                              <ExternalLink size={13} aria-hidden="true" /> Open original record
                            </a>
                          )}
                          {(milestone.source_ids ?? []).length > 0 && (
                            <span>
                              From{" "}
                              {(milestone.source_ids ?? [])
                                .map((id) => sourceMap.get(id)?.connector ?? id)
                                .join(", ")}
                            </span>
                          )}
                        </p>
                      )}
                      {milestone.has_user_task ? (
                        <div className="task-state ready"><ListTodo size={14} /> Your task: {milestone.existing_task_subject || "Task exists"}</div>
                      ) : milestone.suggested_task ? (
                        <button className="task-draft-button" onClick={() => setTaskDraft(milestone.suggested_task ?? null)}>
                          <ListTodo size={14} /> Review suggested task
                        </button>
                      ) : (
                        <div className="task-state missing">No owned task found</div>
                      )}
                    </article>
                  ))}
                </div>
              </section>
            )}

            <section>
              <div className="section-title">
                <BriefcaseBusiness size={17} aria-hidden="true" />
                <h3>Cloud consumption</h3>
                <span className="source-label">Power BI / Fabric</span>
              </div>
              <ConsumptionChart values={b.consumption} unit={b.consumption_unit} />
              <div className="chart-caption">
                <span>{b.consumption_period}</span>
                <span>{b.consumption_unit}</span>
              </div>
            </section>

            <button className="feedback-button" onClick={() => setFeedbackOpen(true)}>
              <MessageSquareText size={16} aria-hidden="true" /> Improve this briefing
            </button>
          </div>
        )}

        {tab === "context" && (
          <div role="tabpanel" id={`${panelId}-panel-context`} aria-labelledby={`${panelId}-tab-context`}>
            <section>
              <div className="section-title">
                <MessageSquareText size={17} aria-hidden="true" />
                <h3>Communication synthesis</h3>
              </div>
              <BulletList items={b.communication_context} briefing={b} />
            </section>
            <section>
              <div className="section-title">
                <FileText size={17} aria-hidden="true" />
                <h3>Original invitation</h3>
              </div>
              <p className="invitation">{detail.invitation}</p>
            </section>
            <section>
              <div className="section-title">
                <ShieldCheck size={17} aria-hidden="true" />
                <h3>Traceable sources</h3>
                <span className="source-label">{b.sources.length} sources</span>
              </div>
              <div className="source-list">
                {b.sources.map((source) => (
                  <SourceRow key={source.id} source={source} />
                ))}
              </div>
            </section>
            <section className="warning-card">
              <AlertTriangle size={17} aria-hidden="true" />
              <div>
                <strong>Data quality notes</strong>
                <BulletList items={b.warnings} />
              </div>
            </section>
          </div>
        )}

        {tab === "chat" && (
          <div
            className="chat-view"
            role="tabpanel"
            id={`${panelId}-panel-chat`}
            aria-labelledby={`${panelId}-tab-chat`}
          >
            <div className="chat-intro">
              <span aria-hidden="true">
                <Sparkles size={22} />
              </span>
              <h3>Ask about this meeting</h3>
              <p>Answers use this meeting's briefing and traceable connected sources.</p>
            </div>
            <div className="suggestions">
              {["What should I present?", "What changed since yesterday?", "Give me three customer questions"].map(
                (item) => (
                  <button key={item} onClick={() => setChatInput(item)}>
                    {item}
                  </button>
                ),
              )}
            </div>
            <div className="chat-messages" aria-live="polite" aria-busy={sending}>
              {messages.map((entry, index) => (
                <div className={`message ${entry.role} ${entry.failed ? "failed" : ""}`} key={index}>
                  {entry.failed && <AlertTriangle size={13} aria-hidden="true" />}
                  <RichText text={entry.text} />
                  {entry.sources && entry.sources.length > 0 && (
                    <span className="message-sources">
                      <small>Sources</small>
                      {entry.sources.map((source) => (
                        <span key={source.id} className="chip">
                          {source.connector}: {source.title}
                        </span>
                      ))}
                    </span>
                  )}
                </div>
              ))}
              {sending && (
                <div className="message assistant typing" aria-label="Assistant is replying">
                  <span />
                  <span />
                  <span />
                </div>
              )}
              <div ref={chatEnd} />
            </div>
            <form className="chat-form" onSubmit={sendChat}>
              <textarea
                value={chatInput}
                onChange={(e) => setChatInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void sendChat(e);
                  }
                }}
                placeholder="Ask a follow-up… (Enter to send, Shift+Enter for a new line)"
                aria-label="Chat message"
                rows={2}
              />
              <button aria-label="Send message" disabled={!chatInput.trim() || sending}>
                <ArrowUp size={17} />
              </button>
            </form>
            <small className="chat-disclaimer">
              Verify important information at the cited source.
            </small>
          </div>
        )}
      </div>

      {feedbackOpen && (
        <div className="modal-layer">
          <form
            ref={feedbackRef}
            className="feedback-modal"
            onSubmit={submitFeedback}
            role="dialog"
            aria-modal="true"
            aria-labelledby={`${panelId}-feedback-title`}
          >
            <div className="modal-title">
              <div>
                <h3 id={`${panelId}-feedback-title`}>Improve this briefing</h3>
                <p>Your correction is never applied beyond this meeting without approval.</p>
              </div>
              <button
                type="button"
                className="icon-button"
                onClick={() => setFeedbackOpen(false)}
                aria-label="Close feedback"
              >
                <X size={18} />
              </button>
            </div>
            <textarea
              data-dialog-autofocus
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              placeholder="What is incorrect or missing?"
              aria-label="Feedback"
              rows={4}
            />
            <label htmlFor={`${panelId}-scope`}>Apply feedback to</label>
            <div className="select-wrap">
              <select
                id={`${panelId}-scope`}
                value={scope}
                onChange={(e) => setScope(e.target.value)}
              >
                <option value="meeting">This meeting only</option>
                <option value="preference">Save as my preference</option>
                <option value="skill">Propose a skill change</option>
              </select>
              <ChevronDown size={15} aria-hidden="true" />
            </div>
            {feedbackError && (
              <p className="modal-error" role="alert">
                {feedbackError}
              </p>
            )}
            <button className="primary-action" disabled={!feedback.trim() || savingFeedback}>
              {savingFeedback ? "Saving…" : "Save feedback"}
            </button>
          </form>
        </div>
      )}

      {proposal && (
        <div className="modal-layer">
          <div
            ref={proposalRef}
            className="proposal-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby={`${panelId}-proposal-title`}
          >
            <div className="modal-title">
              <div>
                <span className="proposal-label">Skill change proposal</span>
                <h3 id={`${panelId}-proposal-title`}>{proposal.title}</h3>
                <p>{proposal.reason}</p>
              </div>
              <button
                className="icon-button"
                onClick={() => setProposal(null)}
                aria-label="Close proposal"
              >
                <X size={18} />
              </button>
            </div>
            <div className="diff">
              <div className="removed">− {proposal.old_content}</div>
              {proposal.new_content.split("\n").map((line, index) => (
                <div className="added" key={`${index}-${line}`}>
                  + {line}
                </div>
              ))}
            </div>
            {proposalError && (
              <p className="modal-error" role="alert">
                {proposalError}
              </p>
            )}

            {proposal.status === "pending" ? (
              <div className="proposal-actions">
                <button onClick={() => decide("reject")}>Reject</button>
                <button className="primary-action" onClick={() => decide("approve")}>
                  Record approval
                </button>
              </div>
            ) : (
              <div className={`decision ${proposal.status}`} role="status">
                {proposal.status === "approved" ? (
                  <Check size={17} aria-hidden="true" />
                ) : (
                  <X size={17} aria-hidden="true" />
                )}{" "}
                Decision recorded as {proposal.status}. This is stored for review only — the skill
                used at runtime is unchanged.
              </div>
            )}

          </div>
        </div>
      )}

      {taskDraft && (
        <div className="modal-layer">
          <div ref={taskDraftRef} className="proposal-modal task-draft-modal" role="dialog" aria-modal="true" aria-labelledby={`${panelId}-task-title`}>
            <div className="modal-title">
              <div><span className="proposal-label">Draft only · no Dataverse write</span><h3 id={`${panelId}-task-title`}>{taskDraft.subject}</h3><p>{taskDraft.reason}</p></div>
              <button className="icon-button" onClick={() => setTaskDraft(null)} aria-label="Close task draft"><X size={18} /></button>
            </div>
            <dl className="task-draft-fields">
              <div><dt>Due date</dt><dd>{taskDraft.due_date}</dd></div>
              <div><dt>Duration</dt><dd>{taskDraft.duration_hours} hours</dd></div>
              <div><dt>Activity type</dt><dd>{taskDraft.activity_type}</dd></div>
              <div><dt>Milestone ID</dt><dd><code>{taskDraft.milestone_id}</code></dd></div>
            </dl>
            <div className="diagnostic-note"><AlertTriangle size={17} /><p>Review and correct the customer, opportunity, milestone, due date, and task type before enabling a future create action.</p></div>
            <div className="proposal-actions"><button className="primary-action" onClick={() => setTaskDraft(null)}>Keep as draft</button></div>
          </div>
        </div>
      )}
    </div>
  );
}
