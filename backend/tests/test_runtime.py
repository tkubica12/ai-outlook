import asyncio
import json
import subprocess
from datetime import datetime, timedelta, timezone
from unittest.mock import AsyncMock

import pytest

from backend import cli_process, copilot_runtime
from backend.copilot_runtime import CopilotCliRuntime
from backend.mcp import McpError
from backend.models import CalendarEvent, ChatTurn, SourceReference
from backend.connectors import ConnectorRegistry


def event():
    now = datetime.now(timezone.utc)
    return CalendarEvent(
        id="event", title="Review", start=now, end=now + timedelta(hours=1),
        category="Calendar", attendees=[], organizer="Organizer", location="Online",
        status="accepted", briefing_status="not_ready",
    )


def payload():
    return {
        "summary": "A supported summary",
        "sources": [{
            "id": "source", "connector": "Outlook", "title": "Original message",
            "timestamp": datetime.now(timezone.utc).isoformat(), "evidence_type": "fact",
            "url": "https://outlook.office.com/mail/id/example",
        }],
        "claim_sources": {"A supported summary": ["source"]},
        "claim_confidence": {"A supported summary": 0.8},
    }


def runtime(monkeypatch, tmp_path):
    monkeypatch.delenv("OUTLOOK_NEXT_DISABLE_COPILOT_CLI", raising=False)
    monkeypatch.setenv("COPILOT_BRIEFING_WORKDIR", str(tmp_path))
    monkeypatch.setattr(copilot_runtime.shutil, "which", lambda _: "copilot")
    return CopilotCliRuntime()


def test_runtime_exposes_only_read_tools(monkeypatch, tmp_path):
    args = runtime(monkeypatch, tmp_path)._arguments("hello")
    visible = [arg for arg in args if arg.startswith("--available-tools=")]
    assert "--available-tools=dataverse-read_query" in visible
    assert "--available-tools=dataverse-describe" in visible
    assert "--available-tools=powerbi-fabric-ExecuteQuery" in visible
    assert not any("create_record" in arg or "shell" in arg or "SendMessage" in arg for arg in visible)
    assert "--no-remote-export" in args
    assert "--additional-mcp-config" in args


@pytest.mark.asyncio
async def test_chat_uses_same_mcp_config_and_cites_new_sources(monkeypatch, tmp_path):
    runner = runtime(monkeypatch, tmp_path)
    source = payload()["sources"][0] | {"id": "new"}
    response = AsyncMock(return_value=json.dumps({
        "answer": "Grounded answer", "sources": [source], "source_ids": ["source"],
    }))
    monkeypatch.setattr(copilot_runtime, "run_cli", response)
    briefing = runner._validate_payload(payload(), event(), None)
    answer = await runner.chat(event(), briefing, "What changed?")
    args = response.call_args.args[0]
    assert "--additional-mcp-config" in args
    assert "--available-tools=dataverse-read_query" in args
    assert [item.id for item in answer.sources] == ["source", "new"]


@pytest.mark.asyncio
async def test_chat_carries_recent_conversation_with_bounded_context(monkeypatch, tmp_path):
    runner = runtime(monkeypatch, tmp_path)
    response = AsyncMock(return_value='{"answer":"Follow-up"}')
    monkeypatch.setattr(copilot_runtime, "run_cli", response)
    history = [ChatTurn(role="user", text="first question"), ChatTurn(role="assistant", text="first answer")]
    await runner.chat(event(), runner._validate_payload(payload(), event(), None), "Explain that", history)
    prompt = response.call_args.args[0][4]
    assert "first question" in prompt and "first answer" in prompt
    assert "untrusted user/assistant" in prompt


@pytest.mark.asyncio
@pytest.mark.parametrize("response", ["[]", '{"answer":""}', '{"answer":"A","source_ids":"bad"}'])
async def test_chat_rejects_invalid_contract(monkeypatch, tmp_path, response):
    runner = runtime(monkeypatch, tmp_path)
    monkeypatch.setattr(copilot_runtime, "run_cli", AsyncMock(return_value=response))
    with pytest.raises(McpError):
        await runner.chat(event(), runner._validate_payload(payload(), event(), None), "Question")


@pytest.mark.parametrize("url", ["javascript:alert(1)", "data:text/html,hello", "file:///secret", "https://user:password@host/", "https://host/\npath"])
def test_unsafe_links_removed_from_saved_and_new_sources(url):
    source = SourceReference.model_validate(payload()["sources"][0] | {"url": url})
    assert source.url is None


def test_calendar_normalization_preserves_categories_and_warns_on_partial_attendees():
    item = ConnectorRegistry._calendar_event({
        "id": "source-id", "subject": "Review", "start": "2026-09-05T10:00:00Z",
        "end": "2026-09-05T11:00:00Z", "categories": "Customer",
        "attendees": [{"emailAddress": None}, {"emailAddress": {"name": "Person"}}],
        "webUrl": "https://outlook.office.com/calendar/item/source-id",
    })
    assert item.category == "Customer"
    assert item.attendees == ["Person"]
    assert item.metadata_warnings
    assert item.source_url.startswith("https://outlook.office.com/")


def test_invalid_evidence_is_not_presented_as_confident():
    data = payload()
    data["claim_sources"] = {"A supported summary": ["invented"]}
    data["claim_confidence"] = {"A supported summary": 1, "unsupported": 0.99}
    briefing = CopilotCliRuntime._validate_payload(data, event(), None)
    assert briefing.claim_sources["A supported summary"] == []
    assert briefing.claim_confidence == {}
    assert briefing.warnings


def test_empty_briefing_is_not_success():
    with pytest.raises(McpError):
        CopilotCliRuntime._validate_payload({}, event(), None)


@pytest.mark.parametrize("owned,confidence,mismatch", [(None, 0.9, False), (True, 0.9, False), (False, 0.4, False), (False, 0.9, True)])
def test_unsafe_task_suggestions_withheld(owned, confidence, mismatch):
    data = payload()
    due = (datetime.now(timezone.utc) + timedelta(days=10)).date().isoformat()
    data["milestones"] = [{
        "id": "milestone", "name": "Milestone", "due_date": due, "status": "On Track",
        "confidence": confidence, "association_reason": "Supported", "has_user_task": owned,
        "suggested_task": {
            "subject": "Follow up", "due_date": due, "duration_hours": 1,
            "activity_type": "Review", "milestone_id": "other" if mismatch else "milestone",
            "reason": "No owned task", "status": "created",
        },
    }]
    briefing = CopilotCliRuntime._validate_payload(data, event(), None)
    assert briefing.milestones[0].suggested_task is None
    assert briefing.warnings


def test_supported_task_remains_a_review_only_draft():
    data = payload()
    due = (datetime.now(timezone.utc) + timedelta(days=10)).date().isoformat()
    data["milestones"] = [{
        "id": "milestone", "name": "Milestone", "due_date": due, "status": "On Track",
        "confidence": 0.9, "association_reason": "Supported", "has_user_task": False,
        "suggested_task": {
            "subject": "Follow up", "due_date": due, "duration_hours": 1,
            "activity_type": "Review", "milestone_id": "milestone",
            "reason": "No owned task", "status": "created",
        },
    }]
    briefing = CopilotCliRuntime._validate_payload(data, event(), None)
    assert briefing.milestones[0].suggested_task.status == "draft"


def test_legacy_consumption_is_not_mislabeled_as_actual():
    data = payload()
    data["consumption"] = [{"label": "Previous estimate", "value": 100}]
    briefing = CopilotCliRuntime._validate_payload(data, event(), None)
    assert briefing.consumption[0].kind == "unspecified"


def test_non_finite_consumption_fails_contract():
    data = payload()
    data["consumption"] = [{"label": "Period", "value": float("inf")}]
    with pytest.raises(McpError, match="contract"):
        CopilotCliRuntime._validate_payload(data, event(), None)


def test_previous_context_is_bounded():
    data = payload()
    data["summary"] = "x" * 50000
    data["business_context"] = {"large": "y" * 50000}
    data["talking_points"] = ["z" * 5000] * 30
    briefing = CopilotCliRuntime._validate_payload(data, event(), None)
    assert len(CopilotCliRuntime._briefing_prompt(event(), briefing)) < 20000


def test_huge_source_identifiers_cannot_bypass_windows_budget(monkeypatch, tmp_path):
    runner = runtime(monkeypatch, tmp_path)
    data = payload()
    data["sources"] = [
        data["sources"][0] | {"id": "x" * 2000 + str(index), "connector": "y" * 2000}
        for index in range(20)
    ]
    briefing = runner._validate_payload(data, event(), None)
    compact = runner._compact_briefing(briefing)
    assert compact["sources"] == []
    args = runner._arguments(runner._briefing_prompt(event(), briefing) + runner._skill_instructions())
    assert len(subprocess.list2cmdline(args).encode("utf-16-le")) // 2 <= 30000


def test_small_chat_context_keeps_relevant_findings():
    data = payload() | {
        "why_now": "Decision is due",
        "open_questions": ["Question"], "communication_context": ["Previous discussion"],
        "public_context": ["Public update"], "changes": ["Changed timing"],
    }
    briefing = CopilotCliRuntime._validate_payload(data, event(), None)
    compact = CopilotCliRuntime._compact_briefing(briefing)
    for field in ("why_now", "role", "open_questions", "communication_context", "public_context", "changes"):
        assert compact[field] == briefing.model_dump(mode="json")[field]


def test_final_command_budget_surfaces_error_before_spawn(monkeypatch, tmp_path):
    runner = runtime(monkeypatch, tmp_path)
    with pytest.raises(McpError, match="input limit"):
        runner._arguments("x" * 40000)


def test_large_meeting_context_is_explicit_excerpt():
    meeting = event()
    meeting.attendees = ["Person " + str(index) + "x" * 100 for index in range(500)]
    context = CopilotCliRuntime._meeting_context(meeting)
    assert "500 attendees" in context["metadata_note"]
    assert len(json.dumps(context)) < 3500


@pytest.mark.asyncio
async def test_timeout_cleans_process(monkeypatch):
    process = AsyncMock()
    process.communicate.side_effect = TimeoutError
    monkeypatch.setattr(cli_process.asyncio, "create_subprocess_exec", AsyncMock(return_value=process))
    stop = AsyncMock()
    monkeypatch.setattr(cli_process, "_stop_process", stop)
    with pytest.raises(McpError, match="deadline"):
        await cli_process.run_cli(["copilot"], timeout=1, timeout_message="deadline")
    stop.assert_awaited_once_with(process)


@pytest.mark.asyncio
async def test_cancellation_cleans_process(monkeypatch):
    process = AsyncMock()
    process.communicate.side_effect = asyncio.CancelledError
    monkeypatch.setattr(cli_process.asyncio, "create_subprocess_exec", AsyncMock(return_value=process))
    stop = AsyncMock()
    monkeypatch.setattr(cli_process, "_stop_process", stop)
    with pytest.raises(asyncio.CancelledError):
        await cli_process.run_cli(["copilot"], timeout=1, timeout_message="deadline")
    stop.assert_awaited_once_with(process)
