import asyncio
import sqlite3
from datetime import datetime, timedelta, timezone
from unittest.mock import AsyncMock

import pytest
from fastapi.testclient import TestClient

from backend import app as module
from backend.copilot_runtime import ANALYSIS_PROFILE
from backend.mcp import McpError
from backend.models import CalendarEvent
from backend.storage import Storage


def event():
    now = datetime.now(timezone.utc)
    return CalendarEvent(
        id="event", title="Review", start=now, end=now + timedelta(hours=1),
        category="Calendar", attendees=[], organizer="Organizer", location="Online",
        status="accepted", briefing_status="not_ready",
    )


@pytest.fixture
def isolated(tmp_path, monkeypatch):
    monkeypatch.setattr(module, "storage", Storage(tmp_path / "jobs.db"))
    monkeypatch.setattr(module, "jobs", {})
    monkeypatch.setattr(module, "_background_tasks", set())
    monkeypatch.setattr(module, "_live_events", {"event": event()})
    monkeypatch.setattr(module, "_analysis_slots", asyncio.Semaphore(1))
    return module


def job(status="queued"):
    return {
        "id": "job", "meeting_id": "event", "status": status, "source": "automatic",
        "created_at": datetime.now(timezone.utc).isoformat(),
    }


def test_pruning_never_deletes_queued_jobs(isolated):
    isolated.jobs.update({str(i): job("queued") for i in range(150)})
    isolated._prune_jobs(keep=1)
    assert len(isolated.jobs) == 150


def test_restart_marks_interrupted_jobs_as_retryable_failure(isolated):
    isolated.storage.save_job(job())
    isolated.storage.recover_jobs()
    isolated.jobs.clear()
    assert isolated._meeting_job("event")["status"] == "failed"
    assert "restart" in isolated._meeting_job("event")["detail"]


def test_database_path_resolved_after_environment_load(tmp_path, monkeypatch):
    path = tmp_path / "configured.db"
    monkeypatch.setenv("OUTLOOK_NEXT_DB", str(path))
    store = Storage()
    assert store.path == path
    store.connection.close()


def test_briefing_history_cannot_silently_overwrite_a_version(isolated):
    briefing = isolated._factual_briefing(event())
    isolated.storage.save_briefing(briefing)
    briefing.summary = "Concurrent overwrite"
    with pytest.raises(sqlite3.IntegrityError):
        isolated.storage.save_briefing(briefing)
    assert isolated.storage.latest_briefing("event").summary != "Concurrent overwrite"


@pytest.mark.asyncio
async def test_failed_job_preserves_previous_and_does_not_retry_on_poll(isolated, monkeypatch):
    previous = isolated._factual_briefing(event())
    isolated.storage.save_briefing(previous)
    monkeypatch.setattr(isolated.runtime, "generate_briefing", AsyncMock(side_effect=McpError("Connection unavailable")))
    isolated.jobs["job"] = job()
    await isolated._run_refresh(event(), "job")
    assert isolated.storage.latest_briefing("event").version == previous.version
    assert isolated.storage.latest_job("event")["status"] == "failed"
    start = AsyncMock()
    monkeypatch.setattr(isolated, "_start_job", start)
    isolated._enqueue_missing_briefings([event()])
    start.assert_not_called()


@pytest.mark.asyncio
async def test_job_uses_snapshot_after_navigation(isolated, monkeypatch):
    previous = isolated._factual_briefing(event())
    monkeypatch.setattr(isolated.runtime, "generate_briefing", AsyncMock(return_value=previous))
    snapshot = event()
    isolated._live_events.clear()
    isolated.jobs["job"] = job()
    await isolated._run_refresh(snapshot, "job")
    assert isolated.jobs["job"]["status"] == "completed"
    assert isolated.storage.latest_briefing("event").input_fingerprint == isolated._fingerprint(snapshot)


@pytest.mark.asyncio
async def test_job_cancellation_persisted(isolated, monkeypatch):
    monkeypatch.setattr(isolated.runtime, "generate_briefing", AsyncMock(side_effect=asyncio.CancelledError))
    isolated.jobs["job"] = job()
    with pytest.raises(asyncio.CancelledError):
        await isolated._run_refresh(event(), "job")
    assert isolated.storage.latest_job("event")["status"] == "failed"


def test_sparse_current_briefing_is_not_recomputed(isolated, monkeypatch):
    previous = isolated._factual_briefing(event())
    previous.analysis_profile = ANALYSIS_PROFILE
    isolated.storage.save_briefing(previous)
    start = AsyncMock()
    monkeypatch.setattr(isolated, "_start_job", start)
    isolated._enqueue_missing_briefings([event()])
    start.assert_not_called()


def test_calendar_rejects_invalid_ranges_without_calling_connector(isolated, monkeypatch):
    fetch = AsyncMock()
    monkeypatch.setattr(isolated.registry, "list_calendar_events", fetch)
    client = TestClient(isolated.app)
    for query in (
        "?start_date=2026-09-01", "?start_date=2026-09-01&end_date=2026-09-01",
        "?start_date=2026-09-01&end_date=2027-01-01",
    ):
        assert client.get("/api/calendar" + query).status_code == 422
    fetch.assert_not_called()


@pytest.mark.asyncio
async def test_workers_respect_concurrency_limit(isolated, monkeypatch):
    monkeypatch.setattr(isolated, "_analysis_slots", asyncio.Semaphore(2))
    running = 0
    peak = 0
    gate = asyncio.Event()
    both_started = asyncio.Event()

    async def generate(snapshot, _previous):
        nonlocal running, peak
        running += 1
        peak = max(peak, running)
        if running == 2:
            both_started.set()
        await gate.wait()
        running -= 1
        return isolated._factual_briefing(snapshot)

    monkeypatch.setattr(isolated.runtime, "generate_briefing", generate)
    tasks = []
    for index in range(5):
        snapshot = event().model_copy(update={"id": f"event-{index}"})
        job_id = f"job-{index}"
        isolated.jobs[job_id] = job() | {"id": job_id, "meeting_id": snapshot.id}
        tasks.append(asyncio.create_task(isolated._run_refresh(snapshot, job_id)))
    await asyncio.wait_for(both_started.wait(), timeout=2)
    assert peak == 2
    assert sum(item["status"] == "queued" for item in isolated.jobs.values()) == 3
    gate.set()
    await asyncio.gather(*tasks)
    assert peak == 2
    assert all(item["status"] == "completed" for item in isolated.jobs.values())
