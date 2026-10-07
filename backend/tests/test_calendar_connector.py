import asyncio
import json
import shutil
import uuid
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import AsyncMock
from zoneinfo import ZoneInfoNotFoundError

import pytest

from backend import cli_process, connectors
from backend.connectors import ConnectorRegistry
from backend.mcp import McpError


DAY = date(2026, 9, 5)


@pytest.fixture
def cache_path(monkeypatch):
    directory = Path.cwd() / f".calendar-test-{uuid.uuid4().hex}"
    directory.mkdir()
    path = directory / "calendar.json"
    monkeypatch.setenv("OUTLOOK_NEXT_CALENDAR_CACHE", str(path))
    monkeypatch.setenv("OUTLOOK_NEXT_DISABLE_COPILOT_CLI", "1")
    # A missing test stub must fail locally, never fall through to real MCP.
    monkeypatch.setattr(
        ConnectorRegistry, "client",
        lambda *args: pytest.fail("Calendar test attempted a live connector"),
    )
    try:
        yield path
    finally:
        shutil.rmtree(directory)


def row(source_id="source-1", day=DAY, **changes):
    start, _ = ConnectorRegistry._day_bounds(day)
    result = {
        "id": source_id,
        "subject": "Customer review",
        "start": (start + timedelta(hours=10)).isoformat(),
        "end": (start + timedelta(hours=11)).isoformat(),
        "organizer": {"emailAddress": {"name": "Organizer"}},
    }
    result.update(changes)
    return result


def event(source_id="source-1", day=DAY, **changes):
    return ConnectorRegistry._calendar_event(row(source_id, day, **changes))


def expire(registry):
    registry._calendar_coverage = {
        key: datetime.now(timezone.utc) - timedelta(minutes=6)
        for key in registry._calendar_coverage
    }


async def settle(registry):
    if registry._calendar_refresh_task:
        await registry._calendar_refresh_task


@pytest.mark.asyncio
async def test_cold_empty_requests_coalesce_and_coverage_survives_restart(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    entered, release = asyncio.Event(), asyncio.Event()
    calls = []

    async def fetch(day):
        calls.append(day)
        entered.set()
        await release.wait()
        return []

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    first = asyncio.create_task(registry.list_calendar_events(DAY, DAY + timedelta(days=1)))
    await entered.wait()
    second = asyncio.create_task(registry.list_calendar_events(DAY, DAY + timedelta(days=1)))
    await asyncio.sleep(0)
    assert registry.calendar_status()["syncing"]
    release.set()
    assert await first == await second == []
    await settle(registry)
    assert calls == [DAY]
    assert await registry.list_calendar_events(DAY, DAY + timedelta(days=1)) == []
    reloaded = ConnectorRegistry()
    assert await reloaded.list_calendar_events(DAY, DAY + timedelta(days=1)) == []
    assert reloaded.calendar_status()["covered_dates"] == [DAY.isoformat()]
    assert not reloaded.calendar_status()["stale_dates"]


@pytest.mark.asyncio
async def test_empty_shard_removes_deleted_events_only_in_successful_shard(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    tomorrow = DAY + timedelta(days=1)
    registry._apply_calendar_shard(DAY, [event()])
    registry._apply_calendar_shard(tomorrow, [event("keep", tomorrow)])
    expire(registry)

    async def fetch(day):
        if day == tomorrow:
            raise McpError("Authentication failed")
        return []

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    await registry.list_calendar_events(DAY, tomorrow + timedelta(days=1))
    await settle(registry)
    result = await registry.list_calendar_events(DAY, tomorrow + timedelta(days=1))
    assert [item.id for item in result] == [event("keep", tomorrow).id]
    status = registry.calendar_status()
    assert status["stale_dates"] == [tomorrow.isoformat()]
    assert status["failures"][tomorrow.isoformat()] == "Authentication failed"
    assert status["last_synced_at"]
    assert status["failure_details"][tomorrow.isoformat()]["attempts"] == 1
    assert not status["syncing"]


@pytest.mark.asyncio
async def test_retries_are_bounded_and_explicit_reset_allows_retry(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    fetch = AsyncMock(side_effect=McpError("Unavailable"))
    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    for attempt in range(1, 4):
        assert await registry.list_calendar_events(DAY, DAY + timedelta(days=1)) == []
        await settle(registry)
        with pytest.raises(McpError, match="Unavailable"):
            await registry.list_calendar_events(DAY, DAY + timedelta(days=1))
        assert fetch.await_count == attempt
        with pytest.raises(McpError):
            await registry.list_calendar_events(DAY, DAY + timedelta(days=1))
        assert fetch.await_count == attempt
        failure = registry._calendar_failures[DAY.isoformat()]
        if attempt < 3:
            failure["retry_at"] = datetime.min.replace(tzinfo=timezone.utc).isoformat()
    assert failure["attempts"] == 3 and failure["retry_at"] is None
    registry.reset_calendar_failures()
    fetch.side_effect = None
    fetch.return_value = []
    assert await registry.list_calendar_events(DAY, DAY + timedelta(days=1)) == []
    await settle(registry)
    assert fetch.await_count == 4
    assert not registry.calendar_status()["failures"]


@pytest.mark.asyncio
async def test_selected_ranges_and_default_no_args(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    fetch = AsyncMock(return_value=[])
    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    assert await registry.list_calendar_events() == []
    await settle(registry)
    today = datetime.now().astimezone().date()
    assert {call.args[0] for call in fetch.await_args_list} == {
        today + timedelta(days=offset) for offset in range(8)
    }
    far = today + timedelta(days=100)
    assert await registry.list_calendar_events(far, far + timedelta(days=42)) == []
    await settle(registry)
    assert fetch.await_count == 50
    for days in (0, -1, 43):
        with pytest.raises(ValueError, match="42"):
            await registry.list_calendar_events(far, far + timedelta(days=days))
    assert fetch.await_count == 50


@pytest.mark.asyncio
async def test_overlapping_inflight_ranges_fetch_each_day_once(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    entered, release = asyncio.Event(), asyncio.Event()
    calls = []

    async def fetch(day):
        calls.append(day)
        entered.set()
        await release.wait()
        return []

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    first = asyncio.create_task(registry.list_calendar_events(DAY, DAY + timedelta(days=2)))
    await entered.wait()
    second = asyncio.create_task(
        registry.list_calendar_events(DAY + timedelta(days=1), DAY + timedelta(days=3))
    )
    await asyncio.sleep(0)
    release.set()
    await asyncio.gather(first, second)
    await settle(registry)
    assert sorted(calls) == [DAY + timedelta(days=offset) for offset in range(3)]


@pytest.mark.asyncio
async def test_navigation_prioritizes_visible_day_over_queued_month(cache_path, monkeypatch):
    monkeypatch.setenv("OUTLOOK_NEXT_CALENDAR_SYNC_CONCURRENCY", "1")
    registry = ConnectorRegistry()
    entered, release = asyncio.Event(), asyncio.Event()
    calls = []

    async def fetch(day):
        calls.append(day)
        if len(calls) == 1:
            entered.set()
            await release.wait()
        return []

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    await registry.list_calendar_events(DAY, DAY + timedelta(days=30))
    await entered.wait()
    selected = DAY + timedelta(days=20)
    await registry.list_calendar_events(selected, selected + timedelta(days=1))
    release.set()
    await settle(registry)
    assert calls[:2] == [DAY, selected]
    assert len(calls) == len(set(calls)) == 30


@pytest.mark.asyncio
async def test_cold_request_returns_before_transport_finishes(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    entered, release = asyncio.Event(), asyncio.Event()

    async def fetch(day):
        entered.set()
        await release.wait()
        return []

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    assert await registry.list_calendar_events(DAY, DAY + timedelta(days=1)) == []
    assert not entered.is_set()
    status = registry.calendar_status()
    assert status["pending_dates"] == [DAY.isoformat()]
    assert status["requested_dates"] == [DAY.isoformat()]
    await entered.wait()
    assert registry.calendar_syncing
    release.set()
    await settle(registry)
    assert DAY.isoformat() in registry.calendar_status()["covered_dates"]


@pytest.mark.asyncio
async def test_calendar_exposes_completed_shard_before_other_shards_finish(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    first_done, release = asyncio.Event(), asyncio.Event()

    async def fetch(day):
        if day == DAY:
            first_done.set()
            return [event()]
        await release.wait()
        return []

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    assert await registry.list_calendar_events(DAY, DAY + timedelta(days=42)) == []
    assert len(registry.calendar_status()["pending_dates"]) == 42
    await first_done.wait()
    assert registry.calendar_syncing
    result = await registry.list_calendar_events(DAY, DAY + timedelta(days=42))
    assert [item.id for item in result] == [event().id]
    assert registry.calendar_status()["covered_dates"] == [DAY.isoformat()]
    release.set()
    await settle(registry)


def test_source_ids_keep_reschedules_stable_and_identical_meetings_distinct(cache_path):
    first = event()
    same_title_different_source = event("source-2")
    moved = event(day=DAY + timedelta(days=1), subject="New title")
    assert first.id != same_title_different_source.id
    assert first.id == moved.id
    registry = ConnectorRegistry()
    registry._apply_calendar_shard(DAY, [first, same_title_different_source])
    registry._apply_calendar_shard(DAY + timedelta(days=1), [moved])
    by_id = {item.id: item for item in registry._calendar_cache}
    assert len(by_id) == 2
    assert by_id[first.id].title == "New title"
    assert by_id[first.id].has_changes


def test_legacy_cache_identity_migrates_and_alias_survives_restart(cache_path):
    old = event()
    old.id = "meeting-existing-briefing"
    cache_path.write_text(json.dumps([old.model_dump(mode="json")]), encoding="utf-8")
    registry = ConnectorRegistry()
    assert not registry.calendar_status()["covered_dates"]
    assert registry._calendar_cache[0].id == old.id
    registry._apply_calendar_shard(DAY, [event()])
    registry._persist_calendar_cache()
    reloaded = ConnectorRegistry()
    reloaded._apply_calendar_shard(DAY, [event(subject="Changed title")])
    assert len(reloaded._calendar_cache) == 1
    assert reloaded._calendar_cache[0].id == old.id
    assert reloaded._calendar_cache[0].has_changes


def test_legacy_alias_does_not_collapse_identical_source_events(cache_path):
    old = event()
    old.id = "meeting-legacy"
    cache_path.write_text(json.dumps([old.model_dump(mode="json")]), encoding="utf-8")
    registry = ConnectorRegistry()
    registry._apply_calendar_shard(DAY, [event(), event("different-source")])
    assert len(registry._calendar_cache) == 2
    assert len({item.id for item in registry._calendar_cache}) == 2


def test_raw_legacy_cache_remains_readable(cache_path):
    cache_path.write_text(json.dumps([row()]), encoding="utf-8")
    registry = ConnectorRegistry()
    old = registry._calendar_cache[0]
    assert old.id.startswith("meeting-")
    registry._apply_calendar_shard(DAY, [event()])
    assert registry._calendar_cache[0].id == old.id


def naive_legacy(source_id="source-1"):
    legacy = event(source_id)
    legacy.id = "meeting-legacy-" + source_id
    legacy.start = legacy.start.astimezone().replace(tzinfo=None)
    legacy.end = legacy.end.astimezone().replace(tzinfo=None)
    return legacy


def test_mixed_legacy_cache_preserves_aware_rows_and_backs_up_skipped_records(cache_path):
    aware, unknown = event("aware", DAY + timedelta(days=1)), naive_legacy()
    original = json.dumps([
        aware.model_dump(mode="json"), unknown.model_dump(mode="json")
    ]).encode()
    cache_path.write_bytes(original)
    registry = ConnectorRegistry()
    assert [item.id for item in registry._calendar_cache] == [aware.id]
    assert [item.id for item in registry._calendar_legacy_candidates] == [unknown.id]
    assert "Skipped 1" in registry.calendar_status()["cache_warning"]
    assert cache_path.read_bytes() == original
    registry._persist_calendar_cache()
    backup = cache_path.parent / registry.calendar_status()["migration_backup"]
    assert backup.read_bytes() == original
    reloaded = ConnectorRegistry()
    assert [item.id for item in reloaded._calendar_cache] == [aware.id]
    assert reloaded.calendar_status()["cache_warning"]
    assert reloaded._calendar_legacy_candidates[0].start.tzinfo is None
    reloaded._persist_calendar_cache()
    assert len(list(cache_path.parent.glob("calendar.legacy-*.json"))) == 1


def test_migration_backup_failure_prevents_overwriting_original(cache_path, monkeypatch):
    original = json.dumps([naive_legacy().model_dump(mode="json")]).encode()
    cache_path.write_bytes(original)
    registry = ConnectorRegistry()
    monkeypatch.setattr(connectors.os, "replace", lambda *args: (_ for _ in ()).throw(
        OSError("backup unavailable")
    ))
    registry._persist_calendar_cache()
    assert cache_path.read_bytes() == original
    assert registry.calendar_status()["migration_backup"] is None
    assert registry.calendar_status()["cache_error"]
    assert list(cache_path.parent.iterdir()) == [cache_path]


def test_unknown_zone_candidate_cannot_alias_without_original_source_metadata(cache_path):
    unknown = naive_legacy()
    cache_path.write_text(json.dumps([unknown.model_dump(mode="json")]), encoding="utf-8")
    registry = ConnectorRegistry()
    assert registry._calendar_cache == []
    registry._apply_calendar_shard(DAY, [event()])
    verified = registry._calendar_cache[0]
    assert verified.id == event().id
    assert verified.id != unknown.id
    assert verified.start == event().start
    assert verified.start.tzinfo is not None
    assert not registry._calendar_aliases
    assert registry._calendar_legacy_candidates[0].start.tzinfo is None
    registry._persist_calendar_cache()
    reloaded = ConnectorRegistry()
    reloaded._apply_calendar_shard(DAY, [event(subject="Rescheduled subject")])
    assert reloaded._calendar_cache[0].id == event().id


@pytest.mark.parametrize("old_count,new_count", [(1, 2), (2, 1)])
def test_unknown_zone_ambiguous_identity_is_never_aliased(cache_path, old_count, new_count):
    candidates = [naive_legacy(f"old-{index}") for index in range(old_count)]
    cache_path.write_text(json.dumps([
        candidate.model_dump(mode="json") for candidate in candidates
    ]), encoding="utf-8")
    registry = ConnectorRegistry()
    fresh = [event(f"fresh-{index}") for index in range(new_count)]
    registry._apply_calendar_shard(DAY, fresh)
    assert not registry._calendar_aliases
    assert {item.id for item in registry._calendar_cache} == {item.id for item in fresh}


@pytest.mark.asyncio
async def test_validation_failure_never_exposes_private_input(cache_path, monkeypatch, caplog):
    registry = ConnectorRegistry()

    async def fetch(day):
        connectors.CalendarEvent.model_validate({"private": "PRIVATE-CALENDAR-PAYLOAD"})

    monkeypatch.setattr(registry, "_list_calendar_day_http", fetch)
    await registry.list_calendar_events(DAY, DAY + timedelta(days=1))
    await settle(registry)
    assert "ValidationError" in registry.calendar_status()["failures"][DAY.isoformat()]
    assert "PRIVATE-CALENDAR-PAYLOAD" not in json.dumps(registry.calendar_status())
    assert "PRIVATE-CALENDAR-PAYLOAD" not in caplog.text
    assert "PRIVATE-CALENDAR-PAYLOAD" not in cache_path.read_text(encoding="utf-8")


@pytest.mark.asyncio
async def test_http_mcp_error_is_sanitized_before_status_or_logs(cache_path, monkeypatch, caplog):
    registry = ConnectorRegistry()
    client = type("Client", (), {
        "list_tools": AsyncMock(return_value=[{"name": "ListCalendarView"}]),
        "call_tool": AsyncMock(side_effect=McpError("PRIVATE-UPSTREAM-PAYLOAD")),
    })()
    monkeypatch.setattr(registry, "client", lambda name: client)
    await registry.list_calendar_events(DAY, DAY + timedelta(days=1))
    await settle(registry)
    assert "authentication" in registry.calendar_status()["failures"][DAY.isoformat()]
    assert "PRIVATE-UPSTREAM-PAYLOAD" not in caplog.text
    assert "PRIVATE-UPSTREAM-PAYLOAD" not in json.dumps(registry.calendar_status())


@pytest.mark.asyncio
async def test_spanning_event_survives_other_shard_and_moves_with_source_id(cache_path):
    registry = ConnectorRegistry()
    tomorrow = DAY + timedelta(days=1)
    after = DAY + timedelta(days=2)
    start, _ = registry._day_bounds(DAY)
    end, _ = registry._day_bounds(after)
    spanning = event(start=start.isoformat(), end=end.isoformat(), isAllDay=True)
    registry._apply_calendar_shard(DAY, [spanning])
    registry._apply_calendar_shard(tomorrow, [spanning])
    registry._apply_calendar_shard(DAY, [])
    assert await registry.list_calendar_events(DAY, tomorrow) == []
    assert len(await registry.list_calendar_events(tomorrow, after)) == 1
    shortened = event(day=tomorrow)
    registry._apply_calendar_shard(tomorrow, [shortened])
    assert registry._calendar_shards[DAY.isoformat()] == []
    assert len(registry._calendar_cache) == 1


@pytest.mark.asyncio
async def test_returned_events_and_status_are_defensive_copies(cache_path):
    registry = ConnectorRegistry()
    registry._apply_calendar_shard(DAY, [event(attendees=["One person"])])
    result = await registry.list_calendar_events(DAY, DAY + timedelta(days=1))
    result[0].attendees.append("Not real")
    registry._record_calendar_failure(DAY, "failure")
    status = registry.calendar_status()
    status["failures"][DAY.isoformat()] = "tampered"
    status["failure_details"][DAY.isoformat()]["message"] = "tampered"
    assert registry._calendar_cache[0].attendees == ["One person"]
    assert registry.calendar_status()["failures"][DAY.isoformat()] == "failure"
    assert registry.calendar_status()["failure_details"][DAY.isoformat()]["message"] == "failure"


def test_out_of_range_rows_preserve_previous_shard(cache_path):
    registry = ConnectorRegistry()
    registry._apply_calendar_shard(DAY, [event()])
    with pytest.raises(McpError, match="out-of-range"):
        registry._apply_calendar_shard(DAY, [event(day=DAY + timedelta(days=1))])
    assert registry._calendar_cache[0].start == event().start


def test_atomic_cache_failure_keeps_previous_file_and_reports_error(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    registry._apply_calendar_shard(DAY, [event()])
    registry._persist_calendar_cache()
    previous = cache_path.read_bytes()

    def fail(*args):
        raise OSError("disk unavailable")

    monkeypatch.setattr(connectors.os, "replace", fail)
    registry._apply_calendar_shard(DAY, [])
    registry._persist_calendar_cache()
    assert cache_path.read_bytes() == previous
    assert registry.calendar_status()["cache_error"] == "disk unavailable"
    assert list(cache_path.parent.iterdir()) == [cache_path]


@pytest.mark.parametrize("result", [
    {"events": [], "complete": False},
    {"value": [], "@odata.nextLink": "next"},
    {"events": [], "hasMoreResults": True},
    {"events": [], "truncated": True},
    {"content": {"events": [], "hasMoreResults": True}},
    [row()] * 150,
    [{"id": ""}],
    [{"id": "bad", "start": "invalid", "end": "invalid"}],
    [row(isCancelled="false")],
])
def test_incomplete_or_invalid_results_are_not_authoritative(result):
    with pytest.raises(McpError):
        ConnectorRegistry._parse_calendar_result(result)


def test_cancelled_events_are_not_returned():
    assert ConnectorRegistry._parse_calendar_result([row(isCancelled=True)]) == []


def test_utc_timezone_dictionary_never_returns_naive_datetime():
    result = ConnectorRegistry._date({"dateTime": "2026-09-05T10:00:00", "timeZone": "UTC"})
    assert result == datetime(2026, 9, 5, 10, tzinfo=timezone.utc)
    assert ConnectorRegistry._date("2026-09-05T10:00:00+02:00").utcoffset() == timedelta(hours=2)
    with pytest.raises(McpError, match="offset"):
        ConnectorRegistry._date("2026-09-05T10:00:00")


def test_windows_zone_maps_exactly_and_missing_database_is_explicit(monkeypatch):
    names = []

    def zone(name):
        names.append(name)
        if name != "Europe/Berlin":
            raise ZoneInfoNotFoundError(name)
        return timezone(timedelta(hours=2))

    monkeypatch.setattr(connectors, "ZoneInfo", zone)
    value = {"dateTime": "2026-09-05T10:00:00", "timeZone": "W. Europe Standard Time"}
    assert ConnectorRegistry._date(value).utcoffset() == timedelta(hours=2)
    assert names == ["Europe/Berlin"]
    with pytest.raises(McpError, match="Unsupported or unavailable"):
        ConnectorRegistry._date({**value, "timeZone": "Unknown Standard Time"})


@pytest.mark.asyncio
async def test_cli_readonly_scope_and_stop_reaps_process(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    entered = asyncio.Event()
    args = []

    class Process:
        returncode = None
        killed = False
        waited = False

        async def communicate(self):
            entered.set()
            await asyncio.Event().wait()

        def kill(self):
            self.killed = True
            self.returncode = -1

        async def wait(self):
            self.waited = True

    process = Process()

    async def spawn(*arguments, **kwargs):
        args.extend(arguments)
        return process

    async def stop(child):
        child.kill()
        await child.wait()

    monkeypatch.setattr(registry, "_copilot_enabled", lambda: True)
    monkeypatch.setattr(registry, "_copilot_has_server", lambda name: True)
    monkeypatch.setattr(connectors.asyncio, "create_subprocess_exec", spawn)
    monkeypatch.setattr(cli_process, "_stop_process", stop)
    caller = asyncio.create_task(registry.list_calendar_events(DAY, DAY + timedelta(days=1)))
    await entered.wait()
    await registry.stop_calendar_sync()
    assert await caller == []
    assert process.killed and process.waited
    assert not registry.calendar_syncing
    assert registry.calendar_status()["failures"][DAY.isoformat()] == "Calendar sync interrupted"
    assert [arg for arg in args if arg.startswith("--allow-tool")] == [
        "--allow-tool=WorkIQ-Calendar(ListCalendarView)"
    ]
    assert [arg for arg in args if arg.startswith("--available-tools")] == [
        "--available-tools=WorkIQ-Calendar-ListCalendarView"
    ]
    assert "--no-remote" in args
    assert "--no-remote-export" in args
    assert "timeZone=UTC" in args[args.index("-sp") + 1]
    assert "webLink" in args[args.index("-sp") + 1]
    assert "webUrl" not in args[args.index("-sp") + 1]


@pytest.mark.asyncio
async def test_http_refresh_keeps_http_transport_and_requests_utc(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    client = type("Client", (), {
        "list_tools": AsyncMock(return_value=[{"name": "ListCalendarView"}]),
        "call_tool": AsyncMock(return_value={"value": []}),
    })()
    monkeypatch.setattr(registry, "client", lambda name: client)
    assert await registry.list_calendar_events(DAY, DAY + timedelta(days=1)) == []
    await settle(registry)
    arguments = client.call_tool.await_args.args[1]
    assert arguments["timeZone"] == "UTC"
    assert "isCancelled" in arguments["select"]
    assert "webLink" in arguments["select"]
    assert "webUrl" not in arguments["select"]
    assert datetime.fromisoformat(arguments["endDateTime"]).date() == DAY + timedelta(days=1)


@pytest.mark.asyncio
async def test_cli_requires_explicit_complete_json_envelope(cache_path, monkeypatch):
    registry = ConnectorRegistry()
    run = AsyncMock(return_value="[]")
    monkeypatch.setattr(connectors, "run_cli", run)
    with pytest.raises(McpError, match="complete"):
        await registry._list_calendar_day(DAY, None)
    run.return_value = '{"complete": true, "events": []}'
    assert await registry._list_calendar_day(DAY, None) == []


def test_graph_event_link_and_missing_participants():
    parsed = event(webLink="https://outlook.office.com/calendar/item/example", attendees=None)
    assert parsed.source_url == "https://outlook.office.com/calendar/item/example"
    assert parsed.attendees == []
    assert parsed.metadata_warnings


@pytest.fixture
def local_mcp_configs(cache_path, monkeypatch):
    project = cache_path.parent / "project"
    home = cache_path.parent / "home"
    project.mkdir()
    (home / ".copilot").mkdir(parents=True)
    monkeypatch.setattr(connectors, "__file__", str(project / "backend" / "connectors.py"))
    monkeypatch.setattr(Path, "home", lambda: home)
    monkeypatch.delenv("COPILOT_CALENDAR_WORKDIR", raising=False)
    monkeypatch.delenv("COPILOT_BRIEFING_WORKDIR", raising=False)
    for _, url_env, token_env in connectors.SERVERS.values():
        monkeypatch.delenv(url_env, raising=False)
        monkeypatch.delenv(token_env, raising=False)
    return project, home / ".copilot" / "mcp-config.json"


def test_project_and_global_mcp_config_detection(local_mcp_configs):
    project, global_path = local_mcp_configs
    global_path.write_text(json.dumps({"servers": {
        "WorkIQ-Calendar": {"type": "http"},
        "DATAVERSE": {"type": "http"},
    }}), encoding="utf-8")
    (project / ".mcp.json").write_text(json.dumps({"mcpServers": {
        "powerbi-fabric": {"type": "http"},
        "dataverse": {"disabled": True},
        "WebIQ-MCP": {"type": "http"},
    }}), encoding="utf-8")
    assert ConnectorRegistry._copilot_has_server("workiq-calendar")
    assert ConnectorRegistry._copilot_has_server("powerbi-fabric")
    assert ConnectorRegistry._copilot_has_server("WebIQ-MCP")
    assert not ConnectorRegistry._copilot_has_server("dataverse")


@pytest.mark.asyncio
async def test_cli_diagnostics_are_configured_not_authenticated_and_never_probe(
    local_mcp_configs, monkeypatch
):
    project, _ = local_mcp_configs
    (project / ".mcp.json").write_text(json.dumps({"mcpServers": {
        name: {"type": "http"} for name in connectors.COPILOT_SERVER_NAMES.values()
    }}), encoding="utf-8")
    registry = ConnectorRegistry()
    monkeypatch.setattr(registry, "_copilot_enabled", lambda: True)
    setup = registry.setup_status()
    assert setup.ready
    assert setup.missing == []
    assert "Power BI / Fabric via Copilot CLI" in setup.configured_connectors
    assert "Dataverse via Copilot CLI" in setup.configured_connectors
    assert "WebIQ via Copilot CLI" in setup.configured_connectors
    capabilities = await registry.capabilities()
    assert len(capabilities) == len(connectors.SERVERS)
    for capability in capabilities:
        assert capability.status == "configured"
        assert capability.capabilities == []
        assert "not proof of authentication" in capability.detail


def test_direct_calendar_credentials_satisfy_readiness(local_mcp_configs, monkeypatch):
    registry = ConnectorRegistry()
    monkeypatch.setenv("WORKIQ_CALENDAR_MCP_URL", "https://calendar.invalid/mcp")
    monkeypatch.setenv("WORKIQ_ACCESS_TOKEN", "synthetic-test-only")
    setup = registry.setup_status()
    assert setup.ready
    assert "Outlook Calendar" in setup.configured_connectors
    assert "Authenticated Copilot CLI with WorkIQ-Calendar" not in setup.missing


def test_bearer_header_is_constructed_from_synthetic_token(monkeypatch):
    monkeypatch.setenv("CALENDAR_UNIT_TEST_TOKEN", "  synthetic-unit-test-only  ")
    assert connectors._headers("CALENDAR_UNIT_TEST_TOKEN") == {
        "Authorization": "Bearer " + "synthetic-unit-test-only"
    }
    monkeypatch.setenv("WEBIQ_API_KEY", "synthetic-api-key")
    assert connectors._headers("WEBIQ_API_KEY") == {"x-apikey": "synthetic-api-key"}
