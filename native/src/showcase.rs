use crate::calendar::Event;
use chrono::{DateTime, Duration, Local};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
