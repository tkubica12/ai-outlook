# Tomlook setup

Most setup is done in the app: **Settings → Setup** shows every connection
with its state and one action. This page explains what each step needs and
what to do when something fails. See the [user guide](user-guide.md) for
features and [privacy-and-safety.md](privacy-and-safety.md) for data handling.

## Prerequisites

| Needed for | Requirement |
|---|---|
| Running Tomlook | Windows 10/11 x64 and the [Microsoft Visual C++ v14 Redistributable (x64)](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist) |
| Calendar and workplace data | A Microsoft 365 work account with access to WorkIQ (see [WorkIQ](#microsoft-365-calendar-workiq)) |
| AI (Insights, Ask, notes) | A GitHub Copilot account and the installed [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli) |
| Auto mode, MSX | The machine-wide [Azure CLI (MSI)](https://learn.microsoft.com/cli/azure/install-azure-cli-windows) |
| Public web search | A WebIQ API key |
| GitHub connector | A GitHub App you register yourself ([Czech guide](github-sign-in.cs.md)) |

## Profile and command line

Tomlook keeps all state in one profile directory, by default
`%LOCALAPPDATA%\Tomlook` (what is stored there is listed in
[privacy-and-safety.md](privacy-and-safety.md#what-is-stored-locally)).
Only one Tomlook instance can use a profile; starting Tomlook again restores
the running window instead.

| Option | Purpose |
|---|---|
| `--state <directory>` | Use another profile directory (for example a second, isolated profile) |
| `--legacy-root <directory>` | Folder whose `data\calendar-cache.json` and `data\outlook-next.db` (from the retired web prototype) are imported once, read-only, into a profile that has not imported yet. Defaults to the current directory. Imported days stay **Not synced** until refreshed. |
| `--demo` / `--stress` | Offline demo with 200 or 5,000 fictional meetings; requires `--state` with a new, empty directory |

Measurement and screenshot options are described in
[development.md](development.md#offline-demo-and-screenshots).

## Setup checklist

A new profile without saved connections first shows a **Welcome** window
(once). **Settings → Setup** (also **Setup checklist** in Commands) lists:

| Step | Done when | Action |
|---|---|---|
| Microsoft 365 calendar (WorkIQ), required | WorkIQ is connected and every day from one month back to six months ahead is synchronized | **Sign in**, **Retry sync** |
| GitHub Copilot, required | The isolated Copilot runtime accepted its sign-in | **Sign in**, or **Install Copilot CLI** when no runtime is configured |
| Identity match, required | Automatic Insights can run: enabled and not paused, both connections ready, today synchronized, and the WorkIQ account owns the calendar | Opens the relevant dialog |
| Windows notifications, required | Windows allows Tomlook notifications | **Send test notification** |
| Updates, required | Automatic daily checks are on | **Turn on automatic checks**, **Check now** |
| WebIQ, GitHub, Foundry / Auto, MSX, optional | Connected | Opens the connection dialog or MSX settings |
| Runtime prerequisites, informational | Visual C++ runtime, Azure CLI, configured Copilot runtime and its version | **Check again** |

**Next step** runs the first incomplete required step. While a required step
is incomplete, the status bar shows **Setup x/5**; right-click → **Don't
show** hides it. No step signs in or connects by itself.

## Microsoft 365 calendar (WorkIQ)

Click **WorkIQ** in the status bar, enter your work e-mail and choose
**Sign in**. Tomlook opens the browser for Microsoft sign-in (PKCE, with a
loopback redirect to `http://localhost:8080/callback`, ten-minute limit) and
connects to the hosted WorkIQ endpoint `https://workiq.svc.cloud.microsoft/mcp`.

- Tomlook uses Microsoft's published WorkIQ public client and is currently
  configured for the Microsoft corporate tenant. Your organization must allow
  WorkIQ; Conditional Access and tenant policy still apply.
- The sign-in is saved encrypted with Windows DPAPI and restored quietly at
  the next start. **Disconnect and forget sign-in** removes it; the cached
  calendar is kept.
- After connecting, the calendar synchronizes one month back and six months
  ahead (visible days first). Days within two weeks refresh every 15 minutes,
  others every six hours.
- Transcripts for notes need the WorkIQ connector's Graph permissions
  (including `OnlineMeetingTranscript.Read.All`) and tenant policy that allows
  them. Calendar writes and Teams chats are blocked by WorkIQ tenant policy by
  default until an administrator enables them.

## GitHub Copilot (AI)

Tomlook runs the installed Copilot CLI as a child process with its own,
isolated Copilot home in the profile; it never uses your regular Copilot
profile, `gh` login or token environment variables.

1. Install the [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli).
2. Copy [`native\connections.example.json`](../native/connections.example.json)
   to `%LOCALAPPDATA%\Tomlook\connections.json` and set:
   - `runtime`: the absolute path of the installed `copilot.exe`;
   - `use_stored_login`: `true`;
   - `model`: the model for meeting preparation (Ask's Quick and Thorough
     modes use fixed models);
   - `servers`: `{}` (WebIQ and GitHub are connected from the status bar).
3. Restart Tomlook, click **GitHub Copilot** in the status bar and choose
   **Sign in**. Approve the sign-in in the browser.

Alternative to step 3, from PowerShell:

```powershell
$env:COPILOT_HOME = "$env:LOCALAPPDATA\Tomlook\copilot"; $env:COPILOT_DISABLE_KEYTAR = '1'
Remove-Item Env:GH_TOKEN, Env:GITHUB_TOKEN, Env:COPILOT_GITHUB_TOKEN -ErrorAction SilentlyContinue
copilot login
```

Tomlook accepts only the stored `user` identity of that isolated home. The
Copilot CLI stores this login as a file under `%LOCALAPPDATA%\Tomlook\copilot`,
readable by your Windows account. Instead of a stored login,
`copilot_credential_env` can name a `TOMLOOK_*` environment variable that
holds a token (never put the token itself in the file); the two options are
mutually exclusive. Connecting checks the identity only and sends no model
request.

## WebIQ (public web search)

Click **WebIQ**, paste the API key and choose **Connect and save key**. One
small public search for "Microsoft" verifies it before it is saved
(DPAPI-encrypted). The model can then search the public web; requests
containing private identifiers or credentials are refused.
**Disconnect and forget** removes the key after a second confirmation.

## GitHub connector

The connector gives research read-only access to repositories, issues and pull
requests through `https://api.githubcopilot.com/mcp/`.

1. Register a dedicated GitHub App with **Device flow** enabled and read-only
   **Contents**, **Issues** and **Pull requests** permissions, and install it
   only on the repositories you want ([Czech step-by-step guide](github-sign-in.cs.md)).
2. Click **GitHub** in the status bar, enter the app's public **Client ID**
   (not the App ID or a client secret) and choose **Sign in**.
3. Enter the displayed code on GitHub's device page and approve.

Tokens and their refresh token are stored DPAPI-encrypted and renewed about
five minutes before expiry. Revoke access in your GitHub settings.

## Foundry and Auto mode

**Auto** in Ask uses a Microsoft-Decision-1 deployment in Azure AI Foundry to
choose between Quick and Thorough. Quick and Thorough work without it.

1. Copy [`native\decision-router.example.json`](../native/decision-router.example.json)
   to `%LOCALAPPDATA%\Tomlook\decision-router.json` and set the Foundry
   `endpoint` (credential-free HTTPS), the `deployment` name, the `azure_cli`
   path, `tenant` and `subscription`. No key is stored in this file.
2. Restart Tomlook, click **Foundry** and, if needed, choose **Sign in to
   Azure (device code)**. The browser does not open automatically; enter the
   shown code at the Microsoft sign-in page.

Tokens come from Azure CLI and stay in memory. Signing in may change Azure
CLI's default account; Tomlook always requests the configured tenant and
subscription. Routing failures show on Foundry only, never as a Copilot
failure.

## MSX and Azure consumption

The **MSX** entry connects read-only to the MSX Dataverse
(`https://microsoftsales.crm.dynamics.com/api/mcp`) and the Fabric Power BI
MCP (`https://api.fabric.microsoft.com/v1/mcp/powerbi`) using your Azure CLI
sign-in.

- **Settings → MSX** turns it on or off and sets the Entra tenant (default
  the Microsoft tenant `72f988bf-86f1-41af-91ab-2d7cd011db47`).
- At start Tomlook makes one quiet token check. If a token read times out or
  needs interaction, the status becomes **sign-in required** and Azure CLI is
  not started again until you act.
- **Sign in to MSX** (browser or Windows prompt) or **Use device code** starts
  a cancellable `az login`; this may change Azure CLI's default account.
- **Disconnect** forgets Tomlook's in-memory tokens only.

## Local sources for research

In **Insights activity → Sources** (status-bar activity count) add up to
eight **Local document folders**, four **Repository collections** (a folder or
drive root in which Git repositories are discovered) and an optional
**Customer folder in OneDrive** (a hint for cloud research; it is never
downloaded locally). Tomlook indexes the first 16,000 bytes of readable text
files into `source-index.db`; hidden, credential and dependency/build files
are skipped.

## Notifications

On start, Tomlook registers the AppUserModelID `Tomlook.Calendar` for the
current user under `HKCU\Software\Classes\AppUserModelId\Tomlook.Calendar`
(display name, icon and a COM activator); no Start-menu shortcut is created.
Turn notifications on or off in **Settings → Notifications** or in Windows
**Settings → System → Notifications → Tomlook**. Clicking **Open** works while
Tomlook runs (also in the tray). Because the window uses this explicit ID, a
taskbar pin from an older version may show as a separate button; pin the
running window again. The demo uses `Tomlook.Calendar.Demo`, removed on exit.

## Updates

Automatic checks are on by default (**Settings → Updates**). Tomlook needs
write access to its own folder to install updates; otherwise it only offers
**Open release page**. Manual updates: exit Tomlook completely (including the
tray), replace `Tomlook.exe` with the one from the new ZIP and keep the
profile.

## Troubleshooting

| Symptom or message | Meaning and fix |
|---|---|
| `VCRUNTIME140.dll` was not found | Install the Visual C++ v14 Redistributable (x64) from Microsoft. |
| SmartScreen warns on first start | The executable is not code-signed yet. Compare its hash with `SHA256SUMS.txt` from the release. |
| "Read Tomlook connections.json …" | The profile has no `connections.json`; create it as in [GitHub Copilot](#github-copilot-ai). |
| "Configure an absolute path to an installed compatible copilot.exe …" | `runtime` in `connections.json` is wrong or the CLI is not installed. |
| "Choose either use_stored_login or copilot_credential_env, not both" | Remove one of the two options. |
| Copilot connects but is rejected | Only the stored `user` identity of Tomlook's isolated Copilot home is accepted, never `gh` or environment identities; sign in again from the GitHub Copilot dialog. |
| Days show **Not synced** | WorkIQ is not connected or synchronization failed; see WorkIQ status and **Retry sync**. Unsynced days are never shown as free. |
| Automatic Insights do not start | Check **Identity match** in the setup checklist: both connections, the WorkIQ account that owns the calendar, today synchronized, Insights not paused. |
| A connection shows its next retry delay | A transient error; Tomlook retries after 30 s, 1, 2, 4 and 8 minutes, then every 15 minutes, using only the saved sign-in. Rejected credentials or policy errors need a new sign-in. |
| Daily AI limit reached | New AI work waits until tomorrow; raise the limit in **Insights activity → AI usage**. |
| A calendar change is rejected with an HTTP status | The provider or tenant policy refused it; nothing was changed. You can try again explicitly. |
| A change shows as uncertain | The result is unknown. Check Outlook, then use **Check calendar result**; Tomlook never resubmits it by itself. |
| Notes report a permission error | The transcript permission or tenant policy is missing; this is not treated as "no transcript". |
| MSX shows **sign-in required** | Use **Sign in to MSX** or **Use device code**; check the tenant in Settings → MSX. |
| MSX reads fail with an IP or network policy error | Power Platform can restrict access by network; connect from an allowed network. Signing in again does not help. |
| Auto mode fails | Foundry is not signed in or the deployment is not accessible; Quick and Thorough still work. |
| No notification banner | Check Windows notification settings, Do not disturb and Focus; use **Send test notification**. |
| "Demo previews require --state pointing to a new, empty directory" | Use a new, empty folder for `--demo` / `--stress`. |
| Second start does nothing visible | Tomlook was already running in the tray; the existing window is restored. |

Diagnostic reason codes (no prompts or meeting titles) are written to
`diagnostics.jsonl` in the profile.

## Removing Tomlook

Exit Tomlook, delete its folder and, to remove all local data and saved
sign-ins, delete `%LOCALAPPDATA%\Tomlook`. Delete the registry key
`HKCU\Software\Classes\AppUserModelId\Tomlook.Calendar` to remove the
notification registration. Revoke the GitHub App authorization in GitHub;
an Azure CLI sign-in belongs to Azure CLI (`az logout`).
