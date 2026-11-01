# Roadmap

Open items for the native app. Release notes in [docs/releases](docs/releases/)
describe what is done.

## Live verification still pending

These features pass offline tests and demo checks but have not yet been
observed with a signed-in live profile:

- MSX and Power BI reads, the customer timeline with real data and **Find
  related communication** (v0.15).
- Writes from scheduling: invitations with attendees, BLOCKER creation and
  release, Teams group chat and message (v0.14).
- Morning **My day** and weekly review notifications, the AI week summary and
  invitation-preview sync (v0.12–v0.13).
- **Suggest prompt improvement** (v0.10) and follow-up e-mail generation
  (v0.11).
- A scheduled meeting reminder firing at its time (v0.9) and the setup
  checklist's connected states (v0.16).

## Planned

- **Code signing:** Authenticode-sign `Tomlook.exe` so SmartScreen no longer
  warns (updates are already Ed25519-signed).
- **Scheduling:** recurring meetings and room/resource booking in the event
  editor and Find a time.

## Ideas

- WorkIQ sign-in for tenants other than the Microsoft corporate tenant.
- Offer categories from the Outlook master category list, not only those
  already used in the calendar.
- Read PDF and Office documents from local folders; recursive OneDrive folder
  coverage; passage-level evidence and claim-to-source verification.
- Optional persistent Ask history.
- Notification **Open** that also starts Tomlook when it is not running.
