---
name: msx-meeting-context
description: Associate a meeting with Dataverse opportunities, future milestones, and owned tasks safely.
---

# MSX meeting context

Use only Dataverse read operations during briefing analysis. Discover actual tool names first; typical
read tools are `dataverse-read_query`, `dataverse-describe`, and `dataverse-search`.
Discover table/column schemas and numeric choice values before querying; do not assume display
names are logical names or copy status numbers without verification.

## Association

Infer a likely customer from organizer domain, attendee organizations, subject, and confirmed aliases.
Infer workload only from supported evidence. Present customer/opportunity association with confidence,
reason, and source IDs. Require user confirmation before treating an inferred association as fact.

## Future milestones

Query `msp_engagementmilestone` for active milestones related to the likely account/opportunity,
normally within 60 days:

- ID `msp_engagementmilestoneid`
- name `msp_name`
- due date `msp_milestonedate`
- status `msp_milestonestatus`
- workload `msp_workload`
- opportunity lookup `msp_opportunityid`
- ownership `ownerid`, creator `createdby`
- active `statecode = 0`

Do not infer milestone-team membership from owner or creator. Do not mark future milestones complete.
Exclude completed, cancelled and hygiene/duplicate milestones using the discovered status values.
Prefer milestones on or after the meeting date. Return original record URLs and supporting source IDs.

## User task gap

For each relevant milestone, query `task` where `ownerid` is the current Dataverse user and
`regardingobjectid` matches the milestone. Distinguish an owned task from merely related activities.
Resolve the current Dataverse `systemuser` through verified sign-in identity; an Entra object ID is
not a Dataverse user ID. Check the regarding record type as well as its ID.
If identity resolution or a query fails, return `has_user_task: null`, not false. A truncated
result cannot prove absence; partition queries to obtain complete results before proposing a gap.

If no owned task exists and association evidence is sufficient, return a review-only draft containing:
subject, due date no later than milestone date, duration, activity type, milestone ID, and reason.
Never call create/update/delete during briefing analysis.
