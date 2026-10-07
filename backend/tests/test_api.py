from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from backend import app as app_module
from backend.mcp import McpError
from backend.models import CalendarEvent
from backend.storage import Storage
from backend.connectors import ConnectorRegistry


@pytest.fixture
def api(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("OUTLOOK_NEXT_DISABLE_COPILOT_CLI", "1")
    app_module.storage = Storage(tmp_path / "test.db")
    app_module.jobs.clear()
    app_module._live_events.clear()
    with TestClient(app_module.app) as client:
        yield client


def live_event() -> CalendarEvent:
    now = datetime.now(timezone.utc)
    return CalendarEvent(
        id="real-event-id",
        title="Customer review",
        start=now,
        end=now + timedelta(hours=1),
        category="Calendar",
        attendees=["User One"],
        organizer="User Two",
        location="Microsoft Teams",
        status="accepted",
        briefing_status="not_ready",
    )


def test_setup_reports_missing_secrets(api, monkeypatch):
    monkeypatch.setenv("OUTLOOK_NEXT_DISABLE_COPILOT_CLI", "1")
    for name in (
        "WORKIQ_ACCESS_TOKEN",
        "FABRIC_ACCESS_TOKEN",
        "DATAVERSE_ACCESS_TOKEN",
        "WEBIQ_API_KEY",
    ):
        monkeypatch.delenv(name, raising=False)
    payload = api.get("/api/setup").json()
    assert payload["ready"] is False
    assert "WORKIQ_ACCESS_TOKEN" in payload["missing"]


def test_calendar_returns_live_connector_events(api, monkeypatch):
    async def list_events():
        return [live_event()]

    monkeypatch.setattr(app_module.registry, "list_calendar_events", list_events)
    response = api.get("/api/calendar")
    assert response.status_code == 200
    assert response.json()[0]["id"] == "real-event-id"
    assert response.json()[0]["briefing_status"] == "not_ready"


def test_calendar_surfaces_mcp_authentication_failure(api, monkeypatch):
    async def fail():
        raise McpError("Authentication required or access was denied")

    monkeypatch.setattr(app_module.registry, "list_calendar_events", fail)
    response = api.get("/api/calendar")
    assert response.status_code == 503
    assert "Authentication required" in response.json()["detail"]


def test_live_meeting_has_no_fabricated_briefing(api, monkeypatch):
    async def list_events():
        return [live_event()]

    monkeypatch.setattr(app_module.registry, "list_calendar_events", list_events)
    api.get("/api/calendar")
    detail = api.get("/api/meetings/real-event-id").json()
    assert detail["briefing"]["status"] == "not_ready"
    assert detail["briefing"]["role"]["confidence"] == 0
    assert detail["briefing"]["consumption"] == []
    assert detail["briefing"]["sources"][0]["is_mock"] is False


def test_chat_requires_a_prepared_briefing(api, monkeypatch):
    async def list_events():
        return [live_event()]

    monkeypatch.setattr(app_module.registry, "list_calendar_events", list_events)
    api.get("/api/calendar")
    response = api.post("/api/meetings/real-event-id/chat", json={"message": "Prepare me"})
    assert response.status_code == 409
    assert "Prepare the meeting briefing" in response.json()["detail"]


def test_health_declares_live_mode(api):
    payload = api.get("/api/health").json()
    assert payload["mode"] == "live"


def test_correlation_id_is_echoed(api):
    response = api.get("/api/health", headers={"x-correlation-id": "abc-123"})
    assert response.headers["x-correlation-id"] == "abc-123"


def test_oversized_attendee_warning_is_not_split_into_characters():
    event = ConnectorRegistry._calendar_event(
        {
            "id": "event",
            "subject": "Meeting",
            "start": "2026-09-04T10:00:00+02:00",
            "end": "2026-09-04T11:00:00+02:00",
            "attendees": "[source attendees too large for requested compact output]",
        }
    )
    assert event.attendees == []
