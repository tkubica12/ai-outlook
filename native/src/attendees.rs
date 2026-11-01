//! Meeting attendees for new events: validated entry, people suggestions from the cached
//! calendar and a bounded, read-only directory search, plus the explicit invitation notice.

use crate::{
    calendar::{Event, person_email},
    i18n::Language,
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Most attendees one new event can invite.
pub const LIMIT: usize = 50;
/// Longest display name kept for an attendee, in UTF-8 bytes.
pub const NAME_LIMIT: usize = 200;
/// Directory search starts only after this many typed characters.
pub const SEARCH_MIN_CHARS: usize = 2;
/// Longest directory search text, in characters.
pub const SEARCH_MAX_CHARS: usize = 100;
/// Directory results requested and kept per search.
pub const SEARCH_RESULTS: usize = 10;
/// Ranked people kept from the cached calendar.
const DIRECTORY_LIMIT: usize = 1000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Required,
    Optional,
}

impl Role {
    /// Graph `attendee.type`.
    pub fn graph(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Required => "Required",
            Self::Optional => "Optional",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Required => Self::Optional,
            Self::Optional => Self::Required,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Attendee {
    /// Lower-case SMTP address; the identity used for de-duplication and read-back.
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default)]
    pub role: Role,
}

impl Attendee {
    pub fn new(email: &str, name: &str, role: Role) -> Result<Self, String> {
        let email = person_email(email.trim().trim_start_matches("mailto:"))
            .filter(|address| !address.contains('<'))
            .ok_or("Enter a valid e-mail address or choose a suggestion")?;
        let attendee = Self {
            name: clean_name(name, &email),
            email,
            role,
        };
        attendee.validate()?;
        Ok(attendee)
    }

    pub fn validate(&self) -> Result<(), String> {
        if person_email(&self.email).as_deref() != Some(self.email.as_str()) {
            return Err("Enter a valid e-mail address or choose a suggestion".into());
        }
        if self.name.len() > NAME_LIMIT
            || self.name.trim() != self.name
            || self
                .name
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | '"'))
        {
            return Err("An attendee name is too long or contains unsupported characters".into());
        }
        Ok(())
    }

    /// The name shown in chips and summaries; the address when no name is known.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.email
        } else {
            &self.name
        }
    }
}

/// A display name without quotes, angle brackets, control characters or a repeated address.
fn clean_name(name: &str, email: &str) -> String {
    let name = name
        .chars()
        .filter(|ch| !ch.is_control() && !matches!(ch, '<' | '>' | '"'))
        .collect::<String>();
    let mut name = name
        .trim()
        .trim_matches('\'')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.eq_ignore_ascii_case(email) {
        return String::new();
    }
    if name.len() > NAME_LIMIT {
        let mut end = NAME_LIMIT;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
        name = name.trim_end().to_owned();
    }
    name
}

/// Parses typed attendees: `alex@example.com`, `Alex Rivers <alex@example.com>` or several of
/// them separated by semicolons, new lines or commas. Outlook's `Last, First <address>` stays one
/// attendee.
pub fn parse_input(text: &str) -> Result<Vec<Attendee>, String> {
    let mut parsed = Vec::new();
    for part in text.split([';', '\n', '\r']) {
        let pieces = if part.matches('@').count() > 1 {
            part.split(',').collect::<Vec<_>>()
        } else {
            vec![part]
        };
        for piece in pieces {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            let (name, email) = match piece.rsplit_once('<') {
                Some((name, rest)) if rest.trim_end().ends_with('>') => {
                    (name, rest.trim_end().trim_end_matches('>'))
                }
                Some(_) => return Err("Enter a valid e-mail address or choose a suggestion".into()),
                None => ("", piece),
            };
            parsed.push(Attendee::new(email, name, Role::Required)?);
            if parsed.len() > LIMIT {
                return Err("A new event can invite at most 50 attendees".into());
            }
        }
    }
    if parsed.is_empty() {
        return Err("Enter a valid e-mail address or choose a suggestion".into());
    }
    Ok(parsed)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AddOutcome {
    pub added: usize,
    pub duplicates: usize,
    pub own: usize,
}

/// Adds attendees once each (case-insensitive), never the user's own mailboxes, all or nothing
/// within [`LIMIT`].
pub fn add(
    list: &mut Vec<Attendee>,
    candidates: Vec<Attendee>,
    own: &[String],
) -> Result<AddOutcome, String> {
    let mut outcome = AddOutcome::default();
    let mut next = list.clone();
    for candidate in candidates {
        candidate.validate()?;
        if own
            .iter()
            .any(|email| email.trim().eq_ignore_ascii_case(&candidate.email))
        {
            outcome.own += 1;
        } else if next.iter().any(|attendee| attendee.email == candidate.email) {
            outcome.duplicates += 1;
        } else {
            next.push(candidate);
            outcome.added += 1;
        }
    }
    if next.len() > LIMIT {
        return Err("A new event can invite at most 50 attendees".into());
    }
    *list = next;
    Ok(outcome)
}

/// The saved attendee list of an approval: valid, unique and bounded.
pub fn validate_list(list: &[Attendee]) -> Result<(), String> {
    if list.len() > LIMIT {
        return Err("A new event can invite at most 50 attendees".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for attendee in list {
        attendee.validate()?;
        if !seen.insert(attendee.email.as_str()) {
            return Err("Each attendee can be invited only once".into());
        }
    }
    Ok(())
}

/// A person who can be suggested as an attendee.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub email: String,
    pub name: String,
    /// Cached meetings shared with this person; zero for directory results.
    pub meetings: u32,
    pub score: f64,
}

impl Person {
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.email
        } else {
            &self.name
        }
    }

    pub fn attendee(&self) -> Result<Attendee, String> {
        Attendee::new(&self.email, &self.name, Role::Required)
    }
}

fn split_person(person: &str) -> Option<(String, String)> {
    let email = person_email(person)?;
    let name = person
        .rsplit_once('<')
        .map_or("", |(name, _)| name);
    Some((clean_name(name, &email), email))
}

/// People from cached meetings (organizers and attendees), ranked by how often and how recently
/// you met them. Your own mailboxes and rooms/resources are excluded.
pub fn from_calendar(events: &[Event], own: &[String], now: DateTime<FixedOffset>) -> Vec<Person> {
    let mut people = BTreeMap::<String, Person>::new();
    for event in events {
        if event.extra.get("isCancelled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let days = (now - event.start).num_days().unsigned_abs() as f64;
        let weight = 1.0 / (1.0 + days / 14.0);
        let resources = event
            .extra
            .get("tomlook_attendee_roles")
            .and_then(Value::as_object);
        let mut seen = std::collections::BTreeSet::new();
        for person in std::iter::once(&event.organizer).chain(event.attendees.iter()) {
            let Some((name, email)) = split_person(person) else {
                continue;
            };
            if own.iter().any(|own| own.trim().eq_ignore_ascii_case(&email))
                || resources
                    .and_then(|roles| roles.get(&email))
                    .and_then(Value::as_str)
                    == Some("resource")
                || !seen.insert(email.clone())
            {
                continue;
            }
            let entry = people.entry(email.clone()).or_insert_with(|| Person {
                email,
                name: String::new(),
                meetings: 0,
                score: 0.0,
            });
            if entry.name.is_empty() {
                entry.name = name;
            }
            entry.meetings = entry.meetings.saturating_add(1);
            entry.score += weight;
        }
    }
    let mut people = people.into_values().collect::<Vec<_>>();
    people.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then(right.meetings.cmp(&left.meetings))
            .then_with(|| left.label().to_lowercase().cmp(&right.label().to_lowercase()))
    });
    people.truncate(DIRECTORY_LIMIT);
    people
}

/// Rank of a match: 0 for a name word or address prefix, 1 for any contained text.
fn match_rank(person: &Person, query: &str) -> Option<u8> {
    let query = crate::search::key(query);
    if query.is_empty() {
        return None;
    }
    let local = person.email.split('@').next().unwrap_or_default();
    let prefix = person
        .name
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '.' | '-'))
        .chain(std::iter::once(local))
        .any(|word| crate::search::key(word).starts_with(&query))
        || crate::search::key(&person.email).starts_with(&query);
    if prefix {
        Some(0)
    } else if crate::search::key(&person.name).contains(&query)
        || crate::search::key(&person.email).contains(&query)
    {
        Some(1)
    } else {
        None
    }
}

/// Suggestions for typed text: prefix matches first, then the given ranking; people already
/// invited are skipped.
pub fn matching<'a>(
    people: &'a [Person],
    query: &str,
    exclude: &[Attendee],
    limit: usize,
) -> Vec<&'a Person> {
    let mut found = people
        .iter()
        .filter(|person| !exclude.iter().any(|attendee| attendee.email == person.email))
        .filter_map(|person| match_rank(person, query).map(|rank| (rank, person)))
        .collect::<Vec<_>>();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().take(limit).map(|(_, person)| person).collect()
}

/// Read-only Graph people search; the typed text is quoted and encoded, results are bounded.
pub fn people_resource(query: &str) -> Result<String, String> {
    let query = query.trim();
    let count = query.chars().count();
    if !(SEARCH_MIN_CHARS..=SEARCH_MAX_CHARS).contains(&count)
        || query.chars().any(|ch| ch.is_control() || ch == '"' || ch == '\\')
    {
        return Err("People search needs 2 to 100 characters".into());
    }
    let mut url = url::Url::parse("https://graph.microsoft.com/me/people")
        .map_err(|_| "Invalid people search path")?;
    url.query_pairs_mut()
        .append_pair("$search", &format!("\"{query}\""))
        .append_pair("$top", &SEARCH_RESULTS.to_string())
        .append_pair("$select", "displayName,scoredEmailAddresses,personType");
    Ok(format!("{}?{}", url.path(), url.query().unwrap_or_default()))
}

/// People search results: persons with a valid address only, never groups or your mailboxes.
pub fn parse_people(value: &Value, own: &[String]) -> Result<Vec<Person>, String> {
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or("People search did not return a list")?;
    let mut people = Vec::new();
    for row in rows.iter().take(25) {
        if row
            .pointer("/personType/class")
            .and_then(Value::as_str)
            .is_some_and(|class| !class.eq_ignore_ascii_case("person"))
        {
            continue;
        }
        let Some(email) = row
            .get("scoredEmailAddresses")
            .and_then(Value::as_array)
            .and_then(|addresses| {
                addresses
                    .iter()
                    .take(8)
                    .find_map(|address| address.get("address").and_then(Value::as_str))
            })
            .and_then(person_email)
        else {
            continue;
        };
        if own.iter().any(|own| own.trim().eq_ignore_ascii_case(&email))
            || people.iter().any(|person: &Person| person.email == email)
        {
            continue;
        }
        let name = row
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|name| name.len() <= 1000)
            .unwrap_or_default();
        people.push(Person {
            name: clean_name(name, &email),
            email,
            meetings: 0,
            score: 0.0,
        });
        if people.len() >= SEARCH_RESULTS {
            break;
        }
    }
    Ok(people)
}

/// The explicit consequence shown before approval: who receives an Outlook invitation.
pub fn invitation_notice(attendees: &[Attendee], language: Language) -> String {
    const LISTED: usize = 10;
    let names = attendees
        .iter()
        .take(LISTED)
        .map(|attendee| match (attendee.role, language) {
            (Role::Required, _) => attendee.label().to_owned(),
            (Role::Optional, Language::En) => format!("{} (optional)", attendee.label()),
            (Role::Optional, Language::Cs) => format!("{} (nepovinně)", attendee.label()),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let more = attendees.len().saturating_sub(LISTED);
    let count = attendees.len();
    match language {
        Language::En => format!(
            "Creating sends an Outlook invitation to {count} {}: {names}{}.",
            if count == 1 { "person" } else { "people" },
            if more > 0 { format!(" and {more} more") } else { String::new() }
        ),
        Language::Cs => format!(
            "Vytvořením odešlete pozvánku z Outlooku {count} {}: {names}{}.",
            if count == 1 { "osobě" } else { "lidem" },
            if more > 0 { format!(" a {more} dalším") } else { String::new() }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn person(text: &str) -> Attendee {
        parse_input(text).unwrap().remove(0)
    }

    #[test]
    fn typed_addresses_are_validated_normalized_and_split_without_breaking_outlook_names() {
        let parsed = parse_input(" Alex Rivers <Alex@Example.com>; jamie@example.com\nRivers, Dana <dana@example.org>").unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], Attendee { email: "alex@example.com".into(), name: "Alex Rivers".into(), role: Role::Required });
        assert_eq!(parsed[1].name, "");
        assert_eq!(parsed[1].label(), "jamie@example.com");
        assert_eq!(parsed[2].name, "Rivers, Dana");
        let parsed = parse_input("a@example.com, b@example.com").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(person("\"Morgan Lee\" <morgan@example.com>").name, "Morgan Lee");
        assert_eq!(person("mailto:kim@example.com").email, "kim@example.com");
        for invalid in ["", "alex", "alex@", "@example.com", "alex@example", "a b@example.com",
            "Alex <alex@example.com", "o'brien@example.com", "x@y@example.com"] {
            assert!(parse_input(invalid).is_err(), "{invalid}");
        }
        let many = (0..=LIMIT).map(|index| format!("p{index}@example.com")).collect::<Vec<_>>().join(";");
        assert!(parse_input(&many).is_err());
        assert!(Attendee::new("a@example.com", &"x".repeat(300), Role::Optional).unwrap().name.len() <= NAME_LIMIT);
        let invalid = Attendee { email: "A@example.com".into(), name: String::new(), role: Role::Required };
        assert!(invalid.validate().is_err());
        let invalid = Attendee { email: "a@example.com".into(), name: "A <b>".into(), role: Role::Required };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn adding_deduplicates_case_insensitively_excludes_own_mailboxes_and_caps_atomically() {
        let own = vec!["Me@Example.com".to_owned(), "alias@example.com".to_owned()];
        let mut list = vec![person("alex@example.com")];
        let outcome = add(&mut list, parse_input("ALEX@example.com; me@example.com; Alias@Example.com; jamie@example.com").unwrap(), &own).unwrap();
        assert_eq!(outcome, AddOutcome { added: 1, duplicates: 1, own: 2 });
        assert_eq!(list.iter().map(|attendee| attendee.email.as_str()).collect::<Vec<_>>(), ["alex@example.com", "jamie@example.com"]);
        let many = (0..LIMIT - 1).map(|index| person(&format!("p{index}@example.com"))).collect::<Vec<_>>();
        assert!(add(&mut list, many, &own).is_err());
        assert_eq!(list.len(), 2, "A rejected batch adds nobody");
        validate_list(&list).unwrap();
        let mut duplicate = list.clone();
        duplicate.push(list[0].clone());
        assert!(validate_list(&duplicate).is_err());
    }

    #[test]
    fn calendar_suggestions_rank_frequent_recent_people_and_skip_self_rooms_and_cancelled() {
        let now = DateTime::parse_from_rfc3339("2026-11-03T12:00:00+01:00").unwrap();
        let meeting = |days: i64, organizer: &str, attendees: &[&str]| {
            let mut event = crate::calendar::fixture(1).remove(0);
            event.start = now + chrono::Duration::days(days);
            event.end = event.start + chrono::Duration::hours(1);
            event.organizer = organizer.into();
            event.attendees = attendees.iter().map(|value| (*value).into()).collect();
            event
        };
        let mut room = meeting(-1, "Me <me@example.com>", &["Room 1 <room1@example.com>", "Alex Rivers <alex@example.com>"]);
        room.extra.insert("tomlook_attendee_roles".into(), json!({"room1@example.com":"resource","alex@example.com":"required"}));
        let mut cancelled = meeting(-1, "Ghost <ghost@example.com>", &[]);
        cancelled.extra.insert("isCancelled".into(), json!(true));
        let events = vec![
            room,
            cancelled,
            meeting(-200, "Old Friend <old@example.com>", &["Alex Rivers <ALEX@example.com>"]),
            meeting(-201, "Old Friend <old@example.com>", &[]),
            meeting(-2, "Jamie Brooks <jamie@example.com>", &["me@example.com"]),
            meeting(3, "Me <me@example.com>", &["Alex Rivers <alex@example.com>", "jamie@example.com", "Alex <alex@example.com>"]),
        ];
        let people = from_calendar(&events, &["ME@example.com".into()], now);
        let emails = people.iter().map(|person| person.email.as_str()).collect::<Vec<_>>();
        assert_eq!(emails, ["alex@example.com", "jamie@example.com", "old@example.com"]);
        assert_eq!(people[0].meetings, 3, "Duplicate mentions in one meeting count once");
        assert_eq!(people[0].name, "Alex Rivers");
        assert_eq!(people[1].name, "Jamie Brooks");
        let invited = vec![person("jamie@example.com")];
        let found = matching(&people, "riv", &invited, 5);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].email, "alex@example.com");
        assert!(matching(&people, "jam", &invited, 5).is_empty(), "Invited people are not suggested again");
        assert_eq!(matching(&people, "o", &[], 5)[0].email, "old@example.com", "Prefix matches come first");
        assert_eq!(matching(&people, "EXAMPLE", &[], 2).len(), 2);
    }

    #[test]
    fn directory_search_is_quoted_bounded_and_parses_people_only() {
        let resource = people_resource("  Novák ").unwrap();
        assert!(resource.starts_with("/me/people?%24search=%22Nov%C3%A1k%22&%24top=10&%24select="), "{resource}");
        for invalid in ["a", "", "x\"y", &"x".repeat(101)] {
            assert!(people_resource(invalid).is_err());
        }
        let value = json!({"value":[
            {"displayName":"Alex Rivers","scoredEmailAddresses":[{"address":"Alex@Example.com"}],"personType":{"class":"Person"}},
            {"displayName":"Team","scoredEmailAddresses":[{"address":"team@example.com"}],"personType":{"class":"Group"}},
            {"displayName":"Me","scoredEmailAddresses":[{"address":"me@example.com"}],"personType":{"class":"Person"}},
            {"displayName":"No address","scoredEmailAddresses":[],"personType":{"class":"Person"}},
            {"displayName":"Alex again","scoredEmailAddresses":[{"address":"alex@example.com"}]}
        ]});
        let people = parse_people(&value, &["me@example.com".into()]).unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].email, "alex@example.com");
        assert_eq!(people[0].name, "Alex Rivers");
        assert!(parse_people(&json!({"error":"x"}), &[]).is_err());
    }

    #[test]
    fn invitation_notice_lists_people_and_roles_in_both_languages() {
        let mut attendees = parse_input("Alex Rivers <alex@example.com>").unwrap();
        assert_eq!(invitation_notice(&attendees, Language::En),
            "Creating sends an Outlook invitation to 1 person: Alex Rivers.");
        assert_eq!(invitation_notice(&attendees, Language::Cs),
            "Vytvořením odešlete pozvánku z Outlooku 1 osobě: Alex Rivers.");
        attendees.push(Attendee { role: Role::Optional, ..person("jamie@example.com") });
        assert_eq!(invitation_notice(&attendees, Language::En),
            "Creating sends an Outlook invitation to 2 people: Alex Rivers, jamie@example.com (optional).");
        assert_eq!(invitation_notice(&attendees, Language::Cs),
            "Vytvořením odešlete pozvánku z Outlooku 2 lidem: Alex Rivers, jamie@example.com (nepovinně).");
        let many = (0..12).map(|index| person(&format!("p{index}@example.com"))).collect::<Vec<_>>();
        assert!(invitation_notice(&many, Language::En).ends_with(", p9@example.com and 2 more."));
        assert!(invitation_notice(&many, Language::Cs).contains("12 lidem"));
        assert!(invitation_notice(&many, Language::Cs).ends_with(" a 2 dalším."));
    }
}
