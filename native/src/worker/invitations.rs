use super::{Sdk, Store};
use crate::{calendar::{Calendar, Event}, invitations as model};
use chrono::{Months, Utc};
use std::{sync::Arc, time::Instant};

#[derive(Default)]
pub(super) struct Queue {
    pub book: Arc<model::Book>,
    pub pending: model::Reader,
    pub teams: model::Reader,
    pub revision: u64,
    sequence: u64,
    discovering: bool,
    blocked: bool,
}

impl Queue {
    pub fn load(&mut self, store: &Store, account: Option<&str>, calendar: &Calendar, demo: bool) -> Result<(), String> {
        self.pending = Default::default();
        self.teams = Default::default();
        self.revision += 1;
        self.blocked = false;
        let Some(account) = account else { self.book = Default::default(); return Ok(()); };
        let result = store.invitations(account);
        let mut book = match result {
            Ok(book) => book,
            Err(error) => {
                self.book = Arc::new(model::Book::new(account));
                self.pending.progress.error = Some(error.clone());
                self.blocked = true;
                return Err(error);
            }
        };
        if !demo && book.refreshed_at.is_none() {
            book.replace(calendar.events.clone(), Utc::now())?;
            book.refreshed_at = None;
        }
        if demo {
            let mut events = calendar.events.iter().filter(|event| !event.is_all_day && event.start > Utc::now())
                .take(3).cloned().collect::<Vec<_>>();
            for (index, event) in events.iter_mut().enumerate() {
                event.id = format!("demo-invitation-{index}");
                event.status = "notResponded".into();
                event.extra = std::collections::BTreeMap::from([
                    ("isOrganizer".into(), serde_json::json!(false)),
                    ("isCancelled".into(), serde_json::json!(false)),
                    ("type".into(), serde_json::json!("singleInstance")),
                    ("categories".into(), serde_json::json!([])),
                ]);
                event.category.clear();
                event.briefing_status.clear();
                event.location = "Microsoft Teams Meeting".into();
                event.organizer = if index == 1 { "Learning <learning@tomlook.invalid>" }
                    else { "Jamie Brooks <jamie@northstar.invalid>" }.into();
                event.attendees = vec!["Alex Rivers <alex@tomlook.invalid>".into(), event.organizer.clone()];
                if index == 1 { event.extra.insert("tomlook_attendees_total".into(), serde_json::json!(100)); }
                event.title = match index {
                    1 => "Internal all hands (invitation)",
                    2 => "Architecture review (invitation)",
                    _ => "Customer workshop (invitation)",
                }.into();
            }
            book.replace(events, Utc::now())?;
        }
        self.book = Arc::new(book);
        Ok(())
    }

    fn dispatch(&mut self, sdk: &Sdk, request: model::ReadKind, team: bool) -> Result<(), String> {
        self.sequence = self.sequence.checked_add(1).ok_or("Invitation read identity exhausted")?;
        let reader = if team { &mut self.teams } else { &mut self.pending };
        reader.start(self.sequence, &self.book.account);
        self.revision += 1;
        if sdk.commands.try_send(crate::ai::Command::ReadInvitations {
            id: self.sequence, account: self.book.account.clone(), request,
        }).is_err() {
            let error = "Invitation read could not be queued; cache preserved".to_owned();
            reader.finish(Err(error.clone()), model::REFRESH_SECONDS);
            return Err(error);
        }
        Ok(())
    }

    pub fn poll(&mut self, sdk: Option<&Sdk>, connected: bool, demo: bool, exiting: bool) -> Result<(), String> {
        if self.pending.expire() | self.teams.expire() { self.revision += 1; }
        if !connected {
            for reader in [&mut self.pending, &mut self.teams] {
                if reader.progress.busy {
                    reader.finish(Err("WorkIQ disconnected during invitation read; cache preserved".into()), model::REFRESH_SECONDS);
                    self.revision += 1;
                }
            }
        }
        if !connected || demo || exiting || self.blocked || self.book.account.is_empty() { return Ok(()); }
        let Some(sdk) = sdk else { return Ok(()); };
        if self.pending.due() {
            let first = chrono::Local::now().date_naive();
            let last = first.checked_add_months(Months::new(6)).ok_or("Invitation dates exceed supported bounds")?;
            self.dispatch(sdk, model::ReadKind::Pending { first, last }, false)?;
        }

        if self.teams.due() {
            let now = Utc::now();
            let groups = self.book.groups.values().filter(|group| group.enabled && group.refreshed_at
                .is_none_or(|stamp| now - stamp >= chrono::Duration::days(1))).cloned().collect::<Vec<_>>();
            if groups.is_empty() {
                self.teams.next = Instant::now() + std::time::Duration::from_secs(3_600);
            } else {
                self.discovering = false;
                self.dispatch(sdk, model::ReadKind::Teams(groups), true)?;
            }
        }
        Ok(())
    }

    pub fn result(&mut self, store: &Store, id: u64, account: &str,
        result: Result<model::ReadResult, String>,
        calendar: &Calendar, freshness: &crate::calendar_sync::Freshness,
    ) -> Result<(), String> {
        let pending = self.pending.accepts(id, account);
        let team = self.teams.accepts(id, account);
        if self.book.account != account || (!pending && !team) { return Ok(()); }
        let saved = (|| {
            let mut book = (*self.book).clone();
            let mut warning = None;
            match result? {
                model::ReadResult::Pending(events) if pending => {
                    let now = Utc::now();
                    let started = self.pending.started_at().ok_or("Invitation read lost its start time")?;
                    let events = events.iter().filter_map(|event|
                        model::current(event, calendar, freshness, started, now).cloned()).collect();
                    book.replace(events, now)?;
                }
                model::ReadResult::Teams(groups) if team && !self.discovering => {
                    if groups.iter().any(|group| group.error.is_some()) {
                        warning = Some("Some team groups could not be refreshed; saved members were kept".to_owned());
                    }
                    book.merge_groups(groups, false)?;
                }
                model::ReadResult::Discovered(groups) if team && self.discovering =>
                    book.merge_groups(groups, true)?,
                _ => return Err("Invitation read returned a different operation; cache preserved".to_owned()),
            }
            store.save_invitations(&book)?;
            self.book = Arc::new(book);
            if let Some(warning) = warning { return Err(warning); }
            Ok(())
        })();
        let reader = if pending { &mut self.pending } else { &mut self.teams };
        reader.finish(saved.clone(), if pending { model::REFRESH_SECONDS } else { model::TEAM_REFRESH_SECONDS });
        self.revision += 1;
        saved
    }

    pub fn handle(&mut self, command: model::Command, store: &mut Store, calendar: &Calendar,
        freshness: &crate::calendar_sync::Freshness, sdk: Option<&Sdk>, demo: bool,
    ) -> Result<Option<Calendar>, String> {
        let connected = sdk.is_some();
        if self.blocked { return Err("The saved invitation queue could not be loaded; restart before making local changes".into()); }
        if self.book.account != command.account() {
            return Err("Invitation queue belongs to a different calendar account".into());
        }
        let mut next = (*self.book).clone();
        match command {
            model::Command::Open { key, .. } => {
                let event = next.records.get(&key).map(|record| &record.event)
                    .ok_or("Invitation is no longer pending")?;
                let event = if let Some(since) = next.refreshed_at {
                    model::current(event, calendar, freshness, since, Utc::now())
                        .ok_or("Invitation is no longer pending")?
                } else { event };
                if !model::pending(event, Utc::now()) { return Err("Invitation is no longer pending".into()); }
                let calendar = if demo {
                    let mut events = calendar.events.clone();
                    events.retain(|cached| cached.id != event.id);
                    events.push(event.clone());
                    Calendar::build(events, calendar.coverage.clone(), calendar.warnings.clone())?
                } else { store.save_calendar_event(event, &next.account)? };
                return Ok(Some(calendar));
            }
            model::Command::Local { action, .. } => {
                let group_changed = matches!(action, model::LocalAction::GroupEnabled { .. } | model::LocalAction::RemoveGroup(_));
                if let model::LocalAction::Snooze { key, .. } = &action
                    && let Some(since) = next.refreshed_at
                    && let Some(record) = next.records.get_mut(key) {
                    let event = model::current(&record.event, calendar, freshness, since, Utc::now())
                        .filter(|event| model::pending(event, Utc::now())).ok_or("Invitation is no longer pending")?;
                    record.event = event.clone();
                }
                next.apply(action, Utc::now())?;
                if group_changed { self.teams = Default::default(); }
                self.teams.next = Instant::now();
            }
            model::Command::Refresh { .. } => {
                if !connected || demo { return Err("Connect WorkIQ before refreshing invitations".into()); }
                if self.pending.progress.busy { return Ok(None); }
                self.pending.next = Instant::now();
                return Ok(None);
            }
            model::Command::RefreshTeams { .. } => {
                if !connected || demo { return Err("Connect WorkIQ before refreshing team membership".into()); }
                if self.teams.progress.busy { return Err("Team discovery or refresh is already running".into()); }
                self.discovering = false;
                let groups = next.groups.values().filter(|group| group.enabled).cloned().collect::<Vec<_>>();
                if groups.is_empty() { return Err("Select a team group before refreshing membership".into()); }
                self.dispatch(sdk.ok_or("WorkIQ executor is unavailable")?, model::ReadKind::Teams(groups), true)?;
                return Ok(None);
            }
            model::Command::Discover { queries, .. } => {
                if !connected || demo { return Err("Connect WorkIQ before searching historical mail".into()); }
                if self.teams.progress.busy { return Err("Team discovery or refresh is already running".into()); }
                model::validate_queries(&queries)?;
                if queries.is_empty() { return Err("Enter a team search before looking in historical mail".into()); }
                next.discovery_queries = queries.clone();
                store.save_invitations(&next)?;
                self.book = Arc::new(next);
                self.discovering = true;
                self.dispatch(sdk.ok_or("WorkIQ executor is unavailable")?, model::ReadKind::Discover(queries), true)?;
                return Ok(None);
            }
        }
        store.save_invitations(&next)?;
        self.book = Arc::new(next);
        self.revision += 1;
        Ok(None)
    }

    pub fn verified(&mut self, store: &Store, event: &Event) -> Result<(), String> {
        // Even a newly seen invitation can have been included in a read that is still in flight.
        self.pending = Default::default();
        self.revision += 1;
        let mut next = (*self.book).clone();
        let Some(id) = next.records.iter().find(|(_, record)| record.event.id == event.id)
            .map(|(id, _)| id.clone()) else { return Ok(()); };
        if model::pending(event, Utc::now()) {
            next.records.get_mut(&id).ok_or("Invitation disappeared")?.event = event.clone();
        } else {
            next.records.remove(&id);
            next.snoozed.remove(&id);
        }
        store.save_invitations(&next)?;
        self.book = Arc::new(next);
        Ok(())
    }

    pub fn removed(&mut self, store: &Store, event_id: &str) -> Result<(), String> {
        self.pending = Default::default();
        self.revision += 1;
        let mut next = (*self.book).clone();
        next.records.retain(|_, record| record.event.id != event_id);
        next.snoozed.retain(|key, _| next.records.contains_key(key));
        if next.records.len() != self.book.records.len() {
            store.save_invitations(&next)?;
            self.book = Arc::new(next);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ACCOUNT: &str = "owner@internal.example";

    fn event() -> Event {
        let mut event = crate::calendar::fixture(1).remove(0);
        event.start = (Utc::now() + chrono::Duration::days(1)).fixed_offset();
        event.end = event.start + chrono::Duration::hours(1);
        event.status = "notResponded".into();
        event.extra.insert("isOrganizer".into(), serde_json::json!(false));
        event.extra.insert("type".into(), serde_json::json!("singleInstance"));
        event.extra.remove("isFollowing");
        event.extra.insert("categories".into(), serde_json::json!([]));
        event.category.clear();
        event
    }

    fn loaded(store: &Store, event: &Event) -> Queue {
        let mut book = model::Book::new(ACCOUNT);
        book.replace(vec![event.clone()], Utc::now()).unwrap();
        store.save_invitations(&book).unwrap();
        let mut queue = Queue::default();
        queue.load(store, Some(ACCOUNT), &Calendar::default(), false).unwrap();
        queue
    }

    #[test]
    fn offline_demo_uses_the_shared_preview_identity_and_opens_its_synthetic_pending_invitations() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let calendar = Calendar::build(crate::showcase::preview_events(chrono::Local::now(), 200),
            Default::default(), vec![]).unwrap();
        let account = crate::feedback::PREVIEW_ACCOUNT;
        let mut queue = Queue::default();
        queue.load(&store, Some(account), &calendar, true).unwrap();
        assert_eq!(queue.book.records.len(), 3);
        assert!(queue.book.records.values().all(|record| !record.event.is_all_day && model::pending(&record.event, Utc::now())));
        let snapshot = model::snapshot(queue.book.clone(), &calendar, &Default::default(), None,
            &Default::default(), Default::default(), Default::default(), Utc::now());
        assert_eq!(snapshot.important(), 2);
        let row = snapshot.rows.iter().find(|row| row.tab == model::Tab::Important).unwrap();
        assert!(calendar.event_index(&row.event.id).is_none());
        let opened = queue.handle(model::Command::Open { account: account.into(), key: row.key.clone() },
            &mut store, &calendar, &Default::default(), None, true).unwrap().unwrap();
        assert!(opened.events.iter().any(|event| event.id == row.event.id && model::pending(event, Utc::now())));
        assert!(store.calendar_owner().unwrap().is_none());
        assert!(model::Book::new("an-invalid-live-account").validate().is_err());
    }

    #[test]
    fn verified_response_invalidates_older_reads_and_is_durable_before_the_next_refresh() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut event = event();
        let mut queue = loaded(&store, &event);
        queue.pending.start(10, ACCOUNT);
        let pending = event.clone();
        event.status = "accepted".into();
        queue.verified(&store, &event).unwrap();
        assert!(queue.book.records.is_empty());
        queue.result(&store, 10, ACCOUNT, Ok(model::ReadResult::Pending(vec![pending.clone()])), &Calendar::default(), &Default::default()).unwrap();
        assert!(queue.book.records.is_empty(), "An older snapshot cannot restore a just-accepted request");
        assert!(store.invitations(ACCOUNT).unwrap().records.is_empty());
        queue.pending.start(11, ACCOUNT);
        queue.verified(&store, &event).unwrap();
        queue.result(&store, 11, ACCOUNT, Ok(model::ReadResult::Pending(vec![pending])), &Calendar::default(), &Default::default()).unwrap();
        assert!(queue.book.records.is_empty(), "This also applies before a record was ever in the book");
    }

    #[test]
    fn invitation_result_identity_and_persistence_failures_never_overwrite_the_last_good_cache() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = event();
        let mut queue = loaded(&store, &source);
        queue.pending.start(4, ACCOUNT);
        queue.result(&store, 3, ACCOUNT, Ok(model::ReadResult::Pending(vec![])), &Calendar::default(), &Default::default()).unwrap();
        queue.result(&store, 4, "other@internal.example", Ok(model::ReadResult::Pending(vec![])), &Calendar::default(), &Default::default()).unwrap();
        assert!(queue.pending.progress.busy);
        let mut invalid = source.clone();
        invalid.id.clear();
        assert!(queue.result(&store, 4, ACCOUNT, Ok(model::ReadResult::Pending(vec![invalid])), &Calendar::default(), &Default::default()).is_err());
        assert!(queue.pending.progress.error.is_some());
        assert_eq!(queue.book.records.len(), 1);
        assert_eq!(store.invitations(ACCOUNT).unwrap().records.len(), 1);
        queue.pending.start(5, ACCOUNT);
        queue.result(&store, 5, ACCOUNT, Ok(model::ReadResult::Pending(vec![])), &Calendar::default(), &Default::default()).unwrap();
        assert!(queue.book.records.is_empty());
        queue.load(&store, Some("other@internal.example"), &Calendar::default(), false).unwrap();
        queue.result(&store, 5, ACCOUNT, Ok(model::ReadResult::Pending(vec![source])), &Calendar::default(), &Default::default()).unwrap();
        assert!(queue.book.records.is_empty());
    }

    #[test]
    fn removing_an_invitation_also_cancels_an_older_queue_read() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let event = event();
        let mut queue = loaded(&store, &event);
        queue.pending.start(2, ACCOUNT);
        queue.removed(&store, &event.id).unwrap();
        queue.result(&store, 2, ACCOUNT, Ok(model::ReadResult::Pending(vec![event])), &Calendar::default(), &Default::default()).unwrap();
        assert!(queue.book.records.is_empty());
        assert!(store.invitations(ACCOUNT).unwrap().records.is_empty());
    }

    #[test]
    fn a_newer_full_calendar_response_wins_over_an_older_filtered_read_and_cannot_be_overwritten_on_open() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let pending = event();
        let mut queue = loaded(&store, &pending);
        queue.pending.start(1, ACCOUNT);
        let mut accepted = pending.clone();
        accepted.status = "accepted".into();
        let day = accepted.calendar_dates().0;
        let calendar = Calendar::build(vec![accepted], std::collections::BTreeSet::from([day]), vec![]).unwrap();
        let freshness = std::collections::BTreeMap::from([(day, Utc::now())]);
        assert!(queue.handle(model::Command::Open { account: ACCOUNT.into(), key: model::key(&pending) },
            &mut store, &calendar, &freshness, None, false).is_err());
        queue.result(&store, 1, ACCOUNT, Ok(model::ReadResult::Pending(vec![pending])), &calendar, &freshness).unwrap();
        assert!(queue.book.records.is_empty());
        assert!(store.invitations(ACCOUNT).unwrap().records.is_empty());
    }
}
