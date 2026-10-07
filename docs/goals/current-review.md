# Current situation: Tomlook

Reviewed 2026-10-07; the owner then renamed the product to Tomlook. Scope:
local source, documented CLI support, current UI
with synthetic inputs, and the supplied Tomas Commander design reference.
This is not a live connector certification or a security audit.

## What exists

The app is a React/TypeScript browser UI with a Python/FastAPI backend,
SQLite briefing/job storage, and a JSON calendar cache. It has four calendar
views, mini-calendar navigation, briefing details, meeting-scoped chat,
sources, diagnostics, and light/dark themes.

The active execution path is **Copilot CLI subprocesses**, not SDK sessions.
`github-copilot-sdk` remains in `pyproject.toml`, but `backend\app.py` constructs
`CopilotCliRuntime`. `backend\agent_runtime.py` is not that execution path.
Do not begin by rewriting an assumed SDK integration.

Existing strengths worth preserving: date-sharded cache coverage; explicit
unknown/failed days; successful empty-shard replacement; cancellation-safe
subprocess cleanup; captured meeting snapshots; persistent analysis-job
history; bounded active concurrency; frontend abort/stale-response guards;
read-only tools and source metadata. These are useful contracts to port, not
evidence that every performance or safety edge is solved.

## Findings that change the plan

| Finding and inspected source | Consequence |
|---|---|
| `backend\cli_process.py`: children inherit the parent environment; no explicit app-owned Copilot home | Isolate the child state boundary, not just its working directory |
| `backend\copilot_runtime.py`: briefings/chat launch `-C` plus `--no-remote` and `--no-remote-export` | Those flags do not establish local session isolation |
| `backend\connectors.py`: calendar reads launch a CLI/model per date shard, and registration checks read `Path.home()\.copilot\mcp-config.json` | Calendar reads/probes must use the same isolation contract; plain date retrieval should preferably avoid model inference |
| `backend\app.py` and `backend\storage.py`: SQLite operations and JSON validation run synchronously in async request/job paths; calendar reads perform per-event lookups | Measure and remove event-loop blocking and repeated hot-path work |
| `backend\connectors.py`: cache serialization/fsync/replacement and Windows retry `sleep` happen synchronously | Async declarations do not prevent these stalls |
| `backend\app.py`: one task is created per candidate before semaphore admission; active concurrency is bounded but waiting tasks are not a bounded priority queue | Add bounded admission, persistent job identities, backpressure, promotion, and independent foreground capacity |
| `backend\app.py`: chat has two slots and awaits completion within its HTTP request; briefing prerequisite returns 409 | Existing async chat is not a calendar-independent assistant or a durable cancellable job |
| `src\components\MeetingPanel.tsx`: refresh polls only while status is `running`; a fresh `queued` response can reach the success toast | Preserve honest queued/running/terminal states; add a regression check rather than port this behavior |
| `src\App.tsx`: all major views/panels are statically imported; analysis status polls every three seconds | Defer optional surfaces and use event/delta updates where practical; measure rather than claim the current bundle is the dominant bottleneck |
| `src\styles.css`: external Google Fonts, several independent colors, gradients, generated mascots, rounded/shadowed chrome | Rebuild presentation in grayscale plus one selected accent; avoid runtime font fetches |
| `src\components\CalendarView.tsx`: `not_required` event buttons are disabled, and `App.tsx` refuses their detail open | Ordinary calendar details must not depend on AI eligibility |
| No native Rust application or tray lifecycle in this checkout | This is a product migration, not a small backend substitution |

The installed CLI identifies itself as **1.0.93-1**.
`copilot help environment` explicitly documents `COPILOT_HOME` as the override
for configuration and state, defaulting to `$HOME\.copilot`.
`copilot --help` describes `-C` as a working-directory change and remote-export
flags as GitHub web/mobile controls. This makes an explicit **child-only
`COPILOT_HOME`** the first isolation candidate. It has not yet been validated
against the regular Copilot App's discovery behavior or an authenticated
app-owned profile.

Changing that home may also change authentication and MCP registration.
`_copilot_has_server` currently ignores that override; redirecting the launcher
alone would create a discovery/runtime mismatch. Do not copy the user's whole
personal profile, tokens, extensions, or history as a shortcut.

## UI observation and inspiration

Computer Use observed the current built application in Edge using the
repository's synthetic navigation/briefing fixtures through a temporary
read-only loopback preview. No production cache, database, credentials, live
MCP call, model session, or mail/calendar write was used.

Observed the dense week view, overlapping event cards, disabled New event/
notification/profile controls, prominent always-visible preparation strip,
and both light/dark appearance. A meeting-detail click was interrupted by
active user input; this review does not claim completion of the detail,
keyboard, DPI, or full appearance matrix. Source inspection covers the panel.
The temporary preview is not a working provider integration.

Computer Use also read the running Tomas Commander window and its grayscale/
green-accent Ledger presentation while another session was working on it.
No actions were taken in that app. Its tree exposes contextual rail modes,
a Commands control, theme/accent controls, and action labels with separate
shortcut hints. Its layout was observed with an active operation-error modal;
this is inspiration, not a copied approved Outlook design.

The supplied project's `docs\decisions\desktop-shell.md` selects egui/eframe
with a native Rust window and Rust-rendered controls, not a WebView. Its
documented implementation and measurements are that project's evidence, not
this app's benchmark or proof that a toolkit guarantees speed.

Recommendation: prototype the real calendar slice with the same native
egui/eframe approach, retaining AccessKit, on-demand repaint, virtualized
content where appropriate, bounded workers/channels, and revision-tagged
results. Keep today's calendar/briefing workflow recognizable; do not transplant
the file manager's two-pane file layout.

Rust should own local domain logic, storage, job scheduling, IPC, rendering,
and connector policy. Run the official Copilot harness behind an isolated
adapter/process boundary if required. Do not promise a Rust-only Copilot SDK
that has not been verified. A thin non-Rust protocol bridge is a decision
requiring a concrete capability reason, not the default design.

## Scope and decisions

Confirmed: native Windows/Rust direction; clean light/dark UI with one
configurable accent; mouse and keyboard operation; async/lazy AI; global
assistant with WorkIQ/WebIQ; future-first tray/background preparation; clicked
unfinished meetings jump ahead of background work.

Proposed: egui/eframe; cache-first deterministic calendar retrieval; a compact
right-side assistant/activity surface; seven-day preparation horizon; two
execution slots and a bounded queue; pilot performance thresholds in the shared
contract; read-only answers plus local action proposals in the first assistant
slice. These proposals are not owner approval.

Unresolved before autonomous execution: numeric performance limits including
active child-process totals; exact horizon/resource policy; isolated sign-in
and connector authorization; live-call budgets; toolkit/harness boundary;
whether "get things done" includes remote drafts/sends/calendar changes.
No write-capable MCP workflow is authorized by these draft cards.

## Existing regression baseline

During review, 89 backend tests and 164 frontend tests passed; existing lint
and frontend production build passed. Backend emitted a Starlette/httpx
deprecation warning; frontend tests emitted React `act` warnings. Dependencies
were not changed. These checks do not prove native performance, live access,
isolation, tray behavior, or visual acceptance.

Known commands: `.\.venv\Scripts\python.exe -m pytest -q`, `npm test`,
`npm run lint`, `npm run build`, and configured `npm run test:e2e`.
The full E2E suite was not run for this review.

Reviewed HEAD: `070bde67d7c1413fadf18e73ae7225668e35da11`. The checkout was
already substantially dirty/untracked. Preserve it; HEAD is not a complete
snapshot of the reviewed source.
