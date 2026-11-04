# Privacy and safety

Tomlook is a single-user desktop app. It has no server of its own: data is
stored in your Windows profile and leaves the PC only in requests to the
services you connect. This page lists what is stored, which services are
contacted, what the AI may do and how every write is approved.

## What is stored locally

All files are in the profile directory, `%LOCALAPPDATA%\Tomlook` by default
(`--state` selects another one). They are protected by your Windows account's
file permissions; only the files marked DPAPI are additionally encrypted.

| File or folder | Contents |
|---|---|
| `tomlook.db` | Calendar cache (bounded meeting metadata and the 255-character invitation preview), per-day sync state, invitation queue and local priority/team caches, Insights and notes with their sources, the preparation queue, recommendations, approvals and write outcomes, scheduling proposals, tasks, follow-up drafts, feedback, customers and cached MSX reads, weekly summaries, preferences and edited prompts |
| `ask-research.db` | Checkpoints of interrupted Thorough research (findings and short source excerpts) |
| `source-index.db` | Full-text index of the first 16,000 bytes of documents in folders you added |
| `ai-usage.db` | Daily AI request counts and token usage |
| `diagnostics.jsonl` (rotated) | Reason codes, request IDs and HTTP status codes; never prompts, credentials, meeting titles or provider responses |
| `workiq-login.db` | WorkIQ sign-in, **DPAPI-encrypted**, bound to account, tenant and client |
| `connector-login.db` | WebIQ key and GitHub tokens, **DPAPI-encrypted**, bound to provider and endpoint |
| `connections.json`, `decision-router.json` | Copilot runtime and Foundry configuration; never secrets |
| `copilot\`, `workspace\` | The isolated Copilot CLI home (including its stored login file) and working directory |
| `company-icons\` | Cached company favicons |
| `notification-icon.png`, `instance.lock` | Notification icon and the single-instance lock |

Not stored on disk: ordinary Ask conversations (memory only, cleared on
exit), participant photos (memory only, at most 128), Azure and MSX tokens,
and WorkIQ access tokens. Full invitation bodies are never synchronized.
Feedback, tasks and customers are never sent anywhere automatically, and
Tomlook sends no usage telemetry.

Updates are staged next to `Tomlook.exe` in `.tomlook-update\` and the
previous executable is kept as `Tomlook.exe.old` until the next start.

## Credentials

- **WorkIQ, WebIQ, GitHub:** saved sign-ins are encrypted with Windows DPAPI
  for the current Windows user. Access tokens are used in memory and never
  passed to the AI runtime; the AI receives tool results only.
- **GitHub Copilot:** the Copilot CLI stores its login in Tomlook's isolated
  Copilot home (a file readable by your Windows account), separate from your
  regular Copilot profile. Ambient `GH_TOKEN`/`GITHUB_TOKEN`-style variables
  and `gh` logins are removed from the runtime's environment and rejected.
- **Azure CLI (Foundry, MSX):** tokens are requested from Azure CLI, checked
  against the configured tenant and kept in memory only. Azure CLI keeps its
  own sign-in cache.
- Personal `.mcp.json`, `gh` credentials and Copilot profiles are never
  imported. Never put keys or tokens into configuration files or chat.

## Network use

| Service | Endpoint | When |
|---|---|---|
| WorkIQ (Microsoft 365) | `workiq.svc.cloud.microsoft/mcp`, sign-in at `login.microsoftonline.com` | After sign-in: calendar and unanswered-invitation sync, selected team-group membership refresh, research reads, transcripts, free/busy, directory lookups, photos, and approved writes. Explicit team discovery reads bounded historical-mail recipient headers. Microsoft Graph operations are made through WorkIQ's tools. |
| GitHub Copilot | Through the installed Copilot CLI runtime | AI requests and Copilot sign-in |
| WebIQ | `api.microsoft.ai/v3/mcp` | Key verification and public web searches by the model |
| GitHub connector | `api.githubcopilot.com/mcp/` (read-only), `github.com/login/device`, `github.com/login/oauth/access_token`, `api.github.com/user` | Sign-in, token renewal and repository reads |
| Foundry | The configured `*.services.ai.azure.com` endpoint, token via Azure CLI | Only for Ask in **Auto** mode |
| MSX | `microsoftsales.crm.dynamics.com/api/mcp`, `api.fabric.microsoft.com/v1/mcp/powerbi`, tokens via Azure CLI | When MSX is enabled and signed in: customer timeline, preparation of customer meetings, Thorough Ask |
| Updates | `api.github.com/repos/tkubica12/ai-outlook/releases/latest`, downloads from `github.com` / `*.githubusercontent.com` | About a minute after start, then daily; unauthenticated, no user data |
| Company icons | `https://<domain>/favicon.ico` (or `www.<domain>`) | When an external company is shown; only the domain is sent, only public IP addresses, no redirects, bounded size |

**Join** links and `mailto:` drafts are opened by Windows in your browser,
Teams or mail app, not by Tomlook. The offline demo contacts no service.

AI requests to Copilot contain the question or task, bounded meeting metadata
and whatever the read tools returned (mail, Teams, document excerpts, MSX
rows). Invitation previews are not sent to the AI.

### Invitation priorities

The invitation queue, snoozes, organizer/series rules, manual team people and
exclusions, group IDs/names/mail addresses, transitive member mail/sign-in aliases,
refresh timestamps and errors are stored only in this profile, separately per
calendar account. They are not encrypted beyond the profile's file permissions.
They are not sent to AI or used to infer unconfirmed MSX/customer associations.

Historical-mail discovery is explicit and samples recipient names/addresses,
not mail bodies, at up to 25 messages per search. It returns disabled group
candidates; selecting a group enables read-only membership refresh. Incomplete
or failed reads retain previous membership and expose an error. This never
changes directory membership. Reading or snoozing an invitation, changing its
priority or opening its calendar context sends no RSVP or message. Responding
uses the same existing occurrence-only confirmation and verified write path.

## AI boundaries

- **Isolated runtime.** The Copilot CLI child runs with its own home and
  working directory in the profile, configuration discovery and remote
  sessions/export disabled, and fail-closed permissions. It gets no shell,
  file-system, arbitrary SQL or approval tools.
- **Read-only tools, per task.**

  | Task | Tools |
  |---|---|
  | Ask (Quick, Thorough, Auto) | Cached calendar and Insights reads, local index search/read, WorkIQ mail/calendar reads and workplace search, configured OneDrive folder reads, WebIQ, GitHub read tools, MSX (Thorough only), and `tomlook_propose_meeting_change`, which only stages a proposal |
  | Meeting preparation | The same reads (MSX limited to six reads); no proposal tool |
  | Customer communication research | WorkIQ mail/calendar reads and workplace search only, 12 reads, 300 s |
  | Notes, translations, follow-up e-mail, week summary, prompt suggestion | No tools |

- **Budgets.** Quick: 60 s, 4 remote and 12 local reads. Thorough: 10 min,
  40 remote and 80 local reads. Preparation: 8 WorkIQ reads, 64,000 bytes per
  tool result. MSX queries: one `SELECT` on `account`, `contact`,
  `opportunity`, `msp_engagementmilestone`, `task` or `systemuser`, at most 50
  rows; Azure consumption: one DAX `EVALUATE`, at most 200 rows. A daily limit
  (default 200 requests) covers all AI work.
- **Untrusted data.** Retrieved content, MSX rows and customer context are
  passed as data marked untrusted, never as instructions.
- **Public web.** WebIQ searches reject private identifiers and Tomlook
  credentials; prompts forbid sending workplace or local content to the web,
  and customer communication research never uses web search.
- **Secrets.** A prompt containing a known Tomlook credential is refused;
  credentials are masked in answers, evidence and streamed text.
- **Sources.** Cited links are matched against what tools actually returned;
  others are marked as not retrieved. Only credential-free HTTP(S) links are
  clickable, Markdown images are not fetched and HTML is never executed.
- **Local files.** Only folders you add are indexed. Hidden and sensitive
  files, recognizable credentials, symbolic links and junctions are skipped,
  and every read must stay inside its approved folder.
- **Model output.** Insights, notes and answers are model interpretations,
  not independently verified facts.

## Writes and approvals

Tomlook only writes after an explicit action by you. Opening a menu or a
preview, pressing Escape or closing a dialog never submits anything. The AI
can stage a proposal but never execute or approve one; saying "approve" in
chat does nothing.

| Write | Approved by |
|---|---|
| Set or clear a category; **Follow** / **Stop following** (category plus free availability, no response to the organizer) | The menu click itself |
| Accept (optionally with a category), tentatively accept or decline (optional comment, explicit "send response"; decline defaults to no response) | Confirmation dialog |
| Bulk **Decline without sending** or **Follow selected invitations** from the invitation queue | **Confirm for all selected invitations**, after reviewing the exact occurrences; neither notifies organizers |
| Cancel a meeting you organize (attendees are notified); delete your own appointment without attendees | Confirmation dialog |
| Create an event, optionally private, with Teams and travel blocks | **Create event** |
| Create an event with attendees (Outlook sends the invitations) | **Create and send invitations**, after the list of recipients |
| Travel blocks around an existing own meeting or accepted invitation | Confirmation with both intervals; only personal, attendee-free blocks are created |
| Recommendations and AI-proposed existing-event changes (Insights/notes sections, category, travel, title/description, deletion, cancellation or decline) | **Approve change** in Messages; notification consequences are shown |
| AI-proposed new events, including Teams and invitations | **Approve and create event** or **Create and send invitations** in Messages, after reviewing the exact event and recipients |
| BLOCKER holds; deleting them | **Create N BLOCKERs**; **Delete N BLOCKERs** / **Release all BLOCKERs** |
| A Teams chat and one message proposing times | **Create chat and send** |

Acceptance category suggestions are deterministic and local; selecting or
preselecting a chip is not approval. One confirmation binds the exact category
and response. Because Graph requires separate calls, Tomlook assigns and verifies
the category first, then accepts once and verifies both. If the category cannot
be verified, acceptance is not attempted. Any failure or uncertain result after
the category succeeds is explicitly reported as a partial result in **Messages**
for review, never as "nothing changed" or an automatically replayable operation.
Checking that result later only reads; it never repeats either write.

Every write follows the same rules:

1. **Single use.** The exact operation is saved durably, bound to the
   connected account, the meeting occurrence and its fingerprint, and
   consumed in the same transaction before it is sent.
2. **Fresh checks.** The meeting is read again and must still match (if it
   changed, nothing is sent and the recommendation or proposal expires); free
   time is rechecked for new blocks; the provider's write schema must match
   the supported contract. This is not atomic: WorkIQ offers no If-Match, so a
   concurrent external change can still end in review.
3. **Submit once, then read back.** The result is read back field by field
   (for invitations: exactly the intended attendees; for Teams: the chat
   members must be exactly you and the approved people before the message is
   sent, then the message itself). Deletion is proven by a "not found" of that
   exact resource. Retries apply to reads only.
4. **Never replay.** An unknown or interrupted outcome stays in Messages for
   review; **Check calendar result** only re-reads. Only a definitive
   rejection allows a new, explicit approval. Rejections keep the HTTP status
   and an allowlisted error code, never provider message bodies.
5. **No rollback.** Multi-part writes stop at the first failure; verified
   parts stay. With attendees, an uncertain outcome may mean invitations were
   already sent; check Outlook first.
6. **Shutdown.** Exit finishes approved operations instead of cancelling
   them; anything interrupted becomes uncertain, never retried on restart.

Series masters are never written. Private events use Outlook's private
sensitivity, which delegates with permission may still read; it is not
encryption. Tenant policy can block writes (WorkIQ blocks them by default);
Tomlook neither grants access nor bypasses policy.

Tomlook never sends e-mail, creates Outlook drafts, or writes to To Do,
Planner, Dataverse/MSX, GitHub, SharePoint/OneDrive or local files you added.
Tasks, feedback, customers, prompts and approved Insights/notes edits are
local only.

## Updates

- One unauthenticated request to the GitHub Releases API, about a minute
  after start and then daily; failed checks retry after an hour. Drafts and
  prereleases are ignored; only a strictly newer version is considered.
- `tomlook-update.json` and its `.sig` (each at most 64 KiB) are downloaded and
  the **Ed25519 signature is verified against the public keys built into
  Tomlook before the manifest is parsed**. The manifest must name Tomlook,
  the release tag, its GitHub release page and the expected archive.
- The ZIP (at most 150 MB) must match the signed size and SHA-256 and contain
  only `Tomlook.exe`, `LICENSE` and `THIRD_PARTY_NOTICES.txt`; the extracted
  executable must match its signed SHA-256 and is hashed again before
  installation. Downloads follow HTTPS redirects within GitHub only.
- Installation renames the running executable to `Tomlook.exe.old`, moves the
  new one into place and rolls back on any failure.
- Updates never run in the demo, stress or measurement modes or in
  development builds inside a Cargo `target` directory. Two Tomlook processes
  with different profiles started from the same folder can race on staging.

The release signing key is stored only DPAPI-protected for the releasing
Windows user (see [development.md](development.md#releasing)); the trusted
public keys are compiled into the app (`TRUSTED_KEYS` in
`native\src\updater.rs`). Rotating the key means shipping a release that
trusts the new public key before signing with it. If the key is lost,
existing installations need one manual update. The executable itself is not
Authenticode-signed yet.

## Offline demo

`--demo` and `--stress` use fictional data in a new, empty profile, never
read the personal profile, never sign in, start AI, contact any service or
change a calendar, and reject existing profiles.
