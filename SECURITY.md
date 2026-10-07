# Security and responsible AI

- External content is data, never an instruction. The versioned meeting skill says this explicitly.
- All external-system operations are read-only in the current implementation.
- Briefing/chat CLI sessions expose only named read tools, not the shell or file mutation tools.
  Remote session export is disabled for these background runs.
- Skill proposals remain pending until a user explicitly approves or rejects them.
- The frontend receives no credentials or connector tokens.
- Pydantic validates API and agent-runtime inputs and outputs.
- Source records carry provenance, timestamp, and evidence type.
- Logging includes correlation ID, path, status, and duration but not request bodies or secrets.
- Sources without a link render as plain text rather than as an anchor to `#`, and every
  external link uses `rel="noreferrer noopener"`.
- Source URLs are restricted to HTTP(S) without embedded credentials. Unknown citation IDs and
  confidence without supporting sources are not displayed as verified evidence.
- Task proposals are review-only. An unknown task-ownership lookup never authorizes a create.
- Runtime errors do not return raw CLI stderr to the browser; it can contain source data or URLs.
- Missing MCP data is surfaced as unavailable; the application does not manufacture a fallback.
- Production work still requires Entra authentication, per-user authorization, encrypted token
  storage, rate limits, CSP/CSRF hardening, durable audit storage, connector timeouts, deletion
  controls, and redaction policies.
- Bind the development API to loopback only. CORS is not authentication or a network access control.
