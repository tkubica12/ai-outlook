use crate::{
    calendar::{Calendar, Event},
    evidence::Recorder,
    storage::Store,
};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, TimeZone};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub const SEARCH: &str = "tomlook_calendar_search";
pub const DETAIL: &str = "tomlook_meeting_read";
pub const INSIGHTS: &str = "tomlook_insights_read";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalQuery {
    Today,
    Week,
    Search(String),
    Insights,
}

pub struct LocalAnswer {
    pub answer: crate::ai::Answer,
    pub meetings: Vec<(String, String)>,
}

pub fn local_answer(
    store: &Store,
    calendar: &Calendar,
    query: LocalQuery,
    meeting: Option<Event>,
    language: crate::i18n::Language,
    demo: bool,
) -> Result<LocalAnswer, String> {
    if query == LocalQuery::Insights {
        let event = meeting.ok_or("Choose a meeting context before reading saved Insights")?;
        let current = calendar
            .events
            .iter()
            .find(|candidate| {
                candidate.id == event.id && candidate.fingerprint() == event.fingerprint()
            })
            .ok_or("The meeting changed; choose its current version")?;
        let payload = if demo {
            Some(crate::showcase::insights(current)?)
        } else {
            store
                .briefing(&event.id)?
                .map(|json| {
                    serde_json::from_str::<Value>(&json).map_err(|_| "Saved Insights are invalid")
                })
                .transpose()?
        };
        let mut text = format!("## {}\n\n", event.title);
        if let Some(payload) = payload {
            if !demo
                && payload["tomlook_event_fingerprint"].as_str()
                    != Some(event.fingerprint().as_str())
            {
                text.push_str(&format!(
                    "**{}**\n\n",
                    language.text("Saved Insights may be stale for this meeting.")
                ));
            }
            let narrative = language.insight(&payload).unwrap_or(&payload);
            if let Some(summary) = narrative["summary"].as_str() {
                text.push_str(summary);
                text.push_str("\n\n");
            }
            for (field, label) in [
                ("preparation", "Preparation"),
                ("talking_points", "Talking points"),
                ("open_questions", "Open questions"),
            ] {
                if let Some(items) = narrative[field]
                    .as_array()
                    .filter(|items| !items.is_empty())
                {
                    text.push_str(&format!("### {}\n\n", language.text(label)));
                    for item in items.iter().filter_map(Value::as_str).take(20) {
                        text.push_str(&format!("- {item}\n"));
                    }
                    text.push('\n');
                }
            }
            if text.len() > 64_000 {
                return Err(
                    "Saved Insights exceed the display limit; open the meeting Insights instead"
                        .into(),
                );
            }
        } else {
            text.push_str(
                language
                    .text("No saved Insights. Open the meeting to prepare them.")
                    .as_ref(),
            );
        }
        return Ok(LocalAnswer {
            answer: crate::ai::Answer::new(
                text,
                Arc::new(Default::default()),
                Arc::new(Vec::new()),
            ),
            meetings: vec![(event.id, event.title)],
        });
    }
    let today = Local::now().date_naive();
    let first = if query == LocalQuery::Week {
        today - Duration::days(i64::from(today.weekday().num_days_from_monday()))
    } else {
        today
    };
    let start = Local
        .from_local_datetime(&first.and_hms_opt(0, 0, 0).ok_or("Invalid local date")?)
        .earliest()
        .ok_or("This local date has no midnight")?
        .fixed_offset();
    let until_date = first
        + Duration::days(if query == LocalQuery::Week {
            7
        } else if matches!(query, LocalQuery::Search(_)) {
            180
        } else {
            1
        });
    let until = Local
        .from_local_datetime(
            &until_date
                .and_hms_opt(0, 0, 0)
                .ok_or("Invalid local date")?,
        )
        .earliest()
        .ok_or("This local date has no midnight")?
        .fixed_offset();
    let search = Search {
        query: if let LocalQuery::Search(query) = query {
            query
        } else {
            String::new()
        },
        from: Some(start),
        until: Some(until),
        offset: 0,
        order: Order::default(),
    };
    let found = ranked_matches(calendar, &search)?;
    let mut text = String::new();
    for (event, matched) in found.iter().take(25) {
        let start = event.start.with_timezone(&Local);
        let end = event.end.with_timezone(&Local);
        let time = if event.is_all_day {
            let (first, last) = event.calendar_dates();
            format!(
                "{} / {}",
                language.range(first, last),
                language.text("All day")
            )
        } else if start.date_naive() == end.date_naive() {
            format!(
                "{} / {} - {}",
                language.date(start.date_naive(), true),
                start.format("%H:%M"),
                end.format("%H:%M")
            )
        } else {
            format!(
                "{} {} - {} {}",
                language.date(start.date_naive(), true),
                start.format("%H:%M"),
                language.date(end.date_naive(), true),
                end.format("%H:%M")
            )
        };
        text.push_str(&format!(
            "**{}**  \n{} / {}\n\n",
            event.title,
            time,
            language.text(event.response_label())
        ));
        if matched.approximate {
            text.push_str(&format!(
                "*{}*\n\n",
                language.text("Approximate keyword match; verify the meeting.")
            ));
        }
    }
    if found.is_empty() {
        text.push_str(&format!(
            "{}\n\n",
            language.text("No matches in the saved calendar.")
        ));
    }
    if found.len() > 25 {
        text.push_str(&format!(
            "{}\n\n",
            language.text("Showing the 25 best matches. Narrow /search to see more.")
        ));
    }
    let days = (until_date - first).num_days();
    if (0..days).any(|day| !calendar.coverage.contains(&(first + Duration::days(day)))) {
        text.push_str(
            language
                .text(
                    "Cache coverage is incomplete; no matches do not prove there are no meetings.",
                )
                .as_ref(),
        );
    } else if !demo {
        let times = store.calendar_sync_days()?;
        let oldest = (0..days)
            .filter_map(|day| times.get(&(first + Duration::days(day))))
            .min();
        if let Some(oldest) = oldest {
            text.push_str(&format!(
                "{}: {oldest}",
                language.text("Oldest calendar refresh")
            ));
        } else {
            text.push_str(language.text("Calendar refresh time is unknown.").as_ref());
        }
    }
    let meetings = found
        .iter()
        .take(25)
        .map(|(event, _)| (event.id.clone(), event.title.clone()))
        .collect();
    Ok(LocalAnswer {
        answer: crate::ai::Answer::new(text, Arc::new(Default::default()), Arc::new(Vec::new())),
        meetings,
    })
}

#[derive(Clone)]
pub struct Cache {
    pub root: PathBuf,
    pub account: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    #[serde(default)]
    pub query: String,
    pub from: Option<DateTime<FixedOffset>>,
    pub until: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub order: Order,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Relevance,
    Start,
}

impl Search {
    pub fn range(&self) -> Result<(DateTime<FixedOffset>, DateTime<FixedOffset>), String> {
        let from = self.from.unwrap_or_else(|| Local::now().fixed_offset());
        let until = self.until.unwrap_or(from + Duration::days(180));
        if self.query.len() > 200
            || self.query.chars().any(char::is_control)
            || self.offset > 5000
            || until <= from
            || until - from > Duration::days(366)
        {
            return Err(
                "Use a search of at most 200 characters and a zoned date range of at most one year"
                    .into(),
            );
        }
        Ok((from, until))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}

pub fn valid(name: &str, arguments: &Value) -> bool {
    match name {
        SEARCH => serde_json::from_value::<Search>(arguments.clone())
            .is_ok_and(|args| args.range().is_ok()),
        DETAIL | INSIGHTS => serde_json::from_value::<Id>(arguments.clone())
            .is_ok_and(|args| !args.id.is_empty() && args.id.len() <= 2000),
        _ => false,
    }
}

pub fn matches<'a>(calendar: &'a Calendar, search: &Search) -> Result<Vec<&'a Event>, String> {
    Ok(ranked_matches(calendar, search)?
        .into_iter()
        .map(|(event, _)| event)
        .collect())
}

fn ranked_matches<'a>(
    calendar: &'a Calendar,
    search: &Search,
) -> Result<Vec<(&'a Event, crate::search::Match)>, String> {
    let (from, until) = search.range()?;
    let query = calendar.search_index.query(&search.query);
    let mut events = calendar
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            if event.is_all_day {
                let (first, last) = event.calendar_dates();
                last >= from.with_timezone(&Local).date_naive()
                    && first
                        <= (until - Duration::nanoseconds(1))
                            .with_timezone(&Local)
                            .date_naive()
            } else {
                event.end > from && event.start < until
            }
        })
        .filter_map(|(index, event)| {
            calendar
                .search_index
                .matches(index, &query)
                .map(|matched| (event, matched))
        })
        .collect::<Vec<_>>();
    events.sort_by(|(left, a), (right, b)| {
        (if matches!(search.order, Order::Relevance) {
            a.rank().cmp(&b.rank())
        } else {
            std::cmp::Ordering::Equal
        })
        .then(left.start.cmp(&right.start))
        .then(left.id.cmp(&right.id))
    });
    Ok(events)
}

fn compact(event: &Event) -> Value {
    let start = event.start.with_timezone(&Local);
    let end = event.end.with_timezone(&Local);
    json!({"id":event.id,"title":event.title,
        "start":if event.is_all_day { event.start.to_rfc3339() } else { start.to_rfc3339() },
        "end":if event.is_all_day { event.end.to_rfc3339() } else { end.to_rfc3339() },
        "local_time_range":(!event.is_all_day).then(|| format!("{} to {}",start.format("%Y-%m-%d %H:%M %:z"),end.format("%Y-%m-%d %H:%M %:z"))),
        "all_day_dates":event.is_all_day.then(|| event.calendar_dates()),
        "is_all_day":event.is_all_day,"category":event.category,"status":event.status,
        "organizer":event.organizer,"attendees":event.attendees.iter().take(16).collect::<Vec<_>>(),
        "attendees_total":event.attendee_count(),"location":event.location,"source_url":event.source_url,
        "event_fingerprint":event.fingerprint()})
}

impl Cache {
    fn open(&self) -> Result<Store, String> {
        let store = Store::read_only(&self.root)?;
        if self.account.is_empty()
            || !store
                .calendar_owner()?
                .is_some_and(|account| account.eq_ignore_ascii_case(&self.account))
        {
            return Err("The cached calendar belongs to a different or unknown account; refresh before asking".into());
        }
        Ok(store)
    }

    pub fn read(&self, name: &str, arguments: Value) -> Result<Value, String> {
        if !valid(name, &arguments) {
            return Err("Invalid cached calendar request".into());
        }
        let store = self.open()?;
        let calendar = store.calendar()?;
        if name == SEARCH {
            let search: Search =
                serde_json::from_value(arguments).map_err(|_| "Invalid cached search")?;
            let (from, until) = search.range()?;
            let events = ranked_matches(&calendar, &search)?;
            let first = from.with_timezone(&Local).date_naive();
            let last = (until - Duration::nanoseconds(1))
                .with_timezone(&Local)
                .date_naive();
            let days = (0..=(last - first).num_days())
                .map(|day| first + Duration::days(day))
                .collect::<Vec<_>>();
            let sync = store.calendar_sync_days()?;
            let covered = days
                .iter()
                .filter(|day| calendar.coverage.contains(day))
                .count();
            let refreshed = days
                .iter()
                .filter_map(|day| sync.get(day))
                .collect::<Vec<_>>();
            return Ok(json!({
                "meetings":events.iter().skip(search.offset).take(25).map(|(event, matched)| {
                    let mut value = compact(event);
                    value["search_match"] = json!(matched);
                    value
                }).collect::<Vec<_>>(),
                "query":search.query,"order":if matches!(search.order, Order::Start) {"start"} else {"relevance_then_start"},
                "total_matches":events.len(),"next_offset":(events.len()>search.offset+25).then_some(search.offset+25),
                "coverage":{"from":from,"until_exclusive":until,"complete":covered==days.len(),
                    "covered_days":covered,"requested_days":days.len(),"all_days_have_sync_time":refreshed.len()==days.len(),
                    "oldest_sync":refreshed.iter().min(),"newest_sync":refreshed.iter().max()},
                "note":"Local snapshot, not a live provider read. Results are candidates, not verified entity matches. search_match shows each keyword's matched field, normalized token and typo distance; fuzzy similarity is not identity or semantic relevance. Inspect actual meeting metadata and reject unrelated candidates. Incomplete or old coverage cannot prove absence. No matches are not proof that an event or material does not exist."
            }));
        }
        let id: Id = serde_json::from_value(arguments).map_err(|_| "Invalid meeting identity")?;
        let event = calendar
            .events
            .iter()
            .find(|event| event.id == id.id)
            .ok_or("Meeting is not in this account's cache")?;
        if name == DETAIL {
            return Ok(compact(event));
        }
        let saved = store
            .briefing(&id.id)?
            .map(|text| {
                serde_json::from_str::<Value>(&text).map_err(|_| "Saved Insights are invalid")
            })
            .transpose()?;
        if saved
            .as_ref()
            .is_some_and(|value| value.to_string().len() > 56_000)
        {
            return Err("Saved Insights exceed the bounded tool result; narrow the request".into());
        }
        let current = saved.as_ref().is_some_and(|value| {
            value["tomlook_event_fingerprint"].as_str() == Some(event.fingerprint().as_str())
        });
        Ok(
            json!({"meeting":compact(event),"insights":saved,"matches_current_meeting":current,
            "note":"Saved AI analysis is derived evidence, not verified current fact. Check captured source dates and gaps. Missing or stale Insights do not prove missing preparation."}),
        )
    }
}

struct Handler {
    cache: Cache,
    recorder: Arc<Recorder>,
    run: Arc<crate::chat::Run>,
}

#[async_trait]
impl github_copilot_sdk::tool::ToolHandler for Handler {
    async fn call(
        &self,
        invocation: github_copilot_sdk::ToolInvocation,
    ) -> Result<github_copilot_sdk::ToolResult, github_copilot_sdk::Error> {
        let name = invocation.tool_name;
        self.run.stage(crate::chat::tool_label(&name));
        let result = async {
            self.run.reserve(false)?;
            let permit = crate::context::read_gate()
                .acquire_owned()
                .await
                .map_err(|_| "Local read admission stopped")?;
            if self
                .run
                .cancelled
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err("This question was stopped; no further read was sent".into());
            }
            let cache = self.cache.clone();
            let name = name.clone();
            let arguments = invocation.arguments;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                cache.read(&name, arguments)
            })
            .await
            .map_err(|_| "Cached calendar worker stopped")?
        }
        .await;
        if let Ok(value) = &result {
            self.run.protect_result(value);
        }
        let succeeded = result.is_ok();
        let output = crate::context::result_for(&self.recorder, &name, result);
        self.run.completed(&name, succeeded);
        Ok(output)
    }
}

pub fn tools(
    cache: Cache,
    recorder: Arc<Recorder>,
    run: Arc<crate::chat::Run>,
) -> Vec<github_copilot_sdk::Tool> {
    let handler = Arc::new(Handler {
        cache,
        recorder,
        run,
    });
    vec![
        github_copilot_sdk::Tool::new(SEARCH)
            .with_description("Search the signed-in user's locally saved calendar with ranked fuzzy keyword retrieval. First interpret the user's intent and extract a few distinctive company/person/topic keywords, not the whole natural-language question. All keywords must match title, organizer, attendees, location or category; use from/until for dates, not date words in query. Matching ignores case, separators and common Latin accents; alphabetic keywords of 4-9 characters allow one typo, 10-64 allow two, including adjacent swaps. Short and numeric keywords are not typo-corrected. NorthStar also finds North Star. Results are CANDIDATES: inspect their real metadata and search_match evidence, reject irrelevant similar names, and resolve ambiguity rather than assume identity. This is not semantic entity resolution. Prefer this cache over remote calendar reads. from/until are RFC3339 zoned times, until exclusive; default now through 180 days. Returns up to 25 matches ordered by relevance then time, next_offset and verified coverage/freshness. Empty query returns chronological meetings. Before a negative conclusion, retry a shorter distinctive keyword, a separate alternative query, or a bounded unfiltered range; complete date coverage does not establish name resolution.")
            .with_parameters(json!({"type":"object","properties":{"query":{"type":"string","maxLength":200},"from":{"type":"string"},"until":{"type":"string"},"offset":{"type":"integer","minimum":0,"maximum":5000},"order":{"type":"string","enum":["relevance","start"],"description":"Default relevance, then meeting start. Use start for nearest/earliest requests so a later stronger name match cannot hide an earlier candidate."}},"additionalProperties":false}))
            .with_handler(handler.clone()),
        github_copilot_sdk::Tool::new(DETAIL).with_description("Read one saved meeting by an already known id. Includes full local times, response state and bounded participant metadata. Calendar search already returns these same fields; do not reread its meetings merely to confirm the title, dates, category or invitation response. Read-only; never changes a meeting.")
            .with_parameters(json!({"type":"object","properties":{"id":{"type":"string","maxLength":2000}},"required":["id"],"additionalProperties":false})).with_handler(handler.clone()),
        github_copilot_sdk::Tool::new(INSIGHTS).with_description("Read saved meeting Insights before researching it again. Reports whether the derived analysis matches the current event fingerprint and includes captured sources/gaps. Old AI conclusions are not fresh original-source verification.")
            .with_parameters(json!({"type":"object","properties":{"id":{"type":"string","maxLength":2000}},"required":["id"],"additionalProperties":false})).with_handler(handler),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn fixture() -> (tempfile::TempDir, Store, Calendar, Cache) {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let mut events = crate::calendar::fixture(35);
        let first = Local::now().date_naive();
        let start = Local
            .from_local_datetime(&first.and_hms_opt(9, 0, 0).unwrap())
            .single()
            .unwrap()
            .fixed_offset();
        for (index, event) in events.iter_mut().enumerate() {
            event.id = format!("saved-{index}");
            event.title = format!("Synthetic workshop {index}");
            event.attendees = vec!["Jamie Brooks <jamie@example.invalid>".into()];
            event.start = start + Duration::minutes(index as i64 * 30);
            event.end = event.start + Duration::minutes(25);
            event.is_all_day = false;
        }
        let coverage = (0..3)
            .map(|offset| first + Duration::days(offset))
            .collect::<BTreeSet<_>>();
        let calendar = Calendar::build(events, coverage, Vec::new()).unwrap();
        store
            .save_workiq_calendar(&calendar, "owner@example.invalid")
            .unwrap();
        let cache = Cache {
            root: root.path().into(),
            account: "OWNER@example.invalid".into(),
        };
        (root, store, calendar, cache)
    }

    #[test]
    fn search_is_paged_chronological_with_explicit_coverage_and_freshness() {
        let (_root, _store, calendar, cache) = fixture();
        let from = calendar.events[0].start - Duration::hours(1);
        let until = from + Duration::days(2);
        let result = cache
            .read(
                SEARCH,
                json!({"query":"Jamie workshop","from":from,"until":until}),
            )
            .unwrap();
        assert_eq!(result["meetings"].as_array().unwrap().len(), 25);
        assert_eq!(result["total_matches"], 35);
        assert_eq!(result["next_offset"], 25);
        assert_eq!(result["coverage"]["complete"], true);
        assert_eq!(result["coverage"]["all_days_have_sync_time"], true);
        assert!(result["coverage"]["oldest_sync"].as_str().is_some());
        let page = cache
            .read(
                SEARCH,
                json!({"query":"Jamie","from":from,"until":until,"offset":25}),
            )
            .unwrap();
        assert_eq!(page["meetings"].as_array().unwrap().len(), 10);
        assert!(page["next_offset"].is_null());
        let incomplete = cache
            .read(SEARCH, json!({"from":from,"until":from+Duration::days(7)}))
            .unwrap();
        assert_eq!(incomplete["coverage"]["complete"], false);
    }

    #[test]
    fn company_spelling_variants_find_workshops_and_hackathons_in_tools_and_local_search() {
        let (_root, mut store, calendar, cache) = fixture();
        let mut events = calendar.events[..6].to_vec();
        for event in &mut events {
            event.title = "Unrelated meeting".into();
            event.organizer.clear();
            event.attendees.clear();
            event.location.clear();
            event.category.clear();
        }
        events[0].title = "Dr. Example / Platform workshop".into();
        events[1].title = "Dr-Example Group AI Agents Hackathon".into();
        events[2].organizer = "Office <team@dr.example.invalid>".into();
        events[3].title = "Dr".into();
        events[3].organizer = "Example".into();
        events[4].attendees = vec!["Dr".into(), "Example".into()];
        let calendar = Calendar::build(events, calendar.coverage, Vec::new()).unwrap();
        store
            .save_workiq_calendar(&calendar, "owner@example.invalid")
            .unwrap();
        for query in [
            "DrExample",
            "dr.example",
            "DR-EXAMPLE",
            "DrExample hackathon",
        ] {
            let result = cache
                .read(
                    SEARCH,
                    json!({"query":query,"from":calendar.events[0].start-Duration::hours(1)}),
                )
                .unwrap();
            let expected = if query.ends_with("hackathon") { 1 } else { 3 };
            assert_eq!(result["total_matches"], expected, "{query}: {result}");
            let local = local_answer(
                &store,
                &calendar,
                LocalQuery::Search(query.into()),
                None,
                crate::i18n::Language::En,
                false,
            )
            .unwrap();
            assert_eq!(local.meetings.len(), expected as usize, "{query}");
            assert!(local.answer.text.contains("Hackathon"));
            if expected == 3 {
                assert!(local.answer.text.contains("Platform workshop"));
            }
        }
    }

    #[test]
    fn fuzzy_keywords_rank_exact_matches_first_and_expose_verifiable_candidates() {
        let (_root, mut store, calendar, cache) = fixture();
        let mut events = calendar.events[..8].to_vec();
        for event in &mut events {
            event.start += Duration::days(1);
            event.end += Duration::days(1);
            event.title = "Unrelated meeting".into();
            event.organizer.clear();
            event.attendees.clear();
            event.location.clear();
            event.category.clear();
        }
        events[0].title = "Dr. Exampl workshop".into();
        events[1].title = "Dr. Example workshop".into();
        events[2].title = "Dr-Example hackathon".into();
        events[3].title = "Český tým / konzultace".into();
        events[4].attendees = vec!["Olvin <olvin@example.invalid>".into()];
        events[5].attendees = vec!["Olven <olven@example.invalid>".into()];
        events[6].title = "Room 102".into();
        events[7].title = "Mac workshop".into();
        let calendar = Calendar::build(events, calendar.coverage, Vec::new()).unwrap();
        store
            .save_workiq_calendar(&calendar, "owner@example.invalid")
            .unwrap();
        for (query, expected) in [
            ("DrExample", vec![1, 2, 0]),
            ("DrExampel", vec![1, 2, 0]),
            ("DrExample workhsop", vec![1, 0]),
            ("DrExample / workhsop", vec![1, 0]),
            ("DrExample hackaton", vec![2]),
            ("DrExample --- hackaton", vec![2]),
            ("cesky tym", vec![3]),
            ("Olvn", vec![4, 5]),
            ("Room 101", vec![]),
            ("Max", vec![]),
            ("---", vec![]),
        ] {
            let result = cache.read(SEARCH, json!({"query":query})).unwrap();
            let actual = result["meetings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| event["id"].as_str().unwrap())
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .map(|index| calendar.events[*index].id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{query}: {result}");
            let local = local_answer(
                &store,
                &calendar,
                LocalQuery::Search(query.into()),
                None,
                crate::i18n::Language::Cs,
                false,
            )
            .unwrap();
            assert_eq!(
                local
                    .meetings
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{query}"
            );
        }
        let result = cache.read(SEARCH, json!({"query":"DrExample"})).unwrap();
        assert_eq!(result["order"], "relevance_then_start");
        assert_eq!(result["meetings"][0]["search_match"]["approximate"], false);
        assert_eq!(result["meetings"][2]["search_match"]["approximate"], true);
        assert_eq!(result["meetings"][2]["search_match"]["edit_distance"], 1);
        assert_eq!(
            result["meetings"][2]["search_match"]["terms"][0]["field"],
            "title"
        );
        let by_start = cache
            .read(SEARCH, json!({"query":"DrExample","order":"start"}))
            .unwrap();
        assert_eq!(by_start["order"], "start");
        assert_eq!(by_start["meetings"][0]["id"], calendar.events[0].id);
        let result = cache.read(SEARCH, json!({"query":"Olvn"})).unwrap();
        assert_eq!(
            result["meetings"][0]["search_match"]["terms"][0]["matched"],
            "olvin"
        );
        assert_eq!(
            result["meetings"][1]["search_match"]["terms"][0]["matched"],
            "olven"
        );
    }

    #[test]
    fn fuzzy_ranking_pages_without_duplicates_and_preserves_range_exclusions() {
        let (_root, mut store, calendar, cache) = fixture();
        let mut events = calendar.events;
        for (index, event) in events.iter_mut().enumerate() {
            event.title = if index < 10 { "Workshap" } else { "Workshop" }.into();
        }
        let calendar = Calendar::build(events, calendar.coverage, Vec::new()).unwrap();
        store
            .save_workiq_calendar(&calendar, "owner@example.invalid")
            .unwrap();
        let from = calendar.events[0].start - Duration::hours(1);
        let first = cache
            .read(SEARCH, json!({"query":"workshop","from":from}))
            .unwrap();
        assert_eq!(first["next_offset"], 25);
        assert!(
            first["meetings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|event| { event["search_match"]["approximate"] == false })
        );
        let second = cache
            .read(SEARCH, json!({"query":"workshop","from":from,"offset":25}))
            .unwrap();
        assert_eq!(second["meetings"].as_array().unwrap().len(), 10);
        let mut ids = BTreeSet::new();
        for page in [&first, &second] {
            for event in page["meetings"].as_array().unwrap() {
                assert!(ids.insert(event["id"].as_str().unwrap()));
            }
        }
        let until = calendar.events[10].start;
        let narrowed = cache
            .read(
                SEARCH,
                json!({"query":"workshop","from":from,"until":until}),
            )
            .unwrap();
        assert_eq!(narrowed["total_matches"], 10);
        assert!(
            narrowed["meetings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|event| { event["search_match"]["approximate"] == true })
        );
        let chronological = matches(
            &calendar,
            &Search {
                from: Some(from),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            chronological
                .windows(2)
                .all(|pair| pair[0].start <= pair[1].start)
        );
    }

    #[test]
    fn fuzzy_search_over_five_thousand_meetings_is_bounded_and_exact_first() {
        let calendar = Calendar::build(
            crate::calendar::fixture(5000),
            Default::default(),
            Vec::new(),
        )
        .unwrap();
        let search = Search {
            query: "workhsop engineering".into(),
            from: Some("2026-10-01T00:00:00Z".parse().unwrap()),
            until: Some("2026-12-01T00:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let found = ranked_matches(&calendar, &search).unwrap();
        assert!(!found.is_empty());
        assert!(
            found.iter().all(
                |(event, matched)| event.title == "Engineering workshop" && matched.approximate
            )
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "Fuzzy lookup exceeded the 500 ms debug regression ceiling: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn cached_multi_day_meetings_keep_both_local_dates_in_the_display_range() {
        let (_root, mut store, calendar, cache) = fixture();
        let mut event = calendar.events[0].clone();
        event.end = event.start + Duration::days(1) + Duration::hours(8);
        let calendar = Calendar::build(vec![event], calendar.coverage, Vec::new()).unwrap();
        store
            .save_workiq_calendar(&calendar, "owner@example.invalid")
            .unwrap();
        let event = &calendar.events[0];
        let result = cache.read(DETAIL, json!({"id":event.id})).unwrap();
        let range = result["local_time_range"].as_str().unwrap();
        for instant in [event.start, event.end] {
            assert!(
                range.contains(
                    &instant
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M %:z")
                        .to_string()
                )
            );
        }
    }

    #[test]
    fn read_only_cache_rechecks_owner_and_insights_fingerprints_on_every_read() {
        let (_root, mut store, calendar, cache) = fixture();
        let event = &calendar.events[0];
        let payload = json!({"summary":"Synthetic saved evidence","tomlook_event_fingerprint":event.fingerprint()});
        store
            .save_prepared(&Default::default(), &event.id, &payload)
            .unwrap();
        let result = cache.read(INSIGHTS, json!({"id":event.id})).unwrap();
        assert_eq!(result["matches_current_meeting"], true);
        let mut changed = calendar.clone();
        changed.events[0].title = "Moved workshop".into();
        store
            .save_workiq_calendar(&changed, "owner@example.invalid")
            .unwrap();
        assert_eq!(
            cache.read(INSIGHTS, json!({"id":event.id})).unwrap()["matches_current_meeting"],
            false
        );
        store
            .save_workiq_calendar(&changed, "new-owner@example.invalid")
            .unwrap();
        assert!(
            cache
                .read(DETAIL, json!({"id":event.id}))
                .unwrap_err()
                .contains("different")
        );
    }

    #[test]
    fn zoned_times_are_compared_as_instants_and_all_day_dates_are_not_shifted() {
        let mut events = crate::calendar::fixture(2);
        events[0].id = "late".into();
        events[0].start = "2026-10-26T09:00:00+00:00".parse().unwrap();
        events[0].end = events[0].start + Duration::minutes(30);
        events[0].is_all_day = false;
        events[1].id = "early".into();
        events[1].start = "2026-10-26T10:00:00+02:00".parse().unwrap();
        events[1].end = events[1].start + Duration::minutes(30);
        events[1].is_all_day = false;
        let calendar = Calendar::build(events, Default::default(), Vec::new()).unwrap();
        let args = Search {
            from: Some("2026-10-26T07:00:00Z".parse().unwrap()),
            until: Some("2026-10-26T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let events = matches(&calendar, &args).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            ["early", "late"]
        );
        let mut all_day = events[0].clone();
        all_day.is_all_day = true;
        all_day.start = "2026-10-26T00:00:00-10:00".parse().unwrap();
        all_day.end = all_day.start + Duration::days(1);
        let compact = compact(&all_day);
        assert_eq!(
            compact["all_day_dates"],
            json!(["2026-10-26", "2026-10-26"])
        );
        assert_eq!(compact["start"], all_day.start.to_rfc3339());
    }

    #[tokio::test]
    async fn completed_activity_never_races_a_missing_evidence_checkpoint() {
        let (_root, _store, _calendar, cache) = fixture();
        let recorder = Arc::new(Recorder::new(
            Arc::new(crate::evidence::Redactor::default()),
        ));
        let recorded = recorder.clone();
        let run = crate::chat::Run::new(
            1,
            crate::chat::Mode::Deep,
            Arc::new(move |progress| {
                if !progress.steps.is_empty() {
                    assert!(!recorded.snapshot().unwrap().records.is_empty());
                }
            }),
        );
        let handler = Handler {
            cache,
            recorder,
            run,
        };
        let invocation = serde_json::from_value(
            json!({"sessionId":"fixture","toolCallId":"one","toolName":SEARCH,"arguments":{}}),
        )
        .unwrap();
        github_copilot_sdk::tool::ToolHandler::call(&handler, invocation)
            .await
            .unwrap();
    }

    #[test]
    fn local_commands_use_czech_insights_and_preserve_all_day_dates() {
        let (_root, mut store, mut calendar, _cache) = fixture();
        let event = calendar.events[0].clone();
        store.save_prepared(&Default::default(),&event.id,&json!({
            "summary":"English sample","preparation":["English preparation"],"tomlook_event_fingerprint":event.fingerprint(),
            "cs":{"summary":"Český přehled","preparation":["Česká příprava"]}
        })).unwrap();
        let answer = local_answer(
            &store,
            &calendar,
            LocalQuery::Insights,
            Some(event),
            crate::i18n::Language::Cs,
            false,
        )
        .unwrap();
        assert!(answer.answer.text.contains("Český přehled"));
        assert!(answer.answer.text.contains("Česká příprava"));
        assert!(!answer.answer.text.contains("English sample"));
        let day = Local::now().date_naive();
        calendar.events[0].is_all_day = true;
        calendar.events[0].start = format!("{day}T00:00:00-10:00").parse().unwrap();
        calendar.events[0].end = calendar.events[0].start + Duration::days(1);
        let answer = local_answer(
            &store,
            &calendar,
            LocalQuery::Today,
            None,
            crate::i18n::Language::Cs,
            false,
        )
        .unwrap();
        assert!(answer.answer.text.contains("Celý den"));
        assert!(!answer.answer.text.contains("00:00"));
        assert!(!answer.answer.text.contains("### "));
        assert!(
            !answer.answer.text.contains(
                crate::i18n::Language::Cs
                    .text("Local calendar / no AI")
                    .as_ref()
            )
        );
    }

    #[test]
    fn typed_tools_reject_unbounded_ranges_writes_unknown_fields_and_invalid_ids() {
        assert!(!valid(SEARCH, &json!({"query":"x".repeat(201)})));
        assert!(!valid(
            SEARCH,
            &json!({"from":"2026-01-01T00:00:00Z","until":"2028-01-01T00:00:00Z"})
        ));
        assert!(!valid(SEARCH, &json!({"sql":"DELETE FROM calendar"})));
        assert!(!valid(DETAIL, &json!({"id":""})));
        assert!(!valid("tomlook_calendar_write", &json!({})));
        assert!(!valid(SEARCH, &json!({"offset":5001})));
        assert!(!valid(SEARCH, &json!({"order":"unsupported"})));
        assert!(valid(SEARCH, &json!({})));
    }
}
