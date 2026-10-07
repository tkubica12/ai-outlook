---
name: azure-consumption
description: Add customer Azure consumption evidence to meeting briefings through Power BI/Fabric MCP.
---

# Azure customer consumption

Use `powerbi-fabric` read/query tools against semantic model `AzureBlueSubscriptionSL4`
(`f7ecc250-c244-43a6-aea5-7a957f9e9d38`) only when the meeting has a supported likely or confirmed
customer association.

Relevant model elements:

- measure `[$ ACR]`
- measure `[$ Average Daily ACR]`
- customer `'DimCustomer'[TPAccountName]`
- subscription/service fields in `'Fact ACR Subscription'`: `SubscriptionName`, `ServiceLevel1`,
  `ServiceLevel2`, `ServiceLevel4`, `ServiceCompGrouping`, `ServiceInfluencer`
- date fields `'DimDate'[FiscalYear]`, `FiscalMonth`, `DateID`

Rules:

1. Discover the live Fabric MCP query tools and schema; never invent a tool name.
2. Derive the current fiscal month from the current date (Microsoft FY begins July 1).
3. Separate completed-month actuals from current partial-month actuals.
4. If predicting current month, use an anchor-calibrated projection and label it as prediction.
5. Show source/model, period, unit, freshness, top service contributors, trend, and anomalies.
6. Consumption is technical evidence, not proof of business intent and not permission to modify MSX.
7. Return no consumption result when customer association or model evidence is insufficient.

Prefer a small number of completed fiscal months and leading service contributors, not an exhaustive
subscription export. Facts have monthly granularity; do not reinterpret a monthly DateID as daily data.
Never mix service breakdowns and month totals into an unlabeled time trend. Give each returned point
an explicit period/label and `kind` (`actual`, `partial_actual`, or `prediction`).

If the calibrated prediction inputs are unavailable, omit the prediction rather than extrapolating
an uncalibrated partial month. Distinguish no matching customer, no matching data, and failed
authentication/query in the warnings. Do not repeat a denied query in the same analysis.
