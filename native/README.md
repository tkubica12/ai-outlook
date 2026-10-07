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
before that adapter is added.

Acceptance still requires the full cold/warm/interaction/DPI/input matrix,
saturated live AI comparison, actual isolation/tray observations and owner
approval. A small test corpus or one idle measurement is not that acceptance.
