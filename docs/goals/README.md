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

Current engineering slices: the native calendar foundation was committed and
pushed as `d16ed22`. The Rust SDK isolation/control pilot includes child-only
state options, lazy async execution, read-only policies and a native assistant
shell, committed/pushed as `56669e6`. Its no-model runtime probe created a session only in the isolated probe
directory. ISO-01 remains **BLOCKED for full acceptance** on isolated sign-in,
approved live-call limits, live connector checks and regular App observation.
NATIVE-01 still needs its full benchmark/input/appearance matrix and owner
acceptance. The subsequent QUEUE-01 engineering slice implements durable bounded
admission, newest foreground promotion, atomic saved-result completion, native
tray controls, profile ownership and priority shutdown. Local fixtures cover
hidden notification backpressure, obsolete results and interrupted recovery;
they do not prove authenticated tray operation or the full latency contract.
QUEUE-01 and ASSIST-01 are not complete; an assistant shell is not
accepted live integration. The delivered offline assistant slice adds bounded
memory-only context drafts/last answers, isolated public topics, snapshot
invalidation, guarded request identities/admission and focus routing. It does
not establish persistent conversations, retrieved-source support, local
proposals or a real-provider question. The subsequent offline follow-up binds
preparation to the connected model/connector/credential/prompt configuration,
preserves explicit retry after failed/interrupted operations, and adds an
optional app-owned SDK credential slot without enabling ambient login.
These repairs do not establish authenticated integration or full acceptance.
The following source-inspection slice captures bounded SDK-observed tool
success/failure results, redacted excerpts and safe original links in memory.
It keeps evidence atomic with its context-bound answer and labels empty capture,
truncation and unverified claims explicitly. Fixture coverage is not real
WorkIQ/WebIQ retrieval or claim-to-passage acceptance. The subsequent local-draft
slice adds editable memory-only task/email/calendar suggestions, bounded
context-scoped retention, changed-snapshot review warnings and explicitly
opted-in typed model proposals. Keeping or reviewing a draft has no remote
effect and never grants write permission. Real-provider proposal checks and
full goal acceptance remain pending. Private run records remain in the execution
artifacts.

### Current reconciliation (2026-10-09)

The engineering slices above are historical, not the current acceptance verdict.
The native product now has app-owned browser authentication and direct Rust
WorkIQ calendar reads, a single connection/progress status bar, editable local
proposals, observed tool evidence, improved keyboard routing, and inclusive
mini-calendar date selection. The owner accepted the date-selection behavior,
not the complete final build.

Independent calendar synchronization covers one calendar month back and six
calendar months ahead, future first, in bounded 42-day requests. Empty-day
freshness is durable. AI preparation uses the six-month future horizon;
automatic read-only failures have three delayed retries, and upcoming automatic
briefings renew daily. Manual and unknown-outcome failures still require review.
This supersedes the original seven-day/no-automatic-retry pilot binding without
changing the acceptance thresholds below.

The current implementation also fixes the actual hosted WorkIQ `answer`
envelope, allows the observed slow context calls without the earlier transport
timeout, and preserves verified calendar delivery after readiness changes.
Repository collections now discover additions and maintain a bounded private
full-text index on a separate worker. HTML text extraction and chunked reads
replace the earlier small raw-file limit; OneDrive customer research remains
on WorkIQ rather than implicitly hydrating cloud placeholders. Offline checks
and a real local repository catalog establish these engineering slices, not
complete source coverage, claim verification or final-build performance.

| Card | Remaining acceptance evidence |
|---|---|
| NATIVE-01 | Final-build cold/warm, interaction and resource thresholds; full appearance/DPI/physical-keyboard matrix; exact-build owner acceptance |
| ISO-01 | Actual product briefing/chat lifecycle and personal-session comparison, including observation of the regular Copilot App after restart |
| QUEUE-01 | Authenticated six-month/tray run, hidden completion and restoration, priority/recovery/child-lifecycle measurements |
| ASSIST-01 | Actual mail/calendar/web questions and follow-up, claim-to-retrieved-passage comparison, complete interaction acceptance |

Local fixtures and earlier live calendar reads establish individual slices,
not these full-card finish lines. The owner subsequently approved encrypted
Windows-user-scoped DPAPI storage for WorkIQ login. The new implementation
restores it quietly and can remove it explicitly. The initial source-index
deployment demonstrated quiet AI/WorkIQ restart and durable calendar growth,
but overlapping context reads timed out before a fresh Insight completed.
A reserved calendar slot and longer, separately bounded context deadline were
then staged. The later owner-approved deployments below supersede that state.
Completion of all AI preparation within 30 minutes has not been established.
No card is DONE.

The subsequent recovery slice completed a real windowless Insights request
through the production engine in 185,944 ms and persisted/reopened its private
result. Three native context reads succeeded; two of six model source links
match bounded captured evidence. This is not independent source-to-claim
acceptance, local-index/customer-folder coverage, or a fresh product-ledger/UI
completion. A live read also reproduced a calendar event touching the requested
start boundary; repaired normalization validated the formerly blocked 42-day
January range with 64 events and three bounded reads, without modifying the
product cache. That normal production release was built, input-bound and
preserved before subsequent deployment. Observation of the regular Copilot App was blocked
by the host's UI safety policy; no alternate personal-state inspection was used.

A subsequent indexed check exercised the production answer function against
a consistent private source-cache snapshot. It exposed and repaired loss of
local-file provenance and replacement of complete workplace prefetch with its
diagnostic excerpt. Topic-based discovery then completed in 99,349 ms, reading
four files from three authorized repositories and using their returned paths
in actionable recommendations. Local documentation supports the suggested
reuse cases; this does not establish customer adoption or fresh demo execution.
The latest full offline suite, strict lint and normal production build passed.
That release was initially privately preserved without deployment; the later
direct-cloud and product checks below supersede that stage, not full-card acceptance.

A later direct native OneDrive slice verified the configured folder's 40 direct
children, a relevant subfolder's five children and its original HTML content.
Unlike semantic search, which returned a readable copy outside that folder,
the native tool verifies drive/parent identities and cites original metadata.
The bounded pipeline took 4,848 ms and seven existing-budget reads; this is not
recursive folder indexing. Catalog prefetch repaired real read-budget contention.
A subsequent production-function SDK diagnostic completed in 109,485 ms, read
three local files plus the original cloud document, and used/cited those sources
in actionable recommendations. Production validation, private persistence/reopen
and owned connection cleanup passed. That windowless cloud slice did not create
a completed product job and establishes neither all cloud-source coverage nor
independent workplace-claim or final-build acceptance.

Cloud catalog and workplace prefetch are retained independently so oversized
history does not replace a usable catalog with its diagnostic excerpt. Offline
regressions cover late catalog entries, context timeouts, redaction and the
unchanged prompt budget. These are engineering checks, not additional live
provider or product acceptance.

With owner-approved deployment, the normal product quietly restored both saved
connections and completed a foreground **Research now** request. A new briefing
was persisted by the production queue, reloaded and displayed in both compact
and expanded native Insights, with actual local and original-cloud evidence.
No diagnostic payload was injected into storage. This closes the missing
fresh-product-completion engineering check, not the remaining full-card gates.
The same run exposed a 604-participant calendar event blocking range delivery.
A coupled repair bounds retained participant records at 500, preserves total
count/full-list identity, and labels omitted participants rather than dropping
the event or rejecting the whole range. Deployment of that repair and
the broader lifecycle/performance/owner matrix are separate checks.
The saved-login native repair diagnostic completed the formerly blocked
29-day range in 39,696 ms with 195 events and seven large meeting instances
(571-610 participants). Counts and list fingerprints were preserved without
product cache writes. After explicit owner approval, the normal repaired
release was deployed and launched once, quietly restoring both connections.
The product completed all 213 horizon days from 10 September 2026 through
10 April 2027. Its durable cache contains the seven large instances with the
retained-list bound, full counts and fingerprints; the native UI displayed
`People (500 of 604)` with its limited-list notice. The saved foreground Insight
survived restart unchanged and reopened in compact and expanded native views.

Automatic Insights were resumed through the activity panel. A new automatic
job completed with actual local and original-cloud reads, was independently
reopened from production SQLite, and displayed in the native Insights UI while
the next automatic job was researching. Navigation continued during the work.
This closes the repaired-deployment and first automatic-completion checks;
the queue-wide 30-minute requirement, hidden-tray lifecycle, exact-build
performance/input/appearance matrix, personal-session comparison and independent
source-to-claim acceptance remain open. No card is DONE.

Acceptance tooling now seeds and reloads the frozen corpora through the real
SQLite cache. Dynamic preview runs are not frozen-corpus evidence. On the
first repaired acceptance-only release, three 1,000-action idle trials passed
for 200 events, but all three 5,000-event trials failed
(p95 54.79-61.98 ms; maxima 109.09-119.43 ms). The subsequent readable-lane build
passed all three 5,000-event trials (p95 33.99-37.08 ms; maxima 85.14-92.96 ms),
but one of its three 200-event trials still failed (maximum 159.19 ms).
Neither result applies to the later Insights/login/branding source changes
without a new exact-build measurement. Every scripted task
state was verified; that does not turn the timing failures into PASS. The
measurement ends at the next native frame, not an independently observed GPU
display timestamp. One loader-inclusive startup observation also does not
satisfy the required cold/warm launch matrix. The running production binary
has not been replaced with this acceptance build.

### Final verification slice (2026-10-10)

The final staged native release passed 193 offline tests, formatting, strict
all-target clippy and locked/offline release builds. The retained backend passed
89 tests; the browser passed 164 tests, lint and its production build.
The initial E2E run failed while loading Vite modules, before calendar assertions.
Vite was recursively watching unrelated generated native/Python directories.
Excluding those directories restored navigation without changing assertions,
timeouts or skips: the four affected desktop scenarios passed, followed by
39 passing full-suite cases and nine existing viewport-specific skips.
A separate watcher check confirmed browser sources remain watched and generated
native, Python and data directories do not. Historical failed runs remain failures.

The staged normal and acceptance executables are independently hash-bound;
their measurements are not transferred from the running product or older
builds. Forty-eight frozen acceptance cases are prepared but not run.
An explicit benchmark-launch confirmation could not be obtained while the
owner was unavailable; no GUI launch or production replacement was performed.
The currently running product retained both connections and continued automatic
research during the local checks. A 61-second active native resource sample is
not settled idle acceptance or an approved whole-process-tree resource limit.

Full-card acceptance remains blocked on exact-build performance and appearance/
physical-input checks, hidden-tray/owned-child lifecycle, the host-blocked
regular Copilot App comparison, actual assistant workload/claim checks, and
final owner acceptance. The product has no configured WebIQ API key or explicit
GitHub MCP token; host-provided tools cannot establish product integration.
No personal MCP configuration or credentials were imported. No card is DONE.

### Controlled native load repair (2026-10-10)

A 5,000-event storage-worker test exposed repeated whole-queue serialization
and activity cloning between local detail requests. The worker now serves
consecutive reads without rebuilding the unchanged queue, yielding for provider
results, lifecycle changes and its existing deadline. The regression verifies
both occupied provider slots, reserved foreground capacity, 80 detail reads,
cancellation before promoted dispatch, stale-result rejection, a current result
larger than 48 KiB, worker detail read-back and interrupted-work recovery.
Malformed saved details explicitly fail and preserve the original payload.

The final source passed 197 offline native tests, formatting, strict all-target
clippy and locked/offline release builds. Three controlled release repetitions
dispatched the promoted request 66.82, 51.57 and 87.74 ms after capacity became
available, all within the existing 100 ms bound. The earlier 307.90 ms debug
timing failure and stalled-detail failure remain recorded; debug timings are
not release performance acceptance. An additional native UI check keeps editing,
search, navigation and cancellation responsive while the provider progress lock
is held and 200 notifications arrive. Automated palette checks pass all eight
theme/accent combinations at the existing contrast limits.

The repaired normal executable is staged under `native\target\goal-load\release`,
not deployed. A new 50-input/four-binary binding and load-repair record preserve
its exact identity separately from previous builds. Specific confirmation to
launch its first isolated acceptance window again returned owner unavailable,
not approval; no GUI launch was performed. Controlled worker tests do not
replace the frozen native UI/GPU workload, real-provider saturation, idle/
process-tree budgets, tray lifecycle, source verification or owner acceptance.
All four cards still retain their open mandatory gates.

### Single-instance tray recovery (2026-10-10)

The normal product now requests the existing same-profile window on a second
launch instead of opening a duplicate worker or only reporting an ownership
error. A session-local, profile-scoped Windows event carries no payload; it
coalesces pending requests and waits in the kernel rather than polling disk.
It is registered before preload and its listener joins after the native event
loop closes. Measurement, preview and diagnostic modes remain exclusive and
cannot reactivate the product. An older instance without the event retains an
explicit, bounded tray-fallback error.

The Windows tests verify interprocess signaling, pending coalescing, startup
races, separate profiles, collision/legacy errors, ownership retention and
cleanup. The shared tray restore function emits visible/unminimized/focus
commands without closing the app. The full native suite now passes 206 offline
tests, formatting and strict all-target clippy. Release transport tests also
pass all eight cases; the controlled 5,000-event promoted dispatch takes
84.33 ms against the unchanged 100 ms bound. The rebuilt normal/acceptance/
fixture executables are staged, not deployed; the prior load-build binaries
are preserved separately before their original target paths were reused.
This does not establish the final product's actual hidden-window presentation
or live-child cleanup.

Computer Use on the unchanged running product verified Ctrl+Shift+P, command
filtering, arrow selection, Enter execution and Escape from expanded Insights.
The global question field accepted a short synthetic full value and clearing;
Shift+Enter and bulk typing returned no verified text change and remain open,
not PASS. No question was submitted, no second GUI was launched and no owner
acceptance was inferred.

### Source provenance and citation repair (2026-10-10)

Native mail/event evidence now retains the host-validated request resource
outside its bounded content excerpt. The inspector distinguishes a message or
meeting read, collection results and a workplace search summary. A large body
cannot hide the request identity; returned data cannot replace it. Raw and
percent-encoded known credentials are rejected before dispatch or budget use.
The model-facing tool result and read limits remain unchanged.

Researched Insights bind each cited URL to the successful captured records that
contain that exact link. Unmatched citations remain visible with a capture-gap
label, not silently removed or declared verified. Legacy cached citations remain
readable and unverified. Compact Insights report link coverage; expanded and
assistant inspectors expose provenance and capture limitations. Credential-
bearing URLs are rejected and not made clickable in older cached sources.
Updated instructions require original reads for decision-bearing history where
available and explicit uncertainty otherwise; the changed system instructions
participate in the existing analysis revision.

The final native suite passes 213 offline tests, formatting and strict all-target
clippy. Release evidence, Windows transport and scheduler suites pass 8, 8 and
23 tests respectively; controlled 5,000-event promoted dispatch takes 71.01 ms
against the unchanged 100 ms bound. Normal/acceptance/fixture builds pass.
Persistence/reopen and rendered compact/expanded text are checked with fixtures,
not presented as live factual-claim or final-build UI acceptance.

The existing product had stopped before the owner's restart request was acted
on. Computer Use reopened the unchanged, previously approved installed version;
both connections restored quietly and calendar freshness reached 213/213 days.
The new source/restore build remains staged: the separate new-build confirmation
returned owner unavailable, not approval. No new manual question or remote write
was submitted. The reopened product's background research continues, but that
does not establish full queue throughput or acceptance of the staged changes.
All four full-card gates remain open as previously recorded.

### Activity navigation repair (2026-10-10)

The activity list used a fixed-height virtual-row estimate while long titles
wrapped into taller rows. Rows now have a measured 34-point height, single-line
truncated titles and full title/error hints. Explicit event-owned widget IDs
survive reordering and filtering; ordinal-dependent IDs did not and were
corrected after a failing focus-identity test.

The local All/Working/Waiting/Problems/Ready filters cover the entire ledger.
Problems includes failed and interrupted work, which are both counted as
needing review in the status bar. An empty filter is labelled explicitly.
Fixtures verify both problem targets among 5,000 jobs, four row widths, stable
identities and keyboard detail opening without any retry or AI submission.
The existing immutable activity snapshot and scheduler policies are unchanged.

The final native suite passes 217 offline tests, formatting and strict clippy;
54 release native UI tests and the controlled saturated-worker check pass.
Promoted dispatch takes 56.62 ms against the unchanged 100 ms bound.
Normal/acceptance/fixture builds pass. These checks do not establish the
staged build's actual display/input/DPI acceptance.

A bounded read-only query of the app-owned ledger explains the one live failed
job: its meeting or AI configuration changed, so the previous outcome requires
review before explicit retry. No retry was performed, and that safeguard was
not removed to make the queue look successful. The installed product remains
running unchanged; the repaired build is staged with its predecessors preserved.
Full-card acceptance remains BLOCKED on the previously recorded external gates.

### Off-thread activity indexes (2026-10-10)

The activity panel still sorted the full ledger and rebuilt a full calendar
identity map on every repaint. The status bar also scanned all jobs to count
ready/problem states. These operations now use indexes built with the immutable
snapshots: storage builds activity ordering/filter views, and calendar loading
builds identity lookup after sorting events. Painting only looks up visible
rows and reads precomputed counts.

Activity indexes are reused when only pause, connection/sync or error state
changes. Queue comparison and index construction happen outside the publication
lock; unchanged snapshots are not cloned and republished. No scheduler ordering,
retry, admission or provider policy changed.

The full native suite passes 220 offline tests, formatting and strict clippy.
New 5,000-job checks compare every filter/order/tie to the previous behavior,
verify lookup returns original jobs, and observe worker publication, index reuse
and replacement after promotion without any provider. Calendar identity tests
cover sorting, removal, renaming, rebuilding and empty/missing identities.
Existing rendered activity/focus tests continue to pass. These local boundaries
do not replace final-build physical latency, DPI/input or owner acceptance.
Release publication tests, all 14 calendar tests, all 54 native UI tests and
the controlled saturated-worker test pass. Dispatch after availability takes
72.79 ms against the unchanged 100 ms bound. Normal, acceptance and fixture
executables build successfully. The new build is staged: action-specific launch
confirmation returned owner unavailable, so the installed build was not replaced.

### Outlook categories and app-owned external sign-in (2026-10-10)

The owner requested Outlook-like category colors and native WebIQ/GitHub
connection controls. Case variants of `nejisté` now share one filter and
color, including synthetic tentative responses; event storage retains the
original Outlook names. The six requested color families are theme-adjusted
and pass label-contrast checks across the existing appearance choices.
Normalized category filters are indexed during calendar construction.

The native status bar opens a password-field WebIQ dialog and a dedicated
GitHub App device-flow dialog. Current-user DPAPI protects saved credentials
with separate provider/endpoint bindings. Fixed read-only MCP endpoints,
discovery and a small verification read precede saving. Startup restoration,
cancellation, expiry, confirmation before forgetting, generation-fenced
vault saves, immutable per-request registry snapshots, expanded credential
redaction and scheduler connection identity changes are wired end to end.
Ambient environment/personal GitHub credentials are not adopted.
GitHub registration, repository installation/grants and actual browser
authorization remain owner actions; expiring tokens require sign-in again.

The native suite passes 230 offline tests, formatting and strict clippy.
Tests cover category/filter/contrast behavior, native dialog command admission
and secret masking, read-only SDK header/policy overlay, device validation and
polling transitions, DPAPI/provider binding and removal fencing, preview denial,
and worker identity replacement/restoration. These tests do not establish actual
WebIQ/GitHub access or final physical UI/provider/claim acceptance. Full-card
gates remain open; no new build was deployed by these implementation checks.
Normal, acceptance and fixture executables build successfully. The serialized
release UI/calendar/scheduler suites pass 55/15/26 tests. Controlled promoted
dispatch takes 72.85 ms against the unchanged 100 ms bound; cancellation is
30.33 ms. The initial concurrent release suite missed the dispatch bound at
155.13 ms; that failed run is retained, and its cause was not independently
established. Neither result replaces the final physical performance trials.
The bound product hash starts `0699C5AF`. The predecessor binaries were
preserved before rebuilding, and the installed executable was not overwritten.
Action-specific launch confirmation returned owner unavailable, not approval.
No new-build GUI or real WebIQ/GitHub authorization was performed. The previously
observed installed process was no longer running when checked; this work did
not stop it, and the reason for its disappearance is unknown.

### Connector protocol and cancellation repair (2026-10-10)

Continued implementation found that connector verification still used rmcp's
newest default protocol despite these providers using the `initialize`
handshake. Verification now selects an initialize-compatible revision and
permits stateless HTTP. Read validation rejects provider errors embedded in
successful MCP responses, empty/oversized results and mismatched GitHub
identities. Account/browser verification does not extend token/device lifetimes.

Loopback HTTP fixtures exercise the real MCP client, discovery and one read for
both providers, duplicate tool rejection and provider-error rejection. They
also verify that cancellation and shutdown close pending discovery/read
requests and the service peer refuses further calls. No model, remote provider,
browser or private profile is used by these fixtures.

Removing a connector now invalidates cached assistant answers as well as the
current answer, retaining unsent questions and local drafts. Successful
reconnection invalidates old account answers even when readiness stays true.
A transient recursion error introduced during that helper refactor was caught
by the UI test and repaired; the final full native suite passes 235 tests with
seven explicit live-test ignores, formatting and strict clippy. These checks
do not complete actual provider authorization or physical UI acceptance.

The repaired source also builds the normal, acceptance and fixture executables
in locked/offline release mode. Serialized release checks pass 100 library
tests (including the 12 connector checks), 56 UI, 15 calendar and 26 scheduler
tests; two library live tests remain explicitly ignored. Controlled promoted
dispatch takes 63.94 ms against the unchanged 100 ms bound; cancellation is
38.84 ms. The earlier 155.13 ms failed run remains recorded. These observations
do not replace physical input/GPU/resource or real-provider acceptance.
The new staged product hash starts `4FE4DAF5`; the 53 frozen source inputs,
seven release executables and documentation are bound in the private
`native-external-http-build-binding.json` evidence. Six predecessor binaries
were preserved before the build. The installed product remains unchanged;
no new-build GUI launch or actual WebIQ/GitHub authorization has occurred.

### AI completion cancellation fence (2026-10-10)

Further investigation found a publication race: an AI task could finish and
drop its cancellation receiver before the executor handled Cancel, source/account
changes or shutdown. Sending cancellation then failed, and the queued success
was checked only against the connector digest. Reconnecting the same credentials
could also restore that digest without restoring the old request's validity.
Cancellation now retains its value even without a live receiver. Questions and
Insights share a final active-request/cancellation/configuration/shutdown check
before notifying the UI or storage. Provider errors remain explicit, and a
completion retires its request exactly once.

Four deterministic regressions cover both request namespaces, a finished
JoinSet task followed by cancellation and same-credential restoration, shutdown,
missing/stale identities, provider failure and duplicate delivery. The full
native suite passes 239 offline tests with seven existing live ignores;
formatting and strict all-target clippy pass. These checks are local completion
fences, not real-provider or final physical acceptance.

The first repaired release passed library/UI/calendar checks but missed the
existing dispatch limit at 100.54 ms. Its log, 53 source inputs and seven binaries
are preserved in `native-completion-fence-failed-release\binding.json`; the
cause of that timing observation was not independently established.
Investigation found that every slot/result refresh serialized and hashed all
unchanged calendar events. The queue now reuses their validated fingerprints
while preserving expiry, daily renewal, coverage and admission checks.
Calendar/configuration changes still trigger full reconciliation, including
running-request invalidation. Manual promotion does not mark calendar inputs
as changed. Two additional regressions compare refresh/full reconciliation
across time, completion, expiry and newly eligible events, and retain changed
meeting/configuration invalidation. The full native suite passes 241 tests,
seven existing live ignores, formatting and strict clippy.

The subsequent locked/offline release builds pass, with 106 library, 56 UI,
15 calendar and 26 scheduler checks. Controlled dispatch takes 51.10 ms and
cancellation 45.25 ms. Two additional same-build trials dispatch in 40.88 and
75.72 ms, with cancellation at 25.12 and 31.36 ms; all three satisfy the unchanged
100 ms bound. Both previous timing failures remain recorded, not discarded or
reclassified as successful trials. These measurements use controlled provider
fixtures, not final physical/GPU or actual-provider acceptance.
The exact repaired build, 53 source inputs, seven executables, logs and preserved
failed predecessor are recorded in the private
`native-completion-fence-dispatch-build-binding.json`. Installed product and
private profile remain untouched; no new-build GUI launch was performed during
that release-validation slice.

### Product readback and WebIQ verification diagnostics (2026-10-10)

The repaired `4119DD80...` normal product was subsequently launched under the
owner's existing run permission. A read-only observer confirmed that an already
automatic research job completed and persisted version-2 Insights after
98.209 seconds, including one successful captured workplace read. This is real
completion/persistence evidence, not proof of tray behavior, all-source coverage
or independent claim correctness.

Computer Use subsequently confirmed AI and WorkIQ connected, one case-folded
`nejisté` filter, and a failed WebIQ verification read. Discovery succeeded;
the fixed endpoint and `x-apikey` header were already correct. The key editor
was cleared and failed verification did not save the key. No owner credential
was extracted or borrowed for a shell probe.

Safe diagnostics now distinguish HTTP authentication/permissions, throttling,
server failures, transport failures and numeric MCP errors without exposing
provider bodies or credentials. The verification HTTP deadline now matches
the existing bounded discovery/read deadline. Local transport fixtures assert
the actual production WebIQ header and exercise HTTP 401/403/429/503 and MCP
invalid-argument responses. The full native suite passes 242 offline tests,
seven existing live ignores, formatting and strict clippy. An owner retry on
the diagnostic build is still required to establish the actual live cause or
successful connectivity. No full goal card is newly accepted by these checks.

### Optional GitHub token sign-in (2026-10-10)

The GitHub status-bar dialog now offers an explicit **Personal token** method
without custom app registration. **Browser sign-in** remains the default and
preserves the existing dedicated GitHub App Device flow. Typed connector inputs
distinguish WebIQ keys, GitHub client IDs and personal tokens; mismatched provider
inputs fail before contacting a provider. A personal token must verify the GitHub
`/user` account and a matching MCP identity before the existing DPAPI vault save.
No ambient `gh`, Copilot or `.mcp.json` credentials are read or imported.

The token editor is masked and cleared on submission, dialog close, provider
switch and sign-in-method switch. Saved device-flow records remain valid without
migration, while personal tokens have no client ID and undergo separate format
validation. The existing fixed endpoint, read-only tool policy, cancellation,
forget generation, registry identity and result fences apply to both methods.
Token expiry/revocation is detected by provider verification/reads rather than
inferred from the token text.

Local checks exercise token validation, rejected provider admission, encrypted
restart/replacement/removal, masking/clearing, both UI commands and preview
isolation. The full native suite passes 245 offline tests with seven existing
live ignores; formatting and strict all-target clippy pass. Real GitHub
authorization and selected-repository evidence remain owner-dependent and are
not established by the fixtures.

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
Current implementation bindings: six-calendar-month forward horizon including ongoing
meetings, 64 pending jobs, two active slots with at most one background job,
and local Windows timezone. Overflow is deferred, never silently discarded.
Automatic read-only AI failures have three bounded delayed retries; manual and
unknown-outcome operations are not automatically retried. There are no live validation
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

Historical first bindings: a seven-day future preparation horizon, two total
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
