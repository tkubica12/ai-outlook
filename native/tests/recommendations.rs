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
