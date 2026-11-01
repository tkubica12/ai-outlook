//! Teams proposals: typed, explicitly approved writes through WorkIQ's generic `create_entity`
//! tool with fixed paths (`POST /chats`, `POST /chats/{id}/messages`), plus read-only capability
//! discovery (`get_schema`) and member resolution. These operations are never offered to a model
//! as tools; only the storage worker's single-use approval reaches [`propose`].

use super::{Connection, calendar_write::SubmissionError, hosted};
use crate::scheduling::{
    ChatCheck, ChatRequest, ChatResult, ChatState, Member, UNCERTAIN_CHAT, chat_body, message_body,
    message_matches, valid_chat_id, valid_user_id,
};
use async_trait::async_trait;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::time::Duration;

pub const UNSUPPORTED: &str = "Teams chats cannot be created through your WorkIQ connection; propose by e-mail instead";
pub const NOT_FOUND: &str = "Not found in your organization's directory";
const DIRECTORY_DENIED: &str = "Directory lookups are not permitted for this account; propose by e-mail instead";
/// Created chats whose members cannot be verified receive no message.
const MEMBERS_UNVERIFIED: &str = "The Teams chat was created, but its members could not be verified, so no message was sent. Open the chat in Teams.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    pub id: String,
    pub web_url: Option<String>,
}

/// The provider operations of a Teams proposal; [`Connection`] implements them with WorkIQ.
#[async_trait]
pub trait Api: Send + Sync {
    fn account(&self) -> &str;
    /// Whether chats and chat messages can be created; reads schemas only.
    async fn capability(&self) -> Result<(), String>;
    async fn create_chat(&self, body: &Value) -> Result<Created, SubmissionError>;
    /// User IDs of the chat's members.
    async fn members(&self, chat: &str) -> Result<Vec<String>, String>;
    async fn send(&self, chat: &str, body: &Value) -> Result<String, SubmissionError>;
    async fn message(&self, chat: &str, id: &str) -> Result<Value, String>;
}

fn chat_path(chat: &str) -> Result<String, String> {
    if !valid_chat_id(chat) {
        return Err("Invalid Teams chat identity".into());
    }
    Ok(format!("/chats/{chat}"))
}

fn valid_message_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Creates the approved chat, verifies that exactly the approved people are its members, sends the
/// one approved message and reads it back. Each write is submitted at most once; an uncertain
/// outcome is reported for review and never repeated.
pub async fn propose(api: &impl Api, request: &ChatRequest) -> ChatResult {
    let mut result = ChatResult {
        proposal: request.proposal.clone(),
        state: ChatState::NotSent,
        chat_id: None,
        web_url: None,
        message_id: None,
        message: String::new(),
    };
    let checked = async {
        if request.account != api.account() {
            return Err("Calendar account changed; nothing was sent".to_owned());
        }
        let chat = chat_body(&request.draft, &request.account)?;
        let message = message_body(&request.draft.message)?;
        api.capability().await?;
        Ok((chat, message))
    }
    .await;
    let (chat, message) = match checked {
        Ok(bodies) => bodies,
        Err(error) => {
            result.message = error;
            return result;
        }
    };
    let created = match api.create_chat(&chat).await {
        Ok(created) => created,
        Err(SubmissionError::Unknown) => {
            result.state = ChatState::Review;
            result.message = UNCERTAIN_CHAT.into();
            return result;
        }
        Err(error) => {
            result.message = rejection("chat", error);
            return result;
        }
    };
    result.chat_id = Some(created.id.clone());
    result.web_url = created.web_url.clone();
    let expected = request.draft.members.iter().map(|member| member.user_id.to_ascii_lowercase()).collect::<std::collections::BTreeSet<_>>();
    match api.members(&created.id).await {
        Ok(ids) => {
            let actual = ids.iter().map(|id| id.to_ascii_lowercase()).collect::<std::collections::BTreeSet<_>>();
            // You plus exactly the approved people; nobody else receives the message.
            if actual.len() != ids.len() || actual.len() != expected.len() + 1 || !expected.is_subset(&actual) {
                result.message = "The Teams chat does not have exactly the approved members, so no message was sent. Open the chat in Teams.".into();
                return result;
            }
        }
        Err(_) => {
            result.message = MEMBERS_UNVERIFIED.into();
            return result;
        }
    }
    let id = match api.send(&created.id, &message).await {
        Ok(id) => id,
        Err(SubmissionError::Unknown) => {
            result.state = ChatState::Review;
            result.message = "The message may have been sent. Open the Teams chat before sending anything again.".into();
            return result;
        }
        Err(error) => {
            result.message = rejection("message", error);
            return result;
        }
    };
    result.message_id = Some(id.clone());
    for attempt in 0..3 {
        if let Ok(value) = api.message(&created.id, &id).await
            && value.get("id").and_then(Value::as_str) == Some(id.as_str())
            && message_matches(&value, &request.draft.message)
        {
            result.state = ChatState::Sent;
            result.message = "Teams chat created and the message was read back".into();
            return result;
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    result.state = ChatState::Review;
    result.message = "The message was sent but could not be read back. Open the Teams chat to check it.".into();
    result
}

fn rejection(kind: &str, error: SubmissionError) -> String {
    let status = match error {
        SubmissionError::Rejected(status) => status.to_string(),
        SubmissionError::RejectedWithCode(status, code) => format!("{status}, {code}"),
        SubmissionError::Unknown => return UNCERTAIN_CHAT.into(),
        SubmissionError::InvalidRequest => return "The approved Teams request is invalid; nothing was sent.".into(),
        SubmissionError::Denied => String::new(),
    };
    match (kind, status.is_empty()) {
        ("chat", true) => "Teams denied creating the chat; nothing was sent.".into(),
        ("chat", false) => format!("Teams rejected the chat (HTTP {status}); nothing was sent. Some people may not be reachable in Teams."),
        (_, true) => "Teams denied the message; the chat exists without it.".into(),
        (_, false) => format!("Teams rejected the message (HTTP {status}); the chat exists without it."),
    }
}

/// Whether the provider's schema can create a chat: a writable `chatType` and, when described,
/// writable members.
fn compatible_chat_schema(schema: &Value) -> bool {
    let property = |name| super::calendar_write::schema_property(schema, schema, name, 0);
    schema.is_object()
        && property("chatType").is_some_and(|property| property.get("readOnly") != Some(&Value::Bool(true)))
        && property("members").is_none_or(|property| property.get("readOnly") != Some(&Value::Bool(true)))
}

/// Whether the provider's schema can create a chat message with a writable `body`.
fn compatible_message_schema(schema: &Value) -> bool {
    schema.is_object()
        && super::calendar_write::schema_property(schema, schema, "body", 0)
            .is_some_and(|property| property.get("readOnly") != Some(&Value::Bool(true)))
}

const CHATS: &str = "/chats";
const MESSAGES: &str = "/chats/{chat-id}/messages";

async fn schema(connection: &Connection, path: &str) -> Result<Value, String> {
    if !connection.calendar_write_tools.contains("create_entity") || !connection.calendar_write_tools.contains("get_schema") {
        return Err(UNSUPPORTED.into());
    }
    if !connection.is_active() {
        return Err("WorkIQ is disconnected; sign in again".into());
    }
    let request = CallToolRequestParams::new("get_schema").with_arguments(
        json!({"path": path, "operationType": "create", "format": "jsonschema"})
            .as_object()
            .cloned()
            .ok_or("Invalid Teams schema request")?,
    );
    let result = tokio::time::timeout(Duration::from_secs(30), connection.peer.call_tool(request))
        .await
        .map_err(|_| "Teams schema discovery timed out")?
        .map_err(|_| "Teams schema discovery failed; nothing was sent")?;
    let value = hosted::read_json(&result)?;
    Ok(value.get("data").cloned().unwrap_or(value))
}

/// Whether this connection can create Teams chats and chat messages; reads schemas only.
pub async fn capability(connection: &Connection) -> Result<(), String> {
    let chats = schema(connection, CHATS).await.map_err(|_| UNSUPPORTED.to_owned())?;
    let messages = schema(connection, MESSAGES).await.map_err(|_| UNSUPPORTED.to_owned())?;
    if !compatible_chat_schema(&chats) || !compatible_message_schema(&messages) {
        return Err(UNSUPPORTED.into());
    }
    Ok(())
}

/// What the provider's create schemas say about chats and chat messages, for the read-only live
/// probe: metadata only, never chat content.
pub async fn schema_shapes(connection: &Connection) -> Value {
    let mut report = serde_json::Map::new();
    for (key, path, compatible) in [
        ("chats_create", CHATS, compatible_chat_schema as fn(&Value) -> bool),
        ("chat_messages_create", MESSAGES, compatible_message_schema),
    ] {
        report.insert(key.into(), match schema(connection, path).await {
            Ok(schema) => json!({
                "compatible": compatible(&schema),
                "shape": super::calendar_write::schema_shape(&schema, &schema, 0),
            }),
            Err(error) => json!({"error": error}),
        });
    }
    Value::Object(report)
}

fn user_resource(email: &str) -> Result<String, String> {
    let mut url = url::Url::parse("https://graph.microsoft.com/users").map_err(|_| "Invalid directory path")?;
    url.path_segments_mut().map_err(|_| "Invalid directory path")?.push(email);
    url.query_pairs_mut().append_pair("$select", "id,displayName,userPrincipalName,mail");
    Ok(format!("{}?{}", url.path(), url.query().unwrap_or_default()))
}

fn mail_resource(email: &str) -> Result<String, String> {
    let mut url = url::Url::parse("https://graph.microsoft.com/users").map_err(|_| "Invalid directory path")?;
    url.query_pairs_mut()
        .append_pair("$filter", &format!("mail eq '{}'", email.replace('\'', "''")))
        .append_pair("$select", "id,displayName,userPrincipalName,mail")
        .append_pair("$top", "2");
    Ok(format!("{}?{}", url.path(), url.query().unwrap_or_default()))
}

fn member(email: &str, user: &Value) -> Option<Member> {
    let id = user.get("id").and_then(Value::as_str).filter(|id| valid_user_id(id))?;
    let name = user
        .get("displayName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control() && !matches!(ch, '<' | '>' | '"'))
        .take(crate::attendees::NAME_LIMIT)
        .collect::<String>();
    Some(Member { email: email.to_owned(), name: name.trim().to_owned(), user_id: id.to_owned() })
}

/// Read-only Teams check: the provider supports chats, and each address resolves to exactly one
/// user of your organization. People who cannot be resolved are reported, never guessed.
pub async fn check(connection: &Connection, emails: &[String]) -> Result<ChatCheck, String> {
    if emails.is_empty() || emails.len() > crate::attendees::LIMIT {
        return Err("A Teams proposal needs 1 to 50 people".into());
    }
    capability(connection).await?;
    let mut check = ChatCheck::default();
    for email in emails {
        if crate::calendar::person_email(email).as_deref() != Some(email.as_str()) {
            check.unresolved.push((email.clone(), "Enter a valid e-mail address or choose a suggestion".into()));
            continue;
        }
        let direct = connection.fetch_resource_in(&user_resource(email)?, &connection.admission.lookup).await;
        let found = match direct {
            Ok(user) => member(email, &user),
            Err(error) if error.ends_with("HTTP 403") => return Err(DIRECTORY_DENIED.into()),
            Err(error) if error.ends_with("HTTP 404") || error.ends_with("HTTP 400") => {
                match connection.fetch_resource_in(&mail_resource(email)?, &connection.admission.lookup).await {
                    Ok(found) => match found.get("value").and_then(Value::as_array).map(Vec::as_slice) {
                        Some([user]) => member(email, user),
                        _ => None,
                    },
                    Err(error) if error.ends_with("HTTP 403") => return Err(DIRECTORY_DENIED.into()),
                    Err(error) => {
                        check.unresolved.push((email.clone(), error));
                        continue;
                    }
                }
            }
            Err(error) => {
                check.unresolved.push((email.clone(), error));
                continue;
            }
        };
        match found {
            Some(member) if !check.members.iter().any(|known| known.user_id.eq_ignore_ascii_case(&member.user_id)) => check.members.push(member),
            Some(_) => check.unresolved.push((email.clone(), "The same person is listed twice".into())),
            None => check.unresolved.push((email.clone(), NOT_FOUND.into())),
        }
    }
    Ok(check)
}

fn submission(result: &rmcp::model::CallToolResult) -> Result<Value, SubmissionError> {
    let payload = hosted::read_json_payload(result).map_err(|_| SubmissionError::Unknown)?;
    match payload["statusCode"].as_u64() {
        Some(401 | 403) => Err(SubmissionError::Denied),
        Some(status @ (400 | 404 | 409 | 412 | 422 | 429)) => Err(super::calendar_write::provider_error_code(&payload)
            .map_or(SubmissionError::Rejected(status as u16), |code| SubmissionError::RejectedWithCode(status as u16, code))),
        Some(200 | 201) if result.is_error != Some(true) => Ok(match &payload["data"] {
            Value::String(text) => serde_json::from_str(text).map_err(|_| SubmissionError::Unknown)?,
            data => data.clone(),
        }),
        _ => Err(SubmissionError::Unknown),
    }
}

fn created_chat(data: &Value) -> Result<Created, SubmissionError> {
    let id = data.get("id").and_then(Value::as_str).filter(|id| valid_chat_id(id)).ok_or(SubmissionError::Unknown)?;
    Ok(Created {
        id: id.to_owned(),
        web_url: data
            .get("webUrl")
            .and_then(Value::as_str)
            .filter(|url| url.len() <= 4000 && crate::calendar::safe_url(url))
            .map(str::to_owned),
    })
}

impl Connection {
    async fn create_in(&self, parent: &str, body: &Value) -> Result<Value, SubmissionError> {
        if !self.is_active() || !self.calendar_write_tools.contains("create_entity") {
            return Err(SubmissionError::InvalidRequest);
        }
        let arguments = json!({"parentUrl": parent, "jsonBody": body.to_string()});
        let request = CallToolRequestParams::new("create_entity")
            .with_arguments(arguments.as_object().cloned().ok_or(SubmissionError::InvalidRequest)?);
        let result = tokio::time::timeout(Duration::from_secs(60), self.peer.call_tool(request))
            .await
            .map_err(|_| SubmissionError::Unknown)?
            .map_err(|_| SubmissionError::Unknown)?;
        submission(&result)
    }
}

#[async_trait]
impl Api for Connection {
    fn account(&self) -> &str {
        &self.account
    }

    async fn capability(&self) -> Result<(), String> {
        capability(self).await
    }

    async fn create_chat(&self, body: &Value) -> Result<Created, SubmissionError> {
        created_chat(&self.create_in(CHATS, body).await?)
    }

    async fn members(&self, chat: &str) -> Result<Vec<String>, String> {
        let data = self.fetch_resource_in(&format!("{}/members", chat_path(chat)?), &self.admission.lookup).await?;
        let rows = data.get("value").and_then(Value::as_array).ok_or("Teams members are unavailable")?;
        if rows.len() > crate::attendees::LIMIT + 1 {
            return Err("The Teams chat has more members than were approved".into());
        }
        rows.iter()
            .map(|row| row.get("userId").and_then(Value::as_str).filter(|id| valid_user_id(id)).map(str::to_owned)
                .ok_or_else(|| "A Teams member has no verifiable identity".to_owned()))
            .collect()
    }

    async fn send(&self, chat: &str, body: &Value) -> Result<String, SubmissionError> {
        let parent = format!("{}/messages", chat_path(chat).map_err(|_| SubmissionError::InvalidRequest)?);
        let data = self.create_in(&parent, body).await?;
        data.get("id").and_then(Value::as_str).filter(|id| valid_message_id(id)).map(str::to_owned).ok_or(SubmissionError::Unknown)
    }

    async fn message(&self, chat: &str, id: &str) -> Result<Value, String> {
        if !valid_message_id(id) {
            return Err("Invalid Teams message identity".into());
        }
        self.fetch_resource_in(&format!("{}/messages/{id}", chat_path(chat)?), &self.admission.lookup).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduling::ChatDraft;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct Fake {
        unsupported: bool,
        create: Result<Created, SubmissionError>,
        members: Option<Vec<String>>,
        send: Result<String, SubmissionError>,
        readback: Option<String>,
        writes: AtomicUsize,
        bodies: Mutex<Vec<Value>>,
    }

    fn fake() -> Fake {
        Fake {
            unsupported: false,
            create: Ok(Created { id: "19:abc@thread.v2".into(), web_url: Some("https://teams.microsoft.com/l/chat/19%3Aabc".into()) }),
            members: Some(vec![
                "ffffffff-0000-4000-8000-00000000000f".into(),
                "aaaaaaaa-0000-4000-8000-00000000000a".into(),
                "bbbbbbbb-0000-4000-8000-00000000000b".into(),
            ]),
            send: Ok("1616965872395".into()),
            readback: None,
            writes: AtomicUsize::new(0),
            bodies: Mutex::new(Vec::new()),
        }
    }

    #[async_trait]
    impl Api for Fake {
        fn account(&self) -> &str { "owner@example.com" }
        async fn capability(&self) -> Result<(), String> {
            if self.unsupported { Err(UNSUPPORTED.into()) } else { Ok(()) }
        }
        async fn create_chat(&self, body: &Value) -> Result<Created, SubmissionError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.bodies.lock().unwrap().push(body.clone());
            self.create.clone()
        }
        async fn members(&self, _: &str) -> Result<Vec<String>, String> {
            self.members.clone().ok_or_else(|| "Hosted WorkIQ read returned HTTP 500".into())
        }
        async fn send(&self, _: &str, body: &Value) -> Result<String, SubmissionError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.bodies.lock().unwrap().push(body.clone());
            self.send.clone()
        }
        async fn message(&self, _: &str, id: &str) -> Result<Value, String> {
            let body = self.bodies.lock().unwrap().last().cloned().unwrap();
            let mut value = json!({"id": id, "body": body["body"]});
            if let Some(text) = &self.readback { value["body"] = json!({"contentType":"text","content":text}); }
            Ok(value)
        }
    }

    fn request() -> ChatRequest {
        let proposal = crate::scheduling::tests::proposal(crate::i18n::Language::En);
        let members = vec![
            Member { email: "alex@example.com".into(), name: "Alex Rivers".into(), user_id: "aaaaaaaa-0000-4000-8000-00000000000a".into() },
            Member { email: "jamie@example.com".into(), name: "Jamie Brooks".into(), user_id: "bbbbbbbb-0000-4000-8000-00000000000b".into() },
        ];
        let message = proposal.chat_message(&members);
        ChatRequest { proposal: proposal.id, account: "owner@example.com".into(),
            draft: ChatDraft { members, topic: Some("Pilot kickoff".into()), message } }
    }

    #[tokio::test]
    async fn the_approved_chat_is_created_verified_and_messaged_exactly_once() {
        let fake = fake();
        let result = propose(&fake, &request()).await;
        assert_eq!(result.state, ChatState::Sent, "{}", result.message);
        assert_eq!(result.chat_id.as_deref(), Some("19:abc@thread.v2"));
        assert_eq!(result.message_id.as_deref(), Some("1616965872395"));
        assert_eq!(fake.writes.load(Ordering::SeqCst), 2);
        let bodies = fake.bodies.lock().unwrap();
        assert_eq!(bodies[0]["chatType"], "group");
        assert_eq!(bodies[0]["topic"], "Pilot kickoff");
        assert_eq!(bodies[0]["members"].as_array().unwrap().len(), 3);
        assert_eq!(bodies[1]["body"]["contentType"], "html");
    }

    #[tokio::test]
    async fn unsupported_rejected_unverified_and_uncertain_outcomes_never_send_or_repeat() {
        let mut unsupported = fake();
        unsupported.unsupported = true;
        let result = propose(&unsupported, &request()).await;
        assert_eq!((result.state, result.message.as_str()), (ChatState::NotSent, UNSUPPORTED));
        assert_eq!(unsupported.writes.load(Ordering::SeqCst), 0);
        let mut other = request();
        other.account = "someone@example.com".into();
        assert_eq!(propose(&fake(), &other).await.state, ChatState::NotSent);
        let mut rejected = fake();
        rejected.create = Err(SubmissionError::RejectedWithCode(400, "BadRequest"));
        let result = propose(&rejected, &request()).await;
        assert_eq!(result.state, ChatState::NotSent);
        assert!(result.message.contains("HTTP 400") && result.chat_id.is_none());
        assert_eq!(rejected.writes.load(Ordering::SeqCst), 1);
        let mut uncertain = fake();
        uncertain.create = Err(SubmissionError::Unknown);
        let result = propose(&uncertain, &request()).await;
        assert_eq!((result.state, result.message.as_str()), (ChatState::Review, UNCERTAIN_CHAT));
        assert_eq!(uncertain.writes.load(Ordering::SeqCst), 1, "Never retried");
        for members in [None, Some(vec!["ffffffff-0000-4000-8000-00000000000f".to_owned(), "aaaaaaaa-0000-4000-8000-00000000000a".to_owned()]),
            Some(vec!["ffffffff-0000-4000-8000-00000000000f".to_owned(), "aaaaaaaa-0000-4000-8000-00000000000a".to_owned(),
                "bbbbbbbb-0000-4000-8000-00000000000b".to_owned(), "eeeeeeee-0000-4000-8000-00000000000e".to_owned()])] {
            let mut wrong = fake();
            wrong.members = members;
            let result = propose(&wrong, &request()).await;
            assert_eq!(result.state, ChatState::NotSent, "{}", result.message);
            assert!(result.chat_id.is_some(), "The created chat is reported with its link");
            assert_eq!(wrong.writes.load(Ordering::SeqCst), 1, "No message unless exactly the approved members are in the chat");
        }
        let mut lost = fake();
        lost.send = Err(SubmissionError::Unknown);
        let result = propose(&lost, &request()).await;
        assert_eq!(result.state, ChatState::Review);
        assert_eq!(lost.writes.load(Ordering::SeqCst), 2);
        let mut refused = fake();
        refused.send = Err(SubmissionError::Denied);
        let result = propose(&refused, &request()).await;
        assert_eq!(result.state, ChatState::NotSent);
        assert!(result.chat_id.is_some() && result.message.contains("without it"));
        let mut changed = fake();
        changed.readback = Some("Something else".into());
        let result = propose(&changed, &request()).await;
        assert_eq!(result.state, ChatState::Review, "An unverified message needs a look in Teams");
        assert_eq!(changed.writes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn schemas_directory_paths_and_created_chats_are_validated() {
        let chat = json!({"title":"microsoft.graph.chat","type":"object","properties":{
            "chatType":{"type":"string"},"topic":{"type":["string","null"]},
            "members":{"type":"array","items":{"$ref":"#/$defs/member"}}},
            "$defs":{"member":{"type":"object"}}});
        assert!(compatible_chat_schema(&chat));
        let mut read_only = chat.clone();
        read_only["properties"]["chatType"]["readOnly"] = json!(true);
        assert!(!compatible_chat_schema(&read_only));
        assert!(!compatible_chat_schema(&json!({"type":"object","properties":{"topic":{}}})));
        assert!(compatible_message_schema(&json!({"type":"object","properties":{"body":{"type":"object"}}})));
        assert!(!compatible_message_schema(&json!({"type":"object","properties":{"body":{"readOnly":true}}})));
        assert_eq!(user_resource("alex@example.com").unwrap(), "/users/alex@example.com?%24select=id%2CdisplayName%2CuserPrincipalName%2Cmail");
        assert!(mail_resource("o'neil@example.com").unwrap().contains("o%27%27neil"), "Quotes are escaped for OData");
        assert_eq!(created_chat(&json!({"id":"19:abc@thread.v2","webUrl":"https://teams.microsoft.com/l/chat/19%3Aabc"})).unwrap().id, "19:abc@thread.v2");
        assert!(created_chat(&json!({"id":"../me/events"})).is_err());
        assert_eq!(created_chat(&json!({"id":"19:abc@thread.v2","webUrl":"javascript:alert(1)"})).unwrap().web_url, None);
        assert!(chat_path("19:abc@thread.v2/../../me").is_err());
        assert!(valid_message_id("1616965872395") && !valid_message_id("1/2"));
        let user = json!({"id":"aaaaaaaa-0000-4000-8000-00000000000a","displayName":"Alex <Rivers>"});
        assert_eq!(member("alex@example.com", &user).unwrap().name, "Alex Rivers");
        assert!(member("alex@example.com", &json!({"id":"alex"})).is_none());
        use rmcp::model::CallToolResult;
        assert!(matches!(submission(&CallToolResult::structured(json!({"statusCode":403}))), Err(SubmissionError::Denied)));
        assert!(matches!(submission(&CallToolResult::structured(json!({"statusCode":500}))), Err(SubmissionError::Unknown)));
        assert_eq!(submission(&CallToolResult::structured(json!({"statusCode":201,"data":"{\"id\":\"1\"}"}))).unwrap()["id"], "1");
    }
}
