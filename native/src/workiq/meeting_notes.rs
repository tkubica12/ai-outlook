use super::Connection;
use crate::{calendar::Event, recaps::Materials};
use base64::Engine;
use chrono::{DateTime, Duration, FixedOffset};
use serde_json::{Value, json};

fn path(parts: &[&str]) -> Result<String, String> {
    if parts.iter().any(|part| part.is_empty() || part.len() > 4000
        || matches!(*part, "." | "..") || part.chars().any(|ch| ch.is_control())
        || part.contains(['/', '\\', '?', '#', '%'])) {
        return Err("Invalid meeting artifact identity".into());
    }
    let mut url = url::Url::parse("https://graph.microsoft.com").map_err(|_| "Invalid artifact host")?;
    url.path_segments_mut().map_err(|_| "Invalid artifact path")?.extend(parts);
    Ok(url.path().into())
}

fn page_next(page: &Value, expected: &str) -> Result<Option<String>, String> {
    let Some(next) = page.get("@odata.nextLink") else { return Ok(None) };
    let next = next.as_str().ok_or("Invalid meeting artifact pagination")?;
    let url = url::Url::parse(next).map_err(|_| "Invalid meeting artifact pagination")?;
    let next_path = url.path().strip_prefix("/v1.0").unwrap_or(url.path());
    if url.scheme() != "https" || url.host_str() != Some("graph.microsoft.com")
        || url.port().is_some() || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some() || next_path != expected || next.len() > 8000 {
        return Err("Meeting artifact pagination left its authorized resource".into());
    }
    Ok(Some(format!("{next_path}?{}", url.query().unwrap_or(""))))
}

fn occurrence_transcript(row: &Value, event: &Event) -> Result<bool, String> {
    let date = |key| row.get(key).and_then(Value::as_str)
        .and_then(|date| DateTime::parse_from_rfc3339(date).ok());
    let end = date("endDateTime").ok_or("Transcript has no trustworthy occurrence time; refusing to summarize another occurrence")?;
    let created = match row.get("createdDateTime").filter(|value| !value.is_null()) {
        Some(_) => Some(date("createdDateTime").ok_or("Transcript creation time is invalid")?),
        None => None,
    };
    Ok(end >= event.start && end <= event.end + Duration::minutes(30)
        && created.is_none_or(|created| created >= event.start - Duration::minutes(15) && created <= event.end + Duration::days(7)))
}

fn transcript_text(value: &Value) -> Result<String, String> {
    if value["statusCode"].as_u64() != Some(200) {
        return Err(format!("WorkIQ transcript content returned HTTP {}", value["statusCode"]));
    }
    let encoded = value["base64Content"].as_str().ok_or("Transcript has no content")?;
    if encoded.len() > crate::recaps::MATERIAL_LIMIT * 4 / 3 + 8 {
        return Err("Transcript exceeds the complete-scan safety limit; no partial notes were published".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)
        .map_err(|_| "Transcript encoding is invalid")?;
    if bytes.len() > crate::recaps::MATERIAL_LIMIT
        || value["sizeBytes"].as_u64() != Some(bytes.len() as u64) {
        return Err("Transcript content size is invalid or too large".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Transcript is not valid UTF-8")?;
    if !text.trim_start_matches('\u{feff}').starts_with("WEBVTT") || !text.contains("-->") {
        return Err("The returned artifact is not a nonempty WebVTT transcript".into());
    }
    Ok(text)
}

impl Connection {
    pub async fn recap_materials(&self, event: &Event) -> Result<Option<Materials>, String> {
        let join = event.extra.get("onlineMeeting").and_then(|meeting| meeting["joinUrl"].as_str())
            .filter(|url| crate::calendar::safe_url(url)).ok_or("Meeting has no verified join URL")?;
        let mut lookup = url::Url::parse("https://graph.microsoft.com/me/onlineMeetings")
            .map_err(|_| "Invalid meeting lookup")?;
        lookup.query_pairs_mut().append_pair("$filter", &format!("JoinWebUrl eq '{}'", join.replace('\'', "''")));
        let meetings = self.fetch_resource_in(&format!("{}?{}", lookup.path(), lookup.query().unwrap_or("")),
            &self.admission.research).await
            .map_err(|error| format!("Meeting transcript lookup is unavailable: {error}"))?;
        let rows = meetings["value"].as_array().ok_or("Meeting lookup has an invalid shape")?;
        let matches = rows.iter().filter(|meeting| meeting["joinWebUrl"].as_str() == Some(join)).collect::<Vec<_>>();
        if matches.len() != 1 || meetings.get("@odata.nextLink").is_some() {
            return Err("Meeting artifacts could not be tied to one exact meeting".into());
        }
        let meeting = matches[0];
        let id = meeting["id"].as_str().ok_or("Meeting artifact identity is missing")?;
        let transcripts_path = path(&["me", "onlineMeetings", id, "transcripts"])?;
        let mut resource = Some(format!("{transcripts_path}?$top=50"));
        let mut seen = std::collections::BTreeSet::new();
        let mut transcript_ids = std::collections::BTreeSet::new();
        for _ in 0..8 {
            let Some(current) = resource.take() else { break };
            if !seen.insert(current.clone()) {
                return Err("Transcript pagination repeated a cursor".into());
            }
            let page = self.fetch_resource_in(&current, &self.admission.research).await
                .map_err(|error| format!("Transcript access is unavailable (tenant permissions may be required): {error}"))?;
            for row in page["value"].as_array().ok_or("Transcript list has an invalid shape")? {
                if occurrence_transcript(row, event)? {
                    transcript_ids.insert(row["id"].as_str().ok_or("Transcript identity is missing")?.to_owned());
                }
            }
            resource = page_next(&page, &transcripts_path)?;
        }
        if resource.is_some() {
            return Err("Transcript discovery exceeded its complete pagination limit".into());
        }
        if transcript_ids.is_empty() { return Ok(None); }
        let mut materials = Materials::default();
        for transcript in transcript_ids {
            let content_path = path(&["me", "onlineMeetings", id, "transcripts", &transcript, "content"])?;
            let value = self.read_tool("fetch_blob", json!({"path":format!("{content_path}?$format=text/vtt")})).await?;
            let text = transcript_text(&value)?;
            if materials.transcript.len() + text.len() + 2 > crate::recaps::MATERIAL_LIMIT {
                return Err("Meeting transcripts exceed the complete-scan safety limit".into());
            }
            materials.transcript.push_str(&text);
            materials.transcript.push_str("\n\n");
        }
        if let Some(url) = event.source_url.as_ref().filter(|url| crate::calendar::safe_url(url)) {
            materials.sources.push(json!({"title":"Original meeting and transcript","url":url}));
        }
        if let Some(chat) = meeting["chatInfo"]["threadId"].as_str() {
            match self.recap_chat(chat, event).await {
                Ok((text, gaps)) => { materials.chat = text; materials.gaps.extend(gaps); }
                Err(error) => materials.gaps.push(format!("Meeting chat could not be read: {error}")),
            }
        } else {
            materials.gaps.push("No meeting chat identity was provided".into());
        }
        let links = materials.chat.split_whitespace()
            .filter(|link| link.len() <= 4000 && crate::calendar::safe_url(link))
            .collect::<std::collections::BTreeSet<_>>();
        for link in links {
            materials.sources.push(json!({"title":link,"url":link}));
        }
        if self.identity.display_name.as_ref().is_none_or(|name| name.trim().is_empty()) {
            return Err("The signed-in user's display name is unavailable; reconnect WorkIQ before personal meeting notes".into());
        }
        Ok(Some(materials))
    }

    async fn recap_chat(&self, chat: &str, event: &Event) -> Result<(String, Vec<String>), String> {
        let chat_path = path(&["chats", chat, "messages"])?;
        let mut resource = Some(format!("{chat_path}?$top=50&$orderby=createdDateTime%20desc"));
        let mut text = String::new();
        let mut gaps = Vec::new();
        let mut cursors = std::collections::BTreeSet::new();
        let first = event.start - Duration::minutes(15);
        let last = (event.end + Duration::days(1)).min(chrono::Local::now().fixed_offset());
        let mut reached_start = false;
        for _ in 0..8 {
            let Some(current) = resource.take() else { break };
            if !cursors.insert(current.clone()) { return Err("Meeting chat pagination repeated".into()); }
            let page = self.fetch_resource_in(&current, &self.admission.research).await?;
            for row in page["value"].as_array().ok_or("Meeting chat has an invalid shape")? {
                let created: DateTime<FixedOffset> = row["createdDateTime"].as_str()
                    .and_then(|date| DateTime::parse_from_rfc3339(date).ok())
                    .ok_or("Meeting chat message has no trustworthy timestamp")?;
                if created < first { reached_start = true; continue; }
                if created > last { continue; }
                let body = row["body"]["content"].as_str().unwrap_or("");
                let body = crate::html_text::extract_with_links(body);
                let links = row["attachments"].as_array().map(|items| items.iter()
                    .filter_map(|item| item["contentUrl"].as_str())
                    .filter(|url| crate::calendar::safe_url(url)).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                let message = format!("\nCHAT {} {}:\n{}\n{}\n", created,
                    row["from"]["user"]["displayName"].as_str().unwrap_or("Unknown sender"), body, links);
                if text.len() + message.len() > crate::recaps::MATERIAL_LIMIT {
                    return Err("Meeting chat exceeds its complete-scan safety limit".into());
                }
                text.push_str(&message);
            }
            resource = page_next(&page, &chat_path)?;
            if reached_start { resource = None; break; }
        }
        if resource.is_some() {
            gaps.push("Meeting chat pagination limit reached; some messages may be missing".into());
        }
        if text.is_empty() {
            gaps.push("No chat messages were returned in the meeting's bounded occurrence window".into());
        }
        Ok((text, gaps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_are_bound_to_the_occurrence_not_another_week_of_the_series() {
        let event = crate::calendar::demo(chrono::Local::now(), 1).remove(0);
        let row = json!({"createdDateTime":event.start,"endDateTime":event.end});
        assert!(occurrence_transcript(&row, &event).unwrap());
        let delayed = json!({"createdDateTime":event.end+Duration::days(2),"endDateTime":event.end});
        assert!(occurrence_transcript(&delayed, &event).unwrap());
        assert!(occurrence_transcript(&json!({"createdDateTime":null,"endDateTime":event.end}), &event).unwrap());
        let other = json!({"createdDateTime":event.start-Duration::days(7),"endDateTime":event.end-Duration::days(7)});
        assert!(!occurrence_transcript(&other, &event).unwrap());
        assert!(occurrence_transcript(&json!({"createdDateTime":event.start}), &event).is_err());
    }

    #[test]
    fn transcript_decode_and_pagination_reject_partial_or_untrusted_artifacts() {
        let text = "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nA verified task\n";
        let blob = json!({"statusCode":200,"sizeBytes":text.len(),"base64Content":base64::engine::general_purpose::STANDARD.encode(text)});
        assert_eq!(transcript_text(&blob).unwrap(), text);
        let mut invalid = blob.clone();
        invalid["sizeBytes"] = 0.into();
        assert!(transcript_text(&invalid).is_err());
        assert!(page_next(&json!({"@odata.nextLink":"https://evil.example/me/onlineMeetings/id/transcripts"}), "/me/onlineMeetings/id/transcripts").is_err());
        assert!(page_next(&json!({"@odata.nextLink":"https://graph.microsoft.com/v1.0/me/onlineMeetings/id/transcripts?$skiptoken=next"}), "/me/onlineMeetings/id/transcripts").unwrap().is_some());
        assert!(path(&["me", "onlineMeetings", "../other"]).is_err());
    }
}
