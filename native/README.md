# Tomlook native

See the root README for build/run instructions and the goal contracts for
acceptance requirements. This package is the production migration target; the
browser prototype remains intact until native parity and live integration pass.

The UI owns only navigation, filters and immutable calendar snapshots. A bounded
command channel feeds a dedicated storage worker; only that worker opens SQLite,
imports existing state, parses payloads and computes per-day overlap layouts.
Worker notifications request repaint on change; no recurring idle UI poll runs.
Meeting detail requests carry a revision so an old response cannot reopen or
overwrite a different meeting. Settings and calendar transactions survive restart.

Native state is separate from the legacy database. Import uses read-only SQLite
for the source and one destination transaction for calendar, briefing versions,
feedback, proposals and old job records. Legacy jobs are preserved as records,
not resumed as native work. Native coverage is deliberately unverified until a
successful live refresh. Source links allow only credential-free HTTP(S).

Cargo source resolution uses the machine's effective Cargo configuration and
the committed lockfile. The Microsoft npm/PyPI/NuGet feeds do not define a Cargo
registry. No Git-source dependencies or build-time Copilot runtime acquisition
are allowed. `.cargo\config.toml` disables the SDK's automatic runtime downloader
for the external-runtime adapter.

## Isolated SDK and assistant pilot

The pinned official Rust `github-copilot-sdk` is an adapter to an installed
Copilot runtime, not a Rust rewrite of that proprietary runtime. It starts only
after **Connect isolated SDK**. The native application uses a separate SDK
executor, two request slots, bounded command channels and coalesced notices and
streaming. Calendar navigation does not wait for a model or briefing.
Ctrl+Space opens the assistant, Ctrl+Enter submits a question from its editor,
and Escape closes it and requests cancellation. Global questions and captured
meeting context are separate; changing a meeting invalidates its old answer.
Draft questions, optional public topics and last answers are retained separately
for up to 32 contexts in memory. Switching context never carries another
context's public-web topic. A changed meeting snapshot invalidates its old
answer; reopening the same snapshot does not cancel an ongoing question.
At the memory limit, a new context is refused visibly instead of evicting
drafts or submitting against the old meeting. Exit clears this state.
Disk conversation retention has not been selected by the owner and is not
enabled. Questions remain independent SDK requests, not a persistent
multi-turn transcript sent to the model. Full claim-to-passage verification and
real-provider acceptance still require live checks.
Opening the panel requests editor focus; closing it returns focus to search.
Question/topic limits are checked as UTF-8 bytes before submission, request
identities cannot collide with preparation, and an unadmitted question
preserves the previous answer. Cancellation reports an uncertain provider
outcome rather than claiming the operation definitely stopped.

**Local drafts** work without connecting AI. Add an editable task note, email
draft or calendar suggestion with a proposed target, title and body. These are
plain local data: no send, schedule, remote draft, CRM record or execution path
exists. Targets remain unverified. Keep at most eight drafts per assistant
context in memory; new questions do not erase them, and Exit clears them.
If the meeting snapshot changes, existing drafts remain available but are
flagged for explicit local review. A refused context blocks draft admission.
Title/target/body limits are 200/512/4,000 UTF-8 bytes; invalid edits are
identified inline. Discard removes only the selected in-memory draft.
Draft editors retain ordinary typing and cursor/navigation keys rather than
triggering calendar view/period/meeting shortcuts. Global panel/search commands
and Escape remain available; calendar keyboard commands still work when a
non-editor control has focus. New or explicitly kept drafts open their editor
and focus the title; switching context clears an outstanding editor-focus request.

Opt into **Suggest local drafts with the next answer** to request a typed
response containing an answer and up to four `task`, `email` or `calendar`
proposals. Strict decoding and validation run on the SDK worker; unknown fields,
action kinds, approval flags, oversized fields and malformed responses fail
visibly rather than becoming an action. Known app credentials are masked and
post-redaction sizes are rechecked. Structured JSON is not streamed into the
panel. Candidate proposals share the answer's context/stale-result guards and
become editable drafts only after **Keep local draft**. This opt-in is scoped to
the current context and does not change ordinary answers or preparation.
Local review is not permission for a future remote write.

Use `connections.example.json` as a schema example, replace the placeholder
runtime/endpoints, and save an app-owned `connections.json` in the `--state`
directory (default `%LOCALAPPDATA%\Tomlook`). It is **not** personal Copilot
configuration. Optional connector credential variables must use `TOMLOOK_*`
names. Keep tokens and real private configuration outside the repository.
Do not copy a personal profile, token store or `.mcp.json` into this profile.
An empty `servers` object can check Copilot identity without connectors.

The runtime child receives `COPILOT_HOME=<state>\copilot` and
`<state>\workspace` as its working directory. Ambient GitHub token variables
and default SDK connection settings are removed for that child; the parent
environment is unchanged. Empty client mode, disabled configuration discovery,
disabled remote sessions/export, exact read-only MCP allowlists, hooks and
fail-closed permissions prevent ambient personal tools from being adopted.
Public web search is disabled unless a user supplies a public topic, which must
be used verbatim; additional private location/context arguments are rejected.
Connector OAuth storage is in-memory. Registration is not proof of live access.

Missing configuration or isolated authentication produces an explicit error;
there is no fallback to the personal account. Isolated sign-in requires owner
approval and credential entry by the owner. No sign-in or credential migration
has been performed by the executor. `--demo` and `--stress` never start the
SDK or connectors, even if Connect is clicked.

`copilot_credential_env` is optional and defaults to `null`. An owner may instead
name an explicitly supplied `TOMLOOK_COPILOT_TOKEN` environment variable in
app-owned `connections.json`; never put the token value in that file. A missing
or invalid configured credential fails before runtime launch, without fallback.
The token goes through the SDK's explicit credential option, not CLI arguments;
only its digest enters preparation identity. It is not copied from personal
state. `use_logged_in_user=false` blocks automatic `gh` login; Empty mode also
disables the shared system keychain in favor of the app-scoped profile.
Both boundaries remain enabled for explicit tokens. This implements an
authentication path, not evidence of a signed-in product or live integration.

For a no-model, no-connector handshake/session probe from this package directory:

```powershell
cargo run --locked --bin sdk-probe -- D:\TomlookProbe C:\path\to\installed\copilot.exe
```

Use a new app-owned directory, not your regular `.copilot` directory. The probe
prints authentication status and its session ID, then detaches and stops.
The local pilot with CLI 1.0.93-1 created its ID only beneath the isolated probe
profile, not the matching personal session path. This does **not** establish
authenticated calendar/briefing/chat isolation after restart, regular App
visibility, or live permission-request compatibility.

Completed answers now carry a memory-only snapshot from the SDK's typed
post-tool success/failure hooks. The collapsed **SDK tool evidence** inspector
shows the tool, host-observed time, result/failure, response excerpt and original
structured links. These are observed tool results, not model-produced source
objects or independently verified claims. Empty capture explicitly says that
retrieved-source support is not established. Tool arguments are not retained.
Each request retains at most 16 results, 4,000 UTF-8 bytes per excerpt and four
safe links per result; truncation and omitted results are visible. Bounded JSON
projection and link extraction run on the SDK worker, not the UI thread.
Answer and evidence travel as one context-bound object; cancellation, changed
meeting snapshots and stale request IDs invalidate both together.
Configured SDK/connector credentials are masked in output, bounded excerpts and
streamed prefixes across chunk boundaries. A prompt containing a known app
credential is refused before session creation. Credential-bearing link queries
and unsafe URL schemes are not clickable. This is not a promise to detect every
secret in arbitrary workplace content or in opaque runtime logs.

Native live calendar retrieval, persistent conversation history,
real-provider proposal checks, claim-to-passage verification and full assistant acceptance are still
pending. No live provider/source acceptance is inferred from fixture coverage.
The web prototype remains the live-data reference.

## Durable preparation and Windows tray pilot

Preparation is disabled by default. Enable it explicitly in Settings only after
setting up the isolated identity and verified calendar data. The rolling horizon
is seven 24-hour days, including ongoing meetings, with 64 pending jobs, two
execution slots and at most one background preparation. Overflow remains durably
deferred. The latest opened eligible unfinished meeting takes foreground
precedence; unknown calendar coverage never becomes permission to prepare.

The SQLite worker owns the versioned queue ledger, admission and result
validation. Input/analysis fingerprints reject obsolete completions. Publishing
a validated result and its completed ledger is one transaction with exact
payload read-back. Model-provided citations are explicitly **not independently
verified**. Previous briefings survive failed or cancelled attempts.
The connected harness freezes its model, connector endpoints, tool policy and
resolved credential headers. A deterministic digest also includes prompt policy
and runtime path/size/modification identity; only the digest enters the ledger.
Changing configuration takes effect after restart and a new isolated connection.
That connection invalidates old completions and rejects old in-flight results
before new dispatch, including when the window is hidden. Failed/interrupted
outcomes still require explicit retry; a configuration change cannot replay them.
Legacy ledgers remain readable and are upgraded on connection. A ready SDK
without its configuration identity cannot admit preparation. No credentials or
resolved connector headers are written to the queue or briefings.
Minute deadlines renew eligibility; metadata requests do not rescan all event
fingerprints. Completed preparations reach storage even when no UI notices are
consumed. Storage notices and SDK connection/focused-answer notices are bounded
and coalesced, so a hidden window cannot block a worker on notification delivery.

Closing/minimizing hides the window when its tray was created successfully.
Left-click the T tray icon or choose **Open Tomlook** to restore. The tray also
has pause/resume and **Exit Tomlook (stop AI)**. Settings and Commands expose
explicit Exit, with Ctrl+Shift+Q as its shortcut. Exit uses a priority SDK stop
signal, waits outside UI callbacks, then drains ready preparation results and
stops storage through an independent stop flag. Final persistence failures are
reported by the shutdown acknowledgement, not hidden in unconsumed UI notices.
Unavailable tray controls are disabled rather than silently doing nothing.
Windows exclusive profile ownership prevents a second instance
from starting another scheduler for the same state.

Failures stop automatic preparation and survive restart. Failed/interrupted
jobs require explicit retry after inspecting the saved evidence.
**Resume other queued jobs** acknowledges the global stop without replaying
failed/interrupted operations. Lost executor outcomes become interrupted, not
success. Restart never silently replays an interrupted provider operation.
Synthetic modes still make zero provider calls even when preparation is enabled.

This is an engineering pilot. Authenticated hidden-window completion, actual
tray restoration, owned-child exit under live load and the full performance
matrix still require their recorded product observations.

Acceptance still requires the full cold/warm/interaction/DPI/input matrix,
saturated live AI comparison, actual isolation/tray observations and owner
approval. A small test corpus or one idle measurement is not that acceptance.
