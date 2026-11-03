# Development

Tomlook is one Rust crate in `native\`. See [ARCHITECTURE.md](../ARCHITECTURE.md)
for threads, modules and data flow, and
[privacy-and-safety.md](privacy-and-safety.md) for the rules every change must
keep.

## Repository layout

```text
native\                Rust crate (edition 2024, Rust 1.95+)
  src\                 library modules (see ARCHITECTURE.md)
  src\app\             egui user interface
  src\bin\             probes and the release-sign tool
  tests\               integration tests (live checks are #[ignore])
  build.rs             generates the icon and the Windows version resource
  .cargo\config.toml   COPILOT_SKIP_CLI_DOWNLOAD=1 (the SDK never downloads a runtime)
  connections.example.json, decision-router.example.json
docs\                  user documentation, release notes, images, history
scripts\               package-release.ps1, publish-native.ps1
.github\skills\        Copilot agent skills for MSX/ACR context and task-write safety
                       (used by coding agents in this repository, not by the app)
```

The web prototype that preceded the native app was removed; it remains in
git history and in tag `v0.6.1`.

## Prerequisites

- Rust 1.95+ (`%USERPROFILE%\.cargo\bin\cargo.exe`; it does not need to be on
  `PATH`).
- Visual Studio C++ Build Tools (x64) and a Windows SDK; builds run inside
  `vcvars64.bat`.
- Dependencies come from the committed `Cargo.lock` and the machine's offline
  Cargo cache. Always build with `--locked --offline`.

## Build and test

Run from the repository root in PowerShell:

```powershell
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
function Invoke-Cargo([string]$Arguments) {
    & $env:ComSpec /d /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cd /d `"$PWD\native`" && `"$env:USERPROFILE\.cargo\bin\cargo.exe`" $Arguments"
}

Invoke-Cargo 'test --locked --offline'                                   # all offline tests
Invoke-Cargo 'test --locked --offline --lib weekly::'                    # one module
Invoke-Cargo 'test --locked --offline --test scheduler'                  # one integration test file
Invoke-Cargo 'test --locked --offline --test invitations'                # queue policy, cache and worker integration
Invoke-Cargo 'clippy --locked --offline --all-targets -- -D warnings'
Invoke-Cargo 'build --release --locked --offline --bin tomlook'          # native\target\release\tomlook.exe
```

- `$env:CARGO_INCREMENTAL = '0'` before `Invoke-Cargo` (the environment is
  inherited) avoids writing incremental data for one-off runs.
- `native\target\debug` grows quickly because every test binary is linked
  separately (around 47 GB on the maintainer's machine, of which about 6 GB is
  incremental data). Check free disk space, run `cargo clean` only when no
  build is running, and delete temporary `--target-dir ..\.build\check`
  directories after use.
- Default test runs never use accounts or the network; providers are faked.

`scripts\publish-native.ps1` builds the release binary and atomically replaces
the local `release\Tomlook.exe` (exit that copy first); on failure the old
executable stays.

## Offline demo and screenshots

```powershell
native\target\debug\tomlook.exe --demo --state <new-empty-dir>        # 200 fictional meetings
native\target\debug\tomlook.exe --stress --state <new-empty-dir>      # 5,000 meetings
native\target\debug\tomlook.exe --demo --state <new-empty-dir> --demo-screenshot <new.png> [--demo-scene <scene>]
```

The demo rejects non-empty or personal profiles and never connects anything.
`--demo-screenshot` captures only the app's own framebuffer, never overwrites
a file, and leaves the app running afterwards; stop that process yourself and
delete the demo directory. The demo notification identity
(`HKCU\Software\Classes\AppUserModelId\Tomlook.Calendar.Demo`) is removed on a
normal exit; delete it manually after a forced stop.

| `--demo-scene` | Shows |
|---|---|
| `details` (default) | The Tuesday architecture workshop with sample Insights |
| `feedback` | Expanded Insights with a rating and the open reason panel |
| `prompts` | Settings → AI prompts with sample feedback |
| `tasks` | The Tasks list |
| `followup` | The follow-up e-mail of yesterday's sample meeting |
| `myday` | My day |
| `notifications` | Settings → Notifications at the morning overview |
| `weekly` | Weekly review of the current week |
| `invite` | New event with three attendees and the scheduling grid |
| `findtime` | Find a time with two candidates selected |
| `teams` | Find a time with the Teams proposal approval |
| `customer` | The Northstar customer timeline |
| `setup` | Settings → Setup |
| `welcome` | The first-run Welcome window |
| `invitations` | The synthetic invitation queue with three-day calendar context and ordinary details |

`docs\images\tomlook-demo-dark.png` is a `details` screenshot.

## Measurements

The `tomlook-acceptance` binary shares the app's entry point but only runs in
measurement modes, so it can be built while `tomlook.exe` runs.

- `calendar-fixture <new-dir> <200|5000>` or `seed-fixture <new-dir> <200|5000>`
  seed a new profile with the frozen benchmark corpus (42 days from
  5 October 2026, overlaps, all-day, multi-day and a DST change).
- `--startup-trace <new.json>` (and `--idle-trace` in the acceptance binary)
  record startup landmarks and exit through the normal shutdown.
- `--fixture-cache --interaction-trace <new.json> --state <seeded-dir> --legacy-root <empty-dir>`
  (acceptance binary) replays 1,000 keyboard interactions and records
  per-frame timings.

Measurement modes disable providers, notifications and updates, and refuse
normal profiles and existing output files.

## Probes and diagnostics

Run with `Invoke-Cargo 'run --locked --offline --bin <name> -- <arguments>'`.
None of them calls a model or writes to a calendar.

| Binary | Purpose |
|---|---|
| `sdk-probe <new-dir> <copilot.exe> [--stored-login]` | Copilot runtime handshake and identity in an isolated probe profile; no model or connectors |
| `connector-probe <state> [connector]` | Runtime-managed OAuth status for connectors configured in `connections.json`; may open a browser sign-in |
| `workiq-hosted-probe [--sign-in <email>]` | Hosted WorkIQ discovery; with `--sign-in`, browser sign-in and one minimal calendar read |
| `workiq-probe <state> [WorkIQ-Calendar]` | OAuth discovery of a per-workload WorkIQ server; no sign-in |
| `workiq-context-probe <state> <email>` | One workplace context read with the saved sign-in; prints only duration and size |
| `workiq-availability-probe <state> [email]` | Create-schema shapes, your own free/busy and meeting suggestions with the saved sign-in; never writes |
| `msx-probe [tenant]` | Azure CLI token check, MSX tool discovery, user resolution and one `TOP 1` read; prints shapes only |
| `source-index <state> [--repositories <path>] [--cloud-folder <name>] [--query <terms>]` | Configures source collections (run while Tomlook is closed) and tests local search |
| `calendar-fixture`, `seed-fixture` | Seed measurement profiles (see above) |
| `release-sign` | Update manifest signing (see below) |

Live checks are ignored tests that use the saved app-owned sign-ins and write
a private result file. Run one at a time, for example
`Invoke-Cargo 'test --locked --offline --test insights_live -- --ignored --nocapture'`:

| Test | Checks | Environment |
|---|---|---|
| `insights_live` | One real read-only Insights request | `TOMLOOK_LIVE_INSIGHTS_ROOT`, `_MEETING`, `_OUTPUT` |
| `calendar_live` | One calendar range read, not saved | `TOMLOOK_LIVE_INSIGHTS_ROOT`, `TOMLOOK_LIVE_WORK_ACCOUNT`, `TOMLOOK_LIVE_CALENDAR_FIRST`, `_LAST`, `_OUTPUT` |
| `invitations_live` | Native saved connection: mail-header group discovery, complete selected memberships, six-month unanswered queue, independent full-calendar comparison | `TOMLOOK_LIVE_INSIGHTS_ROOT`, `TOMLOOK_LIVE_INVITATION_SELECTION` (private JSON with `queries` and `enabled_groups`), `_OUTPUT`; `_SAVE=1` explicitly saves only local invitation/team caches while Tomlook is closed |
| `cloud_context_live` | OneDrive folder reads | `TOMLOOK_LIVE_CLOUD_*`, `TOMLOOK_LIVE_INSIGHTS_ROOT`, `TOMLOOK_LIVE_WORK_ACCOUNT` |
| `chat_live` | Ask answers | `TOMLOOK_LIVE_CHAT_*` |
| `decision_live` | Foundry routing accuracy and latency | `TOMLOOK_LIVE_CHAT_ROOT`, `TOMLOOK_LIVE_DECISION_OUTPUT` |
| `--lib live_indexed_insight_uses_returned_local_files` | Insights using a source-index snapshot | Insights variables plus `TOMLOOK_LIVE_SOURCE_SNAPSHOT` |
| `--lib live_calendar_capabilities_readonly`, `live_approved_category_readback_only` | WorkIQ write schemas and read-back of an existing approval; never write | `TOMLOOK_CALENDAR_CAPABILITIES_REPORT`, `TOMLOOK_APPROVAL_READBACK_*` |

## Releasing

1. Bump `version` in `native\Cargo.toml` and the `tomlook` package entry in
   `native\Cargo.lock` (the Windows file version comes from it).
2. Write `docs\releases\vX.Y.Z.md` (changes, download/upgrade, verification
   scope) and update the user documentation.
3. Run the tests and Clippy, then commit and push.
4. Run `.\scripts\package-release.ps1` to stage, or with `-Publish` to publish:
   - builds `tomlook` (release) and `release-sign` with `--locked --offline`
     (`-SkipBuild` reuses `native\target\release\tomlook.exe`) and checks that
     the executable reports the Cargo version;
   - regenerates `THIRD_PARTY_NOTICES.txt` from `cargo metadata` for the locked
     Windows dependency graph (nothing to edit by hand);
   - stages `Tomlook-vX.Y.Z-win-x64.zip` (exe, `LICENSE`, notices),
     `SHA256SUMS.txt`, `tomlook-update.json` and its Ed25519 signature in
     `.build\releases\vX.Y.Z\`, and verifies the signature, archive and
     executable against the trusted public keys;
   - with `-Publish` (requires a clean, pushed tree and a new tag): creates and
     pushes the annotated tag and the GitHub Release with these four files.
5. Installed copies (v0.8.0+) pick the release up within a day.

**Signing key.** The Ed25519 key is stored DPAPI-protected for the releasing
Windows user at `%LOCALAPPDATA%\Tomlook-release\update-signing-key.dpapi`
(override with `-SigningKey <path>` or `TOMLOOK_RELEASE_SIGNING_KEY`). Never
commit or print it. Only the same Windows user can decrypt it; if it is lost,
releases need a new key and installations one manual update.
`release-sign public-key --key <path>` prints the public key;
`release-sign generate --key <new path>` (refuses to overwrite) creates a
rotation key, whose public key must be added to `TRUSTED_KEYS` in
`native\src\updater.rs` and shipped in a release before anything is signed
with it. `sign` and `verify` are used by the script.

## Conventions

- **UI text:** every `language.text("…")` literal in `src\app\*.rs` and
  `src\ask.rs` must have a Czech entry in `TEXTS` in `src\i18n.rs`; tests
  (`native_static_controls_are_in_the_shared_catalog`,
  `catalogs_are_unique_nonempty_and_templates_preserve_arguments`) enforce
  this. Original meeting and source text is never translated.
- **Dependencies:** versions are pinned with `=` and resolved offline. Do not
  add crates that are not in the offline cache, git dependencies or build-time
  downloads.
- **Threads:** no disk, network or SQLite work on the UI thread; use the
  storage worker or AI executor with bounded channels, and guard results with
  revisions.
- **Writes:** any new write goes through the single-use approval pipeline:
  durable approval bound to account and fingerprint, fresh checks, one
  submission, read-back, and no automatic replay of uncertain outcomes. Never
  expose write or approval tools to a model. Tests use fakes; never perform
  live writes in tests or probes.
- **AI tools:** read-only allowlists, bounded results, untrusted-data marking,
  credential redaction and source validation (see
  [privacy-and-safety.md](privacy-and-safety.md#ai-boundaries)).
- **Demo:** new features get fictional demo data and, where useful, a
  `--demo-scene`, so screenshots never need personal data.
