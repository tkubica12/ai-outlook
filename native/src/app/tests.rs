fn invitation_fixture(language: tomlook::i18n::Language) -> (
    Tomlook, mpsc::Receiver<Command>, tokio::sync::mpsc::Receiver<ai::Command>, tomlook::invitations::Row,
) {
    let (mut app, _, ai) = assistant_fixture();
    let (sender, receiver) = mpsc::sync_channel(16);
    app.worker = Some(Worker { commands: sender, notices: Arc::new(Default::default()),
        activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(Default::default()), done: None });
    app.preferences.language = language;
    app.preferences.hide_unconfirmed = true;
    app.query = "unrelated search".into();
    app.category = "Office".into();
    app.calendar_owner = Some("owner@internal.example".into());
    app.workiq_ready = true;
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.title = "Invitation fixture".into();
    event.start = (chrono::Utc::now() + chrono::Duration::days(3)).fixed_offset();
    event.end = event.start + chrono::Duration::hours(1);
    event.status = "notResponded".into();
    event.is_all_day = false;
    event.organizer = "Organizer <organizer@customer.example>".into();
    event.category.clear();
    event.location = "Microsoft Teams Meeting".into();
    event.extra = BTreeMap::from([
        ("isOrganizer".into(), serde_json::json!(false)), ("type".into(), serde_json::json!("singleInstance")),
        ("categories".into(), serde_json::json!([])),
    ]);
    let mut book = tomlook::invitations::Book::new("owner@internal.example");
    book.discovery_queries = vec!["Region STU".into(), "Region ATU".into()];
    book.replace(vec![event], chrono::Utc::now()).unwrap();
    let snapshot = tomlook::invitations::snapshot(Arc::new(book), &app.calendar, &Default::default(), None,
        &Default::default(), Default::default(), Default::default(), chrono::Utc::now());
    let row = snapshot.rows[0].clone();
    Arc::make_mut(&mut app.activity).invitations = Arc::new(snapshot);
    (app, receiver, ai, row)
}

fn bulk_invitation_fixture(language: tomlook::i18n::Language, other: bool) -> (
    Tomlook, mpsc::Receiver<Command>, tokio::sync::mpsc::Receiver<ai::Command>,
) {
    let (mut app, receiver, ai, row) = invitation_fixture(language);
    let mut second = row.event.clone();
    second.id = "second-invitation".into();
    second.title = "Second invitation".into();
    second.extra.insert("type".into(), serde_json::json!("occurrence"));
    let mut book = (*app.activity.invitations.book).clone();
    book.replace(vec![row.event, second], chrono::Utc::now()).unwrap();
    if other {
        book.organizers.insert("organizer@customer.example".into(), tomlook::invitations::OrganizerRule::Quiet);
    }
    Arc::make_mut(&mut app.activity).invitations = Arc::new(tomlook::invitations::snapshot(
        Arc::new(book), &Default::default(), &Default::default(), None, &Default::default(),
        Default::default(), Default::default(), chrono::Utc::now()));
    app.open_invitations();
    (app, receiver, ai)
}

fn invitation_list_and_bulk_dialog(app: &mut Tomlook, ui: &mut egui::Ui) {
    ui.set_width(300.0);
    app.invitation_sidebar(ui);
    app.invitation_bulk_dialog(ui.ctx());
}

fn invitation_pointer_click(context: &egui::Context, app: &mut Tomlook, label: &str,
    button: egui::PointerButton, modifiers: Modifiers, checkbox: bool,
) {
    let output = draw_with(context, app, vec![], &invitation_list_and_bulk_dialog);
    let point = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.job.text == label => Some(if checkbox {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if (8.0..=24.0).contains(&rect.rect.width())
                    && rect.rect.right() < text.pos.x && rect.rect.center().y >= text.pos.y
                    && rect.rect.center().y <= text.pos.y + 24.0 => Some(rect.rect.center()),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing checkbox for {label}"))
        } else { text.pos + text.galley.size() * 0.5 }),
        _ => None,
    }).unwrap_or_else(|| panic!("Missing invitation: {label}"));
    for pressed in [true, false] {
        let mut output = context.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 2400.0))),
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(point),
                egui::Event::PointerButton { pos: point, button, pressed, modifiers }],
            ..Default::default()
        }, |ui| invitation_list_and_bulk_dialog(app, ui));
        output.textures_delta.clear();
    }
}

#[test]
fn invitation_bulk_actions_confirm_exact_pending_occurrences_in_important_and_other_without_ai() {
    use tomlook::invitations::BulkAction;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for other in [false, true] {
            for action in [BulkAction::Decline, BulkAction::Follow] {
                let (mut app, receiver, mut ai) = bulk_invitation_fixture(language, other);
                let context = egui::Context::default();
                let show: Show = &invitation_list_and_bulk_dialog;
                settled(&context, &mut app, show);
                if other {
                    click_text(&context, &mut app, show, &format!("{} 2", language.text("Other")));
                }
                click_text(&context, &mut app, show, language.text("Select page").as_ref());
                assert_eq!(app.invitations.marked.len(), 2);
                assert!(app.invitations.selected.is_none(), "Bulk selection must not open/stage a meeting");
                invitation_pointer_click(&context, &mut app, "Invitation fixture", egui::PointerButton::Secondary, Modifiers::NONE, false);
                assert_eq!(app.invitations.marked.len(), 2, "Right-click a marked row preserves the complete selection");
                click_text(&context, &mut app, show, language.text(action.label()).as_ref());
                let text = settled(&context, &mut app, show);
                assert!(text.contains("Invitation fixture") && text.contains("Second invitation"));
                assert!(receiver.try_recv().is_err(), "Previewing is not approval");
                click_text(&context, &mut app, show, language.text("Confirm for all selected invitations").as_ref());
                let Command::ConfirmInvitationActions { account, events, action: submitted } = receiver.try_recv().unwrap()
                    else { panic!("One explicitly confirmed batch is required"); };
                assert_eq!(account, "owner@internal.example");
                assert_eq!(submitted, action);
                assert_eq!(events.len(), 2);
                assert!(events.iter().any(|event| event.extra["type"] == "occurrence"));
                assert!(events.iter().all(|event| event.status == "notResponded"));
                assert!(app.invitations.marked.is_empty() && app.invitations.bulk.is_none());
                assert!(app.messages_open);
                assert!(receiver.try_recv().is_err());
                assert!(ai.try_recv().is_err());
            }
        }
    }
}

#[test]
fn invitation_bulk_selection_checkboxes_modifiers_and_unselected_context_target_the_correct_rows() {
    let (mut app, receiver, mut ai) = bulk_invitation_fixture(tomlook::i18n::Language::En, false);
    let context = egui::Context::default();
    let show: Show = &invitation_list_and_bulk_dialog;
    settled(&context, &mut app, show);
    invitation_pointer_click(&context, &mut app, "Invitation fixture", egui::PointerButton::Primary, Modifiers::NONE, true);
    assert_eq!(app.invitations.marked.len(), 1, "Checkbox selection failed; opened row: {:?}", app.invitations.selected);
    assert!(app.invitations.selected.is_none(), "Checkboxes must not also open the row");
    invitation_pointer_click(&context, &mut app, "Second invitation", egui::PointerButton::Primary, Modifiers::SHIFT, false);
    assert_eq!(app.invitations.marked.len(), 2);
    invitation_pointer_click(&context, &mut app, "Second invitation", egui::PointerButton::Primary, Modifiers::CTRL, false);
    assert_eq!(app.invitations.marked.len(), 1);
    invitation_pointer_click(&context, &mut app, "Second invitation", egui::PointerButton::Secondary, Modifiers::NONE, false);
    assert_eq!(app.invitations.marked, BTreeSet::from([tomlook::invitations::key(
        &app.activity.invitations.rows.iter().find(|row| row.event.title == "Second invitation").unwrap().event)]));
    assert!(app.invitations.selected.is_none());
    assert!(receiver.try_recv().is_err());
    assert!(ai.try_recv().is_err());
}

#[test]
fn invitation_bulk_confirmation_blocks_changed_account_or_event_and_preserves_a_full_local_queue() {
    for case in 0..4 {
        let (mut app, receiver, _) = bulk_invitation_fixture(tomlook::i18n::Language::En, false);
        let context = egui::Context::default();
        let show: Show = &invitation_list_and_bulk_dialog;
        settled(&context, &mut app, show);
        click_text(&context, &mut app, show, "Select page");
        click_text(&context, &mut app, show, "Selected invitation actions");
        click_text(&context, &mut app, show, "Decline without sending");
        settled(&context, &mut app, show);
        match case {
            0 => app.calendar_owner = Some("different@internal.example".into()),
            1 => Arc::make_mut(&mut Arc::make_mut(&mut app.activity).invitations).rows[0].event.title = "Changed".into(),
            2 => app.workiq_ready = false,
            _ => {
                for _ in 0..16 { app.worker.as_ref().unwrap().commands.try_send(Command::Wake).unwrap(); }
            }
        }
        click_text(&context, &mut app, show, "Confirm for all selected invitations");
        assert!(app.invitations.bulk.is_some(), "An unsubmitted approval must remain reviewable");
        assert!(receiver.try_iter().all(|command| matches!(command, Command::Wake)));
        assert!(!app.messages_open);
        if case == 3 { assert!(app.error.as_ref().unwrap().contains("Local queue is full")); }
        draw_with(&context, &mut app, vec![egui::Event::Key {
            key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE,
        }], &|app, ui| app.shortcuts(ui.ctx()));
        assert!(app.invitations.bulk.is_none());
    }
}

#[test]
fn invitation_queue_focuses_uncached_requests_despite_filters_and_reuses_the_existing_category_approval() {
    use tomlook::recommendations::{Change, MeetingResponse};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai, row) = invitation_fixture(language);
        let anchor = app.anchor;
        let context = egui::Context::default();
        let sidebar: Show = &|app, ui| { ui.set_width(300.0); app.sidebar(ui); };
        settled(&context, &mut app, sidebar);
        click_text(&context, &mut app, sidebar, &format!("{}  1", language.text("Invitations")));
        assert!(app.invitations.open);
        assert!(receiver.try_recv().is_err(), "Opening the queue does not run research or resolve requests");
        click_text(&context, &mut app, sidebar, &row.event.title);
        assert!(matches!(receiver.try_recv().unwrap(), Command::Invitation(tomlook::invitations::Command::Open { key, .. }) if key == row.key));
        assert!(app.selected.is_none(), "An uncached invitation is staged on the storage worker first");
        assert_eq!(app.visible_days().len(), 3);
        let mut catalog = row.event.clone();
        catalog.id = "category-catalog".into();
        catalog.status = "accepted".into();
        catalog.extra.insert("categories".into(), serde_json::json!(["Call", "Office"]));
        app.calendar = Arc::new(Calendar::build(vec![row.event.clone(), catalog], Default::default(), vec![]).unwrap());
        app.receive_invitation_focus();
        let index = app.calendar.event_index(&row.event.id).unwrap();
        assert_eq!(app.selected, Some(index));
        assert!(matches!(receiver.try_recv().unwrap(), Command::Detail { .. }));
        let query = tomlook::calendar::SearchQuery::new(&app.query);
        assert!(app.matches_event(index, &query), "Hidden-unconfirmed, search and category filters cannot hide review context");
        let context_bar: Show = &|app, ui| app.invitation_context_bar(ui);
        click_text(&context, &mut app, context_bar, language.text("Whole week").as_ref());
        assert_eq!(app.visible_days().len(), 7);
        click_text(&context, &mut app, context_bar, language.text("Day + neighbors").as_ref());
        assert_eq!(app.visible_days().len(), 3);
        app.choose_meeting_action(row.event.clone(), Change::Respond { response: MeetingResponse::Accept,
            comment: String::new(), send_response: false, category: None });
        assert!(matches!(&app.meeting_action.as_ref().unwrap().change,
            Change::Respond { category: Some(category), .. } if category.eq_ignore_ascii_case("call")));
        assert!(receiver.try_recv().is_err(), "Opening the normal acceptance/category preview is not confirmation");
        app.meeting_action = None;
        click_text(&context, &mut app, sidebar, language.text("Calendar").as_ref());
        assert!(!app.invitations.open);
        assert_eq!(app.anchor, anchor);
        assert!(app.date_selection.is_none());
        assert_eq!(app.query, "unrelated search");
        assert_eq!(app.category, "Office");
        assert!(app.preferences.hide_unconfirmed);
        assert!(!app.matches_event(index, &query));
        assert!(ai.try_recv().is_err(), "Invitation triage never requires an AI request");
    }
}

#[test]
fn queued_invitation_is_emphasized_at_full_height_and_stays_pinned_while_reviewing_conflicts() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for dark in [false, true] {
            let (mut app, receiver, mut ai, row) = invitation_fixture(language);
            app.preferences.dark = dark;
            app.loading = false;
            let mut other = row.event.clone();
            other.id = "competing-invitation".into();
            other.title = "Competing fixture".into();
            app.calendar = Arc::new(Calendar::build(vec![row.event.clone(), other.clone()], Default::default(), vec![]).unwrap());
            let original = app.calendar.event_index(&row.event.id).unwrap();
            let competing = app.calendar.event_index(&other.id).unwrap();
            app.open_invitation(&row);
            app.receive_invitation_focus();
            assert!(app.invitation_highlighted(&row.event.id));
            assert!(!app.invitation_highlighted(&other.id));
            app.select(competing);
            assert!(app.invitation_highlighted(&row.event.id), "Reviewing a conflict must not lose the queued invitation's marker");
            let context = egui::Context::default();
            app.style(&context);
            for height in [23.0, 38.0, 100.0, 180.0] {
                let rect = egui::Rect::from_min_size(egui::pos2(40.0, 40.0), Vec2::new(300.0, height));
                let other_rect = rect.translate(Vec2::new(320.0, 0.0));
                let mut output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 640.0))),
                    ..Default::default()
                }, |ui| {
                    app.event_card(ui, original, rect);
                    app.event_card(ui, competing, other_rect);
                });
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(shape) if shape.rect == rect.shrink(1.5)
                        && shape.stroke == Stroke::new(3.0, app.accent()))), "The complete invitation timeslot must have the strong outline");
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(shape) if shape.rect.intersects(other_rect) && shape.stroke.width == 3.0)));
                let text = output_text(&output);
                assert!(text.contains(if height >= 100.0 { &row.event.title } else { "Invitation" }));
                assert_eq!(text.contains(language.text("Selected invitation").as_ref()), height >= 100.0);
                output.textures_delta.clear();
            }
            let mut book = (*app.activity.invitations.book).clone();
            book.replace(vec![row.event.clone(), other.clone()], chrono::Utc::now()).unwrap();
            let snapshot = tomlook::invitations::snapshot(Arc::new(book), &app.calendar, &Default::default(), None,
                &Default::default(), Default::default(), Default::default(), chrono::Utc::now());
            let second = snapshot.rows.iter().find(|row| row.event.id == other.id).unwrap().clone();
            Arc::make_mut(&mut app.activity).invitations = Arc::new(snapshot);
            app.open_invitation(&second);
            assert!(!app.invitation_highlighted(&row.event.id));
            assert!(app.invitation_highlighted(&other.id));
            Arc::make_mut(&mut Arc::make_mut(&mut app.activity).invitations).rows.clear();
            assert!(!app.invitation_highlighted(&other.id), "Resolved or removed requests cannot keep a stale highlight");
            app.invitations.open = false;
            assert!(!app.invitation_highlighted(&row.event.id));
            assert_eq!(app.calendar.events[original].status, "notResponded");
            assert!(receiver.try_iter().all(|command| matches!(command, Command::Invitation(_) | Command::Detail { .. })));
            assert!(ai.try_recv().is_err(), "Emphasis cannot accept an invitation or trigger research");
        }
    }
}

#[test]
#[ignore = "Creates a new synthetic profile for native visual checks; set TOMLOOK_UI_PREVIEW_STATE"]
fn export_invitation_and_status_visual_profile() {
    let root = std::path::PathBuf::from(std::env::var_os("TOMLOOK_UI_PREVIEW_STATE").expect("Set TOMLOOK_UI_PREVIEW_STATE"));
    assert!(root.is_absolute() && !root.exists()
        && root.file_name().unwrap().to_str().unwrap().starts_with("tomlook-ui-preview-"),
        "Only a new, explicitly named preview profile may be written");
    let (mut app, _, _, row) = invitation_fixture(tomlook::i18n::Language::Cs);
    let day = (Local::now() + chrono::Duration::days(3)).date_naive();
    let at = |hour, minute| day.and_hms_opt(hour, minute, 0).unwrap()
        .and_local_timezone(Local).single().unwrap().fixed_offset();
    let mut request = row.event;
    request.title = "Northstar / Architecture review".into();
    request.start = at(11, 0);
    request.end = at(12, 0);
    let mut events = vec![request.clone()];
    for (id, title, start, end) in [
        ("morning", "Internal planning", at(9, 0), at(10, 0)),
        ("conflict", "Project check-in", at(10, 30), at(11, 30)),
        ("afternoon", "Implementation workshop", at(13, 0), at(14, 0)),
    ] {
        let mut event = request.clone();
        event.id = id.into(); event.title = title.into(); event.start = start; event.end = end;
        event.status = "accepted".into();
        events.push(event);
    }
    let coverage = (0..7).map(|offset| day + chrono::Duration::days(offset)).collect();
    let calendar = Calendar::build(events, coverage, vec![]).unwrap();
    let account = "owner@internal.example";
    let mut book = tomlook::invitations::Book::new(account);
    book.replace(vec![request], chrono::Utc::now()).unwrap();
    let mut store = tomlook::storage::Store::open(&root).unwrap();
    store.save_workiq_calendar(&calendar, account).unwrap();
    store.save_workiq_identity(&tomlook::workiq::connection::Identity {
        account: account.into(), mail: None, display_name: Some("Visual fixture".into()),
    }).unwrap();
    store.save_invitations(&book).unwrap();
    app.preferences.paused = true;
    app.preferences.prepare_enabled = false;
    app.preferences.msx_enabled = false;
    app.preferences.welcome_shown = true;
    store.save_preferences(&app.preferences).unwrap();
}

#[test]
fn invitation_team_settings_send_only_explicit_account_bound_reads_and_disable_after_account_changes() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai, _) = invitation_fixture(language);
        let context = egui::Context::default();
        app.open_invitations();
        let sidebar: Show = &|app, ui| { ui.set_width(300.0); app.sidebar(ui); };
        settled(&context, &mut app, sidebar);
        click_text(&context, &mut app, sidebar, language.text("My team").as_ref());
        let settings: Show = &|app, ui| app.invitation_settings(ui.ctx());
        let text = settled(&context, &mut app, settings);
        assert!(text.contains(language.text("Team groups").as_ref()), "{text}");
        assert!(receiver.try_recv().is_err());
        click_text(&context, &mut app, settings, language.text("Find groups in historical mail").as_ref());
        let Command::Invitation(tomlook::invitations::Command::Discover { account, queries }) = receiver.try_recv().unwrap()
            else { panic!("Only an explicit historical-mail discovery may be dispatched"); };
        assert_eq!(account, "owner@internal.example");
        assert_eq!(queries, ["Region STU", "Region ATU"]);
        let snapshot = Arc::make_mut(&mut app.activity).invitations.clone();
        let mut next = (*snapshot).clone();
        next.book = Arc::new(tomlook::invitations::Book::new("other@internal.example"));
        Arc::make_mut(&mut app.activity).invitations = Arc::new(next);
        let text = settled(&context, &mut app, settings);
        assert!(text.contains(language.text("Calendar account changed. Close and reopen these settings.").as_ref()));
        assert!(!text.contains(language.text("Find groups in historical mail").as_ref()));
        assert!(receiver.try_recv().is_err());
        assert!(ai.try_recv().is_err());
    }
}

#[test]
fn invitation_queue_paints_only_a_bounded_page_even_with_hundreds_of_pending_requests() {
    let (mut app, _, _, row) = invitation_fixture(tomlook::i18n::Language::En);
    let events = (0..100).map(|index| {
        let mut event = row.event.clone();
        event.id = format!("request-{index:03}");
        event.title = format!("Request {index:03}");
        event.start += chrono::Duration::minutes(index);
        event.end += chrono::Duration::minutes(index);
        event
    }).collect();
    let mut book = tomlook::invitations::Book::new("owner@internal.example");
    book.replace(events, chrono::Utc::now()).unwrap();
    Arc::make_mut(&mut app.activity).invitations = Arc::new(tomlook::invitations::snapshot(
        Arc::new(book), &Default::default(), &Default::default(), None, &Default::default(),
        Default::default(), Default::default(), chrono::Utc::now()));
    app.open_invitations();
    let context = egui::Context::default();
    app.style(&context);
    let sidebar: Show = &|app, ui| { ui.set_width(300.0); app.sidebar(ui); };
    let text = settled(&context, &mut app, sidebar);
    assert!(text.contains("Request 000") && !text.contains("Request 040"), "{text}");
    click_text(&context, &mut app, sidebar, "Select page");
    assert_eq!(app.invitations.marked.len(), 40);
    click_text(&context, &mut app, sidebar, ">");
    let text = settled(&context, &mut app, sidebar);
    assert!(text.contains("Request 040") && !text.contains("Request 000"), "{text}");
    assert_eq!(app.invitations.marked.len(), 40);
    click_text(&context, &mut app, sidebar, "Select page");
    assert_eq!(app.invitations.marked.len(), 80);
    click_text(&context, &mut app, sidebar, "Other 0");
    assert!(app.invitations.marked.is_empty(), "Changing tabs must not leave invisible bulk targets");
}

#[test]
fn invitation_cards_wrap_long_titles_dates_and_organizers_within_the_real_sidebar_width() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, _, row) = invitation_fixture(language);
        let mut event = row.event;
        event.title = "An important customer discussion with a longer invitation title and several planning topics".into();
        event.organizer = "An organizer with a longer display name <long.organizer.address@customer.example>".into();
        let expected = [event.title.clone(), event.organizer.clone(), meeting_date_label(&event, language)];
        let mut book = tomlook::invitations::Book::new("owner@internal.example");
        book.replace(vec![event], chrono::Utc::now()).unwrap();
        Arc::make_mut(&mut app.activity).invitations = Arc::new(tomlook::invitations::snapshot(
            Arc::new(book), &Default::default(), &Default::default(), None, &Default::default(),
            Default::default(), Default::default(), chrono::Utc::now()));
        app.open_invitations();
        let context = egui::Context::default();
        let mut output = None;
        let mut right = 0.0;
        app.style(&context);
        for _ in 0..3 {
            let mut frame = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                ..Default::default()
            }, |ui| {
                egui::Panel::left("invitation-layout-test").exact_size(300.0).show(ui, |ui| {
                    right = ui.max_rect().right();
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| app.sidebar(ui));
                });
            });
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let mut found = 0;
        for shape in output.unwrap().shapes {
            if let egui::epaint::Shape::Text(text) = shape.shape
                && expected.contains(&text.galley.job.text) {
                found += 1;
                assert!(text.pos.x + text.galley.size().x <= right,
                    "{language:?}: invitation text extends beyond the sidebar: {}", text.galley.job.text);
            }
        }
        assert_eq!(found, 3, "All invitation card fields must actually be rendered");
    }
}

#[test]
fn settings_content_has_clear_padding_from_navigation_in_both_sections_and_themes() {
    for dark in [true, false] {
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            for section in [PreferencesSection::Setup, PreferencesSection::Display, PreferencesSection::Notifications, PreferencesSection::Prompts, PreferencesSection::Msx, PreferencesSection::Updates] {
                let mut app = Tomlook::initial(None, None, None);
                app.preferences.dark = dark;
                app.preferences.language = language;
                app.preferences_open = true;
                app.preferences_section = section;
                let context = egui::Context::default();
                context.set_visuals(if dark {egui::Visuals::dark()} else {egui::Visuals::light()});
                let mut output = None;
                for _ in 0..3 {
                    let mut frame = context.run_ui(egui::RawInput {
                        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                        ..Default::default()
                    }, |ui| app.preferences_dialog(ui.ctx()));
                    frame.textures_delta.clear();
                    output = Some(frame);
                }
                let output = output.unwrap();
                let divider = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::LineSegment {points,..}
                        if (points[0].x - points[1].x).abs() < 0.1
                            && (points[0].y - points[1].y).abs() > 300.0 => Some(points[0].x),
                    _ => None,
                }).expect("Settings navigation must have a vertical divider");
                let heading = language.text(match section {
                    PreferencesSection::Setup => "Setup checklist",
                    PreferencesSection::Display => "Display",
                    PreferencesSection::Notifications => "Notifications",
                    PreferencesSection::Prompts => "AI prompts",
                    PreferencesSection::Msx => "MSX and Azure consumption",
                    PreferencesSection::Updates => "Updates",
                });
                let content = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text)
                        if text.galley.job.text == heading && text.pos.x > divider => Some(text.pos),
                    _ => None,
                }).expect("Settings content heading must be visible");
                assert!(content.x - divider >= 23.0,
                    "{language:?} / {section:?} / dark={dark}: content is only {}px from the divider", content.x - divider);
                let window = context.memory(|memory| memory.area_rect(Id::new("preferences")).unwrap());
                assert!(window.right() - content.x > 450.0, "Padding must preserve usable content width");
            }
        }
    }
}

#[test]
fn acceptance_version_is_visible_and_meeting_slashes_always_bypass_auto_routing() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let context = egui::Context::default();
        let mut text = String::new();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| version_label(ui, language));
        output.textures_delta.clear();
        for shape in output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains(concat!("v", env!("CARGO_PKG_VERSION"))), "{text}");
        for mode in [tomlook::chat::Mode::Fast, tomlook::chat::Mode::Auto, tomlook::chat::Mode::Deep] {
            for command in ["/PREPARE", "/blocker additional topic"] {
                let (mut app, _, mut commands) = assistant_fixture();
                app.preferences.language = language;
                app.ask_mode = mode;
                assert!(app.switch_assistant_context(Some(tomlook::calendar::fixture(1).remove(0))));
                app.question = command.into();
                app.ask_question();
                let ai::Command::Chat(request) = commands.try_recv().unwrap() else { panic!("Expected an Ask request"); };
                assert_eq!(request.mode, tomlook::chat::Mode::Deep);
                assert_eq!(app.ask_mode, tomlook::chat::Mode::Deep);
                assert!(app.active_request.is_none());
                assert_eq!(app.chat.turns.back().unwrap().progress.mode, tomlook::chat::Mode::Deep);
                assert!(commands.try_recv().is_err(), "There is no Auto decision request");
            }
        }
    }
}

#[test]
fn acceptance_dialog_preselects_clear_categories_and_allows_overrides_without_writes_or_ai() {
    use tomlook::recommendations::{Change, MeetingResponse};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for (location, title, preview, suggested, existing) in [
            ("Teams", "Planning", "", Some("Call"), None),
            ("Office", "Planning", "", Some("Office"), None),
            ("", "Planning", "Sejdeme se u zákazníka", Some("Mimo office"), None),
            ("", "BLOCKER: Planning", "", Some("Nejisté"), None),
            ("", "Planning", "TBD", None, None),
            ("Teams", "Already categorized", "", None, Some("Office")),
        ] {
            for choice in [None, Some("Office"), Some("No category")] {
                let (mut app, _, mut ai_commands) = assistant_fixture();
                let (sender, receiver) = mpsc::sync_channel(16);
                app.worker = Some(Worker {
                    commands: sender, notices: Arc::new(Default::default()),
                    activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                    stop: Arc::new(Default::default()), done: None,
                });
                app.preferences.language = language;
                app.calendar_owner = Some("owner".into());
                app.workiq_ready = true;
                let mut event = tomlook::calendar::fixture(1).remove(0);
                event.title = title.into();
                event.location = location.into();
                event.category = existing.unwrap_or_default().into();
                event.organizer = "organizer@example.invalid".into();
                event.extra.insert("type".into(), serde_json::json!("singleInstance"));
                event.extra.insert("isOrganizer".into(), serde_json::json!(false));
                event.extra.insert("categories".into(), serde_json::json!(existing.into_iter().collect::<Vec<_>>()));
                event.extra.insert("bodyPreview".into(), serde_json::json!(preview));
                let mut catalog = event.clone();
                catalog.id = "category-catalog".into();
                catalog.extra.insert("categories".into(), serde_json::json!(["Call", "Office", "Mimo office", "Nejisté", "Follow"]));
                app.calendar = Arc::new(Calendar::build(vec![event.clone(), catalog], Default::default(), vec![]).unwrap());
                app.choose_meeting_action(event.clone(), Change::Respond {
                    response: MeetingResponse::Accept, comment: String::new(), send_response: false, category: None,
                });
                assert!(matches!(&app.meeting_action.as_ref().unwrap().change,
                    Change::Respond { category, .. } if category.as_deref() == suggested));
                let context = egui::Context::default();
                let show: Show = &|app, ui| app.meeting_action_dialog(ui.ctx());
                let text = settled(&context, &mut app, show);
                let keep = if existing.is_some() { "Keep current category" } else { "No category" };
                assert!(text.contains(language.text(keep).as_ref()), "{text}");
                assert!(!text.lines().any(|line| line.trim() == "Follow"), "Follow is not an acceptance category");
                assert!(receiver.try_recv().is_err(), "A preselected category is not an approval");
                if let Some(choice) = choice {
                    click_text(&context, &mut app, show, language.text(if choice == "No category" { keep } else { choice }).as_ref());
                }
                assert!(receiver.try_recv().is_err(), "Choosing a category is not an approval");
                click_text(&context, &mut app, show, language.text(MeetingResponse::Accept.label()).as_ref());
                let Command::ConfirmMeetingAction { event: approved, change, .. } = receiver.try_recv().unwrap()
                    else { panic!("Expected the combined explicit approval"); };
                let expected = match choice { Some("No category") => None, Some(name) => Some(name), None => suggested };
                assert_eq!(approved.id, event.id);
                assert_eq!(tomlook::recommendations::real_categories(&approved), existing.into_iter().map(str::to_owned).collect::<Vec<_>>());
                assert!(matches!(change, Change::Respond {
                    response: MeetingResponse::Accept, category, send_response: false, ..
                } if category.as_deref() == expected));
                assert!(receiver.try_recv().is_err());
                assert!(ai_commands.try_recv().is_err(), "Suggestions never call AI");
            }
        }
    }
}

#[test]
fn acceptance_research_send_button_checks_the_thorough_lane_before_forcing_the_visible_mode() {
    for busy in [tomlook::chat::Mode::Fast, tomlook::chat::Mode::Deep] {
        for command in ["/prepare", "/blocker"] {
            let (mut app, _, mut commands) = assistant_fixture();
            assert!(app.switch_assistant_context(Some(tomlook::calendar::fixture(1).remove(0))));
            Arc::make_mut(&mut app.chat).admit(90, "Running question".into(), busy).unwrap();
            app.ask_mode = if busy == tomlook::chat::Mode::Fast { tomlook::chat::Mode::Auto } else { tomlook::chat::Mode::Fast };
            app.question = command.into();
            let context = egui::Context::default();
            let show: Show = &|app, ui| app.assistant_panel(ui);
            settled(&context, &mut app, show);
            click_text(&context, &mut app, show, "Send");
            if busy == tomlook::chat::Mode::Fast {
                let ai::Command::Chat(request) = commands.try_recv().unwrap() else { panic!("The Thorough lane is available"); };
                assert_eq!(request.mode, tomlook::chat::Mode::Deep);
                assert_eq!(app.ask_mode, tomlook::chat::Mode::Deep);
            } else {
                assert!(commands.try_recv().is_err(), "A busy Thorough lane cannot admit another research command");
                assert_eq!(app.question, command);
            }
        }
    }
}

#[test]
fn acceptance_category_buttons_match_new_event_colors_and_selected_outlines_in_all_themes() {
    use tomlook::recommendations::{Change, MeetingResponse};
    let button = |output: &egui::FullOutput, label: &str| {
        let center = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        }).unwrap_or_else(|| panic!("Missing category label: {label}"));
        output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Rect(rect) if rect.rect.contains(center) => Some(rect),
            _ => None,
        }).min_by(|a, b| a.rect.area().total_cmp(&b.rect.area())).unwrap().clone()
    };
    for dark in [false, true] {
        for accent in 0..ACCENTS.len() {
            for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
                let (mut app, _, mut ai_commands) = assistant_fixture();
                app.preferences.dark = dark;
                app.preferences.accent = accent;
                app.preferences.language = language;
                app.calendar_owner = Some("owner".into());
                let mut event = tomlook::calendar::fixture(1).remove(0);
                event.title = "Planning".into();
                event.location = "Teams".into();
                event.category.clear();
                event.organizer = "organizer@example.invalid".into();
                event.extra.insert("isOrganizer".into(), serde_json::json!(false));
                event.extra.insert("categories".into(), serde_json::json!([]));
                let mut catalog = event.clone();
                catalog.id = "category-catalog".into();
                let categories = ["Call", "Office", "Mimo office", "Nejisté", "Osobní"];
                catalog.extra.insert("categories".into(), serde_json::json!(categories));
                app.calendar = Arc::new(Calendar::build(vec![event.clone(), catalog], Default::default(), vec![]).unwrap());
                let context = egui::Context::default();
                app.style(&context);
                context.style_mut_of(if dark { egui::Theme::Dark } else { egui::Theme::Light },
                    |style| style.animation_time = 0.0);
                app.open_event_draft(Local::now().date_naive(), 540, 600);
                app.event_draft.as_mut().unwrap().category = Some("Call".into());
                let editor: Show = &|app, ui| app.event_creation_dialog(ui.ctx());
                settled(&context, &mut app, editor);
                let created = draw_with(&context, &mut app, vec![], editor);
                app.event_draft = None;
                app.choose_meeting_action(event, Change::Respond {
                    response: MeetingResponse::Accept, comment: String::new(), send_response: false, category: None,
                });
                let acceptance: Show = &|app, ui| app.meeting_action_dialog(ui.ctx());
                settled(&context, &mut app, acceptance);
                let accepted = draw_with(&context, &mut app, vec![], acceptance);
                for category in categories {
                    let original = button(&created, category);
                    let actual = button(&accepted, category);
                    let fill = category_color(category, dark).gamma_multiply(if dark { 0.3 } else { 0.15 });
                    assert_eq!(original.fill, fill, "{category} / {dark} / {accent} / {language:?}");
                    assert_eq!(actual.fill, fill);
                    assert_eq!(actual.stroke, original.stroke);
                    assert_eq!(actual.stroke, if category == "Call" { Stroke::new(2.0, app.accent()) } else { Stroke::NONE });
                }
                assert_eq!(button(&accepted, language.text("No category").as_ref()).fill,
                    button(&created, language.text("None").as_ref()).fill);
                click_text(&context, &mut app, acceptance, "Office");
                let changed = draw_with(&context, &mut app, vec![egui::Event::PointerGone], acceptance);
                assert_eq!(button(&changed, "Office").stroke, Stroke::new(2.0, app.accent()));
                assert_eq!(button(&changed, "Call").stroke, Stroke::NONE);
                assert!(ai_commands.try_recv().is_err(), "Category buttons never call AI");
            }
        }
    }
}

#[test]
fn settings_have_left_navigation_and_language_is_only_in_display_preferences() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.preferences.language = language;
        app.preferences_open = true;
        app.prompt_draft.recap.push_str(" Keep this unsaved edit.");
        let draft = app.prompt_draft.clone();
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                events, ..Default::default()
            }, |ui| app.preferences_dialog(ui.ctx()));
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing settings control: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                }]);
            }
        };
        draw(&mut app, vec![]);
        draw(&mut app, vec![]);
        let rect = context.memory(|memory| memory.area_rect(Id::new("preferences")).unwrap());
        assert!(rect.width() >= 900.0 && rect.height() >= 580.0, "{rect:?}");
        click(&mut app, language.text("AI prompts").as_ref());
        assert!(app.preferences_section == PreferencesSection::Prompts);
        click(&mut app, language.text("Display").as_ref());
        assert!(app.preferences_section == PreferencesSection::Display);
        assert_eq!(app.prompt_draft, draft);
        click(&mut app, if language == tomlook::i18n::Language::En { "Čeština" } else { "English" });
        assert_eq!(app.preferences.language, language.other());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences)
            if preferences.language == language.other()));
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.header(ui));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if matches!(text.galley.job.text.as_str(), "EN" | "CZ"))));
        output.textures_delta.clear();
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn follow_menu_submits_directly_and_decline_defaults_to_no_organizer_response() {
    use tomlook::recommendations::{Change, MeetingResponse};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.preferences.language = language;
        app.calendar_owner = Some("owner".into());
        app.workiq_ready = true;
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.organizer = "organizer@example.test".into();
        event.extra.insert("isOrganizer".into(), false.into());
        event.extra.insert("type".into(), "occurrence".into());
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {events, ..Default::default()},
                |ui| app.meeting_actions(ui, 0));
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing meeting action: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                }]);
            }
        };
        for enabled in [true, false] {
            event.extra.insert("categories".into(),
                if enabled { serde_json::json!(["Call"]) } else { serde_json::json!(["Call", "Follow"]) });
            app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
            click(&mut app, language.text(if enabled { "Follow" } else { "Stop following" }).as_ref());
            assert!(app.meeting_action.is_none(), "Follow must not open a confirmation");
            assert!(matches!(receiver.try_recv().unwrap(), Command::ConfirmMeetingAction {event: submitted,account,change}
                if submitted.id == event.id && account == "owner" && change == Change::Follow(enabled)));
            assert!(receiver.try_recv().is_err());
        }
        click(&mut app, language.text("Decline invitation").as_ref());
        assert!(matches!(app.meeting_action.as_ref().unwrap().change,
            Change::Respond {response: MeetingResponse::Decline,send_response: false,..}));
        assert!(receiver.try_recv().is_err(), "Decline still requires its existing confirmation");
        app.meeting_action = None;
        event.extra.insert("categories".into(), serde_json::json!(["Call"]));
        for unavailable in ["organizer", "metadata", "connection"] {
            let mut unavailable_event = event.clone();
            if unavailable == "organizer" {
                unavailable_event.extra.insert("isOrganizer".into(), true.into());
            } else if unavailable == "metadata" {
                unavailable_event.extra.remove("type");
            }
            app.workiq_ready = unavailable != "connection";
            app.calendar = Arc::new(Calendar::build(vec![unavailable_event], Default::default(), vec![]).unwrap());
            click(&mut app, language.text("Follow").as_ref());
            assert!(app.meeting_action.is_none(), "Unavailable Follow must stay visible without submitting");
            assert!(receiver.try_recv().is_err());
        }
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn mini_month_arrows_browse_without_changing_main_view_or_starting_ai() {
    let (mut app, _, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    let anchor = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
    for view in [View::Day, View::WorkWeek, View::Month] {
        app.anchor = anchor;
        app.mini_anchor = None;
        app.preferences.view = view;
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {events, ..Default::default()}, |ui| app.sidebar(ui));
            output.textures_delta.clear();
        };
        draw(&mut app, vec![]);
        let pos = context.data(|data| data.get_temp::<egui::Rect>(Id::new("mini-next-month")).unwrap()).center();
        for pressed in [true, false] {
            draw(&mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
            }]);
        }
        assert_eq!(app.anchor, anchor);
        assert_eq!(app.preferences.view, view);
        assert_eq!(app.mini_anchor.unwrap().month(), 11);
    }
    assert!(commands.try_recv().is_err());
}

#[test]
fn tentative_hatching_is_neutral_in_both_themes_and_notes_show_personal_evidence() {
    let mut app = Tomlook::initial(None, None, None);
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.status = "tentativelyAccepted".into();
    app.calendar = Arc::new(Calendar::build(vec![event], Default::default(), vec![]).unwrap());
    let context = egui::Context::default();
    for dark in [true, false] {
        app.preferences.dark = dark;
        context.set_visuals(if dark {egui::Visuals::dark()} else {egui::Visuals::light()});
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, 38.0));
            app.event_card(ui, 0, rect);
        });
        let fill = Color32::from_gray(if dark {31} else {247});
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.fill == fill)));
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::LineSegment {points,stroke}
            if stroke.width == 1.0 && points[0].x != points[1].x && points[0].y != points[1].y)));
        output.textures_delta.clear();
    }
    app.briefing = Some(serde_json::json!({"tomlook_recap":true,"summary":"A completed meeting",
        "preparation":["My assignment: deliver a demo"],"open_questions":["My mention: discussed the owner"],
        "talking_points":[],"risks":[],"gaps":[],"sources":[]}));
    let mut output = context.run_ui(egui::RawInput::default(), |ui| app.render_insights(ui, false));
    let mut text = String::new();
    for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
    assert!(text.contains("My tasks"));
    assert!(text.contains("Mentions of me"));
    assert!(text.contains("deliver a demo"));
    assert!(text.contains("discussed the owner"));
    output.textures_delta.clear();
}

#[test]
fn right_click_meeting_opens_paired_travel_confirmation_and_never_recreates_the_meeting() {
    for (language, organizer) in [
        (tomlook::i18n::Language::En, true), (tomlook::i18n::Language::Cs, true),
        (tomlook::i18n::Language::En, false), (tomlook::i18n::Language::Cs, false),
    ] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.id = "owned-meeting".into();
        event.title = "Existing owned meeting".into();
        event.is_all_day = false;
        event.extra.insert("type".into(), serde_json::json!("singleInstance"));
        event.extra.insert("isOrganizer".into(), serde_json::json!(organizer));
        event.status = if organizer { "organizer" } else { "accepted" }.into();
        event.extra.insert("sensitivity".into(), serde_json::json!("private"));
        app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner".into());
        app.workiq_ready = true;
        app.preferences.language = language;
        let context = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), Vec2::new(300.0, 90.0));
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1366.0, 1000.0))),
                events, ..Default::default()
            }, |ui| {
                app.event_card(ui, 0, rect);
                app.event_creation_dialog(ui.ctx());
            });
            output.textures_delta.clear();
            output
        };
        draw(&mut app, Vec::new());
        draw(&mut app, vec![egui::Event::PointerMoved(rect.center()),
            egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE }]);
        draw(&mut app, vec![egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE }]);
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, Vec::new());
            let output = draw(app, Vec::new());
            let point = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing travel action: {label}"));
            draw(app, vec![egui::Event::PointerMoved(point),
                egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        };
        click(&mut app, language.text("Add travel blocks").as_ref());
        let draft = app.event_draft.as_mut().unwrap();
        assert_eq!(draft.parent.as_ref().unwrap().id, event.id);
        assert_eq!(draft.travel_minutes, 30);
        draft.travel_minutes = 45;
        assert!(receiver.try_recv().is_err(), "Opening the menu/dialog must not write");
        let output = draw(&mut app, Vec::new());
        let mut rendered = String::new();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut rendered); }
        assert!(!rendered.contains(language.text("Event title").as_ref()));
        assert!(rendered.contains(language.text("The existing meeting stays unchanged; only two travel blocks are created.").as_ref()));
        click(&mut app, language.text("Create travel blocks").as_ref());
        let Command::CreateEvents { account, draft, parent: Some(parent) } = receiver.try_recv().unwrap() else { panic!("Travel-only approval required") };
        assert_eq!(account, "owner");
        assert_eq!(parent.id, event.id);
        assert_eq!(draft.start, event.start);
        assert_eq!(draft.end, event.end);
        assert_eq!(draft.travel_minutes, Some(45));
        assert!(draft.private && !draft.teams);
        assert!(draft.category.is_none());
        assert!(app.event_draft.is_none());
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn travel_context_action_supports_accepted_invitations_and_escape_never_confirms() {
    let (mut app, _, mut commands) = assistant_fixture();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.is_all_day = false;
    event.extra.insert("type".into(), serde_json::json!("singleInstance"));
    event.extra.insert("isOrganizer".into(), serde_json::json!(false));
    event.extra.insert("sensitivity".into(), serde_json::json!("normal"));
    event.status = "accepted".into();
    app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| app.meeting_actions(ui, 0));
    let mut text = String::new();
    for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
    output.textures_delta.clear();
    assert!(text.contains("Add travel blocks"));
    assert!(tomlook::event_creation::validate_travel_parent(&event).is_ok());
    app.open_travel_draft(event);
    let mut output = context.run_ui(egui::RawInput {
        events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    }, |ui| app.shortcuts(ui.ctx()));
    output.textures_delta.clear();
    assert!(app.event_draft.is_none());
    assert!(commands.try_recv().is_err());
}

#[test]
fn timeline_real_empty_lane_drag_does_not_scroll_and_opens_exact_interval() {
    let (mut app, _, mut commands) = assistant_fixture();
    let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
    app.anchor = day;
    app.preferences.view = View::Day;
    app.calendar = Arc::new(Calendar::build(Vec::new(), [day].into(), Vec::new()).unwrap());
    let context = egui::Context::default();
    let draw = |app: &mut Tomlook, events| {
        let mut output = context.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
            events, ..Default::default()
        }, |ui| app.timeline(ui));
        output.textures_delta.clear();
        output
    };
    draw(&mut app, Vec::new());
    let output = draw(&mut app, Vec::new());
    let nine = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == "09:00" =>
            Some(egui::pos2(text.pos.x + 200.0, text.pos.y - 5.0)),
        _ => None,
    }).expect("Nine o'clock is initially visible");
    let ten = nine + Vec2::new(0.0, 58.0);
    draw(&mut app, vec![egui::Event::PointerMoved(nine)]);
    draw(&mut app, vec![egui::Event::PointerButton {
        pos: nine, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE,
    }]);
    assert!(app.time_drag.is_some());
    draw(&mut app, vec![egui::Event::PointerMoved(ten)]);
    assert!(app.event_draft.is_none());
    draw(&mut app, vec![egui::Event::PointerButton {
        pos: ten, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE,
    }]);
    let draft = app.event_draft.as_ref().unwrap();
    assert_eq!((draft.start, draft.end), (540, 600));
    assert!(commands.try_recv().is_err());
}

#[test]
fn empty_time_click_drag_reverse_midnight_and_cancel_preserve_exact_selection_without_writes() {
    for (start, end, expected) in [
        (540, 600, (540, 600)),
        (600, 540, (540, 600)),
        (540, 540, (540, 570)),
        (1425, 1440, (1425, 1440)),
    ] {
        let (mut app, _, mut commands) = assistant_fixture();
        let context = egui::Context::default();
        let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let mut rect = egui::Rect::NOTHING;
        let mut output = context.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 1600.0))),
            ..Default::default()
        }, |ui| {
            rect = ui.allocate_exact_size(Vec2::new(400.0, 24.0 * 58.0), egui::Sense::hover()).0;
        });
        output.textures_delta.clear();
        let point = |minutes: u16| egui::pos2(rect.left() + 30.0, rect.top() + f32::from(minutes) / 60.0 * 58.0);
        let pointer = |minutes, pressed| egui::Event::PointerButton {
            pos: point(minutes), button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        };
        let mut output = context.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 1600.0))),
            events: vec![egui::Event::PointerMoved(point(start)), pointer(start, true),
                egui::Event::PointerMoved(point(end)), pointer(end, false)],
            ..Default::default()
        }, |ui| app.select_event_time(ui, day, rect, &[]));
        output.textures_delta.clear();
        let draft = app.event_draft.as_ref().expect("Release opens the editor");
        assert_eq!((draft.start, draft.end), expected);
        assert_eq!(draft.day, day);
        assert_eq!(draft.travel_minutes, 30);
        assert!(!draft.travel && !draft.private && !draft.teams);
        assert!(commands.try_recv().is_err());
        let mut output = context.run_ui(egui::RawInput {
            events: vec![egui::Event::Key { key: Key::Escape, physical_key: None,
                pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            ..Default::default()
        }, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(app.event_draft.is_none());
        assert!(commands.try_recv().is_err());
    }
}

#[test]
fn event_time_selection_ignores_existing_cards_and_lost_focus() {
    let (mut app, _, _) = assistant_fixture();
    let context = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(200.0, 600.0));
    let day = Local::now().date_naive();
    for focused in [true, false] {
        let mut output = context.run_ui(egui::RawInput {
            focused,
            events: vec![egui::Event::PointerMoved(rect.center()), egui::Event::PointerButton {
                pos: rect.center(), button: egui::PointerButton::Primary, pressed: true,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        }, |ui| app.select_event_time(ui, day, rect, &[rect]));
        output.textures_delta.clear();
        assert!(app.time_drag.is_none());
        assert!(app.event_draft.is_none());
    }
    app.time_drag = Some((day, 540, 600));
    let mut output = context.run_ui(egui::RawInput { focused: false, ..Default::default() },
        |ui| app.select_event_time(ui, day, rect, &[]));
    output.textures_delta.clear();
    assert!(app.time_drag.is_none());
}

#[test]
fn event_editor_selects_personal_privacy_and_submits_one_exact_explicit_creation_in_both_languages() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.category = "osobní".into();
        event.extra.insert("categories".into(), serde_json::json!(["osobní", "office"]));
        app.calendar = Arc::new(Calendar::build(vec![event], Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner".into());
        app.workiq_ready = true;
        app.preferences.language = language;
        app.open_event_draft(Local::now().date_naive(), 540, 600);
        app.event_draft.as_mut().unwrap().title = "Synthetic private appointment".into();
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1366.0, 1000.0))),
                events, ..Default::default()
            }, |ui| app.event_creation_dialog(ui.ctx()));
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, Vec::new());
            let output = draw(app, Vec::new());
            let position = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing creation control: {label}"));
            draw(app, vec![egui::Event::PointerMoved(position),
                egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        };
        click(&mut app, language.text("Private event").as_ref());
        assert!(app.event_draft.as_ref().unwrap().private);
        assert_eq!(app.event_draft.as_ref().unwrap().category.as_deref(), Some("osobní"));
        click(&mut app, language.text("Teams meeting").as_ref());
        click(&mut app, language.text("Add travel blocks before and after").as_ref());
        assert!(receiver.try_recv().is_err(), "Editing must never submit writes");
        click(&mut app, language.text("Create event").as_ref());
        let Command::CreateEvents { account, draft, parent } = receiver.try_recv().unwrap() else { panic!("Expected exact creation approval") };
        assert!(parent.is_none());
        assert_eq!(account, "owner");
        assert_eq!(draft.title, "Synthetic private appointment");
        assert!(draft.private && draft.teams);
        assert_eq!(draft.category.as_deref(), Some("osobní"));
        assert_eq!(draft.travel_minutes, Some(30));
        assert!(app.event_draft.is_none());
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err(), "No model call for event creation");
    }
}

use super::*;
use super::ask_panel::ASSISTANT_CONTEXT_LIMIT;
use super::calendar::{MIN_EVENT_LANE_WIDTH, timeline_content_width};
use super::connections::{status_text, version_label};
use super::palette::{PaletteCommand, command_matches, command_matches_in};
use super::recommendations::messages_button;
use super::settings::{ACTIVITY_ROW_HEIGHT, activity_row};
use tomlook::proposals::{Kind, Proposal};

#[test]
fn recommendation_cards_separate_action_meeting_reason_and_adjacent_travel_sides() {
    use tomlook::recommendations::{Kind as RecommendationKind, Recommendation, Suggestion};
    let (mut app, _, mut commands) = assistant_fixture();
    let now = Local::now().fixed_offset();
    let monday = tomlook::calendar::monday(now.date_naive()) + chrono::Duration::days(7);
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.start = monday
        .and_hms_opt(14, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
        .fixed_offset();
    event.end = event.start + chrono::Duration::hours(1);
    event.is_all_day = false;
    event.title = "Client workshop".into();
    event.location = "Client office".into();
    event.status = "accepted".into();
    event.category = "mimo office".into();
    event
        .extra
        .insert("categories".into(), serde_json::json!(["mimo office"]));
    app.calendar =
        Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
    app.calendar_owner = Some("owner".into());
    let context = egui::Context::default();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        for (kind, heading, time) in [
            (
                RecommendationKind::Preparation,
                "Make time to prepare",
                "13:30",
            ),
            (
                RecommendationKind::Travel,
                "Reserve travel before meeting",
                "13:30",
            ),
            (
                RecommendationKind::TravelAfter,
                "Reserve travel after meeting",
                "15:00",
            ),
        ] {
            let record = Recommendation::from_insights(
                &event,
                "owner",
                Suggestion {
                    kind,
                    reason: "Explain the architecture and demonstrate the prototype.".into(),
                    reason_cs: "Vysvetli architekturu a ukaz prototyp.".into(),
                    category: None,
                    minutes: Some(30),
                },
                now,
            )
            .unwrap();
            let mut text = String::new();
            for _ in 0..3 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(800.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
                );
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            for label in [
                heading,
                "Meeting",
                "Why?",
                "Add to calendar",
                "Details and checks",
            ] {
                assert!(
                    text.contains(language.text(label).as_ref()),
                    "{language:?}, {kind:?}: {text}"
                );
            }
            assert!(text.contains("Client workshop"));
            assert!(text.contains(time));
            assert!(
                !text.contains(
                    language
                        .text("Suggested slots use weekdays, 08:00-18:00 local time.")
                        .as_ref()
                )
            );
        }
    }
    assert!(commands.try_recv().is_err());
}

#[test]
fn neutral_bell_replaces_the_triangle_and_remains_keyboard_accessible() {
    let context = egui::Context::default();
    let mut identity = None;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for warning in [false, true] {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                let response = inbox_button(ui, language, warning, 3);
                identity = Some(response.id);
            });
            assert!(!output.shapes.iter().any(|shape|
                matches!(&shape.shape, egui::epaint::Shape::Path(path) if path.points.len() == 3)));
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(!text.contains('!'));
            output.textures_delta.clear();
        }
    }
    context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
    let mut clicked = false;
    let mut output = context.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            clicked = inbox_button(ui, tomlook::i18n::Language::Cs, true, 3).clicked();
        },
    );
    output.textures_delta.clear();
    assert!(clicked);
}

#[test]
fn recommendation_navigation_renders_one_detail_and_keyboard_does_not_submit() {
    use tomlook::recommendations::{Kind, Recommendation, Suggestion};
    let (mut app, _, mut commands) = assistant_fixture();
    let now = Local::now().fixed_offset();
    let mut events = Vec::new();
    let mut inbox = BTreeMap::new();
    for index in 0..12 {
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.id = format!("meeting-{index}");
        event.title = format!("Synthetic meeting {index}");
        event.start = now + chrono::Duration::hours(2 + index);
        event.end = event.start + chrono::Duration::minutes(30);
        event.is_all_day = false;
        event.status = "accepted".into();
        let record = Recommendation::from_insights(
            &event,
            "owner",
            Suggestion {
                kind: Kind::MissingMaterials,
                reason: format!("EXPLANATION_{index}_ONLY"),
                reason_cs: format!("VYSVETLENI_{index}_JEN"),
                category: None,
                minutes: None,
            },
            now,
        )
        .unwrap();
        inbox.insert(record.id.clone(), record);
        events.push(event);
    }
    app.calendar = Arc::new(Calendar::build(events, Default::default(), vec![]).unwrap());
    app.calendar_owner = Some("owner".into());
    let mut activity = (*app.activity).clone();
    activity.error = Some("Saved diagnostic issue.".into());
    activity.recommendations = Arc::new(inbox);
    activity.recommendation_view = Arc::new(tomlook::recommendations::inbox_view(
        &app.calendar,
        &activity.recommendations,
        app.calendar_owner.as_deref(),
        now,
        &BTreeSet::new(),
    ));
    app.activity = Arc::new(activity);
    app.messages_open = true;
    let context = egui::Context::default();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        app.recommendation_selected = None;
        app.recommendation_cursor = 0;
        let mut final_text = String::new();
        for _ in 0..30 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1366.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| app.overlays(ui.ctx()),
            );
            final_text.clear();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut final_text);
            }
            output.textures_delta.clear();
        }
        let explanation = if language == tomlook::i18n::Language::En {
            "EXPLANATION"
        } else {
            "VYSVETLENI"
        };
        assert!(final_text.contains(&format!("{explanation}_0_")));
        assert!(!final_text.contains(&format!("{explanation}_1_")));
        assert!(!final_text.contains(language.text("Resume other queued jobs").as_ref()));
        let rect = context.memory(|memory| {
            memory
                .area_rect(Id::new("status-messages-window-v3"))
                .unwrap()
        });
        assert!(
            rect.height() <= 710.0,
            "Recommendation window grew: {rect:?}"
        );
        for cursor in 1..12 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1366.0, 900.0),
                    )),
                    events: vec![egui::Event::Key {
                        key: Key::ArrowDown,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::ALT,
                    }],
                    ..Default::default()
                },
                |ui| app.overlays(ui.ctx()),
            );
            assert_eq!(app.recommendation_cursor, cursor);
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(text.contains(&format!("{explanation}_{cursor}_")), "{text}");
            output.textures_delta.clear();
        }
        assert_eq!(
            app.recommendation_selected.as_deref(),
            app.activity
                .recommendations
                .values()
                .find(|record| record.event_title == "Synthetic meeting 11")
                .map(|record| record.id.as_str())
        );
    }
    app.recommendations_tab = false;
    let mut output = context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
    output.textures_delta.clear();
    let mut text = String::new();
    for shape in &output.shapes {
        append_shape_text(&shape.shape, &mut text);
    }
    assert!(
        text.contains(
            app.preferences
                .language
                .text("Resume other queued jobs")
                .as_ref()
        )
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn conflict_cards_name_counterparts_and_render_the_local_overlap_not_raw_utc_reason() {
    use tomlook::recommendations::{Kind, Recommendation, Suggestion};
    let (mut app, _, mut commands) = assistant_fixture();
    let now = Local::now().fixed_offset();
    let day = tomlook::calendar::monday(now.date_naive()) + chrono::Duration::days(7);
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.start = day
        .and_hms_opt(17, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
        .fixed_offset();
    event.end = event.start + chrono::Duration::hours(3);
    event.is_all_day = false;
    event.status = "accepted".into();
    let mut overlap = event.clone();
    overlap.id = "transfer".into();
    overlap.title = "Synthetic transfer reserve".into();
    overlap.start = (event.start + chrono::Duration::minutes(15))
        .with_timezone(&chrono::FixedOffset::east_opt(0).unwrap());
    overlap.end = event.start + chrono::Duration::hours(4);
    app.calendar = Arc::new(
        Calendar::build(vec![event.clone(), overlap], Default::default(), vec![]).unwrap(),
    );
    app.calendar_owner = Some("owner".into());
    let record = Recommendation::from_insights(
        &event,
        "owner",
        Suggestion {
            kind: Kind::PriorityConflict,
            reason: "Overlap from 15:15 UTC.".into(),
            reason_cs: "Prekryv od 15:15 UTC.".into(),
            category: None,
            minutes: None,
        },
        now,
    )
    .unwrap();
    let context = egui::Context::default();
    let mut text = String::new();
    for _ in 0..3 {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
        );
        text.clear();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        output.textures_delta.clear();
    }
    assert!(text.contains("Synthetic transfer reserve"));
    assert!(text.contains("Overlap: 17:15"));
    assert!(text.contains("17:15 - 21:00"));
    assert!(
        text.contains("20:00"),
        "The intersection must remain secondary to the full counterpart time"
    );
    assert!(!text.contains("15:15 UTC"));
    assert!(commands.try_recv().is_err());
}

#[test]
fn recommendation_meeting_links_and_conflict_actions_are_navigable_and_require_confirmation() {
    use tomlook::recommendations::{Change, MeetingResponse};
    let (mut app, _, mut ai_commands) = assistant_fixture();
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    app.worker = Some(Worker {
        commands: sender,
        notices: Arc::new(Default::default()),
        activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(Default::default()),
        done: None,
    });
    let now = Local::now().fixed_offset();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.title = "Synthetic meeting link".into();
    event.start = now + chrono::Duration::days(7);
    event.end = event.start + chrono::Duration::hours(1);
    event.is_all_day = false;
    event.status = "accepted".into();
    event.organizer = "organizer@example.test".into();
    event
        .extra
        .insert("isOrganizer".into(), serde_json::json!(false));
    event
        .extra
        .insert("type".into(), serde_json::json!("occurrence"));
    app.calendar =
        Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
    app.calendar_owner = Some("owner".into());
    app.workiq_ready = true;
    app.preferences.view = View::Week;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(800.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.recommendation_meeting(ui, &event, true),
            );
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing button: {label}"));
            draw(
                app,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        };
        let output = draw(&mut app, vec![]);
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        assert!(text.contains(&meeting_date_label(&event, language)));
        for response in [MeetingResponse::Tentative, MeetingResponse::Decline] {
            app.messages_open = true;
            click(&mut app, language.text(response.label()).as_ref());
            let draft = app.meeting_action.take().unwrap();
            assert_eq!(draft.event.id, event.id);
            assert!(
                matches!(draft.change, Change::Respond { response: selected, send_response, .. }
                    if selected == response && send_response == (response != MeetingResponse::Decline))
            );
            assert!(!app.messages_open);
            assert!(
                receiver.try_recv().is_err(),
                "Opening confirmation must not submit a write"
            );
        }
        app.messages_open = true;
        app.query = "hidden".into();
        app.category = "other".into();
        click(&mut app, &event.title);
        assert_eq!(app.selected, app.calendar.event_index(&event.id));
        assert_eq!(app.anchor, event.calendar_dates().0);
        assert!(app.query.is_empty() && app.category.is_empty());
        assert!(app.insights_expanded && !app.messages_open && app.scroll_meeting);
        assert!(
            matches!(receiver.try_recv().unwrap(), Command::Detail { id, .. } if id == event.id)
        );
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn missing_material_cards_show_actual_contacts_in_both_languages_without_outreach() {
    use tomlook::recommendations::{Kind, Recommendation, Suggestion};
    let (mut app, _, mut commands) = assistant_fixture();
    let now = Local::now().fixed_offset();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.start = now + chrono::Duration::hours(2);
    event.end = event.start + chrono::Duration::hours(1);
    event.is_all_day = false;
    event.organizer = "Synthetic organizer <agenda@partner.example>".into();
    event.attendees = vec![
        "Synthetic colleague <colleague@home.example>".into(),
        "Own alias <alias@home.example>".into(),
    ];
    app.preferences.work_email = "next-account@tenant.example".into();
    app.calendar_owner = Some("me@home.example".into());
    Arc::make_mut(&mut app.activity).identity =
        Some(Arc::new(tomlook::workiq::connection::Identity {
            account: "me@home.example".into(),
            mail: Some("alias@home.example".into()),
            display_name: None,
        }));
    app.calendar =
        Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
    let record = Recommendation::from_insights(
        &event,
        "me@home.example",
        Suggestion {
            kind: Kind::MissingMaterials,
            reason: "Ask for the workshop agenda.".into(),
            reason_cs: "Pozadej o agendu workshopu.".into(),
            category: None,
            minutes: None,
        },
        now,
    )
    .unwrap();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        let context = egui::Context::default();
        let mut text = String::new();
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(800.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
            );
            text.clear();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(text.contains(language.text("Who to ask").as_ref()));
        assert!(text.contains("agenda@partner.example"));
        assert!(text.contains("colleague@home.example"));
        assert!(!text.contains("alias@home.example"));
        assert!(text.contains(language.text("Same email domain").as_ref()));
        assert!(!text.contains("Send"));
        assert!(commands.try_recv().is_err());
    }
}

#[test]
fn category_transition_has_a_shared_baseline_and_a_vector_arrow_without_a_glyph_box() {
    use tomlook::recommendations::{Kind, Recommendation, Suggestion};
    let (mut app, _, mut commands) = assistant_fixture();
    let now = Local::now().fixed_offset();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.start = now + chrono::Duration::hours(2);
    event.end = event.start + chrono::Duration::hours(1);
    event.is_all_day = false;
    event.status = "accepted".into();
    event.category.clear();
    event
        .extra
        .insert("categories".into(), serde_json::json!([]));
    let mut reference = event.clone();
    reference.id = "category-reference".into();
    reference.status = "cancelled".into();
    reference.category = "nejist\u{e9}".into();
    reference
        .extra
        .insert("categories".into(), serde_json::json!(["nejist\u{e9}"]));
    app.calendar = Arc::new(
        Calendar::build(vec![event.clone(), reference], Default::default(), vec![]).unwrap(),
    );
    app.calendar_owner = Some("owner".into());
    let record = Recommendation::from_insights(
        &event,
        "owner",
        Suggestion {
            kind: Kind::Category,
            reason: "Explicitly tentative.".into(),
            reason_cs: "Nejista udalost.".into(),
            category: Some("nejist\u{e9}".into()),
            minutes: None,
        },
        now,
    )
    .unwrap();
    let context = egui::Context::default();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        let mut centers = (None, None);
        let mut arrow_segments = 0;
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(800.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| app.recommendation_card(ui, &record, &app.calendar.clone(), now),
            );
            centers = (None, None);
            arrow_segments = 0;
            for shape in &output.shapes {
                match &shape.shape {
                    egui::epaint::Shape::Text(text) => {
                        let value = &text.galley.job.text;
                        assert!(!value.contains('\u{2192}'));
                        let center = text.pos.y + text.galley.rect.center().y;
                        if value == language.text("None").as_ref() {
                            centers.0 = Some(center);
                        }
                        if value == "nejist\u{e9}" {
                            centers.1 = Some(center);
                        }
                    }
                    egui::epaint::Shape::LineSegment { stroke, .. } if stroke.width == 1.6 => {
                        arrow_segments += 1
                    }
                    _ => {}
                }
            }
            output.textures_delta.clear();
        }
        assert_eq!(arrow_segments, 3);
        assert!(
            (centers.0.unwrap() - centers.1.unwrap()).abs() < 1.0,
            "{centers:?}"
        );
    }
    assert!(commands.try_recv().is_err());
}

#[test]
fn inbox_is_bilingual_quiet_and_unknown_outcomes_have_no_confirmation_action() {
    let (mut app, _, mut commands) = assistant_fixture();
    let now = Local::now().fixed_offset();
    let mut events = tomlook::calendar::fixture(2);
    events[0].start = now + chrono::Duration::hours(2);
    events[0].end = events[0].start + chrono::Duration::hours(1);
    events[0].is_all_day = false;
    events[0].category = "Call".into();
    events[0]
        .extra
        .insert("categories".into(), serde_json::json!(["Call"]));
    events[1].category = "Office".into();
    events[1].start = now + chrono::Duration::hours(4);
    events[1].end = events[1].start + chrono::Duration::hours(1);
    events[1]
        .extra
        .insert("categories".into(), serde_json::json!(["Office"]));
    app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
    app.calendar_owner = Some("owner".into());
    app.workiq_ready = true;
    let mut recommendation = tomlook::recommendations::Recommendation::from_insights(
        &app.calendar.events[0],
        "owner",
        tomlook::recommendations::Suggestion {
            kind: tomlook::recommendations::Kind::Category,
            reason: "Office visit.".into(),
            reason_cs: "Osobni navsteva kancelare.".into(),
            category: Some("Office".into()),
            minutes: None,
        },
        now,
    )
    .unwrap();
    let context = egui::Context::default();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        for state in [
            tomlook::recommendations::State::Pending,
            tomlook::recommendations::State::Review,
        ] {
            recommendation.state = state;
            let mut activity = (*app.activity).clone();
            activity.recommendations = Arc::new(std::collections::BTreeMap::from([(
                recommendation.id.clone(),
                recommendation.clone(),
            )]));
            activity.recommendation_view = Arc::new(tomlook::recommendations::inbox_view(
                &app.calendar,
                &activity.recommendations,
                app.calendar_owner.as_deref(),
                now,
                &BTreeSet::new(),
            ));
            app.activity = Arc::new(activity);
            for open in [false, true] {
                app.messages_open = open;
                let mut text = String::new();
                for _ in 0..3 {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(1366.0, 768.0),
                            )),
                            ..Default::default()
                        },
                        |ui| app.overlays(ui.ctx()),
                    );
                    for shape in &output.shapes {
                        append_shape_text(&shape.shape, &mut text);
                    }
                    output.textures_delta.clear();
                }
                assert_eq!(
                    text.contains(language.text("Recommendations").as_ref()),
                    open,
                    "Inbox visibility: {language:?}, {state:?}, open={open}: {text}"
                );
                assert_eq!(
                    text.contains(language.text("Use category").as_ref()),
                    open && state == tomlook::recommendations::State::Pending,
                    "Confirmation visibility: {language:?}, {state:?}, open={open}: {text}"
                );
            }
        }
    }
    assert!(commands.try_recv().is_err());
}

#[test]
fn follow_up_opens_local_draft_request_without_submitting_or_erasing_questions() {
    let (mut app, _, mut commands) = assistant_fixture();
    let now = chrono::Local::now().fixed_offset();
    let mut events = tomlook::calendar::fixture(1);
    events[0].start = now - chrono::Duration::hours(2);
    events[0].end = now - chrono::Duration::hours(1);
    app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
    app.calendar_owner = Some("owner".into());
    let recommendation = tomlook::recommendations::Recommendation::from_insights(
        &app.calendar.events[0],
        "owner",
        tomlook::recommendations::Suggestion {
            kind: tomlook::recommendations::Kind::FollowUp,
            reason: "Review the meeting outcomes.".into(),
            reason_cs: "Zkontroluj zavery schuzky.".into(),
            category: None,
            minutes: None,
        },
        now,
    )
    .unwrap();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        app.question.clear();
        app.messages_open = true;
        app.prepare_follow_up(&recommendation);
        assert!(app.assistant && app.focus_assistant);
        assert!(!app.messages_open);
        assert_eq!(
            app.assistant_context.as_ref().unwrap().id,
            recommendation.event_id
        );
        assert!(
            app.question
                .contains(if language == tomlook::i18n::Language::En {
                    "placeholders"
                } else {
                    "zástupná místa"
                })
        );
        app.question = "Keep my unsent question".into();
        app.prepare_follow_up(&recommendation);
        assert_eq!(app.question, "Keep my unsent question");
        assert!(app.ai_error.as_ref().unwrap().contains("was preserved"));
    }
    app.calendar_owner = Some("other-owner".into());
    app.prepare_follow_up(&recommendation);
    assert_eq!(app.question, "Keep my unsent question");
    assert!(app.error.as_ref().unwrap().contains("no longer available"));
    assert!(app.active_request.is_none());
    assert!(commands.try_recv().is_err());
}

#[test]
fn right_clicking_a_meeting_opens_actions_without_selecting_or_submitting() {
    let (mut app, _, mut commands) = assistant_fixture();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.start = Local::now().fixed_offset() + chrono::Duration::hours(1);
    event.end = event.start + chrono::Duration::hours(1);
    event.extra.insert("onlineMeeting".into(), serde_json::json!({
        "joinUrl": "https://teams.microsoft.com/l/meetup-join/synthetic",
    }));
    event
        .extra
        .insert("type".into(), serde_json::json!("singleInstance"));
    app.calendar =
        Arc::new(Calendar::build(vec![event], Default::default(), Vec::new()).unwrap());
    app.calendar_owner = Some("owner".into());
    let context = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), Vec2::new(300.0, 90.0));
    let point = rect.center();
    let mut text = String::new();
    for events in [
        Vec::new(),
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Vec::new(),
        Vec::new(),
    ] {
        let mut output = context.run_ui(
            egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1366.0, 768.0),
                )),
                ..Default::default()
            },
            |ui| app.event_card(ui, 0, rect),
        );
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        output.textures_delta.clear();
    }
    assert!(
        text.contains(app.preferences.language.text("Change category").as_ref()),
        "{text}"
    );
    assert!(
        text.contains(app.preferences.language.text("Join Teams meeting").as_ref()),
        "{text}"
    );
    assert!(app.selected.is_none());
    assert!(app.meeting_action.is_none());
    assert!(commands.try_recv().is_err());
}

#[test]
fn meeting_menu_join_teams_uses_the_clicked_meeting_offline_and_reports_opening_errors() {
    let join_url = "https://teams.microsoft.com/l/meetup-join/synthetic";
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for (url, start_minutes, end_minutes, visible) in [
            (Some(join_url), 60, 120, true),
            (Some(join_url), -60, 60, true),
            (Some(join_url), -120, -60, false),
            (None, 60, 120, false),
            (Some("javascript:alert(1)"), 60, 120, false),
            (Some("http://teams.example.invalid/meeting"), 60, 120, true),
        ] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, writes) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            app.preferences.language = language;
            app.ai_ready = false;
            app.workiq_ready = false;
            let mut selected = tomlook::calendar::fixture(1).remove(0);
            selected.id = "selected-without-link".into();
            selected.extra.remove("onlineMeeting");
            let mut event = selected.clone();
            event.id = "right-clicked-meeting".into();
            event.title = "Clicked meeting".into();
            let now = Local::now().fixed_offset();
            event.start = now + chrono::Duration::minutes(start_minutes);
            event.end = now + chrono::Duration::minutes(end_minutes);
            if let Some(url) = url {
                event.extra.insert("onlineMeeting".into(), serde_json::json!({"joinUrl":url}));
            }
            app.calendar = Arc::new(Calendar::build(vec![selected.clone(), event.clone()], Default::default(), vec![]).unwrap());
            app.selected = app.calendar.event_index(&selected.id);
            let original_selection = app.selected;
            let index = app.calendar.event_index(&event.id).unwrap();
            app.meeting_menu = Some(event.id.clone());
            let context = egui::Context::default();
            let show: Show = &|app, ui| app.meeting_actions(ui, index);
            let text = settled(&context, &mut app, show);
            let label = language.text("Join Teams meeting");
            assert_eq!(text.contains(label.as_ref()), visible, "{url:?} / {start_minutes} / {end_minutes} / {language:?}: {text}");
            if url.is_some_and(|url| url.starts_with("http:")) {
                click_text(&context, &mut app, show, label.as_ref());
                assert_eq!(app.error.as_deref(), Some("Only HTTPS sign-in addresses can be opened"));
                assert!(app.meeting_menu.is_none(), "The clicked action closes the meeting menu");
            }
            assert_eq!(app.selected, original_selection, "Joining uses the clicked meeting, not the selection");
            assert!(ai_commands.try_recv().is_err(), "Joining never starts AI");
            assert!(writes.try_recv().is_err(), "Joining never submits a calendar write");
        }
    }
}

#[test]
fn manual_menu_and_dialog_are_bilingual_local_and_escape_never_submits() {
    let (mut app, _, mut ai_commands) = assistant_fixture();
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    app.worker = Some(Worker {
        commands: sender,
        notices: Arc::new(Default::default()),
        activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(Default::default()),
        done: None,
    });
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event
        .extra
        .insert("type".into(), serde_json::json!("occurrence"));
    event
        .extra
        .insert("isOrganizer".into(), serde_json::json!(false));
    event.organizer = "organizer@example.test".into();
    app.calendar =
        Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
    app.calendar_owner = Some("owner".into());
    app.selected = Some(0);
    app.workiq_ready = true;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        let context = egui::Context::default();
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![key(Key::F10, Modifiers::SHIFT)],
                ..Default::default()
            },
            |ui| app.shortcuts(ui.ctx()),
        );
        output.textures_delta.clear();
        assert_eq!(app.meeting_menu.as_deref(), Some(event.id.as_str()));
        let mut text = String::new();
        for _ in 0..3 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                app.meeting_action_dialog(ui.ctx())
            });
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(
            text.contains(language.text("Accept invitation").as_ref()),
            "{text}"
        );
        assert!(
            text.contains(language.text("Decline invitation").as_ref()),
            "{text}"
        );
        assert!(
            text.contains(&meeting_date_label(&event, language)),
            "{text}"
        );
        assert!(!text.contains(language.text("Cancel organized meeting").as_ref()));
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err());
        let context = egui::Context::default();
        app.choose_meeting_action(
            event.clone(),
            tomlook::recommendations::Change::Respond {
                response: tomlook::recommendations::MeetingResponse::Decline,
                comment: "Unsent draft comment.".into(),
                send_response: false,
                category: None,
            },
        );
        let mut text = String::new();
        for _ in 0..3 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                app.meeting_action_dialog(ui.ctx())
            });
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(
            text.contains(language.text("Decline invitation").as_ref()),
            "{text}"
        );
        assert!(
            text.contains(language.text("This occurrence only").as_ref()),
            "{text}"
        );
        assert!(!text.contains("organizer@example.test"), "{text}");
        assert!(
            !text.contains(language.text("Comment (optional)").as_ref()),
            "{text}"
        );
        assert!(!text.contains(language.text("Account").as_ref()), "{text}");
        assert!(
            !text.contains(
                language
                    .text("Confirmation sends this response and comment.")
                    .as_ref()
            ),
            "{text}"
        );
        assert_eq!(
            app.meeting_action.as_ref().unwrap().change,
            tomlook::recommendations::Change::Respond {
                response: tomlook::recommendations::MeetingResponse::Decline,
                comment: "Unsent draft comment.".into(),
                send_response: false,
                category: None,
            }
        );
        assert!(receiver.try_recv().is_err());
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![key(Key::Escape, Modifiers::NONE)],
                ..Default::default()
            },
            |ui| app.shortcuts(ui.ctx()),
        );
        output.textures_delta.clear();
        assert!(app.meeting_action.is_none());
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn category_menu_submits_once_without_confirmations_or_opening_messages() {
    use tomlook::recommendations::Change;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for clear in [false, true] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, receiver) = std::sync::mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender,
                notices: Arc::new(Default::default()),
                activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()),
                done: None,
            });
            let mut event = tomlook::calendar::fixture(1).remove(0);
            event
                .extra
                .insert("type".into(), serde_json::json!("occurrence"));
            event
                .extra
                .insert("categories".into(), serde_json::json!(["call", "office"]));
            app.calendar = Arc::new(
                Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap(),
            );
            app.calendar_owner = Some("owner".into());
            app.preferences.language = language;
            app.workiq_ready = true;
            app.meeting_menu = Some(event.id.clone());
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1366.0, 768.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.meeting_action_dialog(ui.ctx()),
                );
                output.textures_delta.clear();
                output
            };
            let click = |app: &mut Tomlook, label: &str| {
                draw(app, Vec::new());
                let output = draw(app, Vec::new());
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing menu item: {label}"));
                draw(
                    app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
            };
            click(&mut app, language.text("Change category").as_ref());
            assert!(
                receiver.try_recv().is_err(),
                "Opening the submenu must not write"
            );
            let category = if clear {
                language.text("Remove category")
            } else {
                "office".into()
            };
            click(&mut app, category.as_ref());
            match receiver.try_recv().unwrap() {
                Command::ConfirmMeetingAction {
                    event: selected,
                    account,
                    change,
                } => {
                    assert_eq!(
                        serde_json::to_value(*selected).unwrap(),
                        serde_json::to_value(&event).unwrap()
                    );
                    assert_eq!(account, "owner");
                    assert_eq!(
                        change,
                        if clear {
                            Change::ClearCategory
                        } else {
                            Change::Category("office".into())
                        }
                    );
                }
                _ => panic!("Category selection must dispatch the exact native change"),
            }
            for _ in 0..3 {
                draw(&mut app, Vec::new());
            }
            assert!(app.meeting_action.is_none());
            assert!(app.meeting_menu.is_none());
            assert!(!app.messages_open);
            assert!(
                receiver.try_recv().is_err(),
                "Selection must submit only once"
            );
            assert!(
                ai_commands.try_recv().is_err(),
                "No model request belongs in a manual category change"
            );
        }
    }
}

#[test]
fn compact_meeting_confirmation_submits_once_and_stays_quiet() {
    use tomlook::recommendations::{Change, MeetingResponse};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for response in [
            Some(MeetingResponse::Accept),
            Some(MeetingResponse::Tentative),
            Some(MeetingResponse::Decline),
            Some(MeetingResponse::Cancel),
            None,
        ] {
            for send_response in [false, true] {
                let (mut app, _, mut ai_commands) = assistant_fixture();
                let (sender, receiver) = std::sync::mpsc::sync_channel(16);
                app.worker = Some(Worker {
                    commands: sender,
                    notices: Arc::new(Default::default()),
                    activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
                    stop: Arc::new(Default::default()),
                    done: None,
                });
                let mut event = tomlook::calendar::fixture(1).remove(0);
                event
                    .extra
                    .insert("type".into(), serde_json::json!("occurrence"));
                let organizer = response.is_none() || response == Some(MeetingResponse::Cancel);
                event
                    .extra
                    .insert("isOrganizer".into(), serde_json::json!(organizer));
                event.organizer = "organizer@example.test".into();
                if response.is_none() {
                    event.attendees.clear();
                }
                app.calendar = Arc::new(
                    Calendar::build(vec![event.clone()], Default::default(), Vec::new())
                        .unwrap(),
                );
                app.calendar_owner = Some("owner".into());
                app.preferences.language = language;
                app.workiq_ready = true;
                let action = response.map(MeetingResponse::label).unwrap_or("Delete");
                let comment = "Preview comment.";
                app.choose_meeting_action(
                    event,
                    response
                        .map(|response| Change::Respond {
                            response,
                            comment: comment.into(),
                            send_response,
                            category: None,
                        })
                        .unwrap_or(Change::Delete),
                );
                let context = egui::Context::default();
                let draw = |app: &mut Tomlook, events| {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(1366.0, 768.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| app.meeting_action_dialog(ui.ctx()),
                    );
                    output.textures_delta.clear();
                    output
                };
                draw(&mut app, Vec::new());
                draw(&mut app, Vec::new());
                let output = draw(&mut app, Vec::new());
                let mut text = String::new();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                assert!(
                    text.lines().count() <= if response == Some(MeetingResponse::Accept) { 16 } else { 12 },
                    "Confirmation has too much text: {text}"
                );
                assert!(!text.contains("organizer@example.test"), "{text}");
                assert!(!text.contains(language.text("Account").as_ref()), "{text}");
                assert_eq!(
                    text.contains(language.text("Notifies all attendees.").as_ref()),
                    response == Some(MeetingResponse::Cancel)
                );
                assert!(
                    receiver.try_recv().is_err(),
                    "Opening the confirmation must not write"
                );
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text)
                            if text.galley.job.text == language.text(action) =>
                        {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing confirm button: {text}"));
                draw(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
                match receiver.try_recv().unwrap() {
                    Command::ConfirmMeetingAction { change, .. } => assert_eq!(
                        change,
                        response
                            .map(|response| Change::Respond {
                                response,
                                comment: if send_response || response == MeetingResponse::Cancel
                                {
                                    comment.into()
                                } else {
                                    String::new()
                                },
                                send_response: send_response
                                    || response == MeetingResponse::Cancel,
                                category: None,
                            })
                            .unwrap_or(Change::Delete)
                    ),
                    _ => panic!("Confirmation must dispatch the exact native action"),
                }
                for _ in 0..3 {
                    draw(&mut app, Vec::new());
                }
                assert!(app.meeting_action.is_none());
                assert!(!app.messages_open);
                assert!(receiver.try_recv().is_err());
                assert!(ai_commands.try_recv().is_err());
            }
        }
    }
}

#[test]
fn warnings_only_render_after_opening_messages_and_never_submit_research() {
    let (mut app, _, mut commands) = assistant_fixture();
    app.error = Some("AI executor stopped before an outcome was recorded; inspect saved evidence before explicit retry".into());
    let context = egui::Context::default();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        for opened in [false, true] {
            app.messages_open = opened;
            let mut text = String::new();
            for _ in 0..2 {
                let mut output =
                    context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            assert_eq!(
                text.contains(language.text(app.error.as_ref().unwrap()).as_ref()),
                opened
            );
        }
        app.shortcuts(&context);
        assert!(commands.try_recv().is_err());
    }
}

#[test]
fn status_text_has_no_idle_box_and_keeps_keyboard_activation_across_languages() {
    let context = egui::Context::default();
    let mut identity = None;
    let mut rect = egui::Rect::NOTHING;
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        let response = status_text(ui, "status-fixture", "WorkIQ: connected");
        identity = Some(response.id);
        rect = response.rect;
    });
    assert!(!output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::epaint::Shape::Rect(shape) if shape.rect == rect && shape.fill != Color32::TRANSPARENT)
    }));
    output.textures_delta.clear();
    context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
    let mut clicked = false;
    let mut output = context.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            let response = status_text(
                ui,
                "status-fixture",
                tomlook::i18n::Language::Cs.text("WorkIQ: connected"),
            );
            assert_eq!(response.id, identity.unwrap());
            clicked = response.clicked();
        },
    );
    output.textures_delta.clear();
    assert!(clicked);
}

fn status_bar_fixture() -> (Tomlook, tokio::sync::mpsc::Receiver<ai::Command>) {
    let (mut app, _, commands) = assistant_fixture();
    app.loading = false;
    app.ai_ready = false;
    app.workiq_ready = true;
    app.preferences.msx_enabled = true;
    app.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::Unknown, "Foundry fixture");
    app.setup.eligible = true;
    app.setup.probe = Some(setup_probe(true, true));
    app.external_status.insert(tomlook::connectors::Kind::Webiq, ExternalStatus {
        busy: true, message: "WebIQ fixture".into(), ..Default::default()
    });
    app.external_status.insert(tomlook::connectors::Kind::Github, ExternalStatus {
        ready: true, message: "GitHub fixture".into(), ..Default::default()
    });
    app.update_status.staged = Some(tomlook::updater::Staged {
        version: "9.9.9".into(),
        notes_url: tomlook::updater::release_page("v9.9.9"),
        directory: "C:\\fixture\\staging".into(),
        target: "C:\\fixture\\Tomlook.exe".into(),
        executable_sha256: "0".repeat(64),
    });
    let mut activity = (*app.activity).clone();
    for (id, state) in [
        ("ready-a", tomlook::scheduler::State::Completed),
        ("ready-b", tomlook::scheduler::State::Completed),
        ("problem", tomlook::scheduler::State::Failed),
    ] {
        activity.queue.jobs.insert(id.into(), activity_fixture_job(id, state));
    }
    activity.index = Arc::new(tomlook::worker::ActivityIndex::new(&activity.queue));
    for id in 0..3 {
        activity.queue.flights.insert(id, tomlook::scheduler::Flight {
            event_id: format!("working-{id}"), fingerprint: "0".repeat(64),
            foreground: false, translation_only: false,
        });
    }
    activity.queued = 24_999;
    activity.deferred = 1;
    activity.sync = tomlook::calendar_sync::Progress {
        fresh: 157, total: 213, through: Some(app.anchor), updated: Some(chrono::Utc::now()),
        error: Some("Sync fixture diagnostic".into()), ..Default::default()
    };
    app.activity = Arc::new(activity);
    (app, commands)
}

fn draw_status_bar(
    context: &egui::Context, app: &mut Tomlook, width: f32, events: Vec<egui::Event>,
) -> (egui::FullOutput, egui::Rect) {
    let mut rect = egui::Rect::NOTHING;
    let mut output = context.run_ui(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 640.0))),
        events, ..Default::default()
    }, |ui| {
        rect = egui::Panel::bottom("status").show(ui, |ui| app.status_bar(ui)).response.rect;
    });
    output.textures_delta.clear();
    (output, rect)
}

fn status_control(output: &egui::FullOutput, label: &str) -> egui::Rect {
    let tree = output.platform_output.accesskit_update.as_ref().unwrap();
    let bounds = tree.nodes.iter().find_map(|(_, node)| {
        (node.role() == egui::accesskit::Role::Button
            && node.label().is_some_and(|text| text.starts_with(label)))
            .then(|| node.bounds()).flatten()
    }).unwrap_or_else(|| panic!("Missing accessible status control: {label}"));
    egui::Rect::from_min_max(egui::pos2(bounds.x0 as f32, bounds.y0 as f32), egui::pos2(bounds.x1 as f32, bounds.y1 as f32))
}

fn click_status_control(context: &egui::Context, app: &mut Tomlook, label: &str) {
    for _ in 0..2 { draw_status_bar(context, app, 960.0, vec![]); }
    let (output, _) = draw_status_bar(context, app, 960.0, vec![]);
    let pos = status_control(&output, label).center();
    for pressed in [true, false] {
        draw_status_bar(context, app, 960.0, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        }]);
    }
}

#[test]
fn status_bar_is_one_unclipped_nonoverlapping_line_at_supported_widths_and_themes() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for dark in [false, true] {
            for width in [960.0, 1024.0, 1119.0, 1120.0, 1366.0, 1920.0] {
                let (mut app, mut commands) = status_bar_fixture();
                app.preferences.language = language;
                app.preferences.dark = dark;
                let context = egui::Context::default();
                context.enable_accesskit();
                app.style(&context);
                for _ in 0..3 { draw_status_bar(&context, &mut app, width, vec![]); }
                let (output, panel) = draw_status_bar(&context, &mut app, width, vec![]);
                assert!(panel.height() <= 38.0, "{language:?}, dark={dark}, width={width}: {panel:?}");
                let tree = output.platform_output.accesskit_update.as_ref().unwrap();
                let mut controls = tree.nodes.iter().filter_map(|(_, node)| {
                    if node.role() != egui::accesskit::Role::Button { return None; }
                    let label = node.label().unwrap();
                    Some((label, status_control(&output, label)))
                }).collect::<Vec<_>>();
                assert_eq!(controls.len(), 15, "All six services, cache, sync, four activity counts, bell, update and setup must fit");
                controls.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
                for (label, rect) in &controls {
                    assert!(panel.expand(0.5).contains_rect(*rect), "{label}: {rect:?} outside {panel:?}");
                    assert!(rect.width() >= 18.0 && rect.height() >= 24.0, "Control cannot shrink away: {label}: {rect:?}");
                }
                for pair in controls.windows(2) {
                    assert!(pair[0].1.right() <= pair[1].1.left() + 0.5, "Overlapping controls at width {width}: {pair:?}");
                }
                for clipped in &output.shapes {
                    if let egui::Shape::Text(text) = &clipped.shape {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        assert!(clipped.clip_rect.expand(0.5).contains_rect(rect), "Clipped text at width {width}: {}: {rect:?}, clip={:?}, panel={panel:?}", text.galley.text(), clipped.clip_rect);
                        assert!(panel.expand(0.5).contains_rect(rect), "Text outside footer: {}", text.galley.text());
                    }
                }
                let text = output_text(&output);
                for label in ["2\n", "3\n", "25k\n", "1\n", "157/213\n", concat!("v", env!("CARGO_PKG_VERSION"))] {
                    assert!(text.contains(label), "Missing {label:?}: {text}");
                }
                if width == 960.0 { assert!(text.contains("WIQ\n")); }
                if width >= 1366.0 { assert!(text.contains("WorkIQ\n")); }
                for label in ["GitHub Copilot: disconnected", "Foundry: not checked", "WorkIQ: connected",
                    "WebIQ: connecting...", "GitHub: connected", "Sync 157/213 · retrying"] {
                    status_control(&output, language.text(label).as_ref());
                }
                status_control(&output, &format!("{}: 25000", language.text("Waiting")));
                assert!(commands.try_recv().is_err(), "Rendering must not issue AI/network commands");
            }
        }
    }
}

#[test]
fn status_bar_click_targets_and_zero_count_visibility_are_preserved() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, mut commands) = status_bar_fixture();
        app.preferences.language = language;
        let context = egui::Context::default();
        context.enable_accesskit();
        app.style(&context);
        for (label, target) in [
            ("GitHub Copilot: disconnected", Some(ConnectionPanel::Ai)),
            ("Foundry: not checked", Some(ConnectionPanel::Ai)),
            ("WorkIQ: connected", Some(ConnectionPanel::Workiq)),
            (app.msx_status.label(), Some(ConnectionPanel::Msx)),
            ("WebIQ: connecting...", Some(ConnectionPanel::External(tomlook::connectors::Kind::Webiq))),
            ("GitHub: connected", Some(ConnectionPanel::External(tomlook::connectors::Kind::Github))),
            ("Sync 157/213 · retrying", Some(ConnectionPanel::Workiq)),
        ] {
            app.connection_panel = None;
            // Opening the AI dialog checks Foundry; restore the fixture's passive status.
            app.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::Unknown, "Foundry fixture");
            click_status_control(&context, &mut app, language.text(label).as_ref());
            assert!(app.connection_panel == target, "{label}");
        }
        let cache = language.text(&format!("{} cached meetings", app.calendar.events.len())).into_owned();
        app.connection_panel = None;
        click_status_control(&context, &mut app, &cache);
        assert!(app.connection_panel == Some(ConnectionPanel::Workiq));
        for label in ["Ready", "Researching", "Waiting", "Problems"] {
            app.settings = false;
            click_status_control(&context, &mut app, &format!("{}:", language.text(label)));
            assert!(app.settings, "{label} must open Insights activity");
        }
        app.auto_status = tomlook::decision::Status::new(tomlook::decision::Phase::Unknown, "Foundry fixture");
        let badge = tomlook::setup::summarize(&app.setup_items()).badge();
        click_status_control(&context, &mut app, language.text(&badge).as_ref());
        assert!(app.preferences_open && app.preferences_section == PreferencesSection::Setup);
        app.preferences_open = false;
        click_status_control(&context, &mut app, language.text("Update 9.9.9 ready").as_ref());
        assert!(app.preferences_open && app.preferences_section == PreferencesSection::Updates);
        click_status_control(&context, &mut app, language.text("Messages").as_ref());
        assert!(app.messages_open);
        let mut seen = Vec::new();
        drain_ai(&mut commands, &mut seen);
        assert_eq!(seen.iter().filter(|command| matches!(command, ai::Command::CheckAuto)).count(), 2);
        assert_eq!(seen.iter().filter(|command| matches!(command, ai::Command::RefreshCalendarAvailability)).count(), 1);
        assert_eq!(seen.len(), 3, "Only existing explicit-click actions may issue commands");
        app.activity = Arc::new(Default::default());
        app.preferences.msx_enabled = false;
        app.setup.eligible = false;
        app.update_status.staged = None;
        let (output, _) = draw_status_bar(&context, &mut app, 960.0, vec![egui::Event::PointerGone]);
        let tree = output.platform_output.accesskit_update.as_ref().unwrap();
        let labels = tree.nodes.iter().filter_map(|(_, node)| node.label()).collect::<Vec<_>>();
        assert!(labels.iter().any(|label| label.starts_with(&format!("{}: 0", language.text("Ready")))));
        for label in ["Researching", "Waiting", "Problems"] {
            assert!(!labels.iter().any(|text| text.starts_with(&format!("{}:", language.text(label)))));
        }
        assert!(!labels.iter().any(|label| label.starts_with("MSX")));
        assert!(commands.try_recv().is_err());
    }
}

#[test]
fn status_items_keep_full_hover_accessibility_and_stable_keyboard_activation() {
    use super::connections::{ConnectionState, StatusIcon, status_item};
    let context = egui::Context::default();
    context.enable_accesskit();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        context.style_mut_of(theme, |style| style.interaction.tooltip_delay = 0.0);
    }
    let mut identity = None;
    let mut rect = egui::Rect::NOTHING;
    let mut clicked = false;
    let mut draw = |language: tomlook::i18n::Language, events| {
        let hint = format!("{}: 25000", language.text("Waiting"));
        let mut output = context.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            let response = status_item(ui, "status-counter-fixture", StatusIcon::Clock, "25k", ConnectionState::Unknown, &hint);
            if let Some(id) = identity { assert_eq!(response.id, id); } else { identity = Some(response.id); }
            rect = response.rect;
            clicked |= response.clicked();
        });
        output.textures_delta.clear();
        assert_eq!(status_control(&output, &hint), rect);
        (output, identity.unwrap(), rect)
    };
    let (_, id, rect) = draw(tomlook::i18n::Language::En, vec![]);
    for _ in 0..3 {
        draw(tomlook::i18n::Language::En, vec![egui::Event::PointerMoved(rect.center())]);
    }
    let (output, _, _) = draw(tomlook::i18n::Language::En, vec![]);
    assert!(output_text(&output).contains("Waiting: 25000"), "The exact, unabbreviated count remains available on hover");
    context.memory_mut(|memory| memory.request_focus(id));
    draw(tomlook::i18n::Language::Cs, vec![egui::Event::PointerGone, egui::Event::Key {
        key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE,
    }]);
    assert!(clicked);
}

#[test]
fn status_icon_and_text_have_real_spacing_and_only_problems_are_colored() {
    use super::connections::{ConnectionState, StatusIcon, status_item};
    for dark in [false, true] {
        for layout in [egui::Layout::left_to_right(egui::Align::Center), egui::Layout::right_to_left(egui::Align::Center)] {
            let (mut app, _, _) = assistant_fixture();
            app.preferences.dark = dark;
            let context = egui::Context::default();
            app.style(&context);
            let mut neutral = None;
            for (state, icon) in [
                (ConnectionState::Ready, StatusIcon::Check),
                (ConnectionState::Busy, StatusIcon::Busy),
                (ConnectionState::Unknown, StatusIcon::Calendar),
                (ConnectionState::Offline, StatusIcon::Alert),
            ] {
                let mut response_rect = egui::Rect::NOTHING;
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    neutral = Some(ui.visuals().weak_text_color());
                    ui.with_layout(layout, |ui| {
                        response_rect = status_item(ui, "spacing-fixture", icon, "WorkIQ", state, "WorkIQ fixture").rect;
                    });
                });
                let text = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "WorkIQ" => Some(text),
                    _ => None,
                }).unwrap();
                let text_rect = text.galley.rect.translate(text.pos.to_vec2());
                assert!(text_rect.left() - (response_rect.left() + 12.0) >= 5.9, "The icon needs a six-pixel gap, not spaces inside the label");
                assert!(response_rect.expand(0.5).contains_rect(text_rect));
                let color = text.fallback_color;
                if state == ConnectionState::Offline {
                    assert_ne!(color, neutral.unwrap());
                    assert_eq!(color, if dark { Color32::from_rgb(194, 137, 132) } else { Color32::from_rgb(153, 78, 73) });
                } else {
                    assert_eq!(color, neutral.unwrap(), "Normal, unknown and busy states must not compete with the content");
                }
                output.textures_delta.clear();
            }
        }
    }
}

#[test]
fn github_dialog_only_shows_status_client_id_and_one_action() {
    let (mut app, _, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    app.connection_panel = Some(ConnectionPanel::External(tomlook::connectors::Kind::Github));
    app.github_client_id = "Iv23fixture".into();
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        for ready in [false, true] {
            app.external_status.insert(
                tomlook::connectors::Kind::Github,
                ExternalStatus {
                    ready,
                    message: "Fixture provider diagnostic belongs in Messages".into(),
                    ..Default::default()
                },
            );
            let mut text = String::new();
            for _ in 0..2 {
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    app.connection_dialog(ui.ctx())
                });
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
                output.textures_delta.clear();
            }
            assert!(
                text.contains(
                    language
                        .text(if ready { "Connected" } else { "Not connected" })
                        .as_ref()
                )
            );
            assert!(text.contains("Iv23fixture"));
            assert!(text.contains(language.text("Sign in").as_ref()));
            for removed in [
                "Browser sign-in",
                "Personal token",
                "GitHub App setup",
                "Disconnect and forget",
            ] {
                assert!(
                    !text.contains(language.text(removed).as_ref()),
                    "Unexpected control: {removed}"
                );
            }
            assert!(!text.contains("Fixture provider diagnostic"));
            assert!(commands.try_recv().is_err());
        }
    }
    app.external_status.insert(
        tomlook::connectors::Kind::Github,
        ExternalStatus {
            busy: true,
            message: "Open https://github.com/login/device and enter ABCD-EF12".into(),
            ..Default::default()
        },
    );
    let mut text = String::new();
    for _ in 0..2 {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.connection_dialog(ui.ctx())
        });
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        output.textures_delta.clear();
    }
    assert!(
        text.contains("ABCD-EF12"),
        "Device authorization instructions must remain visible"
    );
    assert!(text.contains(app.preferences.language.text("Cancel").as_ref()));
}

#[test]
fn messages_indicator_is_small_stable_and_keyboard_accessible() {
    let context = egui::Context::default();
    let mut identity = None;
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        let response = messages_button(ui, tomlook::i18n::Language::En);
        identity = Some(response.id);
        assert!(response.rect.width() <= 32.0);
    });
    output.textures_delta.clear();
    context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
    let mut clicked = false;
    let mut output = context.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            let response = messages_button(ui, tomlook::i18n::Language::Cs);
            assert_eq!(response.id, identity.unwrap());
            clicked = response.clicked();
        },
    );
    output.textures_delta.clear();
    assert!(clicked);
}

fn append_shape_text(shape: &egui::epaint::Shape, output: &mut String) {
    match shape {
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                append_shape_text(shape, output);
            }
        }
        egui::epaint::Shape::Text(shape) => {
            output.push_str(shape.galley.text());
            output.push('\n');
        }
        _ => {}
    }
}

fn activity_fixture_job(id: &str, state: tomlook::scheduler::State) -> tomlook::scheduler::Job {
    tomlook::scheduler::Job {
        event_id: id.into(),
        fingerprint: "0".repeat(64),
        start: Local::now().fixed_offset(),
        foreground: false,
        manual: false,
        review: None,
        on_demand: false,
        promotion: 0,
        verified: true,
        state,
        message: "Fixture original-source read failed; explicitly review before retry.".into(),
        retries: 0,
        retry_at: None,
        prepared_at: None,
    }
}

#[test]
fn activity_filters_find_problems_beyond_thousands_of_waiting_jobs_without_loss() {
    use tomlook::scheduler::State;
    let mut queue = tomlook::scheduler::Queue::default();
    for index in 0..5000 {
        let state = match index {
            4994 => State::Deferred,
            4995 => State::Running,
            4996 => State::Completed,
            4997 => State::Cancelled,
            4998 => State::Failed,
            4999 => State::Interrupted,
            _ => State::Queued,
        };
        let job = activity_fixture_job(&format!("fixture-{index:04}"), state);
        queue.jobs.insert(job.event_id.clone(), job);
    }
    let index = tomlook::worker::ActivityIndex::new(&queue);
    assert_eq!(index.len(ActivityFilter::All), 5000);
    assert_eq!(index.len(ActivityFilter::Waiting), 4995);
    assert_eq!(index.len(ActivityFilter::Working), 1);
    assert_eq!(index.len(ActivityFilter::Ready), 1);
    let problems = (0..index.len(ActivityFilter::Problems))
        .map(|row| index.job(&queue, ActivityFilter::Problems, row))
        .collect::<Vec<_>>();
    assert_eq!(problems.len(), 2);
    assert_eq!(problems[0].event_id, "fixture-4998");
    assert_eq!(problems[1].event_id, "fixture-4999");
    assert!(
        problems
            .iter()
            .all(|job| job.message.contains("explicitly review"))
    );
    assert_eq!(queue.jobs.len(), 5000);
    assert!(ActivityFilter::All.matches(State::Cancelled));
    assert!(!ActivityFilter::Problems.matches(State::Cancelled));
}

#[test]
fn activity_rows_keep_fixed_height_and_stable_identity_with_long_multiline_titles() {
    let context = egui::Context::default();
    let job = activity_fixture_job("fixture-row", tomlook::scheduler::State::Failed);
    let other = activity_fixture_job("fixture-other", tomlook::scheduler::State::Queued);
    let title = format!(
        "{} \n {}",
        "A very long meeting title ".repeat(100),
        "\u{010d}esk\u{00e9} ".repeat(100)
    );
    let mut identity = None;
    for (index, width) in [280.0, 360.0, 440.0, 720.0].into_iter().enumerate() {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            if index % 2 == 1 {
                activity_row(ui, &other, "Other row", true, tomlook::i18n::Language::En);
            }
            let row = ui.allocate_ui(Vec2::new(width, ACTIVITY_ROW_HEIGHT), |ui| {
                activity_row(ui, &job, &title, true, tomlook::i18n::Language::En)
            });
            assert!(
                row.response.rect.height() <= ACTIVITY_ROW_HEIGHT + 0.01,
                "{:?}",
                row.response.rect
            );
            assert!(
                row.response.rect.width() <= width + 0.01,
                "{:?}",
                row.response.rect
            );
            assert!(row.inner.rect.height() <= ACTIVITY_ROW_HEIGHT + 0.01);
            if let Some(id) = identity {
                assert_eq!(row.inner.id, id);
            }
            identity = Some(row.inner.id);
        });
        output.textures_delta.clear();
    }
}

#[test]
fn opening_a_problem_from_keyboard_reads_detail_without_retrying_or_submitting_ai() {
    let (mut app, _, mut ai_commands) = assistant_fixture();
    let (commands, storage_commands) = std::sync::mpsc::sync_channel(64);
    app.worker = Some(Worker {
        commands,
        notices: Arc::new(tomlook::worker::Notices::default()),
        activity: Arc::new(std::sync::Mutex::new(Arc::new(
            tomlook::worker::Activity::default(),
        ))),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        done: None,
    });
    app.calendar = Arc::new(
        Calendar::build(
            tomlook::calendar::fixture(1),
            tomlook::calendar::fixture_coverage(),
            Vec::new(),
        )
        .unwrap(),
    );
    let event = &app.calendar.events[0];
    let job = activity_fixture_job(&event.id, tomlook::scheduler::State::Failed);
    let title = event.title.clone();
    let context = egui::Context::default();
    let mut identity = None;
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        identity = Some(activity_row(ui, &job, &title, true, tomlook::i18n::Language::En).id);
    });
    output.textures_delta.clear();
    context.memory_mut(|memory| memory.request_focus(identity.unwrap()));
    let mut output = context.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            if activity_row(ui, &job, &title, true, tomlook::i18n::Language::En).clicked() {
                app.select(0);
            }
        },
    );
    output.textures_delta.clear();
    assert_eq!(app.selected, Some(0));
    let Command::Detail { id, .. } = storage_commands.try_recv().unwrap() else {
        panic!("Only saved detail may be requested");
    };
    assert_eq!(id, job.event_id);
    assert!(storage_commands.try_recv().is_err());
    assert!(ai_commands.try_recv().is_err());
}

#[test]
fn problems_filter_renders_all_failed_targets_and_an_honest_empty_working_view() {
    use tomlook::scheduler::State;
    let (mut app, _, mut commands) = assistant_fixture();
    let mut events = tomlook::calendar::fixture(2);
    events[0].title = "Failed target fixture".into();
    events[1].title = "Interrupted target fixture".into();
    let ids = [events[0].id.clone(), events[1].id.clone()];
    app.calendar = Arc::new(
        Calendar::build(events, tomlook::calendar::fixture_coverage(), Vec::new()).unwrap(),
    );
    let mut queue = tomlook::scheduler::Queue::default();
    for index in 0..4998 {
        let job = activity_fixture_job(&format!("queued-{index}"), State::Queued);
        queue.jobs.insert(job.event_id.clone(), job);
    }
    for (id, state) in ids.iter().zip([State::Failed, State::Interrupted]) {
        queue
            .jobs
            .insert(id.clone(), activity_fixture_job(id, state));
    }
    app.activity = Arc::new(tomlook::worker::Activity {
        index: Arc::new(tomlook::worker::ActivityIndex::new(&queue)),
        queue,
        ..Default::default()
    });
    app.settings = true;
    app.activity_filter = ActivityFilter::Problems;
    let context = egui::Context::default();
    for (filter, expected) in [
        (ActivityFilter::Problems, "Failed target fixture"),
        (ActivityFilter::Working, "No matching jobs."),
    ] {
        app.activity_filter = filter;
        let mut sizing = context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
        sizing.textures_delta.clear();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.overlays(ui.ctx()));
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        output.textures_delta.clear();
        assert!(text.contains(expected), "Missing {expected}: {text}");
        if filter == ActivityFilter::Problems {
            assert!(text.contains("Interrupted target fixture"));
            assert!(!text.contains("queued-"));
        }
    }
    assert_eq!(app.activity.queue.jobs.len(), 5000);
    assert!(commands.try_recv().is_err());
}

#[test]
fn insights_show_unmatched_citations_and_distinguish_search_from_original_reads() {
    let (mut app, _, _) = assistant_fixture();
    app.briefing = Some(serde_json::json!({
        "summary": "Fixture summary",
        "evidence_status": "1/3 cited links in captured results. Claims remain unverified.",
        "sources": [
            {"title":"Captured source", "url":"https://example.com/actual", "retrieved_records":[1]},
            {"title":"Uncaptured source", "url":"https://example.com/uncaptured", "retrieved_records":[]},
            {"title":"Legacy source", "url":"https://example.com/legacy"}
        ],
        "retrieved_evidence": {
            "omitted": 2,
            "records": [
                {"ordinal":1, "tool":tomlook::workiq::connection::CONTEXT_TOOL, "succeeded":true},
                {"ordinal":2, "tool":tomlook::workiq::connection::SDK_TOOL, "resource":"/me/messages/actual", "succeeded":true}
            ]
        }
    }));
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.render_insights(ui, true)
    });
    let mut rendered = String::new();
    for shape in &output.shapes {
        append_shape_text(&shape.shape, &mut rendered);
    }
    output.textures_delta.clear();
    for expected in [
        "Fixture summary",
        "Claims remain unverified.",
        "Referenced in captured results",
        "Not found in captured results",
        "Unverified citation",
        "R1: Workplace search summary",
        "R2: Message read",
        "2 additional tool results not retained.",
    ] {
        assert!(
            rendered.contains(expected),
            "Missing rendered text: {expected}; {rendered}"
        );
    }
    assert!(!rendered.contains("Open original source"));
    let mut compact = context.run_ui(egui::RawInput::default(), |ui| {
        app.render_insights(ui, false)
    });
    let mut rendered = String::new();
    for shape in &compact.shapes {
        append_shape_text(&shape.shape, &mut rendered);
    }
    compact.textures_delta.clear();
    assert!(rendered.contains("Fixture summary"));
    assert!(rendered.contains("Claims remain unverified."));
    assert!(!rendered.contains("Uncaptured source"));
}

#[test]
fn escape_closes_expanded_insights_without_losing_selected_meeting() {
    let (mut app, _, _) = assistant_fixture();
    app.selected = Some(0);
    app.insights_expanded = true;
    let context = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut output = context.run_ui(input, |ui| app.shortcuts(ui.ctx()));
    output.textures_delta.clear();
    assert!(!app.insights_expanded);
    assert_eq!(app.selected, Some(0));
}

#[test]
fn exact_palette_command_precedes_partial_word_matches() {
    let matches = command_matches(" Show WEEK ");
    assert!(matches!(matches[0], PaletteCommand::View(View::Week)));
    assert!(matches!(matches[1], PaletteCommand::View(View::WorkWeek)));
    assert!(matches!(
        command_matches("week")[0],
        PaletteCommand::View(View::WorkWeek)
    ));
}

#[test]
fn all_theme_and_accent_choices_meet_text_and_focus_contrast_limits() {
    let luminance = |color: Color32| {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    };
    let contrast = |foreground: Color32, background: Color32| {
        let foreground = luminance(foreground);
        let background = luminance(background);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    };
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    for dark in [false, true] {
        for accent in 0..ACCENTS.len() {
            app.preferences.dark = dark;
            app.preferences.accent = accent;
            app.style(&context);
            let style = context.style_of(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let visuals = &style.visuals;
            for background in [visuals.panel_fill, visuals.window_fill] {
                for category in [
                    "call",
                    "mimo office",
                    "mimo Prahu",
                    "nejisté",
                    "office",
                    "osobní",
                ] {
                    assert!(
                        contrast(category_color(category, dark), background) >= 4.5,
                        "Category contrast: {category}, dark={dark}"
                    );
                    assert_eq!(
                        category_color(category, dark),
                        category_color(&category.to_uppercase(), dark)
                    );
                }
                assert!(
                    contrast(visuals.text_color(), background) >= 4.5,
                    "Body contrast: dark={dark}, accent={accent}"
                );
                assert!(
                    contrast(visuals.weak_text_color(), background) >= 4.5,
                    "Secondary text contrast: dark={dark}, accent={accent}"
                );
                assert!(
                    contrast(visuals.hyperlink_color, background) >= 4.5,
                    "Link contrast: dark={dark}, accent={accent}"
                );
                assert!(
                    contrast(visuals.selection.stroke.color, background) >= 3.0,
                    "Focus contrast: dark={dark}, accent={accent}"
                );
            }
        }
    }
}

#[test]
fn focus_window_icon_tracks_theme_and_accent_without_rebuilding_on_unchanged_frames() {
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    for dark in [false, true] {
        for accent in 0..ACCENTS.len() {
            app.preferences.dark = dark;
            app.preferences.accent = accent;
            let color = app.accent();
            let mut output = context.run_ui(Default::default(), |ui| {
                app.sync_brand(ui.ctx());
                app.header(ui);
            });
            output.textures_delta.clear();
            let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
            let icons: Vec<_> = commands
                .iter()
                .filter_map(|command| {
                    if let egui::ViewportCommand::Icon(Some(icon)) = command {
                        Some(icon)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(icons.len(), 1);
            assert_eq!((icons[0].width, icons[0].height), (64, 64));
            assert_eq!(
                icons[0].rgba,
                tomlook::brand::rgba_with_color(64, [color.r(), color.g(), color.b()])
            );
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(text.contains("Tomlook"));
            assert!(!text.contains("O^O"));
            let mut output = context.run_ui(Default::default(), |ui| app.sync_brand(ui.ctx()));
            output.textures_delta.clear();
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .all(|command| !matches!(command, egui::ViewportCommand::Icon(_)))
            );
        }
    }
}

#[test]
fn dense_lanes_remain_readable_and_keyboard_selection_reveals_hidden_cards() {
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    let mut events = tomlook::calendar::fixture(5000);
    for event in &mut events {
        event.title = format!("{} {}", event.id, event.title);
    }
    app.calendar = Arc::new(
        Calendar::build(events, tomlook::calendar::fixture_coverage(), Vec::new()).unwrap(),
    );
    app.anchor = tomlook::calendar::fixture_anchor();
    app.preferences.view = View::Day;
    let slots = &app.calendar.days[&app.anchor].timed;
    let width = timeline_content_width(240.0, slots);
    assert!(width > 240.0);
    assert!(
        slots
            .iter()
            .all(|slot| width / slot.lanes as f32 >= MIN_EVENT_LANE_WIDTH)
    );
    let selected = slots.iter().max_by_key(|slot| slot.lane).unwrap().event;
    let title = app.calendar.events[selected].title.clone();
    let draw = |app: &mut Tomlook, frame| {
        context.run_ui(
            egui::RawInput {
                time: Some(frame as f64 * 0.1),
                ..Default::default()
            },
            |ui| {
                ui.set_max_size(Vec2::new(288.0, 640.0));
                ui.set_clip_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    Vec2::new(288.0, 640.0),
                ));
                app.timeline(ui);
            },
        )
    };
    let visible = |output: &egui::FullOutput| {
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Text(text)
            if text.galley.job.text == title && shape.clip_rect.contains(text.pos))
        })
    };
    let mut output = draw(&mut app, 0);
    output.textures_delta.clear();
    assert!(
        !visible(&output),
        "high lane should initially require horizontal scrolling"
    );
    app.selected = Some(selected);
    app.focus_meeting = true;
    app.scroll_meeting = true;
    for frame in 1..10 {
        output = draw(&mut app, frame);
        output.textures_delta.clear();
    }
    assert!(
        visible(&output),
        "keyboard selection must reveal the high lane"
    );
    assert!(!app.focus_meeting);
    assert!(!app.scroll_meeting);
    assert!(context.memory(|memory| memory.focused()).is_some());
    app.query = title.clone();
    for frame in 10..20 {
        output = draw(&mut app, frame);
        output.textures_delta.clear();
    }
    assert!(
        visible(&output),
        "filtered lane must repack into the viewport"
    );
    assert_eq!(app.calendar.events.len(), 5000);
}

#[test]
fn offscreen_meeting_cards_skip_widgets_but_keep_keyboard_reveal() {
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    app.calendar = Arc::new(
        Calendar::build(
            tomlook::calendar::fixture(200),
            tomlook::calendar::fixture_coverage(),
            Vec::new(),
        )
        .unwrap(),
    );
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 100_000.0), Vec2::new(180.0, 70.0));
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.event_card(ui, 0, rect);
    });
    output.textures_delta.clear();
    assert!(output.shapes.is_empty());
    app.selected = Some(0);
    app.focus_meeting = true;
    app.scroll_meeting = true;
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.event_card(ui, 0, rect);
    });
    output.textures_delta.clear();
    assert!(!app.focus_meeting);
    assert!(!app.scroll_meeting);
    assert!(context.memory(|memory| memory.focused()).is_some());
}

#[test]
fn custom_meeting_cards_do_not_layout_a_second_invisible_title() {
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    app.calendar = Arc::new(
        Calendar::build(
            tomlook::calendar::fixture(200),
            tomlook::calendar::fixture_coverage(),
            Vec::new(),
        )
        .unwrap(),
    );
    let title = app.calendar.events[0].title.clone();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(180.0, 70.0));
        app.event_card(ui, 0, rect);
    });
    output.textures_delta.clear();
    let texts: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.iter().filter(|text| **text == title).count(), 1);
    assert!(
        !texts
            .iter()
            .any(|text| **text != title && text.contains(&title))
    );
}

#[test]
fn ordinary_calendar_cards_show_only_their_time_range_not_a_conflict_caption() {
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    let mut events = tomlook::calendar::fixture(1);
    events[0].is_all_day = false;
    events[0].end = events[0].start + chrono::Duration::hours(1);
    events[0].status = "accepted".into();
    let expected = format!(
        "{} - {}",
        events[0].start.with_timezone(&Local).format("%H:%M"),
        events[0].end.with_timezone(&Local).format("%H:%M")
    );
    app.calendar = Arc::new(Calendar::build(events, Default::default(), vec![]).unwrap());
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        app.preferences.language = language;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(320.0, 140.0));
            app.event_card(ui, 0, rect);
        });
        output.textures_delta.clear();
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        assert!(text.contains(&expected), "{text}");
        assert!(!text.contains(language.text("Overlap").as_ref()), "{text}");
    }
}

#[test]
fn closing_palette_releases_its_editor_focus() {
    let context = egui::Context::default();
    let mut app = Tomlook::initial(None, None, None);
    app.palette = true;
    context.memory_mut(|memory| memory.request_focus(Id::new("command-search")));
    app.close_palette(&context);
    assert!(!app.palette);
    assert_ne!(
        context.memory(|memory| memory.focused()),
        Some(Id::new("command-search"))
    );
}

#[test]
fn calendar_keyboard_commands_still_work_when_a_non_editor_control_has_focus() {
    let (mut app, _, _) = assistant_fixture();
    let context = egui::Context::default();
    context.memory_mut(|memory| memory.request_focus(Id::new("fixture-button")));
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: Key::P,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL | Modifiers::SHIFT,
        }],
        ..Default::default()
    };
    let mut output = context.run_ui(input, |ui| app.shortcuts(ui.ctx()));
    output.textures_delta.clear();
    assert!(app.palette);
}

fn fixture_proposal(title: &str) -> Proposal {
    Proposal {
        kind: Kind::Email,
        target: "Unverified fixture message".into(),
        title: title.into(),
        body: "Fixture text, never sent".into(),
    }
}

#[test]
fn invalid_typed_proposals_do_not_reach_the_native_panel() {
    let (mut app, output, _) = assistant_fixture();
    let mut answer = fixture_answer("Fixture");
    answer.proposals = Arc::new(vec![Proposal::default()]);
    app.active_request = Some(9);
    output
        .send(ai::Notice::Answer {
            id: 9,
            result: Ok(answer),
        })
        .unwrap();
    app.receive();
    assert!(app.answer.is_none());
    assert!(app.ai_error.as_ref().unwrap().contains("Title"));
}

fn fixture_answer(text: impl Into<String>) -> ai::Answer {
    ai::Answer::new(
        text.into(),
        Arc::new(tomlook::evidence::Snapshot::default()),
        Arc::new(Vec::new()),
    )
}

fn fixture_source_answer(scope: &str) -> ai::Answer {
    let recorder =
        tomlook::evidence::Recorder::new(Arc::new(tomlook::evidence::Redactor::default()));
    recorder.record(
        "WorkIQ-Mail-GetMessage",
        &serde_json::json!({
            "url":"https://example.com/fixture", "scope":scope
        }),
        true,
    );
    ai::Answer::new(
        scope.into(),
        recorder.snapshot().unwrap(),
        Arc::new(vec![fixture_proposal(scope)]),
    )
}

#[test]
fn captured_evidence_stays_atomic_with_its_answer_across_scope_changes_and_late_results() {
    let (mut app, output, mut input) = assistant_fixture();
    let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
    app.answer = Some(fixture_source_answer("global-fixture"));
    assert!(app.switch_assistant_context(Some(meeting.clone())));
    assert!(app.answer.is_none());
    app.answer = Some(fixture_source_answer("meeting-fixture"));
    assert!(app.switch_assistant_context(None));
    assert!(
        app.answer.as_ref().unwrap().evidence.records[0]
            .excerpt
            .contains("global-fixture")
    );
    app.question = "New global question".into();
    app.ask_question();
    let ai::Command::Chat(request) = input.try_recv().unwrap() else {
        panic!("Question not admitted")
    };
    let question = request.question;
    assert!(!app.switch_assistant_context(Some(meeting.clone())));
    app.cancel_ai();
    assert!(matches!(input.try_recv(), Ok(ai::Command::Cancel(id)) if id == question.id));
    assert!(app.switch_assistant_context(Some(meeting.clone())));
    output
        .send(ai::Notice::Answer {
            id: question.id,
            result: Ok(fixture_source_answer("late-global-fixture")),
        })
        .unwrap();
    app.receive();
    let answer = app.answer.as_ref().unwrap();
    assert_eq!(answer.text.as_str(), "meeting-fixture");
    assert_eq!(answer.proposals[0].title, "meeting-fixture");
    assert!(
        answer.evidence.records[0]
            .excerpt
            .contains("meeting-fixture")
    );
    assert!(
        !answer.evidence.records[0]
            .excerpt
            .contains("late-global-fixture")
    );
    meeting.title = "Changed fixture snapshot".into();
    assert!(app.switch_assistant_context(Some(meeting)));
    assert!(app.answer.is_none());
}

#[test]
fn unsafe_captured_source_payload_cannot_reach_the_answer_inspector() {
    let (mut app, output, _) = assistant_fixture();
    let mut answer = fixture_source_answer("unsafe-fixture");
    Arc::make_mut(&mut answer.evidence).records[0]
        .links
        .push("javascript:alert(1)".into());
    app.active_request = Some(9);
    output
        .send(ai::Notice::Answer {
            id: 9,
            result: Ok(answer),
        })
        .unwrap();
    app.receive();
    assert!(app.answer.is_none());
    assert!(app.ai_error.as_ref().unwrap().contains("unsafe"));
}

#[test]
fn workiq_diagnostics_do_not_change_copilot_connection_state() {
    let (mut app, output, _) = assistant_fixture();
    let connection = app.connection.clone();
    output
        .send(ai::Notice::Workiq {
            message: "Sign-in blocked: issuer mismatch".into(),
        })
        .unwrap();
    app.receive();
    assert!(app.workiq_status.contains("issuer mismatch"));
    assert!(app.ai_ready);
    assert_eq!(app.connection, connection);
}

#[test]
fn hosted_workiq_progress_is_independent_of_assistant_and_calendar() {
    let (mut app, output, _) = assistant_fixture();
    let connection = app.connection.clone();
    let calendar = app.calendar.clone();
    output
        .send(ai::Notice::HostedWorkiq {
            busy: true,
            ready: false,
            message: "Waiting for browser sign-in".into(),
        })
        .unwrap();
    app.receive();
    assert!(app.hosted_workiq_busy);
    assert_eq!(app.hosted_workiq_status, "Waiting for browser sign-in");
    output
        .send(ai::Notice::HostedWorkiq {
            busy: false,
            ready: true,
            message: "WorkIQ read verified; tokens discarded".into(),
        })
        .unwrap();
    app.receive();
    assert!(!app.hosted_workiq_busy);
    assert!(app.ai_ready);
    assert_eq!(app.connection, connection);
    assert!(Arc::ptr_eq(&calendar, &app.calendar));
}

fn assistant_fixture() -> (
    Tomlook,
    Arc<ai::Notices>,
    tokio::sync::mpsc::Receiver<ai::Command>,
) {
    let notices = Arc::new(ai::Notices::default());
    let (commands, input) = tokio::sync::mpsc::channel(64);
    let (stop, _stopped) = tokio::sync::watch::channel(false);
    let engine = Engine {
        commands,
        notices: notices.clone(),
        progress: Arc::new(std::sync::Mutex::new(ai::Progress::default())),
        done: None,
        ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        occupied: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        terminated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        analysis_revision: Arc::new(std::sync::OnceLock::new()),
        workiq_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stop,
    };
    let mut app = Tomlook::initial(None, Some(engine), None);
    app.ai_ready = true;
    (app, notices, input)
}

#[test]
fn auto_authentication_has_separate_status_and_explicit_device_login_controls() {
    use tomlook::decision::{Phase, Status};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, notices, mut commands) = assistant_fixture();
        app.preferences.language = language;
        app.connection = "Connected".into();
        let error = "Microsoft-Decision-1 needs a valid Azure CLI sign-in for the configured tenant and subscription; choose a mode explicitly";
        app.ai_error = Some(error.into());
        Arc::make_mut(&mut app.chat).admit(7, "Synthetic failed Auto question".into(), tomlook::chat::Mode::Auto).unwrap();
        Arc::make_mut(&mut app.chat).finish(7, Err(error.into()));
        notices.send(ai::Notice::Auto(Status::error(error.into()))).unwrap();
        app.receive();
        assert!(app.ai_ready, "Auto auth failure must not disconnect Copilot");
        assert_eq!(app.connection, "Connected");
        let context = egui::Context::default();
        app.apply_ask_input(app.ask_view().input(ask::Action::ConfigureAuto), &context);
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::CheckAuto));
        assert!(commands.try_recv().is_err(), "Opening the dialog must not start login or repeat a question");
        notices.send(ai::Notice::Auto(Status::error(error.into()))).unwrap();
        app.receive();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 1000.0))),
                events, ..Default::default()
            }, |ui| app.connection_dialog(ui.ctx()));
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let position = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing Auto connection control: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(position), egui::Event::PointerButton {
                    pos: position, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                }]);
            }
        };
        draw(&mut app, vec![]);
        let output = draw(&mut app, vec![]);
        let mut text = String::new();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains("GitHub Copilot") && text.contains("Foundry · Microsoft-Decision-1"));
        assert_eq!(text.matches(language.text(error).as_ref()).count(), 1, "Do not repeat the routing error as a Copilot failure: {text}");
        click(&mut app, language.text("Sign in to Azure (device code)").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::SignInAuto));
        assert!(commands.try_recv().is_err());
        let output = draw(&mut app, vec![]);
        text.clear();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains(language.text("Requesting an Azure device code (up to 60 seconds)...").as_ref()));
        assert!(text.contains(language.text("Open Microsoft device sign-in").as_ref()));
        assert!(text.contains(language.text("Cancel Azure sign-in").as_ref()));
        assert!(output.platform_output.commands.is_empty());
        let timeout = "Azure CLI did not provide a device code within 60 seconds. Check connectivity and try signing in again.";
        notices.send(ai::Notice::Auto(Status::error(timeout.into()))).unwrap();
        app.receive();
        let output = draw(&mut app, vec![]);
        text.clear();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains(language.text(timeout).as_ref()));
        assert!(app.ai_ready && !app.auto_status.busy());
        click(&mut app, language.text("Sign in to Azure (device code)").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::SignInAuto));
        notices.send(ai::Notice::Auto(Status {
            phase: Phase::SigningIn,
            message: "Enter this code on Microsoft's device sign-in page.".into(),
            device_code: Some("ABCD-EF12".into()),
        })).unwrap();
        app.receive();
        let output = draw(&mut app, vec![]);
        text.clear();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains("ABCD-EF12"));
        assert!(text.contains(language.text("Open Microsoft device sign-in").as_ref()));
        assert!(output.platform_output.commands.is_empty(), "Rendering a code must not open a browser or copy it automatically");
        click(&mut app, language.text("Cancel Azure sign-in").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::CancelAutoLogin));
        notices.send(ai::Notice::Auto(Status::signed_in())).unwrap();
        app.receive();
        assert!(app.ai_ready && app.auto_status.phase == Phase::SignedIn);
        assert_eq!(app.chat.turns[0].error.as_deref(), Some(error));
        assert!(commands.try_recv().is_err(), "Successful login must not repeat a failed question");
    }
}

fn msx_status(phase: tomlook::msx::Phase, message: &str, code: Option<&str>) -> tomlook::msx::Status {
    let service = tomlook::msx::Service { phase, message: message.into() };
    tomlook::msx::Status {
        phase,
        message: message.into(),
        tenant: tomlook::msx::DEFAULT_TENANT.into(),
        account: (phase == tomlook::msx::Phase::Connected).then(|| "someone@example.test".into()),
        dataverse: service.clone(),
        powerbi: service,
        device_code: code.map(Into::into),
        tools: Vec::new(),
    }
}

#[test]
fn msx_connection_offers_only_explicit_sign_in_and_is_localized() {
    use tomlook::msx::{Phase, auth::TOKEN_TIMEOUT};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, notices, mut commands) = assistant_fixture();
        app.preferences.language = language;
        app.preferences.msx_enabled = true;
        app.connection_panel = Some(ConnectionPanel::Msx);
        notices.send(ai::Notice::Msx(msx_status(Phase::SignInRequired, TOKEN_TIMEOUT, None))).unwrap();
        app.receive();
        assert_eq!(app.msx_status.label(), "MSX: sign-in required");
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 1000.0))),
                events, ..Default::default()
            }, |ui| app.connection_dialog(ui.ctx()));
            output.textures_delta.clear();
            output
        };
        let text = |app: &mut Tomlook| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let mut text = String::new();
            for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
            (text, output.platform_output.commands.is_empty())
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let position = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing MSX connection control: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(position), egui::Event::PointerButton {
                    pos: position, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                }]);
            }
        };
        let (shown, quiet) = text(&mut app);
        for expected in [
            "MSX connection",
            "MSX · Dataverse and Power BI",
            TOKEN_TIMEOUT,
            "Sign in to MSX",
            "Use device code",
            "Check connection",
            "Disconnect",
            "Signed-in Azure account",
            "Not known yet",
            "Uses your Azure CLI sign-in; may change Azure CLI's default account.",
            "Read-only: Tomlook never creates or changes MSX records. Tokens stay in memory.",
        ] {
            assert!(shown.contains(language.text(expected).as_ref()), "{language:?}: missing {expected} in {shown}");
        }
        assert!(shown.contains(tomlook::msx::DEFAULT_TENANT));
        if language == tomlook::i18n::Language::Cs {
            assert!(shown.contains("Přihlásit se k MSX") && shown.contains("Je nutné přihlášení k MSX"));
        }
        assert!(quiet, "Rendering never opens a browser or copies anything");
        assert!(commands.try_recv().is_err(), "Opening the dialog never starts a check or sign-in");

        click(&mut app, language.text("Sign in to MSX").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::SignInMsx { device_code: false }));
        assert!(commands.try_recv().is_err());
        assert!(app.msx_status.busy());
        click(&mut app, language.text("Cancel").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::CancelMsx));

        notices.send(ai::Notice::Msx(msx_status(Phase::SigningIn, tomlook::msx::DEVICE_PROMPT, Some("ABCD-EF12")))).unwrap();
        app.receive();
        let (shown, quiet) = text(&mut app);
        assert!(shown.contains("ABCD-EF12") && shown.contains(language.text("Open Microsoft device sign-in").as_ref()));
        assert!(!shown.contains(language.text("Sign in to MSX").as_ref()), "No second sign-in while one runs");
        assert!(quiet, "A device code is never copied or opened automatically");

        notices.send(ai::Notice::Msx(msx_status(Phase::SignInRequired, tomlook::msx::auth::DEVICE_CODE_TIMEOUT, None))).unwrap();
        app.receive();
        click(&mut app, language.text("Use device code").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::SignInMsx { device_code: true }));

        notices.send(ai::Notice::Msx(msx_status(Phase::Connected, tomlook::msx::CONNECTED, None))).unwrap();
        app.receive();
        let (shown, _) = text(&mut app);
        assert!(shown.contains("someone@example.test") && shown.contains(language.text("Connected").as_ref()));
        click(&mut app, language.text("Check connection").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::CheckMsx));
        notices.send(ai::Notice::Msx(msx_status(Phase::Connected, tomlook::msx::CONNECTED, None))).unwrap();
        app.receive();
        click(&mut app, language.text("Disconnect").as_ref());
        assert!(matches!(commands.try_recv().unwrap(), ai::Command::DisconnectMsx));
        assert_eq!(app.msx_status.message, tomlook::msx::DISCONNECTED);
        assert!(commands.try_recv().is_err());
    }
}

#[test]
fn msx_settings_default_on_can_be_turned_off_and_validate_the_tenant() {
    let legacy: tomlook::storage::Preferences = serde_json::from_str(r#"{"dark":false,"language":"Cs"}"#).unwrap();
    assert!(legacy.msx_enabled);
    assert_eq!(legacy.msx_tenant, tomlook::msx::DEFAULT_TENANT);
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.loading = false;
        app.preferences.language = language;
        app.preferences_open = true;
        app.preferences_section = PreferencesSection::Msx;
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                events, ..Default::default()
            }, |ui| app.preferences_dialog(ui.ctx()));
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing MSX setting: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                }]);
            }
        };
        click(&mut app, language.text("Use MSX and Azure consumption").as_ref());
        assert!(!app.preferences.msx_enabled);
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if !preferences.msx_enabled));
        assert!(matches!(ai_commands.try_recv().unwrap(), ai::Command::MsxSettings(settings) if !settings.enabled));
        assert_eq!(app.msx_status.phase, tomlook::msx::Phase::Disabled);
        click(&mut app, language.text("Use MSX and Azure consumption").as_ref());
        assert!(matches!(ai_commands.try_recv().unwrap(), ai::Command::MsxSettings(settings) if settings.enabled));
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if preferences.msx_enabled));
        app.msx_tenant_editor = "not-a-tenant".into();
        let mut output = draw(&mut app, vec![]);
        output.textures_delta.clear();
        let mut text = String::new();
        for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
        assert!(text.contains(language.text(tomlook::msx::INVALID_TENANT).as_ref()));
        click(&mut app, language.text("Save tenant").as_ref());
        assert_eq!(app.preferences.msx_tenant, tomlook::msx::DEFAULT_TENANT, "An invalid tenant is never saved");
        assert!(receiver.try_recv().is_err());
        let tenant = "00000000-0000-0000-0000-0000000000AB";
        app.msx_tenant_editor = tenant.into();
        click(&mut app, language.text("Save tenant").as_ref());
        assert_eq!(app.preferences.msx_tenant, tenant.to_ascii_lowercase());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if preferences.msx_tenant == tenant.to_ascii_lowercase()));
        assert!(matches!(ai_commands.try_recv().unwrap(), ai::Command::MsxSettings(settings) if settings.enabled && settings.tenant == tenant.to_ascii_lowercase()));
        assert!(ai_commands.try_recv().is_err(), "Settings never start a sign-in");
    }
}

#[test]
fn concurrent_chat_results_keep_their_turn_and_complete_history() {
    let (mut app, notices, mut commands) = assistant_fixture();
    app.ask_mode = tomlook::chat::Mode::Deep;
    app.question = "Research synthetic preparation".into();
    app.ask_question();
    let ai::Command::Chat(deep) = commands.try_recv().unwrap() else {
        panic!("Missing deep request");
    };
    assert!(app.question.is_empty());
    notices
        .send(ai::Notice::ChatProgress {
            id: deep.question.id,
            progress: tomlook::chat::Progress {
                mode: tomlook::chat::Mode::Deep,
                stage: "Searching local materials".into(),
                ..Default::default()
            },
        })
        .unwrap();
    app.receive();
    app.ask_mode = tomlook::chat::Mode::Fast;
    app.question = "A quick synthetic question".into();
    app.ask_question();
    let ai::Command::Chat(fast) = commands.try_recv().unwrap() else {
        panic!("Missing quick request");
    };
    assert!(
        fast.history.is_empty(),
        "Unfinished research is not completed conversation"
    );
    notices
        .send(ai::Notice::Answer {
            id: fast.question.id,
            result: Ok(fixture_answer("Quick answer")),
        })
        .unwrap();
    app.receive();
    assert!(app.chat.turns[0].running);
    assert_eq!(
        app.chat.turns[1].answer.as_ref().unwrap().text.as_str(),
        "Quick answer"
    );
    notices
        .send(ai::Notice::Answer {
            id: deep.question.id,
            result: Ok(fixture_answer("Deep findings")),
        })
        .unwrap();
    app.receive();
    assert!(!app.chat.running());
    app.question = "Use those findings".into();
    app.ask_question();
    let ai::Command::Chat(follow_up) = commands.try_recv().unwrap() else {
        panic!("Missing follow-up");
    };
    assert_eq!(follow_up.history.len(), 2);
    assert_eq!(follow_up.history[0].answer, "Deep findings");
    assert_eq!(follow_up.history[1].answer, "Quick answer");
}

#[test]
fn chat_meeting_links_open_insights_and_preserve_drafts_and_running_work() {
    let (mut app, _, mut commands) = assistant_fixture();
    let event = tomlook::calendar::fixture(1).remove(0);
    app.calendar =
        Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
    app.preferences.view = View::Week;
    app.assistant = true;
    app.focus_assistant = true;
    app.question = "Unsent follow-up".into();
    Arc::make_mut(&mut app.chat)
        .admit(100, "Running research".into(), tomlook::chat::Mode::Deep)
        .unwrap();
    let chat = app.chat.clone();
    let context = egui::Context::default();
    let input = app
        .ask_view()
        .input(ask::Action::OpenMeeting(event.id.clone()));
    app.apply_ask_input(input, &context);
    assert_eq!(app.selected, Some(0));
    assert!(app.assistant);
    assert!(app.insights_expanded);
    assert_eq!(app.question, "Unsent follow-up");
    assert!(Arc::ptr_eq(&chat, &app.chat));
    assert!(app.chat.running());
    assert!(commands.try_recv().is_err());

    app.assistant = true;
    app.assistant_detached = true;
    let input = app.ask_view().input(ask::Action::OpenMeeting(event.id));
    app.apply_ask_input(input, &context);
    assert!(app.assistant, "A separate Ask window stays open");
    assert_eq!(app.question, "Unsent follow-up");
    assert!(app.chat.running());
    assert!(commands.try_recv().is_err());
}

#[test]
fn reviewing_ai_changes_focuses_the_exact_proposal_without_confirming_or_replaying_it() {
    use tomlook::recommendations::{Change, Recommendation};
    let (mut app, _, mut commands) = assistant_fixture();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.extra.insert("type".into(), serde_json::json!("singleInstance"));
    app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
    app.calendar_owner = Some("owner".into());
    let record = Recommendation::from_manual(&event, "owner", Change::ClearCategory,
        Local::now().fixed_offset()).unwrap();
    Arc::make_mut(&mut Arc::make_mut(&mut app.activity).recommendations).insert(record.id.clone(), record.clone());
    app.assistant = true;
    app.assistant_detached = true;
    app.question = "Unsent follow-up".into();
    let chat = app.chat.clone();
    let input = app.ask_view().input(ask::Action::ReviewChanges(vec![record.id.clone()]));
    app.apply_ask_input(input, &egui::Context::default());
    assert!(app.messages_open);
    assert!(app.recommendations_tab);
    assert_eq!(app.recommendation_selected.as_deref(), Some(record.id.as_str()));
    assert!(app.assistant_detached);
    assert_eq!(app.question, "Unsent follow-up");
    assert!(Arc::ptr_eq(&chat, &app.chat));
    assert!(matches!(commands.try_recv().unwrap(), ai::Command::RefreshCalendarAvailability));
    assert!(commands.try_recv().is_err(), "Opening the preview must never approve or submit a change");
}

#[test]
fn local_slash_is_admitted_offline_during_research_and_preserves_rejected_drafts() {
    let (mut app, _, mut ai_commands) = assistant_fixture();
    let (commands, worker_commands) = mpsc::sync_channel(1);
    let notices = Arc::new(tomlook::worker::Notices::default());
    app.worker = Some(Worker {
        commands,
        notices: notices.clone(),
        activity: Default::default(),
        stop: Default::default(),
        done: None,
    });
    app.ai_ready = false;
    Arc::make_mut(&mut app.chat)
        .admit(
            90,
            "Running synthetic research".into(),
            tomlook::chat::Mode::Deep,
        )
        .unwrap();
    app.question = "/search Northstar".into();
    app.ask_question();
    let Command::AskLocal { id, query, .. } = worker_commands.try_recv().unwrap() else {
        panic!("Not a local query");
    };
    assert_eq!(
        query,
        tomlook::chat_cache::LocalQuery::Search("Northstar".into())
    );
    assert!(app.question.is_empty());
    assert!(ai_commands.try_recv().is_err());
    Arc::make_mut(&mut app.chat).finish(id, Ok(fixture_answer("Saved match")));
    assert!(app.chat.turns[0].running);
    assert!(!app.chat.turns[1].running);
    app.worker
        .as_ref()
        .unwrap()
        .commands
        .try_send(Command::Wake)
        .unwrap();
    app.question = "/today".into();
    app.ask_question();
    assert_eq!(
        app.question, "/today",
        "Rejected local draft must not disappear"
    );
    assert_eq!(app.chat.turns.len(), 2);
    assert!(app.ai_error.as_ref().unwrap().contains("queue is full"));
}

#[test]
fn ai_calendar_creation_preview_requires_a_button_and_shows_exact_recipients_in_both_languages() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for invite in [false, true] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, receiver) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            app.preferences.language = language;
            app.calendar_owner = Some("owner@example.invalid".into());
            app.workiq_ready = true;
            let start = Local::now().fixed_offset() + chrono::Duration::days(2);
            let draft = tomlook::event_creation::Draft {
                title: "Synthetic AI planning".into(), description: "Exact approved description".into(),
                start, end: start + chrono::Duration::hours(1), category: None,
                private: true, teams: true, travel_minutes: None, language,
                attendees: if invite { tomlook::attendees::parse_input("Alex <alex@example.invalid>").unwrap() } else { vec![] },
            };
            let mut record = tomlook::event_creation::Record::new("owner@example.invalid".into(), draft.clone()).unwrap();
            record.state = tomlook::event_creation::State::Pending;
            Arc::make_mut(&mut Arc::make_mut(&mut app.activity).event_creations).insert(record.id.clone(), record.clone());
            let context = egui::Context::default();
            let show: Show = &|app, ui| {
                let record = app.activity.event_creations.values().next().unwrap().clone();
                app.event_creation_record(ui, &record);
            };
            let text = settled(&context, &mut app, show);
            assert!(text.contains("Exact approved description"), "{text}");
            assert!(text.contains(language.text("Private event").as_ref()), "{text}");
            assert!(text.contains(language.text("Teams meeting").as_ref()), "{text}");
            assert!(text.contains("owner@example.invalid"), "{text}");
            if invite {
                assert!(text.contains("alex@example.invalid"), "{text}");
                assert!(text.contains(language.text("Attendee availability has not been verified.").as_ref()), "{text}");
                assert!(text.contains(&tomlook::attendees::invitation_notice(&draft.attendees, language)), "{text}");
            }
            assert!(receiver.try_recv().is_err(), "Drawing a proposal cannot approve it");
            let input = app.ask_view().input(ask::Action::ReviewChanges(vec![record.id.clone()]));
            app.apply_ask_input(input, &context);
            assert_eq!(app.recommendation_selected.as_deref(), Some(record.id.as_str()));
            assert!(app.messages_open);
            assert!(!app.recommendations_tab, "Creation review must not be replaced by an unrelated recommendation");
            assert!(receiver.try_recv().is_err(), "Review is not approval");
            let label = if invite { "Create and send invitations" } else { "Approve and create event" };
            app.workiq_ready = false;
            click_text(&context, &mut app, show, language.text(label).as_ref());
            assert!(receiver.try_recv().is_err(), "Disconnected approval stays disabled");
            app.workiq_ready = true;
            click_text(&context, &mut app, show, language.text(label).as_ref());
            let Command::CreateEvents { account, draft: approved, parent } = receiver.try_recv().unwrap() else { panic!("Exact creation approval"); };
            assert_eq!(account, record.account);
            assert_eq!(*approved, draft);
            assert!(parent.is_none());
            assert!(receiver.try_recv().is_err());
            while let Ok(command) = ai_commands.try_recv() {
                assert!(matches!(command, ai::Command::RefreshCalendarAvailability));
            }
        }
    }
}

#[test]
fn ai_calendar_blocker_slash_submits_the_ready_research_prompt_with_its_meeting_context() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut commands) = assistant_fixture();
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.title = "BLOCKER: Synthetic planning".into();
        app.preferences.language = language;
        app.ask_mode = tomlook::chat::Mode::Auto;
        app.switch_assistant_context(Some(event.clone()));
        app.question = "/blocker".into();
        app.ask_question();
        let ai::Command::Chat(request) = commands.try_recv().unwrap() else { panic!("Expected research question"); };
        assert_eq!(request.question.question, language.text(ask::BLOCKER_PROMPT));
        assert_eq!(request.question.meeting.as_ref().unwrap().id, event.id);
        assert_eq!(request.question.language, language);
        assert_eq!(request.mode, tomlook::chat::Mode::Deep);
        assert_eq!(app.ask_mode, tomlook::chat::Mode::Deep);
        assert_eq!(app.chat.turns.back().unwrap().question, "/blocker");
        assert!(commands.try_recv().is_err(), "The slash command cannot approve calendar writes");
    }
}

#[test]
fn ai_calendar_removal_preview_requires_approval_and_labels_notification_consequences() {
    use tomlook::recommendations::{Change, Recommendation};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for invite in [false, true] {
            let (mut app, _, _) = assistant_fixture();
            let (sender, receiver) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            let mut event = tomlook::calendar::fixture(1).remove(0);
            event.title = if invite { "Synthetic organized meeting" } else { "BLOCKER: Synthetic planning" }.into();
            event.extra.insert("isOrganizer".into(), serde_json::json!(true));
            event.extra.insert("type".into(), serde_json::json!("singleInstance"));
            event.extra.remove("tomlook_attendees_total");
            event.attendees = if invite { vec!["Alex <alex@example.invalid>".into()] } else { vec![] };
            let now = Local::now().fixed_offset();
            let change = tomlook::recommendations::removal_change(&event).unwrap();
            let record = Recommendation::from_manual(&event, "owner", change.clone(), now).unwrap();
            app.preferences.language = language;
            app.calendar_owner = Some("owner".into());
            app.calendar = Arc::new(Calendar::build(vec![event], Default::default(), vec![]).unwrap());
            Arc::make_mut(&mut Arc::make_mut(&mut app.activity).recommendations).insert(record.id.clone(), record.clone());
            let show: Show = &|app, ui| {
                let record = app.activity.recommendations.values().next().unwrap().clone();
                let calendar = app.calendar.clone();
                app.recommendation_card(ui, &record, &calendar, Local::now().fixed_offset());
            };
            let context = egui::Context::default();
            let text = settled(&context, &mut app, show);
            assert!(text.contains(language.text("This occurrence only").as_ref()), "{text}");
            let notice = if invite { "Notifies all attendees." } else { "No attendees will be notified." };
            assert!(text.contains(language.text(notice).as_ref()), "{text}");
            assert!(receiver.try_recv().is_err(), "Viewing a removal must never approve it");
            click_text(&context, &mut app, show, language.text("Approve change").as_ref());
            let Command::ConfirmRecommendation { id, change: approved } = receiver.try_recv().unwrap()
                else { panic!("Expected exact existing-event approval"); };
            assert_eq!(id, record.id);
            assert_eq!(approved, change);
            assert_eq!(matches!(approved, Change::Delete), !invite);
            assert!(receiver.try_recv().is_err());
        }
    }
}

#[test]
fn ai_calendar_meeting_menu_runs_prepare_and_blocker_for_the_clicked_meeting_without_erasing_drafts() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for (command, label) in [
            (ask::Command::Prepare, "Prepare for this meeting"),
            (ask::Command::Blocker, "Check BLOCKER"),
        ] {
            let (mut app, _, mut ai_commands) = assistant_fixture();
            let (sender, writes) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands: sender, notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()), done: None,
            });
            app.calendar = Arc::new(Calendar::build(tomlook::calendar::fixture(2), Default::default(), vec![]).unwrap());
            let event = app.calendar.events[1].clone();
            app.preferences.language = language;
            app.ask_mode = tomlook::chat::Mode::Auto;
            app.selected = Some(0);
            app.question = "Keep the global question".into();
            assert!(app.switch_assistant_context(Some(event.clone())));
            app.question = "Keep this meeting's unsent question".into();
            assert!(app.switch_assistant_context(None));
            let context = egui::Context::default();
            let menu: Show = &|app, ui| app.event_card(ui, 1,
                egui::Rect::from_min_size(egui::pos2(50.0, 50.0), Vec2::new(300.0, 90.0)));
            settled(&context, &mut app, menu);
            let point = egui::pos2(200.0, 95.0);
            draw_with(&context, &mut app, vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
            ], menu);
            draw_with(&context, &mut app, vec![
                egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
            ], menu);
            assert!(ai_commands.try_recv().is_err(), "Opening the menu never starts AI");
            click_text(&context, &mut app, menu, language.text(label).as_ref());
            let ai::Command::Chat(request) = ai_commands.try_recv().unwrap() else { panic!("Expected a contextual Ask command"); };
            let ask::Submission::Question(expected) = ask::expand(command.name(), Some(&event), language).unwrap()
                else { panic!("Expected the existing slash prompt"); };
            assert_eq!(request.question.question, expected);
            assert_eq!(request.question.meeting.as_ref().unwrap().fingerprint(), event.fingerprint());
            assert_eq!(request.mode, tomlook::chat::Mode::Deep);
            assert_eq!(request.question.language, language);
            assert_eq!(app.chat.turns.back().unwrap().question, command.name());
            assert_eq!(app.assistant_context.as_ref().unwrap().id, event.id);
            assert_eq!(app.selected, Some(0), "Right-click context is independent of the selected meeting");
            assert_eq!(app.question, "Keep this meeting's unsent question");
            assert!(app.assistant && app.focus_assistant);
            assert!(app.meeting_menu.is_none());
            assert!(ai_commands.try_recv().is_err(), "One click starts exactly one request");
            assert!(writes.try_recv().is_err(), "Research never submits a calendar write");
            Arc::make_mut(&mut app.chat).finish(request.question.id, Ok(fixture_answer("Complete")));
            assert!(app.switch_assistant_context(None));
            assert_eq!(app.question, "Keep the global question");
        }
    }
}

#[test]
fn ai_calendar_meeting_menu_shortcuts_are_disabled_offline_and_do_not_interrupt_another_context() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        for connected in [false, true] {
            for label in ["Prepare for this meeting", "Check BLOCKER"] {
                let (mut app, _, mut commands) = assistant_fixture();
                app.calendar = Arc::new(Calendar::build(tomlook::calendar::fixture(2), Default::default(), vec![]).unwrap());
                app.preferences.language = language;
                let original = app.calendar.events[0].clone();
                assert!(app.switch_assistant_context(Some(original.clone())));
                app.question = "Unsent question in the original context".into();
                app.ai_ready = connected;
                if connected {
                    Arc::make_mut(&mut app.chat).admit(90, "Running research".into(), tomlook::chat::Mode::Deep).unwrap();
                }
                let context = egui::Context::default();
                let menu: Show = &|app, ui| app.meeting_actions(ui, 1);
                let text = settled(&context, &mut app, menu);
                assert!(text.contains(language.text(label).as_ref()), "{text}");
                click_text(&context, &mut app, menu, language.text(label).as_ref());
                assert_eq!(app.assistant_context.as_ref().unwrap().id, original.id);
                assert_eq!(app.question, "Unsent question in the original context");
                assert!(commands.try_recv().is_err());
                if connected {
                    assert!(app.chat.turns[0].running);
                    assert!(app.ai_error.as_ref().unwrap().contains("Stop the current question"));
                }
            }
        }
    }
}

#[test]
fn review_slashes_enqueue_typed_background_work_without_an_ask_or_calendar_write() {
    use tomlook::recommendations::Review;
    let (mut app, _, mut ai_commands) = assistant_fixture();
    let (commands, worker_commands) = mpsc::sync_channel(1);
    app.worker = Some(Worker {
        commands,
        notices: Arc::new(Default::default()),
        activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        done: None,
    });
    app.calendar_owner = Some("owner".into());
    for (question, expected) in [
        ("/travel", Review::Travel),
        ("/categories", Review::Categories),
    ] {
        app.question = question.into();
        app.ask_question();
        let Command::ReviewRecommendations { kind, account } =
            worker_commands.try_recv().unwrap()
        else {
            panic!("Review did not enter the storage queue");
        };
        assert_eq!(kind, expected);
        assert_eq!(account, "owner");
        assert!(app.active_request.is_none());
        assert!(app.answer.as_ref().unwrap().text.contains("14"));
        assert!(ai_commands.try_recv().is_err());
    }
    app.question = "/travel".into();
    app.ask_question();
    app.question = "/travel".into();
    app.ask_question();
    assert!(app.ai_error.as_ref().unwrap().contains("queue is full"));
    assert!(ai_commands.try_recv().is_err());
}

#[test]
fn external_connection_dialog_masks_keys_dispatches_once_and_requires_forget_confirmation() {
    let (mut app, notices, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    let draw = |app: &mut Tomlook, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1366.0, 768.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                app.shortcuts(ui.ctx());
                app.connection_dialog(ui.ctx());
            },
        );
        output.textures_delta.clear();
        output
    };
    let click = |app: &mut Tomlook, label: &str| {
        draw(app, Vec::new());
        let output = draw(app, Vec::new());
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing button: {label}"));
        draw(
            app,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    };
    app.connection_panel = Some(ConnectionPanel::External(tomlook::connectors::Kind::Webiq));
    app.webiq_key = "synthetic-never-rendered-key".into();
    draw(&mut app, Vec::new());
    let output = draw(&mut app, Vec::new());
    let mut text = String::new();
    for shape in output.shapes {
        append_shape_text(&shape.shape, &mut text);
    }
    assert!(!text.contains("synthetic-never-rendered-key"));
    click(&mut app, "Connect and save key");
    assert!(
        matches!(commands.try_recv(), Ok(ai::Command::ConnectExternal { kind: tomlook::connectors::Kind::Webiq, input: tomlook::connectors::Input::WebiqKey(input) }) if input == "synthetic-never-rendered-key")
    );
    assert!(app.webiq_key.is_empty());
    assert!(app.external_status[&tomlook::connectors::Kind::Webiq].busy);
    assert!(commands.try_recv().is_err());
    notices
        .send(ai::Notice::External {
            kind: tomlook::connectors::Kind::Webiq,
            busy: false,
            ready: true,
            message: "Connected".into(),
            client_id: None,
        })
        .unwrap();
    app.receive();
    assert!(app.external_status[&tomlook::connectors::Kind::Webiq].ready);
    app.question = "Keep my unsent question".into();
    app.answer = Some(fixture_answer("Previous connected answer"));
    app.assistant_states.insert(
        Some("cached-meeting".into()),
        AssistantState {
            question: "Keep the cached question".into(),
            answer: Some(fixture_answer("Cached connected answer")),
            ..Default::default()
        },
    );
    click(&mut app, "Disconnect and forget");
    assert_eq!(app.forget_connector, Some(tomlook::connectors::Kind::Webiq));
    assert!(
        commands.try_recv().is_err(),
        "First click must not delete a saved sign-in"
    );
    assert!(
        app.answer.is_some(),
        "The first confirmation must not clear answers"
    );
    click(&mut app, "Remove saved sign-in");
    assert!(matches!(
        commands.try_recv(),
        Ok(ai::Command::ForgetExternal(
            tomlook::connectors::Kind::Webiq
        ))
    ));
    assert!(app.forget_connector.is_none());
    assert!(app.answer.is_none());
    assert!(
        app.assistant_states
            .values()
            .all(|state| state.answer.is_none())
    );
    assert_eq!(app.question, "Keep my unsent question");
    assert_eq!(
        app.assistant_states[&Some("cached-meeting".into())].question,
        "Keep the cached question"
    );
    app.connection_panel = Some(ConnectionPanel::External(tomlook::connectors::Kind::Github));
    app.github_client_id = " Iv23fixture ".into();
    click(&mut app, "Sign in");
    assert!(
        matches!(commands.try_recv(), Ok(ai::Command::ConnectExternal { kind: tomlook::connectors::Kind::Github, input: tomlook::connectors::Input::GithubApp(input) }) if input == "Iv23fixture")
    );
    assert!(commands.try_recv().is_err());
    assert_eq!(app.preferences.github_client_id, "Iv23fixture");
    click(&mut app, "Cancel");
    assert!(matches!(
        commands.try_recv(),
        Ok(ai::Command::CancelExternal(
            tomlook::connectors::Kind::Github
        ))
    ));
    app.webiq_key = "clear-on-dialog-close".into();
    draw(
        &mut app,
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(app.connection_panel.is_none());
    assert!(app.webiq_key.is_empty());
    assert_eq!(app.github_client_id.trim(), "Iv23fixture");
}

#[test]
fn external_reconnection_invalidates_answers_but_preserves_unsent_questions() {
    let (mut app, notices, mut commands) = assistant_fixture();
    app.external_status.insert(
        tomlook::connectors::Kind::Github,
        ExternalStatus {
            ready: true,
            ..Default::default()
        },
    );
    app.active_request = Some(17);
    app.question = "Keep my draft".into();
    app.answer = Some(fixture_answer("Previous account answer"));
    app.assistant_states.insert(
        None,
        AssistantState {
            question: "Global draft".into(),
            answer: Some(fixture_answer("Old global answer")),
            ..Default::default()
        },
    );
    notices
        .send(ai::Notice::External {
            kind: tomlook::connectors::Kind::Github,
            busy: false,
            ready: true,
            message: "Connected as new-fixture-owner".into(),
            client_id: Some("Iv23fixture".into()),
        })
        .unwrap();
    app.receive();
    assert!(matches!(commands.try_recv(), Ok(ai::Command::Cancel(17))));
    assert!(app.active_request.is_none());
    assert!(app.answer.is_none());
    assert!(app.assistant_states[&None].answer.is_none());
    assert_eq!(app.question, "Keep my draft");
    assert_eq!(app.assistant_states[&None].question, "Global draft");
}

#[test]
fn slow_progress_and_burst_notifications_leave_text_navigation_and_cancel_responsive() {
    let (mut app, output, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    let (storage_commands, _storage_input) = std::sync::mpsc::sync_channel(64);
    app.worker = Some(Worker {
        commands: storage_commands,
        notices: Arc::new(tomlook::worker::Notices::default()),
        activity: Arc::new(std::sync::Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        done: None,
    });
    app.calendar = Arc::new(
        Calendar::build(
            tomlook::calendar::fixture(5000),
            tomlook::calendar::fixture_coverage(),
            Vec::new(),
        )
        .unwrap(),
    );
    app.anchor = tomlook::calendar::fixture_anchor();
    app.loading = false;
    app.assistant = true;
    app.focus_assistant = true;
    app.active_request = Some(7);
    let progress = app.ai.as_ref().unwrap().progress.clone();
    app.ai
        .as_ref()
        .unwrap()
        .occupied
        .store(2, std::sync::atomic::Ordering::Release);
    let (locked, lock_ready) = std::sync::mpsc::sync_channel(1);
    let (release, released) = std::sync::mpsc::sync_channel(1);
    let (finished, worker_done) = std::sync::mpsc::sync_channel(1);
    let provider = std::thread::spawn(move || {
        let mut value = progress.lock().unwrap();
        value.id = 7;
        value.text = Arc::new("Delayed synthetic provider progress".into());
        locked.send(()).unwrap();
        let released_early = released
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok();
        drop(value);
        finished.send(released_early).unwrap();
    });
    lock_ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    for index in 0..200 {
        output
            .send(ai::Notice::Sources(format!("Synthetic update {index}")))
            .unwrap();
    }
    let draw = |app: &mut Tomlook, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1366.0, 768.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                app.receive();
                app.style(ui.ctx());
                app.shortcuts(ui.ctx());
                egui::Panel::top("test-header").show(ui, |ui| app.header(ui));
                if app.assistant {
                    egui::Panel::right("test-assistant")
                        .exact_size(380.0)
                        .show(ui, |ui| app.assistant_panel(ui));
                }
                egui::CentralPanel::default().show(ui, |ui| app.timeline(ui));
            },
        );
        output.textures_delta.clear();
    };
    draw(&mut app, Vec::new());
    draw(&mut app, vec![egui::Event::Text("Normal editing".into())]);
    assert_eq!(app.question, "Normal editing");
    assert_eq!(app.active_request, Some(7));
    assert_eq!(app.source_status, "Synthetic update 199");
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    draw(&mut app, vec![key(Key::F, Modifiers::CTRL)]);
    assert_eq!(
        context.memory(|memory| memory.focused()),
        Some(Id::new("calendar-search"))
    );
    draw(&mut app, vec![egui::Event::Text("Synthetic".into())]);
    assert_eq!(app.query, "Synthetic");
    draw(&mut app, vec![key(Key::ArrowRight, Modifiers::ALT)]);
    assert_ne!(app.anchor, tomlook::calendar::fixture_anchor());
    draw(&mut app, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!app.assistant);
    assert_eq!(app.active_request, Some(7));
    assert!(commands.try_recv().is_err());
    app.cancel_ai();
    assert!(matches!(commands.try_recv(), Ok(ai::Command::Cancel(7))));
    assert!(commands.try_recv().is_err());
    assert_eq!(app.calendar.events.len(), 5000);
    assert!(
        worker_done.try_recv().is_err(),
        "The UI waited for the slow progress lock instead of reading nonblockingly"
    );
    release.send(()).unwrap();
    assert!(
        worker_done
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
    );
    provider.join().unwrap();
}

#[test]
fn ai_notices_are_drained_without_storage_notices_and_stale_answers_are_ignored() {
    let output = Arc::new(ai::Notices::default());
    let notices = output.clone();
    let (commands, _input) = tokio::sync::mpsc::channel(1);
    let (stop, _stopped) = tokio::sync::watch::channel(false);
    let engine = Engine {
        commands,
        notices,
        progress: Arc::new(std::sync::Mutex::new(ai::Progress::default())),
        done: None,
        ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        occupied: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        terminated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        analysis_revision: Arc::new(std::sync::OnceLock::new()),
        workiq_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stop,
    };
    let mut app = Tomlook::initial(None, Some(engine), None);
    output
        .send(ai::Notice::Connection {
            ready: true,
            message: "Test connection".into(),
        })
        .unwrap();
    app.receive();
    assert!(app.ai_ready);
    assert_eq!(app.connection, "Test connection");
    app.active_request = Some(2);
    output
        .send(ai::Notice::Answer {
            id: 1,
            result: Ok(fixture_answer("Old meeting")),
        })
        .unwrap();
    app.receive();
    assert!(app.answer.is_none());
    assert_eq!(app.active_request, Some(2));
    output
        .send(ai::Notice::Answer {
            id: 2,
            result: Ok(fixture_answer("Current request")),
        })
        .unwrap();
    app.receive();
    assert_eq!(
        app.answer.as_ref().map(|answer| answer.text.as_str()),
        Some("Current request")
    );
    assert!(app.active_request.is_none());
}

#[test]
fn global_and_meeting_drafts_topics_and_last_answers_do_not_leak() {
    let (mut app, _, _input) = assistant_fixture();
    let meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
    app.question = "Global question".into();
    app.answer = Some(fixture_answer("Global answer"));
    assert!(app.switch_assistant_context(Some(meeting.clone())));
    assert!(app.question.is_empty());
    assert!(app.answer.is_none());
    app.question = "Meeting question".into();
    app.answer = Some(fixture_answer("Meeting answer"));
    assert!(app.switch_assistant_context(None));
    assert_eq!(app.question, "Global question");
    assert_eq!(app.answer.as_ref().unwrap().text.as_str(), "Global answer");
    assert!(app.switch_assistant_context(Some(meeting)));
    assert_eq!(app.question, "Meeting question");
    assert_eq!(app.answer.as_ref().unwrap().text.as_str(), "Meeting answer");
}

#[test]
fn busy_context_is_frozen_and_its_answer_stays_in_its_original_scope() {
    let (mut app, output, mut input) = assistant_fixture();
    app.question = "Global question".into();
    app.ask_question();
    let Some(ai::Command::Chat(request)) = input.try_recv().ok() else {
        panic!("question was not admitted");
    };
    let question = request.question;
    let meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
    assert!(!app.switch_assistant_context(Some(meeting.clone())));
    assert!(input.try_recv().is_err());
    assert!(app.assistant_context.is_none());
    assert_eq!(app.active_request, Some(question.id));
    output
        .send(ai::Notice::Answer {
            id: question.id,
            result: Ok(fixture_answer("Late global answer")),
        })
        .unwrap();
    app.receive();
    assert_eq!(
        app.answer.as_ref().unwrap().text.as_str(),
        "Late global answer"
    );
    assert!(app.active_request.is_none());
    assert!(app.switch_assistant_context(Some(meeting)));
    assert!(app.answer.is_none());
    assert!(app.switch_assistant_context(None));
    assert!(app.question.is_empty());
    assert_eq!(app.chat.turns.back().unwrap().question, "Global question");
    assert_eq!(
        app.answer.as_ref().unwrap().text.as_str(),
        "Late global answer"
    );
}

#[test]
fn ask_presentation_changes_keep_one_request_and_hidden_completion() {
    let (mut app, notices, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    app.assistant = true;
    app.question = "Summarize today".into();
    app.ask_question();
    let ai::Command::Chat(request) = commands.try_recv().unwrap() else {
        panic!("Question not admitted");
    };
    let question = request.question;
    assert!(!question.suggest_drafts);
    assert!(!question.question.starts_with('/'));
    for action in [ask::Action::Detach, ask::Action::Dock, ask::Action::Hide] {
        app.apply_ask_input(app.ask_view().input(action), &context);
        assert_eq!(app.active_request, Some(question.id));
        assert!(app.question.is_empty());
        assert_eq!(app.chat.turns.back().unwrap().question, "Summarize today");
        assert!(commands.try_recv().is_err());
    }
    notices
        .send(ai::Notice::Answer {
            id: question.id + 1,
            result: Ok(fixture_answer("Stale result")),
        })
        .unwrap();
    app.receive();
    assert!(!app.ask_unread);
    notices
        .send(ai::Notice::Answer {
            id: question.id,
            result: Ok(fixture_answer("Finished while hidden")),
        })
        .unwrap();
    app.receive();
    assert!(app.ask_unread);
    assert!(!app.messages_open);
    app.toggle_assistant(&context);
    assert!(!app.ask_unread);
    assert_eq!(
        app.answer.as_ref().unwrap().text.as_str(),
        "Finished while hidden"
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn detached_edits_survive_parent_updates_and_transfer_on_docking() {
    let (mut app, notices, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    app.assistant = true;
    app.assistant_detached = true;
    app.question = "Original question".into();
    app.sync_detached_ask(&context);
    app.detached_ask.lock().unwrap().view.question = "Edited in the native window".into();
    notices
        .send(ai::Notice::Sources("Unrelated background update".into()))
        .unwrap();
    app.receive();
    app.sync_detached_ask(&context);
    let input = app
        .detached_ask
        .lock()
        .unwrap()
        .view
        .input(ask::Action::Dock);
    assert_eq!(input.question, "Edited in the native window");
    app.ask_output.try_send(input).unwrap();
    app.receive_ask_inputs(&context);
    assert!(!app.assistant_detached);
    assert_eq!(app.question, "Edited in the native window");
    assert!(commands.try_recv().is_err());
}

#[test]
fn calendar_selection_does_not_retarget_an_active_contextual_question() {
    let (mut app, _, mut commands) = assistant_fixture();
    app.calendar = Arc::new(
        Calendar::build(tomlook::calendar::fixture(2), Default::default(), vec![]).unwrap(),
    );
    let original = app.calendar.events[0].clone();
    assert!(app.switch_assistant_context(Some(original.clone())));
    app.question = "/prepare".into();
    app.ask_question();
    let ai::Command::Chat(request) = commands.try_recv().unwrap() else {
        panic!("Question not admitted");
    };
    let question = request.question;
    app.select(1);
    assert_eq!(app.assistant_context.as_ref().unwrap().id, original.id);
    assert_eq!(question.meeting.as_ref().unwrap().id, original.id);
    assert!(app.active_request.is_none(), "Preparation uses the Thorough lane, not the quick-answer slot");
    assert!(app.chat.turns.iter().any(|turn| turn.id == question.id && turn.running));
    assert!(commands.try_recv().is_err());
    app.apply_ask_input(
        app.ask_view().input(ask::Action::Stop),
        &egui::Context::default(),
    );
    assert!(matches!(commands.try_recv(), Ok(ai::Command::Cancel(id)) if id == question.id));
    assert!(app.active_request.is_none());
}

#[test]
fn detached_submission_rejects_a_changed_meeting_snapshot() {
    let (mut app, _, mut commands) = assistant_fixture();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    app.switch_assistant_context(Some(event.clone()));
    app.question = "Review this meeting".into();
    let stale = app.ask_view().input(ask::Action::Submit);
    event.title.push_str(" changed");
    app.switch_assistant_context(Some(event));
    app.apply_ask_input(stale, &egui::Context::default());
    assert!(commands.try_recv().is_err());
    assert!(app.ai_error.as_ref().unwrap().contains("context changed"));
}

#[test]
fn detached_scope_changes_save_unsent_edits_and_do_not_leak_between_empty_contexts() {
    let (mut app, _, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    app.assistant_detached = true;
    app.sync_detached_ask(&context);
    app.detached_ask.lock().unwrap().view.question = "Unsent global question".into();
    let event = tomlook::calendar::fixture(1).remove(0);
    assert!(app.switch_assistant_context(Some(event)));
    app.sync_detached_ask(&context);
    assert!(app.detached_ask.lock().unwrap().view.question.is_empty());
    assert_eq!(
        app.assistant_states[&None].question,
        "Unsent global question"
    );
    assert!(app.switch_assistant_context(None));
    app.sync_detached_ask(&context);
    assert_eq!(
        app.detached_ask.lock().unwrap().view.question,
        "Unsent global question"
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn same_context_reopen_preserves_request_but_changed_snapshot_invalidates_answer() {
    let (mut app, _, mut input) = assistant_fixture();
    let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
    app.switch_assistant_context(Some(meeting.clone()));
    app.question = "Question".into();
    app.ask_question();
    assert!(matches!(input.try_recv().unwrap(), ai::Command::Chat(_)));
    let active = app.active_request;
    assert!(app.switch_assistant_context(Some(meeting.clone())));
    assert_eq!(app.active_request, active);
    assert!(input.try_recv().is_err());
    app.cancel_ai();
    app.question = "New unsent draft".into();
    app.answer = Some(fixture_answer("Previous answer"));
    app.switch_assistant_context(None);
    meeting.title = "Updated meeting".into();
    app.switch_assistant_context(Some(meeting));
    assert!(app.answer.is_none());
    assert_eq!(app.question, "New unsent draft");
    assert!(app.ai_error.as_ref().unwrap().contains("invalidated"));
}

#[test]
fn bounded_contexts_refuse_new_scope_without_evicting_drafts_or_allowing_a_question() {
    let (mut app, _, mut input) = assistant_fixture();
    let mut meeting = tomlook::calendar::demo(Local::now(), 1).remove(0);
    app.question = "Preserved global draft".into();
    for index in 0..ASSISTANT_CONTEXT_LIMIT - 1 {
        meeting.id = format!("context-{index}");
        assert!(app.switch_assistant_context(Some(meeting.clone())));
    }
    assert_eq!(app.assistant_states.len(), ASSISTANT_CONTEXT_LIMIT - 1);
    meeting.id = "overflow".into();
    assert!(!app.switch_assistant_context(Some(meeting)));
    app.question = "Must not use the old meeting".into();
    app.ask_question();
    assert!(app.context_error.is_some());
    assert!(app.active_request.is_none());
    assert!(input.try_recv().is_err());
    assert!(app.switch_assistant_context(None));
    assert_eq!(app.question, "Preserved global draft");
    assert!(app.context_error.is_none());
}

#[test]
fn admission_is_single_flight_and_rejection_preserves_the_last_answer() {
    let (mut app, _, mut input) = assistant_fixture();
    app.question = "Question".into();
    app.ask_question();
    app.ask_question();
    assert!(matches!(input.try_recv().unwrap(), ai::Command::Chat(_)));
    assert!(input.try_recv().is_err());
    app.cancel_ai();
    assert!(matches!(input.try_recv().unwrap(), ai::Command::Cancel(_)));
    app.answer = Some(fixture_answer("Last completed answer"));
    drop(input);
    app.question = "Retry after worker failure".into();
    app.ask_question();
    assert!(app.active_request.is_none());
    assert_eq!(
        app.answer.as_ref().unwrap().text.as_str(),
        "Last completed answer"
    );
    assert!(app.ai_error.as_ref().unwrap().contains("unavailable"));
}

#[test]
fn request_and_payload_limits_fail_before_submission_and_never_alias_preparation_ids() {
    let (mut app, output, mut input) = assistant_fixture();
    app.question = "\u{010D}".repeat(4001);
    app.ask_question();
    assert!(input.try_recv().is_err());
    assert_eq!(app.request_id, 0);
    app.question = "Question".into();
    app.request_id = (1 << 63) - 1;
    app.ask_question();
    assert!(input.try_recv().is_err());
    assert!(app.ai_error.as_ref().unwrap().contains("identity limit"));
    app.active_request = Some(1);
    output
        .send(ai::Notice::Answer {
            id: 1,
            result: Ok(fixture_answer("x".repeat(64_001))),
        })
        .unwrap();
    app.receive();
    assert!(app.answer.is_none());
    assert!(app.active_request.is_none());
    assert!(app.ai_error.as_ref().unwrap().contains("response limit"));
}

#[test]
fn workiq_identity_changes_invalidate_answers_without_erasing_local_questions() {
    let (mut app, _, _input) = assistant_fixture();
    app.question = "Current local question".into();
    app.answer = Some(fixture_answer("Current old-account result"));
    app.assistant_states.insert(
        Some("fixture-other".into()),
        AssistantState {
            question: "Other local question".into(),
            answer: Some(fixture_answer("Other old-account result")),
            ..Default::default()
        },
    );
    app.invalidate_connection_answers();
    assert!(app.answer.is_none());
    let other = &app.assistant_states[&Some("fixture-other".into())];
    assert!(other.answer.is_none());
    assert_eq!(other.question, "Other local question");
    assert_eq!(app.question, "Current local question");
}

#[test]
fn connection_dialog_does_not_apply_calendar_shortcuts_and_escape_closes_it() {
    let (mut app, _, _input) = assistant_fixture();
    let context = egui::Context::default();
    app.preferences.view = View::WorkWeek;
    app.connection_panel = Some(ConnectionPanel::Workiq);
    let key_input = |key| egui::RawInput {
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut output = context.run_ui(key_input(Key::Num1), |ui| app.shortcuts(ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(app.preferences.view, View::WorkWeek);
    assert!(app.connection_panel.is_some());
    let mut output = context.run_ui(key_input(Key::Escape), |ui| {
        app.shortcuts(ui.ctx());
        app.header(ui);
    });
    output.textures_delta.clear();
    assert!(app.connection_panel.is_none());
    assert_eq!(
        context.memory(|memory| memory.focused()),
        Some(Id::new("calendar-search"))
    );
}

#[test]
fn simple_question_panel_renders_markdown_without_concept_controls() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut commands) = assistant_fixture();
        app.preferences.language = language;
        app.assistant = true;
        app.focus_assistant = true;
        app.answer = Some(fixture_answer(
            "# Overview\n\n**Important** and *useful*.\n\n- First step\n- Second step\n\n[Source](https://example.com)\n\n```text\nsample code\n```",
        ));
        let context = egui::Context::default();
        let mut rendered = String::new();
        let mut heading_size = 0.0_f32;
        let mut body_size = 0.0_f32;
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1366.0, 768.0),
                    )),
                    ..Default::default()
                },
                |ui| app.assistant_panel(ui),
            );
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut rendered);
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    if text.galley.job.text == "Overview" {
                        heading_size = text.galley.job.sections[0].format.font_id.size;
                    }
                    if text.galley.job.text == "Important and useful." {
                        body_size = text.galley.job.sections[0].format.font_id.size;
                    }
                }
            }
            output.textures_delta.clear();
        }
        assert!(
            rendered.contains(language.text("Ask AI").as_ref()),
            "{rendered}"
        );
        assert!(rendered.contains("Important and useful."), "{rendered}");
        assert!(rendered.contains("First step") && rendered.contains("sample code"));
        assert!(!rendered.contains("**Important**") && !rendered.contains("```"));
        assert!(
            !rendered.contains(language.text("Suggest drafts").as_ref()),
            "{rendered}"
        );
        assert!(
            !rendered.contains(language.text("New local draft").as_ref()),
            "{rendered}"
        );
        assert!(
            !rendered.contains(language.text("More options").as_ref()),
            "{rendered}"
        );
        assert!(heading_size > body_size && body_size > 0.0);
        assert!(
            commands.try_recv().is_err(),
            "Opening the panel must not ask AI"
        );
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(Id::new("assistant-question"))
        );
    }
}

#[test]
fn waiting_panel_does_not_expose_raw_partial_markdown_or_block_on_progress() {
    let (mut app, _, mut commands) = assistant_fixture();
    app.assistant = true;
    app.active_request = Some(7);
    let shared_progress = app.ai.as_ref().unwrap().progress.clone();
    let mut progress = shared_progress.lock().unwrap();
    progress.id = 7;
    progress.text = Arc::new("**Unfinished raw Markdown**".into());
    let context = egui::Context::default();
    let mut rendered = String::new();
    for _ in 0..3 {
        let mut output =
            context.run_ui(egui::RawInput::default(), |ui| app.assistant_panel(ui));
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut rendered);
        }
        output.textures_delta.clear();
    }
    assert!(rendered.contains(app.preferences.language.text("Thinking...").as_ref()));
    assert!(!rendered.contains("Unfinished raw Markdown"));
    assert!(commands.try_recv().is_err());
    drop(progress);
}

#[test]
fn opening_assistant_focuses_question_and_escape_returns_focus_to_search() {
    let (mut app, _, _input) = assistant_fixture();
    let context = egui::Context::default();
    app.assistant = true;
    app.focus_assistant = true;
    app.question = "Saved ordinary question".into();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.assistant_panel(ui);
    });
    output.textures_delta.clear();
    assert_eq!(
        context.memory(|memory| memory.focused()),
        Some(Id::new("assistant-question"))
    );
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut output = context.run_ui(input, |ui| {
        app.shortcuts(ui.ctx());
        app.header(ui);
    });
    output.textures_delta.clear();
    assert!(!app.assistant);
    assert_eq!(
        context.memory(|memory| memory.focused()),
        Some(Id::new("calendar-search"))
    );
}

#[test]
fn enter_from_question_submits_once() {
    let (mut app, _, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    app.question = "Synthetic question".into();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.assistant_panel(ui);
    });
    output.textures_delta.clear();
    context.memory_mut(|memory| memory.request_focus(Id::new("assistant-question")));
    for _ in 0..2 {
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| {
            app.assistant_panel(ui);
        });
        output.textures_delta.clear();
    }
    let ai::Command::Chat(request) = commands.try_recv().unwrap() else {
        panic!("expected one question");
    };
    let question = request.question;
    assert!(question.public_topic.is_none());
    assert!(commands.try_recv().is_err());
}

#[test]
fn shift_enter_adds_a_line_without_submitting() {
    let (mut app, _, mut commands) = assistant_fixture();
    let context = egui::Context::default();
    app.question = "Fixture".into();
    context.memory_mut(|memory| memory.request_focus(Id::new("assistant-question")));
    let mut output = context.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            }],
            ..Default::default()
        },
        |ui| app.assistant_panel(ui),
    );
    output.textures_delta.clear();
    assert!(app.question.contains('\n'));
    assert!(commands.try_recv().is_err());
}

#[test]
fn command_search_and_arrows_execute_the_filtered_command() {
    assert_eq!(command_matches("show month").len(), 1);
    let (mut app, _, _) = assistant_fixture();
    let context = egui::Context::default();
    app.preferences.view = View::Day;
    app.open_palette();
    app.palette_query = "show".into();
    for key in [Key::ArrowDown, Key::Enter] {
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.overlays(ui.ctx()),
        );
        output.textures_delta.clear();
    }
    assert!(!app.palette);
    assert_eq!(app.preferences.view, View::WorkWeek);
}

#[test]
fn mini_calendar_pointer_selection_opens_day_or_exact_inclusive_columns() {
    for (start, end, count, focused) in [
        (6, 6, 1, true),
        (6, 8, 3, true),
        (8, 6, 3, true),
        (9, 12, 4, true),
        (6, 8, 3, false),
        (8, 6, 3, false),
    ] {
        let (mut app, _, mut commands) = assistant_fixture();
        app.anchor = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        app.preferences.view = View::Month;
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, raw| {
            let mut output = context.run_ui(raw, |ui| {
                ui.set_width(192.0);
                app.sidebar(ui);
            });
            output.textures_delta.clear();
        };
        draw(&mut app, egui::RawInput::default());
        let date = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
        let position = |day| {
            context.data(|data| {
                data.get_temp::<egui::Rect>(Id::new(("mini-date", date(day))))
                    .unwrap()
                    .center()
            })
        };
        let first = position(start);
        let last = position(end);
        draw(
            &mut app,
            egui::RawInput {
                time: Some(0.1),
                events: vec![
                    egui::Event::PointerMoved(first),
                    egui::Event::PointerButton {
                        pos: first,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
        );
        draw(
            &mut app,
            egui::RawInput {
                time: Some(0.2),
                events: vec![egui::Event::PointerMoved(last)],
                ..Default::default()
            },
        );
        assert_eq!(app.visible_days().len(), count, "drag preview");
        app.refresh_workiq();
        assert!(commands.try_recv().is_err(), "drag preview must not fetch");
        draw(
            &mut app,
            egui::RawInput {
                time: Some(0.3),
                focused,
                events: vec![egui::Event::PointerButton {
                    pos: last,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
        );
        assert!(app.date_drag.is_none());
        assert_eq!(app.preferences.view, View::Day);
        assert_eq!(app.visible_days().len(), count);
        let range = app.visible_workiq_range().unwrap();
        assert_eq!(range.first, date(start.min(end)));
        assert_eq!(range.last, date(start.max(end)));
        let mut output = context.run_ui(
            egui::RawInput {
                time: Some(0.4),
                ..Default::default()
            },
            |ui| app.timeline(ui),
        );
        let labels = output.shapes.iter().filter(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if ["Mon ", "Tue ", "Wed ", "Thu ", "Fri ", "Sat ", "Sun "].iter().any(|day| text.galley.job.text.starts_with(day))
        )).count();
        output.textures_delta.clear();
        assert_eq!(labels, count, "rendered calendar columns");
    }
}

#[test]
fn preset_views_highlight_their_entire_period_after_a_day_click() {
    let (mut app, _, _) = assistant_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
    app.choose_dates(date, date);
    assert!(app.mini_selected(date));
    assert!(!app.mini_selected(date + chrono::Duration::days(1)));
    for view in [View::WorkWeek, View::Week, View::Month] {
        app.choose_view(view);
        let expected = view
            .days(date)
            .into_iter()
            .filter(|day| view != View::Month || day.month() == date.month())
            .collect::<Vec<_>>();
        for day in View::Month.days(date) {
            assert_eq!(
                app.mini_selected(day),
                expected.contains(&day),
                "{view:?} {day}"
            );
        }
    }
}

#[test]
fn mini_week_chevron_is_a_single_click_and_czech_commands_remain_searchable() {
    let (mut app, _, mut commands) = assistant_fixture();
    app.anchor = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
    app.preferences.language = tomlook::i18n::Language::Cs;
    let monday = tomlook::calendar::monday(app.anchor);
    let context = egui::Context::default();
    let draw = |app: &mut Tomlook, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| app.sidebar(ui),
        );
        output.textures_delta.clear();
    };
    draw(&mut app, Vec::new());
    let rect = context.data(|data| {
        data.get_temp::<egui::Rect>(Id::new(("mini-week", monday)))
            .unwrap()
    });
    assert!(rect.width() >= 16.0 && rect.height() >= 25.0);
    let pos = rect.center();
    draw(
        &mut app,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    draw(
        &mut app,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.preferences.view, View::WorkWeek);
    assert_eq!(app.visible_days().len(), 5);
    assert!(app.date_selection.is_none());
    assert!(app.mini_selected(monday));
    assert!(
        commands.try_recv().is_err(),
        "Picking dates must not start AI"
    );
    let matches = command_matches_in("pracovní", tomlook::i18n::Language::Cs);
    assert_eq!(matches, vec![PaletteCommand::View(View::WorkWeek)]);
}

#[test]
fn mini_calendar_dates_fit_beside_the_navigation_scrollbar() {
    for floating in [false, true] {
        let (mut app, _, _) = assistant_fixture();
        app.anchor = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        app.preferences.language = tomlook::i18n::Language::Cs;
        let context = egui::Context::default();
        let mut clip = egui::Rect::NOTHING;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 420.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.spacing_mut().scroll.floating = floating;
                egui::Panel::left("navigation")
                    .exact_size(NAVIGATION_WIDTH)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .show(ui, |ui| {
                                clip = ui.clip_rect();
                                app.sidebar(ui);
                            });
                    });
            },
        );
        output.textures_delta.clear();
        for date in View::Month.days(app.anchor) {
            let rect = context.data(|data| {
                data.get_temp::<egui::Rect>(Id::new(("mini-date", date)))
                    .unwrap()
            });
            assert!(
                clip.contains_rect(rect),
                "Date {date} clips with floating scrollbar {floating}: {rect:?}, {clip:?}"
            );
        }
    }
}

#[test]
fn ghost_filter_and_rendering_keep_titles_without_category_stripes() {
    let mut app = Tomlook::initial(None, None, None);
    let mut events = tomlook::calendar::fixture(2);
    events[0].status = "notResponded".into();
    events[0].category = "Work".into();
    events[0].title = "Today".into();
    events[1].status = "accepted".into();
    app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
    app.preferences.language = tomlook::i18n::Language::Cs;
    let query = tomlook::calendar::SearchQuery::new("");
    assert!(app.matches_event(0, &query));
    app.preferences.hide_unconfirmed = true;
    assert!(!app.matches_event(0, &query));
    assert!(app.matches_event(1, &query));
    let context = egui::Context::default();
    for height in [23.0, 38.0, 100.0] {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.event_card(
                ui,
                0,
                egui::Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, height)),
            );
        });
        output.textures_delta.clear();
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
            assert!(
                !matches!(&shape.shape, egui::epaint::Shape::LineSegment { stroke, .. } if stroke.width == 3.0),
                "Ghost must have no category stripe"
            );
        }
        assert!(text.contains("Nepotvrzeno"));
        assert!(
            text.contains("Today"),
            "Meeting titles must not be translated"
        );
    }
}

#[test]
fn czech_all_day_cards_use_a_localized_label_instead_of_a_midnight_time() {
    for status in ["accepted", "notResponded"] {
        let mut app = Tomlook::initial(None, None, None);
        let mut events = tomlook::calendar::fixture(1);
        events[0].status = status.into();
        events[0].is_all_day = true;
        events[0].title = "Today".into();
        app.calendar =
            Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
        app.preferences.language = tomlook::i18n::Language::Cs;
        let context = egui::Context::default();
        for height in [23.0, 38.0, 100.0] {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                app.event_card(
                    ui,
                    0,
                    egui::Rect::from_min_size(ui.cursor().min, Vec2::new(320.0, height)),
                );
            });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            assert!(text.contains("Celý den"), "{status}, {height}: {text}");
            assert!(text.contains("Today"), "Original title must be preserved");
            assert!(!text.contains("All day"));
            if status == "notResponded" {
                assert!(text.contains("Nepotvrzeno"));
                assert!(!text.contains("00:00"));
            }
        }
    }
}

#[test]
fn every_day_of_multiday_all_day_meetings_opens_the_same_event_details() {
    for view in [View::WorkWeek, View::Week, View::Month] {
        for status in ["accepted", "notResponded"] {
            let mut app = Tomlook::initial(None, None, None);
            let mut events = tomlook::calendar::fixture(2);
            for (index, event) in events.iter_mut().enumerate() {
                event.start = chrono::DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap()
                    + chrono::Duration::days(index as i64 * 2);
                event.end = event.start + chrono::Duration::days(2);
                event.is_all_day = true;
                event.status = status.into();
                event.title = format!("Span {index}");
            }
            app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
            app.anchor = "2026-11-02".parse().unwrap();
            app.preferences.view = view;
            app.preferences.hide_unconfirmed = false;
            let (commands, input) = mpsc::sync_channel(16);
            app.worker = Some(Worker {
                commands,
                notices: Arc::new(Default::default()),
                activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
                stop: Arc::new(Default::default()),
                done: None,
            });
            let context = egui::Context::default();
            let draw = |app: &mut Tomlook, events| {
                let mut output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1500.0, 1000.0))),
                    events,
                    ..Default::default()
                }, |ui| {
                    if view == View::Month { app.month(ui); } else { app.timeline(ui); }
                });
                output.textures_delta.clear();
                output
            };
            draw(&mut app, vec![]);
            let output = draw(&mut app, vec![]);
            for index in 0..2 {
                let title = format!("Span {index}");
                let positions = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text.ends_with(&title) =>
                        Some(text.pos + Vec2::new(2.0, 5.0)),
                    _ => None,
                }).collect::<Vec<_>>();
                assert_eq!(positions.len(), 2, "{view:?}, {status}: {title} must appear on both days");
                for (day, position) in positions.into_iter().enumerate() {
                    app.selected = None;
                    for pressed in [true, false] {
                        draw(&mut app, vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                            },
                        ]);
                    }
                    assert_eq!(app.selected, Some(index), "{view:?}, {status}: {title}, day {day}");
                    assert!(matches!(input.try_recv().unwrap(), Command::Detail { id, .. } if id == app.calendar.events[index].id));
                    assert!(input.try_recv().is_err(), "One click must request details exactly once");
                    draw(&mut app, vec![egui::Event::PointerGone]);
                }
            }
        }
    }
}

#[test]
fn all_day_timeline_reserves_empty_columns_and_details_show_inclusive_range() {
    let mut app = Tomlook::initial(None, None, None);
    let mut events = tomlook::calendar::fixture(1);
    events[0].start = chrono::DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap();
    events[0].end = chrono::DateTime::parse_from_rfc3339("2026-11-04T00:00:00Z").unwrap();
    events[0].is_all_day = true;
    events[0].title = "All-day fixture".into();
    app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
    app.anchor = "2026-11-02".parse().unwrap();
    app.preferences.language = tomlook::i18n::Language::Cs;
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.details(ui, app.calendar.events[0].clone());
    });
    output.textures_delta.clear();
    let mut text = String::new();
    for shape in &output.shapes {
        append_shape_text(&shape.shape, &mut text);
    }
    assert!(text.contains("pondělí, 2. 11. 2026"));
    assert!(text.contains("úterý, 3. 11. 2026"));
    assert!(text.contains("Celý den"));
    assert!(!text.contains("01:00"));
    assert!(!text.contains("středa, 4."));
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1400.0, 800.0),
            )),
            ..Default::default()
        },
        |ui| app.timeline(ui),
    );
    output.textures_delta.clear();
    let positions = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text)
                if text.galley.text().contains("All-day fixture") =>
            {
                Some(text.pos.x)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        positions.len(),
        2,
        "All-day cards must appear on both covered days: {positions:?}"
    );
    assert!(
        positions[0] < 300.0,
        "First card belongs to Monday: {positions:?}"
    );
    assert!(
        positions[1] > positions[0] + 200.0 && positions[1] < 600.0,
        "Second card belongs to Tuesday: {positions:?}"
    );
}

#[test]
fn dismissed_activity_error_does_not_return_on_unrelated_activity_updates() {
    let activity = Arc::new(std::sync::Mutex::new(Arc::new(tomlook::worker::Activity {
        revision: 1,
        error: Some("First failure".into()),
        ..Default::default()
    })));
    let (commands, _input) = std::sync::mpsc::sync_channel(4);
    let worker = Worker {
        commands,
        notices: Arc::new(tomlook::worker::Notices::default()),
        activity: activity.clone(),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        done: None,
    };
    let mut app = Tomlook::initial(Some(worker), None, None);
    app.receive();
    assert_eq!(app.error.as_deref(), Some("First failure"));
    *activity.lock().unwrap() = Arc::new(tomlook::worker::Activity {
        revision: 2,
        error: Some("First failure".into()),
        ..Default::default()
    });
    app.receive();
    assert_eq!(app.error.as_deref(), Some("First failure"));
    app.error = None;
    *activity.lock().unwrap() = Arc::new(tomlook::worker::Activity {
        revision: 3,
        error: Some("First failure".into()),
        ..Default::default()
    });
    app.receive();
    assert!(app.error.is_none());
    assert_eq!(app.activity.error.as_deref(), Some("First failure"));
    *activity.lock().unwrap() = Arc::new(tomlook::worker::Activity {
        revision: 4,
        error: Some("New failure".into()),
        ..Default::default()
    });
    app.receive();
    assert_eq!(app.error.as_deref(), Some("New failure"));
}

#[test]
fn czech_insights_use_saved_narrative_and_preserve_original_evidence() {
    let mut app = Tomlook::initial(None, None, None);
    app.preferences.language = tomlook::i18n::Language::Cs;
    app.briefing = Some(serde_json::json!({
        "summary": "English overview", "sources": [],
        "cs": { "summary": "Český přehled", "preparation": ["Demo: Otevřít připravenou ukázku"], "open_questions": ["Další krok: Potvrdit vlastníka"] },
        "retrieved_evidence": {"records": []}
    }));
    let context = egui::Context::default();
    for width in [320.0, 760.0] {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(width);
            app.render_insights(ui, true);
        });
        output.textures_delta.clear();
        let mut text = String::new();
        for shape in &output.shapes {
            append_shape_text(&shape.shape, &mut text);
        }
        assert!(text.contains("Český přehled"));
        assert!(!text.contains("English overview"));
        assert!(text.contains("Otevřít připravenou ukázku"));
        assert!(text.contains("Potvrdit vlastníka"));
    }
}

#[test]
fn mini_calendar_focus_loss_cancels_unreleased_preview() {
    let (mut app, _, _) = assistant_fixture();
    let first = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    app.anchor = first;
    app.preferences.view = View::Day;
    app.date_selection = Some(DateSelection::new(first, first));
    app.date_drag = Some((first, first + chrono::Duration::days(2)));
    let context = egui::Context::default();
    let mut output = context.run_ui(
        egui::RawInput {
            focused: false,
            ..Default::default()
        },
        |ui| app.sidebar(ui),
    );
    output.textures_delta.clear();
    assert!(app.date_drag.is_none());
    assert_eq!(app.visible_days(), vec![first]);
}

#[test]
fn mini_calendar_commits_drag_when_events_share_one_frame() {
    for (start, end) in [(6, 8), (8, 6)] {
        let (mut app, _, _) = assistant_fixture();
        app.anchor = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        app.preferences.view = View::Month;
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, raw| {
            let mut output = context.run_ui(raw, |ui| {
                ui.set_width(192.0);
                app.sidebar(ui);
            });
            output.textures_delta.clear();
        };
        draw(&mut app, egui::RawInput::default());
        let position = |day| {
            context.data(|data| {
                data.get_temp::<egui::Rect>(Id::new((
                    "mini-date",
                    NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
                )))
                .unwrap()
                .center()
            })
        };
        let first = position(start);
        let last = position(end);
        draw(
            &mut app,
            egui::RawInput {
                time: Some(0.1),
                events: vec![
                    egui::Event::PointerMoved(first),
                    egui::Event::PointerButton {
                        pos: first,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    egui::Event::PointerMoved(last),
                    egui::Event::PointerButton {
                        pos: last,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
        );
        assert_eq!(position(start), first, "press target moved");
        assert!(app.date_drag.is_none());
        assert_eq!(app.preferences.view, View::Day);
        assert_eq!(
            app.visible_days(),
            DateSelection::new(
                NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            )
            .days()
        );
    }
}

#[test]
fn custom_period_navigation_preserves_length_and_presets_clear_it() {
    let (mut app, _, _) = assistant_fixture();
    let first = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    app.choose_dates(first, first + chrono::Duration::days(2));
    app.shift_period(1);
    assert_eq!(
        app.visible_days(),
        DateSelection::new(
            first + chrono::Duration::days(3),
            first + chrono::Duration::days(5)
        )
        .days()
    );
    app.shift_period(-1);
    assert_eq!(app.visible_days()[0], first);
    app.choose_view(View::Week);
    assert!(app.date_selection.is_none());
    assert_eq!(app.visible_days().len(), 7);
    app.choose_dates(first, first);
    assert_eq!(app.visible_days(), vec![first]);
    assert_eq!(app.preferences.view, View::Day);
    for (view, _) in View::ALL {
        app.choose_view(view);
        app.choose_dates(first, first);
        assert_eq!(app.preferences.view, View::Day);
        assert_eq!(app.visible_days(), vec![first]);
    }
}

#[test]
fn person_addresses_and_photo_errors_are_bounded() {
    assert_eq!(
        person_email("Fixture Person <FIXTURE@example.com>"),
        Some("fixture@example.com".into())
    );
    for person in ["Fixture Person", "@example.com", "a@b@c", "person@"] {
        assert!(person_email(person).is_none());
    }
    let (mut app, notices, _) = assistant_fixture();
    app.workiq_ready = true;
    notices
        .send(ai::Notice::Photos(vec![(
            "fixture@example.com".into(),
            Err("Unavailable photo".into()),
        )]))
        .unwrap();
    app.receive();
    assert_eq!(
        app.photos["fixture@example.com"].error.as_deref(),
        Some("Unavailable photo")
    );
    notices
        .send(ai::Notice::HostedWorkiq {
            busy: false,
            ready: false,
            message: "Disconnected".into(),
        })
        .unwrap();
    app.receive();
    assert!(app.photos.is_empty());
}

#[test]
fn update_settings_offer_restart_only_for_verified_staged_updates() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let render = |app: &mut Tomlook| {
            let context = egui::Context::default();
            let mut text = String::new();
            for _ in 0..2 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1400.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.preferences_dialog(ui.ctx()),
                );
                output.textures_delta.clear();
                text.clear();
                for shape in &output.shapes {
                    append_shape_text(&shape.shape, &mut text);
                }
            }
            text
        };
        let mut app = Tomlook::initial(None, None, None);
        app.preferences.language = language;
        app.preferences_open = true;
        app.preferences_section = PreferencesSection::Updates;
        let text = render(&mut app);
        assert!(
            text.contains(
                language
                    .text("Updates are not available in this session.")
                    .as_ref()
            )
        );
        assert!(!text.contains(language.text("Check now").as_ref()));
        app.update_status = tomlook::updater::Status {
            staged: Some(tomlook::updater::Staged {
                version: "9.9.9".into(),
                notes_url: tomlook::updater::release_page("v9.9.9"),
                directory: "C:\\staging\\9.9.9".into(),
                target: "C:\\Tomlook\\Tomlook.exe".into(),
                executable_sha256: "0".repeat(64),
            }),
            ..Default::default()
        };
        let text = render(&mut app);
        assert!(text.contains(language.text("Restart now").as_ref()));
        assert!(
            text.contains(
                language
                    .text("Tomlook 9.9.9 is downloaded and verified.")
                    .as_ref()
            )
        );
        assert!(
            text.contains(
                language
                    .text("Otherwise the update is installed when you exit Tomlook.")
                    .as_ref()
            )
        );
        assert!(text.contains(language.text("Check now").as_ref()));
        app.update_status = tomlook::updater::Status {
            error: Some(tomlook::updater::INVALID_SIGNATURE.into()),
            ..Default::default()
        };
        let text = render(&mut app);
        assert!(text.contains(language.text(tomlook::updater::INVALID_SIGNATURE).as_ref()));
        assert!(!text.contains(language.text("Restart now").as_ref()));
        assert!(!text.contains(language.text("Open release page").as_ref()));
        assert!(!text.contains(language.text("Tomlook is up to date.").as_ref()));
        app.update_status = tomlook::updater::Status {
            offer: Some(tomlook::updater::Offer {
                version: "9.9.9".into(),
                notes_url: tomlook::updater::release_page("v9.9.9"),
                reason: "Tomlook cannot write to its own folder; download the update from the release page.".into(),
            }),
            ..Default::default()
        };
        let text = render(&mut app);
        assert!(text.contains(language.text("Open release page").as_ref()));
        assert!(!text.contains(language.text("Restart now").as_ref()));
    }
}

#[test]
fn notification_settings_save_reminder_preferences_and_report_status_in_both_languages() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let mut app = Tomlook::initial(None, None, None);
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.loading = false;
        app.preferences.language = language;
        app.preferences_open = true;
        app.preferences_section = PreferencesSection::Notifications;
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                events, ..Default::default()
            }, |ui| app.preferences_dialog(ui.ctx()));
            output.textures_delta.clear();
            output
        };
        let text = |app: &mut Tomlook| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            text
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing notification control: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos,button:egui::PointerButton::Primary,pressed,modifiers:Modifiers::NONE,
                }]);
            }
        };
        let rendered = text(&mut app);
        for expected in ["Notifications", "Send test notification", "Notifications are not available in this session.",
            "Reminders cover timed meetings you organize or have accepted. Unanswered, declined, cancelled, free and followed meetings are skipped."] {
            assert!(rendered.contains(language.text(expected).as_ref()), "{language:?}: {expected}");
        }
        click(&mut app, "10 min");
        assert_eq!(app.preferences.reminder_minutes, 10);
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences)
            if preferences.reminder_minutes == 10 && preferences.meeting_reminders));
        click(&mut app, language.text("Also remind me of tentatively accepted meetings").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if preferences.remind_tentative));
        click(&mut app, language.text("Remind me before meetings").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if !preferences.meeting_reminders));
        app.reminder_status = tomlook::reminders::Status {
            windows: Some(tomlook::reminders::WindowsSetting::DisabledForApplication),
            error: Some("Show the Windows notification: Access is denied.".into()),
            ..Default::default()
        };
        let rendered = text(&mut app);
        assert!(rendered.contains(language.text("Meeting reminders are turned off.").as_ref()));
        assert!(rendered.contains(language.text(tomlook::reminders::WindowsSetting::DisabledForApplication.message().unwrap()).as_ref()));
        assert!(rendered.contains(language.text("Show the Windows notification: Access is denied.").as_ref()));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn reminder_activation_opens_meeting_details_without_ai_or_calendar_writes() {
    let (mut app, _, mut ai_commands) = assistant_fixture();
    let (sender, receiver) = mpsc::sync_channel(16);
    app.worker = Some(Worker {
        commands: sender, notices: Arc::new(Default::default()),
        activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(Default::default()), done: None,
    });
    app.calendar = Arc::new(Calendar::build(tomlook::calendar::fixture(60), Default::default(), Vec::new()).unwrap());
    let target = app.calendar.events[30].clone();
    app.assistant = true;
    app.query = "unrelated".into();
    let (opened, reminder_open) = mpsc::sync_channel(8);
    app.reminder_open = Some(reminder_open);
    opened.send(tomlook::reminders::Activation::Restore).unwrap();
    opened.send(tomlook::reminders::Activation::Meeting(target.id.clone())).unwrap();
    app.receive_reminders();
    assert_eq!(app.selected, app.calendar.event_index(&target.id));
    assert_eq!(app.anchor, target.calendar_dates().0);
    assert!(app.scroll_meeting && !app.insights_expanded && !app.assistant && app.query.is_empty());
    assert!(!app.myday.open);
    assert!(receiver.try_iter().any(|command| matches!(command, Command::Detail { id, .. } if id == target.id)));
    opened.send(tomlook::reminders::Activation::Meeting("missing-meeting".into())).unwrap();
    app.receive_reminders();
    assert_eq!(app.error.as_deref(), Some("Meeting is no longer in the current calendar."));
    app.assistant = true;
    opened.send(tomlook::reminders::Activation::MyDay).unwrap();
    app.receive_reminders();
    assert!(app.myday.open && !app.assistant, "The morning notification opens My day");
    assert!(receiver.try_iter().all(|command| matches!(command, Command::Detail { .. } | Command::Preferences(_))));
    assert!(ai_commands.try_recv().is_err());
}

fn feedback_thumbs(output: &egui::FullOutput) -> Vec<(bool, egui::Pos2)> {
    fn collect(shape: &egui::epaint::Shape, found: &mut Vec<(bool, egui::Pos2)>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, found);
                }
            }
            egui::epaint::Shape::Path(path) if path.closed && path.points.len() == 10 => {
                let rect = egui::Rect::from_points(&path.points);
                found.push((path.points[1].y < rect.center().y, rect.center()));
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut found);
    }
    found.sort_by_key(|(_, pos)| (pos.y.round() as i32, pos.x.round() as i32));
    found
}

#[test]
fn insight_feedback_thumbs_save_clear_and_collect_reasons_locally() {
    use tomlook::feedback::{PromptSource, Rating, Reason};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(32);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.preferences.language = language;
        app.calendar_owner = Some("owner".into());
        let event = tomlook::calendar::fixture(1).remove(0);
        app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        app.selected = Some(0);
        app.briefing = Some(serde_json::json!({
            "summary": "Agree on the pilot.", "talking_points": ["Start with the workflow."],
            "preparation": [], "open_questions": [], "risks": [], "gaps": [], "sources": [],
            "cs": {"summary": "Domluvit pilot.", "talking_points": ["Začít pracovním postupem."],
                "preparation": [], "open_questions": [], "risks": [], "gaps": []}
        }));
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(480.0, 1200.0))),
                events, ..Default::default()
            }, |ui| app.render_insights(ui, true));
            output.textures_delta.clear();
            output
        };
        let click = |app: &mut Tomlook, pos: egui::Pos2| {
            draw(app, vec![]);
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                }]);
            }
        };
        let text_position = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing feedback control: {label}"))
        };
        let rendered = |app: &mut Tomlook| {
            let output = draw(app, vec![]);
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            text
        };
        let output = draw(&mut app, vec![]);
        assert!(matches!(receiver.try_recv().unwrap(), Command::LoadFeedback(id)
            if id.account == "owner" && id.key == event.id && id.document == tomlook::changes::Document::Insights));
        let thumbs = feedback_thumbs(&output);
        assert_eq!(thumbs.iter().map(|(up, _)| *up).collect::<Vec<_>>(), vec![true, false, true, false],
            "Overview and talking points each offer helpful and not helpful");
        let document = app.feedback.document.clone().unwrap();
        app.receive_feedback(document, Ok(Vec::new()));
        click(&mut app, thumbs[0].1);
        let Ok(Command::SaveFeedback(entry)) = receiver.try_recv() else { panic!("Helpful saves immediately") };
        assert_eq!((entry.rating, entry.key.section.as_str(), entry.language), (Rating::Up, "summary", language));
        assert_eq!(entry.prompt_kind, tomlook::prompts::Kind::Preparation);
        assert_eq!(entry.prompt_source, PromptSource::FeedbackTime);
        assert!(entry.reasons.is_empty() && entry.comment.is_empty());
        assert_eq!(app.feedback.entries.len(), 1);
        click(&mut app, thumbs[0].1);
        assert!(matches!(receiver.try_recv().unwrap(), Command::ClearFeedback(key) if *key == entry.key),
            "Clicking the selected thumb again clears the rating");
        assert!(app.feedback.entries.is_empty());
        click(&mut app, thumbs[3].1);
        assert!(receiver.try_recv().is_err(), "Not helpful waits for optional reasons");
        assert!(rendered(&mut app).contains(language.text("What was not helpful?").as_ref()));
        let pos = text_position(&mut app, language.text("Too vague").as_ref());
        click(&mut app, pos);
        app.feedback.draft.as_mut().unwrap().comment = "  Name the trade-offs.  ".into();
        let pos = text_position(&mut app, language.text("Save").as_ref());
        click(&mut app, pos);
        let Ok(Command::SaveFeedback(entry)) = receiver.try_recv() else { panic!("Reasons save the rating") };
        assert_eq!(entry.rating, Rating::Down);
        assert_eq!(entry.key.section, "talking_points");
        assert_eq!(entry.reasons, [Reason::TooVague].into());
        assert_eq!(entry.comment, "Name the trade-offs.");
        assert!(app.feedback.draft.is_none());
        assert!(!rendered(&mut app).contains(language.text("What was not helpful?").as_ref()));
        app.briefing.as_mut().unwrap()["talking_points"] = serde_json::json!(["Re-researched talking point."]);
        app.briefing.as_mut().unwrap()["cs"]["talking_points"] = serde_json::json!(["Nově zjištěné téma."]);
        draw(&mut app, vec![]);
        let new_revision = tomlook::feedback::Output::new("owner", &event, app.briefing.as_ref().unwrap(), language,
            &Default::default()).unwrap().target(app.briefing.as_ref().unwrap(), "talking_points").unwrap();
        assert!(tomlook::feedback::current(&app.feedback.entries, &new_revision.key).is_none(),
            "A re-researched section starts unrated");
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err(), "Feedback is never sent to AI");
        let mut compact = context.run_ui(egui::RawInput::default(), |ui| app.render_insights(ui, false));
        assert!(feedback_thumbs(&compact).is_empty(), "Compact Insights only rate the whole document in the header");
        compact.textures_delta.clear();
        let mut details = context.run_ui(egui::RawInput::default(), |ui| app.details(ui, event.clone()));
        assert_eq!(feedback_thumbs(&details).len(), 2, "Meeting details rate the whole document");
        details.textures_delta.clear();
    }
}

#[test]
fn prompt_feedback_summary_and_suggestion_review_never_save_prompts() {
    use tomlook::feedback::{DOCUMENT, Output, Rating, Reason};
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, notices, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(32);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.loading = false;
        app.preferences.language = language;
        app.calendar_owner = Some("owner".into());
        app.preferences_open = true;
        app.preferences_section = PreferencesSection::Prompts;
        app.prompt_kind = tomlook::prompts::Kind::Preparation;
        app.prompt_draft = app.preferences.prompts.clone();
        let context = egui::Context::default();
        let draw = |app: &mut Tomlook, events| {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 1400.0))),
                events, ..Default::default()
            }, |ui| app.prompt_feedback(ui));
            output.textures_delta.clear();
            output
        };
        let text = |app: &mut Tomlook| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let mut text = String::new();
            for shape in &output.shapes {
                append_shape_text(&shape.shape, &mut text);
            }
            text
        };
        let click = |app: &mut Tomlook, label: &str| {
            draw(app, vec![]);
            let output = draw(app, vec![]);
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("Missing prompt feedback control: {label}"));
            for pressed in [true, false] {
                draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                }]);
            }
        };
        assert!(text(&mut app).contains(language.text("Loading feedback...").as_ref()));
        assert!(matches!(receiver.try_recv().unwrap(), Command::FeedbackSummary(account) if account == "owner"));
        assert!(receiver.try_recv().is_err(), "The summary is requested once");
        let mut event = tomlook::calendar::fixture(1).remove(0);
        event.title = "Fictional pilot review".into();
        let payload = serde_json::json!({"summary": "Agree on the pilot.", "talking_points": ["Start with the workflow."],
            "preparation": [], "open_questions": [], "risks": [], "gaps": [], "sources": []});
        let output = Output::new("owner", &event, &payload, tomlook::i18n::Language::En, &Default::default()).unwrap();
        let now = chrono::Utc::now();
        let entries = vec![
            output.entry(&output.target(&payload, "talking_points").unwrap(), Rating::Down,
                [Reason::TooVague, Reason::TooLong].into(), "Name concrete trade-offs", now).unwrap(),
            output.entry(&output.target(&payload, "summary").unwrap(), Rating::Down,
                [Reason::TooVague].into(), "", now - chrono::Duration::hours(1)).unwrap(),
            output.entry(&output.target(&payload, DOCUMENT).unwrap(), Rating::Up,
                Default::default(), "", now - chrono::Duration::hours(2)).unwrap(),
        ];
        let summary = tomlook::feedback::summarize("owner", &entries, now);
        app.receive_feedback_summary("owner".into(), Ok(Arc::new(summary.clone())));
        let rendered = text(&mut app);
        for expected in [
            language.text("Feedback on AI output").into_owned(),
            format!("{}: 1", language.text("Helpful")),
            format!("{}: 2", language.text("Not helpful")),
            format!("{} · 2", language.text("Too vague")),
            format!("{} · 1", language.text("Talking points")),
            "Name concrete trade-offs".into(),
            "Fictional pilot review".into(),
            language.text("Suggest prompt improvement").into_owned(),
        ] {
            assert!(rendered.contains(&expected), "Missing {expected}: {rendered}");
        }
        assert!(receiver.try_recv().is_err());
        click(&mut app, language.text("Suggest prompt improvement").as_ref());
        let Ok(ai::Command::SuggestPrompt(request)) = ai_commands.try_recv() else { panic!("Suggestion must use the AI executor") };
        assert_eq!(request.kind, tomlook::prompts::Kind::Preparation);
        assert_eq!(request.current, app.prompt_draft.preparation);
        assert_eq!(request.feedback.down, 2);
        assert!(request.prompt().unwrap().len() <= tomlook::feedback::SUGGESTION_INPUT_LIMIT);
        assert!(text(&mut app).contains(language.text("Preparing a prompt suggestion...").as_ref()));
        let revised = format!("{} Name concrete trade-offs.", tomlook::prompts::Kind::Preparation.default_text());
        let raw = serde_json::json!({"prompt": revised, "rationale": ["Adds concrete trade-offs"]}).to_string();
        let suggestion = tomlook::feedback::parse_suggestion(&request, &raw).unwrap();
        notices.send(ai::Notice::PromptSuggestion { id: request.id, result: Ok(suggestion.clone()) }).unwrap();
        app.receive();
        let rendered = text(&mut app);
        for expected in [
            language.text("Suggested prompt (draft)").into_owned(),
            "Adds concrete trade-offs".into(),
            "+ Name concrete trade-offs.".into(),
            language.text("Use draft").into_owned(),
        ] {
            assert!(rendered.contains(&expected), "Missing {expected}: {rendered}");
        }
        click(&mut app, language.text("Use draft").as_ref());
        assert_eq!(app.prompt_draft.preparation, revised);
        assert_eq!(app.preferences.prompts, tomlook::prompts::Prompts::default(), "Use draft never saves");
        assert!(receiver.try_recv().is_err(), "No preferences were written");
        click(&mut app, language.text("Suggest prompt improvement").as_ref());
        let Ok(ai::Command::SuggestPrompt(request)) = ai_commands.try_recv() else { panic!("Retry must be explicit") };
        notices.send(ai::Notice::PromptSuggestion { id: request.id,
            result: Err("Daily AI request limit reached; no prompt suggestion was requested.".into()) }).unwrap();
        app.receive();
        assert!(text(&mut app).contains(language.text("Daily AI request limit reached; no prompt suggestion was requested.").as_ref()));
        app.preview = true;
        draw(&mut app, vec![]);
        assert!(matches!(receiver.try_recv().unwrap(), Command::FeedbackSummary(account) if account == tomlook::feedback::PREVIEW_ACCOUNT));
        app.receive_feedback_summary(tomlook::feedback::PREVIEW_ACCOUNT.into(), Ok(Arc::new(tomlook::feedback::Summary {
            account: tomlook::feedback::PREVIEW_ACCOUNT.into(), ..summary.clone()
        })));
        assert!(text(&mut app).contains(language.text("Offline demo never sends AI requests.").as_ref()));
        click(&mut app, language.text("Suggest prompt improvement").as_ref());
        assert!(ai_commands.try_recv().is_err(), "The offline demo never requests a suggestion");
        app.prompt_kind = tomlook::prompts::Kind::Quick;
        assert!(text(&mut app).contains(language.text(
            "Feedback is collected for the Before the meeting and After the meeting prompts.").as_ref()));
    }
}
fn task_fixture(language: tomlook::i18n::Language) -> (
    Tomlook,
    mpsc::Receiver<Command>,
    tokio::sync::mpsc::Receiver<ai::Command>,
    Arc<ai::Notices>,
) {
    let (mut app, notices, ai_commands) = assistant_fixture();
    let (sender, receiver) = mpsc::sync_channel(64);
    app.worker = Some(Worker {
        commands: sender, notices: Arc::new(Default::default()),
        activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
        stop: Arc::new(Default::default()), done: None,
    });
    app.preferences.language = language;
    app.calendar_owner = Some("owner@example.com".into());
    app.loading = false;
    (app, receiver, ai_commands, notices)
}

fn ended_meeting() -> Event {
    let now = Local::now().fixed_offset();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.id = "ended-meeting".into();
    event.title = "Pilot planning".into();
    event.start = now - chrono::Duration::hours(3);
    event.end = now - chrono::Duration::hours(2);
    event.is_all_day = false;
    event.organizer = "Taylor Quinn <taylor@contoso.example>".into();
    event.attendees = vec![
        "Owner <owner@example.com>".into(),
        "Alex Rivers <alex@contoso.example>".into(),
        "Riley Chen <riley@northstar.example>".into(),
    ];
    event
}

fn notes_payload() -> serde_json::Value {
    serde_json::json!({
        "tomlook_recap": true, "summary": "Pilot agreed.",
        "preparation": ["My task: send the comparison to Alex", "My task: confirm the pilot owner"],
        "talking_points": ["Shared deck https://example.com/deck"],
        "open_questions": ["Mention of me: Jamie asked about the budget"],
        "risks": ["Decision: two-week pilot"], "gaps": [], "sources": [],
        "cs": {"summary": "Pilot dohodnut.",
            "preparation": ["Můj úkol: poslat Alexovi porovnání", "Můj úkol: potvrdit vlastníka pilotu"],
            "talking_points": ["Sdílená prezentace https://example.com/deck"],
            "open_questions": ["Zmínka: Jamie se ptal na rozpočet"],
            "risks": ["Rozhodnutí: dvoutýdenní pilot"], "gaps": []}
    })
}

type Show<'a> = &'a dyn Fn(&mut Tomlook, &mut egui::Ui);

fn draw_with(context: &egui::Context, app: &mut Tomlook, events: Vec<egui::Event>, show: Show) -> egui::FullOutput {
    let mut output = context.run_ui(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 2400.0))),
        events, ..Default::default()
    }, |ui| show(app, ui));
    output.textures_delta.clear();
    output
}

fn output_text(output: &egui::FullOutput) -> String {
    let mut text = String::new();
    for shape in &output.shapes {
        append_shape_text(&shape.shape, &mut text);
    }
    text
}

fn click_text(context: &egui::Context, app: &mut Tomlook, show: Show, label: &str) -> egui::FullOutput {
    draw_with(context, app, vec![], show);
    let output = draw_with(context, app, vec![], show);
    let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
            Some(text.pos + text.galley.size() * 0.5),
        _ => None,
    }).unwrap_or_else(|| panic!("Missing control: {label}\n{}", output_text(&output)));
    let mut last = None;
    for pressed in [true, false] {
        last = Some(draw_with(context, app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        }], show));
    }
    last.unwrap()
}

fn copied(output: &egui::FullOutput) -> Vec<String> {
    output.platform_output.commands.iter().filter_map(|command| match command {
        egui::OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    }).collect()
}

#[test]
fn tasks_panel_orders_by_urgency_and_completes_edits_deletes_undoes_and_adds_locally() {
    use tomlook::tasks::New;
    use tomlook::worker::TaskCommand;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        let account = "owner@example.com";
        let now = chrono::Utc::now();
        let today = Local::now().date_naive();
        let event = ended_meeting();
        app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        let mut tasks = Vec::new();
        for (id, title, due) in [
            (1, "Undated fixture", None),
            (2, "Upcoming fixture", Some(today + chrono::Duration::days(5))),
            (3, "Today fixture", Some(today)),
            (4, "Overdue fixture", Some(today - chrono::Duration::days(2))),
        ] {
            let mut task = New::manual(title, due, language).unwrap().task(account, now).unwrap();
            task.id = id;
            tasks.push(task);
        }
        let mut done = New::manual("Done fixture", None, language).unwrap().task(account, now).unwrap().completed(true, now);
        done.id = 5;
        tasks.push(done);
        let mut imported = tomlook::tasks::from_notes(account, &event, &notes_payload(), language, now).unwrap().remove(0);
        imported.id = 6;
        tasks.push(imported.clone());
        let mut foreign = New::manual("Foreign fixture", None, language).unwrap().task("other@example.com", now).unwrap();
        foreign.id = 7;
        tasks.push(foreign);
        app.activity = Arc::new(tomlook::worker::Activity {
            task_account: Some(account.into()), tasks: Arc::new(tasks), ..Default::default()
        });
        assert_eq!(app.open_task_count(), 5);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.tasks_panel(ui);
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for group in ["Overdue", "Due today", "Upcoming", "No due date"] {
            assert!(text.contains(language.text(group).as_ref()), "{group}: {text}");
        }
        if language == tomlook::i18n::Language::Cs {
            assert!(text.contains("Po termínu") && text.contains("Bez termínu") && text.contains("Přidáno ručně"), "{text}");
        }
        assert!(text.contains(&format!("{} (1)", language.text("Done in the last 30 days"))));
        assert!(text.contains(&imported.title) && text.contains("Pilot planning ·"), "Imported task links its meeting");
        assert!(!text.contains("Foreign fixture"), "Other accounts' tasks are hidden");
        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at("Overdue fixture") < at("Today fixture") && at("Today fixture") < at("Upcoming fixture")
            && at("Upcoming fixture") < at("Undated fixture"));
        click_text(&context, &mut app, show, "Today fixture");
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::SetDone { account, id: 3, done: true })
            if account == "owner@example.com"));
        click_text(&context, &mut app, show, language.text("Delete").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::Delete { id: 4, .. })));
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Task deleted").as_ref()));
        click_text(&context, &mut app, show, language.text("Undo").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::Restore { task, .. }) if task.id == 4));
        click_text(&context, &mut app, show, language.text("Edit").as_ref());
        app.tasks.editing.as_mut().unwrap().title = "Renamed fixture".into();
        click_text(&context, &mut app, show, language.text("Save").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::Edit { id: 4, title, due, .. })
            if title == "Renamed fixture" && due == Some(today - chrono::Duration::days(2))));
        assert!(app.tasks.editing.is_none());
        app.tasks.quick = "Write the summary".into();
        click_text(&context, &mut app, show, language.text("Add").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::Add { account, task })
            if account == "owner@example.com" && task.title == "Write the summary"
                && task.source == tomlook::tasks::Source::Manual && task.meeting.is_none()));
        assert!(app.tasks.quick.is_empty());
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err(), "Tasks never use AI");
    }
}

#[test]
fn tasks_header_badge_palette_and_escape_open_and_close_the_local_list() {
    use tomlook::tasks::New;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands, _) = task_fixture(language);
        let now = chrono::Utc::now();
        let tasks = (1..=3).map(|id| {
            let mut task = New::manual(&format!("Fixture {id}"), None, language).unwrap().task("owner@example.com", now).unwrap();
            task.id = id;
            if id == 3 { task.completed(true, now) } else { task }
        }).collect();
        app.activity = Arc::new(tomlook::worker::Activity {
            task_account: Some("owner@example.com".into()), tasks: Arc::new(tasks), ..Default::default()
        });
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1400.0, 200.0))),
            ..Default::default()
        }, |ui| app.header(ui));
        output.textures_delta.clear();
        let badge = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == "2" => Some(text.pos),
            _ => None,
        });
        assert!(badge.is_some_and(|pos| pos.x > 1200.0), "Two open tasks are counted in the header");
        let matches = command_matches_in(if language == tomlook::i18n::Language::Cs { "úkol" } else { "task" }, language);
        assert!(matches.contains(&PaletteCommand::Tasks) && matches.contains(&PaletteCommand::AddTask));
        app.run_palette_command(PaletteCommand::AddTask, &context);
        assert!(app.tasks_open && app.tasks.focus_quick);
        let mut output = context.run_ui(egui::RawInput {
            events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            ..Default::default()
        }, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.tasks_open, "Escape closes the task list");
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn ended_meeting_details_show_its_tasks_and_a_saved_local_follow_up_with_copy_open_and_discard() {
    use tomlook::worker::TaskCommand;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        let account = "owner@example.com";
        let event = ended_meeting();
        let payload = notes_payload();
        app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        app.selected = Some(0);
        app.briefing = Some(payload.clone());
        let now = chrono::Utc::now();
        let mut tasks = tomlook::tasks::from_notes(account, &event, &payload, language, now).unwrap();
        for (index, task) in tasks.iter_mut().enumerate() {
            task.id = index as i64 + 1;
        }
        let request = tomlook::followup::Request::new(1, account, &event, &payload, language,
            &[account.to_owned()], None).unwrap();
        let saved = tomlook::followup::Saved::new(&request, tomlook::followup::Draft {
            subject: "Follow-up: Pilot planning".into(), body: "Hi all,\n\nthanks for today.".into(),
        }, now);
        app.activity = Arc::new(tomlook::worker::Activity {
            task_account: Some(account.into()), tasks: Arc::new(tasks.clone()),
            followups: Arc::new([(saved.key.clone(), saved.clone())].into()), ..Default::default()
        });
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| {
            let event = app.calendar.events[0].clone();
            app.details(ui, event);
        };
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(receiver.try_iter().all(|command| matches!(command, Command::LoadFeedback(_))));
        for expected in [
            language.text("In my task list").into_owned(), tasks[0].title.clone(), tasks[1].title.clone(),
            language.text("Follow-up e-mail").into_owned(), "Follow-up: Pilot planning".into(),
            language.text("Recipients (3 of 3)").into_owned(),
            "Taylor Quinn <taylor@contoso.example>".into(), "Riley Chen <riley@northstar.example>".into(),
            language.text("Copy").into_owned(), language.text("Open in Outlook").into_owned(),
            language.text("Regenerate").into_owned(), language.text("Discard").into_owned(),
        ] {
            assert!(text.contains(&expected), "Missing {expected}: {text}");
        }
        assert!(!text.contains("<owner@example.com>"), "The signed-in user is never a recipient");
        click_text(&context, &mut app, show, &tasks[1].title);
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::SetDone { id: 2, done: true, .. })));
        let output = click_text(&context, &mut app, show, language.text("Copy").as_ref());
        assert_eq!(copied(&output), vec![saved.clipboard()]);
        assert!(saved.clipboard().starts_with(if language == tomlook::i18n::Language::Cs { "Předmět: " } else { "Subject: " }));
        click_text(&context, &mut app, show, language.text("Open in Outlook").as_ref());
        let link = super::followup::OPENED_LINKS.with(|links| links.borrow().last().cloned()).unwrap();
        assert!(link.starts_with(
            "mailto:taylor@contoso.example,alex@contoso.example,riley@northstar.example?subject=Follow-up%3A%20Pilot%20planning&body=Hi%20all%2C%0D%0A%0D%0Athanks"),
            "{link}");
        click_text(&context, &mut app, show, &format!("Alex Rivers <alex@contoso.example> · {}", language.text("External")));
        let Ok(Command::SaveFollowUp(updated)) = receiver.try_recv() else { panic!("Recipient choice is saved") };
        assert_eq!(updated.selected(), vec!["taylor@contoso.example", "riley@northstar.example"]);
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Recipients (2 of 3)").as_ref()));
        click_text(&context, &mut app, show, language.text("Discard").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::DiscardFollowUp { account, key }
            if account == "owner@example.com" && key == saved.key));
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(text.contains(language.text("Prepare follow-up e-mail").as_ref()) && !text.contains("Follow-up: Pilot planning"),
            "A discarded draft does not reappear while the deletion is saved");
        assert!(receiver.try_iter().all(|command| matches!(command, Command::LoadFeedback(_))));
        while let Ok(command) = ai_commands.try_recv() {
            assert!(!matches!(command, ai::Command::FollowUp(_)), "Opening details never drafts an e-mail");
        }
    }
}

#[test]
fn preparing_a_follow_up_sends_one_explicit_request_and_saves_only_validated_text() {
    let language = tomlook::i18n::Language::En;
    let (mut app, receiver, mut ai_commands, notices) = task_fixture(language);
    let account = "owner@example.com";
    let event = ended_meeting();
    app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
    app.selected = Some(0);
    app.briefing = Some(notes_payload());
    app.activity = Arc::new(tomlook::worker::Activity { task_account: Some(account.into()), ..Default::default() });
    let context = egui::Context::default();
    let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| {
        let event = app.calendar.events[0].clone();
        app.details(ui, event);
    };
    let follow_ups = |commands: &mut tokio::sync::mpsc::Receiver<ai::Command>| {
        let mut found = Vec::new();
        while let Ok(command) = commands.try_recv() {
            if let ai::Command::FollowUp(request) = command { found.push(request); }
        }
        found
    };
    draw_with(&context, &mut app, vec![], show);
    assert!(follow_ups(&mut ai_commands).is_empty(), "Follow-up e-mails are never automatic");
    assert!(receiver.try_iter().all(|command| matches!(command, Command::LoadFeedback(_))));
    click_text(&context, &mut app, show, "Prepare follow-up e-mail");
    let mut requests = follow_ups(&mut ai_commands);
    assert_eq!(requests.len(), 1);
    let request = requests.remove(0);
    assert_eq!(request.key, tomlook::recaps::key(account, &event));
    assert_eq!(request.language, language);
    assert_eq!(request.recipients.iter().map(|recipient| recipient.email.as_str()).collect::<Vec<_>>(),
        vec!["taylor@contoso.example", "alex@contoso.example", "riley@northstar.example"]);
    assert_eq!(request.notes.my_tasks.len(), 2);
    assert!(request.prompt().unwrap().len() <= tomlook::followup::INPUT_LIMIT);
    assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains("Preparing the follow-up e-mail..."));
    notices.send(ai::Notice::FollowUp { id: request.id + 100, result: Ok(tomlook::followup::Draft {
        subject: "Stale".into(), body: "Stale".into() }) }).unwrap();
    app.receive();
    assert!(app.followup.editing.is_none(), "Unknown results are ignored");
    notices.send(ai::Notice::FollowUp { id: request.id, result: Ok(tomlook::followup::Draft {
        subject: "Follow-up: Pilot planning".into(), body: "Hi all,\n\n- Two-week pilot [to confirm]".into() }) }).unwrap();
    app.receive();
    let Ok(Command::SaveFollowUp(saved)) = receiver.try_recv() else { panic!("The validated draft is saved locally") };
    assert_eq!((saved.subject.as_str(), saved.key.as_str()), ("Follow-up: Pilot planning", request.key.as_str()));
    assert_eq!(app.followup.editing.as_ref(), Some(saved.as_ref()));
    let text = output_text(&draw_with(&context, &mut app, vec![], show));
    assert!(text.contains("Follow-up: Pilot planning") && text.contains("- Two-week pilot [to confirm]"));
    click_text(&context, &mut app, show, "Regenerate");
    let request = follow_ups(&mut ai_commands).remove(0);
    notices.send(ai::Notice::FollowUp { id: request.id,
        result: Err("Daily AI request limit reached; no follow-up e-mail was requested.".into()) }).unwrap();
    app.receive();
    let text = output_text(&draw_with(&context, &mut app, vec![], show));
    assert!(text.contains("Daily AI request limit reached; no follow-up e-mail was requested."));
    assert!(text.contains("Follow-up: Pilot planning"), "A failed regeneration keeps the previous text");
    assert!(receiver.try_iter().all(|command| matches!(command, Command::LoadFeedback(_))));
    app.preview = true;
    app.followup = Default::default();
    app.activity = Arc::new(tomlook::worker::Activity { task_account: Some(tomlook::feedback::PREVIEW_ACCOUNT.into()), ..Default::default() });
    click_text(&context, &mut app, show, "Prepare follow-up e-mail");
    assert!(follow_ups(&mut ai_commands).is_empty(), "The offline demo never requests AI");
}

#[test]
fn ask_task_and_email_drafts_add_linked_local_tasks_and_open_only_validated_mail_links() {
    use tomlook::worker::TaskCommand;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        let event = ended_meeting();
        app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), vec![]).unwrap());
        app.assistant = true;
        app.assistant_context = Some(event.clone());
        app.assistant_revision = Some(event.fingerprint());
        let task = Proposal { kind: Kind::Task, target: "Local pilot checklist".into(),
            title: "Prepare the pilot checklist".into(), body: "Three success criteria".into() };
        let email = Proposal { kind: Kind::Email, target: "Alex <alex@contoso.example>".into(),
            title: "Pilot next steps".into(), body: "Hi Alex,\nthe next steps.".into() };
        let mut answer = fixture_answer("Two local drafts");
        answer.proposals = Arc::new(vec![task.clone(), email.clone()]);
        app.answer = Some(answer);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.assistant_panel(ui);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for expected in [language.text("Task note"), language.text("Email draft"), language.text("Add to tasks"),
            language.text("Open in Outlook"), language.text("Drafts stay local until you use them; nothing is sent.")] {
            assert!(text.contains(expected.as_ref()), "Missing {expected}: {text}");
        }
        click_text(&context, &mut app, show, language.text("Add to tasks").as_ref());
        let Ok(Command::Task(TaskCommand::Add { account, task: added })) = receiver.try_recv() else { panic!("Task draft was not added") };
        assert_eq!(account, "owner@example.com");
        assert_eq!((added.title.as_str(), added.source), ("Prepare the pilot checklist", tomlook::tasks::Source::AskDraft));
        assert_eq!(added.meeting.as_ref().unwrap().event_id, event.id, "The Ask meeting context links the task");
        assert!(added.note.contains("Three success criteria") && added.note.contains("Local pilot checklist"));
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Added to tasks").as_ref()));
        click_text(&context, &mut app, show, language.text("Open in Outlook").as_ref());
        let link = super::followup::OPENED_LINKS.with(|links| links.borrow().last().cloned()).unwrap();
        assert_eq!(link, "mailto:alex@contoso.example?subject=Pilot%20next%20steps&body=Hi%20Alex%2C%0D%0Athe%20next%20steps.");
        let output = click_text(&context, &mut app, show, language.text("Copy").as_ref());
        assert_eq!(copied(&output), vec![tomlook::followup::clipboard_text("Pilot next steps", "Hi Alex,\nthe next steps.", language)]);
        assert!(receiver.try_recv().is_err());
        assert!(ai_commands.try_recv().is_err(), "Local drafts never send AI requests");
    }
}
fn myday_day(number: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, number).unwrap()
}

fn myday_at(date: NaiveDate, hour: u32, minute: u32) -> chrono::DateTime<Local> {
    use chrono::TimeZone;
    Local.from_local_datetime(&date.and_hms_opt(hour, minute, 0).unwrap()).earliest().unwrap()
}

fn myday_meeting(id: &str, title: &str, start: chrono::DateTime<Local>, minutes: i64, status: &str) -> Event {
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.id = id.into();
    event.title = title.into();
    event.status = status.into();
    event.start = start.fixed_offset();
    event.end = event.start + chrono::Duration::minutes(minutes);
    event.is_all_day = false;
    event
}

/// Wednesday 11 November 2026, 09:00: a finished stand-up, two overlapping meetings without
/// Insights (one on Teams), a tentative chat, yesterday's notes with tasks and a mention, a
/// missing-materials recommendation and one meeting tomorrow.
fn myday_fixture(language: tomlook::i18n::Language) -> (
    Tomlook,
    mpsc::Receiver<Command>,
    tokio::sync::mpsc::Receiver<ai::Command>,
    String,
) {
    use tomlook::recommendations::{InboxEntry, Kind as RecommendationKind, Recommendation, Suggestion};
    let (mut app, receiver, ai_commands, _) = task_fixture(language);
    let account = "owner@example.com";
    let wednesday = myday_day(11);
    let now = myday_at(wednesday, 9, 0);
    app.myday.now = Some(now);
    let mut review = myday_meeting("review", "Customer review", myday_at(wednesday, 10, 0), 60, "accepted");
    review.extra.insert("onlineMeeting".into(), serde_json::json!({"joinUrl": "https://teams.microsoft.com/l/meetup-join/fixture"}));
    let pilot = myday_meeting("pilot", "Pilot planning", myday_at(myday_day(10), 14, 0), 60, "accepted");
    let events = vec![
        myday_meeting("standup", "Morning standup", myday_at(wednesday, 8, 30), 15, "accepted"),
        review.clone(),
        myday_meeting("architecture", "Architecture sync", myday_at(wednesday, 10, 30), 60, "organizer"),
        myday_meeting("optional", "Optional chat", myday_at(wednesday, 13, 0), 30, "tentativelyAccepted"),
        pilot.clone(),
        myday_meeting("planning", "Thursday planning", myday_at(myday_day(12), 9, 0), 30, "accepted"),
    ];
    app.calendar = Arc::new(Calendar::build(events, [wednesday, myday_day(12)].into(), vec![]).unwrap());
    let created = now.to_utc();
    let manual = |id: i64, title: &str, due: NaiveDate| {
        let mut task = tomlook::tasks::New::manual(title, Some(due), language).unwrap().task(account, created).unwrap();
        task.id = id;
        task
    };
    let mut tasks = vec![manual(1, "Overdue fixture", myday_day(10)), manual(2, "Today fixture", wednesday)];
    for (index, mut task) in tomlook::tasks::from_notes(account, &pilot, &notes_payload(), language, created)
        .unwrap().into_iter().enumerate() {
        task.id = 3 + index as i64;
        tasks.push(task);
    }
    let recommendation = Recommendation::from_insights(&review, account, Suggestion {
        kind: RecommendationKind::MissingMaterials,
        reason: "No agenda was shared.".into(),
        reason_cs: "Nebyl sdílen program.".into(),
        category: None,
        minutes: None,
    }, now.fixed_offset()).unwrap();
    let id = recommendation.id.clone();
    let recaps = tomlook::recaps::Ledger { records: [(tomlook::recaps::key(account, &pilot), tomlook::recaps::Record {
        account: account.into(), event_id: pilot.id.clone(), checking: false, next_check: now.fixed_offset(),
        message: String::new(), failures: 0, payload: Some(notes_payload()),
    })].into() };
    app.activity = Arc::new(tomlook::worker::Activity {
        enabled: true,
        queue: {
            let mut queue = tomlook::scheduler::Queue::default();
            queue.jobs.insert("standup".into(), activity_fixture_job("standup", tomlook::scheduler::State::Completed));
            queue
        },
        task_account: Some(account.into()),
        tasks: Arc::new(tasks),
        recommendations: Arc::new([(id.clone(), recommendation)].into()),
        recommendation_view: Arc::new(vec![InboxEntry { id: id.clone(), conflicts: vec![] }]),
        recaps: Arc::new(recaps),
        ..Default::default()
    });
    app.myday.open = true;
    (app, receiver, ai_commands, id)
}

#[test]
fn my_day_panel_lists_today_and_reuses_existing_actions_in_both_languages() {
    use tomlook::worker::TaskCommand;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let czech = language == tomlook::i18n::Language::Cs;
        let (mut app, receiver, mut ai_commands, recommendation) = myday_fixture(language);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.myday_panel(ui);
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for expected in [
            if czech { "3 schůzky · 1 h 45 min · 08:30–11:30 · 1 nezávazná" } else { "3 meetings · 1 h 45 min · 08:30–11:30 · 1 tentative" },
            if czech { "za 1 h" } else { "in 1 h" },
            if czech { "Jamie se ptal na rozpočet" } else { "Jamie asked about the budget" },
            if czech { "Poslat Alexovi porovnání" } else { "Send the comparison to Alex" },
            if czech { "1 schůzka · první v 09:00" } else { "1 meeting · first at 09:00" },
            "Customer review", "Architecture sync", "Optional chat", "Overdue fixture", "Today fixture", "Teams",
            "09:00–10:00", "11:30–18:00",
        ] {
            assert!(text.contains(expected), "{language:?}: missing {expected}\n{text}");
        }
        for label in [
            "Next meeting", "Today's agenda", "Conflicts", "Not prepared", "Missing materials", "Tasks",
            "Mentions of me", "Focus time", "Waiting for approval", "Tomorrow", "Insights ready",
            "Connect AI and WorkIQ to research these meetings.", "Overdue", "Due today", "Open in Messages",
        ] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}");
        }
        assert!(text.contains(&format!("{}: 10:30 – 11:00", language.text("Overlap"))), "Conflicts use local times");
        assert!(text.contains(&format!("{} · Pilot planning", language.text("From meeting notes"))), "Tasks from yesterday's notes");
        click_text(&context, &mut app, show, language.text("Research now").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::PrepareNow(id) if id == "review" || id == "architecture"));
        app.workiq_ready = true;
        click_text(&context, &mut app, show, language.text("Research all").as_ref());
        let requested = receiver.try_iter().filter_map(|command| match command {
            Command::PrepareNow(id) => Some(id),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(requested, ["architecture", "review"], "Later meetings first, so the earliest is dispatched first");
        app.activity = Arc::new(tomlook::worker::Activity { paused: true, ..(*app.activity).clone() });
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Automatic Insights are paused.").as_ref()));
        click_text(&context, &mut app, show, "Overdue fixture");
        assert!(matches!(receiver.try_recv().unwrap(), Command::Task(TaskCommand::SetDone { account, id: 1, done: true })
            if account == "owner@example.com"));
        click_text(&context, &mut app, show, language.text("Open").as_ref());
        assert_eq!(app.selected, app.calendar.event_index("review"));
        assert_eq!(app.anchor, myday_day(11));
        assert!(receiver.try_iter().any(|command| matches!(command, Command::Detail { id, .. } if id == "review")));
        click_text(&context, &mut app, show, "Pilot planning");
        assert_eq!(app.selected, app.calendar.event_index("pilot"), "A mention opens its meeting");
        click_text(&context, &mut app, show, language.text("Open in Messages").as_ref());
        assert!(app.messages_open && app.recommendations_tab && !app.myday.open);
        assert_eq!(app.recommendation_selected.as_deref(), Some(recommendation.as_str()));
        app.myday.open = true;
        click_text(&context, &mut app, show, language.text("Open tasks").as_ref());
        assert!(app.tasks_open && !app.myday.open);
        while let Ok(command) = ai_commands.try_recv() {
            assert!(matches!(command, ai::Command::RefreshCalendarAvailability), "My day never asks AI");
        }
        let unsynced = Calendar::build(app.calendar.events.clone(), Default::default(), vec![]).unwrap();
        app.calendar = Arc::new(unsynced);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for label in [
            "Today's calendar is not synced yet; the overview may be incomplete.",
            "Today's calendar is not synced yet, so free time is not shown.",
            "Tomorrow's calendar is not synced yet.",
        ] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}");
        }
        assert!(!text.contains("11:30–18:00"), "Free time is never claimed for an unsynced day");
    }
}

#[test]
fn my_day_opens_from_palette_ctrl_d_and_notification_and_escape_closes_it() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands, _) = myday_fixture(language);
        app.myday.open = false;
        let matches = command_matches_in(if language == tomlook::i18n::Language::Cs { "můj den" } else { "my day" }, language);
        assert_eq!(matches.first(), Some(&PaletteCommand::MyDay));
        let context = egui::Context::default();
        app.run_palette_command(PaletteCommand::MyDay, &context);
        assert!(app.myday.open);
        let mut text = String::new();
        for _ in 0..2 {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                ..Default::default()
            }, |ui| app.myday_window(ui.ctx()));
            output.textures_delta.clear();
            text = output_text(&output);
        }
        assert!(text.contains(language.text("My day").as_ref()) && text.contains("Customer review"), "{text}");
        let key = |key, modifiers| egui::RawInput {
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        let mut output = context.run_ui(key(Key::ArrowDown, Modifiers::NONE), |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(app.selected.is_none(), "Calendar navigation pauses while My day is open");
        let mut output = context.run_ui(key(Key::Escape, Modifiers::NONE), |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.myday.open, "Escape closes My day");
        for open in [true, false] {
            let mut output = context.run_ui(key(Key::D, Modifiers::CTRL), |ui| app.shortcuts(ui.ctx()));
            output.textures_delta.clear();
            assert_eq!(app.myday.open, open, "Ctrl+D toggles My day");
        }
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.header(ui));
        output.textures_delta.clear();
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn demo_my_day_renders_fictional_agenda_tasks_and_mentions_without_research() {
    use chrono::TimeZone;
    for (date, language) in [
        (11, tomlook::i18n::Language::En),
        (15, tomlook::i18n::Language::Cs),
        (16, tomlook::i18n::Language::En),
    ] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        app.preview = true;
        app.calendar_owner = None;
        let now = Local.with_ymd_and_hms(2026, 11, date, 7, 30, 0).earliest().unwrap();
        app.myday.now = Some(now);
        let coverage = (0..42).map(|offset| tomlook::calendar::monday(now.date_naive()) + chrono::Duration::days(offset)).collect();
        app.calendar = Arc::new(Calendar::build(tomlook::showcase::preview_events(now, 200), coverage, vec![]).unwrap());
        let ended = app.calendar.events.iter().find(|event| event.title == tomlook::showcase::ENDED_MEETING).unwrap().clone();
        let account = tomlook::feedback::PREVIEW_ACCOUNT;
        let created = now.to_utc();
        let mut tasks = tomlook::showcase::sample_tasks(account, &ended, language, created).unwrap();
        tasks.extend(tomlook::tasks::from_notes(account, &ended, &tomlook::showcase::notes(&ended).unwrap(), language, created).unwrap());
        for (index, task) in tasks.iter_mut().enumerate() {
            task.id = index as i64 + 1;
        }
        app.activity = Arc::new(tomlook::worker::Activity {
            task_account: Some(account.into()), tasks: Arc::new(tasks), ..Default::default()
        });
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.myday_panel(ui);
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        let mention = if language == tomlook::i18n::Language::Cs {
            "Jamie vás požádal, abyste na revizi pilotu znovu prošli kompromisy"
        } else {
            "Jamie asked you to walk through the trade-offs again at the pilot review"
        };
        for expected in ["Northstar / Escalation check-in", mention, tomlook::showcase::ENDED_MEETING] {
            assert!(text.contains(expected), "{date}: missing {expected}\n{text}");
        }
        for label in ["Offline demo - fictional calendar", "Research is not available in the offline demo.", "Not prepared", "Insights ready"] {
            assert!(text.contains(language.text(label).as_ref()), "{date}: missing {label}");
        }
        assert!(!text.contains(language.text("No conflicts today.").as_ref()), "The demo shows a conflict");
        draw_with(&context, &mut app, vec![], show);
        assert!(receiver.try_iter().all(|command| !matches!(command, Command::PrepareNow(_))));
        assert!(ai_commands.try_recv().is_err(), "The demo never requests AI");
    }
}

#[test]
fn morning_overview_settings_save_preferences_show_now_and_report_status() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, _, _) = task_fixture(language);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| {
            let status = app.reminder_status.clone();
            app.morning_preferences(ui, &status);
        };
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for label in ["Morning overview", "Show my day in the morning", "Time", "Also on weekends", "Show now", "Send test"] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}");
        }
        for time in ["07:00", "07:30", "08:00", "09:30", "10:00"] {
            assert!(text.contains(time), "{time}");
        }
        assert!(!text.contains("10:30"), "Only 07:00–10:00 is offered");
        click_text(&context, &mut app, show, "08:30");
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences)
            if preferences.morning_overview_time == 510 && preferences.morning_overview && !preferences.morning_overview_weekends));
        click_text(&context, &mut app, show, language.text("Also on weekends").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if preferences.morning_overview_weekends));
        click_text(&context, &mut app, show, language.text("Show my day in the morning").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if !preferences.morning_overview));
        app.reminder_status.last_morning = Some(chrono::Utc::now());
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(text.contains(language.text("Morning overview is turned off.").as_ref()));
        assert!(text.contains(if language == tomlook::i18n::Language::Cs { "Poslední ranní přehled zobrazen: " } else { "Last morning overview shown: " }), "{text}");
        click_text(&context, &mut app, show, language.text("Show now").as_ref());
        assert!(app.myday.open, "Show now opens the My day panel");
        assert!(receiver.try_iter().all(|command| !matches!(command, Command::PrepareNow(_))));
    }
}

/// Week of Monday 9 November 2026, seen on Wednesday at noon: an overlapping customer review
/// (Teams block only) and architecture sync I organize (empty description), a stand-up with an
/// agenda and an uncategorized planning meeting without a cached description.
fn weekly_fixture(language: tomlook::i18n::Language) -> (
    Tomlook,
    mpsc::Receiver<Command>,
    tokio::sync::mpsc::Receiver<ai::Command>,
    Arc<ai::Notices>,
) {
    let (mut app, receiver, ai_commands, notices) = task_fixture(language);
    let account = "owner@example.com";
    let monday = myday_day(9);
    app.weekly.now = Some(myday_at(myday_day(11), 12, 0));
    let body = |content: &str| serde_json::json!({"contentType": "html", "content": content});
    let teams = "<div>________________</div><div>Microsoft Teams meeting</div><div>Join on your computer, mobile app or room device</div><div>Meeting ID: 312 449 845 811</div><div>Passcode: 7tBHx2</div>";
    let mut review = myday_meeting("review", "Customer review", myday_at(monday, 10, 0), 60, "accepted");
    review.organizer = "Taylor Quinn <taylor@contoso.example>".into();
    review.attendees = vec![format!("Owner <{account}>"), "Riley Chen <riley@northstar.example>".into()];
    review.extra.insert("categories".into(), serde_json::json!(["Customer"]));
    review.extra.insert("body".into(), body(teams));
    let mut architecture = myday_meeting("architecture", "Architecture sync", myday_at(monday, 10, 30), 60, "organizer");
    architecture.organizer = format!("Owner <{account}>");
    architecture.attendees = vec!["Alex Rivers <alex@example.com>".into()];
    architecture.extra.insert("isOrganizer".into(), serde_json::json!(true));
    architecture.extra.insert("categories".into(), serde_json::json!(["Customer"]));
    architecture.extra.insert("body".into(), body(""));
    let mut standup = myday_meeting("standup", "Morning standup", myday_at(monday, 8, 30), 15, "accepted");
    standup.extra.insert("categories".into(), serde_json::json!(["Office"]));
    standup.extra.insert("body".into(), body("<p>Agenda: blockers, release status and today's priorities</p>"));
    let mut planning = myday_meeting("planning", "Planning", myday_at(monday + chrono::Duration::days(1), 9, 0), 60, "accepted");
    planning.extra.insert("categories".into(), serde_json::json!([]));
    planning.category.clear();
    let coverage = (0..7).map(|offset| monday + chrono::Duration::days(offset)).collect();
    app.calendar = Arc::new(Calendar::build(vec![review, architecture, standup, planning], coverage, vec![]).unwrap());
    let created = myday_at(monday + chrono::Duration::days(1), 9, 0).to_utc();
    let mut task = tomlook::tasks::New::manual("Send the deck", None, language).unwrap().task(account, created).unwrap();
    task.id = 1;
    app.activity = Arc::new(tomlook::worker::Activity {
        task_account: Some(account.into()),
        tasks: Arc::new(vec![task]),
        ..Default::default()
    });
    app.weekly.open = true;
    (app, receiver, ai_commands, notices)
}

#[test]
fn weekly_review_panel_shows_metrics_opens_meetings_copies_and_summarizes_on_request() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let czech = language == tomlook::i18n::Language::Cs;
        let (mut app, receiver, mut ai_commands, notices) = weekly_fixture(language);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.weekly_panel(ui);
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for label in [
            "Time by category", "Conflicts", "Meetings without an agenda", "You organize", "Others organize",
            "Meeting load", "People and customers", "Preparation and follow-through", "AI summary",
            "Summarize the week with AI", "Meetings attended", "Meeting time", "Focus time", "Longest free block",
            "Uncategorized", "External companies", "Copy summary", "This week so far, including meetings still scheduled.",
            "No earlier weeks with enough synced days to compare with.",
        ] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}\n{text}");
        }
        for expected in [
            language.range(myday_day(9), myday_day(15)),
            if czech { "2,8 h".into() } else { "2.8 h".into() },
            "33 min".into(),
            "Customer".into(), "Riley Chen".into(), "contoso.example".into(), "northstar.example".into(),
            "Customer review".into(), "Architecture sync".into(),
            if czech { "Neověřeno (popis přibude při příští aktualizaci kalendáře): 1".into() } else { "Not checked (description arrives with the next calendar refresh): 1".into() },
            if czech { "Čas v překryvu: 30 min".into() } else { "Overlapping time: 30 min".into() },
        ] {
            assert!(text.contains(&expected), "{language:?}: missing {expected}\n{text}");
        }
        assert!(!text.contains("example.com"), "My own domain is not an external company and addresses are only shown on hover");
        assert!(ai_commands.try_recv().is_err(), "The weekly review never asks AI by itself");
        let output = click_text(&context, &mut app, show, language.text("Copy summary").as_ref());
        let copied = copied(&output);
        assert_eq!(copied.len(), 1);
        assert!(copied[0].starts_with(&format!("**{} · ", language.text("Weekly review"))), "{}", copied[0]);
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Summary copied.").as_ref()));
        click_text(&context, &mut app, show, "Customer review");
        assert_eq!(app.selected, app.calendar.event_index("review"), "A meeting opens its details");
        assert!(app.weekly.open, "The review stays open");
        click_text(&context, &mut app, show, "<");
        assert_eq!(app.weekly.week, Some(myday_day(2)));
        let earlier = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(earlier.contains(language.text("No meetings this week.").as_ref()) && earlier.contains(language.text("Not synced").as_ref()));
        click_text(&context, &mut app, show, ">");
        assert_eq!(app.weekly.week, None, "Back to the current week; there is no future week");
        click_text(&context, &mut app, show, language.text("Summarize the week with AI").as_ref());
        let mut requests = Vec::new();
        while let Ok(command) = ai_commands.try_recv() {
            if let ai::Command::WeeklySummary(request) = command { requests.push(request); }
        }
        assert_eq!(requests.len(), 1, "One explicit request");
        let request = requests.remove(0);
        assert_eq!((request.week, request.language, request.account.as_str()), (myday_day(9), language, "owner@example.com"));
        assert!(request.prompt().unwrap().len() <= tomlook::weekly::INPUT_LIMIT);
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Summarizing the week...").as_ref()));
        notices.send(ai::Notice::WeeklySummary { id: request.id + 100, result: Ok(tomlook::weekly::AiSummary {
            highlights: vec!["Stale".into()], ..Default::default() }) }).unwrap();
        app.receive();
        assert!(app.weekly.running.is_some(), "Unknown results are ignored");
        notices.send(ai::Notice::WeeklySummary { id: request.id, result: Ok(tomlook::weekly::AiSummary {
            highlights: vec!["Customer meetings took most of the week.".into()],
            suggestions: vec!["Add an agenda to the architecture sync.".into()],
            ..Default::default() }) }).unwrap();
        app.receive();
        let saved = receiver.try_iter().find_map(|command| match command {
            Command::SaveWeeklySummary(saved) => Some(saved),
            _ => None,
        }).expect("The validated summary is saved locally");
        assert_eq!((saved.week, saved.account.as_str()), (myday_day(9), "owner@example.com"));
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for expected in ["Customer meetings took most of the week.", "Add an agenda to the architecture sync."] {
            assert!(text.contains(expected), "{expected}\n{text}");
        }
        assert!(text.contains(language.text("Highlights").as_ref()) && text.contains(language.text("Regenerate").as_ref()));
        click_text(&context, &mut app, show, language.text("Regenerate").as_ref());
        let request = loop {
            if let ai::Command::WeeklySummary(request) = ai_commands.try_recv().unwrap() { break request; }
        };
        notices.send(ai::Notice::WeeklySummary { id: request.id,
            result: Err("Daily AI request limit reached; no weekly summary was requested.".into()) }).unwrap();
        app.receive();
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(text.contains(language.text("Daily AI request limit reached; no weekly summary was requested.").as_ref()));
        assert!(text.contains("Customer meetings took most of the week."), "A failed regeneration keeps the saved summary");
        let partial = Calendar::build(app.calendar.events.clone(), [myday_day(9), myday_day(10)].into(), vec![]).unwrap();
        app.calendar = Arc::new(partial);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(text.contains(language.text("The numbers may be incomplete. Days that are not synced are never counted as free.").as_ref()));
        assert!(text.contains(language.text("Fewer than 4 of 5 workdays are synced, so this week is not used for comparisons or the weekly notification.").as_ref()));
        assert!(text.contains(&format!("{}: {}", language.text("Not synced"), language.date(myday_day(11), false).trim())));
    }
}

#[test]
fn weekly_review_opens_from_palette_my_day_week_header_and_notification_and_escape_closes_it() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands, _) = weekly_fixture(language);
        app.weekly.open = false;
        let matches = command_matches_in(if language == tomlook::i18n::Language::Cs { "týdenní" } else { "weekly review" }, language);
        assert_eq!(matches.first(), Some(&PaletteCommand::WeeklyReview));
        let context = egui::Context::default();
        app.weekly.week = Some(myday_day(2));
        app.run_palette_command(PaletteCommand::WeeklyReview, &context);
        assert!(app.weekly.open && app.weekly.week.is_none(), "The palette opens the current week");
        let mut text = String::new();
        for _ in 0..2 {
            let mut output = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                ..Default::default()
            }, |ui| app.weekly_window(ui.ctx()));
            output.textures_delta.clear();
            text = output_text(&output);
        }
        assert!(text.contains(language.text("Weekly review").as_ref()) && text.contains("Customer review"), "{text}");
        let key = |key, modifiers| egui::RawInput {
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        let mut output = context.run_ui(key(Key::ArrowDown, Modifiers::NONE), |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(app.selected.is_none(), "Calendar navigation pauses while the review is open");
        let mut output = context.run_ui(key(Key::Escape, Modifiers::NONE), |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.weekly.open, "Escape closes the weekly review");
        app.myday.now = app.weekly.now;
        app.myday.open = true;
        let myday: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.myday_panel(ui);
        click_text(&context, &mut app, myday, language.text("Weekly review").as_ref());
        assert!(app.weekly.open && !app.myday.open && app.weekly.week.is_none(), "My day links to this week's review");
        app.weekly.open = false;
        let header: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.header(ui);
        app.preferences.view = View::Week;
        app.anchor = myday_day(4);
        click_text(&context, &mut app, header, language.text("Weekly review").as_ref());
        assert!(app.weekly.open);
        assert_eq!(app.weekly.week, Some(myday_day(2)), "The header reviews the displayed week");
        app.weekly.open = false;
        app.preferences.view = View::WorkWeek;
        app.anchor = myday_day(25);
        click_text(&context, &mut app, header, language.text("Weekly review").as_ref());
        assert!(!app.weekly.open, "Future weeks cannot be reviewed");
        app.preferences.view = View::Month;
        assert!(!output_text(&draw_with(&context, &mut app, vec![], header)).contains(language.text("Weekly review").as_ref()),
            "Only week views offer the button");
        let (opened, reminder_open) = mpsc::sync_channel(8);
        app.reminder_open = Some(reminder_open);
        app.assistant = true;
        opened.send(tomlook::reminders::Activation::WeeklyReview(myday_day(2))).unwrap();
        app.receive_reminders();
        assert!(app.weekly.open && !app.assistant);
        assert_eq!(app.weekly.week, Some(myday_day(2)), "The notification opens the week it reviewed");
        while let Ok(command) = ai_commands.try_recv() {
            assert!(matches!(command, ai::Command::RefreshCalendarAvailability | ai::Command::Companies(_)),
                "Opening the weekly review never asks AI");
        }
    }
}

#[test]
fn weekly_review_settings_save_preferences_show_now_and_report_status() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, _, _) = weekly_fixture(language);
        app.weekly.open = false;
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| {
            let status = app.reminder_status.clone();
            app.weekly_preferences(ui, &status);
        };
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for label in ["Weekly review", "Show a weekly review notification", "Day", "Friday (this week)",
            "Monday (previous week)", "Time", "Show now", "Send test"] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}");
        }
        for time in ["08:00", "12:00", "15:00", "17:00"] {
            assert!(text.contains(time), "{time}");
        }
        click_text(&context, &mut app, show, "16:00");
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences)
            if preferences.weekly_review_time == 960 && preferences.weekly_review && preferences.weekly_review_day == 5));
        click_text(&context, &mut app, show, language.text("Monday (previous week)").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if preferences.weekly_review_day == 1));
        click_text(&context, &mut app, show, language.text("Show now").as_ref());
        assert!(app.weekly.open);
        assert_eq!(app.weekly.week, Some(myday_day(2)), "A Monday notification reviews the previous week");
        click_text(&context, &mut app, show, language.text("Show a weekly review notification").as_ref());
        assert!(matches!(receiver.try_recv().unwrap(), Command::Preferences(preferences) if !preferences.weekly_review));
        app.reminder_status.last_weekly = Some(chrono::Utc::now());
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        assert!(text.contains(language.text("Weekly review is turned off.").as_ref()));
        assert!(text.contains(if language == tomlook::i18n::Language::Cs { "Poslední týdenní přehled zobrazen: " } else { "Last weekly review shown: " }), "{text}");
        assert!(receiver.try_iter().all(|command| !matches!(command, Command::PrepareNow(_))));
    }
}

#[test]
fn demo_weekly_review_shows_fictional_numbers_and_never_requests_ai() {
    use chrono::TimeZone;
    for (date, language) in [(11, tomlook::i18n::Language::En), (15, tomlook::i18n::Language::Cs)] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        app.preview = true;
        app.calendar_owner = None;
        let now = Local.with_ymd_and_hms(2026, 11, date, 16, 0, 0).earliest().unwrap();
        app.weekly.now = Some(now);
        let monday = tomlook::calendar::monday(now.date_naive());
        let coverage = (-14..42).map(|offset| monday + chrono::Duration::days(offset)).collect();
        app.calendar = Arc::new(Calendar::build(tomlook::showcase::preview_events(now, 200), coverage, vec![]).unwrap());
        app.activity = Arc::new(tomlook::worker::Activity {
            task_account: Some(tomlook::feedback::PREVIEW_ACCOUNT.into()), ..Default::default()
        });
        app.weekly.open = true;
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.weekly_panel(ui);
        draw_with(&context, &mut app, vec![], show);
        let text = output_text(&draw_with(&context, &mut app, vec![], show));
        for expected in ["northstar.invalid", "Coffee & roadmap", "Weekly wrap-up", "Alex Rivers"] {
            assert!(text.contains(expected), "{date}: missing {expected}\n{text}");
        }
        for label in ["Offline demo - fictional calendar", "Offline demo never sends AI requests.", "Summarize the week with AI"] {
            assert!(text.contains(language.text(label).as_ref()), "{date}: missing {label}");
        }
        assert!(!text.contains(language.text("No earlier weeks with enough synced days to compare with.").as_ref()),
            "The demo's earlier weeks give a comparison");
        assert!(!text.contains(language.text("No conflicts this week.").as_ref()), "The demo week has a conflict");
        click_text(&context, &mut app, show, language.text("Summarize the week with AI").as_ref());
        assert!(app.weekly.running.is_none());
        assert!(ai_commands.try_recv().is_err(), "The demo never requests AI");
        assert!(receiver.try_iter().all(|command| !matches!(command, Command::SaveWeeklySummary(_))));
    }
}
/// Draws until a window that changed size has been laid out, re-centered and painted.
fn settled(context: &egui::Context, app: &mut Tomlook, show: Show) -> String {
    for _ in 0..5 {
        draw_with(context, app, vec![], show);
    }
    output_text(&draw_with(context, app, vec![], show))
}
fn drain_ai(commands: &mut tokio::sync::mpsc::Receiver<ai::Command>, seen: &mut Vec<ai::Command>) {
    while let Ok(command) = commands.try_recv() {
        seen.push(command);
    }
}

fn synthetic_availability(request: &tomlook::availability::ScheduleRequest, day: NaiveDate) -> tomlook::availability::Availability {
    use tomlook::availability::{Availability, Item, Schedule, Status};
    let at = |minutes| tomlook::event_creation::local_time(day, minutes).unwrap().with_timezone(&chrono::Utc);
    Availability {
        start: request.start,
        end: request.end,
        interval: request.interval,
        schedules: vec![
            Schedule::from_items(request, "owner@example.com".into(), Vec::new()),
            Schedule::from_items(request, "jamie@example.com".into(), vec![Item {
                status: Status::Busy, start: at(600), end: at(630),
                subject: Some("Synthetic review".into()), location: Some("Room 7".into()),
            }]),
            Schedule {
                email: "alex@example.com".into(),
                view: vec![Status::Unknown; request.cells()],
                items: Vec::new(),
                error: Some("Availability is not shared with you".into()),
            },
        ],
    }
}

#[test]
fn event_editor_invites_attendees_shows_their_availability_and_needs_the_explicit_invitation_click() {
    let show: Show = &|app, ui| app.event_creation_dialog(ui.ctx());
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, notices, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        let mut past = tomlook::calendar::fixture(1).remove(0);
        past.organizer = "Alex Rivers <alex@example.com>".into();
        past.attendees = vec!["Jamie Brooks <jamie@example.com>".into(), "Owner <owner@example.com>".into()];
        app.calendar = Arc::new(Calendar::build(vec![past], Default::default(), Vec::new()).unwrap());
        app.calendar_owner = Some("owner@example.com".into());
        app.workiq_ready = true;
        app.preferences.language = language;
        let day = Local::now().date_naive() + chrono::Duration::days(1);
        app.open_event_draft(day, 600, 660);
        app.event_draft.as_mut().unwrap().title = "Synthetic planning".into();
        app.event_draft.as_mut().unwrap().attendee_input = "ja".into();
        let context = egui::Context::default();
        let mut seen = Vec::new();
        let text = settled(&context, &mut app, show);
        assert!(text.contains("Jamie Brooks · jamie@example.com"), "Cached people are suggested at once\n{text}");
        assert!(!text.contains("owner@example.com"), "Your own mailbox is never suggested");
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().any(|command| matches!(command, ai::Command::InvitationCapability(account) if account == "owner@example.com")));
        app.event_draft.as_mut().unwrap().directory.changed = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        draw_with(&context, &mut app, vec![], show);
        drain_ai(&mut ai_commands, &mut seen);
        let (id, own) = seen.iter().rev().find_map(|command| match command {
            ai::Command::People { id, query, own } if query == "ja" => Some((*id, own.clone())),
            _ => None,
        }).expect("Debounced read-only directory search");
        assert!(own.contains(&"owner@example.com".to_owned()));
        notices.send(ai::Notice::People { id, result: Ok(vec![tomlook::attendees::Person {
            email: "jade@example.com".into(), name: "Jade Doe".into(), meetings: 0, score: 0.0,
        }]) }).unwrap();
        notices.send(ai::Notice::InvitationCapability { account: "owner@example.com".into(), result: Ok(()) }).unwrap();
        app.receive();
        let text = settled(&context, &mut app, show);
        assert!(text.contains("Jade Doe · jade@example.com"), "Directory results join the suggestions\n{text}");
        click_text(&context, &mut app, show, "Jamie Brooks · jamie@example.com");
        app.event_draft.as_mut().unwrap().attendee_input = "Alex Rivers <ALEX@example.com>; owner@example.com".into();
        let output = click_text(&context, &mut app, show, language.text("Add").as_ref());
        let draft = app.event_draft.as_ref().unwrap();
        assert_eq!(draft.attendees.iter().map(|attendee| attendee.email.as_str()).collect::<Vec<_>>(), ["jamie@example.com", "alex@example.com"]);
        assert!(draft.attendee_input.is_empty());
        assert!(output_text(&output).contains(language.text("You are the organizer and are not invited").as_ref()));
        assert!(receiver.try_recv().is_err(), "Adding attendees never writes");
        draw_with(&context, &mut app, vec![], show);
        app.event_draft.as_mut().unwrap().availability.changed = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        draw_with(&context, &mut app, vec![], show);
        drain_ai(&mut ai_commands, &mut seen);
        let (id, request) = seen.iter().rev().find_map(|command| match command {
            ai::Command::Availability { id, request } => Some((*id, request.clone())),
            _ => None,
        }).expect("Debounced read-only availability");
        assert_eq!(request.schedules, ["owner@example.com", "jamie@example.com", "alex@example.com"]);
        assert_eq!(request.interval, 15);
        assert_eq!(request.start, tomlook::event_creation::local_time(day, 0).unwrap().with_timezone(&chrono::Utc));
        notices.send(ai::Notice::Availability { id, result: Ok(Arc::new(synthetic_availability(&request, day))) }).unwrap();
        app.receive();
        let text = settled(&context, &mut app, show);
        let summary = match language {
            tomlook::i18n::Language::En => "0 of 2 available · Jamie Brooks busy · Alex Rivers unknown",
            tomlook::i18n::Language::Cs => "Volno nemá nikdo z 2 · Jamie Brooks: obsazeno · Alex Rivers: neznámé",
        };
        assert!(text.contains(summary), "{text}");
        for label in ["You", "Busy", "Tentative", "Out of office", "Working elsewhere", "Free", "Unknown", "Suggest times"] {
            assert!(text.contains(language.text(label).as_ref()), "Missing {label}\n{text}");
        }
        let notice = tomlook::attendees::invitation_notice(&app.event_draft.as_ref().unwrap().attendees, language);
        assert!(text.contains(&notice), "The invitation consequence is explicit\n{text}");
        assert!(!text.contains(&format!("\n{}\n", language.text("Create event"))));
        assert!(receiver.try_recv().is_err());
        click_text(&context, &mut app, show, language.text("Suggest times").as_ref());
        drain_ai(&mut ai_commands, &mut seen);
        let (id, times) = seen.iter().rev().find_map(|command| match command {
            ai::Command::MeetingTimes { id, request } => Some((*id, request.clone())),
            _ => None,
        }).expect("Read-only meeting time suggestions");
        assert_eq!(times.minutes, 60);
        assert_eq!(times.attendees.len(), 2);
        let start = tomlook::event_creation::local_time(day, 840).unwrap().with_timezone(&chrono::Utc);
        notices.send(ai::Notice::MeetingTimes { id, result: Ok(tomlook::availability::MeetingTimes {
            suggestions: vec![tomlook::availability::Suggestion {
                start, end: start + chrono::Duration::hours(1), confidence: Some(100.0),
                organizer: tomlook::availability::Status::Free,
                attendees: vec![("jamie@example.com".into(), tomlook::availability::Status::Free),
                    ("alex@example.com".into(), tomlook::availability::Status::Free)],
            }],
            empty_reason: None,
            reason_code: None,
        }) }).unwrap();
        app.receive();
        let label = format!("{} 14:00–15:00 · {} · 100 %", language.date(day, false), language.text("2/2 available"));
        settled(&context, &mut app, show);
        click_text(&context, &mut app, show, &label);
        let draft = app.event_draft.as_ref().unwrap();
        assert_eq!((draft.day, draft.start, draft.end), (day, 840, 900));
        settled(&context, &mut app, show);
        click_text(&context, &mut app, show, language.text("Create and send invitations").as_ref());
        let Command::CreateEvents { account, draft, parent } = receiver.try_recv().unwrap() else { panic!("Exact invitation approval") };
        assert_eq!(account, "owner@example.com");
        assert!(parent.is_none());
        assert_eq!(draft.attendees.len(), 2);
        assert!(draft.attendees.iter().all(|attendee| attendee.role == tomlook::attendees::Role::Required));
        assert_eq!(draft.start, tomlook::event_creation::local_time(day, 840).unwrap());
        assert!(app.event_draft.is_none());
        assert!(receiver.try_recv().is_err(), "One explicit click is one approval");
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().all(|command| matches!(command, ai::Command::People { .. } | ai::Command::Availability { .. }
            | ai::Command::MeetingTimes { .. } | ai::Command::InvitationCapability(_))), "Only read-only lookups, never a model call");
    }
}

#[test]
fn unsupported_invitations_disable_attendees_with_a_reason_and_failed_availability_still_allows_review() {
    let show: Show = &|app, ui| app.event_creation_dialog(ui.ctx());
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        let (sender, receiver) = mpsc::sync_channel(16);
        app.worker = Some(Worker {
            commands: sender, notices: Arc::new(Default::default()),
            activity: Arc::new(Mutex::new(Arc::new(Default::default()))),
            stop: Arc::new(Default::default()), done: None,
        });
        app.calendar_owner = Some("owner@example.com".into());
        app.workiq_ready = true;
        app.preferences.language = language;
        let reason = tomlook::workiq::connection::calendar_write::INVITATIONS_UNSUPPORTED;
        app.invitation_capability = Some(("owner@example.com".into(), Some(Err(reason.into()))));
        let day = Local::now().date_naive() + chrono::Duration::days(1);
        app.open_event_draft(day, 600, 660);
        let draft = app.event_draft.as_mut().unwrap();
        draft.title = "Synthetic planning".into();
        draft.attendees = tomlook::attendees::parse_input("jamie@example.com").unwrap();
        draft.availability.key = format!("{day}|owner@example.com,jamie@example.com");
        draft.availability.result = Some(Err("Availability lookups are not permitted for this account".into()));
        let context = egui::Context::default();
        let text = settled(&context, &mut app, show);
        assert!(text.contains(&format!("{}: {}", language.text("Invitations are unavailable"), language.text(reason))), "{text}");
        assert!(text.contains(&format!("{}: {}", language.text("Availability could not be checked"),
            language.text("Availability lookups are not permitted for this account"))), "{text}");
        assert!(text.contains(language.text("You can still create the event; availability is not verified.").as_ref()));
        click_text(&context, &mut app, show, language.text("Create and send invitations").as_ref());
        assert!(receiver.try_recv().is_err(), "No invitation while the provider rejects attendees");
        click_text(&context, &mut app, show, "×");
        assert!(app.event_draft.as_ref().unwrap().attendees.is_empty(), "Attendees can still be removed");
        click_text(&context, &mut app, show, language.text("Create event").as_ref());
        let Command::CreateEvents { draft, .. } = receiver.try_recv().unwrap() else { panic!("Plain appointment") };
        assert!(draft.attendees.is_empty());
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().all(|command| !matches!(command, ai::Command::People { .. })));
    }
}

#[test]
fn messages_name_invitees_and_warn_that_uncertain_invitations_may_already_be_out() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, _) = assistant_fixture();
        app.preferences.language = language;
        app.calendar_owner = Some("owner".into());
        let draft = tomlook::event_creation::Draft {
            title: "Synthetic invitation".into(), description: String::new(),
            start: chrono::DateTime::parse_from_rfc3339("2026-11-03T10:00:00+01:00").unwrap(),
            end: chrono::DateTime::parse_from_rfc3339("2026-11-03T11:00:00+01:00").unwrap(),
            category: None, private: false, teams: false, travel_minutes: None, language,
            attendees: tomlook::attendees::parse_input("Alex Rivers <alex@example.com>").unwrap(),
        };
        draft.validate().unwrap();
        let mut record = tomlook::event_creation::Record::new("owner".into(), draft).unwrap();
        record.state = tomlook::event_creation::State::Review;
        record.message = record.review_message(tomlook::workiq::connection::calendar_write::UNCERTAIN);
        app.activity = Arc::new(tomlook::worker::Activity {
            event_creations: Arc::new([(record.id.clone(), record)].into()),
            ..Default::default()
        });
        app.messages_open = true;
        let context = egui::Context::default();
        let show: Show = &|app, ui| app.overlays(ui.ctx());
        draw_with(&context, &mut app, vec![], show);
        let text = settled(&context, &mut app, show);
        assert!(text.contains(language.text(tomlook::event_creation::INVITATIONS_MAY_BE_SENT).as_ref()), "{text}");
        assert!(text.contains(&format!("{}: Alex Rivers", language.text("Invited"))), "{text}");
    }
}

#[test]
fn invite_demo_scene_shows_fictional_attendees_and_availability_without_workiq() {
    let (mut app, _, mut ai_commands) = assistant_fixture();
    app.preview = true;
    app.calendar = Arc::new(Calendar::build(tomlook::showcase::preview_events(Local::now(), 200), Default::default(), Vec::new()).unwrap());
    assert!(!app.demo_invitation_ready());
    let draft = app.event_draft.as_ref().unwrap();
    assert_eq!(draft.attendees.len(), 3);
    assert!(draft.attendees.iter().all(|attendee| attendee.email.ends_with(".invalid")));
    let context = egui::Context::default();
    let show: Show = &|app, ui| app.event_creation_dialog(ui.ctx());
    draw_with(&context, &mut app, vec![], show);
    app.event_draft.as_mut().unwrap().availability.changed = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    draw_with(&context, &mut app, vec![], show);
    assert!(app.demo_invitation_ready());
    let text = settled(&context, &mut app, show);
    for expected in ["Alex Rivers", "Jamie Brooks", "Dana Novak", "Create and send invitations", "Dana Novak unknown"] {
        assert!(text.contains(expected), "Missing {expected}\n{text}");
    }
    assert!(ai_commands.try_recv().is_err(), "The demo never calls WorkIQ or AI");
}
/// All free for everyone except Morgan, whose calendar is not shared.
fn group_availability(request: &tomlook::availability::ScheduleRequest) -> tomlook::availability::Availability {
    use tomlook::availability::{Availability, Schedule, Status};
    Availability {
        start: request.start,
        end: request.end,
        interval: request.interval,
        schedules: request.schedules.iter().map(|email| if email == "morgan@example.com" {
            Schedule { email: email.clone(), view: vec![Status::Unknown; request.cells()], items: Vec::new(),
                error: Some("Availability is not shared with you".into()) }
        } else {
            Schedule::from_items(request, email.clone(), Vec::new())
        }).collect(),
    }
}

fn find_time_fixture(language: tomlook::i18n::Language) -> (Tomlook, mpsc::Receiver<Command>, tokio::sync::mpsc::Receiver<ai::Command>, Arc<ai::Notices>) {
    let (mut app, receiver, ai_commands, notices) = task_fixture(language);
    app.workiq_ready = true;
    let mut people = tomlook::attendees::parse_input("Alex Rivers <alex@example.com>; Jamie Brooks <jamie@example.com>; Morgan Lee <morgan@example.com>").unwrap();
    people[2].role = tomlook::attendees::Role::Optional;
    app.open_find_time("Synthetic sync", people, 30);
    (app, receiver, ai_commands, notices)
}

/// Answers the debounced read-only free/busy request and returns the candidates' labels.
fn answer_group_availability(context: &egui::Context, app: &mut Tomlook, ai_commands: &mut tokio::sync::mpsc::Receiver<ai::Command>,
    notices: &ai::Notices, show: Show) -> Vec<String> {
    draw_with(context, app, vec![], show);
    app.find_time.as_mut().unwrap().availability.changed = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    draw_with(context, app, vec![], show);
    let mut seen = Vec::new();
    drain_ai(ai_commands, &mut seen);
    let (id, request) = seen.iter().rev().find_map(|command| match command {
        ai::Command::GroupAvailability { id, request } => Some((*id, request.clone())),
        _ => None,
    }).expect("Debounced read-only free/busy over the searched days");
    assert_eq!(request.schedules, ["owner@example.com", "alex@example.com", "jamie@example.com", "morgan@example.com"]);
    assert!(request.end - request.start >= chrono::Duration::days(5), "Five workdays are searched");
    notices.send(ai::Notice::GroupAvailability { id, result: Ok(Arc::new(group_availability(&request))) }).unwrap();
    app.receive();
    draw_with(context, app, vec![], show);
    let language = app.preferences.language;
    let Some(Ok(candidates)) = app.find_time.as_ref().unwrap().candidates() else { panic!("Candidates are computed locally") };
    candidates.list.iter().map(|candidate| tomlook::scheduling::slot_label(candidate.start, candidate.end, language)).collect()
}

#[test]
fn find_a_time_shows_candidates_and_holds_selected_times_only_after_the_explicit_blocker_approval() {
    let show: Show = &|app, ui| app.find_time_dialog(ui.ctx());
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, notices) = find_time_fixture(language);
        let context = egui::Context::default();
        let labels = answer_group_availability(&context, &mut app, &mut ai_commands, &notices, show);
        assert!(labels.len() >= 3 && labels.len() <= 10, "{labels:?}");
        let text = settled(&context, &mut app, show);
        for label in ["Find a time", "Candidates", "Topic", "Duration", "Weekdays only", "Hold with BLOCKERs",
            "Propose in a Teams group chat", "Propose by e-mail", "Create meeting"] {
            assert!(text.contains(language.text(label).as_ref()), "Missing {label}\n{text}");
        }
        let reason = match language {
            tomlook::i18n::Language::En => "All required free · Morgan Lee unknown (optional)",
            tomlook::i18n::Language::Cs => "Povinní účastníci mají volno · Morgan Lee: neznámé (nepovinně)",
        };
        assert!(text.contains(reason), "Unknown is never free\n{text}");
        click_text(&context, &mut app, show, &labels[0]);
        click_text(&context, &mut app, show, &labels[1]);
        assert_eq!(app.find_time.as_ref().unwrap().selected.len(), 2);
        click_text(&context, &mut app, show, language.text("Hold with BLOCKERs").as_ref());
        let text = settled(&context, &mut app, show);
        assert!(text.contains(&tomlook::scheduling::hold_notice(2, language)), "The consequence is explicit\n{text}");
        assert!(text.contains(&format!("BLOCKER: Synthetic sync · {}", labels[0])), "{text}");
        assert!(receiver.try_recv().is_err(), "Choosing an action writes nothing");
        click_text(&context, &mut app, show, &tomlook::scheduling::hold_button(2, language));
        let Command::HoldTimes { account, proposal, private, category } = receiver.try_recv().unwrap() else { panic!("Exact BLOCKER approval") };
        assert_eq!(account, "owner@example.com");
        assert_eq!((proposal.topic.as_str(), proposal.candidates.len(), proposal.attendees.len()), ("Synthetic sync", 2, 3));
        assert!(!private && category.is_none(), "Normal sensitivity and no category unless chosen");
        assert!(receiver.try_recv().is_err(), "One click is one approval");
        assert!(app.find_time.as_ref().unwrap().proposal.is_some(), "The proposal's times are now fixed");
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().all(|command| matches!(command, ai::Command::GroupAvailability { .. } | ai::Command::People { .. })),
            "Only read-only lookups reach the AI engine; the write goes through the storage worker");
    }
}

#[test]
fn teams_proposal_shows_exact_members_topic_and_message_and_sends_only_after_create_chat_and_send() {
    let show: Show = &|app, ui| app.find_time_dialog(ui.ctx());
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, notices) = find_time_fixture(language);
        let context = egui::Context::default();
        let labels = answer_group_availability(&context, &mut app, &mut ai_commands, &notices, show);
        click_text(&context, &mut app, show, &labels[0]);
        click_text(&context, &mut app, show, language.text("Propose in a Teams group chat").as_ref());
        draw_with(&context, &mut app, vec![], show);
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        let (id, emails) = seen.iter().rev().find_map(|command| match command {
            ai::Command::ChatCheck { id, account, emails } if account == "owner@example.com" => Some((*id, emails.clone())),
            _ => None,
        }).expect("Read-only Teams check");
        assert_eq!(emails, ["alex@example.com", "jamie@example.com", "morgan@example.com"]);
        notices.send(ai::Notice::ChatCheck { id, result: Ok(tomlook::scheduling::ChatCheck {
            members: emails.iter().enumerate().map(|(index, email)| tomlook::scheduling::Member {
                email: email.clone(), name: email.split('@').next().unwrap().to_owned(),
                user_id: format!("{:08x}-0000-4000-8000-{:012x}", index + 1, index + 1),
            }).collect(),
            unresolved: Vec::new(),
        }) }).unwrap();
        app.receive();
        let text = settled(&context, &mut app, show);
        let notice = match language {
            tomlook::i18n::Language::En => "This creates a Teams chat and sends this message to 3 people.",
            tomlook::i18n::Language::Cs => "Tímto se vytvoří chat v Teams a zpráva se odešle 3 lidem.",
        };
        assert!(text.contains(notice), "{text}");
        assert!(text.contains(&format!("{}: Synthetic sync", language.text("Chat topic"))), "{text}");
        assert!(text.contains("alex (alex@example.com)"), "{text}");
        let message = app.find_time.as_ref().unwrap().message.clone();
        assert!(message.contains(&labels[0]) && message.contains("Synthetic sync"), "{message}");
        assert!(receiver.try_recv().is_err());
        click_text(&context, &mut app, show, language.text("Create chat and send").as_ref());
        let Command::ProposeChat { account, proposal, draft } = receiver.try_recv().unwrap() else { panic!("Exact Teams approval") };
        assert_eq!(account, "owner@example.com");
        assert_eq!(proposal.candidates.len(), 1);
        assert_eq!(draft.members.len(), 3);
        assert_eq!(draft.topic.as_deref(), Some("Synthetic sync"));
        assert_eq!(draft.message, message.trim());
        assert!(receiver.try_recv().is_err());
    }
    for (result, expected) in [
        (Ok(tomlook::scheduling::ChatCheck { members: Vec::new(), unresolved: vec![("guest@partner.example".into(),
            "Not found in your organization's directory".into())] }), "These people cannot be added to a Teams chat:"),
        (Err(tomlook::workiq::connection::chat_write::UNSUPPORTED.to_owned()), "Teams proposals are unavailable"),
    ] {
        let (mut app, receiver, mut ai_commands, notices) = find_time_fixture(tomlook::i18n::Language::En);
        let context = egui::Context::default();
        let labels = answer_group_availability(&context, &mut app, &mut ai_commands, &notices, show);
        click_text(&context, &mut app, show, &labels[0]);
        click_text(&context, &mut app, show, "Propose in a Teams group chat");
        draw_with(&context, &mut app, vec![], show);
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        let id = seen.iter().rev().find_map(|command| match command {
            ai::Command::ChatCheck { id, .. } => Some(*id),
            _ => None,
        }).unwrap();
        notices.send(ai::Notice::ChatCheck { id, result }).unwrap();
        app.receive();
        let text = settled(&context, &mut app, show);
        assert!(text.contains(expected) && text.contains("Use Propose by e-mail instead."), "{text}");
        assert!(!text.contains("Create chat and send"), "Nothing can be sent to a reduced group");
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn email_proposal_is_local_text_with_copy_and_a_mailto_link_and_never_sends() {
    let show: Show = &|app, ui| app.find_time_dialog(ui.ctx());
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, notices) = find_time_fixture(language);
        let context = egui::Context::default();
        let labels = answer_group_availability(&context, &mut app, &mut ai_commands, &notices, show);
        click_text(&context, &mut app, show, &labels[0]);
        click_text(&context, &mut app, show, language.text("Propose by e-mail").as_ref());
        let text = settled(&context, &mut app, show);
        assert!(text.contains(language.text("Tomlook never sends e-mail: copy the text or open a new message in your e-mail app.").as_ref()), "{text}");
        let find = app.find_time.as_ref().unwrap();
        assert!(find.email_body.contains(&labels[0]), "{}", find.email_body);
        click_text(&context, &mut app, show, "Morgan Lee (morgan@example.com)");
        let output = click_text(&context, &mut app, show, language.text("Copy").as_ref());
        assert!(copied(&output).iter().any(|text| text.contains("Synthetic sync")), "Copied locally");
        super::followup::OPENED_LINKS.with(|links| links.borrow_mut().clear());
        click_text(&context, &mut app, show, language.text("Open in Outlook").as_ref());
        let opened = super::followup::OPENED_LINKS.with(|links| links.borrow().clone());
        assert_eq!(opened.len(), 1);
        assert!(opened[0].starts_with("mailto:alex@example.com,jamie@example.com?subject="), "Unchecked recipients are left out: {}", opened[0]);
        assert!(receiver.try_recv().is_err(), "E-mail proposals never write or send");
    }
}

fn blocker_activity(app: &mut Tomlook) -> (Event, tomlook::scheduling::Proposal) {
    let start = (Local::now().date_naive() + chrono::Duration::days(2)).and_hms_opt(10, 0, 0).unwrap()
        .and_local_timezone(Local).earliest().unwrap().fixed_offset();
    let mut proposal = tomlook::scheduling::Proposal::new("owner@example.com", "Synthetic sync",
        tomlook::attendees::parse_input("Alex Rivers <alex@example.com>").unwrap(), 30,
        vec![tomlook::scheduling::Slot { start, end: start + chrono::Duration::minutes(30) }],
        app.preferences.language, Local::now().fixed_offset()).unwrap();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.id = "blocker-event".into();
    event.title = "BLOCKER: Synthetic sync".into();
    event.start = start;
    event.end = start + chrono::Duration::minutes(30);
    event.is_all_day = false;
    event.attendees.clear();
    event.extra.insert("isOrganizer".into(), true.into());
    proposal.blockers = vec![tomlook::scheduling::Blocker {
        start, end: event.end, title: event.title.clone(), transaction_id: "approval-blocker-0".into(),
        event_id: Some(event.id.clone()), state: tomlook::scheduling::BlockerState::Held, message: String::new(),
    }];
    app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
    app.activity = Arc::new(tomlook::worker::Activity {
        task_account: Some("owner@example.com".into()),
        proposals: Arc::new([(proposal.id.clone(), proposal.clone())].into()),
        blockers: Arc::new([(event.id.clone(), proposal.id.clone())].into()),
        ..Default::default()
    });
    (event, proposal)
}

#[test]
fn blocker_details_confirm_the_time_or_release_every_blocker_with_an_explicit_deletion_approval() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, _, _) = task_fixture(language);
        app.workiq_ready = true;
        let (event, proposal) = blocker_activity(&mut app);
        let context = egui::Context::default();
        let details: Show = &|app, ui| { let event = app.calendar.events[0].clone(); app.details(ui, event) };
        let text = settled(&context, &mut app, details);
        for label in ["Proposed time", "Confirm this time", "Release all BLOCKERs"] {
            assert!(text.contains(language.text(label).as_ref()), "Missing {label}\n{text}");
        }
        click_text(&context, &mut app, details, language.text("Release all BLOCKERs").as_ref());
        assert!(app.blocker_release.as_ref().is_some_and(|draft| draft.cancel && draft.events == [event.id.clone()]));
        let dialog: Show = &|app, ui| app.blocker_release_dialog(ui.ctx());
        let text = settled(&context, &mut app, dialog);
        assert!(text.contains(&tomlook::scheduling::release_notice(1, language)), "{text}");
        assert!(receiver.try_recv().is_err(), "Opening the confirmation deletes nothing");
        click_text(&context, &mut app, dialog, &tomlook::scheduling::release_button(1, language));
        let Command::ReleaseBlockers { account, proposal: id, events, cancel } = receiver.try_recv().unwrap() else { panic!("Exact release approval") };
        assert_eq!((account.as_str(), id.as_str(), events, cancel), ("owner@example.com", proposal.id.as_str(), vec![event.id.clone()], true));
        click_text(&context, &mut app, details, language.text("Confirm this time").as_ref());
        let draft = app.event_draft.as_ref().expect("The invitation editor opens");
        assert_eq!((draft.title.as_str(), draft.attendees.len()), ("Synthetic sync", 1));
        assert_eq!(draft.proposal, Some((proposal.id.clone(), event.id.clone())));
        let record = tomlook::event_creation::Record::new("owner@example.com".into(), tomlook::event_creation::Draft {
            title: "Synthetic sync".into(), description: String::new(), start: event.start, end: event.end, category: None,
            private: false, teams: false, travel_minutes: None, language, attendees: draft.attendees.clone(),
        }).unwrap();
        let mut created = record.clone();
        created.state = tomlook::event_creation::State::Created;
        app.event_draft = None;
        app.proposal_confirmation = Some((proposal.id.clone(), record.id.clone()));
        let mut activity = (*app.activity).clone();
        activity.event_creations = Arc::new([(record.id.clone(), created)].into());
        app.activity = Arc::new(activity);
        app.sync_proposal_confirmation();
        let Command::ScheduleProposal { proposal: scheduled, record: confirmed, .. } = receiver.try_recv().unwrap() else { panic!("Scheduled after verification") };
        assert_eq!((scheduled, confirmed), (proposal.id.clone(), record.id.clone()));
        assert!(app.blocker_release.as_ref().is_some_and(|draft| !draft.cancel), "Deleting the BLOCKERs is offered, not done");
        let text = settled(&context, &mut app, dialog);
        assert!(text.contains(language.text("Keep them").as_ref()), "{text}");
        app.messages_open = true;
        let overlays: Show = &|app, ui| app.overlays(ui.ctx());
        let text = settled(&context, &mut app, overlays);
        assert!(text.contains(language.text("Scheduling proposals").as_ref()) && text.contains("Synthetic sync"), "{text}");
    }
}

#[test]
fn meeting_menu_finds_another_time_with_its_people_except_you() {
    let (mut app, receiver, _, _) = task_fixture(tomlook::i18n::Language::En);
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.title = "Synthetic review".into();
    event.organizer = "Alex Rivers <alex@example.com>".into();
    event.attendees = vec!["Owner <owner@example.com>".into(), "Jamie Brooks <jamie@example.com>".into(), "Room 7 <room7@example.com>".into()];
    event.extra.insert("tomlook_attendee_roles".into(), serde_json::json!({"jamie@example.com": "optional", "room7@example.com": "resource"}));
    event.end = event.start + chrono::Duration::minutes(45);
    event.is_all_day = false;
    app.calendar = Arc::new(Calendar::build(vec![event], Default::default(), Vec::new()).unwrap());
    let context = egui::Context::default();
    let menu: Show = &|app, ui| app.meeting_actions(ui, 0);
    click_text(&context, &mut app, menu, "Find another time with these people");
    let find = app.find_time.as_ref().expect("Find a time opens");
    assert_eq!(find.topic, "Synthetic review");
    assert_eq!(find.search.minutes, 45);
    assert_eq!(find.attendees.iter().map(|attendee| (attendee.email.as_str(), attendee.role)).collect::<Vec<_>>(),
        [("alex@example.com", tomlook::attendees::Role::Required), ("jamie@example.com", tomlook::attendees::Role::Optional)],
        "You and rooms are left out; roles are kept");
    assert!(receiver.try_recv().is_err());
}

#[test]
fn find_time_demo_scenes_show_fictional_candidates_and_a_disabled_teams_preview_without_workiq() {
    for teams in [false, true] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        app.preview = true;
        app.loading = false;
        let now = Local::now();
        let mut events = tomlook::showcase::preview_events(now, 198);
        let blockers = tomlook::showcase::demo_blockers(now, &events, tomlook::i18n::Language::En);
        events.extend(blockers);
        app.calendar = Arc::new(Calendar::build(events, Default::default(), Vec::new()).unwrap());
        let context = egui::Context::default();
        let show: Show = &|app, ui| app.find_time_dialog(ui.ctx());
        assert!(!app.demo_find_time_ready(teams));
        draw_with(&context, &mut app, vec![], show);
        app.find_time.as_mut().unwrap().availability.changed = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        let mut ready = false;
        for _ in 0..12 {
            draw_with(&context, &mut app, vec![], show);
            ready = app.demo_find_time_ready(teams);
            if ready { break; }
        }
        assert!(ready, "The demo scene settles");
        let text = settled(&context, &mut app, show);
        assert!(text.contains("Alex Rivers") && text.contains("Kim Lambert") != teams, "{text}");
        assert_eq!(app.find_time.as_ref().unwrap().selected.len(), 2);
        if teams {
            assert!(text.contains("This creates a Teams chat and sends this message to 3 people."), "{text}");
            assert!(text.contains("The offline demo never changes a calendar or contacts anyone."), "Writes are disabled with a reason\n{text}");
        }
        assert!(ai_commands.try_recv().is_err(), "The demo never calls WorkIQ or AI");
    }
}

const CUSTOMER_ACCOUNT_ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn customer_meeting(id: &str, title: &str, start: chrono::DateTime<Local>, minutes: i64, people: &[&str]) -> Event {
    let mut event = myday_meeting(id, title, start, minutes, "accepted");
    event.organizer = "Owner <owner@example.com>".into();
    event.attendees = std::iter::once("Owner <owner@example.com>").chain(people.iter().copied()).map(Into::into).collect();
    event
}

fn customer_snapshot(retrieved: chrono::DateTime<chrono::Utc>, truncated: bool) -> tomlook::customers::MsxSnapshot {
    use tomlook::customers::*;
    let today = Local::now().date_naive();
    let current = today.with_day(1).unwrap();
    let month = |back: u32| current.checked_sub_months(chrono::Months::new(back)).unwrap();
    let mut months = (1..=6).rev().map(|back| MonthRow { month: month(back), value: 9000.0 + f64::from(6 - back) * 500.0, kind: MonthKind::Actual }).collect::<Vec<_>>();
    months.push(MonthRow { month: current, value: 4000.0, kind: MonthKind::Partial });
    months.push(MonthRow { month: current, value: 12500.0, kind: MonthKind::Prediction });
    MsxSnapshot {
        account_id: CUSTOMER_ACCOUNT_ID.into(),
        retrieved_at: retrieved,
        sample: false,
        dataverse: Dataverse::Loaded {
            opportunities: vec![OpportunityRow {
                name: "Intake pilot".into(),
                stage: Some("2 - Qualify".into()),
                close: Some(today + chrono::Duration::days(40)),
                owner: Some("Taylor Quinn".into()),
                url: Some("https://microsoftsales.crm.dynamics.com/main.aspx?etn=opportunity&id=bbbbbbbb-0000-4000-8000-000000000001&pagetype=entityrecord".into()),
            }],
            opportunities_truncated: false,
            milestones: if truncated { vec![] } else { vec![MilestoneRow {
                name: "Pilot go-live".into(),
                status: Some("On track".into()),
                due: Some(today + chrono::Duration::days(20)),
                owner: Some("Jamie Brooks".into()),
                workload: None,
                my_task: None,
                url: None,
            }] },
            milestones_truncated: truncated,
            tasks: Vec::new(),
            tasks_note: None,
            sources: vec![SourceRow { system: "Dataverse".into(), table: "opportunity".into(), rows: 1, truncated: false, retrieved_at: retrieved }],
        },
        consumption: Consumption::Available {
            months,
            unit: "USD".into(),
            dataset: "AzureBlueSubscriptionSL4".into(),
            latest_month: Some(current),
            matched_by: "TPID".into(),
            prediction_note: "Prediction from the model measure [Forecast]; not extrapolated by Tomlook.".into(),
        },
    }
}

/// A confirmed customer with past and upcoming meetings, notes, a linked task, a saved MSX read
/// (cached, MSX not signed in) and a saved communication summary.
fn customer_fixture(language: tomlook::i18n::Language) -> (
    Tomlook,
    mpsc::Receiver<Command>,
    tokio::sync::mpsc::Receiver<ai::Command>,
    Arc<ai::Notices>,
    String,
) {
    use tomlook::customers::{research, *};
    let (mut app, receiver, ai_commands, notices) = task_fixture(language);
    let account = "owner@example.com";
    let now = Local::now();
    app.customers.now = Some(now);
    // Company icons are fetched from public websites; this fixture never asks for them.
    app.companies_busy = true;
    let at = |days: i64, hour: u32| myday_at(now.date_naive() + chrono::Duration::days(days), hour, 0);
    let dana = "Dana Novak <dana@northstar.example>";
    let past = customer_meeting("past", "Northstar pilot planning", at(-3, 10), 60, &[dana]);
    let older = customer_meeting("older", "Northstar discovery", at(-20, 10), 90, &[dana, "Riley Chen <riley@eu.northstar.example>"]);
    let next = customer_meeting("next", "Northstar architecture workshop", at(2, 9), 90, &[dana]);
    let orbit = customer_meeting("orbit", "Orbit kickoff", at(1, 13), 30, &["Kim Lambert <kim@orbit.example>"]);
    app.calendar = Arc::new(Calendar::build(vec![past.clone(), older, next, orbit], Default::default(), vec![]).unwrap());
    let customer = Customer::new(account, "Northstar", &["northstar.example".into()], Some(MsxAccount {
        id: CUSTOMER_ACCOUNT_ID.into(), name: "Northstar Holdings".into(), tpid: Some("123456".into()), confidence: Some(0.82),
    }), chrono::Utc::now()).unwrap();
    let id = customer.id.clone();
    let communication = research::Communication {
        account: account.into(),
        customer: id.clone(),
        created_at: chrono::Utc::now() - chrono::Duration::days(1),
        days: 60,
        topic: Some("Northstar architecture workshop".into()),
        language,
        items: vec![
            research::Item { kind: research::Kind::Mail, date: Some(now.date_naive() - chrono::Duration::days(4)), people: vec!["Dana Novak".into()],
                subject: "Pilot scope".into(), summary: "Dana confirmed the pilot scope.".into(), why_relevant: "It is on the workshop agenda.".into(),
                url: Some("https://outlook.office.com/mail/id/AAMk1".into()), source_id: None, verified: true },
            research::Item { kind: research::Kind::Teams, date: None, people: vec![], subject: "Prototype chat".into(),
                summary: "The recording was shared.".into(), why_relevant: String::new(), url: None, source_id: None, verified: false },
        ],
        open_questions: vec!["Who owns the pilot?".into()],
        commitments: vec![research::Commitment { by: research::Side::Me, text: "Send the summary".into(), due: None }],
        risks: Vec::new(),
        gaps: Vec::new(),
        dropped_sources: 1,
        sample: false,
    };
    let book = Book {
        account: Some(account.into()),
        customers: BTreeMap::from([(id.clone(), customer)]),
        msx: BTreeMap::from([(id.clone(), customer_snapshot(chrono::Utc::now() - chrono::Duration::hours(2), false))]),
        communications: BTreeMap::from([(id.clone(), communication)]),
        ..Default::default()
    };
    let key = tomlook::recaps::key(account, &past);
    let ledger = tomlook::recaps::Ledger { records: BTreeMap::from([(key, tomlook::recaps::Record {
        account: account.into(), event_id: "past".into(), checking: false, next_check: now.fixed_offset(),
        message: "AI meeting notes ready".into(), failures: 0, payload: Some(notes_payload()),
    })]) };
    let mut task = tomlook::tasks::New::manual("Send the comparison to Dana", Some(now.date_naive() + chrono::Duration::days(1)), language).unwrap();
    task.meeting = Some(tomlook::tasks::Meeting::new(account, &past));
    let mut task = task.task(account, chrono::Utc::now()).unwrap();
    task.id = 7;
    app.activity = Arc::new(tomlook::worker::Activity {
        task_account: Some(account.into()),
        tasks: Arc::new(vec![task]),
        recaps: Arc::new(ledger),
        customers: Arc::new(book),
        ..Default::default()
    });
    app.open_customers(Some(id.clone()), None);
    (app, receiver, ai_commands, notices, id)
}

#[test]
fn customer_panel_shows_timeline_notes_tasks_msx_states_consumption_and_communication() {
    use tomlook::msx::Phase;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let czech = language == tomlook::i18n::Language::Cs;
        let (mut app, receiver, mut ai_commands, notices, id) = customer_fixture(language);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.customers_panel(ui);
        let text = settled(&context, &mut app, show);
        for label in [
            "All customers", "Meetings with this customer", "Meetings in the last 90 days", "Hours together in the last 90 days", "Next meeting",
            "Upcoming", "Earlier meetings", "Notes ready", "Notes and decisions", "Decisions", "My tasks", "Tasks from these meetings",
            "Not signed in to MSX. Sign in to read opportunities, milestones and consumption.", "Open MSX connection",
            "Open opportunities", "Active future milestones", "My task: unknown", "Azure consumption", "Completed month",
            "Current month (partial)", "Model prediction", "Consumption is technical evidence, not business intent.",
            "Recent communication", "Find related communication", "Connect GitHub Copilot and WorkIQ first.",
            "Source not among the retrieved results", "Open source", "Commitments", "Open questions", "Edit", "Remove customer",
        ] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}\n{text}");
        }
        for expected in [
            "Northstar", "northstar.example", "Northstar pilot planning", "Northstar discovery", "Northstar architecture workshop",
            if czech { "Pilot dohodnut." } else { "Pilot agreed." }, "Send the comparison to Dana", "Intake pilot", "Pilot go-live", "Pilot scope", "Who owns the pilot?",
            if czech { "Zákazník v MSX: Northstar Holdings" } else { "MSX customer: Northstar Holdings" },
            if czech { "Poslední měsíc s daty:" } else { "Latest month with data:" },
            if czech { "Zobrazuji uložená data z" } else { "Showing the saved read from" },
            if czech { "Jednotka: USD" } else { "Unit: USD" },
            if czech { "Můj závazek: Send the summary" } else { "My commitment: Send the summary" },
            if czech { "Odstraněné citované zdroje, které nebyly načtené nebo nebyly bezpečné: 1." } else { "1 cited sources were removed because they were not retrieved or not safe." },
            "TPID 123456",
        ] {
            assert!(text.contains(expected), "{language:?}: missing {expected}\n{text}");
        }
        assert!(text.contains(if czech { "2,5 h" } else { "2.5 h" }), "Hours together in the last 90 days\n{text}");
        assert!(!text.contains("Orbit kickoff"), "Other customers' meetings are not in this timeline");
        assert!(text.contains(language.text("None found in a complete read.").as_ref()), "A complete read without MSX tasks says so");
        if czech {
            assert!(!text.contains("účet") && !text.contains("Účet"), "A customer is never an 'účet'\n{text}");
        }
        assert!(ai_commands.try_recv().is_err(), "Not signed in to MSX: nothing is read and no AI is asked");
        click_text(&context, &mut app, show, language.text("Open MSX connection").as_ref());
        assert!(app.connection_panel == Some(ConnectionPanel::Msx), "The MSX dialog opens; sign-in still needs its own click");
        assert!(ai_commands.try_recv().is_err());

        app.connection_panel = None;
        notices.send(ai::Notice::Msx(msx_status(Phase::Connected, tomlook::msx::CONNECTED, None))).unwrap();
        app.receive();
        draw_with(&context, &mut app, vec![], show);
        let request = match ai_commands.try_recv() {
            Ok(ai::Command::CustomerMsx { id: request, customer, account }) => {
                assert_eq!((customer.as_str(), account.id.as_str()), (id.as_str(), CUSTOMER_ACCOUNT_ID));
                request
            }
            _ => panic!("A stale saved read is refreshed once while MSX is connected"),
        };
        draw_with(&context, &mut app, vec![], show);
        assert!(ai_commands.try_recv().is_err(), "One refresh per session");
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Loading MSX...").as_ref()));
        notices.send(ai::Notice::CustomerMsx { id: request, customer: id.clone(), result: Err("Dataverse is unreachable: timeout".into()) }).unwrap();
        app.receive();
        let text = settled(&context, &mut app, show);
        assert!(text.contains(&language.text("MSX unavailable: Dataverse is unreachable: timeout").into_owned()), "{text}");
        assert!(text.contains(language.text("A failed read is not evidence that nothing exists.").as_ref()));
        assert!(text.contains("Intake pilot"), "The saved read stays visible, labelled as saved");

        click_text(&context, &mut app, show, language.text("Refresh").as_ref());
        let request = match ai_commands.try_recv() {
            Ok(ai::Command::CustomerMsx { id: request, .. }) => request,
            _ => panic!("Refresh reads again"),
        };
        notices.send(ai::Notice::CustomerMsx { id: request, customer: id.clone(), result: Ok(customer_snapshot(chrono::Utc::now(), true)) }).unwrap();
        app.receive();
        let saved = receiver.try_iter().find_map(|command| match command {
            Command::Customer(tomlook::worker::CustomerCommand::SaveMsx { customer, snapshot, .. }) => Some((customer, snapshot)),
            _ => None,
        }).expect("The read is saved locally");
        assert_eq!(saved.0, id);
        let text = settled(&context, &mut app, show);
        for label in ["Partial MSX data:", "Milestones reached the read limit; more may exist.", "The read reached its limit; more may exist."] {
            assert!(text.contains(language.text(label).as_ref()), "{label}\n{text}");
        }
        assert!(!text.contains(language.text("No active future milestones in a complete read.").as_ref()), "A truncated read never says none");

        app.workiq_ready = true;
        click_text(&context, &mut app, show, language.text("Find related communication").as_ref());
        let research = loop {
            match ai_commands.try_recv() {
                Ok(ai::Command::CustomerResearch(request)) => break request,
                Ok(_) => continue,
                Err(_) => panic!("One explicit research request"),
            }
        };
        assert_eq!((research.customer.as_str(), research.account.as_str(), research.language), (id.as_str(), "owner@example.com", language));
        assert_eq!(research.people, ["dana@northstar.example", "riley@eu.northstar.example"]);
        assert_eq!(research.topic.as_ref().map(|topic| topic.title.as_str()), Some("Northstar architecture workshop"), "Focused on the next meeting");
        assert!(research.prompt().unwrap().len() <= tomlook::customers::research::INPUT_LIMIT);
        assert!(output_text(&draw_with(&context, &mut app, vec![], show)).contains(language.text("Searching mail, Teams and meetings...").as_ref()));
        let mut result = app.activity.customers.communications[&id].clone();
        result.items.truncate(1);
        result.dropped_sources = 0;
        result.created_at = chrono::Utc::now();
        notices.send(ai::Notice::CustomerResearch { id: research.id + 50, result: Ok(result.clone()) }).unwrap();
        app.receive();
        assert!(app.customers.research.is_some(), "Unknown results are ignored");
        notices.send(ai::Notice::CustomerResearch { id: research.id, result: Ok(result) }).unwrap();
        app.receive();
        assert!(receiver.try_iter().any(|command| matches!(command,
            Command::Customer(tomlook::worker::CustomerCommand::SaveCommunication(saved)) if saved.customer == id)));
        let text = settled(&context, &mut app, show);
        assert!(!text.contains("Prototype chat"), "The newest summary replaces the older one\n{text}");
        click_text(&context, &mut app, show, language.text("All customers").as_ref());
        let text = settled(&context, &mut app, show);
        assert!(app.customers.selected.is_none());
        assert!(text.contains(&language.text("Unconfirmed customer: orbit.example").into_owned()), "{text}");
        assert!(text.contains(language.text("Unconfirmed customers in your meetings").as_ref()));
    }
}

#[test]
fn meeting_card_confirms_customers_only_after_an_explicit_choice() {
    use tomlook::msx::Phase;
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, notices, id) = customer_fixture(language);
        app.customers.open = false;
        let context = egui::Context::default();
        let orbit = app.calendar.events[app.calendar.event_index("orbit").unwrap()].clone();
        let card = orbit.clone();
        let show: Show = &move |app: &mut Tomlook, ui: &mut egui::Ui| { app.customer_card(ui, &card); app.confirm_customer_dialog(ui.ctx()); };
        let text = settled(&context, &mut app, show);
        assert!(text.contains(&language.text("Unconfirmed customer: orbit.example").into_owned()), "{text}");
        assert!(text.contains("Kim Lambert") && text.contains(language.text("Not a customer meeting").as_ref()));
        click_text(&context, &mut app, show, language.text("Confirm customer").as_ref());
        assert!(app.customers.confirm.is_some());
        assert!(ai_commands.try_recv().is_err(), "MSX is not connected: no account search");
        let text = settled(&context, &mut app, show);
        for label in ["Customer name", "E-mail domains", "Local customer only (no MSX link)",
            "MSX is not connected; you can confirm a local customer now and link MSX later.",
            "Inferred matches are never linked without your confirmation.", "Confirm"] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}\n{text}");
        }
        assert!(receiver.try_iter().all(|command| !matches!(command, Command::Customer(_))), "Opening the dialog saves nothing");
        click_text(&context, &mut app, show, language.text("Confirm").as_ref());
        let saved = receiver.try_iter().find_map(|command| match command {
            Command::Customer(tomlook::worker::CustomerCommand::Save(customer)) => Some(customer),
            _ => None,
        }).expect("Confirm saves the local customer");
        assert_eq!((saved.name.as_str(), saved.domains.as_slice(), saved.msx.is_none(), saved.confirmed), ("Orbit", &["orbit.example".to_owned()][..], true, true));
        assert!(app.customers.confirm.is_none());

        notices.send(ai::Notice::Msx(msx_status(Phase::Connected, tomlook::msx::CONNECTED, None))).unwrap();
        app.receive();
        click_text(&context, &mut app, show, language.text("Confirm customer").as_ref());
        let search = match ai_commands.try_recv() {
            Ok(ai::Command::CustomerAccounts { id, domains, names }) => {
                assert_eq!((domains, names), (vec!["orbit.example".to_owned()], vec!["Orbit".to_owned()]));
                id
            }
            _ => panic!("MSX connected: candidates are searched"),
        };
        assert!(settled(&context, &mut app, show).contains(language.text("Searching MSX for matching customers...").as_ref()));
        let candidate = |id: &str, name: &str, confidence: f32| tomlook::msx::dataverse::AccountCandidate {
            id: id.into(), name: name.into(), tpid: Some("777".into()), account_number: None, website: Some("https://www.orbit.example".into()),
            owner: Some(tomlook::msx::dataverse::Owner { id: None, name: Some("Jamie Brooks".into()) }),
            url: tomlook::msx::dataverse::record_url("account", id), confidence,
            evidence: vec![tomlook::msx::dataverse::Evidence::WebsiteDomain { domain: "orbit.example".into(), website: "https://www.orbit.example".into() }],
        };
        let likely = "11111111-0000-4000-8000-000000000001";
        notices.send(ai::Notice::CustomerAccounts { id: search, result: Ok(tomlook::msx::dataverse::AccountLookup {
            candidates: vec![candidate(likely, "Orbit Systems", 0.82), candidate("22222222-0000-4000-8000-000000000002", "Orbit Travel", 0.4)],
            selection: tomlook::msx::dataverse::Selection::Likely { account_id: likely.into() },
            requires_confirmation: true, rejected_inputs: vec![], sources: vec![],
        }) }).unwrap();
        app.receive();
        assert!(receiver.try_iter().all(|command| !matches!(command, Command::Customer(_))), "A likely match is never saved silently");
        let text = settled(&context, &mut app, show);
        for expected in [
            language.text("Likely match").into_owned(), "Orbit Systems".into(), "Orbit Travel".into(), "TPID 777".into(),
            language.text("Confidence 82%").into_owned(), language.text("Website https://www.orbit.example matches orbit.example").into_owned(),
            language.text("Owner: Jamie Brooks").into_owned(),
        ] {
            assert!(text.contains(&expected), "{language:?}: missing {expected}\n{text}");
        }
        click_text(&context, &mut app, show, language.text("Confirm").as_ref());
        let saved = receiver.try_iter().find_map(|command| match command {
            Command::Customer(tomlook::worker::CustomerCommand::Save(customer)) => Some(customer),
            _ => None,
        }).expect("One click confirms the shown candidate");
        let msx = saved.msx.clone().unwrap();
        assert_eq!((saved.name.as_str(), msx.id.as_str(), msx.tpid.as_deref(), msx.confidence), ("Orbit Systems", likely, Some("777"), Some(0.82)));

        let book = Arc::make_mut(&mut Arc::make_mut(&mut app.activity).customers);
        book.customers.insert(saved.id.clone(), *saved.clone());
        draw_with(&context, &mut app, vec![], show);
        let text = settled(&context, &mut app, show);
        assert!(text.contains("Orbit Systems") && text.contains(&language.text("MSX: Orbit Systems").into_owned()), "{text}");
        click_text(&context, &mut app, show, language.text("Customer timeline").as_ref());
        assert_eq!((app.customers.open, app.customers.selected.as_deref(), app.customers.focus.as_deref()), (true, Some(saved.id.as_str()), Some("orbit")));
        click_text(&context, &mut app, show, language.text("Not a customer meeting").as_ref());
        let choice = receiver.try_iter().find_map(|command| match command {
            Command::Customer(tomlook::worker::CustomerCommand::Choose(choice)) => Some(choice),
            _ => None,
        }).expect("The meeting is marked as not a customer meeting");
        assert_eq!((choice.customer.as_ref(), choice.event_id.as_str()), (None, "orbit"));
        let book = Arc::make_mut(&mut Arc::make_mut(&mut app.activity).customers);
        book.overrides.insert(choice.occurrence.clone(), *choice);
        let text = settled(&context, &mut app, show);
        assert!(text.contains(language.text("Marked as not a customer meeting.").as_ref()) && !text.contains("Orbit Systems"), "{text}");
        click_text(&context, &mut app, show, language.text("Detect automatically").as_ref());
        assert!(receiver.try_iter().any(|command| matches!(command, Command::Customer(tomlook::worker::CustomerCommand::Reset { .. }))));
        assert!(app.activity.customers.customers.contains_key(&id));
    }
}

#[test]
fn customers_open_from_the_palette_and_company_names_and_escape_closes_them() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, _, _, id) = customer_fixture(language);
        app.customers.open = false;
        let matches = command_matches_in(if language == tomlook::i18n::Language::Cs { "zákazníci" } else { "customers" }, language);
        assert_eq!(matches.first(), Some(&PaletteCommand::Customers));
        let context = egui::Context::default();
        app.run_palette_command(PaletteCommand::Customers, &context);
        assert!(app.customers.open && app.customers.selected.is_none(), "The palette lists all customers");
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.customers_panel(ui);
        let text = settled(&context, &mut app, show);
        assert!(text.contains("Northstar") && text.contains(&language.text("MSX: Northstar Holdings").into_owned()), "{text}");
        click_text(&context, &mut app, show, "Northstar");
        assert_eq!(app.customers.selected.as_deref(), Some(id.as_str()));
        app.open_company("eu.northstar.example", vec![], Some("next".into()));
        assert_eq!((app.customers.selected.as_deref(), app.customers.focus.as_deref()), (Some(id.as_str()), Some("next")));
        app.open_company("orbit.example", vec!["Kim Lambert <kim@orbit.example>".into()], None);
        assert_eq!(app.customers.confirm.as_ref().and_then(|draft| draft.domain.as_deref()), Some("orbit.example"));
        app.open_company("example.com", vec![], None);
        assert_eq!(app.customers.confirm.as_ref().and_then(|draft| draft.domain.as_deref()), Some("orbit.example"), "Own domains are not customers");
        let escape = egui::RawInput { events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], ..Default::default() };
        let _ = context.run_ui(escape.clone(), |ui| app.shortcuts(ui.ctx()));
        assert!(app.customers.confirm.is_none() && app.customers.open, "Escape first closes the confirmation");
        let _ = context.run_ui(escape, |ui| app.shortcuts(ui.ctx()));
        assert!(!app.customers.open, "Then the panel");
    }
}

#[test]
fn demo_customer_panel_shows_marked_sample_data_and_never_reads_msx_or_asks_ai() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        app.preview = true;
        app.loading = false;
        app.preferences.language = language;
        app.msx_status = tomlook::msx::Status::demo();
        let now = Local::now();
        app.calendar = Arc::new(Calendar::build(tomlook::showcase::preview_events(now, 198), Default::default(), Vec::new()).unwrap());
        let account = tomlook::feedback::PREVIEW_ACCOUNT;
        let samples = tomlook::showcase::sample_customers(account, language, chrono::Utc::now()).unwrap();
        let id = samples[0].customer.id.clone();
        let book = tomlook::customers::Book {
            account: Some(account.into()),
            customers: samples.iter().map(|sample| (sample.customer.id.clone(), sample.customer.clone())).collect(),
            msx: samples.iter().filter_map(|sample| Some((sample.customer.id.clone(), sample.msx.clone()?))).collect(),
            communications: samples.iter().filter_map(|sample| Some((sample.customer.id.clone(), sample.communication.clone()?))).collect(),
            ..Default::default()
        };
        let mut book = book;
        let ended = app.calendar.events.iter().find(|event| event.title == tomlook::showcase::ENDED_MEETING).unwrap();
        let choice = tomlook::customers::Override::new(account, ended, Some(id.clone()), chrono::Utc::now());
        book.overrides.insert(choice.occurrence.clone(), choice);
        app.activity = Arc::new(tomlook::worker::Activity { customers: Arc::new(book), ..Default::default() });
        app.open_customers(Some(id), None);
        let context = egui::Context::default();
        let show: Show = &|app: &mut Tomlook, ui: &mut egui::Ui| app.customers_panel(ui);
        let text = settled(&context, &mut app, show);
        for label in [
            "Sample data", tomlook::customers::SAMPLE_NOTE, "Sample communication summary: fictional, not produced by AI.",
            "Offline demo never sends AI requests.", "Azure consumption", "Open opportunities", "My task: yes", "My task: no",
            "My task: unknown", "Northstar / Architecture workshop", "Northstar / Pilot planning", "Notes ready",
        ] {
            assert!(text.contains(language.text(label).as_ref()), "{language:?}: missing {label}\n{text}");
        }
        assert!(!text.contains(language.text("Open in MSX").as_ref()), "Sample data never links to MSX");
        assert!(!text.contains(language.text("Source not among the retrieved results").as_ref()));
        assert!(ai_commands.try_recv().is_err(), "The demo never reads MSX or asks AI");
        let card_event = app.calendar.events.iter().find(|event| event.title == "Orbit / Project kickoff").unwrap().clone();
        let card: Show = &move |app: &mut Tomlook, ui: &mut egui::Ui| app.customer_card(ui, &card_event);
        let text = settled(&context, &mut app, card);
        assert!(text.contains(&language.text("Unconfirmed customer: orbit.invalid").into_owned()), "{text}");
        assert!(!text.contains("example.invalid"), "Fictional colleagues are the demo user's own organization");
    }
}
fn setup_probe(runtime: bool, saved: bool) -> Arc<tomlook::setup::Probe> {
    Arc::new(tomlook::setup::Probe {
        azure_cli: None,
        copilot: if runtime {
            Ok(tomlook::setup::Runtime { path: r"C:\Tools\copilot.exe".into(), version: Some("1.0.93".into()) })
        } else {
            Err("Read Tomlook connections.json: missing".into())
        },
        saved_connections: saved,
    })
}

#[test]
fn copilot_device_sign_in_displays_the_code_and_cancel_without_opening_a_browser() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _storage, mut commands, notices) = task_fixture(language);
        app.ai_ready = false;
        app.connection_panel = Some(ConnectionPanel::Ai);
        notices.send(ai::Notice::AiLogin {
            busy: true,
            message: "Enter ABCD-EFGH at github.com/login/device".into(),
        }).unwrap();
        app.receive();
        let context = egui::Context::default();
        let show: Show = &|app, ui| app.connection_dialog(ui.ctx());
        let text = settled(&context, &mut app, show);
        assert!(text.contains("ABCD-EFGH") && text.contains("github.com/login/device"), "{text}");
        assert!(commands.try_recv().is_err(), "Rendering never starts a login or opens a browser");
        click_text(&context, &mut app, show, language.text("Cancel AI sign-in").as_ref());
        assert!(matches!(commands.try_recv(), Ok(ai::Command::CancelAiLogin)));
        notices.send(ai::Notice::AiLogin {
            busy: false,
            message: "AI sign-in failed: cancelled".into(),
        }).unwrap();
        notices.send(ai::Notice::Connection { ready: false, message: "cancelled".into() }).unwrap();
        app.receive();
        let text = settled(&context, &mut app, show);
        assert!(!app.ai_login_busy && !app.ai_ready);
        assert!(text.contains(language.text("AI sign-in failed: cancelled").as_ref()), "{text}");
        assert!(!text.contains("AI sign-in completed."));
    }
}

#[test]
fn setup_checklist_is_bilingual_and_its_actions_reuse_the_existing_connection_flows() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        app.calendar_owner = None;
        app.ai_ready = false;
        app.connection = "Not connected".into();
        app.setup.probe = Some(setup_probe(true, false));
        app.open_setup_checklist();
        let context = egui::Context::default();
        let dialog: Show = &|app, ui| app.preferences_dialog(ui.ctx());
        let text = settled(&context, &mut app, dialog);
        assert!(text.lines().any(|line| line == language.text("Setup")), "Settings navigation offers Setup\n{text}");
        assert!(text.contains(language.text("Setup checklist").as_ref()));
        let show: Show = &|app, ui| app.setup_preferences(ui);
        let text = settled(&context, &mut app, show);
        for expected in ["Setup checklist", "Required steps", "Optional connections", "Prerequisites",
            "Microsoft 365 calendar (WorkIQ)", "GitHub Copilot", "Identity match", "Windows notifications", "Updates",
            "WebIQ", "GitHub connector", "Foundry / Auto mode", "MSX and Azure consumption", "Runtime prerequisites",
            "0 of 5 required steps done · 0 optional connected", "Copilot CLI installation guide", "Check again",
            "Show setup progress in the status bar", "Enter key", "Connect", "Open Foundry connection", "Open MSX connection",
            "Automatic Insights require matching connected identities and verified calendar coverage.", "Details"] {
            assert!(text.contains(language.text(expected).as_ref()), "{language:?}: missing {expected}\n{text}");
        }
        if language == tomlook::i18n::Language::Cs {
            assert!(text.contains("Kontrolní seznam nastavení") && text.contains("Shoda účtů"));
        }
        click_text(&context, &mut app, show, language.text("Next step: Sign in").as_ref());
        assert!(app.connection_panel == Some(ConnectionPanel::Workiq), "Sign in opens the WorkIQ dialog for the work e-mail");
        assert!(ai_commands.try_recv().is_err(), "Opening a dialog never starts a sign-in");
        app.connection_panel = None;
        app.workiq_ready = true;
        app.hosted_workiq_status = "Connected".into();
        let text = settled(&context, &mut app, show);
        assert!(text.contains(language.text("Next step: Sign in").as_ref()), "Copilot is the next required step\n{text}");
        click_text(&context, &mut app, show, language.text("Sign in").as_ref());
        assert!(matches!(ai_commands.try_recv(), Ok(ai::Command::SignInAi)), "The same isolated sign-in as the AI dialog");
        assert!(ai_commands.try_recv().is_err());
        assert!(app.ai_login_busy && app.connection_panel == Some(ConnectionPanel::Ai));
        app.connection_panel = None;
        click_text(&context, &mut app, show, language.text("Enter key").as_ref());
        assert!(app.connection_panel == Some(ConnectionPanel::External(tomlook::connectors::Kind::Webiq)));
        click_text(&context, &mut app, show, language.text("Connect").as_ref());
        assert!(app.connection_panel == Some(ConnectionPanel::External(tomlook::connectors::Kind::Github)));
        assert!(ai_commands.try_recv().is_err(), "Optional connections only open their dialogs");
        click_text(&context, &mut app, show, language.text("Show setup progress in the status bar").as_ref());
        assert!(app.preferences.setup_hint_hidden);
        assert!(matches!(receiver.try_recv(), Ok(Command::Preferences(preferences)) if preferences.setup_hint_hidden));
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().all(|command| matches!(command, ai::Command::MsxSettings(_))), "No sign-in or check from a preference change");
        click_text(&context, &mut app, show, language.text("Details").as_ref());
        let text = settled(&context, &mut app, show);
        assert!(text.contains(language.text("Calendar sync").as_ref()) || text.contains(language.text("Status").as_ref()),
            "Details show the honest current state\n{text}");
    }
}

#[test]
fn identity_match_uses_preparation_readiness_and_completed_steps_hide_their_actions() {
    let (mut app, _, mut ai_commands, _) = task_fixture(tomlook::i18n::Language::En);
    app.setup.probe = Some(setup_probe(true, true));
    app.workiq_ready = true;
    app.hosted_workiq_account = "someone@example.com".into();
    let items = app.setup_items();
    let identity = items.iter().find(|item| item.step == tomlook::setup::Step::Insights).unwrap();
    assert!(identity.details.iter().any(|detail| detail.value == tomlook::scheduler::Blocker::Coverage.message()));
    let today = Local::now().date_naive();
    app.calendar = Arc::new(Calendar::build(Vec::new(), [today].into(), Vec::new()).unwrap());
    let items = app.setup_items();
    let identity = items.iter().find(|item| item.step == tomlook::setup::Step::Insights).unwrap();
    assert_eq!(identity.action, Some(tomlook::setup::Action::OpenWorkiq));
    assert!(identity.details.iter().any(|detail| detail.value == tomlook::scheduler::FOREIGN_CALENDAR));
    app.hosted_workiq_account = "owner@example.com".into();
    let items = app.setup_items();
    let identity = items.iter().find(|item| item.step == tomlook::setup::Step::Insights).unwrap();
    assert_eq!((identity.state, identity.action), (tomlook::setup::State::Done, None));
    app.preferences.paused = true;
    let items = app.setup_items();
    let identity = items.iter().find(|item| item.step == tomlook::setup::Step::Insights).unwrap();
    assert_eq!(identity.action, Some(tomlook::setup::Action::OpenInsights));
    app.run_setup_action(tomlook::setup::Action::OpenInsights, &egui::Context::default());
    assert!(app.settings, "Opens the existing Insights activity window");
    assert!(ai_commands.try_recv().is_err());
}

#[test]
fn demo_setup_checklist_and_welcome_explain_the_offline_demo_without_actions_or_probes() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, _, mut ai_commands) = assistant_fixture();
        app.preview = true;
        app.loading = false;
        app.preferences.language = language;
        app.setup.root = Some(std::path::PathBuf::from(r"C:\not-probed"));
        app.open_setup_checklist();
        let context = egui::Context::default();
        let show: Show = &|app, ui| app.setup_preferences(ui);
        let text = settled(&context, &mut app, show);
        assert!(text.contains(language.text(tomlook::setup::DEMO).as_ref()), "{text}");
        assert!(text.contains(language.text("Offline demo: Tomlook never connects to Microsoft 365, GitHub Copilot or other services.").as_ref()));
        for absent in ["Sign in", "Check again", "Send test notification", "Enter key", "Copilot CLI installation guide", "Show setup progress in the status bar"] {
            assert!(!text.lines().any(|line| line == language.text(absent)), "{language:?}: the demo offers no {absent}\n{text}");
        }
        assert!(app.setup.probe.is_none() && app.setup.probing.is_none(), "The demo never probes the computer");
        app.run_setup_action(tomlook::setup::Action::SignInCopilot, &context);
        app.run_setup_action(tomlook::setup::Action::SignInWorkiq, &context);
        assert!(app.connection_panel.is_none() && ai_commands.try_recv().is_err());
        app.preferences_open = false;
        app.setup.welcome = true;
        let show: Show = &|app, ui| app.welcome_window(ui.ctx());
        let text = settled(&context, &mut app, show);
        for expected in ["Welcome to Tomlook", "Required steps", "Open setup checklist", "Later", tomlook::setup::DEMO] {
            assert!(text.contains(language.text(expected).as_ref()), "{language:?}: missing {expected}\n{text}");
        }
        assert!(!text.lines().any(|line| line == language.text("Sign in")), "The demo welcome offers no sign-in\n{text}");
        app.setup.eligible = true;
        let show: Show = &|app, ui| {
            ui.horizontal(|ui| app.setup_indicator(ui));
        };
        assert!(!settled(&context, &mut app, show).contains("0/5"), "The demo shows no setup indicator");
        assert!(ai_commands.try_recv().is_err());
    }
}

#[test]
fn welcome_opens_once_for_a_new_profile_and_never_for_upgraded_or_diagnostic_profiles() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        app.calendar_owner = None;
        app.ai_ready = false;
        app.connection = "Not connected".into();
        app.setup.eligible = true;
        app.sync_setup();
        assert!(!app.setup.welcome && !app.setup.decided, "Waits for the background probe");
        app.setup.probe = Some(setup_probe(false, false));
        app.sync_setup();
        assert!(app.setup.welcome && app.preferences.welcome_shown);
        assert!(matches!(receiver.try_recv(), Ok(Command::Preferences(preferences)) if preferences.welcome_shown));
        let context = egui::Context::default();
        let show: Show = &|app, ui| app.welcome_window(ui.ctx());
        let text = settled(&context, &mut app, show);
        for expected in ["Welcome", "Welcome to Tomlook", "Language", "Theme", "Dark", "Required steps",
            "Microsoft 365 calendar (WorkIQ)", "GitHub Copilot", "Identity match", "Windows notifications", "Updates",
            "Install Copilot CLI", "Open setup checklist", "Later",
            "The checklist stays in Settings > Setup and in Commands (Ctrl+Shift+P)."] {
            assert!(text.contains(language.text(expected).as_ref()), "{language:?}: missing {expected}\n{text}");
        }
        assert!(!text.contains(language.text("WebIQ").as_ref()), "Only required steps are listed");
        click_text(&context, &mut app, show, if language == tomlook::i18n::Language::En { "Čeština" } else { "English" });
        let language = language.other();
        assert_eq!(app.preferences.language, language);
        assert!(matches!(receiver.try_recv(), Ok(Command::Preferences(preferences)) if preferences.language == language));
        click_text(&context, &mut app, show, language.text("Later").as_ref());
        assert!(!app.setup.welcome);
        app.sync_setup();
        assert!(!app.setup.welcome, "The welcome opens only once");
        app.setup.welcome = true;
        click_text(&context, &mut app, show, language.text("Open setup checklist").as_ref());
        assert!(!app.setup.welcome && app.preferences_open && app.preferences_section == PreferencesSection::Setup);
        app.setup.welcome = true;
        click_text(&context, &mut app, show, language.text("Sign in").as_ref());
        assert!(!app.setup.welcome && app.connection_panel == Some(ConnectionPanel::Workiq));
        app.setup.welcome = true;
        let escape = egui::RawInput { events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], ..Default::default() };
        app.connection_panel = None;
        app.preferences_open = false;
        let mut output = context.run_ui(escape, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.setup.welcome, "Escape closes the welcome");
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().all(|command| matches!(command, ai::Command::MsxSettings(_))), "The welcome never signs in by itself");
    }
    let (mut upgraded, receiver, _, _) = task_fixture(tomlook::i18n::Language::En);
    upgraded.setup.eligible = true;
    upgraded.setup.probe = Some(setup_probe(false, false));
    upgraded.sync_setup();
    assert!(!upgraded.setup.welcome && upgraded.preferences.welcome_shown, "A profile with a synced calendar is an upgrade");
    assert!(matches!(receiver.try_recv(), Ok(Command::Preferences(preferences)) if preferences.welcome_shown));
    let (mut saved, receiver, _, _) = task_fixture(tomlook::i18n::Language::En);
    saved.calendar_owner = None;
    saved.setup.eligible = true;
    saved.setup.probe = Some(setup_probe(true, true));
    saved.sync_setup();
    assert!(!saved.setup.welcome && saved.preferences.welcome_shown);
    assert!(receiver.try_recv().is_ok());
    let (mut diagnostic, receiver, _, _) = task_fixture(tomlook::i18n::Language::En);
    diagnostic.calendar_owner = None;
    diagnostic.setup.probe = Some(setup_probe(false, false));
    diagnostic.sync_setup();
    assert!(!diagnostic.setup.welcome && !diagnostic.preferences.welcome_shown && receiver.try_recv().is_err(),
        "Demo, stress, measurement and acceptance runs never welcome or save");
    let (mut shown, receiver, _, _) = task_fixture(tomlook::i18n::Language::En);
    shown.calendar_owner = None;
    shown.preferences.welcome_shown = true;
    shown.setup.eligible = true;
    shown.setup.probe = Some(setup_probe(false, false));
    shown.sync_setup();
    assert!(!shown.setup.welcome && receiver.try_recv().is_err());
}

#[test]
fn setup_indicator_shows_progress_opens_the_checklist_and_dont_show_persists() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let (mut app, receiver, mut ai_commands, _) = task_fixture(language);
        app.setup.eligible = true;
        app.setup.probe = Some(setup_probe(true, true));
        app.ai_ready = false;
        app.connection = "Not connected".into();
        let context = egui::Context::default();
        let show: Show = &|app, ui| {
            ui.horizontal(|ui| app.setup_indicator(ui));
        };
        let badge = "0/5";
        assert!(settled(&context, &mut app, show).contains(badge));
        click_text(&context, &mut app, show, badge);
        assert!(app.preferences_open && app.preferences_section == PreferencesSection::Setup);
        app.hosted_workiq_busy = true;
        assert!(!settled(&context, &mut app, show).contains(badge), "No indicator while a required step is still being checked");
        app.hosted_workiq_busy = false;
        let output = draw_with(&context, &mut app, vec![], show);
        let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == badge => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        }).unwrap();
        for pressed in [true, false] {
            draw_with(&context, &mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE,
            }], show);
        }
        click_text(&context, &mut app, show, language.text("Don't show").as_ref());
        assert!(app.preferences.setup_hint_hidden);
        assert!(matches!(receiver.try_recv(), Ok(Command::Preferences(preferences)) if preferences.setup_hint_hidden));
        assert!(!settled(&context, &mut app, show).contains(badge), "Dismissal persists");
        app.preferences.setup_hint_hidden = false;
        app.setup.eligible = false;
        assert!(!settled(&context, &mut app, show).contains(badge), "Diagnostic runs show no indicator");
        let mut seen = Vec::new();
        drain_ai(&mut ai_commands, &mut seen);
        assert!(seen.iter().all(|command| matches!(command, ai::Command::MsxSettings(_))));
    }
}

#[test]
fn setup_checklist_opens_from_the_command_palette_in_both_languages() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
        let matches = command_matches_in(if language == tomlook::i18n::Language::Cs { "kontrolní seznam" } else { "setup" }, language);
        assert_eq!(matches.first(), Some(&PaletteCommand::Setup));
        let (mut app, _, mut ai_commands, _) = task_fixture(language);
        let context = egui::Context::default();
        app.open_palette();
        app.run_palette_command(PaletteCommand::Setup, &context);
        assert!(app.preferences_open && app.preferences_section == PreferencesSection::Setup && !app.palette);
        assert!(ai_commands.try_recv().is_err());
    }
}
