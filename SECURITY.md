# Security policy

Tomlook is a single-user Windows desktop app. Its security model is described
in [docs/privacy-and-safety.md](docs/privacy-and-safety.md): local storage,
DPAPI-protected sign-ins and memory-only tokens, read-only AI tools,
single-use approvals with read-back for every write, and Ed25519-signed
updates.

## Supported versions

Only the [latest release](https://github.com/tkubica12/ai-outlook/releases/latest)
receives fixes. Automatic updates install them.

## Reporting a vulnerability

Please do not publish exploit details, tokens, calendar content or customer
data in a public issue. Open a short GitHub issue that only says you want to
report a security problem, or contact the maintainer
([@tkubica12](https://github.com/tkubica12)) directly, and details will be
exchanged privately. Ordinary bugs can be reported as regular
[issues](https://github.com/tkubica12/ai-outlook/issues).

Useful information: the Tomlook version (**Settings → Updates**), what you
did, what happened, and whether a write was approved. Remove personal data
from screenshots and logs; `diagnostics.jsonl` contains reason codes only,
but review it before sharing.
