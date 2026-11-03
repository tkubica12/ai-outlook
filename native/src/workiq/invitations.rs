use super::{Connection, Range, READ_TOO_LARGE, calendar_with_filter};
use crate::{calendar::person_email, invitations::{self, Group, ReadKind, ReadResult}};
use chrono::{Duration, NaiveDate};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn resource(path: &str, pairs: &[(&str, String)]) -> Result<String, String> {
    let mut url = url::Url::parse("https://graph.microsoft.com")
        .map_err(|_| "Invalid fixed invitation directory URL")?;
    url.set_path(path);
    for (name, value) in pairs { url.query_pairs_mut().append_pair(name, value); }
    Ok(format!("{}?{}", url.path(), url.query().unwrap_or("")))
}

fn rows(data: &Value, limit: usize, complete: bool) -> Result<&Vec<Value>, String> {
    let rows = data.get("value").and_then(Value::as_array)
        .ok_or("Invitation read did not return a collection")?;
    if rows.len() > limit { return Err("Invitation read exceeded its record limit".into()); }
    match data.get("@odata.nextLink") {
        None | Some(Value::Null) => {}
        Some(Value::String(link)) if !link.is_empty() && !complete => {}
        Some(Value::String(link)) if !link.is_empty() =>
            return Err("Team membership exceeds one complete supported page; cached membership was preserved".into()),
        _ => return Err("Invitation read returned invalid pagination metadata".into()),
    }
    Ok(rows)
}

impl Connection {
    pub async fn read_invitations(&self, request: ReadKind) -> Result<ReadResult, String> {
        tokio::time::timeout(std::time::Duration::from_secs(180), async {
            if !self.is_active() { return Err("Connect WorkIQ before reading invitations".into()); }
            let result = match request {
                ReadKind::Pending { first, last } =>
                    ReadResult::Pending(self.pending_invitations(first, last).await?),
                ReadKind::Teams(groups) => {
                    if groups.len() > invitations::GROUP_LIMIT {
                        return Err("Too many invitation team groups".into());
                    }
                    let mut refreshed = Vec::new();
                    for mut group in groups {
                        group.validate()?;
                        match self.invitation_group_members(&group.id).await {
                            Ok(members) => {
                                group.members = members;
                                group.refreshed_at = Some(chrono::Utc::now());
                                group.error = None;
                            }
                            Err(error) => group.error = Some(error),
                        }
                        refreshed.push(group);
                    }
                    ReadResult::Teams(refreshed)
                }
                ReadKind::Discover(queries) =>
                    ReadResult::Discovered(self.discover_invitation_groups(&queries).await?),
            };
            if !self.is_active() { return Err("WorkIQ connection changed during invitation read".into()); }
            Ok(result)
        }).await.map_err(|_| "Invitation refresh exceeded its three-minute deadline; cache preserved".to_owned())?
    }

    async fn pending_invitations(&self, first: NaiveDate, last: NaiveDate) -> Result<Vec<crate::calendar::Event>, String> {
        if last < first || (last - first).num_days() > 186 {
            return Err("Invitation refresh requires up to six months of dates".into());
        }
        let mut events = BTreeMap::new();
        let mut day = first;
        let mut bytes = 0usize;
        while day <= last {
            let end = (day + Duration::days(41)).min(last);
            let range = Range::new(day, end)?;
            let batch = calendar_with_filter(range, |resource| async move {
                self.fetch_resource_in(&resource, &self.admission.invitations).await
            }, true).await?;
            if !batch.coverage.is_empty() { return Err("Invitation reads cannot mark days synchronized".into()); }
            for event in batch.events {
                if let Some(previous) = events.get(&event.id) {
                    if serde_json::to_value(previous).map_err(|_| "Encode invitation failed")?
                        != serde_json::to_value(&event).map_err(|_| "Encode invitation failed")?
                    {
                        return Err("Invitations changed during refresh; previous queue preserved".into());
                    }
                    continue;
                }
                bytes += serde_json::to_vec(&event).map_err(|_| "Encode invitation failed")?.len();
                if bytes > invitations::PAYLOAD_LIMIT || events.len() >= 5_000 {
                    return Err("Invitation snapshot exceeds its safety limit; previous queue preserved".into());
                }
                events.insert(event.id.clone(), event);
            }
            day = end.succ_opt().ok_or("Invitation date range exceeds supported dates")?;
        }
        Ok(events.into_values().collect())
    }

    async fn invitation_group_members(&self, id: &str) -> Result<BTreeSet<String>, String> {
        if !crate::msx::auth::uuid(id) { return Err("Invitation team group ID is invalid".into()); }
        let query = resource(&format!("/groups/{id}/transitiveMembers/graph.user"), &[
            ("$select", "id,mail,userPrincipalName".into()), ("$top", "999".into()),
        ])?;
        let data = self.fetch_resource_in(&query, &self.admission.invitations).await
            .map_err(|error| if error == READ_TOO_LARGE {
                "Team membership exceeds the supported response size; cached membership was preserved".into()
            } else { error })?;
        let mut members = BTreeSet::new();
        for row in rows(&data, 999, true)? {
            let mut found = false;
            for field in ["mail", "userPrincipalName"] {
                if let Some(value) = row.get(field).and_then(Value::as_str) {
                    members.insert(person_email(value).ok_or("Team member email address is invalid")?);
                    found = true;
                }
            }
            if !found { return Err("Team membership contains an unreadable user; cached membership was preserved".into()); }
        }
        Ok(members)
    }

    async fn discover_invitation_groups(&self, queries: &[String]) -> Result<Vec<Group>, String> {
        invitations::validate_queries(queries)?;
        if queries.is_empty() { return Err("Enter a team search before looking in historical mail".into()); }
        let mut candidates = BTreeMap::new();
        for query in queries {
            let query = query.trim();
            let path = resource("/me/messages", &[
                ("$search", format!("\"{query}\"")),
                ("$select", "toRecipients,ccRecipients".into()), ("$top", "25".into()),
            ])?;
            let data = self.fetch_resource_in(&path, &self.admission.invitations).await?;
            for message in rows(&data, 25, false)? {
                for field in ["toRecipients", "ccRecipients"] {
                    let recipients = message.get(field).and_then(Value::as_array)
                        .ok_or("Team discovery mail has an invalid recipient list")?;
                    if recipients.len() > 1_000 { return Err("Team discovery recipient list exceeds its safety limit".into()); }
                    for recipient in recipients {
                        let address = recipient.get("emailAddress").ok_or("Team discovery recipient is invalid")?;
                        let name = address.get("name").and_then(Value::as_str).unwrap_or("");
                        let mail = address.get("address").and_then(Value::as_str).and_then(person_email);
                        let Some(mail) = mail else { continue; };
                        if team_search_matches(query, name, &mail) { candidates.insert(mail, name.to_owned()); }
                    }
                }
            }
        }
        if candidates.len() > invitations::GROUP_LIMIT {
            return Err("Too many team group candidates; narrow the historical mail searches".into());
        }
        let mut groups = BTreeMap::new();
        for (mail, _) in candidates {
            let path = resource("/groups", &[
                ("$filter", format!("mail eq '{mail}'")),
                ("$select", "id,displayName,mail".into()), ("$top", "2".into()),
            ])?;
            let data = self.fetch_resource_in(&path, &self.admission.invitations).await?;
            let found = rows(&data, 2, true)?;
            if found.len() > 1 { return Err("A team mail address resolves to multiple directory groups".into()); }
            let Some(found) = found.first() else { continue; };
            let group = Group {
                id: found.get("id").and_then(Value::as_str).ok_or("Team group has no ID")?.into(),
                name: found.get("displayName").and_then(Value::as_str).ok_or("Team group has no display name")?.into(),
                mail: found.get("mail").and_then(Value::as_str).and_then(person_email)
                    .ok_or("Team group has no readable mail address")?,
                enabled: false, members: BTreeSet::new(), refreshed_at: None, error: None,
            };
            group.validate()?;
            groups.insert(group.id.clone(), group);
        }
        if groups.is_empty() {
            return Err("No readable directory groups matched the selected historical mail searches".into());
        }
        Ok(groups.into_values().collect())
    }
}

fn team_search_matches(query: &str, name: &str, mail: &str) -> bool {
    let words = crate::search::words(&format!("{name} {mail}")).into_iter().collect::<BTreeSet<_>>();
    crate::search::words(query).iter().all(|word| words.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_matches_team_words_without_inferring_people_or_hardcoding_an_organization() {
        assert!(team_search_matches("Region STU", "STU Region", "sales-tech@org.example"));
        assert!(team_search_matches("region ATU", "Region EC ATU", "accounts@org.example"));
        assert!(!team_search_matches("Region CSU", "Regional Customer", "csuite@org.example"));
        assert!(!team_search_matches("Region STU", "Worldwide STU", "tech@org.example"));
    }

    #[test]
    fn partial_membership_is_never_complete_and_query_values_cannot_inject_paths() {
        assert!(rows(&serde_json::json!({"value":[], "@odata.nextLink":"/groups/x?$skip=999"}), 999, true).is_err());
        assert!(rows(&serde_json::json!({"value":[], "@odata.nextLink":"/me/messages?$skip=25"}), 25, false).is_ok());
        assert!(invitations::validate_queries(&["Region STU\" OR anything".into()]).is_err());
        let path = resource("/groups", &[("$filter", "mail eq 'group@org.example'".into())]).unwrap();
        assert!(path.starts_with("/groups?"));
        assert!(!path.contains("https://"));
    }

    #[tokio::test]
    async fn filtered_invitation_reads_partition_completely_but_never_publish_calendar_coverage() {
        use chrono::TimeZone;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let start = chrono::Utc.with_ymd_and_hms(2026, 11, 3, 10, 0, 0).unwrap();
        let row = serde_json::json!({"id":"pending","subject":"Synthetic invitation",
            "start":{"dateTime":start.to_rfc3339(),"timeZone":"UTC"},
            "end":{"dateTime":(start + Duration::hours(1)).to_rfc3339(),"timeZone":"UTC"},
            "isOrganizer":false,"responseStatus":{"response":"notResponded"},"type":"singleInstance"});
        let reads = AtomicUsize::new(0);
        let calendar = calendar_with_filter(Range::new(day, day).unwrap(), |path| {
            let url = url::Url::parse(&format!("https://graph.microsoft.com{path}")).unwrap();
            let filter = url.query_pairs().find(|(name, _)| name == "$filter").unwrap().1;
            assert_eq!(filter, "(responseStatus/response eq 'notResponded' or responseStatus/response eq 'none')");
            let data = if reads.fetch_add(1, Ordering::AcqRel) == 0 {
                serde_json::json!({"value":[], "@odata.nextLink":"/me/calendarView?$skiptoken=blocked"})
            } else { serde_json::json!({"value":[row.clone()]}) };
            async move { Ok(data) }
        }, true).await.unwrap();
        assert_eq!(reads.load(Ordering::Acquire), 3);
        assert_eq!(calendar.events.len(), 1);
        assert!(calendar.coverage.is_empty());
        assert!(invitations::pending(&calendar.events[0], start - Duration::days(1)));
        let mut null = row.clone();
        null["seriesMasterId"] = Value::Null;
        assert_eq!(super::super::parse_event(row.clone()).unwrap().fingerprint(),
            super::super::parse_event(null).unwrap().fingerprint(),
            "Adding an absent series ID must not invalidate unrelated single-meeting research or approvals");
        let mut recurring = row;
        recurring["seriesMasterId"] = serde_json::json!("master");
        assert_eq!(super::super::parse_event(recurring).unwrap().extra["seriesMasterId"], "master");
    }
}
