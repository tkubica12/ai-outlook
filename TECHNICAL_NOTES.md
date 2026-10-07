# Technical notes

## Implemented

- Live-only onboarding; no seeded meetings or synthetic fallback.
- Generic Streamable HTTP MCP client with initialize, tool discovery, tool invocation, MCP session
  IDs, JSON and SSE response decoding, and explicit authentication errors.
- Live Work IQ calendar adapter using discovered `ListCalendarView`.
- Registered Work IQ, Fabric, Dataverse, and WebIQ connector endpoints with independent diagnostics.
- Backend-only credential boundary and ignored local `.env`.
- Product iconography in `src/assets/generated/` was generated specifically for this project
  with the existing `MAI-Image-2.5-Pro` deployment in Azure AI Foundry, then locally
  background-cleaned, cropped, quantized, and verified at production display sizes.
- Business enrichment loads repository-local skills from `.github/skills/`: ACR querying through
  the `AzureBlueSubscriptionSL4` semantic model, Dataverse opportunity/future-milestone/task-gap
  discovery, and an explicit-confirmation write policy. Analysis itself has no Dataverse write
  tool permission.

## Active runtime

- Copilot CLI is the working integration; the installed SDK is not currently the execution path.
- Default analysis model: GPT-5.6 Terra / low, four workers. A change of the development assistant's
  model does not change the application's model.
- Project skills are included directly in the briefing prompt, in addition to CLI skill discovery.
- Dataverse native permission patterns are `dataverse(describe)` and `dataverse(read_query)`;
  the model-visible names are `dataverse-describe` and `dataverse-read_query`. These names must not
  be accidentally double-prefixed.
- Chat uses the same MCP config and read-only restrictions and supports newly retrieved citations,
  not just source IDs from the original briefing.
- Previous briefing context is bounded to avoid Windows subprocess command-line limits.

## Connection prerequisites and remaining boundaries

1. **Delegated identity.** CLI OAuth handles local MCP sign-in. Complete browser consent from the
   configured working directory; a registered tool is not proof of an authenticated data connection.
2. **Tenant consent and policy.** Access to WorkIQ, Fabric models and Dataverse may require separate
   consent. Power Platform IP-policy errors require an allowed network, not repeated sign-in.
3. **Resource scopes/audiences.** Optional direct HTTP tokens must be valid for each service;
   never reuse a WorkIQ token for Fabric or Dataverse without the correct audience.
4. **Power BI target.** The skill targets `AzureBlueSubscriptionSL4`; missing customer association,
   permissions, or data must remain explicit. Partial-month values are not full-month totals.
5. **Dataverse model.** Discover live schemas before querying. Resolve the Dataverse systemuser
   separately from Microsoft Entra identity. An incomplete query cannot prove there is no task.
6. **Writes.** Task editing/review is local only. A future write flow needs exact-field confirmation,
   a durable idempotent ledger, and post-create read verification before it can be enabled.
7. **Feedback.** Feedback and skill-proposal decisions are stored locally; approving a skill proposal
   does not currently rewrite the runtime's skill files.

## Security

Tokens are currently read from the backend environment and are never logged or returned to the
browser. Production must use an encrypted token cache or managed identity/On-Behalf-Of design,
automatic refresh, per-user isolation, and explicit scopes.
