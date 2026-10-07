---
name: task-write-safety
description: Govern explicit confirmation and verification of proposed Dataverse task writes.
---

# Dataverse task write safety

1. Build a complete indexed draft before any write: subject, due date, type, duration, milestone,
   opportunity, and association confidence.
2. Display the draft for review. No write is authorized by viewing, editing, or saving a draft.
3. Require an explicit confirmation for the exact draft fields.
4. Write only confirmed rows and immediately verify the returned `activityid` with a read query.
5. Maintain a ledger: draft index, subject, Dataverse ID, write result, verification result.
6. Surface errors directly. Never silently retry or update closed/cancelled activities.
7. Consumption evidence can support a recommendation but can never authorize a write.

