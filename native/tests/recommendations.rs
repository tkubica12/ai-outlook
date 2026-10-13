use chrono::{Duration, Local, TimeZone};
use std::collections::BTreeMap;
use tomlook::{
    calendar::{self, Calendar},
    recommendations::{self, Change, Kind, Operation, Recommendation, State, Suggestion},
    storage::Store,
};

fn fixture() -> (Calendar, Recommendation) {
    let now = Local::now().fixed_offset();
    let mut event = calendar::fixture(1).remove(0);
    event.start = now + Duration::hours(2);
    event.end = event.start + Duration::hours(1);
    event.is_all_day = false;
    let suggestion = Suggestion {
        kind: Kind::Category,
        reason: "Office visit.".into(),
        reason_cs: "Navsteva kancelare.".into(),
        category: Some("Office".into()),
        minutes: None,
    };
    let recommendation = Recommendation::from_insights(&event, "owner", suggestion, now).unwrap();
    let day = event.start.with_timezone(&Local).date_naive();
    (
        Calendar::build(vec![event], [day].into(), Vec::new()).unwrap(),
        recommendation,
    )
}

#[test]
fn reciprocal_conflicts_keep_distinct_edges_and_skips_survive_restart_and_metadata_changes() {
    let (calendar, record) = fixture();
    let now = Local::now().fixed_offset();
    let mut first = calendar.events[0].clone();
    first.id = "a".into();
    first.end = first.start + Duration::hours(2);
    let mut second = first.clone();
    second.id = "b".into();
    second.start = first.start + Duration::hours(1);
    second.end = second.start + Duration::hours(2);
    let mut third = first.clone();
    third.id = "c".into();
    third.start = first.start + Duration::minutes(150);
    third.end = third.start + Duration::hours(1);
    let calendar = Calendar::build(
        vec![first.clone(), second.clone(), third.clone()],
        Default::default(),
        vec![],
    )
    .unwrap();
    let suggestion = Suggestion {
        kind: Kind::PriorityConflict,
        category: None,
        ..record.suggestion
    };
    let mut inbox = calendar
        .events
        .iter()
        .map(|event| {
            let record =
                Recommendation::from_insights(event, "owner", suggestion.clone(), now).unwrap();
            (record.id.clone(), record)
        })
        .collect::<BTreeMap<_, _>>();
    let view =
        recommendations::inbox_view(&calendar, &inbox, Some("owner"), now, &Default::default());
    assert_eq!(view.len(), 2);
    assert_eq!(view[0].conflicts, ["b"]);
    assert_eq!(view[1].conflicts, ["c"]);
    assert!(
        recommendations::inbox_view(&calendar, &inbox, Some("other"), now, &Default::default())
            .is_empty()
    );
    let skipped = &view[1];
    let key = recommendations::conflict_dismissal_key("owner", "b", "c");
    assert_eq!(
        key,
        recommendations::conflict_dismissal_key("owner", "c", "b")
    );
    assert_ne!(
        key,
        recommendations::conflict_dismissal_key("other", "b", "c")
    );
    inbox.get_mut(&skipped.id).unwrap().state = State::Dismissed;
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    store
        .save_recommendations_with_dismissals(&inbox, &[key.clone()].into())
        .unwrap();
    assert!(
        store
            .save_recommendations_with_dismissals(&inbox, &["invalid".into()].into())
            .is_err()
    );
    drop(store);
    let store = Store::open(root.path()).unwrap();
    let keys = store.recommendation_dismissals().unwrap();
    let inbox = store.recommendations().unwrap();
    let view = recommendations::inbox_view(&calendar, &inbox, Some("owner"), now, &keys);
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].conflicts, ["b"], "Skipping B/C must not hide A/B");
    second.title.push_str(" changed");
    let changed = Calendar::build(
        vec![first.clone(), second.clone(), third.clone()],
        Default::default(),
        vec![],
    )
    .unwrap();
    let regenerated = changed
        .events
        .iter()
        .map(|event| {
            let record =
                Recommendation::from_insights(event, "owner", suggestion.clone(), now).unwrap();
            (record.id.clone(), record)
        })
        .collect();
    let view = recommendations::inbox_view(&changed, &regenerated, Some("owner"), now, &keys);
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].conflicts, ["b"]);
    second.status = "tentativelyAccepted".into();
    let resolved = Calendar::build(vec![first, second, third], Default::default(), vec![]).unwrap();
    assert!(
        recommendations::inbox_view(&resolved, &regenerated, Some("owner"), now, &keys).is_empty()
    );
}

#[test]
fn conflict_dedup_preserves_triangle_edges_and_unknown_write_outcomes() {
    let (calendar, record) = fixture();
    let now = Local::now().fixed_offset();
    let events = ["a", "b", "c"]
        .into_iter()
        .map(|id| {
            let mut event = calendar.events[0].clone();
            event.id = id.into();
            event
        })
        .collect();
    let calendar = Calendar::build(events, Default::default(), vec![]).unwrap();
    let mut inbox = calendar
        .events
        .iter()
        .map(|event| {
            let record = Recommendation::from_insights(
                event,
                "owner",
                Suggestion {
                    kind: Kind::PriorityConflict,
                    category: None,
                    ..record.suggestion.clone()
                },
                now,
            )
            .unwrap();
            (record.id.clone(), record)
        })
        .collect::<BTreeMap<_, _>>();
    let view =
        recommendations::inbox_view(&calendar, &inbox, Some("owner"), now, &Default::default());
    assert_eq!(
        view.iter()
            .map(|entry| entry.conflicts.len())
            .sum::<usize>(),
        3
    );
    assert_eq!(view.len(), 2);
    let legacy = inbox
        .values_mut()
        .find(|record| record.event_id == "a")
        .unwrap();
    legacy.state = State::Dismissed;
    let view =
        recommendations::inbox_view(&calendar, &inbox, Some("owner"), now, &Default::default());
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].conflicts, ["c"]);
    let review = inbox
        .values_mut()
        .find(|record| record.event_id == "c")
        .unwrap();
    review.state = State::Review;
    let review_id = review.id.clone();
    let keys = calendar
        .events
        .iter()
        .flat_map(|event| {
            recommendations::conflicts(&calendar, event)
                .into_iter()
                .map(|other| recommendations::conflict_dismissal_key("owner", &event.id, &other.id))
        })
        .collect();
    assert!(
        recommendations::inbox_view(&calendar, &inbox, Some("owner"), now, &keys)
            .iter()
            .any(|entry| entry.id == review_id)
    );
}

#[test]
fn material_contacts_use_invitation_identities_prioritize_home_domain_and_never_invent_ownership() {
    let (calendar, _) = fixture();
    let mut event = calendar.events[0].clone();
    event.organizer = "Agenda contact <organizer@partner.example>".into();
    event.attendees = vec![
        "External <external@partner.example>".into(),
        "Me <ME@home.example>".into(),
        "Colleague <colleague@home.example>".into(),
        "Duplicate <COLLEAGUE@home.example>".into(),
        "Not an email".into(),
        "Unsafe <unsafe@home.example/path>".into(),
    ];
    let contacts = recommendations::material_contacts(&event, "me@home.example", None);
    assert_eq!(contacts.len(), 3);
    assert_eq!(contacts[0].email, "organizer@partner.example");
    assert!(contacts[0].organizer);
    assert!(!contacts[0].same_domain);
    assert_eq!(contacts[1].email, "colleague@home.example");
    assert!(contacts[1].same_domain);
    assert!(!contacts[1].organizer);
    assert_eq!(contacts[2].email, "external@partner.example");
    event
        .attendees
        .extend((0..20).map(|index| format!("Person {index} <person{index}@home.example>")));
    assert_eq!(
        recommendations::material_contacts(&event, "me@home.example", None).len(),
        6
    );
    assert!(
        recommendations::material_contacts(&event, "", None)
            .iter()
            .all(|contact| !contact.same_domain)
    );
    let data = serde_json::to_value(contacts).unwrap();
    assert!(data[0].get("agenda_owner").is_none());
}

#[test]
fn material_contacts_exclude_the_verified_primary_address_across_restart_and_bind_it_to_the_owner()
{
    let (calendar, _) = fixture();
    let mut event = calendar.events[0].clone();
    event.organizer = "Organizer <organizer@home.example>".into();
    event.attendees = vec![
        "Me <LOGIN@tenant.example>".into(),
        "Primary Me <primary@home.example>".into(),
        "Colleague <colleague@home.example>".into(),
    ];
    let identity = tomlook::workiq::connection::Identity {
        account: "login@tenant.example".into(),
        mail: Some("primary@home.example".into()),
    };
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.workiq_identity().unwrap(), None);
    store.save_workiq_identity(&identity).unwrap();
    drop(store);
    let store = Store::open(root.path()).unwrap();
    let saved = store.workiq_identity().unwrap().unwrap();
    assert_eq!(saved, identity);
    let contacts = recommendations::material_contacts(&event, "LOGIN@tenant.example", Some(&saved));
    assert_eq!(contacts.len(), 2);
    assert!(contacts.iter().all(|contact| contact.same_domain));
    assert_eq!(contacts[1].email, "colleague@home.example");

    let mut other = saved.clone();
    other.account = "other@tenant.example".into();
    let contacts = recommendations::material_contacts(&event, &saved.account, Some(&other));
    assert_eq!(contacts.len(), 3);
    assert!(
        contacts
            .iter()
            .any(|contact| contact.email == "primary@home.example")
    );
    assert!(contacts.iter().all(|contact| !contact.same_domain));

    other.mail = Some("invalid@home.example/path".into());
    assert!(store.save_workiq_identity(&other).is_err());
    assert_eq!(store.workiq_identity().unwrap(), Some(saved.clone()));
    let no_mail = tomlook::workiq::connection::Identity {
        mail: None,
        ..saved
    };
    store.save_workiq_identity(&no_mail).unwrap();
    assert_eq!(store.workiq_identity().unwrap(), Some(no_mail));
}

#[test]
fn manual_actions_are_exact_one_shot_and_never_available_to_ai_suggestions() {
    use recommendations::MeetingResponse;
    let (mut calendar, _) = fixture();
    let event = &mut calendar.events[0];
    event
        .extra
        .insert("type".into(), serde_json::json!("occurrence"));
    event
        .extra
        .insert("isOrganizer".into(), serde_json::json!(false));
    event.organizer = "organizer@example.test".into();
    event.status = "notResponded".into();
    let event = event.clone();
    let now = Local::now().fixed_offset();
    let change = Change::Respond {
        response: MeetingResponse::Decline,
        comment: "I cannot join.".into(),
        send_response: true,
    };
    let mut record = Recommendation::from_manual(&event, "owner", change.clone(), now).unwrap();
    assert!(record.current(&calendar, Some("owner"), now).is_some());
    assert!(record.current(&calendar, Some("other"), now).is_none());
    assert!(record.suggestion.validate().is_err());
    assert!(
        Recommendation::from_insights(&event, "owner", record.suggestion.clone(), now).is_err()
    );
    let mut different_comment = change.clone();
    if let Change::Respond { comment, .. } = &mut different_comment {
        *comment = "Different comment.".into();
    }
    assert_ne!(
        record.id,
        Recommendation::from_manual(&event, "owner", different_comment, now)
            .unwrap()
            .id
    );
    assert_ne!(
        record.id,
        Recommendation::from_manual(&event, "other", change.clone(), now)
            .unwrap()
            .id
    );
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    store.save_workiq_calendar(&calendar, "owner").unwrap();
    store
        .save_recommendations(&BTreeMap::from([(record.id.clone(), record.clone())]))
        .unwrap();
    record.state = State::Dispatching;
    let operation = Operation {
        recommendation: record,
        event: event.clone(),
        change,
    };
    assert!(operation.validate("owner").is_ok());
    let mut altered = operation.clone();
    altered.change = Change::Respond {
        response: MeetingResponse::Accept,
        comment: String::new(),
        send_response: true,
    };
    assert!(altered.validate("owner").is_err());
    altered = operation.clone();
    altered.event.title = "Different meeting".into();
    assert!(altered.validate("owner").is_err());
    store.approve_recommendation(&operation).unwrap();
    assert!(store.approve_recommendation(&operation).is_err());
    drop(store);
    let mut store = Store::open(root.path()).unwrap();
    assert_eq!(
        store.recommendations().unwrap()[&operation.recommendation.id].state,
        State::Review
    );
    assert!(store.approve_recommendation(&operation).is_err());
    assert!(store.remove_calendar_event(&event.id, "other").is_err());
    assert!(
        store
            .remove_calendar_event(&event.id, "owner")
            .unwrap()
            .events
            .is_empty()
    );
    drop(store);
    assert!(
        Store::open(root.path())
            .unwrap()
            .calendar()
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn manual_roles_comments_and_recurrence_are_checked_before_approval() {
    use recommendations::{MeetingResponse, validate_manual_change};
    let (calendar, _) = fixture();
    let mut event = calendar.events[0].clone();
    let response = |response, comment: &str, send_response| Change::Respond {
        response,
        comment: comment.into(),
        send_response,
    };
    assert!(validate_manual_change(&event, &Change::ClearCategory).is_err());
    event
        .extra
        .insert("type".into(), serde_json::json!("seriesMaster"));
    assert!(validate_manual_change(&event, &Change::Category("Office".into())).is_err());
    event
        .extra
        .insert("type".into(), serde_json::json!("singleInstance"));
    event
        .extra
        .insert("isOrganizer".into(), serde_json::json!(false));
    event.organizer = "organizer@example.test".into();
    assert!(validate_manual_change(&event, &response(MeetingResponse::Accept, "", false)).is_ok());
    assert!(
        validate_manual_change(
            &event,
            &response(MeetingResponse::Decline, "Comment", false)
        )
        .is_err()
    );
    assert!(
        validate_manual_change(&event, &response(MeetingResponse::Decline, "\0", true)).is_err()
    );
    assert!(
        validate_manual_change(
            &event,
            &response(MeetingResponse::Decline, &"x".repeat(4097), true)
        )
        .is_err()
    );
    assert!(validate_manual_change(&event, &response(MeetingResponse::Cancel, "", true)).is_err());
    assert!(validate_manual_change(&event, &Change::Delete).is_err());
    event
        .extra
        .insert("isOrganizer".into(), serde_json::json!(true));
    assert!(validate_manual_change(&event, &response(MeetingResponse::Accept, "", true)).is_err());
    assert!(
        validate_manual_change(
            &event,
            &response(MeetingResponse::Cancel, "Canceled.", true)
        )
        .is_ok()
    );
    assert!(validate_manual_change(&event, &Change::Delete).is_err());
    event.attendees.clear();
    assert!(validate_manual_change(&event, &Change::Delete).is_ok());
    assert!(validate_manual_change(&event, &response(MeetingResponse::Cancel, "", true)).is_err());
    event
        .extra
        .insert("isCancelled".into(), serde_json::json!(true));
    assert!(validate_manual_change(&event, &Change::ClearCategory).is_err());
}

#[test]
fn approval_is_durable_consumed_once_and_unknown_outcomes_are_never_replayed() {
    let root = tempfile::tempdir().unwrap();
    let (calendar, recommendation) = fixture();
    let mut store = Store::open(root.path()).unwrap();
    store.save_workiq_calendar(&calendar, "owner").unwrap();
    let inbox = BTreeMap::from([(recommendation.id.clone(), recommendation.clone())]);
    store.save_recommendations(&inbox).unwrap();
    let mut approved = recommendation.clone();
    approved.state = State::Dispatching;
    let operation = Operation {
        recommendation: approved,
        event: calendar.events[0].clone(),
        change: Change::Category("Office".into()),
    };
    store.approve_recommendation(&operation).unwrap();
    assert!(store.approve_recommendation(&operation).is_err());
    store.save_recommendations(&inbox).unwrap();
    drop(store);
    let mut store = Store::open(root.path()).unwrap();
    let recovered = store.recommendations().unwrap();
    assert_eq!(recovered[&recommendation.id].state, State::Review);
    let proof = store.calendar_approval(&recommendation.id).unwrap();
    assert_eq!(proof.event.id, operation.event.id);
    assert_eq!(proof.change, operation.change);
    proof.validate("owner").unwrap();
    assert!(proof.validate("other").is_err());
    assert!(store.calendar_approval("missing").is_err());
    assert!(store.calendar_approval(&"a".repeat(64)).is_err());
    store.save_recommendations(&recovered).unwrap();
    assert!(store.approve_recommendation(&operation).is_err());
}

#[test]
fn logistics_placeholders_do_not_inherit_another_meetings_role_or_buffer() {
    let now = Local::now().fixed_offset();
    let (base, _) = fixture();
    for title in [
        "P\u{159}esun / rezerva",
        "Presun a priprava",
        "BLOCKER p\u{159}esun",
        "P\u{159}\u{ed}prava",
        "Travel buffer",
    ] {
        let mut event = base.events[0].clone();
        event.title = title.into();
        let calendar =
            Calendar::build(vec![event.clone()], base.coverage.clone(), Vec::new()).unwrap();
        assert_eq!(
            recommendations::calendar_context(&calendar, &event)["current_title_is_buffer"],
            true
        );
        for kind in [
            Kind::Preparation,
            Kind::Travel,
            Kind::FollowUp,
            Kind::Category,
        ] {
            let mut recommendation = Recommendation::from_insights(
                &event,
                "owner",
                Suggestion {
                    kind,
                    reason: "Evidence-led suggestion.".into(),
                    reason_cs: "Navrh podle podkladu.".into(),
                    category: (kind == Kind::Category).then(|| "Office".into()),
                    minutes: matches!(kind, Kind::Preparation | Kind::Travel).then_some(30),
                },
                now,
            )
            .unwrap();
            assert_eq!(
                recommendation
                    .current(&calendar, Some("owner"), now)
                    .is_some(),
                kind == Kind::Category
            );
            if kind == Kind::Preparation || kind == Kind::Travel {
                recommendation.state = State::Dispatching;
                let operation = Operation {
                    recommendation,
                    event: event.clone(),
                    change: Change::Buffer {
                        title: "Preparation".into(),
                        start: event.start - Duration::minutes(30),
                        end: event.start,
                        category: None,
                    },
                };
                assert!(operation.validate("owner").is_err());
            }
        }
    }
    for title in [
        "Travel industry workshop",
        "Transfer learning workshop",
        "Workshop planning",
    ] {
        let mut event = base.events[0].clone();
        event.title = title.into();
        assert_eq!(
            recommendations::calendar_context(&base, &event)["current_title_is_buffer"],
            false
        );
    }
}

#[test]
fn account_changes_block_approval_and_old_unresolved_writes_remain_visible() {
    let root = tempfile::tempdir().unwrap();
    let (calendar, mut recommendation) = fixture();
    let mut store = Store::open(root.path()).unwrap();
    store.save_workiq_calendar(&calendar, "other").unwrap();
    store
        .save_recommendations(&BTreeMap::from([(
            recommendation.id.clone(),
            recommendation.clone(),
        )]))
        .unwrap();
    recommendation.state = State::Dispatching;
    let operation = Operation {
        recommendation: recommendation.clone(),
        event: calendar.events[0].clone(),
        change: Change::Category("Office".into()),
    };
    assert!(store.approve_recommendation(&operation).is_err());
    recommendation.state = State::Review;
    let future = Local::now().fixed_offset() + Duration::days(365);
    assert!(recommendation.visible(&Calendar::default(), Some("owner"), future));
    assert!(!recommendation.visible(&calendar, Some("other"), future));
}

#[test]
fn free_slots_respect_collisions_workdays_and_immediate_travel() {
    let (mut calendar, _) = fixture();
    let now = Local::now().fixed_offset();
    let monday = calendar::monday(now.date_naive()) + Duration::days(7);
    let start = Local
        .from_local_datetime(&monday.and_hms_opt(10, 0, 0).unwrap())
        .single()
        .unwrap()
        .fixed_offset();
    let mut meeting = calendar.events[0].clone();
    meeting.start = start;
    meeting.end = start + Duration::hours(1);
    calendar = Calendar::build(vec![meeting.clone()], [monday].into(), Vec::new()).unwrap();
    assert_eq!(
        recommendations::free_interval(&calendar, &meeting, 30, now, true),
        Some((start - Duration::minutes(30), start))
    );
    let mut busy = meeting.clone();
    busy.id = "busy".into();
    busy.start = start - Duration::minutes(15);
    calendar.events.push(busy);
    assert!(recommendations::free_interval(&calendar, &meeting, 30, now, true).is_none());
    meeting.start = Local
        .from_local_datetime(&(monday + Duration::days(5)).and_hms_opt(10, 0, 0).unwrap())
        .single()
        .unwrap()
        .fixed_offset();
    meeting.end = meeting.start + Duration::hours(1);
    assert!(recommendations::free_interval(&calendar, &meeting, 30, now, true).is_none());
}

#[test]
fn calendar_context_contains_only_bounded_confirmed_neighbours_and_protects_private_text() {
    let (mut calendar, _) = fixture();
    let meeting = calendar.events[0].clone();
    let mut previous = meeting.clone();
    previous.id = "previous".into();
    previous.start = meeting.start - Duration::minutes(30);
    previous.end = meeting.start;
    previous.title = "Travel buffer".into();
    previous.location = "Private location".into();
    calendar.events.push(previous);
    let context = recommendations::calendar_context(&calendar, &meeting);
    assert_eq!(context["existing_travel_buffer"], true);
    assert_eq!(context["position_and_traffic_verified"], false);
    let mut question = meeting;
    question
        .extra
        .insert("tomlook_calendar_context".into(), context);
    let protected = recommendations::protected_calendar_context(&question);
    assert!(protected.iter().any(|text| text == "Private location"));
}

fn offsite_fixture() -> (
    Calendar,
    calendar::Event,
    chrono::DateTime<chrono::FixedOffset>,
) {
    let now = Local::now().fixed_offset();
    let monday = calendar::monday(now.date_naive()) + Duration::days(7);
    let mut event = calendar::fixture(1).remove(0);
    event.start = Local
        .from_local_datetime(&monday.and_hms_opt(14, 0, 0).unwrap())
        .single()
        .unwrap()
        .fixed_offset();
    event.end = event.start + Duration::hours(1);
    event.is_all_day = false;
    event.status = "accepted".into();
    event.title = "Client architecture workshop".into();
    event.location = "Client office".into();
    event.category = "mimo office".into();
    event
        .extra
        .insert("categories".into(), serde_json::json!(["mimo office"]));
    let calendar = Calendar::build(vec![event.clone()], [monday].into(), Vec::new()).unwrap();
    (calendar, event, now)
}

#[test]
fn office_means_home_employer_premises_and_not_any_office() {
    let meanings = recommendations::category_meanings(&[
        "office".into(),
        "mimo office".into(),
        "call".into(),
        "Custom".into(),
    ]);
    assert!(
        meanings["office"]
            .as_str()
            .unwrap()
            .contains("employer's premises")
    );
    assert!(
        meanings["office"]
            .as_str()
            .unwrap()
            .contains("not this category")
    );
    assert!(
        meanings["mimo office"]
            .as_str()
            .unwrap()
            .contains("customer")
    );
    assert!(
        meanings["call"]
            .as_str()
            .unwrap()
            .contains("conference link alone")
    );
    assert!(meanings.get("Custom").is_none());
}

#[test]
fn offsite_suggestions_cover_arrival_and_departure_without_inventing_a_route() {
    let (calendar, event, now) = offsite_fixture();
    let suggestions = recommendations::complete_suggestions(&calendar, &event, vec![]).unwrap();
    assert_eq!(suggestions.len(), 2);
    for (kind, start, end) in [
        (
            Kind::Travel,
            event.start - Duration::minutes(30),
            event.start,
        ),
        (
            Kind::TravelAfter,
            event.end,
            event.end + Duration::minutes(30),
        ),
    ] {
        let suggestion = suggestions
            .iter()
            .find(|suggestion| suggestion.kind == kind)
            .unwrap();
        assert!(suggestion.reason.contains("estimated 30 minutes"));
        assert_eq!(suggestion.minutes, Some(30));
        let mut record =
            Recommendation::from_insights(&event, "owner", suggestion.clone(), now).unwrap();
        let change = recommendations::proposed_change(
            &record,
            &calendar,
            Some("owner"),
            now,
            tomlook::i18n::Language::Cs,
        )
        .unwrap();
        assert!(
            matches!(&change, Change::Buffer { start: actual_start, end: actual_end, .. }
            if *actual_start == start && *actual_end == end)
        );
        record.state = State::Dispatching;
        let operation = Operation {
            recommendation: record,
            event: event.clone(),
            change,
        };
        operation.validate("owner").unwrap();
        let mut reversed = operation.clone();
        if let Change::Buffer { start, end, .. } = &mut reversed.change {
            *start = event.start - Duration::minutes(30);
            *end = event.start;
        }
        assert_eq!(reversed.validate("owner").is_ok(), kind == Kind::Travel);
    }
    let first =
        Recommendation::from_insights(&event, "owner", suggestions[0].clone(), now).unwrap();
    let second =
        Recommendation::from_insights(&event, "owner", suggestions[1].clone(), now).unwrap();
    assert_ne!(first.id, second.id);
}

#[test]
fn arrival_and_departure_collisions_never_move_the_buffer_away_from_the_meeting() {
    let (mut calendar, event, now) = offsite_fixture();
    let mut busy = event.clone();
    busy.id = "other-meeting".into();
    busy.category.clear();
    busy.start = event.end;
    busy.end = event.end + Duration::minutes(15);
    calendar.events.push(busy.clone());
    assert!(recommendations::travel_interval(&calendar, &event, 30, now, Kind::Travel).is_some());
    assert!(
        recommendations::travel_interval(&calendar, &event, 30, now, Kind::TravelAfter).is_none()
    );
    calendar.events.last_mut().unwrap().start = event.start - Duration::minutes(15);
    calendar.events.last_mut().unwrap().end = event.start;
    assert!(recommendations::travel_interval(&calendar, &event, 30, now, Kind::Travel).is_none());
    assert!(
        recommendations::travel_interval(&calendar, &event, 30, now, Kind::TravelAfter).is_some()
    );
    calendar.events.last_mut().unwrap().status = "notResponded".into();
    assert!(recommendations::travel_interval(&calendar, &event, 30, now, Kind::Travel).is_some());
    calendar.events.last_mut().unwrap().status = "accepted".into();
    calendar
        .events
        .last_mut()
        .unwrap()
        .extra
        .insert("showAs".into(), serde_json::json!("free"));
    assert!(recommendations::travel_interval(&calendar, &event, 30, now, Kind::Travel).is_some());
}

#[test]
fn travel_sides_respect_working_hours_and_suppress_existing_buffers_independently() {
    let (mut calendar, event, now) = offsite_fixture();
    let mut buffer = event.clone();
    buffer.id = "arrival".into();
    buffer.title = "Travel buffer".into();
    buffer.start = event.start - Duration::minutes(30);
    buffer.end = event.start;
    calendar.events.push(buffer.clone());
    let suggestions = recommendations::complete_suggestions(&calendar, &event, vec![]).unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].kind, Kind::TravelAfter);
    buffer.id = "departure".into();
    buffer.start = event.end;
    buffer.end = event.end + Duration::minutes(30);
    calendar.events.push(buffer);
    assert!(
        recommendations::complete_suggestions(&calendar, &event, vec![])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        recommendations::calendar_context(&calendar, &event)["existing_travel_buffer_after"],
        true
    );
    let mut late = event.clone();
    late.start += Duration::hours(3);
    late.end += Duration::hours(3);
    assert!(
        recommendations::travel_interval(&Calendar::default(), &late, 30, now, Kind::Travel)
            .is_some()
    );
    assert!(
        recommendations::travel_interval(&Calendar::default(), &late, 30, now, Kind::TravelAfter)
            .is_none()
    );
    let mut early = event.clone();
    early.start -= Duration::hours(6);
    early.end -= Duration::hours(6);
    assert!(
        recommendations::travel_interval(&Calendar::default(), &early, 30, now, Kind::Travel)
            .is_none()
    );
}

#[test]
fn a_remote_reclassification_does_not_inherit_offsite_buffers_and_models_keep_supported_estimates()
{
    let (calendar, mut event, _) = offsite_fixture();
    let category = |category: &str| Suggestion {
        kind: Kind::Category,
        category: Some(category.into()),
        minutes: None,
        reason: "Participation mode is explicit.".into(),
        reason_cs: "Forma ucasti je uvedena.".into(),
    };
    let mut remote = event.clone();
    remote.id = "remote".into();
    remote.category = "call".into();
    remote
        .extra
        .insert("categories".into(), serde_json::json!(["call"]));
    let mut events = calendar.events.clone();
    events.push(remote.clone());
    let calendar = Calendar::build(events, calendar.coverage.clone(), vec![]).unwrap();
    let suggestions =
        recommendations::complete_suggestions(&calendar, &event, vec![category("call")]).unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].kind, Kind::Category);
    assert!(
        recommendations::complete_suggestions(&calendar, &remote, vec![])
            .unwrap()
            .is_empty()
    );
    let mut estimate = category("mimo office");
    estimate.kind = Kind::Travel;
    estimate.category = None;
    estimate.minutes = Some(60);
    let suggestions =
        recommendations::complete_suggestions(&calendar, &event, vec![estimate]).unwrap();
    assert_eq!(
        suggestions
            .iter()
            .find(|suggestion| suggestion.kind == Kind::Travel)
            .unwrap()
            .minutes,
        Some(60)
    );
    let travel = suggestions[0].clone();
    let suggestions = recommendations::complete_suggestions(
        &calendar,
        &event,
        vec![category("call"), travel.clone()],
    )
    .unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].kind, Kind::Category);
    event.status = "notResponded".into();
    assert!(
        recommendations::complete_suggestions(&calendar, &event, vec![])
            .unwrap()
            .is_empty()
    );
    event.status = "accepted".into();
    event.is_all_day = true;
    assert!(
        recommendations::complete_suggestions(&calendar, &event, vec![travel.clone()])
            .unwrap()
            .is_empty()
    );
    event.is_all_day = false;
    event.title = "Travel buffer".into();
    assert!(
        recommendations::complete_suggestions(&calendar, &event, vec![travel])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn both_travel_sides_fit_the_four_draft_limit_without_displacing_preparation() {
    let (calendar, event, _) = offsite_fixture();
    let suggestions = [
        Kind::Preparation,
        Kind::MissingMaterials,
        Kind::PriorityConflict,
        Kind::Transition,
    ]
    .into_iter()
    .map(|kind| Suggestion {
        kind,
        reason: "A relevant meeting action.".into(),
        reason_cs: "Ukol ke schuzce.".into(),
        category: None,
        minutes: (kind == Kind::Preparation).then_some(30),
    })
    .collect();
    let suggestions =
        recommendations::complete_suggestions(&calendar, &event, suggestions).unwrap();
    assert_eq!(suggestions.len(), 4);
    for kind in [Kind::Preparation, Kind::Travel, Kind::TravelAfter] {
        assert!(suggestions.iter().any(|suggestion| suggestion.kind == kind));
    }
}

#[test]
fn legacy_category_rules_expire_without_hiding_unknown_writes_or_discarding_preparation() {
    let (calendar, event, now) = offsite_fixture();
    let suggestion = Suggestion {
        kind: Kind::Category,
        category: Some("office".into()),
        minutes: None,
        reason: "Legacy category proposal.".into(),
        reason_cs: "Stary navrh kategorie.".into(),
    };
    let record = Recommendation::from_insights(&event, "owner", suggestion, now).unwrap();
    let mut json = serde_json::to_value(&record).unwrap();
    json.as_object_mut().unwrap().remove("policy_revision");
    let mut legacy: Recommendation = serde_json::from_value(json).unwrap();
    assert!(legacy.current(&calendar, Some("owner"), now).is_none());
    assert!(!legacy.visible(&calendar, Some("owner"), now));
    legacy.state = State::Review;
    assert!(legacy.visible(&calendar, Some("owner"), now));
    legacy.state = State::Pending;
    legacy.suggestion.kind = Kind::Preparation;
    legacy.suggestion.category = None;
    legacy.suggestion.minutes = Some(30);
    assert!(legacy.current(&calendar, Some("owner"), now).is_some());
}

#[test]
fn confirmed_buffers_are_uncategorized_and_only_inherit_the_parent_uncertainty() {
    let (mut calendar, mut event, now) = offsite_fixture();
    let mut reference = event.clone();
    reference.id = "uncertainty-reference".into();
    reference.status = "cancelled".into();
    reference.category = "nejist\u{e9}".into();
    reference
        .extra
        .insert("categories".into(), serde_json::json!(["nejist\u{e9}"]));
    calendar.events.push(reference);
    for parent_category in ["mimo office", "nejist\u{e9}", "blocker"] {
        event.category = parent_category.into();
        event
            .extra
            .insert("categories".into(), serde_json::json!([parent_category]));
        calendar = Calendar::build(
            vec![event.clone(), calendar.events.last().unwrap().clone()],
            calendar.coverage.clone(),
            vec![],
        )
        .unwrap();
        for kind in [
            Kind::Travel,
            Kind::TravelAfter,
            Kind::Preparation,
            Kind::PreparationAdvance,
        ] {
            let record = Recommendation::from_insights(
                &event,
                "owner",
                Suggestion {
                    kind,
                    reason: "Reserve time for this meeting.".into(),
                    reason_cs: "Vyhrad si cas na schuzku.".into(),
                    category: None,
                    minutes: Some(if kind == Kind::PreparationAdvance {
                        120
                    } else {
                        30
                    }),
                },
                now,
            )
            .unwrap();
            let change = recommendations::proposed_change(
                &record,
                &calendar,
                Some("owner"),
                now,
                tomlook::i18n::Language::En,
            )
            .unwrap();
            let Change::Buffer { category, .. } = &change else {
                panic!("Expected a buffer")
            };
            assert_eq!(
                category.as_deref(),
                (parent_category != "mimo office").then_some(parent_category)
            );
            let mut approved = record;
            approved.state = State::Dispatching;
            let operation = Operation {
                recommendation: approved,
                event: event.clone(),
                change,
            };
            operation.validate("owner").unwrap();
            let mut wrong = operation.clone();
            if let Change::Buffer { category, .. } = &mut wrong.change {
                *category = Some("rezerva".into());
            }
            assert!(wrong.validate("owner").is_err());
        }
    }
    assert!(
        recommendations::category_meanings(&["nejist\u{e9}".into()])["nejist\u{e9}"]
            .as_str()
            .unwrap()
            .contains("genuinely uncertain")
    );
}

#[test]
fn skipped_intents_survive_rewording_duration_metadata_policy_changes_and_compaction() {
    let root = tempfile::tempdir().unwrap();
    let (calendar, event, now) = offsite_fixture();
    let suggestion = Suggestion {
        kind: Kind::Travel,
        reason: "Arrival.".into(),
        reason_cs: "Prijezd.".into(),
        category: None,
        minutes: Some(30),
    };
    let mut skipped =
        Recommendation::from_insights(&event, "owner", suggestion.clone(), now).unwrap();
    skipped.state = State::Dismissed;
    skipped.created_at = now - Duration::days(40);
    skipped.policy_revision = 0;
    let key = skipped.dismissal_key().unwrap();
    let store = Store::open(root.path()).unwrap();
    store
        .save_recommendations(&BTreeMap::from([(skipped.id.clone(), skipped.clone())]))
        .unwrap();
    let mut inbox = store.recommendations().unwrap();
    recommendations::compact(&mut inbox, &Calendar::default(), Some("owner"), now);
    assert!(inbox.is_empty());
    store.save_recommendations(&inbox).unwrap();
    drop(store);
    let store = Store::open(root.path()).unwrap();
    assert!(store.recommendation_dismissals().unwrap().contains(&key));
    let mut changed = event.clone();
    changed.title = "Updated meeting title".into();
    changed.start += Duration::hours(1);
    changed.end += Duration::hours(1);
    changed
        .extra
        .insert("changeKey".into(), serde_json::json!("new-version"));
    let mut suggestion = suggestion;
    suggestion.reason = "Updated route estimate.".into();
    suggestion.minutes = Some(60);
    let regenerated =
        Recommendation::from_insights(&changed, "owner", suggestion.clone(), now).unwrap();
    assert_ne!(skipped.id, regenerated.id);
    assert_eq!(skipped.dismissal_key(), regenerated.dismissal_key());
    store
        .save_recommendations(&BTreeMap::from([(
            regenerated.id.clone(),
            regenerated.clone(),
        )]))
        .unwrap();
    assert_eq!(
        store.recommendations().unwrap()[&regenerated.id].state,
        State::Dismissed
    );
    for (account, id, kind, category) in [
        ("other-owner", event.id.as_str(), Kind::Travel, None),
        ("owner", "other-meeting", Kind::Travel, None),
        ("owner", event.id.as_str(), Kind::TravelAfter, None),
        (
            "owner",
            event.id.as_str(),
            Kind::Category,
            Some("mimo office"),
        ),
    ] {
        changed.id = id.into();
        suggestion.kind = kind;
        suggestion.category = category.map(str::to_owned);
        suggestion.minutes = kind.is_travel().then_some(30);
        assert_ne!(
            Recommendation::from_insights(&changed, account, suggestion.clone(), now)
                .unwrap()
                .dismissal_key(),
            Some(key.clone())
        );
    }
    assert_eq!(calendar.events.len(), 1);
}

#[test]
fn dismissal_persistence_is_atomic_and_does_not_hide_an_unknown_write() {
    let root = tempfile::tempdir().unwrap();
    let (calendar, event, now) = offsite_fixture();
    let mut record = Recommendation::from_insights(
        &event,
        "owner",
        Suggestion {
            kind: Kind::Travel,
            reason: "Arrival.".into(),
            reason_cs: "Prijezd.".into(),
            category: None,
            minutes: Some(30),
        },
        now,
    )
    .unwrap();
    record.state = State::Dismissed;
    let store = Store::open(root.path()).unwrap();
    let oversized = (0..513)
        .map(|index| {
            let mut clone = record.clone();
            clone.id = format!("{index:064x}");
            (clone.id.clone(), clone)
        })
        .collect();
    assert!(store.save_recommendations(&oversized).is_err());
    assert!(store.recommendation_dismissals().unwrap().is_empty());
    store
        .save_recommendations(&BTreeMap::from([(record.id.clone(), record.clone())]))
        .unwrap();
    let mut changed = event.clone();
    changed.title = "Different metadata".into();
    let mut unknown =
        Recommendation::from_insights(&changed, "owner", record.suggestion.clone(), now).unwrap();
    unknown.state = State::Dispatching;
    store
        .save_recommendations(&BTreeMap::from([(unknown.id.clone(), unknown.clone())]))
        .unwrap();
    let recovered = store.recommendations().unwrap();
    assert_eq!(recovered[&unknown.id].state, State::Review);
    assert!(recovered[&unknown.id].visible(&calendar, Some("owner"), now));
}

#[test]
fn advance_preparation_finds_two_hours_this_week_for_next_weeks_workshop() {
    let (mut calendar, mut event, _) = offsite_fixture();
    let meeting_day = event.start.with_timezone(&Local).date_naive();
    let monday = meeting_day - Duration::days(7);
    let local_time = |day: chrono::NaiveDate, hour, minute| {
        Local
            .from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap())
            .single()
            .unwrap()
            .fixed_offset()
    };
    let now = local_time(monday, 7, 0);
    let mut busy = event.clone();
    busy.id = "busy-monday".into();
    busy.start = local_time(monday, 8, 0);
    busy.end = local_time(monday, 18, 0);
    let mut next_busy = busy.clone();
    next_busy.id = "busy-tuesday".into();
    next_busy.start = local_time(monday + Duration::days(1), 8, 0);
    next_busy.end = local_time(monday + Duration::days(1), 10, 30);
    calendar = Calendar::build(
        vec![event.clone(), busy, next_busy],
        calendar.coverage.clone(),
        vec![],
    )
    .unwrap();
    let expected_start = local_time(monday + Duration::days(1), 10, 30);
    assert_eq!(
        recommendations::advance_preparation_interval(&calendar, &event, 120, now),
        Some((expected_start, expected_start + Duration::hours(2)))
    );
    let mut record = Recommendation::from_insights(
        &event,
        "owner",
        Suggestion {
            kind: Kind::PreparationAdvance,
            reason: "Create a new workshop demo.".into(),
            reason_cs: "Priprav nove demo na workshop.".into(),
            category: None,
            minutes: Some(120),
        },
        now,
    )
    .unwrap();
    let change = recommendations::proposed_change(
        &record,
        &calendar,
        Some("owner"),
        now,
        tomlook::i18n::Language::Cs,
    )
    .unwrap();
    assert!(
        matches!(&change, Change::Buffer { start, end, category: None, .. } if *start == expected_start && *end - *start == Duration::hours(2))
    );
    let advance_key = record.dismissal_key();
    let mut rehearsal = record.suggestion.clone();
    rehearsal.kind = Kind::Preparation;
    rehearsal.minutes = Some(30);
    assert_eq!(
        advance_key,
        Recommendation::from_insights(&event, "owner", rehearsal.clone(), now)
            .unwrap()
            .dismissal_key()
    );
    assert!(
        recommendations::validate_suggestions(&[record.suggestion.clone(), rehearsal]).is_err()
    );
    record.suggestion.minutes = Some(30);
    assert!(record.suggestion.validate().is_err());
    event.is_all_day = true;
    assert!(recommendations::advance_preparation_interval(&calendar, &event, 120, now).is_none());
    event.is_all_day = false;
    let last_weekday = local_time(monday + Duration::days(4), 17, 30);
    let weekend = recommendations::advance_preparation_interval(
        &Calendar::default(),
        &event,
        120,
        last_weekday,
    )
    .unwrap();
    assert_eq!(weekend.0.with_timezone(&Local).date_naive(), meeting_day);
    assert_eq!(weekend.0, local_time(meeting_day, 8, 0));
    assert!(recommendations::advance_preparation_interval(&calendar, &event, 181, now).is_none());
    assert!(
        recommendations::advance_preparation_interval(&calendar, &event, 120, event.start)
            .is_none()
    );
    let during_work = local_time(monday, 8, 21) + Duration::seconds(15);
    let empty = Calendar::default();
    let first_offer =
        recommendations::advance_preparation_interval(&empty, &event, 120, during_work).unwrap();
    assert_eq!(first_offer.0, local_time(monday, 8, 30));
    assert_eq!(
        recommendations::advance_preparation_interval(
            &empty,
            &event,
            120,
            during_work + Duration::seconds(20)
        ),
        Some(first_offer)
    );
}

#[test]
fn conflict_context_has_local_offsets_and_the_actual_overlap_counterparts() {
    let (mut calendar, event, _) = offsite_fixture();
    let mut busy = event.clone();
    busy.id = "transfer".into();
    busy.title = "Transfer reserve".into();
    busy.start = (event.start + Duration::minutes(15))
        .with_timezone(&chrono::FixedOffset::east_opt(0).unwrap());
    busy.end = event.end;
    calendar.events.push(busy.clone());
    let context = recommendations::calendar_context(&calendar, &event);
    let described_start = chrono::DateTime::parse_from_rfc3339(
        context["overlaps_confirmed"][0]["start"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(described_start, busy.start);
    assert_eq!(
        described_start.offset(),
        busy.start.with_timezone(&Local).fixed_offset().offset()
    );
    assert_eq!(
        context["overlaps_confirmed"][0]["title"],
        "Transfer reserve"
    );
    assert_eq!(context["display_time_zone"], "local");
    assert_eq!(recommendations::conflicts(&calendar, &event).len(), 1);
    calendar.events.last_mut().unwrap().status = "notResponded".into();
    assert!(recommendations::conflicts(&calendar, &event).is_empty());
}
