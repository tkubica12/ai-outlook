# Tomlook goal contracts

The owner authorized execution on 2026-10-07: Rust-native throughout, the
official Rust Copilot SDK, and commit/push at significant verified milestones.
Local implementation is authorized. Live authentication and final visual/
physical-keyboard acceptance remain distinct gates; no card is already DONE.
The project owner decides material tradeoffs and final visual acceptance.

## Verdict

**Use a loop** for each implementation card within the bindings below.
Failed isolation, ordering, responsiveness, or interaction checks have small,
testable repairs. A missing login, undecided budget, or awaiting human acceptance
is BLOCKED, not a reason to keep iterating.

| Card | Outcome | Capability prerequisites |
|---|---|---|
| [ISO-01](copilot-isolation.md) | App-owned Copilot state; no personal session noise | Approved isolated sign-in and live-call limits |
| [NATIVE-01](native-workspace.md) | Real Rust Windows calendar with restrained appearance and mouse/keyboard parity | Native toolkit and benchmark contract approval; isolated adapter for live reads |
| [QUEUE-01](background-preparation.md) | Tray-resident, future-first preparation with foreground promotion | ISO-01; native process/lifecycle boundary from NATIVE-01 |
| [ASSIST-01](assistant-panel.md) | Global, contextual WorkIQ/WebIQ assistant without blocking navigation | ISO-01; native UI; QUEUE-01 cancellation/priority contract |

Execution order: NATIVE-01 local foundation, ISO-01 Rust SDK adapter,
QUEUE-01 future preparation/tray, ASSIST-01 global/contextual assistant, then
final cross-card reconciliation. The native foundation must remain useful
without AI credentials, so full live isolation validation is not its prerequisite.
Keep each accepted slice working; do not delete the browser version or caches
before migration and parity are verified.

## What changed and why

| Original intent | Observable contract |
|---|---|
| Isolate SDK sessions | Isolate every actual CLI/SDK child and connector probe; confirm no new app-owned sessions in personal state or the regular Copilot App |
| Absolutely fast Rust app | Release-build latency/resource measurements under frozen data and saturated AI; no network, disk, model, or heavy parsing on the UI thread |
| AI strictly async | Durable bounded queues, promotion, cancellation, revision-tagged results, worker isolation, and explicit queued/running/completed states |
| Absolutely perfect UI | Frozen task/appearance matrix plus the owner's acceptance of the exact final build; no self-awarded visual PASS |
| Easily get things done | Calendar-independent questions, real sources, typed read-only tools, navigation during slow answers, and local action proposals |

Confirmed follow-up: preparation must continue in the tray/background, focus
on future meetings, and prioritize an unprocessed meeting when the user opens
it. It is not on-demand-only.

## Shared execution and evidence contract

Every card incorporates this section. A fresh executor must read this file,
the selected card, and [the current review](current-review.md) before acting.
The owner's execution request authorizes implementation and local tray
scheduling, not credential changes, remote mutations or new paid services.

Run binding: `tomlook-native-20261007`, repository `D:\ai-outlook`, package
`native\`, state `%LOCALAPPDATA%\Tomlook`, Copilot home beneath that state,
egui/eframe with AccessKit and Glow, and official `github-copilot-sdk`.
Resolve only through the effective configured Cargo source; no Git dependencies.
The SDK is an external-runtime adapter, not a promise that the proprietary
runtime is rewritten in Rust. Disable automatic build-time runtime downloads;
use an explicit installed compatible runtime.
Pilot implementation bindings: seven-day forward horizon including ongoing
meetings, 64 pending jobs, two active slots with at most one background job,
and local Windows timezone. Overflow is deferred, never silently discarded.
Automatic retries of failed AI jobs remain off. There are no live validation
calls until the isolated identity is available; this does not block local work.
Use the proposed numeric limits below as frozen pilot targets, not claim that
the owner separately approved them or that unmeasured limits have passed.

Before a run, bind and record the card version, owner-approved unresolved
choices, exact output paths, source snapshot including dirty files, frozen
fixture inventory, machine/build conditions, check procedures, live-call caps,
and cumulative run ID. HEAD alone is not the input version: most current
application files are untracked. Capture a scoped source inventory without
secrets. Existing personal data and unfinished work remain untouched.

Use the execution session's existing artifact directory, exposed by its host.
At run start resolve that directory to an absolute path. Store
`<card-id>-<run-id>-status.json` there. This is a binding rule, not a second
logging platform. Keep private evidence out of repository documentation.
The record contains card/input/build versions, each check ID, result
(NOT_RUN/PASS/FAIL/BLOCKED), time, checker or exact command, observed values,
evidence pointers, repairs, invalidated checks, cumulative resource counts,
next gap, and external operation IDs. Never store tokens or raw private mail.

The executor may add tests implementing these checks; it may not lower their
bar, remove scenarios, disable tests, or change evaluator inputs to pass.
New harness commands must be recorded and inspected before they can be used
as evidence. Proposed commands in a draft do not imply a harness exists.
After repair, recheck the named dependent checks. At final reconciliation
verify every output, required check, source/build identity, permission, and
budget. Human acceptance must name the exact submitted build.

Read/write authority is limited to the approved feature, related tests/docs,
owned synthetic fixtures, and app-owned state. Verified milestone commits and
pushes to the existing repository/branch are now owner-authorized. No broad cleanup,
deployment, cloud services, OS startup registration, personal configuration
changes, bulk credential copying, or mail/calendar/CRM mutations. New packages
must follow the managed-machine feed policy, preserve lockfiles, and resolve
through an approved effective source. Verify Cargo source policy explicitly;
the supplied npm/PyPI/NuGet feeds do not establish a Cargo exception.

Live checks must use the actual product adapter and authorized identity.
Calls made by this chat's WorkIQ tools, canned answers, and registered connector
names cannot prove product integration. Fixtures prove local logic and failure
handling only. Missing authorized integration is BLOCKED.

No unknown-outcome external action may be retried automatically. Read back
first; use durable operation identities where a future approved write needs
deduplication. Stored approval is never inferred from model text.

## Proposed performance contract

These are **frozen pilot targets, not measured results or final owner acceptance**.
They guide this authorized implementation. Missing measurement/approval remains
BLOCKED for full card acceptance; it does not forbid constructing the product.

| Measure | Proposed finish line |
|---|---|
| Startup to usable cached calendar | Every one of 10 process-cold launches <= 2 s; every one of 10 subsequent warm launches <= 1 s |
| Local action to rendered response | p95 <= 50 ms and maximum <= 100 ms for 1,000 scripted interactions per trial, in three trials |
| AI interference | Busy-versus-idle p95 increase <= 10 ms; both meet absolute limits |
| Idle native app footprint | <= 150 MiB private bytes; <= 1% of one logical CPU core averaged over 60 s after settling |
| Tray-only idle | Same resource limits; no unconditional continuous repaint or busy polling |
| Foreground admission | Open/request acknowledged <= 50 ms; an eligible promoted job starts within 100 ms of worker availability, not model completion |

Measure a release build on the owner's Windows laptop with its CPU/RAM,
power mode, display/DPI, graphics backend, runtime versions, and background
activity recorded. Process-cold means the application process is absent;
it is not a claim of cold OS disk caches. Use monotonic timestamps for input
dispatch, frame presentation, queue admission/start, and startup landmarks;
record clock alignment for any cross-process measurements.

Freeze two deterministic corpora: a normal 42-day calendar with 200 events and
a stress corpus of 5,000 events across the same 42 days, including overlaps,
all-day/multi-day events, long/Czech text, and DST boundaries. Record the
generator/input version and expected event inventory. Visible results must
remain complete and reachable; hiding meetings is not a performance repair.
The interaction sequence cycles day/work-week/week/month, previous/next/today,
search/filter, event open/close, keyboard traversal, and assistant open/close.
Each action gets a task-specific observable end state, not just a paint counter.

Run with AI disabled, then with configured worker slots full and deliberately
slow provider responses while progress and completion arrive. Include large
result parsing, persistence, cancellation, and burst notifications. Use test
doubles to reproduce timing; also verify responsiveness during bounded real
provider calls. Report every trial, not the best one.

The native idle limit excludes external CLI processes only when none is alive.
When AI is running, record the **whole product process tree**, including CLI/MCP
children, private bytes, CPU, lifetime, and process count separately from the
native app. Total active-AI memory/CPU ceilings remain unresolved and must be
approved; do not quietly exclude children from the total.

## Example repair branches

- ISO-01 C01 fails because calendar reads still inherit personal Copilot state:
  use the same explicit child environment boundary as briefings/chat, then
  rerun C01-C03. Record the unchanged personal session inventory.
- NATIVE-01 C03 fails because completion parsing freezes navigation:
  move parsing/persistence to a bounded worker and publish a small typed delta;
  rerun C02-C04. Progress is a lower latency distribution without lost events.
- QUEUE-01 C02 fails because a clicked meeting stays behind queued work:
  promote its existing identity rather than enqueueing a duplicate, then rerun
  C01-C04. The dispatch trace must show the promoted job and one active copy.
- ASSIST-01 C02 fails because a late answer appears in another meeting:
  bind results to conversation/context revisions, then rerun C01-C04.
  Progress is zero cross-context leaks in the delayed-result matrix.

## First-run learning

Establish the local latency/resource baseline before optimizing. Determine
whether deterministic WorkIQ calendar calls are viable with the app's isolated
authentication; do not put ordinary event retrieval through a model merely
because the prototype does. Measure actual CLI cold-start and child-process
cost before choosing a resident harness. A Rust-owned product may still need
the official external Copilot runtime; an exception is explicit, not hidden.

Suggested first bindings: a seven-day future preparation horizon, two total
AI execution slots with one available for foreground work, and a bounded
64-item pending queue. These are proposals; measure burst load, coverage,
energy, model/MCP limits, and foreground waits before owner revision.
Do not mistake provider wait time for UI input latency.

Optional ideas to discuss, not extra completion requirements: battery-aware
preparation, visible Pause/Resume, local full-text search before semantic
retrieval, and a compact activity/source inspector rather than more permanent
calendar chrome. Auto-start at Windows login and remote writes are separate
future decisions.

## Runner handoff

The owner has authorized sequential execution in this session. No extra workflow
or recurring automation has been enabled. Each run reads its exact card/version:

> Read the selected Goal Card and its shared contract. Bind all run parameters
> and verify permission before acting. Assess the current result first. Repair
> the next failed check within the allowed outputs, record evidence, and rerun
> invalidated checks. Keep cumulative caps across resume. Return DONE, BLOCKED,
> CAPPED, or CANCELLED. Never weaken the card or treat a capped run as success.

Markdown is a portable contract, not an enforced runner. Use supported host
limits when execution is separately authorized. The finite engineering repair
loop is distinct from the product's long-running tray scheduler.
