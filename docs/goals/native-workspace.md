# Goal Card: fast native calendar workspace

## OBJECTIVE

Card: NATIVE-01 | Version: 2 | Readiness: Ready for local implementation; live and owner-acceptance gates remain
Mode: repair loop | Decision owner: project owner

Deliver a real Windows executable with Rust-rendered UI and Rust-owned local
services. Preserve recognizable calendar/briefing workflows while adopting
grayscale plus one configurable accent and full mouse/keyboard operation.
Non-goals: a WebView wrapper, web deployment, cosmetic prototype-only delivery,
or reimplementing the proprietary Copilot runtime.

## OUTPUT

New package: `native\` with `Cargo.toml`, `Cargo.lock`, Rust modules,
and native tests. Use native egui/eframe with AccessKit and Glow.
Update related root development/architecture docs only after functionality
exists. Deliver its reproducible release executable and safe migration path.
Leave `src\`, `backend\`, existing databases/cache, and the browser entry point
working until parity and owner acceptance. Store benchmark/UI/evidence records
in the shared run location, not committed private screenshots.

## DONE WHEN

| ID | Pass condition | Verify | Evidence | If it fails | Recheck |
|---|---|---|---|---|---|
| C01 | Shipping product is a real Rust Windows window without browser/WebView/localhost UI; Rust owns state, persistence, scheduling boundaries and rendering; external harness exceptions are explicit | Inspect production dependencies/process tree and run the actual release executable; inspect native/service boundaries and inventory | Release path/version, dependency/features inventory, process tree, approved exception if needed | Repair packaging/boundaries or stop for an unsupported capability; no silent toolkit switch | C01-C04 |
| C02 | Day/work-week/week/month, mini-calendar, previous/next/today, search/filter, every event's ordinary details, briefings and sources work; preserve timezone/DST, overlaps, all-day/multi-day, empty/unknown/failed days, cancellation and selection semantics; existing cache migration loses no recorded events/briefings | Freeze behavior cases from `src\dates.test.ts`, calendar component tests and `e2e\calendar-navigation.spec.ts`; native automated cases plus Computer Use on owned fixtures; copy migration inputs before testing; product live read through ISO-01 | Scenario inventory with results, expected/actual event IDs, migration comparison and redacted real read-back | Repair the specific port/migration defect; never modify original cache to make migration pass | C02-C04 |
| C03 | Release meets every owner-approved threshold in the shared performance contract, including saturated AI and approved whole-process-tree active limits; no blocking IO/model/parsing in UI callbacks | Execute the frozen three-trial action workload and startup/idle trials on the bound machine; inspect monotonic traces and actual frames; compare busy/idle and inspect callback ownership | All trial data, build/input identity, machine profile, resource/process breakdown and exact harness command | Optimize the measured bottleneck, worker/delta boundaries or visible-content layout without hiding events; BLOCKED for unbound limits/tools | C02-C04 |
| C04 | Calendar, details, assistant shell, palette, diagnostics and dialogs pass the task/appearance matrix; owner accepts final build | Mouse and keyboard independently complete C02 tasks; arrows/Enter/Space/Tab/Escape have clear focus behavior, text input keeps normal editing; UIA names/states; light/dark x blue/red/green/yellow at 1366x768 and 1920x1080, Windows 100/150/200% DPI with matching window sizes recorded, CZ/US input; owner review | Task/focus matrix, screenshots, clipping/contrast observations, UIA results, owner approval tied to build | Fix a named contrast/geometry/focus failure; stop awaiting owner review rather than self-approve | C02-C04 |
| C05 | Final executable, source, docs, migration and test evidence reconcile; no coupled regression | Once package exists run `cargo fmt --manifest-path native\Cargo.toml --all -- --check`, `cargo test --manifest-path native\Cargo.toml --locked`, `cargo clippy --manifest-path native\Cargo.toml --locked --all-targets -- -D warnings`, `cargo build --manifest-path native\Cargo.toml --release --locked`; existing browser/backend regression commands while retained | Exact commands/results and all output/check pointers for final build | Repair scoped failures; no skipped checks or rewritten expectations | C05 and affected C01-C04 |

## QUALITY

These Cargo commands are proposed bindings, not an existing test harness.
Toolkit choice alone proves no speed. Preserve the calendar layout/workflow,
not Tomas Commander's file panes. Use system/bundled fonts without runtime
fetches, restrained borders, no ornamental gradients/mascots, short action
labels and consistent secondary shortcut hints. Categories/status use labels
and shapes with grayscale/selected-accent variants, not extra color families.
Body text contrast >= 4.5:1; controls/focus indicators >= 3:1 (C04).
A passing example: an unfinished event opens metadata immediately and leaves
navigation usable. A failing example: disabled details until AI finishes.

## CONTEXT

Required: shared contract/review, current React date/layout models, backend
model/storage semantics, and ISO-01 Rust SDK for live integration. Authorized reference:
`D:\TomasCommander\docs\decisions\desktop-shell.md` and its running Ledger UI.
Recommended toolkit: egui/eframe with accessibility and one measured renderer;
do not copy version pins without approved-feed resolution.
Unresolved acceptance gates: numeric thresholds including child totals,
benchmark harness, migration compatibility decisions and live-read budget.
Bind original input copies and machine conditions before checks.

## CONSTRAINTS

Incorporate the shared contract. Change the approved native package, related
tests/docs and owned fixture copies; never overwrite original work data.
Use bounded workers/channels, revision-tagged results and event-driven repaint.
Do not enumerate, sort/reparse full corpora, fsync, call providers, or wait on a
worker during rendering. AI or missing authentication must not prevent the
cached shell from opening. Any toolkit/bridge exception requires an owner
decision. No new service, installer or startup registration. Verified milestone
commits/pushes are authorized by the owner's execution request.

## STAGES

1. Freeze parity/appearance/performance cases and baseline; build real native
   calendar navigation from local input, not a new mock design gallery.
2. Port storage/domain behavior and isolated live adapter; preserve old entry
   points and verify safe migration.
3. Repair measured/UI defects, run final matrix, and obtain owner acceptance.

## STOP-CAPS

Pilot proposal: four cycles and 240 elapsed minutes per bounded native slice,
reserving 30 minutes for final checks. Freeze slice boundaries before running;
a partial slice is not DONE for this whole card. No paid calls until the owner
binds their count and identity. Stop after three consecutive cycles without
closing a check or improving latency by > 10 ms without a new regression.

DONE requires C01-C05 PASS, final owner acceptance and all shared thresholds.
BLOCKED for missing toolkit/feed authority, performance limits, live access,
measurement tools or human acceptance. CAPPED/CANCELLED preserve the working
slice and exact remaining cases. Resume keeps cumulative counters.
