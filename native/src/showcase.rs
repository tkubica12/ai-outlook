use crate::calendar::Event;
use chrono::{DateTime, Datelike, Duration, Local, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;

/// What `--demo-screenshot` captures; chosen with `--demo-scene`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scene {
    /// Calendar with the selected workshop's details and sample Insights.
    #[default]
    Details,
    /// Expanded sample Insights with ratings and an open "not helpful" reason panel.
    Feedback,
    /// Settings > AI prompts summarizing fictional sample feedback.
    Prompts,
    /// The local task list next to the ended sample meeting, whose notes created some of the tasks.
    Tasks,
    /// Details of the ended sample meeting: notes, its tasks and a saved follow-up e-mail.
    FollowUp,
    /// The My day panel for the demo's today.
    MyDay,
    /// Settings > Notifications with meeting reminders and the morning overview.
    Notifications,
    /// The weekly review of the demo's current week.
    Weekly,
}

impl Scene {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "details" => Ok(Self::Details),
            "feedback" => Ok(Self::Feedback),
            "prompts" => Ok(Self::Prompts),
            "tasks" => Ok(Self::Tasks),
            "followup" => Ok(Self::FollowUp),
            "myday" => Ok(Self::MyDay),
            "notifications" => Ok(Self::Notifications),
            "weekly" => Ok(Self::Weekly),
            _ => Err("--demo-scene must be details, feedback, prompts, tasks, followup, myday, notifications or weekly".into()),
        }
    }

    /// Whether the scene first selects [`Scene::meeting`] and waits for its saved document.
    pub fn selects_meeting(self) -> bool {
        !matches!(self, Self::MyDay | Self::Notifications | Self::Weekly)
    }

    /// The sample meeting whose details the scene shows.
    pub fn meeting(self) -> &'static str {
        if matches!(self, Self::Tasks | Self::FollowUp) {
            ENDED_MEETING
        } else {
            "Northstar / Architecture workshop"
        }
    }
}

/// The fictional preview meeting that already ended and has sample notes, tasks and a follow-up.
pub const ENDED_MEETING: &str = "Northstar / Pilot planning";

/// Fictional ratings of sample Insights, saved only by the `prompts` screenshot scene.
pub fn sample_feedback(
    events: &[Event],
    prompts: &crate::prompts::Prompts,
    now: DateTime<Utc>,
) -> Result<Vec<crate::feedback::Entry>, String> {
    use crate::feedback::{DOCUMENT, Output, Rating, Reason};
    type Sample = (&'static str, &'static str, Rating, &'static [Reason], &'static str);
    const SAMPLES: &[Sample] = &[
        ("Northstar / Architecture workshop", "talking_points", Rating::Down, &[Reason::TooVague],
            "Name the concrete trade-offs instead of a generic order."),
        ("Cedar / Discovery call", "preparation", Rating::Down, &[Reason::TooLong],
            "Three bullets are enough for a short call."),
        ("Orbit / Project kickoff", "risks", Rating::Down, &[Reason::NotRelevant], ""),
        ("Northstar / Pilot review", "talking_points", Rating::Down,
            &[Reason::TooVague, Reason::MissingImportant],
            "Missing the open budget question from the last review."),
        ("Weekly planning", DOCUMENT, Rating::Up, &[], ""),
        ("Product design review", "summary", Rating::Up, &[], "Clear and short."),
        ("Cedar / Solution review", DOCUMENT, Rating::Up, &[], ""),
    ];
    SAMPLES
        .iter()
        .enumerate()
        .map(|(index, (title, section, rating, reasons, comment))| {
            let event = events
                .iter()
                .find(|event| event.title == *title)
                .ok_or("Sample feedback meeting is missing")?;
            let payload = insights(event)?;
            let output = Output::new(
                crate::feedback::PREVIEW_ACCOUNT,
                event,
                &payload,
                crate::i18n::Language::En,
                prompts,
            )
            .ok_or("Sample feedback output is invalid")?;
            let target = output
                .target(&payload, section)
                .ok_or("Sample feedback section is missing")?;
            output.entry(
                &target,
                *rating,
                reasons.iter().copied().collect(),
                comment,
                now - Duration::hours(index as i64 + 1),
            )
        })
        .collect()
}

// Day, start minute, duration, title, category, location, response.
const MEETINGS: &[(i64, u32, i64, &str, &str, &str, &str)] = &[
    (0, 0, 1440, "Product launch week", "Office", "", "accepted"),
    (
        0,
        510,
        30,
        "Weekly planning",
        "Office",
        "Studio / Room 2",
        "accepted",
    ),
    (
        0,
        555,
        60,
        "Focus / Sprint design",
        "Focus",
        "",
        "organizer",
    ),
    (
        0,
        630,
        45,
        "Cedar / Discovery call",
        "Video calls",
        "Online",
        "accepted",
    ),
    (0, 720, 60, "Lunch & a walk", "Personal", "", "organizer"),
    (
        0,
        810,
        60,
        "Product design review",
        "Office",
        "Studio / Room 2",
        "accepted",
    ),
    (
        0,
        900,
        30,
        "Community office hours",
        "Video calls",
        "Online",
        "notResponded",
    ),
    (
        0,
        960,
        60,
        "Focus / Workshop prep",
        "Focus",
        "",
        "organizer",
    ),
    (
        1,
        510,
        30,
        "Travel / Northstar",
        "Logistics",
        "City centre",
        "organizer",
    ),
    (
        1,
        555,
        90,
        "Northstar / Architecture workshop",
        "Customer",
        "Northstar / Demo studio",
        "accepted",
    ),
    (
        1,
        660,
        30,
        "Workshop follow-up",
        "Customer",
        "Northstar / Demo studio",
        "accepted",
    ),
    (
        1,
        720,
        60,
        "Lunch with the team",
        "Personal",
        "",
        "accepted",
    ),
    (
        1,
        795,
        30,
        "Travel / Office",
        "Logistics",
        "City centre",
        "organizer",
    ),
    (
        1,
        855,
        45,
        "Engineering sync",
        "Office",
        "Studio / Room 2",
        "accepted",
    ),
    (1, 945, 60, "Focus / Prototype", "Focus", "", "organizer"),
    (
        2,
        525,
        45,
        "Orbit / Project kickoff",
        "Video calls",
        "Online",
        "accepted",
    ),
    (2, 600, 90, "Focus / Build", "Focus", "", "organizer"),
    (2, 720, 45, "Lunch & a walk", "Personal", "", "organizer"),
    (
        2,
        795,
        60,
        "Cedar / Solution review",
        "Customer",
        "Cedar / Demo studio",
        "accepted",
    ),
    (
        2,
        885,
        30,
        "Partner drop-in",
        "Video calls",
        "Online",
        "notResponded",
    ),
    (
        2,
        930,
        60,
        "Team learning lab",
        "Office",
        "Studio / Room 2",
        "accepted",
    ),
    (
        3,
        0,
        1440,
        "Team collaboration day",
        "Office",
        "",
        "accepted",
    ),
    (
        3,
        510,
        30,
        "Coffee & roadmap",
        "Office",
        "Studio / Room 2",
        "accepted",
    ),
    (
        3,
        570,
        90,
        "Orbit / Hands-on lab",
        "Customer",
        "Studio / Demo lab",
        "accepted",
    ),
    (
        3,
        690,
        45,
        "Lab retrospective",
        "Office",
        "Studio / Room 2",
        "accepted",
    ),
    (
        3,
        765,
        45,
        "Lunch with the team",
        "Personal",
        "",
        "accepted",
    ),
    (
        3,
        855,
        45,
        "Cedar / Checkpoint",
        "Video calls",
        "Online",
        "accepted",
    ),
    (
        3,
        945,
        60,
        "Focus / Documentation",
        "Focus",
        "",
        "organizer",
    ),
    (
        4,
        525,
        45,
        "Northstar / Pilot review",
        "Customer",
        "Online",
        "accepted",
    ),
    (4, 600, 60, "Focus / Backlog", "Focus", "", "organizer"),
    (
        4,
        690,
        30,
        "Design catch-up",
        "Video calls",
        "Online",
        "notResponded",
    ),
    (4, 750, 60, "Lunch & a walk", "Personal", "", "organizer"),
    (
        4,
        840,
        60,
        "Demo day / Show & tell",
        "Office",
        "Studio / Demo lab",
        "accepted",
    ),
    (
        4,
        930,
        45,
        "Weekly wrap-up",
        "Office",
        "Studio / Room 2",
        "organizer",
    ),
];

/// Meetings the demo user organizes with colleagues (other organized items are solo blocks).
const HOSTED: &[&str] = &["Weekly wrap-up"];

/// Weeks of the preview, relative to the current one, in the order the meeting cycles fill them:
/// two earlier weeks give the weekly review something to compare with.
const WEEKS: [i64; 6] = [0, -1, 1, -2, 2, 3];

/// A fictional Teams join block with no agenda of its own.
const TEAMS_BLOCK: &str = "<div>________________________________________________________________________________</div><div>Microsoft Teams meeting</div><div>Join on your computer, mobile app or room device</div><div><a href=\"https://example.invalid/teams/join\">Click here to join the meeting</a></div><div>Meeting ID: 000 000 000 000</div><div>Passcode: demo00</div><div>________________________________________________________________________________</div>";

/// Fictional invitation texts: agendas, empty descriptions and Teams join blocks only. Meetings
/// not listed have no cached description, so their agenda is unknown.
const DESCRIPTIONS: &[(&str, &str)] = &[
    ("Weekly planning", "<p>Agenda:</p><ol><li>Review last week's outcomes</li><li>Agree on this week's priorities</li><li>Confirm owners for open items</li></ol>"),
    ("Cedar / Discovery call", ""),
    ("Product design review", "<p>Walk through the new onboarding screens and decide which variant goes to usability testing.</p>"),
    ("Northstar / Architecture workshop", "<p>Agenda: compare the two architecture options, demo the prototype and agree on pilot success criteria.</p>"),
    ("Workshop follow-up", "<p>Collect open questions from the workshop and assign owners.</p>"),
    ("Engineering sync", "<p>Program: stav buildu, rizika vydání a další kroky.</p>"),
    ("Orbit / Project kickoff", "<p>Introductions, project goals, timeline and the first milestone.</p>"),
    ("Cedar / Solution review", ""),
    ("Team learning lab", "<p>Hands-on session: prompt patterns for meeting preparation.</p>"),
    ("Coffee & roadmap", "<p>Agenda: TBD</p>"),
    ("Orbit / Hands-on lab", "<p>Guided lab: deploy the sample app, then review costs and monitoring.</p>"),
    ("Cedar / Checkpoint", ""),
    ("Northstar / Pilot review", "<p>Review pilot metrics against the three success criteria and decide on phase two.</p>"),
    ("Demo day / Show & tell", "<p>Five-minute demos from each team, then voting.</p>"),
    ("Weekly wrap-up", "<p>&nbsp;</p>"),
    ("Northstar / Escalation check-in", ""),
    (ENDED_MEETING, "<p>Agenda: agree on the pilot scope, success criteria and owners.</p>"),
];

/// A fictional customer contact for meetings with one of the demo's customers.
fn customer_contact(title: &str) -> Option<&'static str> {
    [
        ("Northstar /", "Dana Novak <dana@northstar.invalid>"),
        ("Cedar /", "Sam Ortiz <sam@cedar.invalid>"),
        ("Orbit /", "Kim Lambert <kim@orbit.invalid>"),
    ]
    .into_iter()
    .find(|(prefix, _)| title.starts_with(prefix))
    .map(|(_, contact)| contact)
}

/// The fictional cached description of a demo meeting: an agenda, an empty text or only the
/// Teams join block.
fn description(title: &str) -> Option<Value> {
    DESCRIPTIONS
        .iter()
        .find(|(name, _)| *name == title)
        .map(|(_, agenda)| {
            json!({"contentType": "html", "content": format!("<html><body>{agenda}{TEAMS_BLOCK}</body></html>")})
        })
}

pub fn events(now: DateTime<Local>, count: usize) -> Vec<Event> {
    let monday = crate::calendar::monday(now.date_naive());
    (0..count)
        .map(|index| {
            let (day, minute, duration, title, category, location, status) =
                MEETINGS[index % MEETINGS.len()];
            let date = monday + Duration::days(day + WEEKS[(index / MEETINGS.len()) % WEEKS.len()] * 7);
            let start = date
                .and_hms_opt(minute / 60, minute % 60, 0)
                .expect("valid demo time")
                .and_local_timezone(Local)
                .earliest()
                .expect("demo daytime exists")
                .fixed_offset();
            Event {
                id: format!("synthetic-{index}"),
                title: title.into(),
                start,
                end: start + Duration::minutes(duration),
                category: category.into(),
                attendees: if (status == "organizer" && !HOSTED.contains(&title)) || minute == 0 {
                    Vec::new()
                } else {
                    [
                        "Alex Rivers <alex@example.invalid>",
                        "Jamie Brooks <jamie@example.invalid>",
                        "Morgan Lee <morgan@example.invalid>",
                    ]
                    .into_iter()
                    .chain(customer_contact(title))
                    .map(Into::into)
                    .collect()
                },
                organizer: "Taylor Quinn <taylor@example.invalid>".into(),
                location: location.into(),
                status: status.into(),
                briefing_status: "ready".into(),
                is_all_day: minute == 0,
                source_url: None,
                extra: BTreeMap::from([
                    ("synthetic".into(), json!(true)),
                    ("type".into(), json!("singleInstance")),
                    ("isOrganizer".into(), json!(status == "organizer")),
                    ("categories".into(), json!([category])),
                ])
                .into_iter()
                // Like Outlook's default, all-day items are shown as free, so they leave focus time.
                .chain((minute == 0).then(|| ("showAs".into(), json!("free"))))
                .chain(description(title).map(|body| ("body".into(), body)))
                .collect(),
            }
        })
        .collect()
}

pub fn insights(event: &Event) -> Result<Value, String> {
    if event.extra.get("synthetic") != Some(&json!(true)) {
        return Err("Demo Insights require an explicitly synthetic meeting".into());
    }
    let (summary, cs_summary) = if event.title == "Northstar / Architecture workshop" {
        (
            "You lead the workshop. Compare two architecture options, demo the prototype, and agree on a small pilot with clear success criteria.",
            "Workshop vedete vy. Porovnejte dvě varianty architektury, ukažte prototyp a domluvte malý pilot s jasnými kritérii úspěchu.",
        )
    } else {
        (
            "Keep the meeting focused: review the demo plan, surface open questions, and leave with one clear next step and an owner.",
            "Držte se cíle schůzky: projděte ukázkový plán, vyjasněte otevřené otázky a domluvte konkrétní další krok a odpovědnost.",
        )
    };
    let raw = json!({
        "summary": summary,
        "preparation": [
            "Prepare a five-minute prototype walkthrough.",
            "Bring a one-page comparison of the two architecture options.",
            "Confirm who owns the pilot and how success will be measured."
        ],
        "talking_points": ["Start with the workflow, then explain the trade-offs."],
        "open_questions": [
            "Which workflow should the pilot validate first?",
            "What would a successful two-week pilot demonstrate?"
        ],
        "risks": ["Avoid expanding the pilot scope before agreeing on success criteria."],
        "gaps": ["All names, meetings and context in this preview are fictional."],
        "sources": [],
        "cs": {
            "summary": cs_summary,
            "preparation": [
                "Připravte pětiminutovou ukázku prototypu.",
                "Přineste jednostránkové porovnání obou variant architektury.",
                "Domluvte odpovědnost za pilot a způsob měření úspěchu."
            ],
            "talking_points": ["Začněte pracovním postupem, potom vysvětlete kompromisy."],
            "open_questions": [
                "Který pracovní postup má pilot ověřit jako první?",
                "Co má úspěšný dvoutýdenní pilot prokázat?"
            ],
            "risks": ["Nerozšiřujte pilot, dokud nejsou dohodnuta kritéria úspěchu."],
            "gaps": ["Všechna jména, schůzky i souvislosti v tomto náhledu jsou smyšlené."]
        }
    });
    let mut value = crate::worker::validate_prepared(&raw.to_string(), &event.fingerprint())?;
    value["synthetic"] = json!(true);
    value["evidence_status"] = json!("Sample Insights - not generated by AI");
    Ok(value)
}

// Start minute, duration, title, category, location, response, sample Insights saved.
type Extra = (u32, i64, &'static str, &'static str, &'static str, &'static str, bool);

/// Extra fictional meeting on the demo's weekday today: it overlaps the workweek agenda and has
/// no sample Insights, so My day shows a conflict and a meeting to prepare.
const TODAY_WEEKDAY: &[Extra] = &[(
    600,
    45,
    "Northstar / Escalation check-in",
    "Customer",
    "Online",
    "accepted",
    false,
)];

/// A short fictional agenda when the demo runs on a weekend, so My day is never empty.
const TODAY_WEEKEND: &[Extra] = &[
    (540, 45, "Launch readiness check", "Office", "Online", "accepted", true),
    (
        600,
        45,
        "Northstar / Escalation check-in",
        "Customer",
        "Online",
        "accepted",
        false,
    ),
    (630, 60, "Focus / Launch checklist", "Focus", "", "organizer", true),
    (
        780,
        30,
        "Partner office hours",
        "Video calls",
        "Online",
        "tentativelyAccepted",
        true,
    ),
    (
        900,
        45,
        "Orbit / Release review",
        "Customer",
        "Online",
        "accepted",
        false,
    ),
];

fn today_events(today: chrono::NaiveDate) -> Vec<Event> {
    let extras = if today.weekday().num_days_from_monday() < 5 {
        TODAY_WEEKDAY
    } else {
        TODAY_WEEKEND
    };
    extras
        .iter()
        .enumerate()
        .map(
            |(index, (minute, duration, title, category, location, status, ready))| {
                let start = today
                    .and_hms_opt(minute / 60, minute % 60, 0)
                    .expect("valid demo time")
                    .and_local_timezone(Local)
                    .earliest()
                    .expect("demo daytime exists")
                    .fixed_offset();
                Event {
                    id: format!("synthetic-today-{index}"),
                    title: (*title).into(),
                    start,
                    end: start + Duration::minutes(*duration),
                    category: (*category).into(),
                    attendees: if *status == "organizer" {
                        Vec::new()
                    } else {
                        [
                            "Alex Rivers <alex@example.invalid>",
                            "Jamie Brooks <jamie@example.invalid>",
                            "Riley Chen <riley@example.invalid>",
                        ]
                        .into_iter()
                        .chain(customer_contact(title))
                        .map(Into::into)
                        .collect()
                    },
                    organizer: "Taylor Quinn <taylor@example.invalid>".into(),
                    location: (*location).into(),
                    status: (*status).into(),
                    briefing_status: if *ready { "ready" } else { "not_ready" }.into(),
                    is_all_day: false,
                    source_url: None,
                    extra: BTreeMap::from([
                        ("synthetic".into(), json!(true)),
                        ("type".into(), json!("singleInstance")),
                        ("isOrganizer".into(), json!(*status == "organizer")),
                        ("categories".into(), json!([category])),
                    ])
                    .into_iter()
                    .chain(description(title).map(|body| ("body".into(), body)))
                    .collect(),
                }
            },
        )
        .collect()
}

/// The offline preview calendar: `count` meetings. Today gets a few extra fictional meetings
/// (see [`TODAY_WEEKDAY`] and [`TODAY_WEEKEND`]) and the last meeting is the fictional
/// [`ENDED_MEETING`] that ended yesterday and has sample notes, tasks and a follow-up e-mail.
pub fn preview_events(now: DateTime<Local>, count: usize) -> Vec<Event> {
    let today = if count > 40 {
        today_events(now.date_naive())
    } else {
        Vec::new()
    };
    let mut events = events(now, count.saturating_sub(1 + today.len()));
    events.extend(today);
    if count > 0 {
        let start = (now.date_naive() - Duration::days(1))
            .and_hms_opt(14, 0, 0)
            .expect("valid demo time")
            .and_local_timezone(Local)
            .earliest()
            .expect("demo daytime exists")
            .fixed_offset();
        events.push(Event {
            id: format!("synthetic-{}", count - 1),
            title: ENDED_MEETING.into(),
            start,
            end: start + Duration::minutes(60),
            category: "Customer".into(),
            attendees: vec![
                "Alex Rivers <alex@example.invalid>".into(),
                "Jamie Brooks <jamie@example.invalid>".into(),
                "Morgan Lee <morgan@example.invalid>".into(),
                "Riley Chen <riley@example.invalid>".into(),
            ],
            organizer: "Taylor Quinn <taylor@example.invalid>".into(),
            location: "Online".into(),
            status: "accepted".into(),
            briefing_status: "ready".into(),
            is_all_day: false,
            source_url: None,
            extra: BTreeMap::from([
                ("synthetic".into(), json!(true)),
                ("type".into(), json!("singleInstance")),
                ("isOrganizer".into(), json!(false)),
                ("categories".into(), json!(["Customer"])),
                // A fictional Teams link of an ended meeting; Join is never offered for it.
                ("onlineMeeting".into(), json!({"joinUrl": "https://example.invalid/teams/pilot-planning"})),
            ])
            .into_iter()
            .chain(description(ENDED_MEETING).map(|body| ("body".into(), body)))
            .collect(),
        });
    }
    events
}

/// The saved document a preview meeting shows: sample notes for the ended meeting, nothing for a
/// meeting without sample Insights, otherwise sample Insights.
pub fn detail(event: &Event) -> Result<Option<Value>, String> {
    if event.title == ENDED_MEETING {
        notes(event).map(Some)
    } else if event.briefing_status != "ready" {
        Ok(None)
    } else {
        insights(event).map(Some)
    }
}

/// Fictional bilingual post-meeting notes of [`ENDED_MEETING`]; the first personal task contains
/// an explicit date so the import shows a due date.
pub fn notes(event: &Event) -> Result<Value, String> {
    if event.extra.get("synthetic") != Some(&json!(true)) || event.title != ENDED_MEETING {
        return Err("Demo notes require the explicitly synthetic ended meeting".into());
    }
    let due = event.start.date_naive() + Duration::days(3);
    let raw = json!({
        "summary": "Northstar agreed to a two-week pilot of the intake workflow. The pilot owner and the second-phase budget still need confirmation.",
        "preparation": [
            format!("My task: send the one-page architecture comparison to Alex by {}", due.format("%Y-%m-%d")),
            "My task: confirm who owns the pilot on the Northstar side",
            "My task: share the prototype recording with the team"
        ],
        "talking_points": [
            "Shared material: pilot plan draft (fictional link) https://example.com/northstar/pilot-plan",
            "Takeaway: start with the intake workflow; reporting comes in a later phase"
        ],
        "open_questions": ["Mention of me: Jamie asked you to walk through the trade-offs again at the pilot review"],
        "risks": [
            "Decision: the pilot runs for two weeks with three success criteria",
            "Unresolved: the budget for the second phase is not approved yet"
        ],
        "gaps": ["All names, meetings and context in this preview are fictional."],
        "sources": [{"title": "Pilot plan draft (fictional)", "url": "https://example.com/northstar/pilot-plan"}],
        "cs": {
            "summary": "Northstar souhlasil s dvoutýdenním pilotem příjmu požadavků. Vlastníka pilotu a rozpočet druhé fáze je ještě potřeba potvrdit.",
            "preparation": [
                format!("Můj úkol: poslat Alexovi jednostránkové porovnání architektury do {}. {}. {}", due.day(), due.month(), due.year()),
                "Můj úkol: ověřit, kdo za Northstar převezme odpovědnost za pilot",
                "Můj úkol: nasdílet týmu záznam ukázky prototypu"
            ],
            "talking_points": [
                "Sdílený materiál: návrh plánu pilotu (smyšlený odkaz) https://example.com/northstar/pilot-plan",
                "Poznatek: začít příjmem požadavků; reporting přijde v pozdější fázi"
            ],
            "open_questions": ["Zmínka o vás: Jamie vás požádal, abyste na revizi pilotu znovu prošli kompromisy"],
            "risks": [
                "Rozhodnutí: pilot poběží dva týdny se třemi kritérii úspěchu",
                "Otevřené: rozpočet druhé fáze zatím není schválený"
            ],
            "gaps": ["Všechna jména, schůzky i souvislosti v tomto náhledu jsou smyšlené."]
        }
    });
    let mut value = crate::worker::validate_prepared(&raw.to_string(), &event.fingerprint())?;
    value["tomlook_recap"] = json!(true);
    value["synthetic"] = json!(true);
    value["evidence_status"] = json!("Sample meeting notes - not generated by AI");
    Ok(value)
}

/// Fictional manual tasks of the preview: overdue, due today, upcoming and one completed task.
pub fn sample_tasks(
    account: &str,
    event: &Event,
    language: crate::i18n::Language,
    now: DateTime<Utc>,
) -> Result<Vec<crate::tasks::Task>, String> {
    use crate::{i18n::Language, tasks::New};
    let today = Local::now().date_naive();
    let samples: [(&str, &str, Option<i64>, bool, bool); 4] = [
        ("Review the Cedar solution proposal", "Projít návrh řešení pro Cedar", Some(-1), false, false),
        ("Book the room for the Orbit hands-on lab", "Zarezervovat místnost na praktický workshop Orbit", Some(0), false, false),
        ("Prepare the pilot success criteria", "Připravit kritéria úspěchu pilotu", Some(4), true, false),
        ("Send the workshop agenda", "Poslat agendu workshopu", None, false, true),
    ];
    samples
        .into_iter()
        .map(|(english, czech, due, linked, done)| {
            let title = if language == Language::Cs { czech } else { english };
            let mut new = New::manual(title, due.map(|days| today + Duration::days(days)), language)?;
            if linked {
                new.meeting = Some(crate::tasks::Meeting::new(account, event));
            }
            let task = new.task(account, now)?;
            Ok(if done { task.completed(true, now) } else { task })
        })
        .collect()
}

/// A fictional saved follow-up e-mail of [`ENDED_MEETING`]; it was never generated by AI.
pub fn sample_followup(
    account: &str,
    event: &Event,
    language: crate::i18n::Language,
    now: DateTime<Utc>,
) -> Result<crate::followup::Saved, String> {
    let request = crate::followup::Request::new(0, account, event, &notes(event)?, language, &[], None)?;
    let (subject, body) = match language {
        crate::i18n::Language::En => (
            "Follow-up: Northstar pilot planning",
            "Hi all,\n\nthank you for today's pilot planning. A short summary:\n\n- We start a two-week pilot of the intake workflow with three success criteria.\n- Reporting comes in a later phase.\n- The budget for the second phase is not approved yet [to confirm].\n\nNext steps:\n- I will send the one-page architecture comparison.\n- I will confirm who owns the pilot on the Northstar side.\n- I will share the prototype recording.\n\nPilot plan draft: https://example.com/northstar/pilot-plan\n\nBest regards",
        ),
        crate::i18n::Language::Cs => (
            "Shrnutí: plánování pilotu Northstar",
            "Dobrý den všem,\n\nděkuji za dnešní plánování pilotu. Krátké shrnutí:\n\n- Spouštíme dvoutýdenní pilot příjmu požadavků se třemi kritérii úspěchu.\n- Reporting přijde v pozdější fázi.\n- Rozpočet druhé fáze zatím není schválený [ověřit].\n\nDalší kroky:\n- Pošlu jednostránkové porovnání architektury.\n- Ověřím, kdo za Northstar převezme odpovědnost za pilot.\n- Nasdílím záznam ukázky prototypu.\n\nNávrh plánu pilotu: https://example.com/northstar/pilot-plan\n\nS pozdravem",
        ),
    };
    let saved = crate::followup::Saved::new(
        &request,
        crate::followup::Draft {
            subject: subject.into(),
            body: body.into(),
        },
        now,
    );
    saved.validate()?;
    Ok(saved)
}

pub fn save_screenshot(
    path: &std::path::Path,
    image: &eframe::egui::ColorImage,
) -> Result<(), String> {
    use image::ImageEncoder;
    use std::io::Write;
    let width = u32::try_from(image.width()).map_err(|error| error.to_string())?;
    let height = u32::try_from(image.height()).map_err(|error| error.to_string())?;
    let pixels: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("Encode demo screenshot: {error}"))?;
    let parent = path
        .parent()
        .ok_or("Demo screenshot needs a parent directory")?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Create demo screenshot directory: {error}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Create demo screenshot: {error}"))?;
    file.write_all(&png)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Persist demo screenshot: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_export_is_a_real_png_and_never_overwrites_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("demo.png");
        let image =
            eframe::egui::ColorImage::filled([8, 4], eframe::egui::Color32::from_rgb(20, 30, 40));
        save_screenshot(&path, &image).unwrap();
        let saved = image::open(&path).unwrap();
        assert_eq!((saved.width(), saved.height()), (8, 4));
        let original = std::fs::read(&path).unwrap();
        assert!(save_screenshot(&path, &image).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn preview_is_a_full_colored_workweek_with_fictional_contacts_and_quiet_invites() {
        let events = events(Local::now(), 200);
        assert_eq!(events.len(), 200);
        let calendar =
            crate::calendar::Calendar::build(events, Default::default(), vec![]).unwrap();
        assert_eq!(calendar.categories.len(), 6);
        for day in 0..5 {
            let date = crate::calendar::monday(Local::now().date_naive()) + Duration::days(day);
            assert!(calendar.days[&date].events.len() >= 6);
        }
        assert!(calendar.events.iter().any(Event::unconfirmed));
        assert!(calendar.events.iter().any(|event| event.is_all_day));
        assert!(calendar.events.iter().all(|event| {
            event.validate().is_ok()
                && event.extra["synthetic"] == true
                && event.source_url.is_none()
                && event.organizer.ends_with("@example.invalid>")
                && event
                    .attendees
                    .iter()
                    .all(|name| name.ends_with(".invalid>"))
        }));
    }

    #[test]
    fn sample_feedback_is_fictional_valid_and_summarizes_for_the_demo_account() {
        let now = Utc::now();
        let entries = sample_feedback(&events(Local::now(), 200), &Default::default(), now).unwrap();
        assert!(entries.iter().all(|entry| entry.validate().is_ok()
            && entry.key.document.account == crate::feedback::PREVIEW_ACCOUNT
            && entry.event_id.starts_with("synthetic-")));
        let summary = crate::feedback::summarize(crate::feedback::PREVIEW_ACCOUNT, &entries, now);
        assert_eq!((summary.preparation.up, summary.preparation.down), (3, 4));
        assert_eq!(summary.preparation.hot_spots[0], ("talking_points".into(), 2));
        assert_eq!(summary.preparation.reasons[0], (crate::feedback::Reason::TooVague, 2));
        for (value, scene) in [("details", Scene::Details), ("feedback", Scene::Feedback), ("prompts", Scene::Prompts),
            ("tasks", Scene::Tasks), ("followup", Scene::FollowUp), ("myday", Scene::MyDay),
            ("notifications", Scene::Notifications), ("weekly", Scene::Weekly)] {
            assert_eq!(Scene::parse(value).unwrap(), scene);
        }
        assert!(Scene::parse("settings").is_err());
        assert_eq!(Scene::FollowUp.meeting(), ENDED_MEETING);
        assert!(Scene::Details.selects_meeting() && !Scene::MyDay.selects_meeting() && !Scene::Notifications.selects_meeting());
        assert!(!Scene::Weekly.selects_meeting());
    }

    #[test]
    fn preview_has_an_ended_meeting_with_fictional_notes_tasks_and_a_follow_up() {
        use crate::i18n::Language;
        let now = Local::now();
        let events = preview_events(now, 200);
        assert_eq!(events.len(), 200);
        let ended = events.iter().find(|event| event.title == ENDED_MEETING).unwrap();
        assert!(ended.end < now.fixed_offset() && ended.end > now.fixed_offset() - Duration::days(7));
        assert!(ended.attendees.iter().chain([&ended.organizer]).all(|person| person.ends_with("@example.invalid>")));
        crate::calendar::Calendar::build(events.clone(), Default::default(), vec![]).unwrap();
        let saved = detail(ended).unwrap().unwrap();
        assert_eq!(saved["tomlook_recap"], true);
        assert_eq!(saved["evidence_status"], "Sample meeting notes - not generated by AI");
        assert!(detail(events.iter().find(|event| event.title == "Weekly planning").unwrap()).unwrap().unwrap()["tomlook_recap"].is_null());
        let unprepared = events.iter().find(|event| event.title == "Northstar / Escalation check-in").unwrap();
        assert_eq!(detail(unprepared).unwrap(), None, "A meeting without sample Insights shows none");
        for language in [Language::En, Language::Cs] {
            let imported = crate::tasks::from_notes(crate::feedback::PREVIEW_ACCOUNT, ended, &saved, language, Utc::now()).unwrap();
            assert_eq!(imported.len(), 3);
            assert_eq!(imported[0].due, Some(ended.start.date_naive() + Duration::days(3)), "The explicit date is parsed");
            assert!(imported[1..].iter().all(|task| task.due.is_none()));
            let samples = sample_tasks(crate::feedback::PREVIEW_ACCOUNT, ended, language, Utc::now()).unwrap();
            assert_eq!(samples.iter().filter(|task| task.status == crate::tasks::Status::Done).count(), 1);
            let followup = sample_followup(crate::feedback::PREVIEW_ACCOUNT, ended, language, Utc::now()).unwrap();
            assert_eq!(followup.language, language);
            assert_eq!(followup.recipients.len(), 5);
            assert!(followup.mailto().unwrap().uri.starts_with("mailto:taylor@example.invalid,alex@example.invalid"));
        }
        let mut real = ended.clone();
        real.extra.remove("synthetic");
        assert!(notes(&real).is_err());
    }

    #[test]
    fn demo_today_has_a_meaningful_my_day_on_weekdays_and_weekends() {
        use chrono::TimeZone;
        use std::sync::Arc;
        for (day, meetings) in [(11, 6), (15, 4)] {
            let now = Local.with_ymd_and_hms(2026, 11, day, 7, 30, 0).earliest().unwrap();
            let events = preview_events(now, 200);
            assert_eq!(events.len(), 200);
            let ended = events.iter().find(|event| event.title == ENDED_MEETING).unwrap().clone();
            let coverage = (0..42).map(|offset| crate::calendar::monday(now.date_naive()) + Duration::days(offset)).collect();
            let calendar = crate::calendar::Calendar::build(events, coverage, vec![]).unwrap();
            let ready = calendar.events.iter().filter(|event| event.briefing_status == "ready").map(|event| event.id.clone()).collect();
            let notes = [(ended.id.clone(), crate::myday::Notes::from_payload(&notes(&ended).unwrap()).unwrap())].into();
            let sources = crate::myday::Sources {
                calendar: Arc::new(calendar),
                ready: Arc::new(ready),
                notes: Arc::new(notes),
                ..Default::default()
            };
            let overview = crate::myday::overview(&sources, now, crate::i18n::Language::En);
            assert!(overview.verified);
            assert_eq!(overview.meetings, meetings, "{day}: {:?}", overview.agenda);
            assert!(!overview.conflicts.is_empty(), "{day}");
            assert!(overview.agenda.iter().any(|item| item.title == "Northstar / Escalation check-in" && item.unprepared()));
            assert!(overview.focus.as_ref().is_some_and(|focus| !focus.is_empty()));
            assert_eq!(overview.mentions.len(), 1, "Yesterday's sample notes mention me");
        }
    }

    #[test]
    fn demo_week_has_categories_a_conflict_meetings_without_agenda_customers_and_a_trend() {
        use chrono::TimeZone;
        use std::sync::Arc;
        for day in [11, 15] {
            let now = Local.with_ymd_and_hms(2026, 11, day, 16, 0, 0).earliest().unwrap();
            let events = preview_events(now, 200);
            assert_eq!(events.len(), 200);
            let monday = crate::calendar::monday(now.date_naive());
            let coverage = (-14..42).map(|offset| monday + Duration::days(offset)).collect();
            let calendar = crate::calendar::Calendar::build(events, coverage, vec![]).unwrap();
            let ended = calendar.events.iter().find(|event| event.title == ENDED_MEETING).unwrap();
            assert!(ended.join_url().is_some() && ended.end < now.fixed_offset());
            let sources = crate::weekly::Sources {
                own_domains: ["example.invalid".to_owned()].into(),
                ready: Arc::new(calendar.events.iter().filter(|event| event.briefing_status == "ready").map(|event| event.id.clone()).collect()),
                notes: Arc::new([ended.id.clone()].into()),
                calendar: Arc::new(calendar),
                ..Default::default()
            };
            let review = crate::weekly::review(&sources, monday, now);
            assert!(review.coverage.sufficient() && !review.coverage.partial(), "{day}");
            assert!(review.headline.meetings > 20 && review.headline.focus_minutes.is_some_and(|minutes| minutes > 0), "{day}");
            assert!(review.categories.len() >= 5, "{day}: {:?}", review.categories);
            assert!(!review.conflicts.is_empty(), "{day}: today's extra meeting overlaps");
            assert!(review.agenda.organized().count() >= 1 && review.agenda.others().count() >= 3, "{day}: {:?}", review.agenda);
            assert!(review.agenda.missing.iter().any(|item| item.title == "Coffee & roadmap"));
            assert!(review.agenda.unknown > 0 && review.agenda.checked > review.agenda.missing.len());
            assert_eq!(
                review.companies.iter().map(|company| company.domain.as_str()).collect::<BTreeSet<_>>(),
                ["cedar.invalid", "northstar.invalid", "orbit.invalid"].into(),
                "Customers are external companies; the demo's colleagues are not"
            );
            assert!(review.people.iter().any(|person| person.name == "Alex Rivers"));
            assert_eq!((review.preparation.teams_ended, review.preparation.notes), (1, 1));
            assert!(review.load.short > 0 && review.load.long > 0);
            let trend = crate::weekly::trend(&sources, monday).unwrap();
            assert_eq!(trend.weeks, [monday - Duration::days(7), monday - Duration::days(14)]);
            assert!(trend.meetings > 20.0);
        }
    }

    #[test]
    fn bilingual_sample_insights_are_valid_and_never_claim_retrieved_evidence() {
        for event in events(Local::now(), MEETINGS.len()) {
            let value = insights(&event).unwrap();
            assert_eq!(value["synthetic"], true);
            assert_eq!(value["sources"], json!([]));
            assert_eq!(
                value["evidence_status"],
                "Sample Insights - not generated by AI"
            );
            assert!(!value["cs"]["summary"].as_str().unwrap().is_empty());
            let mut real = event;
            real.extra.remove("synthetic");
            assert!(insights(&real).is_err());
        }
    }
}
