# Tomlook

Tomlook is a fast, native Windows calendar for Microsoft 365 with an AI
assistant built in. It keeps your calendar in a local cache, prepares
bilingual **Insights** for upcoming meetings in the background, writes notes
and tasks after Teams meetings, and helps with planning, scheduling and
customer context, while every change to your calendar or to other people
needs your explicit approval. It is a Rust/egui application (no browser or
WebView), available in English and Czech, with light/dark themes and four
accent colors.

![Tomlook offline demo: a fictional work week with categories and sample meeting Insights](docs/images/tomlook-demo-dark.png)

*The offline demo with fictional meetings, people and sample Insights. No
personal calendar, customer records or live AI output are shown.*

## Features

**Calendar**

- Day, work week, week, month and custom date ranges; mini-calendar, category
  and text filters, **Hide unconfirmed**, command palette and full keyboard use.
- Cache-first sync of one month back and six months ahead through WorkIQ; days
  that were never synchronized show **Not synced**, never "free".
- New appointments by click or drag, with Private, Teams and travel blocks;
  categories, **Follow**, invitation responses, cancel and delete.
- Keeps working in the Windows tray; one instance per profile.

**AI preparation and Ask**

- Automatic Insights for meetings you organize or accepted, up to six months
  ahead: overview, preparation, talking points, open questions, risks and
  gaps, with sources from mail, Teams, earlier meetings, OneDrive/SharePoint
  and optionally local folders, Git repositories, GitHub and WebIQ.
- **Ask** (Ctrl+Space): a chat about your calendar or one meeting with Quick,
  Thorough and Auto modes, plus offline commands such as `/today` and `/search`.
- Quiet recommendations (categories, preparation and travel blocks, missing
  materials, conflicts, follow-ups) and AI-proposed edits that change nothing
  until you approve them.
- Thumbs up/down on Insights and notes; your feedback can produce a prompt
  improvement draft that you review and save yourself.

**After meetings**

- Notes from Teams transcripts and meeting chat: your tasks, mentions of you,
  decisions, technical takeaways and shared links.
- A local **Tasks** list that collects your tasks from those notes.
- **Prepare follow-up e-mail**: an editable draft to copy or open in your mail
  app. Tomlook never sends e-mail.

**Planning and scheduling**

- **My day** (Ctrl+D) and an optional morning notification.
- **Weekly review**: meeting time, focus time, categories, conflicts, meetings
  without an agenda, people, trends and an optional AI summary.
- Attendees with a free/busy grid and **Suggest times**; **Find a time** for a
  group, held with **BLOCKERs** or proposed in a Teams chat or by e-mail.

**Customers**

- Customers detected from external e-mail domains and confirmed by you.
- A customer timeline with meetings, notes, tasks, MSX opportunities and
  milestones, Azure consumption and a summary of recent communication, using
  an optional read-only MSX connection.

**Notifications and updates**

- Meeting reminders with **Join** and **Open**, a morning **My day** overview
  and a weekly review notification.
- Signed automatic updates from GitHub Releases.

## Download and install

1. Download `Tomlook-vX.Y.Z-win-x64.zip` from the
   [latest release](https://github.com/tkubica12/ai-outlook/releases/latest).
   `SHA256SUMS.txt` in the same release lists the checksums.
2. Extract it to a folder you can write to and run **Tomlook.exe**. It is a
   portable app, not an installer; your data lives in
   `%LOCALAPPDATA%\Tomlook`.
3. Windows 10/11 x64 needs the
   [Microsoft Visual C++ v14 Redistributable (x64)](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist).
   If Windows reports a missing `VCRUNTIME140.dll`, install that package from
   Microsoft; never download single DLLs from other sites.

The executable is not code-signed yet, so Windows SmartScreen may warn on the
first start. Updates are different: Tomlook checks GitHub Releases once a day,
accepts only releases whose manifest carries a valid Ed25519 signature and
whose files match the signed SHA-256 hashes, and installs them when you choose
**Restart now** or exit Tomlook.

Live AI needs a GitHub Copilot account and the installed GitHub Copilot CLI.
Microsoft 365 data needs access to WorkIQ in your organization. The offline
demo needs neither.

## First run

A new profile opens a short **Welcome** window. **Settings → Setup** (also
**Setup checklist** in the command palette) then shows each connection with
its state and one action; **Next step** runs the first required step that is
missing. Nothing signs in or connects until you click. See
[docs/setup.md](docs/setup.md) for details and troubleshooting.

## Safety principles

- **Local-first.** Calendar cache, Insights, notes, tasks, feedback and
  customers stay in your Windows profile. Data leaves the PC only in requests
  to the services you connect (there is no Tomlook server).
- **Credentials.** Saved sign-ins are encrypted with Windows DPAPI for your
  Windows account; Microsoft and Azure access tokens are kept in memory only
  and are never given to the AI runtime.
- **Read-only by default.** Synchronization, research, MSX and GitHub access
  only read.
- **Explicit, single-use approvals.** Every write (calendar change,
  invitation, BLOCKER, Teams message) is approved once for exactly what is
  shown, read back afterwards, and never replayed automatically when the
  outcome is uncertain.
- **AI cannot write.** The assistant only gets read tools and can at most
  stage a proposal that you approve yourself.
- **No e-mail sending.** Follow-up and proposal e-mails are local text that
  you copy or open in your own mail app.

Details: [docs/privacy-and-safety.md](docs/privacy-and-safety.md).

## Offline demo

The demo uses only fictional data and never signs in, starts AI, reads your
calendar or changes anything. It needs a new, empty profile directory:

```powershell
$demo = Join-Path $env:TEMP ("tomlook-demo-" + [guid]::NewGuid().ToString("N"))
.\Tomlook.exe --demo --state $demo
```

Select **Northstar / Architecture workshop** on Tuesday to see sample
Insights. Delete the demo directory when you are done.

## Documentation

| Document | Contents |
|---|---|
| [User guide](docs/user-guide.md) | Every feature, settings and keyboard shortcuts |
| [Setup](docs/setup.md) | Connections, notifications, updates, troubleshooting |
| [Privacy and safety](docs/privacy-and-safety.md) | Stored data, network use, approvals, AI boundaries |
| [GitHub sign-in (Czech)](docs/github-sign-in.cs.md) | Registering the GitHub App for the GitHub connector |
| [Architecture](ARCHITECTURE.md) | Threads, modules, data flow |
| [Development](docs/development.md) | Building, testing, demo scenes, probes, releasing |
| [Roadmap](ROADMAP.md) | Open items |
| [Release notes](docs/releases/) | Changes per version |
| [Security](SECURITY.md) | Security policy and reporting |

## License

[MIT](LICENSE). Third-party notices ship in each release archive.
