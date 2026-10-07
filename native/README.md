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
executor, two request slots, bounded command/notice channels and coalesced
streaming. Calendar navigation does not wait for a model or briefing.
Ctrl+Space opens the assistant, Ctrl+Enter submits a question from its editor,
and Escape closes it and requests cancellation. Global questions and captured
meeting context are separate; changing a meeting invalidates its old answer.

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

Native live calendar retrieval, tray preparation and promotion, persistent
conversation history, structured answer sources/proposals and full assistant
acceptance are still pending. AI output is currently plain text, not verified
source objects. The web prototype remains the live-data reference.

Acceptance still requires the full cold/warm/interaction/DPI/input matrix,
saturated live AI comparison, actual isolation/tray observations and owner
approval. A small test corpus or one idle measurement is not that acceptance.
