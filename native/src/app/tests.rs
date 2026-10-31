#[test]
fn settings_content_has_clear_padding_from_navigation_in_both_sections_and_themes() {
    for dark in [true, false] {
        for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
            for section in [PreferencesSection::Display, PreferencesSection::Notifications, PreferencesSection::Prompts, PreferencesSection::Updates] {
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
                    PreferencesSection::Display => "Display",
                    PreferencesSection::Notifications => "Notifications",
                    PreferencesSection::Prompts => "AI prompts",
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
fn right_click_owned_meeting_opens_paired_travel_confirmation_and_never_recreates_the_meeting() {
    for language in [tomlook::i18n::Language::En, tomlook::i18n::Language::Cs] {
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
        event.extra.insert("isOrganizer".into(), serde_json::json!(true));
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
        click(&mut app, language.text("Add travel blocks before and after").as_ref());
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
fn travel_context_action_is_not_offered_for_attendee_meetings_and_escape_never_confirms() {
    let (mut app, _, mut commands) = assistant_fixture();
    let mut event = tomlook::calendar::fixture(1).remove(0);
    event.is_all_day = false;
    event.extra.insert("type".into(), serde_json::json!("singleInstance"));
    event.extra.insert("isOrganizer".into(), serde_json::json!(false));
    event.extra.insert("sensitivity".into(), serde_json::json!("normal"));
    app.calendar = Arc::new(Calendar::build(vec![event.clone()], Default::default(), Vec::new()).unwrap());
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| app.meeting_actions(ui, 0));
    let mut text = String::new();
    for shape in &output.shapes { append_shape_text(&shape.shape, &mut text); }
    output.textures_delta.clear();
    assert!(!text.contains("Add travel blocks before and after"));
    event.extra.insert("isOrganizer".into(), serde_json::json!(true));
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
    assert!(app.selected.is_none());
    assert!(app.meeting_action.is_none());
    assert!(commands.try_recv().is_err());
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
                send_response: false
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
                    text.lines().count() <= 12,
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
    assert_eq!(app.active_request, Some(question.id));
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
    opened.send(None).unwrap();
    opened.send(Some(target.id.clone())).unwrap();
    app.receive_reminders();
    assert_eq!(app.selected, app.calendar.event_index(&target.id));
    assert_eq!(app.anchor, target.calendar_dates().0);
    assert!(app.scroll_meeting && !app.insights_expanded && !app.assistant && app.query.is_empty());
    assert!(receiver.try_iter().any(|command| matches!(command, Command::Detail { id, .. } if id == target.id)));
    opened.send(Some("missing-meeting".into())).unwrap();
    app.receive_reminders();
    assert_eq!(app.error.as_deref(), Some("Meeting is no longer in the current calendar."));
    assert!(receiver.try_iter().all(|command| matches!(command, Command::Detail { .. } | Command::Preferences(_))));
    assert!(ai_commands.try_recv().is_err());
}
