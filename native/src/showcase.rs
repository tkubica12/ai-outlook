use crate::calendar::Event;
use chrono::{DateTime, Datelike, Duration, Local, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
}

impl Scene {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "details" => Ok(Self::Details),
            "feedback" => Ok(Self::Feedback),
            "prompts" => Ok(Self::Prompts),
            "tasks" => Ok(Self::Tasks),
            "followup" => Ok(Self::FollowUp),
            _ => Err("--demo-scene must be details, feedback, prompts, tasks or followup".into()),
        }
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
        "accepted",
    ),
];

pub fn events(now: DateTime<Local>, count: usize) -> Vec<Event> {
    let monday = crate::calendar::monday(now.date_naive());
    (0..count)
        .map(|index| {
            let (day, minute, duration, title, category, location, status) =
                MEETINGS[index % MEETINGS.len()];
            let date = monday + Duration::days(day + ((index / MEETINGS.len()) % 6) as i64 * 7);
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
                attendees: if status == "organizer" || minute == 0 {
                    Vec::new()
                } else {
                    vec![
                        "Alex Rivers <alex@example.invalid>".into(),
                        "Jamie Brooks <jamie@example.invalid>".into(),
                        "Morgan Lee <morgan@example.invalid>".into(),
                    ]
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
                ]),
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

/// The offline preview calendar: `count` meetings, the last of which is the fictional
/// [`ENDED_MEETING`] that ended yesterday and has sample notes, tasks and a follow-up e-mail.
pub fn preview_events(now: DateTime<Local>, count: usize) -> Vec<Event> {
    let mut events = events(now, count.saturating_sub(1));
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
            ]),
        });
    }
    events
}

/// The saved document a preview meeting shows: sample notes for the ended meeting, otherwise
/// sample Insights.
pub fn detail(event: &Event) -> Result<Value, String> {
    if event.title == ENDED_MEETING {
        notes(event)
    } else {
        insights(event)
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
                    .all(|name| name.ends_with("@example.invalid>"))
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
            ("tasks", Scene::Tasks), ("followup", Scene::FollowUp)] {
            assert_eq!(Scene::parse(value).unwrap(), scene);
        }
        assert!(Scene::parse("settings").is_err());
        assert_eq!(Scene::FollowUp.meeting(), ENDED_MEETING);
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
        let saved = detail(ended).unwrap();
        assert_eq!(saved["tomlook_recap"], true);
        assert_eq!(saved["evidence_status"], "Sample meeting notes - not generated by AI");
        assert!(detail(events.iter().find(|event| event.title == "Weekly planning").unwrap()).unwrap()["tomlook_recap"].is_null());
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
