use crate::calendar::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const PROPOSE: &str = "tomlook_propose_meeting_change";
pub const SECTION_NAMES: [&str; 6] = [
    "summary",
    "preparation",
    "talking_points",
    "open_questions",
    "risks",
    "gaps",
];
const OVERRIDES: &str = "tomlook_approved_sections";
const DOCUMENT_LIMIT: usize = 512_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Document {
    Insights,
    Notes,
}

impl Document {
    pub fn label(self) -> &'static str {
        match self {
            Self::Insights => "Edit Insights",
            Self::Notes => "Edit meeting notes",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub name: String,
    pub en: Value,
    pub cs: Value,
}

impl Section {
    pub fn text(&self, language: crate::i18n::Language) -> String {
        let value = match language {
            crate::i18n::Language::En => &self.en,
            crate::i18n::Language::Cs => &self.cs,
        };
        match value {
            Value::String(text) => text.clone(),
            Value::Array(items) => items
                .iter()
                .map(|item| match item {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            other => other.to_string(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !SECTION_NAMES.contains(&self.name.as_str()) {
            return Err(
                "Only narrative sections can be edited; sources and host metadata are protected"
                    .into(),
            );
        }
        for value in [&self.en, &self.cs] {
            let valid = if self.name == "summary" {
                value.as_str().is_some_and(|text| text.len() <= 16_000)
            } else {
                value.as_array().is_some_and(|items| {
                    items.len() <= 64
                        && items
                            .iter()
                            .all(|item| item.as_str().is_some_and(|text| text.len() <= 4000))
                })
            };
            if !valid || value.to_string().len() > 24_000 {
                return Err("Use a bounded bilingual string for summary, or arrays of strings for other sections".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DocumentEdit {
    pub document: Document,
    pub revision: String,
    pub before: Vec<Section>,
    pub after: Vec<Section>,
}

pub fn revision(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "Encode saved document failed")?;
    if bytes.len() > DOCUMENT_LIMIT {
        return Err("Saved document exceeds its 512 kB safety limit".into());
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_sections(sections: &[Section]) -> Result<(), String> {
    let mut names = std::collections::BTreeSet::new();
    if sections.is_empty() || sections.len() > SECTION_NAMES.len() {
        return Err("Choose one to six narrative sections".into());
    }
    for section in sections {
        section.validate()?;
        if !names.insert(&section.name) {
            return Err("A section may only occur once in an edit".into());
        }
    }
    if serde_json::to_vec(sections)
        .map_err(|_| "Encode section edits failed")?
        .len()
        > 64_000
    {
        return Err("Section edits exceed their 64 kB limit; propose fewer sections".into());
    }
    Ok(())
}

fn sections_from(value: &Value, after: &[Section]) -> Result<Vec<Section>, String> {
    after
        .iter()
        .map(|section| {
            let en = value
                .get(&section.name)
                .cloned()
                .ok_or("Saved narrative section is missing")?;
            let cs = value
                .get("cs")
                .and_then(|cs| cs.get(&section.name))
                .cloned()
                .ok_or(
                    "Saved Czech narrative section is missing; finish bilingual preparation first",
                )?;
            Ok(Section {
                name: section.name.clone(),
                en,
                cs,
            })
        })
        .collect()
}

impl DocumentEdit {
    pub fn new(
        document: Document,
        expected: String,
        after: Vec<Section>,
        value: &Value,
    ) -> Result<Self, String> {
        validate_sections(&after)?;
        if revision(value)? != expected {
            return Err(
                "Saved document changed; read its current revision before proposing an edit".into(),
            );
        }
        let before = sections_from(value, &after)?;
        let edit = Self {
            document,
            revision: expected,
            before,
            after,
        };
        edit.validate()?;
        if edit.before == edit.after {
            return Err("The proposed sections are already saved".into());
        }
        Ok(edit)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_sections(&self.before)?;
        validate_sections(&self.after)?;
        if self.revision.len() != 64
            || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self
                .before
                .iter()
                .map(|section| &section.name)
                .ne(self.after.iter().map(|section| &section.name))
        {
            return Err("Document approval has an invalid revision or mismatched sections".into());
        }
        Ok(())
    }

    pub fn apply(&self, value: &Value, approval: &str) -> Result<Value, String> {
        self.validate()?;
        if revision(value)? != self.revision || sections_from(value, &self.after)? != self.before {
            return Err("Saved document changed after this proposal; nothing was saved. Ask for a new proposal.".into());
        }
        let mut next = value.clone();
        if !next.is_object() || !next["cs"].is_object() {
            return Err("Saved bilingual document is invalid".into());
        }
        for section in &self.after {
            next[&section.name] = section.en.clone();
            next["cs"][&section.name] = section.cs.clone();
            if next.get(OVERRIDES).is_none() {
                next[OVERRIDES] = json!({});
            }
            if !next[OVERRIDES].is_object() {
                return Err("Saved approved sections are invalid".into());
            }
            next[OVERRIDES][&section.name] = json!({
                "en":section.en, "cs":section.cs, "approval":approval,
                "approved_at":chrono::Utc::now().to_rfc3339()
            });
        }
        revision(&next)?;
        Ok(next)
    }
}

pub fn approved_context(value: &Value) -> Option<&Value> {
    value.get(OVERRIDES)
}

pub fn preparation_context(value: &Value) -> Result<Option<Value>, String> {
    let Some(approved) = approved_context(value) else {
        return Ok(None);
    };
    let approved = approved
        .as_object()
        .ok_or("Saved approved sections are invalid")?;
    let mut sections = serde_json::Map::new();
    let mut complete = true;
    for (name, value) in approved {
        let section = Section {
            name: name.clone(),
            en: value["en"].clone(),
            cs: value["cs"].clone(),
        };
        section.validate()?;
        let text = section.text(crate::i18n::Language::En);
        let end = text.floor_char_boundary(text.len().min(1000));
        complete &= end == text.len();
        sections.insert(
            name.clone(),
            json!({"excerpt":&text[..end],"complete":end == text.len()}),
        );
    }
    Ok(Some(json!({"sections":sections,"complete":complete,
        "note":"User-approved local narrative, not primary source verification. The host retains these sections while other sections refresh. Some context may be explicitly excerpted: missing text is not missing preparation. Do not recommend creating already existing materials; surface source-backed contradictions as gaps for user review instead of overwriting approved sections."})))
}

pub fn retain_approved(previous: &Value, next: &mut Value) -> Result<(), String> {
    let Some(approved) = previous.get(OVERRIDES) else {
        return Ok(());
    };
    let sections = approved
        .as_object()
        .ok_or("Saved approved sections are invalid")?;
    if sections.len() > SECTION_NAMES.len() || !next.is_object() || !next["cs"].is_object() {
        return Err("Approved sections cannot be merged into this bilingual document".into());
    }
    for (name, value) in sections {
        let section = Section {
            name: name.clone(),
            en: value["en"].clone(),
            cs: value["cs"].clone(),
        };
        section.validate()?;
        next[name] = section.en;
        next["cs"][name] = section.cs;
    }
    next[OVERRIDES] = approved.clone();
    revision(next)?;
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BodyEdit {
    pub content_type: String,
    pub before: String,
    pub replacement: Option<String>,
    pub append: Option<String>,
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('\n', "<br>")
}

impl BodyEdit {
    pub fn before_text(&self) -> String {
        if self.content_type == "html" {
            crate::html_text::extract_with_links(&self.before)
        } else {
            self.before.clone()
        }
    }
    pub fn validate(&self, event: &Event) -> Result<(), String> {
        if !matches!(self.content_type.as_str(), "html" | "text")
            || self.before.len() > 64_000
            || self.replacement.is_some() == self.append.is_some()
        {
            return Err("Read the original description; choose either replacement or append, with its original html/text format".into());
        }
        let text = self
            .replacement
            .as_ref()
            .or(self.append.as_ref())
            .ok_or("Description edit is empty")?;
        if text.len() > 16_000
            || text
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
            || (self.append.is_some() && text.trim().is_empty())
        {
            return Err(
                "Description edit exceeds 16 kB or contains unsupported control characters".into(),
            );
        }
        if self.replacement.is_some()
            && (event.extra.get("isOnlineMeeting") == Some(&Value::Bool(true))
                || event
                    .extra
                    .get("onlineMeeting")
                    .is_some_and(|meeting| !meeting.is_null()))
        {
            return Err("Online meeting descriptions support append only, preserving the complete original meeting blob".into());
        }
        Ok(())
    }

    pub fn after(&self) -> String {
        let text = self
            .replacement
            .as_deref()
            .or(self.append.as_deref())
            .unwrap_or_default();
        if self.content_type == "text" {
            return if self.replacement.is_some() {
                text.into()
            } else {
                format!("{}\n\n{text}", self.before)
            };
        }
        let addition = format!("<div>{}</div>", html_escape(text));
        if self.replacement.is_some() {
            return addition;
        }
        let lower = self.before.to_ascii_lowercase();
        let offset = lower
            .rfind("</body>")
            .or_else(|| lower.rfind("</html>"))
            .unwrap_or(self.before.len());
        format!(
            "{}{}{}",
            &self.before[..offset],
            addition,
            &self.before[offset..]
        )
    }

    pub fn matches_before(&self, body: &Value) -> bool {
        body["contentType"]
            .as_str()
            .is_some_and(|kind| kind.eq_ignore_ascii_case(&self.content_type))
            && body["content"].as_str() == Some(self.before.as_str())
    }

    pub fn matches_after(&self, body: &Value) -> bool {
        let Some(content) = body["content"].as_str() else {
            return false;
        };
        let kind = body["contentType"].as_str().unwrap_or_default();
        let actual = if kind.eq_ignore_ascii_case("html") {
            crate::html_text::extract_with_links(content)
        } else {
            content.into()
        };
        let after = self.after();
        let expected = if self.content_type == "html" {
            crate::html_text::extract_with_links(&after)
        } else {
            after
        };
        actual.split_whitespace().eq(expected.split_whitespace())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextEdit {
    pub title: Option<String>,
    pub body: Option<BodyEdit>,
}

impl TextEdit {
    pub fn validate(&self, event: &Event) -> Result<(), String> {
        if event.extra.get("isOrganizer") != Some(&Value::Bool(true)) {
            return Err(
                "Title and description edits require a meeting or appointment you organize".into(),
            );
        }
        if self.title.is_none() && self.body.is_none()
            || self.title.as_ref().is_some_and(|title| {
                title.trim().is_empty() || title.len() > 1000 || title.chars().any(char::is_control)
            })
        {
            return Err("Choose a title or description edit; the title must be nonempty and at most 1000 bytes".into());
        }
        if let Some(body) = &self.body {
            body.validate(event)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub event_revision: String,
    pub reason: String,
    pub reason_cs: String,
    pub action: Action,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Create {
        event: NewEvent,
    },
    Delete {},
    Document {
        document: Document,
        revision: String,
        sections: Vec<Section>,
    },
    Category {
        category: Option<String>,
    },
    Travel {
        minutes: u16,
        after: bool,
    },
    Text {
        title: Option<String>,
        body: Option<BodyEdit>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewEvent {
    pub title: String,
    pub start: chrono::DateTime<chrono::FixedOffset>,
    pub end: chrono::DateTime<chrono::FixedOffset>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub teams: bool,
    #[serde(default)]
    pub attendees: Vec<crate::attendees::Attendee>,
}

impl NewEvent {
    pub fn draft(&self, language: crate::i18n::Language) -> crate::event_creation::Draft {
        crate::event_creation::Draft {
            title: self.title.clone(),
            description: self.description.clone(),
            start: self.start,
            end: self.end,
            category: self.category.clone(),
            private: self.private,
            teams: self.teams,
            travel_minutes: None,
            language,
            attendees: self.attendees.clone(),
        }
    }
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.reason, &self.reason_cs]
            .iter()
            .any(|text| text.trim().is_empty() || text.len() > 1600)
        {
            return Err("A proposal needs bounded English/Czech reasons".into());
        }
        if let Action::Create { event } = &self.action {
            if !self.id.is_empty() || !self.event_revision.is_empty() {
                return Err("New events must not carry an existing meeting identity or revision".into());
            }
            return event.draft(crate::i18n::Language::En).validate();
        }
        if self.id.is_empty() || self.id.len() > 2000 || self.event_revision.len() != 64
            || !self.event_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("A proposal needs a saved meeting id and current event revision".into());
        }
        match &self.action {
            Action::Document {
                revision, sections, ..
            } => {
                if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err("Read the saved document revision before proposing changes".into());
                }
                validate_sections(sections)
            }
            Action::Category {
                category: Some(category),
            } if category.trim().is_empty()
                || category.len() > 256
                || category.chars().any(char::is_control) =>
            {
                Err("Invalid proposed category".into())
            }
            Action::Travel { minutes, .. }
                if !(15..=180).contains(minutes) || minutes % 15 != 0 =>
            {
                Err("Travel must be 15-180 minutes in 15-minute steps".into())
            }
            Action::Text { title, body }
                if title.as_ref().is_some_and(|title| title.len() > 1000)
                    || body.as_ref().is_some_and(|body| {
                        body.before.len() > 64_000
                            || body
                                .replacement
                                .as_ref()
                                .is_some_and(|text| text.len() > 16_000)
                            || body.append.as_ref().is_some_and(|text| text.len() > 16_000)
                    }) =>
            {
                Err("Proposed meeting text exceeds its safety limit".into())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
pub(crate) fn fixture_document() -> Value {
    let narrative = json!({"summary":"A draft already exists.",
            "preparation":["Review the existing draft."],"talking_points":["Technical details"],
            "open_questions":[],"risks":[],"gaps":[]});
    let mut value = narrative.clone();
    value["cs"] = narrative;
    value["sources"] = json!([{"id":"R1","url":"https://example.invalid/draft",
            "observed_at":"2026-10-28T10:00:00Z","excerpt":"Original captured evidence"}]);
    value["recommendations"] = json!([]);
    value["tomlook_fingerprint"] = json!("original-analysis");
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> Section {
        Section {
            name: "summary".into(),
            en: json!("Existing [draft](https://example.invalid/draft); review it."),
            cs: json!("Existujici [koncept](https://example.invalid/draft); zkontrolovat."),
        }
    }

    #[test]
    fn document_edits_bind_exact_before_and_after_without_rewriting_original_evidence() {
        let value = fixture_document();
        let edit = DocumentEdit::new(
            Document::Insights,
            revision(&value).unwrap(),
            vec![summary()],
            &value,
        )
        .unwrap();
        let next = edit.apply(&value, "approval-id").unwrap();
        assert_eq!(next["summary"], summary().en);
        assert_eq!(next["cs"]["summary"], summary().cs);
        for field in [
            "sources",
            "recommendations",
            "tomlook_fingerprint",
            "preparation",
            "talking_points",
        ] {
            assert_eq!(next[field], value[field], "{field}");
        }
        assert_eq!(next[OVERRIDES]["summary"]["approval"], "approval-id");
        let mut changed_source = value.clone();
        changed_source["sources"][0]["excerpt"] = json!("A newer source");
        assert!(edit.apply(&changed_source, "approval-id").is_err());
        let mut forged_before = edit.clone();
        forged_before.before[0].en = json!("A different before preview");
        assert!(forged_before.apply(&value, "approval-id").is_err());
    }

    #[test]
    fn approved_sections_survive_refresh_but_other_sections_and_sources_keep_updating() {
        let value = fixture_document();
        let edit = DocumentEdit::new(
            Document::Insights,
            revision(&value).unwrap(),
            vec![summary()],
            &value,
        )
        .unwrap();
        let approved = edit.apply(&value, "approved").unwrap();
        let mut fresh = fixture_document();
        fresh["summary"] = json!("Generated replacement that must not overwrite approved text");
        fresh["preparation"] = json!(["New remaining work"]);
        fresh["cs"]["preparation"] = json!(["Nova zbyvajici prace"]);
        fresh["sources"][0]["observed_at"] = json!("2026-10-29T10:00:00Z");
        retain_approved(&approved, &mut fresh).unwrap();
        assert_eq!(fresh["summary"], summary().en);
        assert_eq!(fresh["cs"]["summary"], summary().cs);
        assert_eq!(fresh["preparation"], json!(["New remaining work"]));
        assert_eq!(fresh["cs"]["preparation"], json!(["Nova zbyvajici prace"]));
        assert_eq!(fresh["sources"][0]["observed_at"], "2026-10-29T10:00:00Z");
        assert!(
            preparation_context(&fresh).unwrap().unwrap()["complete"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn document_edits_reject_protected_keys_duplicate_sections_wrong_shapes_and_noops() {
        let value = fixture_document();
        for name in ["sources", "recommendations", "tomlook_account", "cs"] {
            let mut section = summary();
            section.name = name.into();
            assert!(
                DocumentEdit::new(
                    Document::Insights,
                    revision(&value).unwrap(),
                    vec![section],
                    &value
                )
                .is_err()
            );
        }
        assert!(
            DocumentEdit::new(
                Document::Insights,
                revision(&value).unwrap(),
                vec![summary(), summary()],
                &value
            )
            .is_err()
        );
        let mut section = summary();
        section.en = json!(["Not a string"]);
        assert!(
            DocumentEdit::new(
                Document::Insights,
                revision(&value).unwrap(),
                vec![section],
                &value
            )
            .is_err()
        );
        let unchanged = Section {
            name: "summary".into(),
            en: value["summary"].clone(),
            cs: value["cs"]["summary"].clone(),
        };
        assert!(
            DocumentEdit::new(
                Document::Notes,
                revision(&value).unwrap(),
                vec![unchanged],
                &value
            )
            .is_err()
        );
        assert!(
            DocumentEdit::new(Document::Insights, "0".repeat(64), vec![summary()], &value).is_err()
        );
    }

    #[test]
    fn approved_preparation_context_is_bounded_and_never_silently_truncated() {
        let value = fixture_document();
        let section = Section {
            name: "summary".into(),
            en: json!("\u{1f4dd}".repeat(3000)),
            cs: json!("Bounded Czech"),
        };
        let edit = DocumentEdit::new(
            Document::Insights,
            revision(&value).unwrap(),
            vec![section],
            &value,
        )
        .unwrap();
        let next = edit.apply(&value, "approved").unwrap();
        let context = preparation_context(&next).unwrap().unwrap();
        assert_eq!(context["complete"], false);
        assert_eq!(context["sections"]["summary"]["complete"], false);
        assert!(context.to_string().len() < 8000);
        assert_eq!(next["summary"].as_str().unwrap().len(), 12000);
    }

    #[test]
    fn online_description_append_preserves_every_original_byte_and_escapes_inserted_html() {
        let mut event = crate::calendar::fixture(1).remove(0);
        event.extra.insert("isOnlineMeeting".into(), json!(true));
        let before = "<html><body>Agenda<div id=\"teams\"><a href=\"https://example.invalid/join\">Join</a><p>Meeting ID: 123</p></div></body></html>";
        let body = BodyEdit {
            content_type: "html".into(),
            before: before.into(),
            replacement: None,
            append: Some("New reference <script>not executable</script>\nDetails".into()),
        };
        body.validate(&event).unwrap();
        let addition =
            "<div>New reference &lt;script&gt;not executable&lt;/script&gt;<br>Details</div>";
        let after = body.after();
        assert_eq!(after.replacen(addition, "", 1), before);
        assert!(!after.contains("<script>"));
        assert!(body.matches_before(&json!({"contentType":"HTML","content":before})));
        assert!(body.matches_after(&json!({"contentType":"html","content":after})));
        assert!(!body.matches_after(
            &json!({"contentType":"html","content":after.replace("/join", "/different")})
        ));
        let replacement = BodyEdit {
            replacement: Some("A whole new body".into()),
            append: None,
            ..body
        };
        assert!(replacement.validate(&event).is_err());
    }
}
