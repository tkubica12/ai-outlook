# Goal Card: nonblocking WorkIQ and WebIQ control panel

## OBJECTIVE

Card: ASSIST-01 | Version: 2 | Readiness: Ready for local implementation; live/owner acceptance remains
Mode: repair loop | Decision owner: project owner

Ask useful workplace/web questions from a native control panel, with optional
current-meeting context, clear sources and uninterrupted calendar navigation.
Use the official Rust Copilot SDK behind ISO-01 and governed WorkIQ/WebIQ adapters.
Non-goals for this proposed first slice: arbitrary shell/file execution,
autonomous email sends, calendar changes or CRM writes.

## OUTPUT

Update the approved native assistant/palette, conversation state, typed tool
policy and adapter modules plus related tests/docs. Module paths bind after
NATIVE-01. Preserve meeting briefings; add calendar-independent questions and
local action proposals, not a duplicate meeting-only chat implementation.
Deliver the real authorized provider workflow and shared run evidence.
Do not retain raw mail or tokens in repository fixtures/status records.

## DONE WHEN

| ID | Pass condition | Verify | Evidence | If it fails | Recheck |
|---|---|---|---|---|---|
| C01 | From calendar or tray-restored shell, questions work with no meeting selected and with an explicit selected context, without requiring a briefing; WorkIQ mail/calendar and WebIQ each return actually retrieved, correctly scoped sources | Owner freezes small question/source inventory; through the real product ask at least one mail, one future-calendar and one public-web question plus meeting follow-up; compare decision-bearing answer claims to retrieved passages | Redacted question/result/source comparison, tool names, observed outcomes and product/runtime identity | Repair context/tool/citation wiring; BLOCKED for unavailable source/permission; no canned success | C01-C04 |
| C02 | Slow answers never block navigation/search/focus; queued/running/cancel/retry/error states are honest; switching meeting/conversation keeps late output attached to its original context; no duplicate submission | Controlled delays/disconnects, large result, cancel/reopen, rapid context changes, repeated Enter/click; repeat the shared busy/idle workload and bounded real product call | State/focus/result-routing matrix, request IDs and latency trials | Repair lifecycle/revision/dedup guards or worker boundary; unknown external outcome stops retry | C01-C04 |
| C03 | Host code exposes only explicitly allowlisted reads; untrusted mail/web/model instructions cannot invoke writes/shell; local proposals identify targets but cause zero remote mutations; secrets never reach rendered/logged output | Inspect actual harness/tool boundary; integration tests for malicious retrieved instructions and proposed sends/calendar edits; inspect attempted tool calls and owned test targets before/after | Policy inventory, allowed/denied invocation records, redacted logs and actual state read-back | Fix typed validation/policy or stop; never enable broad allow-all flags to pass | C01-C04 |
| C04 | Same commands work by mouse and keyboard, including opening panel/palette, typing, submitting, cancelling, source inspection and returning focus; all eight appearance combinations pass NATIVE-01's contrast/DPI/layout rubric; owner accepts final UI | Computer Use on final native build plus bound automated focus/input scenarios; normal text editing and CZ/US shortcut verification; named owner review | Completed task/rubric matrix, UIA states, screenshots, accepted build identity | Fix a named interaction/visual failure; human acceptance pending is BLOCKED | C02-C04 |
| C05 | Final panel, source policy, provider evidence and regressions reconcile | NATIVE-01's locked format/test/lint/build checks plus bound C01-C04 harness; review final outputs and status pointers | Exact commands/results and final input/build/check inventory | Repair coupled failures | C05 and affected C01-C04 |

## QUALITY

Answer directly, distinguish source facts/inferences/unknowns, show useful
original links and retrieval time, and never treat model confidence as proof.
C01 checks support, not citation count. Main panel stays compact; detailed
sources/activity are discoverable rather than always filling the calendar.
No fake working buttons or success before saved/read-back state.
Panel may open instantly while provider connection/tool discovery loads later.

## CONTEXT

Required: shared contract/review, ISO-01, NATIVE-01 UI and QUEUE-01 foreground
lifecycle. Reuse current `CopilotCliRuntime.chat`, source validation and
read-only allowlist semantics, not the old briefing-required API constraint.
Owner must bind identity, allowed account/calendar/mail scope, question
inventory, model/harness version, MCP endpoints, retention and live-call caps.
Unresolved: scope of "get things done"; this draft proposes read-only answers
and local drafts. Actual remote draft creation is also a write and excluded.
Use the resolved official Rust SDK with streaming events and an explicit
deny-by-default permission handler. The external Copilot runtime remains
isolated. No Python/Node bridge or handwritten one-shot CLI protocol.

## CONSTRAINTS

Incorporate shared execution/evidence/permission rules. Read only authorized
WorkIQ data and explicitly requested WebIQ scope through product adapters.
Do not transmit workplace content to public-web search unless specifically
authorized; derive public queries from approved public entities/topics.
Write only approved native surfaces/tests/docs and local app-owned state.
Remote sends, draft creation, calendar/CRM mutations, OS actions and arbitrary
shell are forbidden in this slice. A future write card must add exact-action
confirmation, idempotency/unknown-outcome recovery and actual read-back first.
Do not broaden tool allowlists based on model text.

## STAGES

1. Freeze question/tool/privacy scope and final interaction tasks.
2. Wire lazy native panel, context-bound conversations and isolated real reads.
3. Check sources, failure/cancel/focus and measured load; repair scoped gaps;
   obtain owner acceptance.

## STOP-CAPS

Pilot proposal: three cycles and 150 elapsed minutes, reserve 25 minutes for
verification. Proposed live ceiling: six model runs and 30 MCP tool invocations
including retries; owner approval and enforceable metering are required.
If invocation counts cannot be observed/enforced, BLOCKED for live validation.
No new paid service. Stop after three cycles without closing a failed check or
reducing named routing/source/interaction defects, or earlier at any cap.

DONE requires C01-C05 PASS with real product integration, approved performance
limits and owner acceptance. BLOCKED for missing scope, identity, metering,
provider or human decision. CAPPED/CANCELLED preserve partial outputs, actual
external outcome IDs, check states and remaining gaps. Resume retains counters.
