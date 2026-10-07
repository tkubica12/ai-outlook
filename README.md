# Tomlook

Tomlook is a local-first intelligent calendar that reads real Microsoft 365 data through
remote MCP servers. It does not seed or fall back to synthetic meetings.

The [goal contracts](docs/goals/README.md) govern the authorized native Rust
migration, isolated official Rust Copilot SDK, future-first tray preparation,
and assistant panel. Execution order is native foundation, SDK isolation,
tray scheduling, then assistant integration. Full acceptance is not yet claimed.
Existing `OUTLOOK_NEXT_*` environment variables and `data/outlook-next.db`
remain unchanged for configuration and stored-data compatibility.

## Native Windows foundation

`native\` is a real Rust-rendered Windows application using egui/eframe,
AccessKit and OpenGL; the UI needs no browser, WebView or Python/Node bridge.
It currently provides day/work-week/week/month calendars, category/search filters,
meeting details and saved briefings, commands, and persisted light/dark appearance
with one of four accents. Disk access, SQLite, cache parsing and calendar layout
indexing run on a dedicated bounded worker, not in UI callbacks.

Build with Rust 1.95+ and the x64 Visual Studio C++/Windows SDK environment:

```powershell
Set-Location .\native
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
.\target\release\tomlook.exe --legacy-root D:\ai-outlook
```

Application state lives in `%LOCALAPPDATA%\Tomlook\tomlook.db`. On first use,
the optional legacy-root cache and briefing/history database are imported
read-only into that state, preserving original files. Unknown coverage is
displayed as **Not synced**, never as proof that a day is free. Invalid/unzoned
legacy events are reported, not displayed with invented timezone information.
`--state <directory>` selects an isolated local profile.

For explicitly synthetic, offline UI review, use `--demo` (200 meetings) or
`--stress` (5,000 meetings), with a separate `--state` directory. These modes
are clearly labelled and never replace production calendar data.
Keyboard: Ctrl+F search, Ctrl+K commands, 1-4 views, T today, Alt+Left/Right
period, Up/Down meeting, Escape close, Tab/Shift+Tab focus, and Ctrl+Shift+Q Exit.

The calendar remains cache-first; native live synchronization is not implemented
yet. A durable native scheduler and Windows tray now retain future preparation
jobs and promote an opened unfinished meeting. Preparation is **disabled by
default**; Settings provides enable/pause and **Exit Tomlook**; meeting details
provide explicit retry. Closing/minimizing hides the calendar when its tray is
available, rather than stopping it. Restore through
the Tomlook tray icon; Exit stops the workers. Each state directory allows one
native instance, preventing duplicate preparation from a second launch.

A lazy official Rust Copilot SDK adapter and native
global/meeting assistant pilot are now available. SDK work, streaming and
cancellation run on a separate executor with bounded channels; opening the
calendar or assistant does not start Copilot. See [native setup](native/README.md)
for the separate identity/configuration boundary and remaining acceptance gates.
The SDK still owns an installed proprietary Copilot runtime child, not a Rust
rewrite of that runtime. Preparation requires isolated authentication and
verified calendar coverage; fixture tests do not establish live readiness.
The retained web prototype below is still the live-data reference during migration.

## Web prototype: connect live data

1. Copy `.env.example` to `.env`.
2. Sign in to GitHub Copilot CLI and authenticate its WorkIQ MCP servers with the Microsoft
   identity that owns the calendar. The application reuses this local CLI configuration; no new
   Entra application registration is required for this development flow.
3. Authenticate the project-scoped Fabric and Dataverse servers as described below.
   The access-token environment variables are optional for the direct HTTP MCP adapter, not
   requirements for the Copilot-managed flow.
4. Configure WebIQ in the CLI MCP host with its server-side `x-apikey` header. Never put an API
   key in a frontend environment variable or commit it.
5. Start the app with `.\scripts\start.ps1`.

The MCP URLs supplied for the tenant are already present in `.env.example`. Secrets belong only in
the ignored `.env` file or, later, a managed secret store.

For business enrichment, authenticate the project-scoped MCP servers once in an interactive
Copilot CLI session:

```text
copilot --additional-mcp-config '@D:\ai-outlook\.mcp.json'
/mcp auth powerbi-fabric
/mcp auth dataverse
```

Complete each browser consent prompt using the same account and working directory as the backend,
then retry failed analysis. Meeting analysis includes the instructions from
the local `azure-consumption`, `msx-meeting-context`, and `task-write-safety` skills. ACR is used
only as evidence; Dataverse access is read-only during analysis. Missing owned milestone tasks are
presented as review-only drafts and are never created automatically.

Open <http://127.0.0.1:5173>. API docs are at <http://127.0.0.1:8000/docs>. Diagnostics distinguish
registered CLI connectors from directly verified HTTP connections. Registration alone does not
prove that a remote service's OAuth consent, model access, or tenant policy is valid.

## Web prototype: runtime and background processing

The current runtime launches **Copilot CLI subprocesses**, not SDK sessions. Briefings and chat
share the same MCP configuration and read-only tool availability. The default model is
`gpt-5.6-terra` with `low` reasoning and four briefing workers (configurable, capped at eight).
Calendar synchronization has its own bounded concurrency; chat has two separate slots.

Day/week/month navigation requests the visible date range. Successfully read days replace their
cached contents, including cancellations and empty days; failed days retain the previous data with
an explicit synchronization warning. A blank, unqueried day is not evidence of a free calendar.

Future meetings without a current briefing are prepared in the background. Refresh jobs are saved
in SQLite; a failed or interrupted job waits for an explicit retry instead of starting again on
every browser poll. The previous briefing remains available. Sparse but valid briefings are not
recomputed just because they contain few citations.

ACR requires a supported customer association **and** current Fabric access. Current partial-month
actuals and predictions are distinguished from completed actuals. Milestone ownership queries must
identify the current Dataverse user; an unknown task lookup is not treated as "no task".
Task proposals are review-only: this version cannot create CRM tasks.

This is a single-user local development application. Do not expose its unauthenticated API to a
network. Azure Container Apps Sandboxes and a production multi-user/OBO design remain out of scope.

## Validate

```powershell
.\.venv\Scripts\python.exe -m pytest
npm test
npm run lint
npm run build
npm run test:e2e
```
