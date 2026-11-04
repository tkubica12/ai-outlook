use super::*;
use tomlook::invitations::{self, BulkAction, Conflicts, LocalAction, OrganizerRule, Tab};

pub(super) struct BulkDraft {
    account: String,
    events: Vec<Event>,
    action: BulkAction,
}

struct ReturnToCalendar {
    anchor: NaiveDate,
    mini_anchor: Option<NaiveDate>,
    dates: Option<DateSelection>,
    selected: Option<String>,
}

pub(super) struct InvitationsUi {
    pub open: bool,
    pub selected: Option<String>,
    pub(super) marked: BTreeSet<String>,
    selection_anchor: Option<String>,
    selection_account: String,
    pub(super) bulk: Option<BulkDraft>,
    pending_focus: Option<String>,
    back: Option<ReturnToCalendar>,
    tab: Tab,
    page: usize,
    week: bool,
    settings: bool,
    settings_account: String,
    queries: String,
    included: String,
    excluded: String,
}

impl Default for InvitationsUi {
    fn default() -> Self {
        Self { open: false, selected: None, marked: BTreeSet::new(), selection_anchor: None,
            selection_account: String::new(), bulk: None, pending_focus: None, back: None, tab: Tab::Important,
            page: 0, week: false, settings: false, settings_account: String::new(), queries: String::new(),
            included: String::new(), excluded: String::new() }
    }
}

fn addresses(text: &str) -> Result<BTreeSet<String>, String> {
    text.split(['\n', ';', ',']).map(str::trim).filter(|value| !value.is_empty())
        .map(|value| person_email(value).ok_or_else(|| "Team list contains an invalid email address".into()))
        .collect()
}

impl Tomlook {
    pub(super) fn open_invitations(&mut self) {
        if !self.invitations.open {
            self.invitations.back = Some(ReturnToCalendar { anchor: self.anchor, mini_anchor: self.mini_anchor,
                dates: self.date_selection, selected: self.selected.map(|index| self.calendar.events[index].id.clone()) });
        }
        self.invitations.open = true;
    }

    fn close_invitations(&mut self) {
        self.invitations.open = false;
        self.invitations.selected = None;
        self.invitations.pending_focus = None;
        self.invitations.marked.clear();
        self.invitations.selection_anchor = None;
        self.invitations.bulk = None;
        if let Some(back) = self.invitations.back.take() {
            self.anchor = back.anchor;
            self.mini_anchor = back.mini_anchor;
            self.date_selection = back.dates;
            self.date_drag = None;
            if let Some(index) = back.selected.as_deref().and_then(|id| self.calendar.event_index(id)) {
                self.select(index);
            } else {
                self.flush_followup();
                self.selected = None;
                self.detail_revision += 1;
                self.briefing = None;
            }
        }
    }

    pub(super) fn invitation_context(&self) -> bool {
        self.invitations.open && self.invitations.selected.is_some()
    }

    pub(super) fn invitation_highlighted(&self, event_id: &str) -> bool {
        self.invitations.open
            && self.calendar_owner.as_deref() == Some(self.activity.invitations.book.account.as_str())
            && self.invitations.selected.as_deref().is_some_and(|key| {
                self.activity.invitations.rows.iter().any(|row| row.key == key && row.event.id == event_id)
            })
    }

    fn invitation_command(&mut self, command: invitations::Command) {
        self.command(Command::Invitation(command));
    }

    fn invitation_local(&mut self, action: LocalAction) {
        let account = self.activity.invitations.book.account.clone();
        self.invitation_command(invitations::Command::Local { account, action });
    }

    pub(super) fn invitation_navigation(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.invitations.open, language.text("Calendar")).clicked() {
                self.close_invitations();
            }
            let count = self.activity.invitations.important();
            let label = if count == 0 { language.text("Invitations").into_owned() }
                else { format!("{}  {count}", language.text("Invitations")) };
            if ui.selectable_label(self.invitations.open, label)
                .on_hover_text(language.text("Unanswered invitations, prioritized without AI"))
                .clicked() { self.open_invitations(); }
        });
        ui.separator();
    }

    fn focus_invitation_dates(&mut self, day: NaiveDate) {
        let (first, last) = if self.invitations.week {
            let first = tomlook::calendar::monday(day);
            (first, first + chrono::Duration::days(6))
        } else { (day.pred_opt().unwrap_or(day), day.succ_opt().unwrap_or(day)) };
        self.anchor = day;
        self.mini_anchor = None;
        self.date_drag = None;
        self.date_selection = Some(DateSelection::new(first, last));
        self.timeline_period = None;
        self.scroll_meeting = true;
    }

    pub(super) fn open_invitation(&mut self, row: &invitations::Row) {
        self.open_invitations();
        self.invitations.selected = Some(row.key.clone());
        self.invitations.pending_focus = Some(row.event.id.clone());
        self.focus_invitation_dates(row.event.calendar_dates().0);
        self.assistant = false;
        self.invitation_command(invitations::Command::Open {
            account: self.activity.invitations.book.account.clone(), key: row.key.clone(),
        });
    }

    pub(super) fn receive_invitation_focus(&mut self) {
        if let Some(id) = &self.invitations.pending_focus
            && let Some(index) = self.calendar.event_index(id) {
            self.invitations.pending_focus = None;
            self.focus_invitation_dates(self.calendar.events[index].calendar_dates().0);
            self.select(index);
            self.focus_meeting = true;
            self.scroll_meeting = true;
        }
    }

    pub(super) fn invitation_context_bar(&mut self, ui: &mut egui::Ui) {
        if !self.invitation_context() { return; }
        let language = self.preferences.language;
        ui.horizontal_wrapped(|ui| {
            ui.weak(language.text("Invitation context: calendar filters are temporarily ignored."));
            if ui.selectable_label(!self.invitations.week, language.text("Day + neighbors")).clicked() {
                self.invitations.week = false;
                self.focus_invitation_dates(self.anchor);
            }
            if ui.selectable_label(self.invitations.week, language.text("Whole week")).clicked() {
                self.invitations.week = true;
                self.focus_invitation_dates(self.anchor);
            }
        });
        if let Some(event) = self.selected.and_then(|index| self.calendar.events.get(index))
            && matches!(event.extra.get("type").and_then(serde_json::Value::as_str), Some("occurrence" | "exception")) {
            ui.weak(language.text("Recurring invitation: responses apply only to this occurrence."));
        }
    }

    pub(super) fn invitation_sidebar(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        let snapshot = self.activity.invitations.clone();
        let account = &snapshot.book.account;
        if self.invitations.selection_account != *account {
            self.invitations.marked.clear();
            self.invitations.selection_anchor = None;
            self.invitations.selection_account = account.clone();
        }
        let visible_keys = snapshot.rows.iter().filter(|row| row.tab == self.invitations.tab)
            .map(|row| &row.key).collect::<BTreeSet<_>>();
        self.invitations.marked.retain(|key| visible_keys.contains(key));
        ui.horizontal_wrapped(|ui| {
            ui.heading(language.text("Invitations"));
            if ui.add_enabled(!snapshot.sync.busy && self.workiq_ready,
                egui::Button::new(language.text("Refresh"))).clicked() {
                self.invitation_command(invitations::Command::Refresh { account: account.clone() });
            }
            if ui.small_button(language.text("My team")).clicked() { self.open_invitation_settings(); }
        });
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [(Tab::Important, "Important"), (Tab::Other, "Other"), (Tab::Snoozed, "Snoozed")] {
                let count = snapshot.rows.iter().filter(|row| row.tab == tab).count();
                if ui.selectable_label(self.invitations.tab == tab,
                    format!("{} {count}", language.text(label))).clicked() {
                    self.invitations.tab = tab;
                    self.invitations.page = 0;
                    self.invitations.marked.clear();
                    self.invitations.selection_anchor = None;
                }
            }
        });
        if snapshot.sync.busy { ui.spinner(); ui.weak(language.text("Checking unanswered invitations...")); }
        else if let Some(stamp) = snapshot.book.refreshed_at {
            ui.weak(format!("{} {}", language.text("Last checked:"), stamp.with_timezone(&Local).format("%d.%m. %H:%M")));
        } else { ui.weak(language.text("Cached invitations; a complete queue refresh has not finished yet.")); }
        if !self.workiq_ready { ui.weak(language.text("Offline: showing saved invitations.")); }
        if let Some(error) = &snapshot.sync.error { ui.colored_label(Color32::LIGHT_RED, language.text(error)); }
        ui.weak(language.text("Next six months. Reading is not accepting; snoozing changes only Tomlook."));
        ui.add_space(6.0);
        let tab = self.invitations.tab;
        let rows = snapshot.rows.iter().filter(|row| row.tab == tab).collect::<Vec<_>>();
        let pages = rows.len().div_ceil(40).max(1);
        self.invitations.page = self.invitations.page.min(pages - 1);
        if pages > 1 {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.invitations.page > 0, egui::Button::new("<"))
                    .on_hover_text(language.text("Previous invitations")).clicked() { self.invitations.page -= 1; }
                ui.weak(format!("{} / {pages}", self.invitations.page + 1));
                if ui.add_enabled(self.invitations.page + 1 < pages, egui::Button::new(">"))
                    .on_hover_text(language.text("Next invitations")).clicked() { self.invitations.page += 1; }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!rows.is_empty(), egui::Button::new(language.text("Select page"))).clicked() {
                self.invitations.marked.extend(rows.iter().skip(self.invitations.page * 40).take(40).map(|row| row.key.clone()));
            }
            if ui.add_enabled(!self.invitations.marked.is_empty(), egui::Button::new(language.text("Clear selection"))).clicked() {
                self.invitations.marked.clear();
                self.invitations.selection_anchor = None;
            }
            ui.weak(format!("{}: {}", language.text("Selected invitations"), self.invitations.marked.len()));
        });
        if !self.invitations.marked.is_empty() {
            ui.menu_button(language.text("Selected invitation actions"), |ui| self.invitation_bulk_menu(ui));
        }
        for row in rows.iter().skip(self.invitations.page * 40).take(40) {
            let selected = self.invitations.selected.as_deref() == Some(row.key.as_str());
            let marked = self.invitations.marked.contains(&row.key);
            let response = ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                egui::Frame::new().fill(if selected || marked {
                    self.accent().gamma_multiply(if self.preferences.dark { 0.2 } else { 0.08 })
                } else { ui.visuals().faint_bg_color })
                    .stroke(if selected || marked { Stroke::new(1.0, self.accent()) } else { Stroke::NONE })
                    .corner_radius(6).inner_margin(8).show(ui, |ui| {
                        ui.set_width((ui.available_width() - 2.0).max(0.0));
                        ui.horizontal_top(|ui| {
                            let mut checked = marked;
                            let checkbox = ui.checkbox(&mut checked, "").on_hover_text(language.text("Select invitation"));
                            let title = ui.add(egui::Label::new(RichText::new(&row.event.title).strong()).wrap());
                            if checkbox.labelled_by(title.id).changed() {
                                if checked { self.invitations.marked.insert(row.key.clone()); }
                                else { self.invitations.marked.remove(&row.key); }
                                self.invitations.selection_anchor = Some(row.key.clone());
                            }
                        });
                        ui.add(egui::Label::new(meeting_date_label(&row.event, language)).wrap());
                        ui.add(egui::Label::new(RichText::new(&row.event.organizer).weak()).wrap());
                        ui.add(egui::Label::new(RichText::new(language.text(row.reason.label())).small().color(self.accent())).wrap());
                        if !row.detail.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&row.detail).weak()).wrap());
                        }
                        ui.add(egui::Label::new(RichText::new(match row.conflicts {
                            Conflicts::Unknown => language.text("Conflicts not checked"),
                            Conflicts::Clear => language.text("No conflicts in recently refreshed calendar"),
                            Conflicts::Found(_) => language.text("Calendar conflict - review before responding"),
                        }).weak()).wrap());
                        if row.updated { ui.weak(language.text("Invitation changed")); }
                        else if chrono::Utc::now() - row.first_seen < chrono::Duration::days(1) {
                            ui.weak(language.text("Newly seen"));
                        }
                        if let Some(snooze) = snapshot.book.snoozed.get(&row.key)
                            && row.tab == Tab::Snoozed {
                            ui.weak(format!("{} {}", language.text("Snoozed until:"),
                                snooze.until.with_timezone(&Local).format("%d.%m. %H:%M")));
                        }
                    });
            }).response;
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &row.event.title));
            if response.clicked() {
                let modifiers = ui.input(|input| input.modifiers);
                if modifiers.shift || modifiers.ctrl || modifiers.command {
                    let anchor = self.invitations.selection_anchor.as_deref()
                        .and_then(|key| rows.iter().position(|row| row.key == key));
                    let clicked = rows.iter().position(|candidate| candidate.key == row.key).unwrap();
                    if modifiers.shift && let Some(anchor) = anchor {
                        self.invitations.marked.extend(rows[anchor.min(clicked)..=anchor.max(clicked)]
                            .iter().map(|row| row.key.clone()));
                    } else {
                        if !self.invitations.marked.remove(&row.key) { self.invitations.marked.insert(row.key.clone()); }
                        self.invitations.selection_anchor = Some(row.key.clone());
                    }
                } else {
                    self.open_invitation(row);
                }
            }
            if response.secondary_clicked() && !self.invitations.marked.contains(&row.key) {
                self.invitations.marked.clear();
                self.invitations.marked.insert(row.key.clone());
                self.invitations.selection_anchor = Some(row.key.clone());
            }
            response.context_menu(|ui| self.invitation_row_menu(ui, row));
            ui.add_space(6.0);
        }
        if rows.is_empty() { ui.label(language.text("No invitations in this view.")); }
    }

    pub(super) fn demo_invitation_queue_ready(&mut self) -> bool {
        self.open_invitations();
        if self.invitations.selected.is_none() {
            let row = self.activity.invitations.rows.iter().find(|row| row.tab == Tab::Important).cloned();
            let Some(row) = row else { return false; };
            self.open_invitation(&row);
        }
        self.invitations.pending_focus.is_none() && self.selected.is_some()
    }

    fn invitation_row_menu(&mut self, ui: &mut egui::Ui, row: &invitations::Row) {
        let language = self.preferences.language;
        self.invitation_bulk_menu(ui);
        if self.invitations.marked.len() > 1 { return; }
        ui.separator();
        if ui.button(language.text("Open in calendar")).clicked() { self.open_invitation(row); ui.close(); }
        ui.separator();
        if row.tab == Tab::Snoozed {
            if ui.button(language.text("Unsnooze")).clicked() {
                self.invitation_local(LocalAction::Unsnooze(row.key.clone())); ui.close();
            }
        } else {
            for (hours, label) in [(1, "Snooze 1 hour"), (24, "Snooze 1 day"), (168, "Snooze 1 week")] {
                if ui.button(language.text(label)).clicked() {
                    self.invitation_local(LocalAction::Snooze { key: row.key.clone(),
                        until: chrono::Utc::now() + chrono::Duration::hours(hours) });
                    ui.close();
                }
            }
        }
        if let Some(email) = person_email(&row.event.organizer) {
            for (rule, label) in [(Some(OrganizerRule::Important), "Highlight this organizer"),
                (Some(OrganizerRule::Quiet), "Quiet this organizer"), (None, "Restore organizer priority")] {
                if ui.button(language.text(label)).clicked() {
                    self.invitation_local(LocalAction::Organizer { email: email.clone(), rule }); ui.close();
                }
            }
        }
        let quiet = !self.activity.invitations.book.quiet_series.contains(&row.key);
        let recurring = matches!(row.event.extra.get("type").and_then(serde_json::Value::as_str), Some("occurrence" | "exception"));
        let label = match (quiet, recurring) {
            (true, true) => "Quiet this series", (false, true) => "Restore this series",
            (true, false) => "Quiet this invitation", (false, false) => "Restore this invitation",
        };
        if ui.button(language.text(label)).clicked() {
            self.invitation_local(LocalAction::QuietSeries { key: row.key.clone(), quiet }); ui.close();
        }
    }

    fn invitation_bulk_menu(&mut self, ui: &mut egui::Ui) {
        let language = self.preferences.language;
        ui.strong(format!("{}: {}", language.text("Selected invitations"), self.invitations.marked.len()));
        let ready = self.workiq_ready && self.ai.is_some()
            && self.calendar_owner.as_deref() == Some(self.activity.invitations.book.account.as_str())
            && !self.invitations.marked.is_empty();
        for action in [BulkAction::Decline, BulkAction::Follow] {
            if ui.add_enabled(ready, egui::Button::new(language.text(action.label())))
                .on_disabled_hover_text(language.text("Connect WorkIQ before confirming a calendar change")).clicked() {
                let events = self.activity.invitations.rows.iter()
                    .filter(|row| self.invitations.marked.contains(&row.key)).map(|row| row.event.clone()).collect();
                self.invitations.bulk = Some(BulkDraft {
                    account: self.activity.invitations.book.account.clone(), events, action,
                });
                ui.close();
            }
        }
    }

    pub(super) fn invitation_bulk_dialog(&mut self, context: &egui::Context) {
        let Some(draft) = self.invitations.bulk.take() else { return; };
        let language = self.preferences.language;
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        let unchanged = self.calendar_owner.as_deref() == Some(draft.account.as_str())
            && self.activity.invitations.book.account == draft.account
            && !draft.events.is_empty()
            && draft.events.iter().all(|event| self.activity.invitations.rows.iter().any(|row|
                row.event.id == event.id && invitations::pending(&row.event, chrono::Utc::now())
                    && tomlook::scheduler::fingerprint(&row.event) == tomlook::scheduler::fingerprint(event)));
        let validation = draft.events.iter().try_for_each(|event|
            tomlook::recommendations::validate_manual_change(event, &draft.action.change()));
        egui::Window::new(language.text(draft.action.label())).id(Id::new("invitation-bulk-confirm"))
            .open(&mut open).default_width(480.0).resizable(true).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(context, |ui| {
                ui.strong(format!("{}: {}", language.text("Selected invitations"), draft.events.len()));
                ui.label(language.text(match draft.action {
                    BulkAction::Decline => "Declines every listed invitation without notifying the organizer.",
                    BulkAction::Follow => "Adds Follow, keeps other categories and marks this occurrence as free. Your invitation response is unchanged; the organizer is not notified.",
                }));
                ui.weak(language.text("Recurring invitation: responses apply only to this occurrence."));
                egui::ScrollArea::vertical().id_salt("invitation-bulk-events").max_height(280.0).show(ui, |ui| {
                    for event in &draft.events {
                        ui.strong(&event.title);
                        ui.weak(meeting_date_label(event, language));
                        ui.weak(&event.organizer);
                        ui.separator();
                    }
                });
                ui.weak(language.text("Review individual progress and outcomes in Messages. Failed or uncertain changes are not retried automatically."));
                if !unchanged {
                    ui.colored_label(Color32::LIGHT_RED, language.text("Selected invitations changed; close this dialog and select them again"));
                }
                if let Err(error) = &validation { ui.colored_label(Color32::LIGHT_RED, language.text(error)); }
                ui.horizontal(|ui| {
                    confirm = ui.add_enabled(unchanged && validation.is_ok() && self.workiq_ready && self.ai.is_some(),
                        egui::Button::new(language.text("Confirm for all selected invitations"))).clicked();
                    cancel = ui.button(language.text("Back")).clicked();
                });
            });
        if confirm {
            let command = Command::ConfirmInvitationActions {
                account: draft.account.clone(), events: draft.events.clone(), action: draft.action,
            };
            let result = self.worker.as_ref().ok_or("The storage worker is unavailable. Restart Tomlook.")
                .and_then(|worker| worker.commands.try_send(command).map_err(|error| match error {
                    mpsc::TrySendError::Full(_) => "Local queue is full. Nothing was submitted; try again.",
                    mpsc::TrySendError::Disconnected(_) => "The storage worker is unavailable. Restart Tomlook.",
                }));
            match result {
                Ok(()) => {
                    self.invitations.marked.clear();
                    self.invitations.selection_anchor = None;
                    self.messages_open = true;
                }
                Err(error) => {
                    self.error = Some(error.into());
                    self.invitations.bulk = Some(draft);
                }
            }
        } else if open && !cancel {
            self.invitations.bulk = Some(draft);
        }
    }

    fn open_invitation_settings(&mut self) {
        let book = &self.activity.invitations.book;
        self.invitations.settings_account = book.account.clone();
        self.invitations.queries = book.discovery_queries.join("\n");
        self.invitations.included = book.people.iter().cloned().collect::<Vec<_>>().join("\n");
        self.invitations.excluded = book.excluded_people.iter().cloned().collect::<Vec<_>>().join("\n");
        self.invitations.settings = true;
    }

    pub(super) fn invitation_settings(&mut self, context: &egui::Context) {
        if !self.invitations.settings { return; }
        let language = self.preferences.language;
        let snapshot = self.activity.invitations.clone();
        let account = snapshot.book.account.clone();
        let mut open = true;
        egui::Window::new(language.text("Invitation priorities and my team")).id(Id::new("invitation-settings"))
            .open(&mut open).default_width(630.0).default_height(610.0).resizable(true).show(context, |ui| {
                if self.invitations.settings_account != account || account.is_empty() {
                    ui.colored_label(Color32::LIGHT_RED, language.text("Calendar account changed. Close and reopen these settings."));
                    return;
                }
                egui::ScrollArea::vertical().id_salt("invitation-settings-scroll").show(ui, |ui| {
                    ui.weak(&account);
                    ui.label(language.text("Customer matches use confirmed domains. A different domain alone is not proof of another tenant or a customer."));
                    ui.heading(language.text("Team groups"));
                    ui.label(language.text("Search historical received mail for group names, one search per line."));
                    ui.add(egui::TextEdit::multiline(&mut self.invitations.queries).desired_rows(3)
                        .desired_width(f32::INFINITY).hint_text(language.text("Region STU\nRegion ATU\nRegion CSU")));
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(self.workiq_ready && !snapshot.teams.busy,
                            egui::Button::new(language.text("Find groups in historical mail"))).clicked() {
                            let queries = self.invitations.queries.lines().map(str::trim)
                                .filter(|value| !value.is_empty()).map(str::to_owned).collect();
                            self.invitation_command(invitations::Command::Discover { account: account.clone(), queries });
                        }
                        if ui.add_enabled(self.workiq_ready && !snapshot.teams.busy,
                            egui::Button::new(language.text("Refresh team members"))).clicked() {
                            self.invitation_command(invitations::Command::RefreshTeams { account: account.clone() });
                        }
                        if snapshot.teams.busy { ui.spinner(); }
                    });
                    ui.weak(language.text("Discovery samples up to 25 messages per search. Select only groups that really represent your close team; membership refreshes daily."));
                    if let Some(error) = &snapshot.teams.error { ui.colored_label(Color32::LIGHT_RED, language.text(error)); }
                    for group in snapshot.book.groups.values() {
                        ui.separator();
                        let mut enabled = group.enabled;
                        if ui.checkbox(&mut enabled, &group.name).changed() {
                            self.invitation_local(LocalAction::GroupEnabled { id: group.id.clone(), enabled });
                        }
                        ui.weak(&group.mail);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{}: {}", language.text("Cached email aliases"), group.members.len()));
                            if let Some(stamp) = group.refreshed_at {
                                ui.weak(stamp.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string());
                            } else { ui.weak(language.text("Membership not loaded yet")); }
                            if ui.small_button(language.text("Remove from my team")).clicked() {
                                self.invitation_local(LocalAction::RemoveGroup(group.id.clone()));
                            }
                        });
                        if let Some(error) = &group.error { ui.colored_label(Color32::LIGHT_RED, language.text(error)); }
                    }
                    ui.add_space(12.0);
                    ui.heading(language.text("People and exceptions"));
                    ui.weak(language.text("One email address per line. Explicit exclusions override group membership."));
                    ui.label(language.text("Include in my close team"));
                    ui.add(egui::TextEdit::multiline(&mut self.invitations.included).desired_rows(3).desired_width(f32::INFINITY));
                    ui.label(language.text("Exclude from team priority"));
                    ui.add(egui::TextEdit::multiline(&mut self.invitations.excluded).desired_rows(3).desired_width(f32::INFINITY));
                    if ui.button(language.text("Save people and exceptions")).clicked() {
                        match addresses(&self.invitations.included).and_then(|included|
                            addresses(&self.invitations.excluded).map(|excluded| (included, excluded))) {
                            Ok((included, excluded)) => self.invitation_local(LocalAction::People { included, excluded }),
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if !snapshot.book.organizers.is_empty() {
                        ui.add_space(12.0);
                        ui.heading(language.text("Organizer rules"));
                        for (email, rule) in &snapshot.book.organizers {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(email);
                                ui.weak(language.text(match rule { OrganizerRule::Important => "Highlighted organizer",
                                    OrganizerRule::Quiet => "Quiet by your choice" }));
                                if ui.small_button(language.text("Restore default")).clicked() {
                                    self.invitation_local(LocalAction::Organizer { email: email.clone(), rule: None });
                                }
                            });
                        }
                    }
                    ui.add_space(12.0);
                    ui.weak(language.text("These priorities and caches stay in this Tomlook profile. No meeting response, message or directory change is sent."));
                });
            });
        self.invitations.settings = open;
    }
}
