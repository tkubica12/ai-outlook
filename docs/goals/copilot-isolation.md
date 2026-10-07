# Goal Card: app-owned Copilot state

## OBJECTIVE

Card: ISO-01 | Version: 2 | Readiness: Ready for local implementation; live identity gate remains
Mode: repair loop | Decision owner: project owner

Run every Tomlook Copilot operation without adding sessions to the owner's
regular Copilot App/session store. Preserve authorized authentication and MCP
capabilities. Cover briefings, calendar retrieval, chat, probes, retries, and
restart. Non-goals: deleting old personal sessions, changing the regular App,
or starting the Rust migration.

## OUTPUT

Implement `native\`'s official Rust SDK adapter and configuration/permission
boundary, its tests/configuration examples/docs, and the app-owned profile.
The old Python prototype remains available but is not the native execution path.
Do not copy or overwrite existing profiles. Suggested destination:
`%LOCALAPPDATA%\Tomlook\copilot`; resolve before running.
Store private check evidence/status using `README.md`'s shared run record.
Partial outputs remain explicitly incomplete.

## DONE WHEN

All required checks start NOT_RUN; every check must pass on the final files.

| ID | Pass condition | Verify | Evidence | If it fails | Recheck |
|---|---|---|---|---|---|
| C01 | Every production CLI child receives the same explicit app-owned `COPILOT_HOME`; discovery uses that home; no parent/global environment change | Executor inventories each launcher/probe and records intercepted child environments in unit tests; includes conflicting inherited home, spaces/Czech paths, retry, and restart | Redacted launcher inventory and exact test command/result | Repair the common child boundary or inconsistent probe; no broad HOME/USERPROFILE override | C01-C03 |
| C02 | Real authorized calendar read, briefing, and chat run in the isolated profile; zero new app-owned session IDs appear in personal local state or regular Copilot App after completion and restart/refresh | Snapshot personal session IDs, execute bounded product operations with a unique run tag, read isolated state, then inspect the regular App with Computer Use; identify unrelated concurrent sessions rather than attributing all changes to the app | Timestamped tagged session-ID comparison, isolated state paths, redacted product results, App inspection | Fix remaining leakage; BLOCKED if discovery/auth behavior cannot be verified | C01-C03 |
| C03 | Isolated setup matches real connector access; missing/expired identity yields an actionable error; original personal configuration and sessions remain intact; cancel/shutdown leave no owned child running | Product read-back with approved identity; controlled missing-auth test; before/after personal config fingerprints without content; owned process-tree check | Config integrity comparison, errors, connector read-back, exit/process results | Repair discovery/error/lifecycle handling; stop for owner sign-in; never bulk-copy credentials | C01-C03 |
| C04 | Saved final changes preserve existing calendar/job behavior and accurately document isolation | Run NATIVE-01's locked native checks and retained prototype regressions; review exact changed files and documentation; reconcile C01-C03 against final source | Command/exit result, final input/output inventory and evidence pointers | Repair coupled regressions; preserve unrelated dirty files | C04 and checks affected by repair |

## QUALITY

`COPILOT_HOME` is documented in installed CLI 1.0.93-1; `-C` and disabling
remote export are not substitutes. C02 proves actual local App behavior,
not just an environment assertion. Authentication and MCP access are separate
from registration (C03). Never fabricate live success or expose tokens.

## CONTEXT

Required: shared `README.md`, `current-review.md`, resolved official Rust SDK
APIs, old runtime/connector tests as behavioral reference, and native package.
Reuse a lazily started SDK-managed runtime; never launch one-shot CLI prompts.
Bind the CLI version and profile path at run start. Do not read unrelated
personal transcript contents; C02 needs session identifiers only.
Unresolved: isolated sign-in method and owner-approved identity/profile;
live validation and its caps. Suggested live ceiling: one
calendar read, one briefing, and one chat, with at most one approved retry each.
This is a budget proposal, not permission to call providers.

## CONSTRAINTS

Incorporate the shared execution/evidence/permission contract.
Writes are limited to the files above, related isolation tests, and the
explicit app-owned profile. Sign-in or credential migration requires owner
approval; credential entry remains with the owner. Never redirect this
development agent's own profile, modify the regular App's discovery settings,
delete history, change system environment, or weaken tool allowlists.
Read back unknown outcomes before retrying; resume preserves counters.

## STAGES

1. Freeze launcher inventory and personal/isolated-state observations.
2. Implement one child-environment and discovery boundary; exercise failure paths.
3. Validate real isolated operations and the regular App, then reconcile files.

## STOP-CAPS

Pilot proposal: at most three cycles and 120 elapsed minutes per run, including
waiting; reserve 20 minutes for final verification. Live limits must be approved
and bound before calls. Stop after three cycles without closing a failed check,
or earlier at a cap. Fixing a launcher while breaking access is not progress.

DONE requires C01-C04 PASS with final evidence. BLOCKED means missing identity,
App observation, permission, or a supported isolation mechanism. CAPPED means
the cycle/time/action cap ended an incomplete run. CANCELLED means the owner
stopped it. Save partial changes, check states, exact gap, and unblock action
in the shared run record; no automatic cleanup of personal state.
