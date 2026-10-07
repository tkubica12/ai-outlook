from __future__ import annotations

import asyncio
import hashlib
import logging
import os
import uuid
from contextlib import asynccontextmanager
from datetime import date, datetime, timezone

from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from dotenv import load_dotenv

from backend.connectors import ConnectorRegistry
from backend.copilot_runtime import ANALYSIS_PROFILE, CopilotCliRuntime
from backend.mcp import McpError
from backend.models import (
    Briefing,
    CalendarEvent,
    ChatRequest,
    ChatResponse,
    ConnectorCapability,
    FeedbackRequest,
    HealthResponse,
    MeetingDetail,
    SetupStatus,
    SourceReference,
    EvidenceType,
    RoleEstimate,
    SkillProposal,
)
from backend.storage import Storage

logging.basicConfig(level=logging.INFO, format="%(message)s")
logger = logging.getLogger("outlook-next")

load_dotenv()
storage = Storage()
registry = ConnectorRegistry()
runtime = CopilotCliRuntime()
jobs: dict[str, dict[str, str]] = {}
_background_tasks: set[asyncio.Task] = set()
_live_events: dict[str, CalendarEvent] = {}
_analysis_concurrency = max(1, min(8, int(os.getenv("OUTLOOK_NEXT_ANALYSIS_CONCURRENCY", "4"))))
_analysis_slots = asyncio.Semaphore(_analysis_concurrency)
_chat_slots = asyncio.Semaphore(2)


@asynccontextmanager
async def lifespan(_: FastAPI):
    storage.recover_jobs()
    try:
        yield
    finally:
        for task in list(_background_tasks):
            task.cancel()
        await asyncio.gather(*_background_tasks, return_exceptions=True)
        await registry.close()


app = FastAPI(
    title="Tomlook API",
    description="Live MCP-backed intelligent calendar API",
    version="0.2.0",
    lifespan=lifespan,
)
app.add_middleware(
    CORSMiddleware,
    # The Vite dev server binds to 127.0.0.1, which is a different origin than
    # localhost for the browser's same-origin checks. Allow both by default.
    allow_origins=os.getenv(
        "CORS_ORIGINS", "http://localhost:5173,http://127.0.0.1:5173"
    ).split(","),
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.middleware("http")
async def add_correlation_id(request: Request, call_next):
    correlation_id = request.headers.get("x-correlation-id", str(uuid.uuid4()))
    started = datetime.now(timezone.utc)
    response = await call_next(request)
    response.headers["x-correlation-id"] = correlation_id
    logger.info(
        {
            "correlation_id": correlation_id,
            "path": request.url.path,
            "status": response.status_code,
            "duration_ms": int((datetime.now(timezone.utc) - started).total_seconds() * 1000),
        }
    )
    return response


@app.get("/api/health", response_model=HealthResponse)
async def health():
    return HealthResponse(
        status="healthy",
        runtime=f"Copilot CLI / {runtime.model}" if runtime.available else "Not configured",
        database="SQLite",
        mode="live",
        active_jobs=sum(
            1 for item in jobs.values() if item["status"] in {"queued", "running"}
        ),
    )


@app.get("/api/calendar", response_model=list[CalendarEvent])
async def calendar(start_date: date | None = None, end_date: date | None = None):
    if (start_date is None) != (end_date is None):
        raise HTTPException(status_code=422, detail="Provide both start_date and end_date.")
    if start_date and end_date and not 0 < (end_date - start_date).days <= 42:
        raise HTTPException(status_code=422, detail="Calendar range must be 1 to 42 days, with an exclusive end date.")
    try:
        items = (
            await registry.list_calendar_events(start_date, end_date)
            if start_date and end_date else await registry.list_calendar_events()
        )
    except McpError as error:
        raise HTTPException(status_code=503, detail=str(error)) from error
    for item in items:
        _live_events[item.id] = item
        stored = storage.latest_briefing(item.id)
        job = _meeting_job(item.id)
        if item.briefing_status == "not_required":
            continue
        if stored:
            item.briefing_status = (
                "analyzing"
                if job and job["status"] in {"queued", "running"}
                else ("error" if job and job["status"] == "failed" else stored.status)
            )
            item.has_changes = item.has_changes or (bool(stored.changes) and stored.version > 1)
        else:
            if job and job["status"] in {"queued", "running"}:
                item.briefing_status = "analyzing"
            elif job and job["status"] == "failed":
                item.briefing_status = "error"
    if os.getenv("OUTLOOK_NEXT_DISABLE_COPILOT_CLI") != "1":
        _enqueue_missing_briefings(items)
    return items


@app.get("/api/calendar-status")
async def calendar_status():
    return registry.calendar_status()


@app.post("/api/calendar/retry")
async def retry_calendar():
    registry.reset_calendar_failures()
    return registry.calendar_status()


def _event(meeting_id: str) -> CalendarEvent:
    match = _live_events.get(meeting_id)
    if not match:
        raise HTTPException(status_code=404, detail="Meeting not found; reload the live calendar")
    return match


def _factual_briefing(event: CalendarEvent) -> Briefing:
    source = SourceReference(
        id=f"calendar-{event.id}",
        connector="Outlook Calendar",
        title=event.title,
        timestamp=datetime.now(timezone.utc),
        evidence_type=EvidenceType.FACT,
        url=event.source_url,
        is_mock=False,
    )
    return Briefing(
        meeting_id=event.id,
        version=1,
        created_at=datetime.now(timezone.utc),
        status="not_ready",
        summary="No AI briefing has been generated yet.",
        objective="Not determined from the available calendar metadata.",
        why_now="Not analyzed.",
        role=RoleEstimate(
            role="Not analyzed",
            confidence=0,
            explanation="No role estimate is available until this meeting has been analyzed.",
            evidence_source_ids=[source.id],
        ),
        preparation=[],
        open_questions=[],
        risks=[],
        decisions=[],
        talking_points=[],
        communication_context=[],
        business_context={},
        consumption=[],
        consumption_unit="",
        consumption_period="",
        public_context=[],
        changes=[],
        sources=[source],
        warnings=["Only live calendar metadata is currently available."],
    )


@app.get("/api/meetings/{meeting_id}", response_model=MeetingDetail)
async def meeting_detail(meeting_id: str):
    event = _event(meeting_id)
    current = storage.latest_briefing(meeting_id) or _factual_briefing(event)
    job = _meeting_job(meeting_id)
    event = event.model_copy(update={
        "briefing_status": (
            "analyzing" if job and job["status"] in {"queued", "running"}
            else "error" if job and job["status"] == "failed"
            else current.status
        ),
    })
    return MeetingDetail(
        event=event,
        invitation="Open the source event in Outlook to view the complete invitation.",
        response_status=event.status,
        briefing=current,
    )


@app.get("/api/meetings/{meeting_id}/briefings", response_model=list[Briefing])
async def briefing_history(meeting_id: str):
    _event(meeting_id)
    return storage.history(meeting_id)


def _fingerprint(event: CalendarEvent) -> str:
    return hashlib.sha256(event.model_dump_json(exclude={
        "briefing_status", "has_changes",
    }).encode("utf-8")).hexdigest()


async def _run_refresh(event: CalendarEvent, job_id: str):
    job = jobs[job_id]
    try:
        async with _analysis_slots:
            job["status"] = "running"
            job["started_at"] = datetime.now(timezone.utc).isoformat()
            storage.save_job(job)
            meeting_id = event.id
            previous = storage.latest_briefing(meeting_id)
            updated = await runtime.generate_briefing(event, previous)
            updated.input_fingerprint = _fingerprint(event)
            storage.save_briefing(updated)
            job["status"] = "completed"
    except asyncio.CancelledError:
        job.update(status="failed", detail="Analysis was interrupted. Retry when ready.")
        raise
    except Exception as error:  # job boundary: preserve the previous briefing, expose failure
        logger.error("refresh job %s failed (%s)", job_id, type(error).__name__)
        job["status"] = "failed"
        job["detail"] = (
            str(error) if isinstance(error, McpError)
            else "The analysis returned an invalid result or could not be saved. The previous briefing is unchanged."
        )
    finally:
        job["completed_at"] = datetime.now(timezone.utc).isoformat()
        storage.save_job(job)


def _meeting_job(meeting_id: str) -> dict[str, str] | None:
    return next(
        (
            item
            for item in reversed(list(jobs.values()))
            if item["meeting_id"] == meeting_id
        ),
        storage.latest_job(meeting_id),
    )


def _start_job(meeting_id: str, source: str) -> dict[str, str]:
    _prune_jobs()
    job_id = str(uuid.uuid4())
    jobs[job_id] = {
        "id": job_id,
        "meeting_id": meeting_id,
        "status": "queued",
        "source": source,
        "created_at": datetime.now(timezone.utc).isoformat(),
    }
    storage.save_job(jobs[job_id])
    task = asyncio.create_task(_run_refresh(_event(meeting_id).model_copy(deep=True), job_id))
    _background_tasks.add(task)
    task.add_done_callback(_background_tasks.discard)
    return jobs[job_id]


def _enqueue_missing_briefings(items: list[CalendarEvent]) -> None:
    if not runtime.available:
        return
    now = datetime.now(timezone.utc)
    candidates = sorted(
        (
            item
            for item in items
            if item.briefing_status != "not_required"
            if item.end.astimezone(timezone.utc) >= now
            if (
                not (stored := storage.latest_briefing(item.id))
                or stored.analysis_profile != ANALYSIS_PROFILE
                or (stored.input_fingerprint and stored.input_fingerprint != _fingerprint(item))
            )
            and not (
                (job := _meeting_job(item.id))
                and job["status"] in {"queued", "running", "failed"}
            )
        ),
        key=lambda item: abs((item.start.astimezone(timezone.utc) - now).total_seconds()),
    )
    for item in candidates:
        _start_job(item.id, "automatic")


def _prune_jobs(keep: int = 100) -> None:
    """Bound the in-memory job log so a long demo session cannot grow without limit."""
    finished = [key for key, item in jobs.items() if item["status"] in {"completed", "failed"}]
    for key in finished[: max(0, len(finished) - keep)]:
        jobs.pop(key, None)


@app.post("/api/meetings/{meeting_id}/refresh", status_code=202)
async def refresh(meeting_id: str):
    event = _event(meeting_id)
    if event.briefing_status == "not_required":
        raise HTTPException(status_code=409, detail="This calendar item does not require a briefing.")
    if not runtime.available:
        raise HTTPException(status_code=503, detail="Copilot CLI is disabled or not installed.")
    running = _meeting_job(meeting_id)
    if running and running["status"] not in {"queued", "running"}:
        running = None
    if running:
        return running
    _prune_jobs()
    return _start_job(meeting_id, "manual")


@app.get("/api/jobs/{job_id}")
async def job_status(job_id: str):
    job = jobs.get(job_id) or storage.get_job(job_id)
    if not job:
        raise HTTPException(status_code=404, detail="Job not found")
    return job


@app.get("/api/analysis-status")
async def analysis_status():
    meeting_states = []
    for event in _live_events.values():
        if event.briefing_status == "not_required":
            meeting_states.append({"meeting_id": event.id, "status": "not_required", "source": None})
            continue
        stored = storage.latest_briefing(event.id)
        job = _meeting_job(event.id)
        status = (
            job["status"]
            if job and job["status"] in {"queued", "running", "failed"}
            else ("ready" if stored else "not_started")
        )
        meeting_states.append(
            {
                "meeting_id": event.id,
                "status": status,
                "source": job.get("source") if job else None,
                "detail": job.get("detail") if job and job["status"] == "failed" else None,
            }
        )
    counts = {
        state: sum(1 for item in meeting_states if item["status"] == state)
        for state in ("ready", "queued", "running", "failed", "not_started")
    }
    total = sum(counts.values())
    finished = counts["ready"] + counts["failed"]
    return {
        "total": total,
        **counts,
        "percent": round((finished / total) * 100) if total else 100,
        "meetings": meeting_states,
        "model": runtime.model,
        "concurrency": _analysis_concurrency,
    }


@app.post("/api/meetings/{meeting_id}/chat", response_model=ChatResponse)
async def chat(meeting_id: str, request: ChatRequest):
    event = _event(meeting_id)
    briefing = storage.latest_briefing(meeting_id)
    if not briefing:
        raise HTTPException(
            status_code=409,
            detail="Prepare the meeting briefing before asking a follow-up question.",
        )
    try:
        async with _chat_slots:
            return await runtime.chat(event, briefing, request.message, request.history)
    except McpError as error:
        raise HTTPException(status_code=503, detail=str(error)) from error


@app.post("/api/meetings/{meeting_id}/feedback")
async def feedback(meeting_id: str, request: FeedbackRequest):
    _event(meeting_id)
    storage.save_feedback(meeting_id, request)
    proposal = None
    if request.scope == "skill":
        proposal = SkillProposal(
            id=str(uuid.uuid4()),
            title="Clarify role evidence requirements",
            reason=request.message,
            status="pending",
            old_content="- Estimate the user's role from available context.",
            new_content=(
                "- Estimate the user's role only when two independent sources support it.\n"
                "- Otherwise label the role as low-confidence and state what evidence is missing."
            ),
            created_at=datetime.now(timezone.utc),
        )
        storage.save_proposal(proposal)
    return {"saved": True, "scope": request.scope, "proposal": proposal}


@app.get("/api/skill-proposals/{proposal_id}", response_model=SkillProposal)
async def get_proposal(proposal_id: str):
    proposal = storage.get_proposal(proposal_id)
    if not proposal:
        raise HTTPException(status_code=404, detail="Proposal not found")
    return proposal


@app.post("/api/skill-proposals/{proposal_id}/{action}", response_model=SkillProposal)
async def decide_proposal(proposal_id: str, action: str):
    if action not in {"approve", "reject"}:
        raise HTTPException(status_code=400, detail="Action must be approve or reject")
    proposal = storage.update_proposal(
        proposal_id, "approved" if action == "approve" else "rejected"
    )
    if not proposal:
        raise HTTPException(status_code=404, detail="Proposal not found")
    return proposal


@app.get("/api/connectors", response_model=list[ConnectorCapability])
async def connector_capabilities():
    return await registry.capabilities()


@app.get("/api/setup", response_model=SetupStatus)
async def setup_status():
    return registry.setup_status()
