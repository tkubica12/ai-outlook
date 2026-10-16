# Tomlook native

See the root README for build/run instructions and the goal contracts for
acceptance requirements. This package is the production migration target; the
browser prototype remains intact until native parity and live integration pass.

The portable Windows 10/11 x64 executable requires the current
[Microsoft Visual C++ v14 Redistributable (x64)](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist),
including `VCRUNTIME140.dll`. The archive does not install this runtime or the
separate Copilot CLI runtime required for live AI.

Tomlook uses the **Focus** identity: a T inside four rounded focus corners.
The header draws the mark directly as vectors. Window and tray icons follow
the selected light/dark accent and update only when that color changes.
The executable embeds matching transparent icons at 16, 20, 24, 32, 40, 48,
64, 128 and 256 pixels for Windows Explorer and high-DPI taskbar use.
All native sizes share the same geometry; no font, image file or network
request is needed to display the brand.

The UI owns only navigation, filters and immutable calendar snapshots. A bounded
command channel feeds a dedicated storage worker; only that worker opens SQLite,
imports existing state, parses payloads and computes per-day overlap layouts.
Worker notifications request repaint on change; no recurring idle UI poll runs.
Meeting detail requests carry a revision so an old response cannot reopen or
overwrite a different meeting. Settings and calendar transactions survive restart.

## Privacy-safe showcase

`Tomlook.exe --demo --state <new-empty-directory>` renders a curated, fictional
workweek with six categories, full-day events, travel buffers, quiet unanswered
invitations, and bilingual sample Insights. `--stress` uses the same preview
generator with 5,000 events. The default profile and any nonempty profile are
rejected before opening storage. The legacy root is never imported in preview;
the original frozen benchmark corpus is unchanged.

The demo window/header are explicitly labelled, connector controls are replaced
by an offline-demo status, and sample Insights are marked as not AI-generated.
Selecting a demo meeting asks the bounded storage worker for validated,
hard-coded sample Insights, not the saved production briefing table or a model.
Contacts use `example.invalid`; there are no source links or fabricated
retrieval records. Existing preview guards still prohibit provider connections,
browser sign-in, live reads and calendar writes. No credentials or personal
calendar data are required for README screenshots.

`--demo-screenshot <new-file.png>` automatically selects the Tuesday architecture
workshop and captures the actual native framebuffer after its sample Insights
have loaded. It cannot be used in production or combined with measurements.
Only the demo window content is captured, not other desktop windows or the
Windows title bar. PNG encoding and exclusive-create file output run on the
bounded storage worker. Existing images are never overwritten.

## Native interaction measurement

The optional `tomlook-acceptance` binary shares the production desktop entry
point, so it can be built without replacing a running `tomlook.exe`.
It is restricted to measurement modes, not a runnable production update.
The current product is always `..\release\Tomlook.exe`. Publish it from the
repository root with `.\scripts\publish-native.ps1` after exiting that release.
The script builds only `tomlook`, with locked offline dependencies, then stages
and atomically replaces the single product. It leaves the old release intact
on failure and cleans its own temporary copy. Cargo caches stay under ignored
`native\target`; temporary measurements/staging belong under ignored `.build`.
Do not distribute acceptance binaries or create version-named release directories.
For an isolated temporary check, build from this package directory:

```powershell
cargo build --release --locked --offline --bin tomlook --target-dir ..\.build\check
```

The test product is `..\.build\check\release\tomlook.exe`; remove that named
temporary directory after the check. Deploying and launching is separate from
building. These build commands do not create a GitHub Release. The native
package version sets Windows file/product metadata; public deliveries are
published separately as tagged releases.

First use the non-UI `calendar-fixture <new-tomlook-fixture-directory> <200|5000>`
utility to seed a new owned profile with the frozen corpus through production
SQLite storage. Existing data is never replaced. Then use
`--fixture-cache --interaction-trace <new-file.json> --state <seeded-directory> --legacy-root <empty-directory>`
to run 1,000 keyboard interactions through eframe's raw-input hook.
The cache's complete event/coverage fingerprint is checked against both its
receipt and the frozen generator on the preload worker. Dynamic `--demo` and
`--stress` previews are not this acceptance corpus. Normal profiles and legacy
imports are rejected, providers are disabled in measurement mode, no
interaction-test tray icon is created, and physical input aborts the trial.
The 120-second deadline is also enforced from the non-rendering logic callback
while the measurement window is minimized or hidden.
Each sample verifies a task-specific UI state before measuring the start of
the next native frame, with separate UI-callback and framework rendering-CPU
diagnostics. The application exits and records results after owned
worker shutdown. Use a new output path for every trial; preserve failed runs.

This is a framework-input/presentation-boundary measurement, not an independent
GPU-display timestamp or OS input latency measurement. It does not establish
physical CZ/US keyboard acceptance, active-provider interference, the complete
appearance matrix, or full goal-card acceptance. Existing `--startup-trace`
remains separate. The acceptance executable requires the seeded fixture cache
for startup traces as well; these load the actual persisted calendar, not a
fresh demo. Startup reports additionally include Windows process-creation time
and its alignment with the entry-point monotonic clock, so executable-loader
time is not omitted.

Meeting cards keep their complete accessible title and selected state without
shaping a second transparent copy of the title. Offscreen cards skip widget
construction; their calendar entries remain indexed, searchable and scrollable.
Keyboard selection still realizes the selected card when focus or scroll
reveal is pending.

All-day events retain their source calendar dates and use an exclusive end:
November 2 at midnight through November 4 at midnight is displayed on November
2 and 3 only, without shifting the boundaries into the viewer's time zone.
Details and AI context expose the same inclusive date range. Timed events still
use the viewer's local time zone.

The mini-calendar always opens Day when a date is clicked or activated from
the keyboard. Hold and drag to preview an inclusive date range; release commits
exactly one column per selected date. Reverse selection, month/week boundaries
and Shift-click are supported, including intervening weekends. Selection is
limited to the 42 visible mini-calendar dates. Escape or loss of focus cancels
the drag. WorkIQ refresh waits for release and uses the committed range only.
Previous/next and Alt+Left/Right advance by the custom range's length; preset
views clear it. A new timeline opens at 08:00 while earlier meetings remain
available by scrolling. Manual scrolling is preserved within the same period.

Preset selection highlights every displayed day in the mini-calendar (only the
actual month in Month view). The chevron before each row selects Monday-Friday
with one click; its full-height hitbox and the sidebar width leave room for
every date beside the scrollbar. Its accessible name identifies the week. Category markers are
filled small squares. Unconfirmed responses have no category stripe, location
or summary: only muted time, response and truncated title. The direct
**Hide unconfirmed** filter also removes their overlap lanes and keyboard targets.
Graph `isOrganizer` keeps owned appointments normal. Follow is used only when
explicitly supplied; `showAs=free` and meeting title prefixes do not establish it.

## Language and structured Insights

Right-click a meeting or press **Shift+F10** on the selected meeting to open
the same native actions. The detail pane also has a **Meeting actions** button.
Category changes replace all previous categories with one existing category;
clearing removes them all. Selecting or clearing a category submits that exact
change immediately, without a confirmation dialog or opening Messages.
The current single category is marked and cannot be selected again.
Attendees can accept, tentatively accept or decline,
with an optional comment and an explicit send-response choice. Organizers cancel
meetings with attendees (notifying everyone); deletion is offered only for owned
appointments without attendees. Invitation responses, cancellation and deletion
use one compact confirmation: meeting, date/time, single-occurrence scope when
relevant, and optional comment/send-response controls or cancellation notification.
Confirming closes it without opening another window. Escape, closing the dialog
and merely opening the menu never submit a change.

Only a verified single event, occurrence or exception is writable; series
masters and old cache entries without event type remain read-only until refreshed.
Missing-type days are refreshed off the UI thread. Both an explicit category
selection and a confirmed response/deletion reuse the durable one-shot approval
pipeline, bound to the connected account and exact meeting, independently of the
daily AI budget.
The provider validates fresh event identity and fingerprints, supported native
tool/schema contracts and read-back. Eventual consistency retries reads only,
not writes. WorkIQ entity schemas validate category/buffer writes; invitation
responses use the verified `do_action` tool contract and four fixed native Graph
operation/body contracts. `get_schema` is not used as an action-schema endpoint.
An exact resource HTTP 404 verifies removal; permission errors
do not. Calendar read-back does not independently prove notification delivery.
Unknown/interrupted outcomes stay in Messages for review and are not replayed.
**Check calendar result** on existing-event operations retries read-back only
against the saved approval, never resubmits the write. A changed recurring
occurrence can legitimately become a Graph exception; verification allows only
that metadata transition while requiring the exact identity, categories, times
and other fields. These reads use the reserved calendar lane, so background
research cannot starve manual calendar verification.
Definitive provider rejections retain HTTP status and an allowlisted Graph error
identifier when returned, without saving arbitrary response bodies or private
provider messages. HTTP status is also logged. Local request validation is never
reported as a provider HTTP 400. A rejected response is not retried automatically;
the user must explicitly confirm a new attempt.

Recommendation titles navigate to the meeting and saved Insights, clearing
search/category filters that could hide it. Priority-conflict cards show full
local meeting times plus the intersection and offer role-appropriate tentative,
decline, cancel or delete controls, each using the existing single confirmation.
Reciprocal pairs appear only once; distinct additional conflicts remain visible.
Conflict skips store symmetric account/occurrence-pair keys atomically with the
inbox, surviving metadata/research changes and restarts without hiding other pairs
or unknown write outcomes. The storage worker derives and caches the inbox view;
the UI never scans the whole calendar to deduplicate advice. Large conflict lists
are paged six meetings at a time.
Missing-material cards show at most six actual invitation contacts, with the
organizer and same-email-domain attendees first. Same-domain presence is a clue,
not proof of employer or agenda ownership. The verified `/me` sign-in and primary
mail addresses are cached locally, bound to the calendar account, and excluded
from suggestions and AI contact context, including after restart. Fresh Insights investigate ownership
in retrieved mail/Teams evidence and suggest a concrete question to ask; uncertain
ownership is labeled as such. No outreach is submitted.

`src\i18n.rs` owns English keys, natural Czech labels and argument-preserving
message templates. New UI literals must enter that catalog; coverage, uniqueness,
dates, templates and persisted preferences have regression checks.
EN/CZ is a direct header control, not a settings dialog. Calendar view changes
and language switching never submit research. Switching language preserves the
current question and answer, including a running request. Future questions use
the selected language.
Original user/source text and raw external technical diagnostics remain original.
**Ask** and **Ctrl+Space** open a resizable right-side question-and-answer panel,
sharing the meeting-detail slot rather than adding a second sidebar.
**Detach Ask** opens a real, movable native window with its own taskbar entry;
**Dock Ask** returns it to the calendar. Both views share the conversation,
unsent draft, mode and explicit meeting context. New chat, detach/dock and hide
are theme-aware vector icons grouped in the Ask header, with translated tooltips,
accessible button names and visible keyboard focus. Enter asks; Shift+Enter adds
a line. Meeting links select the meeting and open its saved Insights without
losing the conversation or draft.
Hide, native-window Close, and Ctrl+Space only hide Ask, never cancel it.
Escape first dismisses visible command suggestions, then hides the panel.
**Stop** explicitly requests cancellation. A quiet Ask status in the main header
shows a running question or an unread completed result, independently of the
recommendation bell.
Answers render CommonMark headings, emphasis, lists, code, tables and safe
HTTP(S) links. Parsing happens once on the AI executor; the UI shares the prepared
document. Images are not fetched and HTML is never executed. Admitted questions
clear the composer and become user bubbles. The assistant shows a spinner,
elapsed time, actual tool/stage activity and bounded, prepared streaming Markdown.
Internal reasoning is never displayed. Earlier turns collapse after the six
most recent full turns to bound layout cost.
Copy and expandable source evidence remain available with the final answer.
There are no local-concept controls or **More options**. Nine shortcuts appear
below the composer only after `/`: `/today`, `/week`, `/search`, `/insights`,
`/prepare`, `/conflicts`, `/find`, `/travel`, `/categories`.
Typing filters their names/descriptions; Up/Down selects, Tab completes, and
clicking anywhere on a row completes it. Completion never submits a model call.
`/today`, `/week`, `/search <words>` and contextual `/insights` execute algorithms
on the storage worker without AI, including offline or during Thorough research.
Search matches all supplied keywords across meeting names, people, locations and
categories in the saved future calendar. The shared Rust search index ignores
case, separators and common Latin/Czech accents, and ranks normalized matches
ahead of typo candidates. Alphabetic keywords of 4-9 characters allow one edit;
10-64 allow two, including adjacent transpositions. Short and numeric keywords
are not typo-corrected. Adjacent name parts are indexed within a single metadata
field, never across different attendees or fields. `NorthStar`, `North Star`
and `North-Star` agree without customer-specific aliases. Vocabulary matching
is compiled once per search, not once per meeting or UI frame. `/search` lists
the best candidates first, with chronology as a tie-breaker, and labels fuzzy
results for verification. It never calls Luna. The calendar's live filter
retains inexpensive normalized substring filtering, not fuzzy scoring on render.
Date coverage is not proof of name resolution;
the assistant must retry a narrower name token or inspect a bounded calendar range
before drawing a negative conclusion. Timed meeting tools include a complete
local date/time range; answers must preserve both dates for multi-day meetings,
and keep invitation responses separate from tentative categories.
Results include freshness/coverage,
up to 25 ranked matches and meeting links. Narrow the query for more.
`/prepare` requires an explicit meeting context; `/find` requires search text.
`/travel` and `/categories` take no arguments and check the next rolling 14 days,
independently of the selected meeting. They reuse the asynchronous Insights
worker and durable bounded queue, including its single background slot, verified
calendar coverage, account binding, daily budget and retry safeguards.
Travel reviews publish only before/after travel proposals; category reviews
publish only category proposals. Existing unrelated recommendations remain
untouched, duplicate requests share running research, and saved skips remain
respected. Results appear in the recommendation bell and still require approval
before a calendar write. The two checks can also run when automatic preparation
is paused; ordinary questions retain their separate interactive slot.
Selecting a different calendar meeting never retargets Ask. Context changes are
blocked while a question runs; changing the underlying meeting or account still
invalidates stale output. Questions do not send messages or change the calendar.
App-owned recovery, connector and index status messages are translated, including
all-day card labels and their accessibility names. Unconfirmed all-day meetings
keep the all-day label rather than showing a misleading midnight time.

New Insights contain the existing English narrative and a nested `cs` narrative
with the same six fields and matching array lengths/order. Sources and retrieved
evidence are shared. Validation bounds text and rejects missing/extra Czech fields,
empty items or mismatched counts. The compact panel has an overview and counts;
expanded sections are cards in one or two columns according to available width.

The automatic future queue can supplement old unchanged Insights without
re-researching: a no-tools isolated session receives only the stored English
narrative. The worker checks the event/legacy configuration fingerprint,
preserves all original citations and evidence, applies the usual cancellation,
account and connector fences, and persists a new version only after validation.
Translation-only results are explicitly labeled; translation is not new research
or independent verification. Old English stays available if translation fails.
Unsupported legacy fingerprints, changed events and manual Research now use fresh
research instead. Past meetings outside the automatic horizon require Research now.
Language switching itself does not wait for or start translation.

For exact GitHub App registration fields, see the
[Czech sign-in guide](../docs/github-sign-in.cs.md).

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

### Quiet recommendations and confirmed calendar changes

Fresh bilingual Insights can include up to four distinct, evidence-led drafts:
category, short preparation, advance preparation, arrival travel, departure travel, missing materials, priority conflict, follow-up,
and meeting transition. Suggestions reuse that research; there is no separate
recommendation model loop. The status-bar inbox uses a neutral, keyboard-accessible
bell and a small count; diagnostics do not turn it into an alarm triangle.
Opening it never submits research. A larger master-detail window keeps a virtualized
list on the left and one selected action on the right. Previous/Next and
Alt+Up/Alt+Down navigate without submitting anything. Selection survives refreshed
wording, and list navigation reveals offscreen selections. Cards separate the proposed action and time,
associated meeting, and rationale, with technical checks under Details.
Category chips and buffer time ranges are prominent; reasons and controls are
localized in EN/CZ. The category arrow is drawn natively, not a font-dependent glyph.
Conflict cards use current confirmed counterparts and local overlap times rather
than model-written UTC prose. Upcoming suggestions
cover fourteen days; follow-up cards activate after the source meeting ends.
**Draft follow-up** opens Ask with a localized request for follow-up text.
It preserves an existing unsent question, does
not submit research automatically, and never sends mail. Unknown decisions,
owners and dates must remain placeholders until evidence establishes them.
Skip atomically saves a separate account/meeting/action/category intent key, independent
of rationale, event fingerprint, policy revision and duration. Arrival and departure
remain separate, as do different category targets. Preparation variants share a skip
intent. These decisions survive restarts and inbox compaction; unknown writes remain
reviewable and approval fingerprints are never weakened. The bounded dismissal
ledger reports errors instead of silently forgetting decisions. Closed, obsolete
records older than thirty days can be compacted; unresolved writes are retained.
The model receives bounded confirmed-calendar neighbours and overlaps; these
are calendar facts, not proof of the user's physical position. Existing contiguous
travel/reserve blocks suppress duplicate tips independently before and after a meeting.
`office` explicitly means the user's home/employer office, not a customer's or
partner's premises; `mimo office` covers those offsite visits. The model receives
these meanings alongside the exact existing category names. Legacy category and
travel drafts from the previous rules expire; manual actions and unresolved
writes remain available for inspection. New Insights replace superseded pending
actions, while dismissed/applied equivalent actions are not reintroduced.
For a confirmed timed offsite visit, missing arrival/departure drafts are completed
on the storage worker from the existing or suggested offsite classification.
Absent a supported estimate, each is an estimated 30-minute reserve, explained
concisely; unverified route and traffic details stay under Details.
Recognizable travel/reserve/preparation placeholder titles do not receive another
buffer or post-meeting follow-up. A neighbouring workshop does not establish an
active role in that placeholder; recommendations must concern their own meeting.
Category corrections remain available for these entries.

The SDK and its tools remain strictly read-only. A separate native WorkIQ writer
accepts only a concrete user-confirmed proposal for the calendar's connected owner.
It can replace an event's **entire categories array with exactly one existing
category**, or create one busy preparation/travel buffer with **no attendees**.
Allowed category names currently come from the cached calendar, not the Outlook
master-category catalog; unused categories cannot yet be proposed. Buffer slots
use weekdays 08:00-18:00 local time, confirmed-calendar collision checks, and a
five-minute lead. Arrival travel must end immediately before the meeting;
departure travel must start immediately after it. Neither is moved to a detached
gap when the adjacent time is busy. Traffic and real-world availability are not
verified. Confirmed parents have uncategorized buffers, even when an uncertainty
category exists elsewhere in the calendar. Only an explicitly uncertain parent
category (Nejiste/Blocker) is inherited; invitation-response safeguards are unchanged.
Short preparation keeps the latest fitting gap within two days. Evidence-led
advance preparation requires a 60-180-minute estimate and chooses the earliest
fitting workday gap within fourteen days before the meeting, respecting busy
intervals, weekends and the five-minute lead. No free slot means no write.

Confirmation is consumed transactionally before dispatch, with the exact approved
operation saved locally. The writer discovers compatible schemas, refetches the
meeting, checks its complete fingerprint, rechecks covered free time, submits once,
then reads back the exact result before reporting success. Fresh reads are **not
atomic remote concurrency protection**: generic WorkIQ update_entity has no
documented If-Match parameter. A concurrent external change can still cause review.
Only known non-submission/rejection permits another explicit confirmation; an
unknown or mismatched outcome requires inspection in Outlook and is never replayed
automatically. Shutdown drains confirmed operations instead of cancelling them.

Microsoft's [WorkIQ tool reference](https://learn.microsoft.com/microsoft-365/copilot/extensibility/work-iq/mcp/tool-reference)
states that tenant policy blocks mutation operations by default. Read access and
compatible metadata do not establish permission to write. Tenant authorization
must be enabled separately by its administrator; Tomlook neither grants access nor
bypasses policy. Automated safety tests use fakes, not real calendar mutations.
Meeting response actions use a direct JSON object when the discovered `do_action`
schema accepts objects or leaves the body unconstrained, as the live provider's
parameter description documents. String-only schemas retain JSON-encoded bodies;
accept/tentative/decline omit the optional comment when none was entered, while
preserving the exact notification choice. Working create/update requests are
unchanged. Error-code extraction also handles
bounded JSON-encoded provider error envelopes, persisting only allowlisted codes,
never arbitrary provider messages or bodies. HTTP 400 alone does not establish
an OAuth or tenant-policy denial.

### AI accounting, diagnostics and connection maintenance

The default budget is **200 admitted research/question requests per local day**,
shared by manual and background requests and persisted across restarts in
`ai-usage.db`. Admission occurs before context/session creation; admitted failed or
cancelled attempts conservatively consume budget. One request can make multiple
model calls. At the limit, the queue waits for tomorrow without redispatch spinning.

The usage panel displays actual SDK input, cache-read, cache-write and output
tokens. Missing data is unavailable, not zero. Totals can be partial; a short
bounded event drain captures queued usage around idle/cancellation, but does not
guarantee final provider billing totals. SDK `cost` is a billing model multiplier,
**not USD/CZK**. No currency estimate is presented.

Controlled reason codes, run/request identifiers and numeric HTTP statuses go to
bounded rotating `diagnostics.jsonl` logs, never prompts, credentials, meeting
titles or provider bodies. A background writer flushes at owned-executor shutdown.
Logging/accounting failures use the existing Messages warning, not a new banner.
Saved connector restoration retries transient failures indefinitely with
30/60/120/240/480/900-second capped backoff, including when the window is in tray.
Cancellation/forget and permanent authentication/policy errors stop restoration;
retry never launches a browser. Unconfirmed meetings retain explicit/manual
preparation but do not consume automatic research. Incremental-index, passage-level
evidence and additional Office/PDF readers remain separate unfinished improvements.

**Optional command-line WorkIQ preflight.** The diagnostic
initializes the configured Calendar connector using the official Rust
MCP client, then follows its OAuth challenge to published metadata. It runs on
the background executor, independently of SDK connection, with a 60-second
deadline and shutdown cancellation. It never starts Copilot, calls a model,
imports tokens, registers a client, opens a browser or calls workplace tools.
Synthetic previews block the probe. Repeated clicks coalesce to one active test.
Missing setup, discovery failures and the need for an approved client identity
are shown explicitly; discovering OAuth is not successful authenticated access.

Run the same app-owned preflight without launching the UI:

```powershell
cargo run --locked --bin workiq-probe -- "$env:LOCALAPPDATA\Tomlook" WorkIQ-Calendar
```

It prints a bounded, credential-free report, with exit code `2` when
authenticated read access remains unverified and `1` on a test error.
Mail, Teams and SharePoint can also be selected explicitly. This diagnostic
does not implement the browser callback/token exchange: an approved OAuth
client ID and matching redirect URI must be selected before adding that path.
It never borrows Copilot's first-party client identity.
The current WorkIQ authority advertises `organizations/v2.0`, whose Entra
metadata returns a `{tenantid}` issuer template. rmcp's strict discovery rejects
that mismatch before sign-in. For the recognized Agent 365 resource URLs only,
the probe extracts the configured resource's GUID tenant and independently
tests Microsoft's tenant-specific authority. It accepts only published metadata
whose issuer and authorization/token endpoints exactly match that tenant.
The report retains the original generic-discovery error and distinguishes
verified tenant discovery from authenticated access. No metadata is rewritten
and issuer validation is never disabled. Missing PKCE metadata is reported as
"not advertised", not proof that PKCE is unsupported.

Microsoft's [WorkIQ coding-client setup](https://learn.microsoft.com/en-us/microsoft-agent-365/tooling-servers-overview#set-up-work-iq-mcp-servers-for-coding-agents)
documents an organization-approved public client application, connector-specific
permissions and a desktop redirect such as `http://localhost:8080/callback`.
An existing suitably configured application can be selected; otherwise creating
one and granting permissions are separate owner/admin decisions. No shared
first-party client is assumed. Tenant discovery does not establish those grants.

**Hosted WorkIQ Rust experiment.** A separate diagnostic targets Microsoft's
newer `https://workiq.svc.cloud.microsoft/mcp` endpoint, not the four per-workload
Agent 365 servers. It uses the public client published in Microsoft's
[host-neutral WorkIQ configuration](https://github.com/microsoft/work-iq/blob/main/plugins/workiq-preview/README.md)
and the registered `http://localhost:8080/callback` redirect. This experiment
is currently scoped to the Microsoft work tenant; it is not a general-purpose
production connector or an assumption that every tenant has granted consent.

```powershell
# Published resource and exact tenant issuer discovery only; no sign-in.
cargo run --locked --bin workiq-hosted-probe
# Explicit browser sign-in, account verification and one bounded calendar read.
cargo run --locked --bin workiq-hosted-probe -- --sign-in "<expected-work-email>"
```

The diagnostic uses S256 PKCE, fresh state, exact tenant metadata validation
and a loopback-only listener bound before opening the browser. Callback
method/path/Host/state/issuer and duplicate OAuth fields are validated. Browser
sign-in has a ten-minute deadline; discovery and tool reads are bounded.
Credentials remain in rmcp's memory-only store and are discarded on exit.
It never starts Copilot, registers an app, grants consent, invokes a model,
calls write tools or imports personal caches. A normal user sign-in may still
be blocked by tenant policy; it does not bypass Conditional Access.

After discovering and checking the live `fetch` schema, it verifies the
explicitly selected account before fetching at most one calendar event with
only `id`, `start` and `end`. The report contains counts and verification
status, not meeting contents, authorization codes or tokens. HTTP/tool errors,
wrong-account responses and unexpected output shapes are failures, never empty
successful reads. Exit codes are `0` for verified read, `2` for discovery-only
and `1` for failure. Neither this diagnostic nor its browser flow changes the
currently running native app.

The live Rust experiment verified browser authentication, the selected work
account, discovery of 12 tools and one calendar event through `fetch`, with
zero model or mutation calls. The successful run took 3,888 ms including sign-in;
this is one observation, not an application performance guarantee. Initial
authenticated initialization attempts failed before discovery. The successful
configuration explicitly pairs `serve` with `LATEST_WITH_INITIALIZE` and allows
optional server session IDs; rmcp's newer default protocol uses a different
lifecycle. Both settings changed together, so the experiment does not establish
which alone caused the earlier failures. Long-running token refresh and other
workloads still require separate live acceptance.

The bottom status bar has independent, clickable **AI** and **WorkIQ**
connections. WorkIQ's dialog asks for the expected work email, starts browser
sign-in and shows progress, cancellation, refresh and disconnect controls.
Authentication and reads run on Rust workers without starting Copilot or
reserving an AI request slot. With the owner's explicit approval, WorkIQ login
is stored in `workiq-login.db` encrypted by Windows-user-scoped DPAPI and bound
to the expected account, tenant and client. Tokens are never stored as plaintext
or passed to the SDK. Startup quietly restores both isolated AI and WorkIQ;
missing or rejected authentication requires an explicit Sign in, not an
unsolicited browser. Disconnect and forget sign-in removes the saved credential,
including when offline, and revokes late refresh saves. Exit retains encrypted
login. The last expected work email is
saved as a preference, with the cached calendar owner used for older installations.
A browser
tab already opened by sign-in is not closed automatically. Synthetic preview
never opens a browser or contacts either provider. Legacy per-workload metadata
diagnostics are command-line only, not part of the application UI.

After account verification, the app loads the visible calendar dates and
refreshes when the visible range changes. Complete snapshots are persisted by
the existing storage worker and published to the native calendar. The local
cache is account-owned (the owner email is saved with the calendar, not as an
authentication credential): refreshes preserve other cached dates for the same
account, and the first successful refresh for a different account replaces the
old account's cache. Disconnect and Exit retain calendar data.
Disconnect or a lost WorkIQ connection invalidates cached AI answers across
contexts; unsent questions remain available.
Reads cover at most 42 local days, 128 requests, 5,000 unique events and a
16 MiB normalized snapshot, with a two-minute operation deadline. Meetings
support up to 500 attendees. WorkIQ blocks Graph pagination tokens, so the
loader partitions time ranges for both pagination and oversized structured/text
responses, and refuses incomplete or inconsistent coverage
rather than publishing a partial page. Errors retain the previous cache.

Interactive AI requests can use the app's **tomlook_workiq_fetch** custom tool.
Only bounded reads of the authenticated user's mail and calendar are allowed:
eight reads per request, 30 seconds per read and 64,000 UTF-8 bytes per model-tool result.
The SDK receives results, never WorkIQ credentials. Native read arguments and
permissions are checked; managed approval requirements are not auto-approved.
Legacy WorkIQ servers are excluded from the native app's runtime configuration,
even when disconnected, avoiding runtime OAuth or legacy credential resolution.
Both interactive meeting questions and background preparation now use this
connection. They require the cached calendar's account, and every meeting
request retrieves broader context before model submission. The native
`tomlook_workiq_context` tool uses hosted WorkIQ's built-in `ask` to search
email history, Teams, previous meetings, SharePoint and OneDrive. The app
exposes no mutation tools or alternate agent IDs; read-only instructions are
also supplied to the hosted service, but are not a proof about its internal
agent implementation. Fetch and context share the eight-read budget, including
the initial context lookup. Context reads have a 120-second deadline; the HTTP
transport timeout is 135 seconds so it cannot prematurely terminate a valid
context read. The advertised `ask` question schema and returned `answer` field
are validated. Missing context is an explicit error. Request rejection, timeout
and response-processing failures do not by themselves discard sign-in; a closed
or failed transport and an explicit HTTP 401 invalidate connection readiness.
Preparation identities include the calendar owner, document folders, repository
collections and the optional customer OneDrive folder.

Tomlook reserves one read slot for calendar synchronization and one
for research/photos. Research cannot occupy the calendar slot; admission wait
is bounded separately and does not consume the actual provider-read deadline.
Context calls have a 120-second provider deadline and request at most eight
concise original sources. Answer-context setup, including admission, is bounded
to five minutes. Model generation is bounded to five minutes for Insights and
two minutes for interactive answers; cancellation remains available throughout.
Before the owner-approved deployments described below, the reserved-slot
follow-up was staged. Its noninteractive context diagnostic returned a verified response
in 59,248 ms (5,787 encoded bytes) using the saved login. This establishes one
native read, not a completed SDK Insight or a latency guarantee. The earlier
release restored both connections without browser interaction and durably
expanded the calendar from 47 to 294 meetings across 84 synchronized days.
Its live overlapping research still timed out, so automatic Insights were paused
pending the repair. The later product deployment supersedes that paused state.
Calendar synchronization and local indexing remained active.
Opt-in, windowless checks have subsequently completed real Insights requests
through the production engine and persisted their detailed results privately,
without changing that production ledger. The latest repaired run took 186 seconds
and completed three native context reads. Two of its six model source links
match bounded captured links/excerpts; this is not independent claim verification.
The diagnostic did not attach the local index or establish targeted customer-folder
coverage. This is not a completion displayed in the running product.

Context answers that fit the model limit are now retained when oversized
supporting metadata would otherwise reject the whole result. Metadata omission
is explicit, answers are never silently shortened, and an oversized answer still
fails with a narrower-query instruction. Original inline Markdown source links
from hosted WorkIQ answers are captured alongside safe structured links, including
links beyond the bounded excerpt. Link, traversal and credential guards remain.
Live checking also exposed a closed-boundary calendar response: an event ending
exactly at the requested start was stopping the January backfill. Native
calendar windows now exclude only those touching edges; genuinely out-of-range
events still reject the batch. A read-only ignored library diagnostic can observe
the production partitioning without publishing coverage or altering the cache.
The repaired January diagnostic validated 42 days and 64 events through three
bounded reads. It did not save them to the product calendar.

Insights instructions specify the exact JSON example and source keys `title`
and `url`, avoiding the observed model output field `original url`. If the
mandatory broad context search times out while the verified connection remains
active, its failure is captured and that session can use targeted native
mail/calendar reads. The broad tool is disabled for that request, missing
workload coverage must be stated, and completion still requires an actual
successful workplace read. Authentication, schema and other failures do not
take this path.
That earlier offline suite and production release build passed. The normal
`tomlook` release was preserved privately and matched all 37 bound Rust/build
inputs after compilation. It was initially staged without launch approval;
the subsequent cloud/calendar deployment below supersedes it. The targeted-read
timeout continuation has offline coverage, not a successful live-branch check.

`workiq-context-probe` is a bounded, noninteractive saved-login diagnostic. It
retrieves workplace context, closes the connection, and prints only outcome,
duration and encoded result size, never credentials or response contents:

```powershell
cargo run --locked --offline --bin workiq-context-probe -- "$env:LOCALAPPDATA\Tomlook" "your-account@example.com"
```

The ignored `insights_live` integration test exercises the production AI engine
and the same evidence validator used by the storage worker. It requires
`TOMLOOK_LIVE_INSIGHTS_ROOT` (the authorized app-owned state),
`TOMLOOK_LIVE_INSIGHTS_MEETING` (one cached meeting ID), and
`TOMLOOK_LIVE_INSIGHTS_OUTPUT` (a new private JSON file). It restores existing
logins without opening sign-in, performs one real read-only Insights request,
checks the detailed arrays and captured workplace evidence, shuts down its owned
executor, and writes/reopens the private result:

```powershell
cargo test --locked --offline --test insights_live -- --ignored --nocapture
```

This opt-in check does not refresh the calendar, alter the production job ledger,
attach the local source index, or launch a window. A persisted diagnostic is not
a product UI completion, local-source acceptance, or independent claim check.
Failed/unvalidated responses are retained in a separate private `.raw.json`
artifact, never published as completed Insights. Default test runs ignore it
and never access live accounts.

The ignored library check `live_indexed_insight_uses_returned_local_files`
uses the same production answer function with a private consistent snapshot of
the authorized source cache. Set the three Insights variables above and
`TOMLOOK_LIVE_SOURCE_SNAPSHOT` to a separate private directory containing
`source-index.db`, copied through SQLite's backup API rather than copying an
active database/WAL independently. It never refreshes or mutates the product
index. It requires an actual successful local file read and a returned path
used in actionable content, not merely tools offered to the model:

```powershell
cargo test --locked --offline --lib live_indexed_insight_uses_returned_local_files -- --ignored --nocapture
```

Successful local-read evidence preserves root, folder, path and offset before
the bounded content excerpt, so long document bodies cannot hide provenance.
The complete tool result is unchanged; record, excerpt and link bounds remain.
This diagnostic still does not establish product UI completion, full source
coverage or independent source-to-claim correctness.

Native mail/calendar reads also retain their validated relative request resource
outside the bounded excerpt. A large message body cannot erase the requested
message identity, and returned content cannot supply or replace that identity.
The source inspector distinguishes a message/meeting read, collection results
and a workplace search summary; a summary or cited URL is not original-content
verification. Failed/invalid reads cannot supply supporting citation records.

Saved researched Insights bind each model citation to the successful captured
records containing that exact URL. Uncaptured citations are preserved and
visibly marked, not silently dropped or presented as verified. Older saved
Insights remain readable with unverified citations. The compact overview reports
link coverage; expanded sources show capture gaps, retrieval time, request
resource and excerpt limitations. Citation URLs use the same credential-safe
filter as tool evidence, including when displaying older cached results.
Prompt changes invalidate the analysis revision normally; no source-content or
factual-claim acceptance is inferred from matching a URL.

An indexed live check exposed another latency defect: the model received only
the 4,000-byte diagnostic excerpt of the mandatory workplace prefetch, repeated
broad searches and exhausted its answer deadline despite successful local reads.
The model now receives the complete bounded prefetch when it fits the remaining
64,000-byte prompt budget. Oversized input falls back explicitly to the bounded
excerpt; the evidence snapshot remains small. Instructions reuse the supplied
research before requesting tools for concrete gaps. Answer deadlines and tool
budgets are unchanged.
The subsequent check completed, but searched only customer/title terms and
missed reusable technical material present in the same cache. Instructions
now require topic-based discovery as well, reading promising matches before
recommendation and separating reusable demos from actual customer history.
An offered local tool or a successful empty name search is not proof that no
useful demo exists.
The repaired indexed check then completed in 99,349 ms, with four successful
searches and four actual reads from three authorized repositories. Returned
paths appeared in actionable recommendations, provenance survived capture,
and private persistence/reopen and owned connection cleanup passed. These are
documented reuse suggestions, not proof of customer adoption or fresh demo
execution. That diagnostic did not establish targeted customer OneDrive coverage
or a job completion displayed by the product.

Configured customer OneDrive folders now have a direct native read tool in
addition to semantic WorkIQ research. The selected folder is resolved within
the verified account's own drive, using either a relative folder name or its
original personal OneDrive URL. Fixed path construction and returned drive/
parent identities prevent traversal and following external shortcuts. Lists
contain at most 100 direct children, explicitly reporting incomplete coverage;
provider-blocked skip/skiptoken pagination is never bypassed. Only safe original
URLs and selected metadata reach the model, never preauthenticated download
links. UTF-8 HTML, Markdown and text files are capped at 256 KiB and returned in
8,000-byte text chunks. HTML extraction runs off the UI thread and never executes
scripts. Every underlying metadata/content call consumes the existing shared
eight-read workplace budget, including the initial own-drive lookup.

A live semantic check returned a readable copy outside the selected folder and
correctly failed targeted-folder acceptance. Direct reads verified the actual
original HTML. ID-addressed children calls returned errors, while the documented
path-addressed alias succeeded without different credentials or broader access.
The complete bounded pipeline then listed 40 root children and five children of
the relevant subfolder and read the original document in 4,848 ms using seven
provider reads. It did not recursively index the folder or modify product state.

An initial real SDK check exhausted the read budget on repeated semantic calls
before reading cloud content. The configured root catalog is now prefetched
before the broad workplace search, retained with the complete prompt context,
including when that search times out, and its provenance recorded independently.
Timeout continuation retains explicit coverage gaps and the existing prompt
budget; this branch has offline regression coverage, not live acceptance.
Cloud catalogs and workplace history use separate prefetched payloads. A large
workplace answer can therefore fall back to its explicitly labeled excerpt
without also dropping later cloud catalog entries. Neither payload may exceed
the existing shared prompt budget; individual oversized catalogs remain partial.
Instructions prioritize a relevant
subfolder and file before further semantic searches. A subsequent real meeting
Insights request completed in 109,485 ms, read three local files and the original
cloud HTML, and cited the returned cloud URL in actionable recommendations.
Private persistence/reopen, production validation and owned SDK/WorkIQ cleanup
passed. This windowless diagnostic is separate from the subsequent fresh
product completion; recursive cloud coverage and independent workplace-claim
verification remain open.

The cloud-enabled normal product was subsequently deployed with owner approval.
Quiet saved AI/WorkIQ login restoration and a real **Research now** completion
were observed in the native UI. The product wrote and reloaded a new briefing
version through its own queue/storage path; the short overview and expanded
Insights show actual local-document and original OneDrive evidence. No diagnostic
result was injected into the product ledger. This establishes one durable
product completion, not full claim verification or all-card acceptance.

That live run also exposed a valid 604-participant meeting blocking a calendar
range. Calendar projection now keeps at most 500 participant records while
preserving the full count and a full-list fingerprint for change detection.
The People panel labels the limited list, and compact AI context uses the actual
total. Malformed oversized participant fields still fail validation. The
existing provider-response and calendar-read limits are unchanged; a large
participant list no longer discards the meeting or blocks a complete range.
An ignored saved-login check then read the formerly blocked 29-day range in
39,696 ms: 195 events, all 29 days, including seven large meeting instances
with 571-610 participants. Full counts/list fingerprints and the retained
500-person bound matched, and owned connection cleanup passed. This diagnostic
did not update the production calendar.

The owner subsequently approved deployment of the normal repaired executable,
which launched once and restored both saved connections without sign-in prompts.
The product itself completed the previously blocked backfill: all 213 days from
10 September 2026 through 10 April 2027 were synchronized, with seven large
meeting instances durably retaining 500 people, full counts and fingerprints.
The native People panel displayed `People (500 of 604)` and the limited-list
notice. The earlier foreground Insight survived restart with its exact payload
unchanged and remained readable in compact and expanded views.

Automatic Insights were resumed through the product activity panel. A genuine
automatic job completed with local-document and configured-folder cloud reads,
was independently reopened from SQLite, and appeared in the native Insights UI;
the next automatic job was already researching. Calendar navigation remained
usable during that work. This establishes one automatic completion, not a
30-minute guarantee for the remaining queue, hidden-tray acceptance, measured
latency/resource limits, or completion of all goal cards.

To repeat that bounded diagnostic, `calendar_live` requires
`TOMLOOK_LIVE_INSIGHTS_ROOT`, `TOMLOOK_LIVE_WORK_ACCOUNT`,
`TOMLOOK_LIVE_CALENDAR_FIRST`, `TOMLOOK_LIVE_CALENDAR_LAST`, and a new private
`TOMLOOK_LIVE_CALENDAR_OUTPUT`. It never signs in interactively, performs model
requests, or writes product cache/ledger state.

The ignored `cloud_context_live` checks use app-owned saved login and independently
verified private URLs/content markers supplied through environment variables.
The direct pipeline requires `TOMLOOK_LIVE_INSIGHTS_ROOT`,
`TOMLOOK_LIVE_WORK_ACCOUNT`, `TOMLOOK_LIVE_CLOUD_FOLDER_URL`,
`TOMLOOK_LIVE_CLOUD_FILE_PATH`, `TOMLOOK_LIVE_CLOUD_FILE_URL`,
`TOMLOOK_LIVE_CLOUD_CONTENT_MARKER`, `TOMLOOK_LIVE_CLOUD_CHILD_COUNT`, and a new
private `TOMLOOK_LIVE_CLOUD_OUTPUT`. The semantic check intentionally fails when
only out-of-folder copies are found. The indexed SDK check can additionally
require actual original-cloud-content use by setting
`TOMLOOK_LIVE_EXPECT_CLOUD_CONTENT_MARKER`; it never relaxes local-read acceptance.
Default offline tests do not use accounts or access the network.

**Research now** explicitly prioritizes the selected meeting, including past
cached meetings with verified coverage, even when automatic preparation is
disabled or paused. It yields an automatic preparation if both preparation
slots are occupied, waits for cancellation to free execution capacity, and
never publishes the yielded request's stale result. Merely selecting a meeting
does not start AI work. Research can queue before connection finishes; provider
readiness still gates actual dispatch. Interactive questions are not cancelled
to free a slot. New profiles enable automatic Insights; saved pause/enable
choices are retained.

Local detail reads stay on the storage worker, but consecutive reads no longer
clone and serialize the complete preparation queue between requests. The read
loop yields for completed provider results, lifecycle changes and its existing
deadline. A controlled 5,000-event regression fills both provider slots, requests
80 details, checks cancellation before manual dispatch, rejects the yielded
answer, and saves/reloads a current result larger than 48 KiB while another
request remains running. Malformed saved details report an error and retain the
failed ledger rather than returning an empty successful result.

An offline native UI regression also holds the provider progress lock while
editing, searching, navigating and cancelling across 5,000 cached events and
200 coalesced notifications. All eight light/dark and accent combinations have
automated body/secondary/link contrast checks of at least 4.5:1 and focus checks
of at least 3:1. These checks establish local boundaries and palette contrast,
not physical-input/GPU latency, a real-provider workload, DPI/layout acceptance,
or owner approval.

Meeting **Insights** have a short overview and a larger, resizable detail window
with preparation actions, talking points, open questions, risks, gaps and original
source links. Successful workplace reads and bounded captured tool excerpts are
required for a new live research completion; model interpretation is not claimed
to be independently verified. Detailed fields and tool evidence survive restart.
The single status bar opens a virtualized activity list showing the actual
waiting/running/ready/failed meetings; selecting a row opens that meeting.
Its **All**, **Working**, **Waiting**, **Problems** and **Ready** filters cover the
entire ledger, including deferred jobs. **Problems** includes interrupted work,
and the status bar counts both failed and interrupted jobs as needing review.
Each virtualized row has a fixed height and a truncated single-line title with
the full title/error on hover, so long or multiline names cannot break scrolling.
Event-owned widget identities survive reordering/filtering. Keyboard activation
opens saved detail only; inspecting a failure does not retry or submit AI.
Activity ordering and filter indexes are built by the storage worker and reused
when the queue is unchanged. Calendar identity lookup is built with the calendar
snapshot; painting the activity panel does not scan or sort either full corpus.
The redundant Settings button is gone. Appearance remains in the header;
pause, automatic Insights and collapsed source controls live in Insights activity.

People appear as company groups with two-letter initials and full name/email/
domain on hover. Microsoft is distinguished from external domains; the full
virtualized roster is collapsed by default. Company icons are fetched lazily
from the company's own HTTPS favicon endpoint (never a tracking/logo service),
without sending email addresses or meeting data. Public-address checks,
no redirects, bounded downloads/decoding and an app-owned icon cache apply.
Unavailable icons retain a text label with an explicit hover reason.
Only visible expanded-roster attendees request photos,
in bounded batches, through `fetch_blob`; unavailable photos use initials and
errors remain available on hover. Images are bounded, decoded off the UI thread
and resized to 48 pixels. The memory-only cache holds at most 128 photos and
clears on disconnect. No photo tokens or image files are persisted.

Insights activity accepts up to eight specific **Local document folders** and
four **Repository collections**, empty by default. A collection may be a drive
root: only discovered Git repositories become readable sources, not arbitrary
drive files. Discovery repeats every five minutes and includes newly added repos.
Hidden/system directories, links, dependency and build trees are excluded.
Discovery is bounded to 30,000 entries, eight levels and ten seconds per collection;
partial coverage and access failures are reported, never presented as a complete
inventory. Every discovered root receives a separate indexing budget rather than
letting the first repository consume the whole pass.

The Rust background worker maintains an app-owned SQLite full-text index
(`source-index.db`) with paths and the first 16,000 UTF-8 bytes of readable
document text. Timestamp/size checks reuse unchanged entries; changed and removed
documents update the index. Each root pass is bounded to 10,000 entries, eight
levels, two seconds and 64 changed documents. Change-limited passes backfill
after five seconds; permanent traversal limits remain explicitly partial and
retry on the ordinary five-minute interval. The index has a 512 MiB page limit
and a 2 MiB SQLite page cache. It contains private text in the user's local app
state, not encrypted credentials; it is not a public/shareable artifact.

Meeting requests attempt a cached local search before model submission and
capture its actual results or failure alongside workplace evidence. Search
returns at most 40 ranked matches, bounded excerpts, index time and coverage.
Eight local operations per answer include the initial search; tool calls have
a five-second deadline. Reads revalidate the current opened file and return
8,000-byte UTF-8-safe chunks with `next_offset`. UTF-8 raw documents are limited
to 1 MiB. HTML slides are reduced to readable text without executing scripts;
head/style/script/SVG boilerplate is excluded. Binary PDF/Office decoding is not
implemented. Traversal, hidden/sensitive files, credential fields, recognizable
credential tokens, symlinks and junctions are rejected, and the opened Windows
file handle must remain inside its approved root.

**Customer folder in OneDrive** is a cloud-research hint passed as data to
WorkIQ, not authorization to hydrate a local cloud tree. Customer materials
and cloud-only HTML remain on the authenticated WorkIQ route with original
source links. Local indexing never follows OneDrive reparse points and does
not implicitly download them. GitHub remains an optional remote source; local
repositories provide current, including uncommitted, content without requiring
a second credential flow.

The non-model `source-index` diagnostic can configure explicitly authorized
collections and verify bounded search, without printing document contents:

```powershell
cargo run --locked --offline --bin source-index -- "$env:LOCALAPPDATA\Tomlook" --repositories "D:\" --cloud-folder "CustomerFolder" --query "demo topic"
```

Run configuration changes while Tomlook is exited so its in-memory preferences
cannot overwrite them. Existing document-folder preferences migrate without
granting any new collection scope.

### WebIQ and GitHub sign-in

The bottom status bar uses frameless clickable text for AI, WorkIQ, WebIQ,
GitHub and activity. Hover underlines and keyboard-focus outlines preserve
discoverability without permanent gray button boxes. The neutral inbox bell
also remains clickable without a filled box.
The connector text opens app-owned WebIQ and GitHub connection dialogs.
Enter the WebIQ key in its password field and choose **Connect and save key**.
Verification performs one small public search for `Microsoft`; no workplace
content or model request is sent. The editor is cleared after submission or
closing the dialog, and the key is saved only after successful verification.

The GitHub dialog shows only connection status, the remembered public Client ID
and one **Sign in** button. During device authorization, the status includes
the required URL/code and the same action becomes **Cancel**. Once connected,
sign-in is disabled. Provider failure details remain in Messages and the status
tooltip, not additional dialog controls. A valid Client ID is saved separately
from credentials when submitted or its editor loses focus; an expired legacy
vault record can also restore its Client ID without renewing or authorizing it.
Existing encrypted personal-token credentials remain supported on restoration,
but the dialog no longer offers token entry or GitHub registration links.

Register a dedicated [GitHub App](https://github.com/settings/apps/new).
Enable **Device flow**, grant repository **Contents**, **Issues** and
**Pull requests** read-only permission, and install it only on approved
repositories. Enter its public **Client ID**, not its App ID or client secret,
in Tomlook and choose **Sign in**. The browser shows GitHub's device page;
enter the code displayed by Tomlook and approve the selected identity.
Account verification and a read-only MCP identity read precede saving.
GitHub or organization approval/installation errors remain explicit.
No registration, installation or repository grant is performed automatically.

The fixed GitHub endpoint is `https://api.githubcopilot.com/mcp/`, with
`X-MCP-Readonly: true`, the `context,repos,issues,pull_requests` toolsets and an explicit
read-tool allowlist. The `context` toolset supplies `get_me` for account verification;
the same bounded configuration is used for verification and SDK sessions. Selecting
this toolset does not grant new GitHub App permissions or expose other context tools
to the assistant. The WebIQ endpoint is `https://api.microsoft.ai/v3/mcp`,
using `x-apikey`. Saved credentials reside in `connector-login.db`, encrypted
with current-user Windows DPAPI and separate provider/endpoint bindings.
Neither personal `gh` credentials nor `.mcp.json` are imported.
The native app ignores configured/environment WebIQ and GitHub credentials;
standalone connector diagnostics retain their explicit `TOMLOOK_*` setup.

OAuth requires a registered client identity. Copilot supplies its own registered
client; Tomlook cannot borrow that identity or its saved credentials. Registering
a dedicated Tomlook client is application-author setup, not inherently a task
for every end user. The [GitHub MCP host integration guide](https://github.com/github/github-mcp-server/blob/main/docs/host-integration.md)
also supports a personal access token without custom app registration.
The native credential adapter retains compatibility with previously saved
personal tokens; the simplified dialog uses GitHub App Device flow only.

Startup quietly revalidates saved connections without opening a browser.
GitHub device-flow credentials retain both access and refresh tokens in the
DPAPI vault. The async executor renews them about five minutes before access
expiry, and on restoration after expiry. Device flow needs no client secret
for renewal. Rotation commits the encrypted replacement pair before cancellable
MCP verification, so a verification failure does not lose the new refresh token.
Transient renewal failures retain saved credentials and retry no sooner than
five minutes later. Revocation or refresh-token expiry still requires sign-in.
Old vault records remain readable but did not retain refresh tokens: their owner
must sign in once with this version to enable renewal.
Personal-token expiry/revocation is enforced by GitHub and detected on saved-login
verification or subsequent provider reads; Tomlook does not know its configured
expiry from the token text. It does not embed an app client secret or claim
perpetual refresh. Account/app changes, removal and personal-token replacement
cancel old research and change its ledger configuration identity. Routine
device-token renewal for the same verified account/app preserves context
identity and does not invalidate cached Insights;
late answers cannot be saved under another connection's identity.
WebIQ **Disconnect and forget** requires a second confirmation and revokes
pending vault saves. Cancellation cannot undo a sign-in already successfully
committed. The simplified GitHub dialog does not expose credential removal;
provider access can be revoked in GitHub settings. Calendar/Insights caches
are retained.
Discovery and one verification read establish connectivity, not access to every
repository or independent claim correctness. Real WebIQ/GitHub acceptance still
requires the owner's credentials, selected repository and source comparison.
Verification explicitly negotiates an `initialize`-compatible MCP revision and
supports stateless HTTP providers. Tool errors, structured provider errors,
empty/oversized reads and conflicting GitHub account identities fail before
saving. Local HTTP fixtures exercise the actual MCP transport and verify that
cancel/shutdown closes pending discovery and read requests. These fixtures
send no remote queries, model requests or browser actions.
Verification reports safe HTTP status and MCP error categories rather than
provider bodies, URLs or credentials. HTTP 401/403, rate limits, server errors
and invalid tool arguments are distinct. The MCP HTTP deadline matches the
bounded 60-second discovery/read operation; initialization remains bounded to
30 seconds and cleanup to five seconds. Local HTTP fixtures assert the
production `x-apikey` header on every WebIQ request. The observed live failure
occurred during the verification read, after discovery; these diagnostic
changes do not establish its cause or prove a successful owner connection.
Saved assistant answers are invalidated when connections change or are removed;
unsent questions are retained. Token and device-flow lifetimes
are measured from receipt, not extended by subsequent account/browser checks.
Cancellation is retained even after the AI task finishes but before its result
is published. Both questions and Insights recheck the active request,
cancellation, connection identity and shutdown state before delivery.
Reconnecting the same credentials cannot revive a cancelled old result.
Slot availability and provider results refresh queue eligibility without
reserializing unchanged calendar events. Calendar/configuration changes still
recompute fingerprints; time-based expiry, renewal, coverage and admission
checks are unchanged.

Outlook category names are preserved in stored events, but filters deduplicate
case variants. A tentative response adds `nejisté` only when that category is
absent. The owner's six categories use familiar ochre, brown, purple, green,
gray and red families, adjusted for readable light/dark labels and small
calendar stripes. Normalized filter keys are built on the calendar worker,
not allocated across the full calendar during repaint.

Live acceptance through the release UI loaded 39 events for October 5-9 into
the account-owned cache and refreshed the same range without another sign-in.
All 39 cached events also survived a final release-app restart without restoring WorkIQ
credentials. The isolated SDK connected independently without a model request.
Disconnecting WorkIQ retained all 39 cached events and left AI connected.
The custom SDK handler is exercised offline with synthetic tool invocations:
success/failure wire results, evidence deduplication, refusal before fetching,
failed reads consuming the eight-read budget, and the exact 64,000-byte UTF-8
result boundary. These checks make no model or workplace calls. A model-triggered WorkIQ read and
the new experimental GitHub browser-login path have not yet been live accepted.

The pinned official Rust `github-copilot-sdk` is an adapter to an installed
Copilot runtime, not a Rust rewrite of that proprietary runtime. It starts
asynchronously at normal startup or after explicit **Sign in**. Automatic connection uses only
the existing isolated identity and never opens a sign-in browser.
Browser sign-in uses the SDK's experimental accounts API with no terminal,
model request or automatically approved plaintext-credential consent. An
unsupported runtime API or provider interaction is an explicit failure, not
a fallback to personal credentials. The native application uses a separate SDK
executor, bounded request lanes/channels and coalesced notices and
streaming. Calendar navigation does not wait for a model or briefing.
Ctrl+Space opens the assistant, Enter submits a question from its editor,
Shift+Enter inserts a newline, Ctrl+Shift+P opens searchable commands,
and Escape dismisses command suggestions or hides it without cancellation.
Only **Stop** requests cancellation. Global questions and captured
meeting context are separate; changing a meeting invalidates its old answer.
Drafts and up to 32 conversation turns are retained separately
for up to 32 contexts in memory. Configured WebIQ and WorkIQ tools are offered
without a public-topic editor. Public search rejects private identifiers and
app credentials; prompts prohibit sharing workplace or local-file contents.
A changed meeting snapshot invalidates its old
answer; reopening the same snapshot does not cancel an ongoing question.
At the memory limit, a new context is refused visibly instead of evicting
drafts or submitting against the old meeting. Exit clears this state.
Ordinary chat is memory-only. Each independent isolated SDK request receives
the bounded completed conversation history (48 KiB); running/failed turns are
not silently treated as completed context. Large recent answers use an explicitly
labelled excerpt. Full claim-to-passage verification still requires source review.
Opening the panel requests editor focus; closing it returns focus to search.
Question/topic limits are checked as UTF-8 bytes before submission, request
identities cannot collide with preparation, and an unadmitted question
preserves the previous answer. Cancellation reports an uncertain provider
outcome rather than claiming the operation definitely stopped.

Ask contains only questions, formatted answers and expandable retrieved sources.
The former local draft editor and optional typed-proposal mode are not exposed.
Cloud question shortcuts expand immediately before admission; local commands
instead share the typed cached-calendar algorithms used by model tools.
The two ad-hoc review commands instead enqueue typed background work; they never
ask the model to execute a slash string or expose calendar-mutation tools.
The separate window uses a deferred viewport: typing repaints that window, not
the calendar. Its bounded action channel and shared view use nonblocking access;
final notices update the chat even when the main window is hidden. Animation
repaints only a visible Ask surface; streaming preparation runs on the executor.

### Ask profiles, cache and durable research

**Quick** selects `gpt-6-luna` with `low` reasoning: 60 seconds of execution,
four remote reads and twelve combined local reads. **Thorough** selects
`gpt-6.1-sol` / `medium`: ten minutes, forty remote reads and eighty local reads.
Session setup has a separate thirty-second bound. **Auto** calls
Microsoft-Decision-1's typed `choice` API directly, without creating another
Copilot session. A fixed rubric and labeled examples distinguish direct retrieval
of explicit facts (including a month's calendar listing or reading a named
README) from discovering relationships, commitments, preparation requirements
or the user's presenter-versus-listener role. A short question about one demo
can need Thorough: finding which meeting needs it requires evidence of the
connection, not a similar calendar title. Extracting an already established
finding from recent context remains Quick; resolving a previously uncertain
connection is research. Brevity, date range and the word "demo" alone do not
determine the profile. Only `fast` and `deep` with valid normalized probabilities are
accepted, mapping to Luna/low and Sol/medium respectively. Recent context is
bounded to two turns; context is data, never routing instructions.
Routing and answering share one admitted daily request; Foundry inference is
billed separately from Copilot and is not a Copilot billing multiplier. Routing
latency is recorded without question contents. Choosing Quick or Thorough
explicitly avoids this extra network call. Missing model IDs/efforts and
invalid routing are explicit errors, never silent model substitutions.
The existing `connections.json` `model` setting still controls legacy meeting
preparation, not these fixed Ask profiles.

Auto additionally requires app-owned `decision-router.json`; copy
`decision-router.example.json` beside `connections.json` and set the actual
Foundry resource endpoint, deployment name, existing Azure CLI executable,
tenant and subscription. The deployment name is independent of the model name
(for example `mai-decision-1` for the model `Microsoft-Decision-1`).
Only credential-free HTTPS Azure Foundry resource endpoints are accepted;
redirects are disabled. No API key or bearer token is stored in this file.
The router lazily obtains a Cognitive Services Entra token from the explicitly
configured existing Azure CLI sign-in and retains it in memory until renewal.
It never signs in, creates a deployment or changes grants on the user's behalf.
On Windows MSI installations it runs the CLI's bundled isolated Python module
directly, avoiding an intermediate shell or orphaned CLI child on cancellation.
Token acquisition and inference each have an eight-second bound; requests are
asynchronous, cancellable and response-size limited. Missing/expired sign-in,
HTTP failures, timeouts and invalid choices are visible errors; manual profiles
and offline slash commands remain usable. Restart/reconnect after changing this
configuration. No Azure SDK dependency or new package version is required.

One Quick/Auto lane and one Thorough lane can run concurrently. Auto acquires
the Thorough lane before releasing its Quick lane. A running preparation can
coexist within the three-job executor cap; new preparation retains its existing
two-job admission and single-background-job safeguards. Local slash queries use
the separate storage worker.

Both cloud profiles see the same narrowly allowlisted reads: typed cached
calendar search/meeting/Insights, authorized indexed local search/text chunks,
native WorkIQ reads/context and configured public WebIQ/GitHub reads. A cache
tool opens a short-lived read-only SQLite snapshot, rechecks its account owner,
reports per-day coverage/refresh times and marks stale Insights. Search bounds,
pagination and unknown fields are validated in Rust. There is no arbitrary
SQL, shell or drive access. Cache and indexed-document reads share two blocking
read permits; a timed-out disk read retains its permit until it actually exits.
Host counters enforce aggregate local/remote budgets; prompt instructions prefer
relevant cached evidence rather than mandatory broad prefetches.
For natural-language calendar lookup, the model interprets intent, supplies
distinctive keywords and date bounds, inspects candidate metadata and
`search_match` evidence, rejects unrelated similar names, and then answers.
Each matched keyword reports its field, normalized token, match kind and edit
distance. This is retrieval evidence, not semantic identity or a probability.
Default ordering is relevance then time; `order: "start"` supports nearest
meeting requests without hiding earlier candidates behind stronger later name
matches. Empty queries are chronological. Alternative keyword searches and
bounded calendar inspection remain available before a negative conclusion.

Thorough jobs checkpoint question, bounded conversation/findings/evidence,
account, exact selected-meeting identity/fingerprint and connection/source
binding in private `ask-research.db` beside the application database. This file
can contain personal derived findings/source excerpts and follows the profile's
Windows-user filesystem access, not a claim of encrypted conversation storage.
At most eight unfinished jobs are retained; they are never silently evicted.
Completed checkpoints are pruned on later saves. Interrupted jobs appear in Ask
with **Continue research** and a separately confirmed **Discard saved research**.
Continuation is a new model request using historical findings, not resumption
of a disconnected provider call. Changed accounts, meetings or source bindings
require a new question; discard provides a repair path for incompatible jobs.
Only bounded evidence excerpts are saved; they are not a complete source archive.
Stop and timeouts abort/detach the owned session, retain partial findings
honestly and never report an unfinished answer as success.

The ignored `decision_live` tests use `TOMLOOK_LIVE_CHAT_ROOT` and a new private
`TOMLOOK_LIVE_DECISION_OUTPUT` receipt per run. Select one test at a time with
`--exact --ignored`: `decision_routes_regressions_and_measures_warm_latency`
exercises the real configured router on forty development/regression intents,
balanced between twenty Quick and twenty Thorough cases. These include the
reported spoken demo-to-meeting regression, direct
lookups, implicit commitments, attendee roles, selected-meeting context and
verified-versus-unverified follow-ups. Test formulations are not included in the
router's examples. `decision_routes_post_freeze_holdouts` checks twelve separate
balanced formulations authored after freezing the repaired rubric, without
tuning on their outcomes. Each receipt records false-Quick and false-Thorough counts
separately, plus warm median and p95 latency; all cases must classify correctly
and the warm median must stay below three seconds. Requests are paced by 1.2
seconds outside the measured routing interval to respect deployment rate limits.
These are bounded labeled sets, not a general classifier-accuracy claim.
The tests do not provision resources, change authentication, read mail or
calendar data, or execute the questions; only the labeled requests and synthetic
context are classified.

Use `connections.example.json` as a schema example, replace the placeholder
runtime/endpoints, and save an app-owned `connections.json` in the `--state`
directory (default `%LOCALAPPDATA%\Tomlook`). It is **not** personal Copilot
configuration. Native WebIQ/GitHub sign-ins use the status bar, not environment
variables. Standalone diagnostics still require `TOMLOOK_*` names for explicitly
configured credential variables. Keep real private configuration outside the repository.
Do not copy a personal profile, token store or `.mcp.json` into this profile.
An empty `servers` object can check Copilot identity without connectors.

The runtime child receives `COPILOT_HOME=<state>\copilot` and
`<state>\workspace` as its working directory. Ambient GitHub token variables
and default SDK connection settings are removed for that child; the parent
environment is unchanged. Empty client mode, disabled configuration discovery,
disabled remote sessions/export, exact read-only MCP allowlists, hooks and
fail-closed permissions prevent ambient personal tools from being adopted.
Configured public web search is available without a separate topic field;
private identifiers, credentials and extra context arguments are rejected.
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

**Isolated stored login (recommended).** Sign in once into Tomlook's own Copilot
home, with ambient tokens hidden and the shared keychain disabled:

```powershell
$env:COPILOT_HOME = "$env:LOCALAPPDATA\Tomlook\copilot"; $env:COPILOT_DISABLE_KEYTAR = '1'
Remove-Item Env:GH_TOKEN, Env:GITHUB_TOKEN, Env:COPILOT_GITHUB_TOKEN -ErrorAction SilentlyContinue
copilot login   # approve in the browser; answer y to store in this app-owned folder
```

Then set `"use_stored_login": true` in `connections.json`. Tomlook connects
only if the runtime reports the stored `user` identity; `gh`, environment or
other identity kinds are rejected and the runtime is stopped. The token is a
plain-text file under `%LOCALAPPDATA%\Tomlook\copilot`, readable by your Windows
account and separate from your regular Copilot profile. `use_stored_login` and
`copilot_credential_env` are mutually exclusive. Connect checks identity only;
it sends no model request.

For a no-model, no-connector handshake/session probe from this package directory
(add `--stored-login` to check the isolated stored login):

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
is six calendar months, including ongoing meetings, with 64 pending jobs, two
execution slots and at most one background preparation. Overflow remains durably
deferred. The latest opened eligible unfinished meeting takes foreground
precedence; unknown calendar coverage never becomes permission to prepare.

The SQLite worker owns the versioned queue ledger, admission and result
validation. Input/analysis fingerprints reject obsolete completions. Publishing
a validated result and its completed ledger is one transaction with exact
payload read-back. Model-provided citations are explicitly **not independently
verified**. Previous briefings survive failed or cancelled attempts.
The base harness freezes its model and static configuration. Each research
session captures an immutable app-owned connector registry and credential set.
A deterministic digest also includes prompt policy
and runtime path/size/modification identity; only the digest enters the ledger.
Changing static configuration takes effect after restart and a new isolated connection.
App-owned WebIQ/GitHub changes are overlaid on the next session immediately,
with an additional connection digest in the scheduler's configuration identity.
That connection invalidates old completions and rejects old in-flight results
before new dispatch, including when the window is hidden. Failed/interrupted
manual outcomes still require explicit retry. Automatic read-only failures retry
up to three times with backoff, then remain visibly failed without blocking other
meetings. Changes to meeting/configuration identity invalidate automatic results.
Expected cancellation during a requested shutdown, pause or context change is
recorded on the affected job, not as a global preparation failure. Automatic
work can resume; canceled manual work stays interrupted until explicit retry.
Unexpected executor failure, unknown crash outcomes and persistence errors still
stop dispatch visibly. The exact legacy pre-send shutdown banner is repaired on
load without deleting results or replaying interrupted manual work. Dismissing a
global error keeps it dismissed until the underlying error changes. Diagnostics
no longer take a full-width row away from the calendar: a keyboard-accessible
neutral bell next to the queue counts opens Messages in EN/CZ. Active
blocking errors remain available after dismissal; Messages also links to the
interrupted-job view and explicit resumption of unrelated queued work.
Outcome delivery waits asynchronously for bounded storage capacity rather than
stopping the executor on a temporarily full channel. Connector identity updates
are coalesced and retried without blocking UI or stopping the executor.
Storage observes executor termination before draining delivered outcomes;
a result arriving concurrently with termination is not prematurely classified
as an unknown outcome. Genuine missing outcomes still require explicit review.
Upcoming automatically prepared briefings are refreshed after 24 hours within
the nearest 14 days; legacy upcoming results without a recorded age refresh once.
Distant briefings are reused until their meeting/configuration changes. One-off
manual preparations are not turned into recurring requests.
Legacy ledgers remain readable and are upgraded on connection. A ready SDK
without its configuration identity cannot admit preparation. No credentials or
resolved connector headers are written to the queue or briefings.
Minute deadlines renew eligibility; metadata requests do not rescan all event
fingerprints. Completed preparations reach storage even when no UI notices are
consumed. Storage notices and SDK connection/focused-answer notices are bounded
and coalesced, so a hidden window cannot block a worker on notification delivery.

After WorkIQ connects, the storage worker independently backfills one calendar
month behind and six months ahead, future first, in at most 42-day reads.
Visible-range requests take priority over the next background batch. SQLite
stores per-day successful-refresh timestamps atomically with the calendar;
restart resumes from those timestamps, including empty days. Days within two
weeks of today refresh every 15 minutes, others every six hours. Failed batches
preserve cache and retry with bounded exponential backoff.
Completed, account-verified batches are still saved if readiness changes before
storage receives them; connection-generation guards reject disconnected or
replaced-provider results before delivery.
The status bar shows
fresh days, target days, prepared/waiting/failed meetings and refresh details.
This work does not depend on painting the window and continues in the tray.
When automatic preparation is enabled and WorkIQ connects, the isolated AI
runtime connects lazily without opening a login browser. Missing AI credentials
are reported; use the existing sign-in controls. With explicit approval, WorkIQ
credentials are saved in the app-owned Windows DPAPI vault and restored silently.
Background synchronization does not initiate interactive sign-in.

Backfill, restart freshness, obsolete/partial responses and bounded AI retries
are exercised with injected provider responses without consuming UI notices.
The release UI retains the real cache and displays offline synchronization
progress. Full six-month live backfill and AI throughput still require an
authenticated run; completion of the entire AI queue within 30 minutes is not
guaranteed.

Closing/minimizing hides the window when its tray was created successfully.
Left-click the Focus tray icon or choose **Open Tomlook** to restore. Opening the
normal product again also requests the existing same-profile window, even when
it is hidden; it exits before opening storage or starting AI. The tray also
has pause/resume and **Exit Tomlook (stop AI)**. Insights activity and Commands expose
explicit Exit, with Ctrl+Shift+Q as its shortcut. Exit uses a priority SDK stop
signal, waits outside UI callbacks, then drains ready preparation results and
stops storage through an independent stop flag. Final persistence failures are
reported by the shutdown acknowledgement, not hidden in unconsumed UI notices.
Unavailable tray controls are disabled rather than silently doing nothing.
Windows exclusive profile ownership prevents a second instance
from starting another scheduler for the same state. Window restoration uses a
profile-scoped, session-local auto-reset Windows event with the default creator
ACL and non-inheritable handles. It carries no data or commands other than
restoring visibility. Its worker sleeps in a kernel wait, not a polling loop;
pending launches coalesce and the listener stops after the native event loop
closes. Only a secondary launch waits up to two seconds for primary startup.
Older binaries without this signal report the tray fallback rather than
starting another instance. Preview, measurement and diagnostic launches remain
exclusive and never reactivate the normal product.

Offline Windows tests exercise the actual event transport across two test
processes, pre-window coalescing, startup races, separate profiles, collision/
legacy errors, ownership retention and listener cleanup. An egui test checks
the same visible/unminimized/focus commands used by the tray. These tests do not
establish actual hidden-window presentation or a live AI child's exit.

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

## Startup measurement

`calendar::fixture` is the frozen benchmark corpus (`tomlook-frozen-fixture-v1`):
42 days from 5 October 2026, 200 or 5,000 events, with overlaps, all-day and
multi-day events, long Czech text and a 25 October DST-end event. Offsets are
fixed, so content does not depend on today's date. Seed a new, empty owned
state directory (existing state is refused):

```powershell
cargo run --locked --release --bin seed-fixture -- D:\TomlookBench\state-5000 5000
target\release\tomlook.exe --state D:\TomlookBench\state-5000 --legacy-root D:\TomlookBench\empty --startup-trace D:\TomlookBench\trace.json
```

`--startup-trace` records monotonic landmarks from `main`, the window/GL ready
point, worker/tray construction, calendar receipt and the start of the frame
after the first loaded-calendar frame. It then exits through the normal Exit
path and writes JSON only after the window closes, never from a UI callback.
Process creation time comes from the harness. It is opt-in and makes no AI calls.

The cached calendar now loads on a preload thread started before the native
window, overlapping SQLite read/parse/layout with window and OpenGL setup. The
open store is handed to the storage worker, which remains its only later user.
On the owner's laptop this cut the median 5,000-event wait after UI construction
from about 92 ms to 2 ms. Window/OpenGL creation (about 130-190 ms median, with
rare spikes inside that eframe stage above 800 ms shortly after resume from standby) and
pre-`main` process loading now dominate. These are trial observations, not the
full owner-approved startup matrix.
