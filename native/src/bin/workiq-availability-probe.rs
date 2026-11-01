//! Read-only live check of the new-event and Find a time lookups with the saved Tomlook WorkIQ
//! sign-in. It never signs in interactively and never writes: it reads the event create schema
//! (`get_schema`), the Teams chat and chat message create schemas (`get_schema` for `/chats` and
//! `/chats/{chat-id}/messages`), your own free/busy for the next workday and for the next five
//! workdays (`getSchedule`) and meeting-time suggestions for your own mailbox only
//! (`findMeetingTimes`). The report contains counts and schema shapes, not calendar text. No chat
//! is created, no message is sent and no directory lookup of other people is made.
use std::{path::PathBuf, time::Duration};
use tomlook::{
    attendees::{Attendee, Role},
    availability::{MeetingTimesRequest, ScheduleRequest},
    workiq::connection::{Connection, calendar_write, chat_write},
};

#[tokio::main]
async fn main() {
    let result = async {
        let mut args = std::env::args_os().skip(1);
        let root = std::path::absolute(PathBuf::from(
            args.next().ok_or("Usage: workiq-availability-probe <Tomlook-state-directory> [work-account]")?,
        ))
        .map_err(|error| format!("Resolve state directory: {error}"))?;
        let account = match args.next() {
            Some(account) => account.into_string().map_err(|_| "Invalid work account")?,
            None => tomlook::storage::Store::read_only(&root)?
                .calendar_owner()?
                .ok_or("No saved calendar account; pass the work account explicitly")?,
        };
        if args.next().is_some() {
            return Err("Unexpected arguments; no credential or runtime options are accepted".to_owned());
        }
        let connection = Connection::connect_saved(&root, &account, false, |_| {}).await?;
        let own = connection
            .identity
            .mail
            .clone()
            .unwrap_or_else(|| connection.account.clone());
        let today = chrono::Local::now().date_naive();
        let mut workday = today + chrono::Duration::days(1);
        while chrono::Datelike::weekday(&workday).num_days_from_monday() >= 5 {
            workday += chrono::Duration::days(1);
        }
        let capability = calendar_write::invitation_capability(&connection).await;
        let attendee_schema = calendar_write::invitation_schema_shape(&connection).await;
        let schedule = match ScheduleRequest::day(workday, vec![own.clone()]) {
            Ok(request) => connection.schedule(&request).await,
            Err(error) => Err(error),
        };
        let times_request = Attendee::new(&own, "", Role::Required).and_then(|attendee| {
            MeetingTimesRequest::around(workday + chrono::Duration::days(1), 30, vec![attendee], chrono::Utc::now())
        });
        let times = match &times_request {
            Ok(request) => connection.meeting_times(request).await,
            Err(error) => Err(error.clone()),
        };
        let slots = times_request.as_ref().map(|request| request.slots.clone()).unwrap_or_default();
        let chat_capability = chat_write::capability(&connection).await;
        let chat_schemas = chat_write::schema_shapes(&connection).await;
        let search = tomlook::scheduling::Search::new(today, 30);
        let range = match search.schedule_request(vec![own.clone()]) {
            Ok(request) => connection.schedule(&request).await,
            Err(error) => Err(error),
        };
        let closed = connection.close().await;
        let report = serde_json::json!({
            "chat_capability": match &chat_capability {
                Ok(()) => serde_json::json!("compatible"),
                Err(error) => serde_json::json!({"error": error}),
            },
            "chat_schemas": chat_schemas,
            "own_find_a_time_schedule": match &range {
                Ok(availability) => {
                    let row = &availability.schedules[0];
                    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
                    for status in &row.view {
                        *counts.entry(status.label()).or_default() += 1;
                    }
                    serde_json::json!({
                        "searched_days": search.dates().len(),
                        "hours": (availability.end - availability.start).num_hours(),
                        "cells": row.view.len(),
                        "status_counts": counts,
                        "error": row.error,
                    })
                }
                Err(error) => serde_json::json!({"error": error}),
            },
            "invitation_capability": match &capability {
                Ok(()) => serde_json::json!("compatible"),
                Err(error) => serde_json::json!({"error": error}),
            },
            "attendee_schema_shape": match &attendee_schema {
                Ok(shape) => shape.clone(),
                Err(error) => serde_json::json!({"error": error}),
            },
            "own_schedule": match &schedule {
                Ok(availability) => {
                    let row = &availability.schedules[0];
                    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
                    for status in &row.view {
                        *counts.entry(status.label()).or_default() += 1;
                    }
                    serde_json::json!({
                        "cells": row.view.len(),
                        "interval_minutes": availability.interval,
                        "status_counts": counts,
                        "items": row.items.len(),
                        "items_with_visible_subject": row.items.iter().filter(|item| item.subject.is_some()).count(),
                        "error": row.error,
                    })
                }
                Err(error) => serde_json::json!({"error": error}),
            },
            "own_meeting_times": match &times {
                Ok(times) => serde_json::json!({
                    "suggestions": times.suggestions.len(),
                    "suggestions_with_confidence": times.suggestions.iter().filter(|suggestion| suggestion.confidence.is_some()).count(),
                    "organizer_statuses": times.suggestions.iter().map(|suggestion| suggestion.organizer.label()).collect::<Vec<_>>(),
                    "all_within_requested_slots": times.suggestions.iter().all(|suggestion| slots.iter()
                        .any(|(start, end)| suggestion.start >= *start && suggestion.end <= *end)),
                    "all_on_quarter_hours": times.suggestions.iter().all(|suggestion| {
                        use chrono::Timelike;
                        [suggestion.start, suggestion.end].iter().all(|time| {
                            let local = time.with_timezone(&chrono::Local);
                            local.minute() % 15 == 0 && local.second() == 0
                        })
                    }),
                    "empty_reason": times.empty_reason,
                    "reason_code": times.reason_code,
                }),
                Err(error) => serde_json::json!({"error": error}),
            },
            "calendar_writes": 0,
            "invitations_sent": 0,
            "chats_created": 0,
            "messages_sent": 0,
            "model_calls": 0,
        });
        closed?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|_| "Encode availability report failed")?
        );
        Ok::<(), String>(())
    };
    match tokio::time::timeout(Duration::from_secs(180), result).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("WorkIQ availability probe: {error}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("WorkIQ availability probe exceeded its deadline");
            std::process::exit(1);
        }
    }
}
