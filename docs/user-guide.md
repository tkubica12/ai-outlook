# Tomlook user guide

This guide describes what each feature does, where to find it and the rules it
follows. Connection setup is in [setup.md](setup.md); data handling and write
safeguards are in [privacy-and-safety.md](privacy-and-safety.md).

- [Window layout](#window-layout)
- [Calendar](#calendar)
- [Meeting details and actions](#meeting-details-and-actions)
- [Creating events and inviting people](#creating-events-and-inviting-people)
- [Find a time, BLOCKERs and proposals](#find-a-time-blockers-and-proposals)
- [Insights (meeting preparation)](#insights-meeting-preparation)
- [Ask](#ask)
- [Recommendations and Messages](#recommendations-and-messages)
- [AI-proposed edits](#ai-proposed-edits)
- [Notes after meetings](#notes-after-meetings)
- [Tasks](#tasks)
- [Follow-up e-mail](#follow-up-e-mail)
- [Feedback on AI output](#feedback-on-ai-output)
- [My day](#my-day)
- [Weekly review](#weekly-review)
- [Customers](#customers)
- [Notifications](#notifications)
- [Updates](#updates)
- [Settings](#settings)
- [Keyboard shortcuts](#keyboard-shortcuts)

## Window layout

- **Header:** period navigation, view selection, search, **New event**, and
  icons for **Ask**, **Commands**, **My day** (sunrise), **Tasks** (with the
  open-task count) and **Settings** (gear).
- **Sidebar:** mini-calendar and category filters.
- **Detail pane:** the selected meeting, or the docked Ask panel.
- **Status bar:** connections (GitHub Copilot, Foundry, WorkIQ, MSX, WebIQ,
  GitHub), the Insights activity count, the recommendation bell, and small
  **Setup x/5** and **Update X.Y.Z ready** indicators when relevant. Each entry
  opens its dialog.

Closing or minimizing hides Tomlook to the tray; background work continues.
Restore it from the tray icon (**Open Tomlook**) or by starting Tomlook again.
**Exit Tomlook** (tray, Commands or Ctrl+Shift+Q) stops all work.

## Calendar

- **Views:** Day, Work week, Week and Month. In the mini-calendar, a click opens
  that day; drag across dates for a custom range (up to the 42 visible dates);
  the chevron before a row selects its Monday–Friday. The mini-calendar's own
  arrows browse months without changing the main view.
- **Navigation:** Alt+Left/Right moves by the current period (or custom range
  length); Up/Down selects the previous/next meeting. A new timeline opens at
  08:00.
- **Filters:** search (Ctrl+F) matches titles, people, locations and
  categories; category keys filter by category; **Hide unconfirmed** removes
  unanswered invitations from all views and keyboard navigation.
- **Card styles:** unanswered meetings are shown on a faint hatch without a
  category stripe; accepted ones have full cards with category colors. A
  lined-document icon means Insights are ready, a checked document means notes
  are ready.
- **Coverage:** days that were never synchronized show **Not synced** and are
  never treated as free. All-day events keep their calendar dates; timed
  events use your local time zone.

## Meeting details and actions

Selecting a meeting shows its time, organizer, location, Teams link, people,
**Customers in this meeting**, saved Insights or notes, and their sources.
Selecting a meeting never starts AI. **People** groups attendees by company
with initials, photos (loaded on demand, kept in memory) and company icons.
Very large meetings list up to 500 people with the full count.

Right-click a meeting, press **Shift+F10**, or use **Meeting actions**:

| Action | Behavior |
|---|---|
| Category | Sets one existing category or clears categories immediately; the Follow marker is preserved. |
| **Follow** / **Stop following** | Adds the `Follow` category and sets availability to free in one update; the organizer gets no response. Stop following removes only the category. Followed meetings are not prepared automatically or reminded. |
| Accept / Tentative / Decline | Confirmation with optional comment and a "send response" choice; Decline defaults to not notifying the organizer. |
| Cancel meeting | Organizer only; notifies attendees after confirmation. |
| Delete | Only your own appointments without attendees, after confirmation. |
| **Add travel blocks before and after** | Your own timed meetings: two adjacent busy blocks (15–180 minutes, default 30) after confirmation. |
| **Find another time with these people** | Opens Find a time with the meeting's people, title and duration. |

Only single events, occurrences and exceptions can be changed; series masters
are read-only. Every change is verified by reading it back; results and
uncertain outcomes appear in **Messages**. An unavailable action stays visible
with the reason on hover.

## Creating events and inviting people

Click an empty time slot (30 minutes) or drag a range (15-minute steps), or
use **New event**. The editor has time sliders, title, attendees, description,
categories, **Private** (also selects your existing personal category),
**Teams** and **Add travel blocks before and after** (15–180 minutes each,
default 30, uncategorized, never with attendees or Teams). Cached conflicts
and unsynchronized days are shown as warnings. Private uses Outlook's private
sensitivity, which delegates with permission can still read; all parts of a
private event, including travel blocks, are private.

**Attendees:** type a name or e-mail. Suggestions come from people you meet
and, when WorkIQ is connected, a read-only directory search. Each attendee is
required or optional (up to 50). You, your other mailboxes, rooms and
duplicates are never added. Rooms and recurrence are not supported.

With attendees, a scheduling grid shows you and each attendee from 08:00 to
18:00: busy, tentative, out of office, working elsewhere, free or unknown
(hatched). Clicking a column moves the start; a summary such as "1 of 3
available · Jamie tentative" is shown. **Suggest times** asks Microsoft 365
for up to five candidates around the selected day.

The button reads **Create event**, or **Create and send invitations** when
attendees are present, and the dialog lists exactly who receives an Outlook
invitation. That click is the only approval. Progress and the outcome are
saved under **Messages → Event creation**. If an outcome is uncertain, check
Outlook before creating anything again: invitations may already have been
sent. Parts that succeeded before a later failure are not rolled back.

## Find a time, BLOCKERs and proposals

Open **Find a time** from Commands, from the new-event editor or from a
meeting's **Find another time with these people**. Inputs: topic, attendees,
duration (15–240 minutes), first day and 1–15 days, daily window (default
08:00–18:00), **Weekdays only** and the minimum number of candidates
(default 3).

Tomlook reads free/busy for everyone and ranks times locally: required people
free first, then fewer unknown and tentative required people, fewer busy
optional people, avoiding 12:00–13:00 and spreading candidates across days.
Unknown availability is never treated as free, and your busy or past time is
never offered. Select one to five candidates, then:

- **Create meeting** (one time): opens the new-event editor with the normal
  invitation approval.
- **Hold with BLOCKERs:** **Create N BLOCKERs** creates one busy
  `BLOCKER: <topic>` event per time in your own calendar, with no attendees,
  no Teams meeting and no reminder; optionally private and with an existing
  category.
- **Propose in a Teams group chat:** after a read-only check that everyone
  resolves to one person in your organization, the approval shows the
  members, chat topic and full editable message (times with their UTC offset).
  **Create chat and send** creates one chat (one-on-one for a single person)
  and sends one message. If anyone cannot be resolved, nothing can be sent.
- **Propose by e-mail:** local subject and body with **Copy** and
  **Open in Outlook** (`mailto:`). Tomlook never sends e-mail.

BLOCKERs created by Tomlook show **Proposed time** with **Confirm this time**
(opens the invitation editor for that slot, then offers to delete the
remaining BLOCKERs) and **Release all BLOCKERs**. Only events that still match
the stored event ID, creation transaction ID and `BLOCKER:` title, with you as
organizer and nobody invited, are deleted. BLOCKERs count as busy time but
not as meetings in My day or the weekly review. Proposals are saved locally;
each can be held once and sent to Teams at most once.

## Insights (meeting preparation)

Insights are bilingual (English and Czech) preparation notes: overview,
preparation, talking points, open questions, risks and evidence gaps, plus
source links. They are researched from mail, Teams, earlier meetings,
OneDrive/SharePoint and, if configured, local document folders, Git
repositories, GitHub and WebIQ.

- **Automatic preparation** (on by default) covers meetings you organize or
  accepted in the next six months, nearest first. Unanswered,
  tentative, declined and followed meetings are skipped. It needs both
  GitHub Copilot and WorkIQ connected with the WorkIQ account that owns the
  calendar, and synchronized calendar days.
- Insights within the next 14 days are refreshed after 24 hours; others are
  reused until the meeting changes. A failed automatic attempt is retried up to
  three times with backoff without blocking other meetings.
- **Research now** in meeting details prepares any cached meeting (also past
  ones) immediately, even while automatic work is paused.
- **Insights activity** (click the activity count in the status bar) lists
  waiting, running, ready and failed meetings with filters, has **Automatic
  Insights** and **Pause Insights**, meeting-notes status, **AI usage** with
  the daily request limit (default 200 requests a day, shared by all AI
  features), and **Sources** for local folders.
- **Sources:** up to eight local document folders and four repository
  collections (only Git repositories found inside are read; plain text,
  Markdown and HTML, no PDF/Office), plus an optional customer folder in
  OneDrive used as a hint for cloud research.
- Model-cited links are matched against what the tools actually returned;
  links that were not retrieved are marked as such. Insights are model output,
  not verified facts.
- Older English-only Insights can gain Czech by a translation-only update,
  labeled as such.

## Ask

**Ask** (header icon or Ctrl+Space) is a chat panel docked on the right;
**Detach Ask** moves it into its own window. It answers about your whole
calendar or about the meeting it was opened for. Enter sends, Shift+Enter
adds a line, **Stop** cancels; hiding the panel never cancels.

| Mode | Model | Limits |
|---|---|---|
| **Quick** | GPT-6 Luna, low effort | 60 s, 4 remote and 12 local reads |
| **Thorough** | GPT-6.1 Sol, medium effort | 10 min, 40 remote and 80 local reads |
| **Auto** | Foundry Microsoft-Decision-1 picks Quick or Thorough | needs the Foundry connection |

Slash commands appear after typing `/`: `/today`, `/week`, `/search <words>`
and `/insights` (with a meeting) run locally without AI or network;
`/prepare` (with a meeting), `/conflicts` and `/find <text>` expand to model
questions; `/travel` and `/categories` review the next 14 days in the
background and put proposals in the recommendation bell. `/search` is
typo-tolerant and accent-insensitive.

Conversations are kept in memory only (up to 32 turns per context) and
cleared on exit. Thorough research saves private checkpoints, so after an
interruption you can **Continue research** (a new request using the saved
findings) or **Discard saved research**. Answers show expandable tool
evidence. Ask can draft tasks (**Add to tasks**) and e-mails (**Copy**,
**Open in Outlook**) but never sends anything or changes the calendar.

## Recommendations and Messages

The bell in the status bar opens **Messages**: recommendations, event
creation results, scheduling proposals, applied changes and errors.

Fresh Insights can add up to four recommendations: a category, short or
advance preparation time, travel before or after an offsite meeting, missing
materials (with whom to ask), a priority conflict, a follow-up after the
meeting, or a transition. Each card shows the action, the meeting and the
rationale, with **Open Insights** and **Skip recommendation** (remembered
permanently) at the top; the action button approves the change. Conflict
cards offer tentative/decline/cancel/delete according to your role.
Recommendations only cover the next 14 days; follow-ups appear after the
meeting ends.

Preparation and travel blocks are placed on weekdays 08:00–18:00 in free
time; arrival travel ends exactly at the meeting start and departure travel
starts at its end. In the owner's category set, `office` means your own
office and `mimo office` a customer's or partner's premises.

**Check calendar result** on an uncertain item re-reads the calendar; it never
submits the change again.

## AI-proposed edits

In Quick, Thorough and Auto conversations the assistant can stage a pending
proposal instead of describing a change:

- replace sections of saved Insights or notes (summary, preparation, talking
  points, open questions, risks, gaps) in both languages;
- set or clear an ordinary category (Follow is preserved);
- add one travel block (15–180 minutes) before or after an accepted timed
  meeting;
- edit the title or description of an event you organize (Teams meetings:
  append only, so the join information is kept).

**Review changes** opens the exact proposal in Messages with a before/after
preview; only **Approve change** applies it. Saying "approve" in chat never
does. Approved document sections survive later automatic preparation. At
most four proposals per question and sixteen pending per account.

## Notes after meetings

For Teams meetings you organized, accepted or follow, Tomlook checks for a
transcript every 30 minutes for up to seven days after the meeting. Once one
exists, it reads the transcript and the meeting chat (from 15 minutes before
the start to one day after the end) and saves bilingual notes: your tasks,
mentions of you, decisions, technical takeaways and shared links. Each
meeting is processed once; **Check transcript** asks for an earlier check.
No transcript means no AI request. Transcripts larger than 512,000 bytes are
rejected rather than summarized partially. Access depends on WorkIQ's Graph
permissions and your tenant's policy; Follow does not start recording or
transcription.

## Tasks

**Tasks** (header icon, or **Open tasks** / **Add task** in Commands) is a
list stored only on this computer; nothing is written to To Do, Planner or
Dataverse. Open tasks are grouped as overdue, today, upcoming and undated;
tasks completed in the last 30 days are collapsed below. Tasks can be edited,
given a due date, completed, deleted (with **Undo**) and opened at their
meeting.

When notes become ready, each item of **My tasks** becomes a task in the UI
language (the other language goes to the note). A due date is set only if the
item names exactly one date with a year. Completed or deleted imported tasks
never return.

## Follow-up e-mail

Ended meetings with notes offer **Prepare follow-up e-mail**. One explicit
Quick AI request without tools drafts a subject and body from the notes and
marks uncertain items. Choose recipients (organizer and attendees by default),
edit, then **Copy** or **Open in Outlook** (a `mailto:` link; very long bodies
are copied to the clipboard instead). The draft is kept per meeting until
**Discard**. Tomlook never sends e-mail or creates Outlook drafts.

## Feedback on AI output

Insights and notes have **Helpful** / **Not helpful** thumbs for the whole
document and for each section. Not helpful offers reasons and an optional
comment (up to 500 characters). A rating belongs to the exact text you saw.
Ratings stay in your local profile and are never sent automatically.

**Settings → AI prompts** summarizes the last 90 days. **Suggest prompt
improvement** (needs at least one negative rating and Copilot) sends one
tool-free Thorough request with the prompt and the feedback; the draft shows
its rationale and a diff. **Use draft** only copies it into the editor; it is
saved only with **Save prompts**.

## My day

**My day** (sunrise icon, Ctrl+D or Commands) summarizes today from cached
data, without AI or network: meeting count and time, the next meeting with
**Join** and **Open**, the agenda with badges, conflicts, meetings without
Insights (**Research now**, **Research all**), missing materials, overdue and
due tasks plus tasks and mentions from the previous workday's notes, focus
time (free gaps of 30+ minutes between 08:00 and 18:00, only when today is
synchronized), recommendations waiting for approval and tomorrow.

## Weekly review

**Weekly review** (Commands, the link in My day, or the button next to the
week's dates) covers Monday to Sunday from cached data:

- meetings attended and meeting time (overlaps count once), per workday and
  as a share of Monday–Friday 08:00–18:00; focus time and the longest block;
- time by category, conflicts, meetings without an agenda (split into yours
  and others'), back-to-back chains and long days;
- people and external companies you met most, Insights and notes coverage,
  tasks created and completed;
- trends against the previous four sufficiently synchronized weeks.

Unsynchronized days are listed and the numbers marked partial. **Copy
summary** copies Markdown. **Summarize the week with AI** sends one tool-free
Quick request with only these numbers and the week's notes; the last summary
per week is kept locally.

Agenda detection uses the synchronized invitation preview (first 255
characters) and ignores the Teams join block; meetings synchronized before
v0.13 show **not checked** until their day is refreshed.

## Customers

External e-mail domains of a meeting are grouped into customers. Your own
domains, consumer mail domains and, for a Microsoft sign-in,
Microsoft-owned domains are never customers. Unknown domains appear as
**Unconfirmed customer**.

- **Confirm customer** sets the name and domains and, with MSX connected,
  offers MSX account candidates with confidence and evidence. Nothing is
  linked until you click **Confirm**. Per meeting you can choose **Not a
  customer meeting**, **Only this customer** or **Detect automatically**.
- The **customer timeline** (from the meeting card, a company name or icon,
  **Customers** in Commands, or Settings → MSX) shows meetings one month back
  and six months ahead, notes and decisions, tasks, MSX opportunities and
  milestones (with whether you own a task: yes, no or unknown), a 12-month
  Azure consumption chart (completed, current partial and predicted months
  drawn differently) and recent communication.
- MSX data is labeled with its state: off, not linked, not signed in,
  loading, partial or unavailable. A failed or truncated read is never shown
  as "none".
- **Find related communication** runs one Thorough request limited to mail,
  Teams and meeting reads over the last 60 days; only links actually
  retrieved are kept as sources.
- With **Add customer context to meeting preparation** (Settings → MSX, on by
  default), preparation of a confirmed customer meeting adds a small customer
  summary marked as untrusted data. It does not by itself re-prepare
  meetings.

Customer associations stay in the local profile and are never written to MSX.

## Notifications

All notifications are native Windows notifications and also work while
Tomlook is in the tray. They are configured in **Settings → Notifications**.

- **Meeting reminders** (default 5 minutes before; 1, 2, 5, 10 or 15): time,
  title, location and up to three points from saved Insights, with **Join**
  (safe Teams link), **Open** and Windows **Snooze**. Only timed meetings you
  organize or accepted; tentative ones only with **Also remind me of
  tentatively accepted meetings**. A reminder never starts AI. After sleep, a
  missed reminder is still shown if the meeting started at most two minutes
  ago.
- **Morning overview** (workdays at 08:00 by default, 07:00–10:00, optionally
  weekends): one **My day** summary, shown until noon if missed, only when
  today is synchronized and there is something to act on.
- **Weekly review** (Friday 15:00 for the current week, or Monday for the
  previous week): one summary, only when at least four workdays are
  synchronized.
- **Send test notification**, **Show now** and **Send test** let you check
  each one; the section also shows whether Windows allows Tomlook
  notifications, the next and last reminder and the last error.

Windows **Settings → System → Notifications → Tomlook** controls banners and
sounds; Do not disturb and Focus suppress banners but keep the notification
center entry.

## Updates

**Settings → Updates** shows the version, the last check and a verified update
ready to install, with **Check now** and the automatic-check switch. Tomlook
checks about a minute after start and then daily. A verified update installs
with **Restart now** or when you exit Tomlook; a failed replacement rolls
back. If the folder is not writable or the executable was renamed, only
**Open release page** is offered. See
[privacy-and-safety.md](privacy-and-safety.md#updates) for verification.

## Settings

The gear opens **Settings**:

| Section | Contents |
|---|---|
| **Setup** | Setup checklist: required and optional connections, runtime prerequisites, **Next step** |
| **Display** | Language (English/Czech), light/dark theme, accent color |
| **Notifications** | Reminders, morning overview, weekly review, tests, Windows status |
| **AI prompts** | Five editable prompts (before the meeting, after the meeting, quick answers, thorough research, workplace research) with **Restore default**, feedback summary and **Suggest prompt improvement** |
| **MSX** | Turn MSX on/off, Entra tenant, customer context in preparation, customers |
| **Updates** | Version, checks and installation |

Prompt edits apply after **Save prompts** and only to future output. Safety
rules, tool limits and approvals are not editable. Insights-related switches
are in **Insights activity** (status bar).

## Keyboard shortcuts

| Keys | Action |
|---|---|
| Ctrl+F | Search the calendar |
| Ctrl+Shift+P | Commands (searchable command palette) |
| Ctrl+Space | Show or hide Ask |
| Ctrl+D | Show or hide My day |
| Alt+Left / Alt+Right | Previous / next period |
| Up / Down | Previous / next meeting |
| Shift+F10 | Meeting actions for the selected meeting |
| Alt+Up / Alt+Down | Previous / next recommendation in Messages |
| Enter / Shift+Enter | Send / new line in Ask |
| Tab / Shift+Tab | Move focus |
| Escape | Close the topmost dialog or panel, then clear the selection |
| Ctrl+Shift+Q | Exit Tomlook |
