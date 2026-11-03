use chrono::{DateTime, Duration, Utc};
use std::{collections::{BTreeMap, BTreeSet}, sync::Arc};
use tomlook::{
    calendar::{Calendar, Event},
    customers,
    invitations::{self, Book, Conflicts, Group, LocalAction, OrganizerRule, Reason, Tab},
    storage::Store,
};

const ACCOUNT: &str = "me@internal.example";

fn now() -> DateTime<Utc> { "2026-11-02T08:00:00Z".parse().unwrap() }

fn event(id: &str) -> Event {
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.id = id.into();
    event.title = "Planning".into();
    event.start = (now() + Duration::days(1)).fixed_offset();
    event.end = event.start + Duration::hours(1);
    event.organizer = "Organizer <organizer@internal.example>".into();
    event.attendees = vec![ACCOUNT.into()];
    event.category.clear();
    event.location = "Microsoft Teams Meeting".into();
    event.status = "notResponded".into();
    event.is_all_day = false;
    event.source_url = None;
    event.extra = BTreeMap::from([
        ("isOrganizer".into(), serde_json::json!(false)),
        ("isCancelled".into(), serde_json::json!(false)),
        ("type".into(), serde_json::json!("singleInstance")),
    ]);
    event
}

fn group() -> Group {
    Group { id: "00000000-0000-4000-8000-000000000001".into(), name: "Regional team".into(),
        mail: "team@internal.example".into(), enabled: true,
        members: BTreeSet::from(["organizer@internal.example".into(), "alias@internal.example".into()]),
        refreshed_at: Some(now()), error: None }
}

fn view(book: &Book, customers: &customers::Book) -> invitations::Snapshot {
    invitations::snapshot(Arc::new(book.clone()), &Calendar::default(), customers,
        None, &Default::default(), Default::default(), Default::default(), now())
}

fn classify(event: Event, team: bool) -> invitations::Row {
    let mut book = Book::new(ACCOUNT);
    if team { book.merge_groups(vec![group()], true).unwrap(); }
    book.replace(vec![event], now()).unwrap();
    view(&book, &Default::default()).rows.remove(0)
}

#[test]
fn invitation_eligibility_excludes_answered_owned_cancelled_followed_and_expired_events() {
    let source = event("pending");
    for (status, expected) in [("notResponded", true), ("none", true), ("accepted", false),
        ("organizer", false), ("declined", false), ("tentativelyAccepted", false)] {
        let mut event = source.clone();
        event.status = status.into();
        assert_eq!(invitations::pending(&event, now()), expected, "{status}");
    }
    for (field, value) in [("isOrganizer", serde_json::json!(true)),
        ("isCancelled", serde_json::json!(true)), ("isFollowing", serde_json::json!(true)),
        ("type", serde_json::json!("seriesMaster")), ("categories", serde_json::json!(["Follow"]))] {
        let mut event = source.clone();
        event.extra.insert(field.into(), value);
        assert!(!invitations::pending(&event, now()), "{field}");
    }
    let mut unknown_owner = source.clone();
    unknown_owner.extra.remove("isOrganizer");
    assert!(!invitations::pending(&unknown_owner, now()));
    assert!(!invitations::pending(&source, now() + Duration::days(2)));
}

#[test]
fn invitation_policy_prioritizes_verified_team_aliases_and_external_signals_without_claiming_customers() {
    let mut source = event("one");
    assert_eq!(classify(source.clone(), false).tab, Tab::Other);
    assert_eq!(classify(source.clone(), true).reason, Reason::Team);
    source.organizer = "Alias <alias@internal.example>".into();
    assert_eq!(classify(source.clone(), true).reason, Reason::Team);
    source.organizer = "Someone <someone@unknown.example>".into();
    let row = classify(source.clone(), false);
    assert_eq!((row.tab, row.reason), (Tab::Important, Reason::External));
    assert!(row.detail.is_empty());
    source.organizer = "Organizer <organizer@internal.example>".into();
    source.attendees.push("guest@unknown.example".into());
    assert_eq!(classify(source.clone(), false).reason, Reason::ExternalParticipant);
    source.extra.insert("tomlook_attendee_roles".into(), serde_json::json!({"guest@unknown.example":"resource"}));
    assert_eq!(classify(source, false).tab, Tab::Other);
}

#[test]
fn invitation_policy_keeps_broadcast_noise_out_but_never_hides_a_confirmed_customer_or_explicit_pin() {
    let mut source = event("broadcast");
    source.attendees.push("guest@customer.example".into());
    for title in ["Company All-Hands", "Office hours", "Regional webinar", "Optional enablement"] {
        source.title = title.into();
        assert_eq!(classify(source.clone(), true).reason, Reason::Broadcast, "{title}");
    }
    source.title = "Customer training".into();
    assert_eq!(classify(source.clone(), true).reason, Reason::Team, "Training alone is not noise");
    source.extra.insert("tomlook_attendees_total".into(), serde_json::json!(50));
    assert_eq!(classify(source.clone(), true).reason, Reason::Team);
    source.extra.insert("tomlook_attendees_total".into(), serde_json::json!(51));
    assert_eq!(classify(source.clone(), true).reason, Reason::Broadcast);
    let customer = customers::Customer::new(ACCOUNT, "Confirmed customer",
        &["customer.example".into()], None, now()).unwrap();
    let customers = customers::Book { account: Some(ACCOUNT.into()),
        customers: BTreeMap::from([(customer.id.clone(), customer)]), ..Default::default() };
    let mut book = Book::new(ACCOUNT);
    book.replace(vec![source.clone()], now()).unwrap();
    let customer_row = view(&book, &customers).rows.remove(0);
    assert_eq!(customer_row.reason, Reason::Customer);
    assert_eq!(customer_row.detail, "Confirmed customer");
    book.apply(LocalAction::Organizer { email: "organizer@internal.example".into(), rule: Some(OrganizerRule::Quiet) }, now()).unwrap();
    assert_eq!(view(&book, &customers).rows[0].reason, Reason::Quiet, "Explicit quiet wins even over customer relevance");
    book.apply(LocalAction::Organizer { email: "organizer@internal.example".into(), rule: Some(OrganizerRule::Important) }, now()).unwrap();
    assert_eq!(view(&book, &Default::default()).rows[0].reason, Reason::Pinned);
}

#[test]
fn invitation_own_optional_role_and_explicit_noncustomer_or_team_exclusions_are_respected() {
    let mut source = event("optional");
    source.extra.insert("tomlook_attendee_roles".into(), serde_json::json!({ACCOUNT:"optional"}));
    assert_eq!(classify(source.clone(), true).reason, Reason::Broadcast);
    source = event("excluded");
    let mut book = Book::new(ACCOUNT);
    book.merge_groups(vec![group()], true).unwrap();
    book.apply(LocalAction::People { included: Default::default(),
        excluded: BTreeSet::from(["organizer@internal.example".into()]) }, now()).unwrap();
    book.replace(vec![source.clone()], now()).unwrap();
    assert_eq!(view(&book, &Default::default()).rows[0].tab, Tab::Other);
    source.organizer = "Someone <someone@unknown.example>".into();
    book.replace(vec![source.clone()], now()).unwrap();
    let exclusion = customers::Override::new(ACCOUNT, &source, None, now());
    let customers = customers::Book {
        overrides: BTreeMap::from([(exclusion.occurrence.clone(), exclusion)]),
        ..Default::default()
    };
    assert_eq!(view(&book, &customers).rows[0].tab, Tab::Other);
}

#[test]
fn invitation_series_collapse_to_first_pending_occurrence_and_material_updates_cancel_snoozes() {
    let mut first = event("occurrence-one");
    first.extra.insert("seriesMasterId".into(), serde_json::json!("series"));
    first.extra.insert("type".into(), serde_json::json!("occurrence"));
    let mut second = first.clone();
    second.id = "occurrence-two".into();
    second.start += Duration::days(7);
    second.end += Duration::days(7);
    let id = invitations::key(&first);
    assert_eq!(id, invitations::key(&second));
    let mut book = Book::new(ACCOUNT);
    book.replace(vec![second.clone(), first.clone()], now()).unwrap();
    assert_eq!(book.records.len(), 1);
    assert_eq!(book.records[&id].event.id, first.id);
    book.apply(LocalAction::Snooze { key: id.clone(), until: now() + Duration::days(1) }, now()).unwrap();
    assert_eq!(view(&book, &Default::default()).rows[0].tab, Tab::Snoozed);
    first.category = "Call".into();
    first.extra.insert("categories".into(), serde_json::json!(["Call"]));
    book.replace(vec![first.clone()], now() + Duration::minutes(1)).unwrap();
    assert!(book.snoozed.contains_key(&id), "Category changes do not undo snoozes");
    first.title = "Updated planning".into();
    book.replace(vec![first.clone()], now() + Duration::minutes(2)).unwrap();
    assert!(book.records[&id].updated);
    assert!(book.snoozed.is_empty());
    first.status = "accepted".into();
    book.replace(vec![first, second.clone()], now() + Duration::minutes(3)).unwrap();
    assert_eq!(book.records[&id].event.id, second.id, "The next occurrence may genuinely still be pending");
    book.replace(vec![], now()).unwrap();
    assert!(view(&book, &Default::default()).rows.is_empty());
}

#[test]
fn invitation_group_discovery_and_failures_preserve_complete_membership_and_user_choices() {
    let mut book = Book::new(ACCOUNT);
    let source = group();
    book.merge_groups(vec![source.clone()], true).unwrap();
    let mut rediscovered = source.clone();
    rediscovered.enabled = false;
    rediscovered.members.clear();
    rediscovered.refreshed_at = None;
    book.merge_groups(vec![rediscovered.clone()], true).unwrap();
    assert!(book.groups[&source.id].enabled);
    assert_eq!(book.groups[&source.id].members, source.members);
    rediscovered.error = Some("Directory unavailable".into());
    book.merge_groups(vec![rediscovered], false).unwrap();
    assert_eq!(book.groups[&source.id].members, source.members);
    assert_eq!(book.groups[&source.id].refreshed_at, source.refreshed_at);
    assert!(book.groups[&source.id].error.is_some());
    book.apply(LocalAction::GroupEnabled { id: source.id.clone(), enabled: false }, now()).unwrap();
    assert!(book.team().is_empty());
    book.apply(LocalAction::RemoveGroup(source.id), now()).unwrap();
    book.merge_groups(vec![group()], false).unwrap();
    assert!(book.groups.is_empty(), "A late membership result cannot resurrect a removed group");
}

#[test]
fn invitation_conflicts_require_complete_recent_coverage_and_never_treat_filtered_reads_as_free_time() {
    let pending = event("pending");
    let mut busy = event("busy");
    busy.status = "accepted".into();
    let day = pending.calendar_dates().0;
    let mut book = Book::new(ACCOUNT);
    book.replace(vec![pending.clone()], now()).unwrap();
    for (coverage, stamp, expected) in [
        (false, now(), Conflicts::Unknown), (true, now() - Duration::minutes(16), Conflicts::Unknown),
        (true, now(), Conflicts::Found(1)),
    ] {
        let calendar = Calendar::build(vec![pending.clone(), busy.clone()],
            if coverage { BTreeSet::from([day]) } else { Default::default() }, vec![]).unwrap();
        let snapshot = invitations::snapshot(Arc::new(book.clone()), &calendar, &Default::default(), None,
            &BTreeMap::from([(day, stamp)]), Default::default(), Default::default(), now());
        assert_eq!(snapshot.rows[0].conflicts, expected);
    }
    let calendar = Calendar::build(vec![pending], BTreeSet::from([day]), vec![]).unwrap();
    let snapshot = invitations::snapshot(Arc::new(book), &calendar, &Default::default(), None,
        &BTreeMap::from([(day, now())]), Default::default(), Default::default(), now());
    assert_eq!(snapshot.rows[0].conflicts, Conflicts::Clear);
}

#[test]
fn invitation_storage_is_account_isolated_and_invalid_writes_preserve_the_last_good_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let mut book = Book::new(ACCOUNT);
    book.replace(vec![event("one")], now()).unwrap();
    book.merge_groups(vec![group()], true).unwrap();
    store.save_invitations(&book).unwrap();
    assert!(store.invitations("another@internal.example").unwrap().records.is_empty());
    let mut invalid = book.clone();
    invalid.people.insert("not an email".into());
    assert!(store.save_invitations(&invalid).is_err());
    invalid = book.clone();
    invalid.records.values_mut().next().unwrap().event.title = "x".repeat(invitations::PAYLOAD_LIMIT);
    assert!(store.save_invitations(&invalid).is_err());
    let reopened = Store::open(root.path()).unwrap().invitations(ACCOUNT).unwrap();
    assert_eq!(serde_json::to_value(&book).unwrap(), serde_json::to_value(reopened).unwrap());
}

#[test]
fn invitation_readers_reject_old_and_cross_account_results_and_expose_retry_backoff() {
    let mut reader = invitations::Reader::default();
    assert!(reader.due());
    reader.start(1, ACCOUNT);
    assert!(!reader.due());
    assert!(reader.accepts(1, ACCOUNT));
    assert!(!reader.accepts(1, "other@internal.example"));
    reader.finish(Err("Connection changed".into()), invitations::REFRESH_SECONDS);
    assert!(!reader.progress.busy);
    assert!(reader.progress.error.is_some());
    assert!(!reader.accepts(1, ACCOUNT));
    assert!(!reader.due());
    reader.start(2, ACCOUNT);
    reader.finish(Ok(()), invitations::REFRESH_SECONDS);
    assert!(reader.progress.checked_at.is_some());
    assert!(reader.progress.error.is_none());
}

#[test]
fn invitation_worker_reads_do_not_require_ai_and_opening_does_not_claim_complete_calendar_coverage() {
    use std::{sync::atomic::{AtomicBool, AtomicUsize}, time::{Duration as Wait, Instant}};
    use tomlook::{ai, worker::{Command, Options, Worker}, workiq::connection::Identity};
    let root = tempfile::tempdir().unwrap();
    let source = {
        let mut source = event("uncached-invitation");
        source.start = (Utc::now() + Duration::days(2)).fixed_offset();
        source.end = source.start + Duration::hours(1);
        source
    };
    let day = source.calendar_dates().0;
    let covered = day.pred_opt().unwrap();
    {
        let mut store = Store::open(root.path()).unwrap();
        store.save_workiq_calendar(&Calendar::build(vec![], BTreeSet::from([covered]), vec![]).unwrap(), ACCOUNT).unwrap();
        store.save_workiq_identity(&Identity { account: ACCOUNT.into(), mail: None, display_name: None }).unwrap();
        store.save_preferences(&tomlook::storage::Preferences { prepare_enabled: false, paused: true, ..Default::default() }).unwrap();
    }
    let mut worker = Worker::start(Options { state: root.path().into(), legacy_root: root.path().join("no-legacy"),
        demo: None }, eframe::egui::Context::default()).unwrap();
    let (sender, mut reads) = tokio::sync::mpsc::channel(64);
    worker.commands.send(Command::AttachAi { commands: sender, ready: Arc::new(AtomicBool::new(false)),
        occupied: Arc::new(AtomicUsize::new(0)), terminated: Arc::new(AtomicBool::new(false)),
        analysis_revision: Arc::new(std::sync::OnceLock::new()), workiq_ready: Arc::new(AtomicBool::new(true)) }).unwrap();
    let deadline = Instant::now() + Wait::from_secs(8);
    let id = loop {
        if let Ok(ai::Command::ReadInvitations { id, account, request: invitations::ReadKind::Pending { first, last } }) = reads.try_recv() {
            assert_eq!(account, ACCOUNT);
            assert!((180..=186).contains(&(last - first).num_days()));
            break id;
        }
        assert!(Instant::now() < deadline, "WorkIQ invitation reads were incorrectly gated on AI");
        std::thread::sleep(Wait::from_millis(5));
    };
    worker.commands.send(Command::InvitationResult { id, account: ACCOUNT.into(),
        result: Ok(invitations::ReadResult::Pending(vec![source.clone()])) }).unwrap();
    let key = loop {
        let snapshot = worker.activity.lock().unwrap().clone();
        if let Some(row) = snapshot.invitations.rows.first() { break row.key.clone(); }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Wait::from_millis(5));
    };
    worker.commands.send(Command::Invitation(invitations::Command::Open { account: ACCOUNT.into(), key })).unwrap();
    loop {
        let store = Store::read_only(root.path()).unwrap();
        let calendar = store.calendar().unwrap();
        if calendar.event_index(&source.id).is_some() {
            assert_eq!(calendar.coverage, BTreeSet::from([covered]));
            assert!(!calendar.coverage.contains(&day));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Wait::from_millis(5));
    }
    worker.commands.send(Command::Exit).unwrap();
    worker.done.take().unwrap().recv_timeout(Wait::from_secs(5)).unwrap().unwrap();
    assert_eq!(Store::open(root.path()).unwrap().invitations(ACCOUNT).unwrap().records.len(), 1);
}
