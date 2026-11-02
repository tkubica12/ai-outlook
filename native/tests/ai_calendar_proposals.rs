use chrono::{Duration, Local};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::{AtomicBool, AtomicUsize}},
    time::{Duration as Wait, Instant},
};
use tomlook::{
    ai,
    calendar::{self, Calendar, Event},
    changes::{self, Request},
    event_creation::{CreationResult, State},
    recommendations::{Change, MeetingResponse, State as ChangeState},
    storage::Store,
    worker::{Activity, Command, Options, Worker},
};

const OWNER: &str = "owner@example.invalid";

fn wait(worker: &Worker, ready: impl Fn(&Activity) -> bool) -> Arc<Activity> {
    let deadline = Instant::now() + Wait::from_secs(10);
    loop {
        let snapshot = worker.activity.lock().unwrap().clone();
        if ready(&snapshot) { return snapshot; }
        assert!(Instant::now() < deadline, "Expected calendar proposal state was not published");
        std::thread::sleep(Wait::from_millis(5));
    }
}

fn start(events: Vec<Event>) -> (tempfile::TempDir, Store, Worker, tokio::sync::mpsc::Receiver<ai::Command>) {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    let today = Local::now().date_naive();
    let calendar = Calendar::build(events, (0..60).map(|offset| today + Duration::days(offset)).collect(), vec![]).unwrap();
    store.save_workiq_calendar(&calendar, OWNER).unwrap();
    store.save_preferences(&tomlook::storage::Preferences {
        paused: true, prepare_enabled: false, ..Default::default()
    }).unwrap();
    let worker = Worker::start(Options {
        state: root.path().into(), legacy_root: root.path().join("absent"), demo: None,
    }, eframe::egui::Context::default()).unwrap();
    let (sender, commands) = tokio::sync::mpsc::channel(64);
    worker.commands.send(Command::AttachAi {
        commands: sender, ready: Arc::new(AtomicBool::new(true)),
        occupied: Arc::new(AtomicUsize::new(0)), terminated: Arc::new(AtomicBool::new(false)),
        analysis_revision: Arc::new(std::sync::OnceLock::new()), workiq_ready: Arc::new(AtomicBool::new(true)),
    }).unwrap();
    wait(&worker, |activity| activity.task_account.as_deref() == Some(OWNER));
    (root, store, worker, commands)
}

fn propose(worker: &Worker, account: &str, request: Value, cancelled: bool) -> Result<Value, String> {
    let run = tomlook::chat::Run::new(1, tomlook::chat::Mode::Fast, Arc::new(|_| {}));
    run.cancelled.store(cancelled, std::sync::atomic::Ordering::Release);
    let (reply, mut result) = tokio::sync::oneshot::channel();
    worker.commands.send(Command::ProposeChange {
        account: account.into(), request: Box::new(serde_json::from_value(request).unwrap()), run, reply,
    }).unwrap();
    let deadline = Instant::now() + Wait::from_secs(10);
    loop {
        match result.try_recv() {
            Ok(result) => return result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(error) => panic!("Proposal reply failed: {error}"),
        }
        assert!(Instant::now() < deadline, "Proposal timed out");
        std::thread::sleep(Wait::from_millis(5));
    }
}

fn next_write(commands: &mut tokio::sync::mpsc::Receiver<ai::Command>) -> ai::Command {
    let deadline = Instant::now() + Wait::from_secs(10);
    loop {
        if let Ok(command) = commands.try_recv()
            && matches!(command, ai::Command::CreateEvents { .. } | ai::Command::CalendarChange { .. })
        {
            return command;
        }
        assert!(Instant::now() < deadline, "Approved write was not dispatched");
        std::thread::sleep(Wait::from_millis(5));
    }
}

fn no_writes(commands: &mut tokio::sync::mpsc::Receiver<ai::Command>) {
    while let Ok(command) = commands.try_recv() {
        assert!(!matches!(command, ai::Command::CreateEvents { .. } | ai::Command::CalendarChange { .. }),
            "Proposing or reviewing must never dispatch a calendar write");
    }
}

fn stop(mut worker: Worker) {
    worker.commands.send(Command::Exit).unwrap();
    worker.done.take().unwrap().recv_timeout(Wait::from_secs(10)).unwrap().unwrap();
}

fn creation() -> Value {
    let start = Local::now().fixed_offset() + Duration::days(3);
    json!({"reason":"User requested planning","reason_cs":"Uzivatel pozadal o planovani",
        "action":{"kind":"create","event":{"title":"Planning","start":start,"end":start + Duration::hours(1),
            "description":"Review the plan","private":true,"teams":true,
            "attendees":[{"email":"alex@example.invalid","name":"Alex","role":"required"},
                {"email":"jamie@example.invalid","role":"optional"}]}}})
}

fn event(id: &str, title: &str, days: i64) -> Event {
    let mut event = calendar::fixture(1).remove(0);
    event.id = id.into();
    event.title = title.into();
    event.start = Local::now().fixed_offset() + Duration::days(days);
    event.end = event.start + Duration::hours(1);
    event.is_all_day = false;
    event.status = "organizer".into();
    event.attendees.clear();
    event.extra.remove("tomlook_attendees_total");
    event.extra.insert("isOrganizer".into(), json!(true));
    event.extra.insert("type".into(), json!("singleInstance"));
    event
}

fn deletion(event: &Event) -> Value {
    json!({"id":event.id,"event_revision":tomlook::scheduler::fingerprint(event),
        "reason":"Obsolete blocker; keep the final invitation","reason_cs":"Stary blocker; finalni pozvanku zachovat",
        "action":{"kind":"delete"}})
}

#[test]
fn ai_calendar_creation_is_persistent_pending_until_exact_single_use_button_approval() {
    let (root, store, worker, mut commands) = start(vec![]);
    let request = creation();
    assert!(propose(&worker, "another@example.invalid", request.clone(), false).is_err());
    assert!(propose(&worker, OWNER, request.clone(), true).is_err());
    let result = propose(&worker, OWNER, request.clone(), false).unwrap();
    let id = result["change_id"].as_str().unwrap();
    let snapshot = wait(&worker, |activity| activity.event_creations.contains_key(id));
    let pending = snapshot.event_creations[id].clone();
    assert_eq!(pending.state, State::Pending);
    assert_eq!(Store::open(root.path()).unwrap().event_creations().unwrap()[id].state, State::Pending);
    assert_eq!(propose(&worker, OWNER, request, false).unwrap()["change_id"], id);
    no_writes(&mut commands);
    worker.commands.send(Command::CreateEvents {
        account: OWNER.into(), draft: Box::new(pending.draft.clone()), parent: None,
    }).unwrap();
    let ai::Command::CreateEvents { record, reply, wake } = next_write(&mut commands) else { panic!("Expected creation"); };
    assert_eq!(record.id, id);
    assert_eq!(record.draft, pending.draft);
    assert_eq!(record.draft.attendees.len(), 2);
    assert_eq!(record.state, State::Creating);
    assert_eq!(store.event_creations().unwrap()[id].state, State::Review, "A crash after approval is uncertain, not replayable");
    reply.blocking_send(CreationResult {
        id: id.into(), state: State::Review, events: vec![], message: record.review_message("Outcome unknown"),
    }).unwrap();
    wake.send(Command::Wake).unwrap();
    wait(&worker, |activity| activity.event_creations[id].state == State::Review);
    worker.commands.send(Command::CreateEvents {
        account: OWNER.into(), draft: Box::new(pending.draft), parent: None,
    }).unwrap();
    wait(&worker, |activity| activity.event_creations[id].state == State::Review);
    stop(worker);
    no_writes(&mut commands);
    assert_eq!(store.event_creations().unwrap()[id].state, State::Review);
}

#[test]
fn ai_calendar_creation_can_be_dismissed_and_cannot_be_approved_by_another_account() {
    let (_root, mut store, worker, mut commands) = start(vec![]);
    let result = propose(&worker, OWNER, creation(), false).unwrap();
    let id = result["change_id"].as_str().unwrap().to_owned();
    let pending = store.event_creations().unwrap()[&id].clone();
    let calendar = store.calendar().unwrap();
    store.save_workiq_calendar(&calendar, "other@example.invalid").unwrap();
    worker.commands.send(Command::CreateEvents {
        account: OWNER.into(), draft: Box::new(pending.draft.clone()), parent: None,
    }).unwrap();
    wait(&worker, |activity| activity.task_account.as_deref() == Some("other@example.invalid"));
    store.save_workiq_calendar(&calendar, OWNER).unwrap();
    worker.commands.send(Command::DismissEventCreation { id: id.clone(), account: OWNER.into() }).unwrap();
    wait(&worker, |activity| activity.event_creations.get(&id).is_some_and(|record| record.state == State::Dismissed));
    worker.commands.send(Command::CreateEvents {
        account: OWNER.into(), draft: Box::new(pending.draft), parent: None,
    }).unwrap();
    stop(worker);
    no_writes(&mut commands);
    assert_eq!(store.event_creations().unwrap()[&id].state, State::Dismissed);
}

#[test]
fn ai_calendar_delete_proposes_only_the_old_blocker_and_preserves_the_final_invitation() {
    let blocker = event("AAMkOld/Blocker+ID=", "BLOCKER: Networking", 30);
    let final_meeting = event("final-invitation", "Networking final", 38);
    let (_root, store, worker, mut commands) = start(vec![blocker.clone(), final_meeting.clone()]);
    let result = propose(&worker, OWNER, deletion(&blocker), false).unwrap();
    let id = result["change_id"].as_str().unwrap().to_owned();
    let snapshot = wait(&worker, |activity| activity.recommendation_view.iter().any(|entry| entry.id == id));
    let proposal = &snapshot.recommendations[&id];
    assert_eq!(proposal.state, ChangeState::Pending);
    assert_eq!(proposal.manual_change, Some(Change::Delete));
    assert_eq!(store.recommendations().unwrap().len(), 1);
    no_writes(&mut commands);
    worker.commands.send(Command::ConfirmRecommendation { id: id.clone(), change: Change::Delete }).unwrap();
    let ai::Command::CalendarChange { operation, verify_only, reply, wake } = next_write(&mut commands) else { panic!("Expected deletion"); };
    assert!(!verify_only);
    assert_eq!(operation.event.id, blocker.id);
    assert_eq!(operation.change, Change::Delete);
    reply.blocking_send(ai::CalendarResult {
        id: id.clone(), outcome: tomlook::workiq::connection::calendar_write::Outcome::Removed(blocker.id.clone()),
    }).unwrap();
    wake.send(Command::Wake).unwrap();
    wait(&worker, |activity| activity.recommendations[&id].state == ChangeState::Applied);
    worker.commands.send(Command::ConfirmRecommendation { id: id.clone(), change: Change::Delete }).unwrap();
    stop(worker);
    no_writes(&mut commands);
    let saved = store.calendar().unwrap();
    assert!(saved.event_index(&blocker.id).is_none());
    assert!(saved.event_index(&final_meeting.id).is_some());
}

#[test]
fn ai_calendar_removal_selects_role_safe_actions_and_never_a_series() {
    let mut own = event("owned", "Meeting", 3);
    assert_eq!(tomlook::recommendations::removal_change(&own).unwrap(), Change::Delete);
    own.attendees = vec!["Alex <alex@example.invalid>".into()];
    assert!(matches!(tomlook::recommendations::removal_change(&own).unwrap(),
        Change::Respond { response: MeetingResponse::Cancel, send_response: true, .. }));
    own.extra.insert("isOrganizer".into(), json!(false));
    own.organizer = "Organizer <organizer@example.invalid>".into();
    assert!(matches!(tomlook::recommendations::removal_change(&own).unwrap(),
        Change::Respond { response: MeetingResponse::Decline, send_response: false, .. }));
    own.extra.insert("type".into(), json!("seriesMaster"));
    assert!(tomlook::recommendations::removal_change(&own).is_err());
    own.extra.insert("type".into(), json!("occurrence"));
    assert!(tomlook::recommendations::removal_change(&own).is_ok());
    own.extra.remove("isOrganizer");
    assert!(tomlook::recommendations::removal_change(&own).is_err());
}

#[test]
fn ai_calendar_changed_blocker_cannot_consume_the_old_deletion_approval() {
    let mut blocker = event("old-blocker", "BLOCKER: Planning", 3);
    let (_root, store, worker, mut commands) = start(vec![blocker.clone()]);
    let result = propose(&worker, OWNER, deletion(&blocker), false).unwrap();
    let id = result["change_id"].as_str().unwrap().to_owned();
    wait(&worker, |activity| activity.recommendations.contains_key(&id));
    blocker.title = "Confirmed final meeting".into();
    let calendar = Calendar::build(vec![blocker.clone()], store.calendar().unwrap().coverage, vec![]).unwrap();
    worker.commands.send(Command::WorkiqCalendar { account: OWNER.into(), calendar: Arc::new(calendar) }).unwrap();
    wait(&worker, |activity| activity.recommendations[&id].state == ChangeState::Expired);
    worker.commands.send(Command::ConfirmRecommendation { id, change: Change::Delete }).unwrap();
    stop(worker);
    no_writes(&mut commands);
    assert_eq!(store.calendar().unwrap().events[0].title, blocker.title);
}

#[test]
fn ai_calendar_inflight_removal_cannot_change_the_new_accounts_calendar() {
    let blocker = event("same-id", "BLOCKER: Old account", 3);
    let (_root, store, worker, mut commands) = start(vec![blocker.clone()]);
    let result = propose(&worker, OWNER, deletion(&blocker), false).unwrap();
    let id = result["change_id"].as_str().unwrap().to_owned();
    worker.commands.send(Command::ConfirmRecommendation { id: id.clone(), change: Change::Delete }).unwrap();
    let ai::Command::CalendarChange { reply, wake, .. } = next_write(&mut commands) else { panic!("Expected deletion"); };
    let other = event("same-id", "Other account's appointment", 3);
    let calendar = Calendar::build(vec![other.clone()], store.calendar().unwrap().coverage, vec![]).unwrap();
    worker.commands.send(Command::WorkiqCalendar {
        account: "other@example.invalid".into(), calendar: Arc::new(calendar),
    }).unwrap();
    wait(&worker, |activity| activity.task_account.as_deref() == Some("other@example.invalid"));
    reply.blocking_send(ai::CalendarResult {
        id: id.clone(), outcome: tomlook::workiq::connection::calendar_write::Outcome::Removed(blocker.id),
    }).unwrap();
    wake.send(Command::Wake).unwrap();
    wait(&worker, |activity| activity.recommendations[&id].state == ChangeState::Applied);
    stop(worker);
    assert_eq!(store.calendar_owner().unwrap().as_deref(), Some("other@example.invalid"));
    assert_eq!(store.calendar().unwrap().events[0].title, other.title);
}

#[test]
fn ai_calendar_proposal_schema_rejects_approval_injection_unzoned_times_and_invalid_targets() {
    let valid = creation();
    let request: Request = serde_json::from_value(valid.clone()).unwrap();
    request.validate().unwrap();
    assert!(tomlook::chat_cache::valid(changes::PROPOSE, &valid));
    for path in ["/action/event/start", "/action/event/end"] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(path).unwrap() = json!("2027-01-03T10:00:00");
        assert!(!tomlook::chat_cache::valid(changes::PROPOSE, &invalid));
    }
    for patch in [json!({"approve":true}), json!({"account":"other@example.invalid"}), json!({"id":"existing"})] {
        let mut invalid = valid.clone();
        invalid.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        assert!(!tomlook::chat_cache::valid(changes::PROPOSE, &invalid));
    }
    assert!(!tomlook::chat_cache::valid(changes::PROPOSE, &json!({
        "reason":"Requested deletion","reason_cs":"Pozadovane smazani","action":{"kind":"delete"}})));
    let changes::Action::Create { event } = request.action else { panic!("Expected new event"); };
    let draft = event.draft(tomlook::i18n::Language::En);
    let body = draft.parts("approved")[0].body();
    let sent = chrono::NaiveDateTime::parse_from_str(body["start"]["dateTime"].as_str().unwrap(), "%Y-%m-%dT%H:%M:%S%.f").unwrap();
    assert_eq!(sent.and_utc(), draft.start.with_timezone(&chrono::Utc), "The exact approved time, including fractions, must be sent");
}

#[test]
fn acceptance_category_is_consumed_as_one_approval_and_partial_results_cannot_replay() {
    let mut invitation = event("invitation", "Planning", 3);
    invitation.status = "notResponded".into();
    invitation.organizer = "organizer@example.invalid".into();
    invitation.category.clear();
    invitation.extra.insert("isOrganizer".into(), json!(false));
    invitation.extra.insert("categories".into(), json!([]));
    let mut catalog = event("catalog", "Other meeting", 4);
    catalog.category = "Call".into();
    catalog.extra.insert("categories".into(), json!(["Call"]));
    let (_root, store, worker, mut commands) = start(vec![invitation.clone(), catalog]);
    let change = |category: &str| Change::Respond {
        response: MeetingResponse::Accept, comment: String::new(),
        send_response: false, category: Some(category.into()),
    };
    for category in ["Invented category", "Call"] {
        worker.commands.send(Command::ConfirmMeetingAction {
            event: Box::new(invitation.clone()), account: OWNER.into(), change: change(category),
        }).unwrap();
    }
    let ai::Command::CalendarChange { operation, verify_only, reply, wake } = next_write(&mut commands)
        else { panic!("Expected one combined acceptance"); };
    let id = operation.recommendation.id.clone();
    assert!(!verify_only);
    assert_eq!(operation.change, change("Call"));
    let saved = store.recommendations().unwrap();
    assert_eq!(saved.len(), 1, "Unknown categories are rejected before creating an approval");
    assert_eq!(operation.recommendation.state, ChangeState::Dispatching);
    assert_eq!(saved[&id].state, ChangeState::Review, "Reloading the durable consumed approval requires review, never replay");
    assert_eq!(saved[&id].manual_change.as_ref(), Some(&change("Call")));
    reply.blocking_send(ai::CalendarResult {
        id: id.clone(), outcome: tomlook::workiq::connection::calendar_write::Outcome::Review("Category assigned; acceptance uncertain.".into()),
    }).unwrap();
    wake.send(Command::Wake).unwrap();
    wait(&worker, |activity| activity.recommendations[&id].state == ChangeState::Review);
    worker.commands.send(Command::ConfirmRecommendation { id: id.clone(), change: change("Call") }).unwrap();
    worker.commands.send(Command::ConfirmMeetingAction {
        event: Box::new(invitation), account: OWNER.into(), change: change("Call"),
    }).unwrap();
    stop(worker);
    no_writes(&mut commands);
    assert_eq!(store.recommendations().unwrap()[&id].state, ChangeState::Review);
}
