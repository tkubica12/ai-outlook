use chrono::{Duration, Local};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
    time::Instant,
};
use tomlook::{
    ai,
    calendar::{self, Calendar, Event},
    event_creation::{CreationResult, State},
    scheduling::{
        BlockerState, ChatDraft, ChatResult, ChatState, Member, Proposal, ProposalStatus,
        ReleaseOutcome, ReleaseResult, Slot,
    },
    storage::Store,
    worker::{Activity, Command, Options, Worker},
};

const ACCOUNT: &str = "owner@example.com";
const OTHER: &str = "other@example.com";

/// Makes `account` the calendar owner, as a refresh of another signed-in account does.
fn own_calendar(store: &mut Store, account: &str) {
    let today = Local::now().date_naive();
    let coverage = (0..30).map(|offset| today + Duration::days(offset)).collect();
    store.save_workiq_calendar(&Calendar::build(Vec::new(), coverage, Vec::new()).unwrap(), account).unwrap();
}

fn switch_account(store: &mut Store, worker: &Worker, account: &str) -> Arc<Activity> {
    own_calendar(store, account);
    worker.commands.send(Command::Wake).unwrap();
    wait_for(worker, |activity| activity.task_account.as_deref() == Some(account))
}

/// The saved proposal once `ready`, exactly as saved.
fn saved(store: &Store, account: &str, id: &str, ready: impl Fn(&Proposal) -> bool) -> Proposal {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let saved = store.proposal(account, id).unwrap().unwrap();
        if ready(&saved) {
            return saved;
        }
        assert!(Instant::now() < deadline, "The outcome was not saved: {saved:?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn member(email: &str, id: char) -> Member {
    Member { email: email.into(), name: String::new(), user_id: format!("{0}{0}{0}{0}{0}{0}{0}{0}-0000-4000-8000-00000000000{0}", id) }
}

fn next_ai(commands: &mut tokio::sync::mpsc::Receiver<ai::Command>, wanted: fn(&ai::Command) -> bool) -> ai::Command {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match commands.try_recv() {
            Ok(command) if wanted(&command) => return command,
            Ok(_) => {}
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
        assert!(Instant::now() < deadline, "The expected AI command was not dispatched");
    }
}

fn wait_for(worker: &Worker, ready: impl Fn(&Activity) -> bool) -> Arc<Activity> {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let snapshot = worker.activity.lock().unwrap().clone();
        if ready(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "The proposal state was not published");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn created(part: &tomlook::event_creation::Part, id: &str) -> Event {
    let mut event = calendar::fixture(1).remove(0);
    event.id = id.into();
    event.title = part.title.clone();
    event.start = part.start;
    event.end = part.end;
    event.attendees.clear();
    event.status = "organizer".into();
    event.is_all_day = false;
    event.extra.insert("isOrganizer".into(), serde_json::json!(true));
    event.extra.insert("showAs".into(), serde_json::json!("busy"));
    event.extra.insert("type".into(), serde_json::json!("singleInstance"));
    event
}

#[test]
fn blockers_teams_proposal_and_release_are_single_use_worker_approvals_with_persisted_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let today = Local::now().date_naive();
    let coverage = (0..30).map(|offset| today + Duration::days(offset)).collect();
    Store::open(root.path())
        .unwrap()
        .save_workiq_calendar(&Calendar::build(Vec::new(), coverage, Vec::new()).unwrap(), ACCOUNT)
        .unwrap();
    let mut worker = Worker::start(
        Options { state: root.path().into(), legacy_root: root.path().join("no-legacy"), demo: None },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(64);
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: Arc::new(AtomicBool::new(false)),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    let start = (today + Duration::days(3)).and_hms_opt(10, 0, 0).unwrap()
        .and_local_timezone(Local).earliest().unwrap().fixed_offset();
    let slots = vec![
        Slot { start, end: start + Duration::minutes(30) },
        Slot { start: start + Duration::hours(2), end: start + Duration::hours(2) + Duration::minutes(30) },
    ];
    let mut attendees = tomlook::attendees::parse_input("Alex Rivers <alex@example.com>; jamie@example.com").unwrap();
    attendees[1].role = tomlook::attendees::Role::Optional;
    let proposal = Proposal::new(ACCOUNT, "Pilot kickoff", attendees, 30, slots, tomlook::i18n::Language::En,
        Local::now().fixed_offset()).unwrap();

    worker.commands.send(Command::HoldTimes {
        account: ACCOUNT.into(), proposal: Box::new(proposal.clone()), private: false, category: None,
    }).unwrap();
    let ai::Command::CreateEvents { record, reply, wake } = next_ai(&mut commands, |command| matches!(command, ai::Command::CreateEvents { .. })) else {
        unreachable!()
    };
    assert_eq!(record.hold.as_ref().unwrap().proposal, proposal.id);
    let parts = record.parts();
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().all(|part| part.title == "BLOCKER: Pilot kickoff" && part.attendees.is_empty() && !part.teams));
    let snapshot = wait_for(&worker, |activity| activity.proposals.get(&proposal.id)
        .is_some_and(|saved| saved.blockers.iter().all(|blocker| blocker.state == BlockerState::Creating)));
    assert_eq!(snapshot.proposals[&proposal.id].blockers.len(), 2, "The approval is saved before anything is written");
    worker.commands.send(Command::HoldTimes {
        account: ACCOUNT.into(), proposal: Box::new(proposal.clone()), private: false, category: None,
    }).unwrap();
    let events = vec![created(&parts[0], "blocker-a"), created(&parts[1], "blocker-b")];
    reply.blocking_send(CreationResult { id: record.id.clone(), state: State::Created, events, message: "Verified".into() }).unwrap();
    let _ = wake.send(Command::Wake);
    let snapshot = wait_for(&worker, |activity| activity.proposals.get(&proposal.id)
        .is_some_and(|saved| saved.held().count() == 2));
    assert_eq!(snapshot.blockers.get("blocker-a"), Some(&proposal.id));
    assert_eq!(snapshot.blockers.get("blocker-b"), Some(&proposal.id));
    assert_eq!(snapshot.event_creations.values().filter(|record| record.hold.is_some()).count(), 1,
        "A second BLOCKER request for the same proposal is never approved");

    let member = |email: &str, id: char| Member {
        email: email.into(), name: String::new(), user_id: format!("{0}{0}{0}{0}{0}{0}{0}{0}-0000-4000-8000-00000000000{0}", id),
    };
    let draft = ChatDraft {
        members: vec![member("alex@example.com", 'a'), member("jamie@example.com", 'b')],
        topic: Some("Pilot kickoff".into()),
        message: "Which time works?".into(),
    };
    worker.commands.send(Command::ProposeChat {
        account: ACCOUNT.into(), proposal: Box::new(proposal.clone()), draft: Box::new(draft.clone()),
    }).unwrap();
    let ai::Command::ProposeChat { request, reply, wake } = next_ai(&mut commands, |command| matches!(command, ai::Command::ProposeChat { .. })) else {
        unreachable!()
    };
    assert_eq!((request.account.as_str(), &request.draft), (ACCOUNT, &draft));
    wait_for(&worker, |activity| activity.proposals[&proposal.id].chat.as_ref().is_some_and(|chat| chat.state == ChatState::Sending));
    worker.commands.send(Command::ProposeChat {
        account: ACCOUNT.into(), proposal: Box::new(proposal.clone()), draft: Box::new(draft.clone()),
    }).unwrap();
    reply.blocking_send(ChatResult {
        proposal: proposal.id.clone(), state: ChatState::Sent, chat_id: Some("19:abc@thread.v2".into()),
        web_url: Some("https://teams.microsoft.com/l/chat/19%3Aabc".into()), message_id: Some("1".into()),
        message: "Read back".into(),
    }).unwrap();
    let _ = wake.send(Command::Wake);
    wait_for(&worker, |activity| activity.proposals[&proposal.id].chat.as_ref().is_some_and(|chat| chat.state == ChatState::Sent));

    worker.commands.send(Command::ReleaseBlockers {
        account: ACCOUNT.into(), proposal: proposal.id.clone(), events: vec!["blocker-a".into(), "blocker-b".into()], cancel: true,
    }).unwrap();
    let ai::Command::ReleaseBlockers { request, reply, wake } = next_ai(&mut commands, |command| matches!(command, ai::Command::ReleaseBlockers { .. })) else {
        unreachable!()
    };
    assert_eq!(request.blockers.iter().map(|blocker| blocker.transaction_id.clone()).collect::<Vec<_>>(),
        parts.iter().map(|part| part.transaction_id.clone()).collect::<Vec<_>>(), "Matched by the creation transaction IDs");
    wait_for(&worker, |activity| activity.proposals[&proposal.id].blockers.iter().all(|blocker| blocker.state == BlockerState::Releasing));
    reply.blocking_send(ReleaseResult { proposal: proposal.id.clone(), outcomes: vec![
        ReleaseOutcome { event_id: "blocker-a".into(), state: BlockerState::Released, message: "Deleted".into() },
        ReleaseOutcome { event_id: "blocker-b".into(), state: BlockerState::ReleaseReview, message: "Uncertain".into() },
    ] }).unwrap();
    let _ = wake.send(Command::Wake);
    let snapshot = wait_for(&worker, |activity| activity.proposals[&proposal.id].status == ProposalStatus::Cancelled);
    assert_eq!(snapshot.proposals[&proposal.id].blockers[1].state, BlockerState::ReleaseReview, "Never retried");
    assert!(!snapshot.blockers.contains_key("blocker-a"));
    let mut seen = Vec::new();
    while let Ok(command) = commands.try_recv() {
        seen.push(command);
    }
    assert!(!seen.iter().any(|command| matches!(command, ai::Command::CreateEvents { .. }
        | ai::Command::ProposeChat { .. } | ai::Command::ReleaseBlockers { .. })), "Repeated requests were rejected, not dispatched");

    worker.commands.send(Command::Exit).unwrap();
    worker.done.take().unwrap().recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
    let store = Store::open(root.path()).unwrap();
    let saved = store.proposals(ACCOUNT).unwrap().remove(&proposal.id).unwrap();
    assert_eq!(saved.status, ProposalStatus::Cancelled);
    assert_eq!(saved.chat.unwrap().chat_id.as_deref(), Some("19:abc@thread.v2"));
    assert!(store.calendar().unwrap().events.iter().all(|event| event.id != "blocker-a"), "Released BLOCKERs leave the cache");
    assert!(store.calendar().unwrap().events.iter().any(|event| event.id == "blocker-b"));
}

#[test]
fn saved_proposals_are_account_bound_and_in_flight_outcomes_become_uncertain_after_a_restart() {
    let root = tempfile::tempdir().unwrap();
    let start = (Local::now() + Duration::days(2)).fixed_offset();
    let proposal = Proposal::new(ACCOUNT, "Pilot kickoff", tomlook::attendees::parse_input("alex@example.com").unwrap(), 30,
        vec![Slot { start, end: start + Duration::minutes(30) }], tomlook::i18n::Language::Cs, Local::now().fixed_offset()).unwrap();
    let mut flying = proposal.clone();
    let draft = flying.hold_draft(false, None).unwrap();
    let record = tomlook::event_creation::Record::blockers(ACCOUNT.into(), flying.id.clone(), draft, flying.candidates.clone()).unwrap();
    flying.begin_hold(&record, flying.created_at).unwrap();
    {
        let store = Store::open(root.path()).unwrap();
        store.save_proposal(&flying).unwrap();
        let mut other = flying.clone();
        other.account = "other@example.com".into();
        assert!(store.save_proposal(&other).is_err(), "A proposal never changes its account");
    }
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.proposal(ACCOUNT, &proposal.id).unwrap().as_ref(), Some(&flying), "A single read never recovers");
    assert!(store.proposal("other@example.com", &proposal.id).unwrap().is_none());
    assert_eq!(store.proposals_except(ACCOUNT, |saved| saved.id == proposal.id).unwrap()[&proposal.id], flying,
        "A write still in flight in this process is kept");
    let saved = store.proposals(ACCOUNT).unwrap();
    assert_eq!(saved[&proposal.id].blockers[0].state, BlockerState::Review, "Never replayed after a restart");
    assert!(store.proposals("other@example.com").unwrap().is_empty());
    assert_eq!(Store::open(root.path()).unwrap().proposals(ACCOUNT).unwrap()[&proposal.id], saved[&proposal.id], "The recovery is persisted");
}

#[test]
fn outcomes_of_writes_in_flight_are_saved_when_the_calendar_account_changes_meanwhile() {
    use tomlook::i18n::Language::En;
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    own_calendar(&mut store, ACCOUNT);
    let mut worker = Worker::start(
        Options { state: root.path().into(), legacy_root: root.path().join("no-legacy"), demo: None },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let (sender, mut commands) = tokio::sync::mpsc::channel(64);
    let terminated = Arc::new(AtomicBool::new(false));
    worker
        .commands
        .send(Command::AttachAi {
            commands: sender,
            ready: Arc::new(AtomicBool::new(true)),
            occupied: Arc::new(AtomicUsize::new(0)),
            terminated: terminated.clone(),
            analysis_revision: Arc::new(std::sync::OnceLock::new()),
            workiq_ready: Arc::new(AtomicBool::new(true)),
        })
        .unwrap();
    let now = Local::now().fixed_offset();
    let start = (Local::now().date_naive() + Duration::days(3)).and_hms_opt(9, 0, 0).unwrap()
        .and_local_timezone(Local).earliest().unwrap().fixed_offset();
    let slot = |hours: i64| Slot { start: start + Duration::hours(hours), end: start + Duration::hours(hours) + Duration::minutes(30) };
    let people = || tomlook::attendees::parse_input("Alex Rivers <alex@example.com>; jamie@example.com").unwrap();
    let held = Proposal::new(ACCOUNT, "Pilot kickoff", people(), 30, vec![slot(0), slot(2)], En, now).unwrap();
    let chatted = Proposal::new(ACCOUNT, "Budget review", people(), 30, vec![slot(4)], En, now).unwrap();
    let interrupted = Proposal::new(ACCOUNT, "Roadmap", people(), 30, vec![slot(6)], En, now).unwrap();
    let draft = ChatDraft { members: vec![member("alex@example.com", 'a')], topic: None, message: "Which time works?".into() };

    // A BLOCKER creation and a Teams send are in flight when another account is signed in.
    worker.commands.send(Command::HoldTimes {
        account: ACCOUNT.into(), proposal: Box::new(held.clone()), private: false, category: None,
    }).unwrap();
    let ai::Command::CreateEvents { record, reply: hold_reply, wake: hold_wake } =
        next_ai(&mut commands, |command| matches!(command, ai::Command::CreateEvents { .. })) else { unreachable!() };
    wait_for(&worker, |activity| activity.proposals.get(&held.id)
        .is_some_and(|saved| saved.blockers.iter().all(|blocker| blocker.state == BlockerState::Creating)));
    worker.commands.send(Command::ProposeChat {
        account: ACCOUNT.into(), proposal: Box::new(chatted.clone()), draft: Box::new(draft.clone()),
    }).unwrap();
    let ai::Command::ProposeChat { reply: chat_reply, wake: chat_wake, .. } =
        next_ai(&mut commands, |command| matches!(command, ai::Command::ProposeChat { .. })) else { unreachable!() };
    wait_for(&worker, |activity| activity.proposals.get(&chatted.id)
        .and_then(|saved| saved.chat.as_ref()).is_some_and(|chat| chat.state == ChatState::Sending));
    assert!(switch_account(&mut store, &worker, OTHER).proposals.is_empty());

    // The Teams outcome arrives while the other account is shown.
    chat_reply.blocking_send(ChatResult {
        proposal: chatted.id.clone(), state: ChatState::Sent, chat_id: Some("19:budget@thread.v2".into()),
        web_url: Some("https://teams.microsoft.com/l/chat/19%3Abudget".into()), message_id: Some("7".into()),
        message: "Read back".into(),
    }).unwrap();
    let _ = chat_wake.send(Command::Wake);
    let sent = saved(&store, ACCOUNT, &chatted.id, |saved| saved.chat.as_ref().is_some_and(|chat| chat.state != ChatState::Sending));
    let chat = sent.chat.unwrap();
    assert_eq!(chat.state, ChatState::Sent, "{}", chat.message);
    assert_eq!(chat.chat_id.as_deref(), Some("19:budget@thread.v2"));
    assert_eq!(chat.web_url.as_deref(), Some("https://teams.microsoft.com/l/chat/19%3Abudget"));
    assert!(store.proposal(ACCOUNT, &held.id).unwrap().unwrap().blockers.iter().all(|blocker| blocker.state == BlockerState::Creating),
        "The BLOCKER creation still in flight is not mistaken for an interrupted one");

    // Back on the first account while the BLOCKERs are still being created.
    let snapshot = switch_account(&mut store, &worker, ACCOUNT);
    assert!(snapshot.proposals[&held.id].blockers.iter().all(|blocker| blocker.state == BlockerState::Creating),
        "Loading the account again does not interrupt a write in flight");
    assert_eq!(snapshot.proposals[&chatted.id].chat.as_ref().unwrap().chat_id.as_deref(), Some("19:budget@thread.v2"));
    let parts = record.parts();
    hold_reply.blocking_send(CreationResult {
        id: record.id.clone(), state: State::Created,
        events: vec![created(&parts[0], "blocker-a"), created(&parts[1], "blocker-b")], message: "Verified".into(),
    }).unwrap();
    let _ = hold_wake.send(Command::Wake);
    let snapshot = wait_for(&worker, |activity| activity.proposals.get(&held.id).is_some_and(|saved| saved.held().count() == 2));
    assert_eq!(snapshot.blockers.get("blocker-a"), Some(&held.id));
    assert_eq!(snapshot.blockers.get("blocker-b"), Some(&held.id));
    let persisted = store.proposal(ACCOUNT, &held.id).unwrap().unwrap();
    assert_eq!(persisted.blockers.iter().map(|blocker| (blocker.state, blocker.event_id.as_deref())).collect::<Vec<_>>(),
        [(BlockerState::Held, Some("blocker-a")), (BlockerState::Held, Some("blocker-b"))], "Verified BLOCKERs keep their event IDs");

    // A deletion is in flight when another account is signed in; its outcome arrives meanwhile.
    worker.commands.send(Command::ReleaseBlockers {
        account: ACCOUNT.into(), proposal: held.id.clone(), events: vec!["blocker-a".into(), "blocker-b".into()], cancel: true,
    }).unwrap();
    let ai::Command::ReleaseBlockers { reply, wake, .. } =
        next_ai(&mut commands, |command| matches!(command, ai::Command::ReleaseBlockers { .. })) else { unreachable!() };
    wait_for(&worker, |activity| activity.proposals[&held.id].blockers.iter().all(|blocker| blocker.state == BlockerState::Releasing));
    switch_account(&mut store, &worker, OTHER);
    reply.blocking_send(ReleaseResult { proposal: held.id.clone(), outcomes: vec![
        ReleaseOutcome { event_id: "blocker-a".into(), state: BlockerState::Released, message: "Deleted".into() },
        ReleaseOutcome { event_id: "blocker-b".into(), state: BlockerState::Released, message: "Deleted".into() },
    ] }).unwrap();
    let _ = wake.send(Command::Wake);
    let released = saved(&store, ACCOUNT, &held.id, |saved| !saved.in_flight());
    assert!(released.blockers.iter().all(|blocker| blocker.state == BlockerState::Released), "{:?}", released.blockers);
    assert_eq!(released.status, ProposalStatus::Cancelled);
    let snapshot = switch_account(&mut store, &worker, ACCOUNT);
    assert_eq!(snapshot.proposals[&held.id], released);
    assert!(snapshot.blockers.is_empty(), "Nothing is held any more");

    // A Teams send whose executor ends while another account is shown stays uncertain.
    worker.commands.send(Command::ProposeChat {
        account: ACCOUNT.into(), proposal: Box::new(interrupted.clone()), draft: Box::new(draft),
    }).unwrap();
    let _unanswered = next_ai(&mut commands, |command| matches!(command, ai::Command::ProposeChat { .. }));
    wait_for(&worker, |activity| activity.proposals.get(&interrupted.id)
        .and_then(|saved| saved.chat.as_ref()).is_some_and(|chat| chat.state == ChatState::Sending));
    switch_account(&mut store, &worker, OTHER);
    terminated.store(true, std::sync::atomic::Ordering::Release);
    worker.commands.send(Command::Wake).unwrap();
    own_calendar(&mut store, ACCOUNT);
    worker.commands.send(Command::Wake).unwrap();
    let snapshot = wait_for(&worker, |activity| activity.task_account.as_deref() == Some(ACCOUNT)
        && activity.proposals.get(&interrupted.id).and_then(|saved| saved.chat.as_ref()).is_some_and(|chat| chat.state != ChatState::Sending));
    let chat = snapshot.proposals[&interrupted.id].chat.clone().unwrap();
    assert_eq!((chat.state, chat.message.as_str()), (ChatState::Review, tomlook::scheduling::UNCERTAIN_CHAT));

    worker.commands.send(Command::Exit).unwrap();
    worker.done.take().unwrap().recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
    let store = Store::open(root.path()).unwrap();
    let saved = store.proposals(ACCOUNT).unwrap();
    assert_eq!(saved[&held.id], released);
    assert_eq!(saved[&chatted.id].chat.as_ref().unwrap().web_url.as_deref(), Some("https://teams.microsoft.com/l/chat/19%3Abudget"));
    assert_eq!(saved[&interrupted.id].chat.as_ref().unwrap().state, ChatState::Review);
}

#[test]
fn writes_in_flight_in_an_earlier_run_become_uncertain_when_the_worker_starts_again() {
    use tomlook::{event_creation::Record, i18n::Language::En};
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path()).unwrap();
    own_calendar(&mut store, ACCOUNT);
    let now = Local::now().fixed_offset();
    let start = (Local::now() + Duration::days(2)).fixed_offset();
    let slot = |hours: i64| Slot { start: start + Duration::hours(hours), end: start + Duration::hours(hours) + Duration::minutes(30) };
    let people = || tomlook::attendees::parse_input("Alex Rivers <alex@example.com>; jamie@example.com").unwrap();
    let draft = ChatDraft { members: vec![member("alex@example.com", 'a')], topic: None, message: "Which time works?".into() };
    // The earlier run saved its approvals and stopped before any outcome came back.
    let mut creating = Proposal::new(ACCOUNT, "Pilot kickoff", people(), 30, vec![slot(0), slot(2)], En, now).unwrap();
    let record = Record::blockers(ACCOUNT.into(), creating.id.clone(), creating.hold_draft(false, None).unwrap(),
        creating.candidates.clone()).unwrap();
    creating.begin_hold(&record, now).unwrap();
    store.approve_event_creation(&record).unwrap();
    store.save_proposal(&creating).unwrap();
    let mut sending = Proposal::new(ACCOUNT, "Budget review", people(), 30, vec![slot(4)], En, now).unwrap();
    sending.begin_chat(draft.clone(), now).unwrap();
    store.save_proposal(&sending).unwrap();
    let mut releasing = Proposal::new(ACCOUNT, "Roadmap", people(), 30, vec![slot(6)], En, now).unwrap();
    let hold = Record::blockers(ACCOUNT.into(), releasing.id.clone(), releasing.hold_draft(false, None).unwrap(),
        releasing.candidates.clone()).unwrap();
    releasing.begin_hold(&hold, now).unwrap();
    releasing.finish_hold(State::Created, &["blocker-r".into()], "Verified", now);
    releasing.begin_release(&["blocker-r".into()], now).unwrap();
    store.save_proposal(&releasing).unwrap();
    let mut other = Proposal::new(OTHER, "Budget review", people(), 30, vec![slot(4)], En, now).unwrap();
    other.begin_chat(draft, now).unwrap();
    store.save_proposal(&other).unwrap();

    let mut worker = Worker::start(
        Options { state: root.path().into(), legacy_root: root.path().join("no-legacy"), demo: None },
        eframe::egui::Context::default(),
    )
    .unwrap();
    let snapshot = wait_for(&worker, |activity| activity.task_account.as_deref() == Some(ACCOUNT) && activity.proposals.len() == 3);
    let blockers = |id: &str| snapshot.proposals[id].blockers.iter().map(|blocker| (blocker.state, blocker.message.clone())).collect::<Vec<_>>();
    let uncertain = tomlook::workiq::connection::calendar_write::UNCERTAIN.to_owned();
    assert_eq!(blockers(&creating.id), [(BlockerState::Review, uncertain.clone()), (BlockerState::Review, uncertain)], "Never replayed after a restart");
    assert_eq!(snapshot.event_creations[&record.id].state, State::Review);
    let chat = snapshot.proposals[&sending.id].chat.clone().unwrap();
    assert_eq!((chat.state, chat.message.as_str()), (ChatState::Review, tomlook::scheduling::UNCERTAIN_CHAT));
    assert_eq!(blockers(&releasing.id), [(BlockerState::ReleaseReview, tomlook::scheduling::UNCERTAIN_RELEASE.to_owned())]);
    for id in [&creating.id, &sending.id, &releasing.id] {
        assert_eq!(store.proposal(ACCOUNT, id).unwrap().as_ref(), Some(&snapshot.proposals[id]), "The recovery is persisted");
    }
    // Another account's write of the earlier run is uncertain too once that account is shown.
    let snapshot = switch_account(&mut store, &worker, OTHER);
    assert_eq!(snapshot.proposals[&other.id].chat.as_ref().unwrap().state, ChatState::Review);
    assert_eq!(store.proposal(OTHER, &other.id).unwrap().unwrap().chat.unwrap().state, ChatState::Review);
    worker.commands.send(Command::Exit).unwrap();
    worker.done.take().unwrap().recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
}
