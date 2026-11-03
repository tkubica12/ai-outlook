use chrono::{Duration, Months, Utc};
use serde::Deserialize;
use std::{collections::BTreeSet, fs::OpenOptions, io::Write, path::PathBuf, sync::Arc, time::Instant};
use tomlook::{invitations::{self, LocalAction, ReadKind, ReadResult}, storage::Store, workiq::connection::{Connection, Range}};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamSelection {
    queries: Vec<String>,
    enabled_groups: BTreeSet<String>,
}

#[tokio::test]
#[ignore = "Requires authorized saved WorkIQ login, a private team-selection file and a new private output; provider reads only; local cache writes require an explicit flag"]
async fn saved_invitation_queue_and_selected_team_groups_are_complete() {
    let root = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_INSIGHTS_ROOT").unwrap());
    let selection = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_INVITATION_SELECTION").unwrap());
    let output = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_INVITATION_OUTPUT").unwrap());
    let save = std::env::var("TOMLOOK_LIVE_INVITATION_SAVE").as_deref() == Ok("1");
    assert!(root.is_absolute() && selection.is_absolute() && output.is_absolute() && !output.exists());
    let _ownership = tomlook::instance::acquire(&root)
        .expect("Close Tomlook before live invitation verification or local cache writes");
    let selection: TeamSelection = serde_json::from_slice(&std::fs::read(selection).unwrap()).unwrap();
    invitations::validate_queries(&selection.queries).unwrap();
    assert!(!selection.enabled_groups.is_empty() && selection.enabled_groups.len() <= invitations::GROUP_LIMIT);
    let (account, mut book, customers, identity) = {
        let store = Store::read_only(&root).unwrap();
        let account = store.calendar_owner().unwrap().expect("The live profile needs an owned calendar");
        let identity = store.workiq_identity().unwrap().expect("The live profile needs a saved verified identity");
        assert_eq!(identity.account, account);
        let book = store.invitations(&account).unwrap();
        let customers = store.customer_book(&account).unwrap();
        (account, book, customers, identity)
    };
    let started = Instant::now();
    let connection = Connection::connect_saved(&root, &account, false, |_| {}).await.unwrap();
    let result: Result<_, String> = async {
        let ReadResult::Discovered(discovered) = connection.read_invitations(ReadKind::Discover(selection.queries.clone())).await?
            else { return Err("Team discovery returned a different operation".into()); };
        let selected = discovered.iter().filter(|group| selection.enabled_groups.contains(&group.id))
            .cloned().collect::<Vec<_>>();
        if selected.len() != selection.enabled_groups.len() {
            return Err("Not all explicitly selected groups were rediscovered in the bounded historical-mail sample".into());
        }
        let ReadResult::Teams(refreshed) = connection.read_invitations(ReadKind::Teams(selected)).await?
            else { return Err("Team membership returned a different operation".into()); };
        if refreshed.iter().any(|group| group.error.is_some() || group.refreshed_at.is_none()) {
            return Err("A selected team membership read was incomplete; the saved cache was not changed".into());
        }
        let first = chrono::Local::now().date_naive();
        let last = first.checked_add_months(Months::new(6)).ok_or("Date range overflow")?;
        let ReadResult::Pending(events) = connection.read_invitations(ReadKind::Pending { first, last }).await?
            else { return Err("Invitation read returned a different operation".into()); };
        let observed_at = Utc::now();
        let ids = events.iter().filter(|event| invitations::pending(event, observed_at))
            .map(|event| event.id.as_str()).collect::<BTreeSet<_>>();
        let range = Range::new(first, first + Duration::days(13))?;
        let comparison = connection.calendar(range).await?;
        if comparison.events.iter().filter(|event| invitations::pending(event, observed_at))
            .any(|event| !ids.contains(event.id.as_str())) {
            return Err("Independent full-calendar comparison found an unanswered occurrence missing from the queue read".into());
        }
        let occurrences = ids.len();
        book.merge_groups(discovered, true)?;
        book.merge_groups(refreshed, false)?;
        for id in &selection.enabled_groups {
            book.apply(LocalAction::GroupEnabled { id: id.clone(), enabled: true }, observed_at)?;
        }
        book.discovery_queries = selection.queries;
        book.replace(events, observed_at)?;
        book.validate()?;
        Ok((first, last, observed_at, occurrences))
    }.await;
    let cleanup = connection.close().await;
    let mut saved = false;
    if result.is_ok() && cleanup.is_ok() && save {
        let store = Store::open(&root).unwrap();
        assert_eq!(store.calendar_owner().unwrap().as_deref(), Some(account.as_str()), "Account changed during the live read");
        store.save_invitations(&book).unwrap();
        assert_eq!(serde_json::to_value(store.invitations(&account).unwrap()).unwrap(), serde_json::to_value(&book).unwrap());
        saved = true;
    }
    let snapshot = invitations::snapshot(Arc::new(book.clone()), &Default::default(), &customers,
        Some(&identity), &Default::default(), Default::default(), Default::default(), Utc::now());
    let evidence = serde_json::json!({
        "observed_at":Utc::now(), "elapsed_ms":started.elapsed().as_millis(),
        "result":match &result {
            Ok((first,last,checked,occurrences)) => serde_json::json!({
                "first":first,"last":last,"checked_at":checked,"unanswered_occurrences":occurrences,
                "queue_rows":snapshot.rows.len(),"important":snapshot.important(),
                "selected_groups":selection.enabled_groups.len(),
                "selected_cached_aliases":book.groups.values().filter(|group| selection.enabled_groups.contains(&group.id))
                    .map(|group| group.members.len()).sum::<usize>(),
                "independent_full_calendar_comparison_days":14,
            }),
            Err(error) => serde_json::json!({"error":error}),
        },
        "owned_connection_cleanup":if cleanup.is_ok() {"passed"} else {"failed"},
        "local_invitation_cache_saved_and_reopened":saved,
        "provider_writes":false, "calendar_coverage_modified":false,
    });
    let mut file = OpenOptions::new().write(true).create_new(true).open(&output).unwrap();
    file.write_all(&serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    file.sync_all().unwrap();
    cleanup.unwrap();
    result.unwrap();
    if save { assert!(saved); }
}
