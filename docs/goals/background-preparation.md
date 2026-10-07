# Goal Card: future-first tray preparation

## OBJECTIVE

Card: QUEUE-01 | Version: 2 | Readiness: Ready for local implementation; live/lifecycle acceptance remains
Mode: repair loop | Decision owner: project owner

Keep preparing future meetings while the app is in the tray. Opening an
unfinished meeting promotes its existing job ahead of background work, while
calendar navigation stays fast. Deliver a bounded Rust scheduler and explicit
lifecycle rather than more semaphore waiters. Non-goals: past-meeting backfill,
Windows-login auto-start, cloud scheduling, or remote mutations.

## OUTPUT

Update the approved `native\` worker/scheduler, persistence, runtime adapter,
tray/lifecycle surfaces and their tests/docs. Exact module paths bind after
NATIVE-01 establishes the package. Keep existing caches and finished briefings;
update only app-owned state with explicit schema/version handling.
Use shared evidence/status records. Partial scheduler work is not a complete
tray-resident product.

## DONE WHEN

| ID | Pass condition | Verify | Evidence | If it fails | Recheck |
|---|---|---|---|---|---|
| C01 | A bounded durable queue prepares only eligible future/ongoing meetings in the approved horizon, nearest first; past browsing triggers no auto-analysis; refreshes invalidate by input/analysis revision; one live job per identity; pagination/unknown days remain honest | Freeze clock/timezone and run normal/burst fixtures including elapsed meetings, cancellations, changed meetings, full queue and successful empty days; inspect persisted state and dispatch trace; bind ongoing-event rule before running | Input inventory, expected/actual ordering, peak queue/process counts, cache/job read-back | Repair eligibility, deduplication or backpressure; keep previous briefing on failure | C01-C04 |
| C02 | Clicking an unfinished event opens metadata within the approved local response limit and promotes its existing job; next eligible dispatch precedes background work; stale results never overwrite changed/cancelled context | Fill worker slots and queue with controlled slow jobs, click another event, reopen it repeatedly, change/cancel it, then free a worker; verify monotonic timestamps and zero duplicate execution; confirm with bounded actual product calls | Admission/start trace, identities, UI state and actual-provider observations | Repair priority/reserved capacity/revision guards; never create a second job or replay an unknown provider operation | C01-C04 |
| C03 | Minimize/close-to-tray keeps approved work running without an open calendar view; restore shows actual state; explicit Exit stops owned children; pause/offline/auth failure are visible and bounded; restart handles interrupted work without retry storms | Computer Use through actual tray/window controls; persisted read-back across owned process restart; deny/network-delay cases; user-requested Exit process-tree check; observe running job completion while UI hidden | Lifecycle trace, tray/UI states, recovery ledger, owned child exits and retry counts | Fix process ownership/recovery or stop for an unavailable tray/accessibility mechanism | C01-C04 |
| C04 | Saturated preparation meets shared latency/resource limits and approved concurrency/horizon caps; queued/running never displays success; terminal success requires saved briefing read-back | Frozen performance workload with UI visible/hidden; automated job-state transition tests including queued/running/failed/cancelled/completed; actual saved version comparison | Busy/idle trial data, terminal-state matrix and saved-version evidence | Move blocking persistence/parsing off UI path or fix state handling; preserve thresholds | C01-C04 |
| C05 | Final scheduler implementation, native regressions and evidence reconcile | Bound native tests for C01-C04, run NATIVE-01's locked test/lint/build commands; review saved queue/schema/docs and final build | Exact commands, final inventory, check IDs and evidence pointers | Repair scoped regressions | C05 and affected C01-C04 |

## QUALITY

Future priority is a confirmed owner requirement. "Jump the queue" means
dispatch precedence, not an invented promise of immediate model completion.
Proposal: two execution slots, at most one occupied by background work so
foreground demand has capacity. Use a lazily started reusable SDK runtime, not per-job CLI commands.
Preserve provider/tool rate limits and allow
background fairness when no interactive demand remains. A completed obsolete
response may be retained as historical evidence but not published as current.
Cancellation is not proof the provider charged nothing.

## CONTEXT

Required: shared contract/review, ISO-01 and the native lifecycle boundary.
Prior art: `backend\app.py` job ledger/admission and
`backend\connectors.py` date queue, cache coverage/retry handling;
`src\components\MeetingPanel.tsx` queued-success defect.
Proposed bindings: rolling seven-day horizon, 64 pending items, two total
execution slots, stable key of event identity + input fingerprint + analysis
profile. Bind timezone, rolling-window renewal, ongoing-event eligibility,
overflow/coalescing, foreground fairness, retry/rate/energy policy and actual
provider-call caps before running. No silent defaults for material policies.

## CONSTRAINTS

Incorporate shared permissions/evidence. Write only approved native
scheduler/tray surfaces, related tests/docs and app-owned job/cache state.
Do not mutate mail/calendar/CRM, register an OS service or auto-start, execute
past backfill, retry unknown-outcome operations, or let UI painting enqueue
duplicate work. Bound active processes, pending work, payloads and notification
delivery. Read back state before resuming; counters survive resume.
Any intentional close-to-tray behavior must be clear to the owner, with an
accessible explicit Exit.

## STAGES

1. Freeze policies/clock/scenarios; assess current persisted work.
2. Implement durable priority/admission and tray lifecycle, then isolate workers.
3. Check promotion, failure/restart and resource behavior; repair failed checks.

## STOP-CAPS

Pilot proposal: three cycles and 150 elapsed minutes per run, reserve 25 minutes
for verification. Bind live model/MCP operation counts before any real calls;
fixtures cannot replace mandatory integration evidence. No-progress means no
closed check, no reduction in dispatch violations and no meaningful latency
improvement without new failures; stop after three such cycles or earlier cap.

DONE requires C01-C05 PASS on final native artifacts. BLOCKED means missing
policy/authority, live adapter, tray observation or numeric budgets.
CAPPED/CANCELLED save the queue safely, partial changes, check states,
cumulative limits and precise next gap. Never treat a stopped worker as DONE.
