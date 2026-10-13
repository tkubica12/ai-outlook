use super::{Connection, EVENT_FIELDS, Range, hosted, parse_event};
use crate::{
    calendar::{Calendar, Event},
    recommendations::{Change, MeetingResponse, Operation, real_categories},
};
use async_trait::async_trait;
use chrono::{Local, Utc};
use rmcp::model::{CallToolRequestParams, Tool};
use serde_json::{Value, json};
use std::time::Duration;

pub const POLICY_BLOCKED: &str = "WorkIQ denied this calendar change. Your tenant may block mutations; an administrator must enable calendar writes. Nothing was changed by this request.";
pub const UNCERTAIN: &str = "Calendar write outcome is uncertain. Inspect Outlook before proposing another change; this operation will not be replayed automatically.";

#[derive(Clone, Debug)]
pub enum Outcome {
    Applied(Box<Event>),
    Removed(String),
    NotSent(String),
    Review(String),
}

pub fn validate_tool(tool: &Tool) -> bool {
    let parameters: &[&str] = match tool.name.as_ref() {
        "create_entity" => &["parentUrl", "jsonBody"],
        "update_entity" => &["entityUrl", "jsonBody"],
        "get_schema" => &["path", "operationType"],
        "delete_entity" => &["entityUrl"],
        "do_action" => &["actionUrl", "jsonBody"],
        _ => return false,
    };
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object);
    let required = tool.input_schema.get("required").and_then(Value::as_array);
    parameters.iter().all(|name| {
        properties
            .and_then(|properties| properties.get(*name))
            .is_some_and(|property| {
                property["type"] == "string"
                    || property["type"]
                        .as_array()
                        .is_some_and(|types| types.iter().any(|kind| kind == "string"))
                    || (*name == "jsonBody"
                        && property.is_object()
                        && (property.get("type").is_none()
                            || (tool.name == "do_action"
                                && (property["type"] == "object"
                                    || property["type"].as_array().is_some_and(|types| {
                                        types.iter().any(|kind| kind == "object")
                                    })))))
            })
    }) && required.is_none_or(|required| {
        required
            .iter()
            .all(|name| name.as_str().is_some_and(|name| parameters.contains(&name)))
    }) && (tool.name == "get_schema"
        || parameters
            .iter()
            .filter(|name| tool.name != "do_action" || **name == "actionUrl")
            .all(|name| {
                required.is_some_and(|required| {
                    required.iter().any(|value| value.as_str() == Some(*name))
                })
            }))
}

pub(super) fn action_body_is_string(tool: &Tool) -> bool {
    let kind = &tool.input_schema["properties"]["jsonBody"]["type"];
    kind == "string"
        || kind.as_array().is_some_and(|types| {
            types.iter().any(|kind| kind == "string") && !types.iter().any(|kind| kind == "object")
        })
}

#[async_trait]
pub trait Api: Send + Sync {
    fn account(&self) -> &str;
    async fn schema(&self, create: bool) -> Result<(), String>;
    async fn capability(&self, change: &Change) -> Result<(), String> {
        match change {
            Change::Category(_) | Change::ClearCategory => self.schema(false).await,
            Change::Buffer { .. } => self.schema(true).await,
            _ => Err("Calendar provider has no verified meeting action capability".into()),
        }
    }
    async fn event(&self, id: &str) -> Result<Event, String>;
    async fn calendar(&self, range: Range) -> Result<Calendar, String>;
    async fn submit(&self, operation: &Operation) -> Result<String, SubmissionError>;
    async fn removed(&self, _id: &str) -> Result<bool, String> {
        Err("Calendar removal read-back is unavailable".into())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum SubmissionError {
    Denied,
    Rejected(u16),
    RejectedWithCode(u16, &'static str),
    InvalidRequest,
    Unknown,
}

pub async fn execute(api: &impl Api, operation: &Operation) -> Outcome {
    let check = async {
        operation.validate(api.account())?;
        api.capability(&operation.change).await?;
        let current = api.event(&operation.event.id).await?;
        if crate::scheduler::fingerprint(&current) != operation.recommendation.fingerprint {
            return Err(
                "Meeting changed after the recommendation; no calendar write was submitted".into(),
            );
        }
        if let Change::Category(category) = &operation.change
            && real_categories(&current) == [category.clone()]
        {
            return Err(
                "The meeting already has the proposed category; no write was submitted".into(),
            );
        }
        if matches!(operation.change, Change::ClearCategory) && real_categories(&current).is_empty()
        {
            return Err("The meeting already has no category; no write was submitted".into());
        }
        if let Change::Respond { response, .. } = operation.change
            && current.status.eq_ignore_ascii_case(response.status())
        {
            return Err("The meeting already has this response; no write was submitted".into());
        }
        if let Change::Buffer { start, end, .. } = &operation.change {
            let first = start.with_timezone(&Local).date_naive();
            let last = end.with_timezone(&Local).date_naive();
            let calendar = Api::calendar(api, Range::new(first, last)?).await?;
            if !(0..=(last - first).num_days()).all(|offset| {
                calendar
                    .coverage
                    .contains(&(first + chrono::Duration::days(offset)))
            }) {
                return Err("Calendar coverage is incomplete; no write was submitted".into());
            }
            if calendar.events.iter().any(|event| {
                !event.unconfirmed()
                    && event.start < *end
                    && event.end > *start
                    && event.extra.get("showAs").and_then(Value::as_str) != Some("free")
                    && event.extra.get("isCancelled").and_then(Value::as_bool) != Some(true)
            }) {
                return Err(
                    "The proposed interval is no longer free; no calendar write was submitted"
                        .into(),
                );
            }
            operation.validate(api.account())?;
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(message) = check {
        return Outcome::NotSent(message);
    }
    let id = match api.submit(operation).await {
        Ok(id) => id,
        Err(SubmissionError::Denied) => return Outcome::NotSent(POLICY_BLOCKED.into()),
        Err(SubmissionError::Rejected(status)) => {
            return Outcome::NotSent(format!(
                "Calendar provider rejected the write with HTTP {status}; nothing was changed by this request"
            ));
        }
        Err(SubmissionError::RejectedWithCode(status, code)) => {
            return Outcome::NotSent(format!(
                "Calendar provider rejected the write with HTTP {status} ({code}); nothing was changed by this request"
            ));
        }
        Err(SubmissionError::InvalidRequest) => {
            return Outcome::NotSent(
                "Confirmed calendar request is invalid; no write was submitted".into(),
            );
        }
        Err(SubmissionError::Unknown) => return Outcome::Review(UNCERTAIN.into()),
    };
    verify_readback(api, operation, &id).await
}

pub async fn verify_only(api: &impl Api, operation: &Operation) -> Outcome {
    if operation.validate(api.account()).is_err()
        || matches!(operation.change, Change::Buffer { .. })
    {
        return Outcome::Review(
            "Calendar approval no longer matches its account, meeting or proposal".into(),
        );
    }
    verify_readback(api, operation, &operation.event.id).await
}

async fn verify_readback(api: &impl Api, operation: &Operation, id: &str) -> Outcome {
    let may_remove = matches!(
        operation.change,
        Change::Delete
            | Change::Respond {
                response: MeetingResponse::Decline | MeetingResponse::Cancel,
                ..
            }
    );
    for attempt in 0..4 {
        match api.event(id).await {
            Ok(event) if verified(&event, operation) => return Outcome::Applied(Box::new(event)),
            Err(_) if may_remove && api.removed(id).await == Ok(true) => {
                return Outcome::Removed(id.to_owned());
            }
            _ => {}
        }
        if attempt < 3 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    Outcome::Review("Calendar write was returned but read-back did not verify the exact change. Inspect Outlook; the operation will not be replayed automatically.".into())
}

fn verified(event: &Event, operation: &Operation) -> bool {
    let mut original = operation.event.clone();
    // A changed recurring occurrence becomes an exception, without changing the series.
    if original.extra.get("type").and_then(Value::as_str) == Some("occurrence")
        && event.extra.get("type").and_then(Value::as_str) == Some("exception")
    {
        original.extra.insert("type".into(), json!("exception"));
    }
    match &operation.change {
        Change::Category(_) | Change::ClearCategory => {
            let categories = match &operation.change {
                Change::Category(category) => vec![category.clone()],
                _ => Vec::new(),
            };
            let mut expected = original;
            expected.category = categories.join(", ");
            expected
                .extra
                .insert("categories".into(), json!(categories));
            event.id == operation.event.id
                && real_categories(event) == categories
                && crate::scheduler::fingerprint(event) == crate::scheduler::fingerprint(&expected)
        }
        Change::Respond { response, .. } => {
            let mut expected = original;
            expected.status = event.status.clone();
            if let Some(show_as) = event.extra.get("showAs") {
                expected.extra.insert("showAs".into(), show_as.clone());
            }
            let response_matches = if *response == MeetingResponse::Cancel {
                expected.extra.insert("isCancelled".into(), json!(true));
                event.extra.get("isCancelled") == Some(&json!(true))
            } else {
                event.status.eq_ignore_ascii_case(response.status())
            };
            response_matches
                && crate::scheduler::fingerprint(event) == crate::scheduler::fingerprint(&expected)
        }
        Change::Delete => false,
        Change::Buffer {
            title,
            start,
            end,
            category,
        } => {
            event.title == *title
                && event.start == *start
                && event.end == *end
                && event.attendees.is_empty()
                && !event.is_all_day
                && event.extra.get("showAs").and_then(Value::as_str) == Some("busy")
                && event.extra.get("isCancelled").and_then(Value::as_bool) != Some(true)
                && real_categories(event) == category.iter().cloned().collect::<Vec<_>>()
        }
    }
}

fn event_path(id: &str) -> Result<String, String> {
    if id.is_empty()
        || id.len() > 3000
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='))
    {
        return Err("Calendar writes require one verified event identity".into());
    }
    Ok(format!("/me/events/{id}"))
}

fn request(
    operation: &Operation,
    action_body_is_string: bool,
) -> Result<CallToolRequestParams, String> {
    let (name, arguments) = match &operation.change {
        Change::Category(category) => {
            if category.trim().is_empty()
                || category.len() > 256
                || category.chars().any(char::is_control)
            {
                return Err("Invalid confirmed calendar category".into());
            }
            (
                "update_entity",
                json!({"entityUrl":event_path(&operation.event.id)?,
                "jsonBody":json!({"categories":[category]}).to_string()}),
            )
        }
        Change::ClearCategory => (
            "update_entity",
            json!({"entityUrl":event_path(&operation.event.id)?, "jsonBody":json!({"categories":[]}).to_string()}),
        ),
        Change::Delete => (
            "delete_entity",
            json!({"entityUrl":event_path(&operation.event.id)?}),
        ),
        Change::Respond {
            response,
            comment,
            send_response,
        } => {
            let body = if *response == MeetingResponse::Cancel {
                json!({"comment":comment})
            } else {
                let mut body = json!({"sendResponse":send_response});
                if !comment.is_empty() {
                    body["comment"] = json!(comment);
                }
                body
            };
            let body = if action_body_is_string {
                Value::String(body.to_string())
            } else {
                body
            };
            (
                "do_action",
                json!({"actionUrl":format!("{}/{}", event_path(&operation.event.id)?, response.action()),
                "jsonBody":body}),
            )
        }
        Change::Buffer {
            title,
            start,
            end,
            category,
        } => {
            let utc = |time: &chrono::DateTime<chrono::FixedOffset>| {
                json!({
                "dateTime":time.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S%.f").to_string(),"timeZone":"UTC"})
            };
            (
                "create_entity",
                json!({"parentUrl":"/me/events","jsonBody":json!({
                "subject":title,"start":utc(start),"end":utc(end),"showAs":"busy",
                "isReminderOn":false,"categories":category.iter().cloned().collect::<Vec<_>>(),
                "transactionId":operation.recommendation.id,
            }).to_string()}),
            )
        }
    };
    let arguments = arguments
        .as_object()
        .cloned()
        .ok_or("Invalid calendar mutation arguments")?;
    Ok(CallToolRequestParams::new(name).with_arguments(arguments))
}

#[async_trait]
impl Api for Connection {
    fn account(&self) -> &str {
        &self.account
    }

    async fn schema(&self, create: bool) -> Result<(), String> {
        let schema = discover_schema(self, create).await?;
        let schema = schema.get("data").unwrap_or(&schema);
        if !compatible_schema(schema, create) {
            return Err(
                "Calendar provider did not return a compatible JSON schema; no write was submitted"
                    .into(),
            );
        }
        Ok(())
    }

    async fn event(&self, id: &str) -> Result<Event, String> {
        parse_event(
            self.fetch_in(
                &format!("{}?$select={EVENT_FIELDS}", event_path(id)?),
                &self.admission.calendar,
            )
            .await?,
        )
    }
    async fn capability(&self, change: &Change) -> Result<(), String> {
        match change {
            Change::Category(_) | Change::ClearCategory => Api::schema(self, false).await,
            Change::Buffer { .. } => Api::schema(self, true).await,
            Change::Delete if self.calendar_write_tools.contains("delete_entity") => Ok(()),
            // get_schema describes entity fetch/create/update, not POST actions.
            // Responses use fixed native Graph contracts and the validated do_action tool.
            Change::Respond { .. } if self.calendar_write_tools.contains("do_action") => Ok(()),
            _ => Err("Calendar provider has no verified meeting action capability".into()),
        }
    }
    async fn removed(&self, id: &str) -> Result<bool, String> {
        let _permit = super::ReadSlots::acquire(&self.admission.calendar).await?;
        if !self.is_active() {
            return Err("WorkIQ is disconnected; sign in again".into());
        }
        let request = super::fetch_request(&event_path(id)?)?;
        let result = tokio::time::timeout(Duration::from_secs(30), self.peer.call_tool(request))
            .await
            .map_err(|_| "Calendar removal read-back timed out")?
            .map_err(|_| "Calendar removal read-back failed")?;
        removal_result(&result)
    }
    async fn calendar(&self, range: Range) -> Result<Calendar, String> {
        Connection::calendar(self, range).await
    }
    async fn submit(&self, operation: &Operation) -> Result<String, SubmissionError> {
        let request = request(operation, self.calendar_action_body_is_string)
            .map_err(|_| SubmissionError::InvalidRequest)?;
        let result = tokio::time::timeout(Duration::from_secs(60), self.peer.call_tool(request))
            .await
            .map_err(|_| SubmissionError::Unknown)?
            .map_err(|_| SubmissionError::Unknown)?;
        submission_result(&result, operation)
    }
}

fn removal_result(result: &rmcp::model::CallToolResult) -> Result<bool, String> {
    let payload = hosted::read_json_payload(result)?;
    let rows = payload
        .get("results")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() == 1)
        .ok_or("Calendar removal read-back has no unambiguous HTTP status")?;
    match rows[0]["statusCode"].as_u64() {
        Some(404) => Ok(true),
        Some(200) if result.is_error != Some(true) => Ok(false),
        _ => Err("Calendar removal read-back was denied or failed".into()),
    }
}

async fn discover_schema(connection: &Connection, create: bool) -> Result<Value, String> {
    let operation = if create {
        "create_entity"
    } else {
        "update_entity"
    };
    if !connection.calendar_write_tools.contains(operation)
        || !connection.calendar_write_tools.contains("get_schema")
    {
        return Err(
            "WorkIQ has no compatible calendar write tool; read-only research remains available"
                .into(),
        );
    }
    let request = CallToolRequestParams::new("get_schema").with_arguments(
        json!({
            "path": if create { "/me/events" } else { "/me/events/{event-id}" },
            "operationType":if create { "create" } else { "update" }, "format":"jsonschema"
        })
        .as_object()
        .cloned()
        .ok_or("Invalid calendar schema request")?,
    );
    let result = tokio::time::timeout(Duration::from_secs(30), connection.peer.call_tool(request))
        .await
        .map_err(|_| "Calendar schema discovery timed out")?
        .map_err(|_| "Calendar schema discovery failed; no write was submitted")?;
    hosted::read_json(&result)
}

fn schema_property<'a>(
    root: &'a Value,
    node: &'a Value,
    name: &str,
    depth: usize,
) -> Option<&'a Value> {
    if depth > 8 {
        return None;
    }
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        return schema_property(
            root,
            root.pointer(reference.strip_prefix('#')?)?,
            name,
            depth + 1,
        );
    }
    node.get("properties")
        .and_then(|properties| properties.get(name))
        .or_else(|| {
            node.get("allOf")
                .and_then(Value::as_array)?
                .iter()
                .find_map(|node| schema_property(root, node, name, depth + 1))
        })
}

fn compatible_schema(schema: &Value, create: bool) -> bool {
    let event = schema["type"] == "object"
        && schema["title"] == "microsoft.graph.event"
        && ["start", "end", "showAs", "isAllDay"]
            .iter()
            .all(|name| schema_property(schema, schema, name, 0).is_some());
    let array = |node: &Value| {
        node.get("type").is_some_and(|kind| {
            kind == "array"
                || kind
                    .as_array()
                    .is_some_and(|types| types.iter().any(|kind| kind == "array"))
        })
    };
    // WorkIQ's event schema omits inherited outlookItem categories and subject.
    let categories = schema_property(schema, schema, "categories", 0).map_or(event, |categories| {
        categories.get("readOnly") != Some(&Value::Bool(true))
            && (array(categories)
                || categories
                    .get("anyOf")
                    .and_then(Value::as_array)
                    .is_some_and(|nodes| nodes.iter().any(array)))
    });
    categories
        && (!create
            || [
                "subject",
                "start",
                "end",
                "showAs",
                "transactionId",
                "isReminderOn",
            ]
            .iter()
            .all(|name| {
                schema_property(schema, schema, name, 0)
                    .map_or(event && *name == "subject", |property| {
                        property.get("readOnly") != Some(&Value::Bool(true))
                    })
            }))
}

fn submission_result(
    result: &rmcp::model::CallToolResult,
    operation: &Operation,
) -> Result<String, SubmissionError> {
    let payload = hosted::read_json_payload(result).map_err(|_| SubmissionError::Unknown)?;
    match payload["statusCode"].as_u64() {
        Some(401 | 403) => Err(SubmissionError::Denied),
        Some(status @ (400 | 404 | 409 | 412 | 422 | 429)) => Err(provider_error_code(&payload)
            .map_or(SubmissionError::Rejected(status as u16), |code| {
                SubmissionError::RejectedWithCode(status as u16, code)
            })),
        Some(200 | 201) if result.is_error != Some(true) => payload["data"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or(SubmissionError::Unknown),
        Some(204)
            if result.is_error != Some(true)
                && matches!(
                    operation.change,
                    Change::Category(_) | Change::ClearCategory | Change::Delete
                ) =>
        {
            Ok(operation.event.id.clone())
        }
        Some(202 | 204)
            if result.is_error != Some(true)
                && matches!(operation.change, Change::Respond { .. }) =>
        {
            Ok(operation.event.id.clone())
        }
        _ => Err(SubmissionError::Unknown),
    }
}

fn provider_error_code(payload: &Value) -> Option<&'static str> {
    // Persist known error identifiers, never arbitrary provider text or calendar data.
    const CODES: &[&str] = &[
        "BadRequest",
        "Request_BadRequest",
        "RequestBodyRead",
        "InvalidRequest",
        "ErrorInvalidRequest",
        "ErrorInvalidOperation",
        "ErrorInvalidIdMalformed",
        "ErrorItemNotFound",
        "ErrorAccessDenied",
        "Authorization_RequestDenied",
        "ErrorInvalidParameter",
        "ErrorInvalidProperty",
        "ErrorInvalidRecipients",
        "ErrorInvalidUser",
        "ErrorCalendarOccurrenceIsDeletedFromRecurrence",
        "ErrorCalendarCannotMoveOrCopyOccurrence",
        "ErrorOccurrenceCrossingBoundary",
        "ErrorIrresolvableConflict",
        "ErrorCannotDeleteObject",
        "TooManyRequests",
    ];
    fn read(node: &Value, depth: usize) -> Option<&'static str> {
        if depth > 3 {
            return None;
        }
        if let Some(code) = node.get("code").and_then(Value::as_str)
            && let Some(known) = CODES.iter().copied().find(|known| *known == code)
        {
            return Some(known);
        }
        if let Some(text) = node.as_str().filter(|text| text.len() <= 16_384) {
            let decoded: Value = serde_json::from_str(text).ok()?;
            return read(&decoded, depth + 1);
        }
        ["error", "data", "body"]
            .into_iter()
            .find_map(|key| node.get(key).and_then(|node| read(node, depth + 1)))
    }
    read(payload, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[tokio::test]
    #[ignore = "Requires an explicit saved approval fingerprint and app-owned WorkIQ sign-in; performs reads only"]
    async fn live_approved_category_readback_only() {
        let root =
            std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("Tomlook");
        let store = crate::storage::Store::open(&root).unwrap();
        let fingerprint = std::env::var("TOMLOOK_APPROVAL_READBACK_FINGERPRINT")
            .expect("Select the exact saved category approval explicitly");
        let records = store
            .recommendations()
            .unwrap()
            .into_values()
            .filter(|record| record.fingerprint == fingerprint)
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 1, "Select exactly one saved approval");
        let record = &records[0];
        let operation = store.calendar_approval(&record.id).unwrap();
        operation.validate(&record.account).unwrap();
        let categories = match &operation.change {
            Change::Category(category) => vec![category.clone()],
            Change::ClearCategory => Vec::new(),
            _ => panic!("Select a saved category approval"),
        };
        let original = &operation.event;
        let preferences = store.preferences().unwrap();
        let connection = Connection::connect_saved(&root, &preferences.work_email, false, |_| {})
            .await
            .unwrap();
        let current = Api::event(&connection, &original.id).await;
        let report = match current {
            Ok(current) => {
                let mut expected = original.clone();
                expected.category = categories.join(", ");
                expected
                    .extra
                    .insert("categories".into(), json!(categories));
                let occurrence_to_exception = expected.extra.get("type").and_then(Value::as_str)
                    == Some("occurrence")
                    && current.extra.get("type").and_then(Value::as_str) == Some("exception");
                if occurrence_to_exception {
                    expected.extra.insert("type".into(), json!("exception"));
                }
                let left = serde_json::to_value(&expected).unwrap();
                let right = serde_json::to_value(&current).unwrap();
                let changed = left
                    .as_object()
                    .unwrap()
                    .keys()
                    .filter(|key| {
                        key.as_str() != "briefing_status" && left.get(*key) != right.get(*key)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                json!({"exact_category":real_categories(&current)==categories,
                    "unchanged_other_fields":changed.is_empty(),"differing_field_names":changed,
                    "single_occurrence_became_exception":occurrence_to_exception,
                    "original_event_type":original.extra.get("type"),
                    "returned_event_type":current.extra.get("type"),
                    "native_verification_passed":verified(&current, &operation),
                    "live_mutations":0,"approved_operation_state":record.state})
            }
            Err(error) => json!({"read_error":error,"live_mutations":0}),
        };
        if let Some(path) = std::env::var_os("TOMLOOK_APPROVAL_READBACK_REPORT") {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .unwrap()
                .write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
                .unwrap();
        }
        connection.close().await.unwrap();
        assert!(
            report["exact_category"] == true
                && report["unchanged_other_fields"] == true
                && report["native_verification_passed"] == true,
            "{report}"
        );
    }

    #[tokio::test]
    #[ignore = "Reads live WorkIQ tool schemas with app-owned saved sign-in; never submits calendar writes"]
    async fn live_calendar_capabilities_readonly() {
        let root =
            std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("Tomlook");
        let preferences = crate::storage::Store::open(&root)
            .unwrap()
            .preferences()
            .unwrap();
        let connection = Connection::connect_saved(&root, &preferences.work_email, false, |_| {})
            .await
            .map_err(
                |_| "Saved WorkIQ connection could not be restored without interactive sign-in",
            )
            .unwrap();
        let category = Api::schema(&connection, false).await;
        let buffer = Api::schema(&connection, true).await;
        let actions = {
            let mut actions = Vec::new();
            for response in [
                MeetingResponse::Accept,
                MeetingResponse::Tentative,
                MeetingResponse::Decline,
                MeetingResponse::Cancel,
            ] {
                let change = Change::Respond {
                    response,
                    comment: String::new(),
                    send_response: true,
                };
                let result = Api::capability(&connection, &change).await;
                actions.push(json!({"action":response.action(),"compatible":result.is_ok(),"error":result.err()}));
            }
            actions
        };
        let deletion = Api::capability(&connection, &Change::Delete).await;
        fn shape(value: &Value, depth: usize) -> Value {
            if depth > 4 {
                return Value::Null;
            }
            match value {
                Value::Object(object) => Value::Object(
                    object
                        .iter()
                        .take(24)
                        .map(|(key, value)| {
                            (
                                key.clone(),
                                if matches!(key.as_str(), "type" | "$ref" | "readOnly") {
                                    value.clone()
                                } else {
                                    shape(value, depth + 1)
                                },
                            )
                        })
                        .collect(),
                ),
                Value::Array(values) => Value::Array(
                    values
                        .iter()
                        .take(3)
                        .map(|value| shape(value, depth + 1))
                        .collect(),
                ),
                _ => Value::Null,
            }
        }
        let category_shape = discover_schema(&connection, false)
            .await
            .map(|schema| shape(&schema, 0));
        let buffer_shape = discover_schema(&connection, true)
            .await
            .map(|schema| shape(&schema, 0));
        let title = discover_schema(&connection, false)
            .await
            .ok()
            .and_then(|schema| schema.get("title").cloned());
        let discovered = connection.peer.list_tools(None).await.unwrap();
        let schemas = discovered.tools.into_iter()
            .filter(|tool| matches!(tool.name.as_ref(), "create_entity" | "update_entity" | "get_schema" | "delete_entity" | "do_action"))
            .map(|tool| {
                let properties = tool.input_schema.get("properties").and_then(Value::as_object);
                let types = ["parentUrl", "entityUrl", "actionUrl", "jsonBody", "path", "operationType", "agentId"].into_iter()
                    .filter_map(|name| properties.and_then(|properties| properties.get(name)).map(|property|
                        (name.to_owned(), json!({"type":property.get("type"), "anyOf":property.get("anyOf").and_then(Value::as_array)
                            .map(|types| types.iter().map(|kind| kind.get("type")).collect::<Vec<_>>())}))))
                    .collect::<serde_json::Map<String, Value>>();
                json!({"tool":tool.name, "description":tool.description,
                    "json_body_schema":properties.and_then(|properties| properties.get("jsonBody")),
                    "required":tool.input_schema.get("required"), "parameter_types":types})
            }).collect::<Vec<_>>();
        let mut response_schemas = Vec::new();
        for action in [
            "decline",
            "tentativelyAccept",
            "microsoft.graph.tentativelyAccept",
        ] {
            let path = format!("/me/events/{{event-id}}/{action}");
            let request = CallToolRequestParams::new("get_schema").with_arguments(
                json!({"path":path, "operationType":"create", "format":"jsonschema"})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            let result =
                tokio::time::timeout(Duration::from_secs(30), connection.peer.call_tool(request))
                    .await
                    .unwrap()
                    .unwrap();
            let payload = hosted::read_json_payload(&result).unwrap();
            let schema = payload.get("data").unwrap_or(&payload);
            let parameters = ["comment", "sendResponse", "proposedNewTime"]
                .into_iter()
                .filter_map(|name| {
                    schema_property(schema, schema, name, 0)
                        .map(|property| (name.to_owned(), shape(property, 0)))
                })
                .collect::<serde_json::Map<_, _>>();
            response_schemas.push(json!({
                "path":path, "operation":"get_schema", "operation_type":"create",
                "http_status":payload.get("statusCode"),
                "error_code":provider_error_code(&payload),
                "title":schema.get("title"),
                "parameters":parameters,
                "tool_error":result.is_error
            }));
        }
        let report = json!({
            "registered_tools":connection.capabilities,
            "schema_title":title,
            "category_schema_shape":category_shape,
            "buffer_schema_shape":buffer_shape,
            "write_tool_metadata":schemas,
            "response_action_schema_probes":response_schemas,
            "response_action_body_is_string":connection.calendar_action_body_is_string,
            "category_metadata_compatible":category.is_ok(),
            "buffer_metadata_compatible":buffer.is_ok(),
            "category_error":category.as_ref().err(),
            "buffer_error":buffer.as_ref().err(),
            "response_tool_contracts":actions,
            "response_body_contract":"Native Graph action contracts with separately captured action-schema probe results",
            "delete_metadata_compatible":deletion.is_ok(),
            "delete_error":deletion.err(),
            "write_permission_verified":false,
            "calendar_mutations":0,
        });
        if let Some(path) = std::env::var_os("TOMLOOK_CALENDAR_CAPABILITIES_REPORT") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap();
            file.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
                .unwrap();
        }
        connection.close().await.unwrap();
        assert!(
            category.is_ok()
                && buffer.is_ok()
                && actions.iter().all(|action| action["compatible"] == true),
            "{report}"
        );
    }

    fn operation(buffer: bool) -> Operation {
        let now = Local::now().fixed_offset();
        let mut event = crate::calendar::fixture(1).remove(0);
        event.start = now + chrono::Duration::hours(2);
        event.end = event.start + chrono::Duration::hours(1);
        event.is_all_day = false;
        event.category = "Call, Travel".into();
        event
            .extra
            .insert("categories".into(), json!(["Call", "Travel"]));
        let suggestion = crate::recommendations::Suggestion {
            kind: if buffer {
                crate::recommendations::Kind::Travel
            } else {
                crate::recommendations::Kind::Category
            },
            reason: "Confirmed physical visit.".into(),
            reason_cs: "Potvrzena osobni navsteva.".into(),
            category: (!buffer).then(|| "Office".into()),
            minutes: buffer.then_some(30),
        };
        let mut recommendation =
            crate::recommendations::Recommendation::from_insights(&event, "owner", suggestion, now)
                .unwrap();
        recommendation.state = crate::recommendations::State::Dispatching;
        let change = if buffer {
            Change::Buffer {
                title: "Travel buffer".into(),
                start: event.start - chrono::Duration::minutes(30),
                end: event.start,
                category: None,
            }
        } else {
            Change::Category("Office".into())
        };
        Operation {
            recommendation,
            event,
            change,
        }
    }

    struct Fake {
        current: Event,
        written: Mutex<Option<Event>>,
        writes: AtomicUsize,
        rejection: Option<SubmissionError>,
        mismatch: bool,
        busy: bool,
        incomplete: bool,
        account: &'static str,
        removed: AtomicBool,
        stale_reads: AtomicUsize,
    }

    impl Fake {
        fn new(operation: &Operation) -> Self {
            Self {
                current: operation.event.clone(),
                written: Mutex::new(None),
                writes: AtomicUsize::new(0),
                rejection: None,
                mismatch: false,
                busy: false,
                incomplete: false,
                account: "owner",
                removed: AtomicBool::new(false),
                stale_reads: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Api for Fake {
        fn account(&self) -> &str {
            self.account
        }
        async fn schema(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        async fn capability(&self, _: &Change) -> Result<(), String> {
            Ok(())
        }
        async fn event(&self, _: &str) -> Result<Event, String> {
            if self.removed.load(Ordering::Relaxed) {
                return Err("HTTP 404".into());
            }
            if self.writes.load(Ordering::Relaxed) > 0
                && self
                    .stale_reads
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
            {
                return Ok(self.current.clone());
            }
            Ok(self
                .written
                .lock()
                .unwrap()
                .as_ref()
                .unwrap_or(&self.current)
                .clone())
        }
        async fn removed(&self, _: &str) -> Result<bool, String> {
            Ok(self.removed.load(Ordering::Relaxed))
        }
        async fn calendar(&self, range: Range) -> Result<Calendar, String> {
            let mut event = self.current.clone();
            event.start -= chrono::Duration::hours(1);
            Ok(Calendar::build(
                if self.busy { vec![event] } else { Vec::new() },
                if self.incomplete {
                    Default::default()
                } else {
                    (0..=(range.last - range.first).num_days())
                        .map(|offset| range.first + chrono::Duration::days(offset))
                        .collect()
                },
                Vec::new(),
            )
            .unwrap())
        }
        async fn submit(&self, operation: &Operation) -> Result<String, SubmissionError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.rejection {
                return Err(error);
            }
            let mut event = operation.event.clone();
            match &operation.change {
                Change::Category(category) => {
                    event.category = category.clone();
                    event.extra.insert("categories".into(), json!([category]));
                }
                Change::ClearCategory => {
                    event.category.clear();
                    event.extra.insert("categories".into(), json!([]));
                }
                Change::Respond {
                    response: MeetingResponse::Cancel,
                    ..
                } => {
                    event.extra.insert("isCancelled".into(), json!(true));
                }
                Change::Respond { response, .. } => {
                    event.status = response.status().into();
                }
                Change::Delete => {
                    self.removed.store(true, Ordering::Relaxed);
                    return Ok(operation.event.id.clone());
                }
                Change::Buffer {
                    title,
                    start,
                    end,
                    category,
                } => {
                    event.id = "created-buffer".into();
                    event.title = title.clone();
                    event.start = *start;
                    event.end = *end;
                    event.attendees.clear();
                    event.is_all_day = false;
                    event.category = category.clone().unwrap_or_default();
                    event.extra.insert("showAs".into(), json!("busy"));
                    event.extra.insert(
                        "categories".into(),
                        json!(category.iter().collect::<Vec<_>>()),
                    );
                }
            }
            if self.mismatch {
                event.title.push_str(" unexpected change");
            }
            let id = event.id.clone();
            *self.written.lock().unwrap() = Some(event);
            Ok(id)
        }
    }

    #[tokio::test]
    async fn exactly_one_category_or_solo_buffer_is_submitted_and_read_back() {
        for buffer in [false, true] {
            let operation = operation(buffer);
            let api = Fake::new(&operation);
            assert!(matches!(
                execute(&api, &operation).await,
                Outcome::Applied(_)
            ));
            assert_eq!(api.writes.load(Ordering::SeqCst), 1);
            if buffer {
                let request = request(&operation, false).unwrap();
                let body: Value =
                    serde_json::from_str(request.arguments.unwrap()["jsonBody"].as_str().unwrap())
                        .unwrap();
                assert_eq!(body["transactionId"], operation.recommendation.id);
                assert!(body.get("attendees").is_none());
                assert_eq!(body["categories"], json!([]));
            }
        }
    }

    #[tokio::test]
    async fn a_travel_buffer_inherits_only_its_uncertain_parent_category() {
        let mut operation = operation(true);
        operation.event.category = "nejist\u{e9}".into();
        operation
            .event
            .extra
            .insert("categories".into(), json!(["nejist\u{e9}"]));
        operation.recommendation = crate::recommendations::Recommendation::from_insights(
            &operation.event,
            "owner",
            operation.recommendation.suggestion.clone(),
            Local::now().fixed_offset(),
        )
        .unwrap();
        operation.recommendation.state = crate::recommendations::State::Dispatching;
        if let Change::Buffer { category, .. } = &mut operation.change {
            *category = Some("nejist\u{e9}".into());
        }
        let api = Fake::new(&operation);
        assert!(matches!(
            execute(&api, &operation).await,
            Outcome::Applied(_)
        ));
        assert_eq!(api.writes.load(Ordering::SeqCst), 1);
        let request = request(&operation, false).unwrap();
        let body: Value =
            serde_json::from_str(request.arguments.unwrap()["jsonBody"].as_str().unwrap()).unwrap();
        assert_eq!(body["categories"], json!(["nejist\u{e9}"]));
        assert!(body.get("attendees").is_none());
    }

    #[tokio::test]
    async fn stale_account_event_conflicting_or_uncovered_interval_never_submits() {
        for mode in 0..4 {
            let operation = operation(mode >= 2);
            let mut api = Fake::new(&operation);
            match mode {
                0 => api.account = "other",
                1 => api.current.title.push_str(" changed"),
                2 => api.busy = true,
                _ => api.incomplete = true,
            }
            assert!(matches!(
                execute(&api, &operation).await,
                Outcome::NotSent(_)
            ));
            assert_eq!(api.writes.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn denied_unknown_and_mismatched_outcomes_are_not_replayed() {
        for mode in 0..3 {
            let operation = operation(false);
            let mut api = Fake::new(&operation);
            match mode {
                0 => api.rejection = Some(SubmissionError::Denied),
                1 => api.rejection = Some(SubmissionError::Unknown),
                _ => api.mismatch = true,
            }
            let result = execute(&api, &operation).await;
            assert!(if mode == 0 {
                matches!(result, Outcome::NotSent(_))
            } else {
                matches!(result, Outcome::Review(_))
            });
            assert_eq!(api.writes.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn structured_policy_denials_survive_mcp_error_flag_and_schemas_resolve_local_refs() {
        let result: rmcp::model::CallToolResult = serde_json::from_value(json!({
            "content":[], "isError":true, "structuredContent":{"statusCode":403}
        }))
        .unwrap();
        assert!(matches!(
            submission_result(&result, &operation(false)),
            Err(SubmissionError::Denied)
        ));
        let schema = json!({"$ref":"#/$defs/event", "$defs":{"event":{"properties":{"categories":{"type":"array"}}}}});
        assert!(compatible_schema(&schema, false));
        assert!(!compatible_schema(&schema, true));
        assert!(!compatible_schema(
            &json!({"$ref":"#/$defs/event","$defs":{"event":{"$ref":"#/$defs/event"}}}),
            false
        ));
        assert!(!compatible_schema(
            &json!({"properties":{"categories":{"type":"array","readOnly":true}}}),
            false
        ));
        assert!(event_path("../users/other").is_err());
        assert!(event_path("event?query=1").is_err());
    }

    #[test]
    fn published_wire_contract_accepts_live_nullable_and_unconstrained_metadata() {
        let create: Tool = serde_json::from_value(json!({"name":"create_entity", "inputSchema":{
            "type":"object","properties":{"parentUrl":{"type":"string"},"jsonBody":{}},
            "required":["parentUrl","jsonBody"]}}))
        .unwrap();
        let schema: Tool = serde_json::from_value(json!({"name":"get_schema", "inputSchema":{
            "type":"object","properties":{"path":{"type":["string","null"]},"operationType":{"type":["string","null"]}}}})).unwrap();
        assert!(validate_tool(&create));
        assert!(validate_tool(&schema));
        let wrong: Tool = serde_json::from_value(json!({"name":"create_entity", "inputSchema":{
            "type":"object","properties":{"parentUrl":{"type":"string"},"jsonBody":{"type":"object"}},
            "required":["parentUrl","jsonBody"]}})).unwrap();
        assert!(!validate_tool(&wrong));
        let event = json!({"title":"microsoft.graph.event","type":"object",
            "properties":{"start":{},"end":{},"showAs":{},"isAllDay":{},"transactionId":{},"isReminderOn":{}}});
        assert!(compatible_schema(&event, false));
        assert!(compatible_schema(&event, true));
        let mut unknown = event.clone();
        unknown["title"] = "unknown".into();
        assert!(!compatible_schema(&unknown, false));
        let mut readonly = event;
        readonly["properties"]["categories"] = json!({"type":"array","readOnly":true});
        assert!(!compatible_schema(&readonly, false));
    }

    #[test]
    fn response_body_encoding_follows_discovered_provider_schema() {
        for (property, valid, encoded) in [
            (json!({}), true, false),
            (json!({"type":"object"}), true, false),
            (json!({"type":["object","null"]}), true, false),
            (json!({"type":"string"}), true, true),
            (json!({"type":["string","null"]}), true, true),
            (json!({"type":["object","string","null"]}), true, false),
            (json!({"type":"integer"}), false, false),
        ] {
            let tool: Tool = serde_json::from_value(json!({
                "name":"do_action", "inputSchema":{
                    "type":"object",
                    "properties":{"actionUrl":{"type":"string"},"jsonBody":property},
                    "required":["actionUrl","jsonBody"]
                }
            }))
            .unwrap();
            assert_eq!(validate_tool(&tool), valid);
            assert_eq!(action_body_is_string(&tool), encoded);
        }
    }

    #[test]
    fn category_wire_body_replaces_all_categories_and_never_updates_other_fields() {
        let mut event = crate::calendar::fixture(1).remove(0);
        event.start = Local::now().fixed_offset() + chrono::Duration::hours(2);
        event.end = event.start + chrono::Duration::hours(1);
        let suggestion = crate::recommendations::Suggestion {
            kind: crate::recommendations::Kind::Category,
            reason: "Office.".into(),
            reason_cs: "Kancelar.".into(),
            category: Some("Office".into()),
            minutes: None,
        };
        let mut recommendation = crate::recommendations::Recommendation::from_insights(
            &event,
            "owner",
            suggestion,
            Local::now().fixed_offset(),
        )
        .unwrap();
        recommendation.state = crate::recommendations::State::Dispatching;
        let operation = Operation {
            recommendation,
            event,
            change: Change::Category("Office".into()),
        };
        let request = request(&operation, false).unwrap();
        assert_eq!(request.name, "update_entity");
        let body: Value =
            serde_json::from_str(request.arguments.unwrap()["jsonBody"].as_str().unwrap()).unwrap();
        assert_eq!(body, json!({"categories":["Office"]}));
    }

    fn manual_operation(change: Change, organizer: bool) -> Operation {
        let mut event = operation(false).event;
        event.extra.insert("type".into(), json!("occurrence"));
        event.extra.insert("isOrganizer".into(), json!(organizer));
        event.organizer = "organizer@example.test".into();
        event.status = if organizer {
            "organizer"
        } else {
            "notResponded"
        }
        .into();
        if matches!(change, Change::Delete) {
            event.attendees.clear();
        }
        let mut recommendation = crate::recommendations::Recommendation::from_manual(
            &event,
            "owner",
            change.clone(),
            Local::now().fixed_offset(),
        )
        .unwrap();
        recommendation.state = crate::recommendations::State::Dispatching;
        Operation {
            event,
            change,
            recommendation,
        }
    }

    #[tokio::test]
    async fn manual_responses_are_single_submission_with_exact_native_requests() {
        for response in [
            MeetingResponse::Accept,
            MeetingResponse::Tentative,
            MeetingResponse::Decline,
            MeetingResponse::Cancel,
        ] {
            let operation = manual_operation(
                Change::Respond {
                    response,
                    comment: "Confirmed comment.".into(),
                    send_response: true,
                },
                response == MeetingResponse::Cancel,
            );
            let wire = request(&operation, false).unwrap();
            assert_eq!(wire.name, "do_action");
            let arguments = wire.arguments.unwrap();
            assert_eq!(
                arguments["actionUrl"],
                format!("/me/events/{}/{}", operation.event.id, response.action())
            );
            let body = &arguments["jsonBody"];
            assert!(body.is_object());
            assert_eq!(
                *body,
                if response == MeetingResponse::Cancel {
                    json!({"comment":"Confirmed comment."})
                } else {
                    json!({"comment":"Confirmed comment.","sendResponse":true})
                }
            );
            let encoded = request(&operation, true).unwrap().arguments.unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(encoded["jsonBody"].as_str().unwrap()).unwrap(),
                *body
            );
            assert_eq!(
                submission_result(
                    &rmcp::model::CallToolResult::structured(json!({"statusCode":202})),
                    &operation
                )
                .unwrap(),
                operation.event.id
            );
            let fake = Fake::new(&operation);
            fake.stale_reads.store(2, Ordering::Relaxed);
            assert!(matches!(
                execute(&fake, &operation).await,
                Outcome::Applied(_)
            ));
            assert_eq!(fake.writes.load(Ordering::Relaxed), 1);
        }
        let operation = manual_operation(
            Change::Respond {
                response: MeetingResponse::Decline,
                comment: String::new(),
                send_response: false,
            },
            false,
        );
        let body = request(&operation, false).unwrap().arguments.unwrap()["jsonBody"].clone();
        assert_eq!(body, json!({"sendResponse":false}));
        assert!(body["sendResponse"].is_boolean());
    }

    #[test]
    fn responses_without_comments_omit_the_optional_parameter_and_preserve_notification_choice() {
        for response in [
            MeetingResponse::Accept,
            MeetingResponse::Tentative,
            MeetingResponse::Decline,
        ] {
            for send_response in [false, true] {
                let operation = manual_operation(
                    Change::Respond {
                        response,
                        comment: String::new(),
                        send_response,
                    },
                    false,
                );
                for encoded in [false, true] {
                    let arguments = request(&operation, encoded).unwrap().arguments.unwrap();
                    let body = if encoded {
                        serde_json::from_str::<Value>(arguments["jsonBody"].as_str().unwrap())
                            .unwrap()
                    } else {
                        arguments["jsonBody"].clone()
                    };
                    assert_eq!(body, json!({"sendResponse":send_response}));
                }
            }
        }
    }

    #[tokio::test]
    async fn rejection_codes_are_safe_persistent_and_never_trigger_a_write_retry() {
        let operation = manual_operation(
            Change::Respond {
                response: MeetingResponse::Decline,
                comment: String::new(),
                send_response: false,
            },
            false,
        );
        let error = submission_result(&rmcp::model::CallToolResult::structured(json!({
            "statusCode":400,
            "data":{"error":{"code":"ErrorInvalidOperation","message":"PRIVATE body and person@example.test"}}
        })), &operation).unwrap_err();
        assert!(matches!(
            error,
            SubmissionError::RejectedWithCode(400, "ErrorInvalidOperation")
        ));
        let mut fake = Fake::new(&operation);
        fake.rejection = Some(error);
        let Outcome::NotSent(message) = execute(&fake, &operation).await else {
            panic!("Expected definitive rejection");
        };
        assert!(message.contains("HTTP 400 (ErrorInvalidOperation)"));
        assert!(!message.contains("PRIVATE"));
        assert!(!message.contains("person@"));
        assert_eq!(fake.writes.load(Ordering::Relaxed), 1);
        for code in [
            "person@example.test",
            "private-token",
            "UnknownPrivateIdentifier",
        ] {
            assert!(matches!(
                submission_result(
                    &rmcp::model::CallToolResult::structured(json!({
                        "statusCode":400, "error":{"code":code,"message":"private"}
                    })),
                    &operation
                ),
                Err(SubmissionError::Rejected(400))
            ));
        }
        fake.rejection = Some(SubmissionError::InvalidRequest);
        let Outcome::NotSent(message) = execute(&fake, &operation).await else {
            panic!("Expected local validation failure");
        };
        assert!(!message.contains("HTTP"));
        assert!(message.contains("no write was submitted"));
    }

    #[test]
    fn encoded_error_envelopes_retain_only_known_codes_with_bounded_decoding() {
        let error = json!({
            "error":{"code":"RequestBodyRead","message":"PRIVATE person@example.test"}
        });
        for payload in [
            json!({"data":error.to_string()}),
            json!({"body":error.to_string()}),
            json!({"error":{"code":"RequestBodyRead","message":"PRIVATE"}}),
            json!({"data":{"body":error}}),
        ] {
            assert_eq!(provider_error_code(&payload), Some("RequestBodyRead"));
        }
        for payload in [
            json!({"data":"not JSON; PRIVATE person@example.test"}),
            json!({"data":json!({"error":{"code":"person@example.test"}}).to_string()}),
            json!({"body":"x".repeat(16_385)}),
            json!({"data":{"body":{"data":{"error":{"code":"RequestBodyRead"}}}}}),
        ] {
            assert_eq!(provider_error_code(&payload), None);
        }
    }

    #[tokio::test]
    async fn manual_clear_delete_and_role_staleness_follow_verified_outcome_rules() {
        let operation = manual_operation(Change::ClearCategory, false);
        let body: Value = serde_json::from_str(
            request(&operation, false).unwrap().arguments.unwrap()["jsonBody"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body, json!({"categories":[]}));
        assert!(matches!(
            execute(&Fake::new(&operation), &operation).await,
            Outcome::Applied(_)
        ));
        let operation = manual_operation(Change::Delete, true);
        assert_eq!(request(&operation, false).unwrap().name, "delete_entity");
        let fake = Fake::new(&operation);
        assert!(
            matches!(execute(&fake, &operation).await, Outcome::Removed(id) if id == operation.event.id)
        );
        assert_eq!(fake.writes.load(Ordering::Relaxed), 1);
        let operation = manual_operation(
            Change::Respond {
                response: MeetingResponse::Accept,
                comment: String::new(),
                send_response: true,
            },
            false,
        );
        let mut fake = Fake::new(&operation);
        fake.current.extra.insert("isOrganizer".into(), json!(true));
        assert!(matches!(
            execute(&fake, &operation).await,
            Outcome::NotSent(_)
        ));
        assert_eq!(fake.writes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn only_an_exact_single_resource_404_proves_removal() {
        use rmcp::model::CallToolResult;
        assert_eq!(
            removal_result(&CallToolResult::structured(
                json!({"results":[{"statusCode":404}]})
            )),
            Ok(true)
        );
        assert_eq!(
            removal_result(&CallToolResult::structured(
                json!({"results":[{"statusCode":200}]})
            )),
            Ok(false)
        );
        for payload in [
            json!({"results":[{"statusCode":401}]}),
            json!({"results":[{"statusCode":403}]}),
            json!({"results":[{"statusCode":500}]}),
            json!({"statusCode":404}),
            json!({"results":[{"statusCode":404},{"statusCode":404}]}),
            json!({"results":[]}),
        ] {
            assert!(removal_result(&CallToolResult::structured(payload)).is_err());
        }
    }

    #[tokio::test]
    async fn recurring_exception_recheck_never_repeats_the_write() {
        let operation = manual_operation(Change::Category("Office".into()), false);
        let fake = Fake::new(&operation);
        let mut returned = operation.event.clone();
        returned.category = "Office".into();
        returned
            .extra
            .insert("categories".into(), json!(["Office"]));
        returned.extra.insert("type".into(), json!("exception"));
        *fake.written.lock().unwrap() = Some(returned.clone());
        assert!(matches!(
            verify_only(&fake, &operation).await,
            Outcome::Applied(_)
        ));
        assert_eq!(fake.writes.load(Ordering::Relaxed), 0);
        returned.extra.insert("type".into(), json!("seriesMaster"));
        assert!(!verified(&returned, &operation));
        returned
            .extra
            .insert("type".into(), json!("singleInstance"));
        assert!(!verified(&returned, &operation));
        returned.extra.insert("type".into(), json!("exception"));
        let mut unexpected = operation.clone();
        unexpected.event.extra.remove("type");
        assert!(!verified(&returned, &unexpected));
        unexpected
            .event
            .extra
            .insert("type".into(), json!("exception"));
        returned.extra.insert("type".into(), json!("occurrence"));
        assert!(!verified(&returned, &unexpected));
        returned.extra.insert("type".into(), json!("exception"));
        returned.start += chrono::Duration::minutes(1);
        assert!(!verified(&returned, &operation));
    }
}
