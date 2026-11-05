//! Customer panel (timeline), the customer card of meeting details and customer confirmation.
use super::myday::{card, chip};
use super::*;
use tomlook::customers::{self as model, Party, research};
use tomlook::worker::{CustomerCommand, TaskCommand};

const PAST_SHOWN: usize = 8;
const UPCOMING_SHOWN: usize = 5;
const NOTES_SHOWN: usize = 4;
const TASKS_SHOWN: usize = 12;
const CANDIDATES_SHOWN: usize = 6;
/// A cached MSX read older than this is refreshed once when the panel shows it while MSX is
/// connected.
const MSX_STALE_MINUTES: i64 = 60;

/// MSX account search of the confirmation dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum Lookup {
    #[default]
    Idle,
    Running(u64),
    Ready(tomlook::msx::dataverse::AccountLookup),
    Failed(String),
}

/// The MSX link the user chooses in the confirmation dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Link {
    /// Keep the customer's current link while editing.
    Keep,
    Local,
    /// An MSX account candidate by ID.
    Candidate(String),
}

pub(super) struct ConfirmDraft {
    /// The unconfirmed domain the dialog was opened for.
    pub(super) domain: Option<String>,
    /// All unconfirmed domains of the organization the dialog was opened for.
    pub(super) group: Vec<String>,
    pub(super) people: Vec<String>,
    /// The customer being edited.
    pub(super) editing: Option<String>,
    /// Add the domain to this existing customer instead of creating one.
    pub(super) existing: Option<String>,
    pub(super) name: String,
    name_edited: bool,
    /// Comma-separated e-mail domains.
    pub(super) domains: String,
    pub(super) kind: model::Kind,
    pub(super) link: Link,
    pub(super) lookup: Lookup,
    /// Filter of the territory list.
    pub(super) filter: String,
    pub(super) error: Option<String>,
    focus: bool,
}

pub(super) struct Research {
    pub(super) id: u64,
    pub(super) request: Box<research::Request>,
}

/// What the panel shows of one customer, computed from local caches.
pub(super) struct Overview {
    pub(super) timeline: model::Timeline,
    pub(super) notes: Vec<model::NoteDigest>,
    pub(super) tasks: Vec<tomlook::tasks::Task>,
}

/// Inputs the cached overview was built from; held so their addresses cannot be reused.
struct Cached {
    calendar: Arc<Calendar>,
    activity: Arc<tomlook::worker::Activity>,
    customer: String,
    minute: i64,
    view: Arc<Overview>,
}

#[derive(Default)]
pub(super) struct CustomersUi {
    pub(super) open: bool,
    raise: bool,
    /// The customer shown; `None` lists all customers.
    pub(super) selected: Option<String>,
    pub(super) filter: String,
    pub(super) confirm: Option<ConfirmDraft>,
    request_id: u64,
    /// Background MSX suggestions for unconfirmed organizations by organization key.
    pub(super) tips: BTreeMap<String, Lookup>,
    /// Customer and request of the running MSX read.
    pub(super) msx_running: Option<(String, u64)>,
    pub(super) msx_errors: BTreeMap<String, String>,
    /// Customers read automatically once in this session.
    msx_refreshed: BTreeSet<String>,
    /// Reads received in this session until the storage worker publishes them.
    msx_received: BTreeMap<String, model::MsxSnapshot>,
    pub(super) research: Option<Research>,
    pub(super) research_errors: BTreeMap<String, String>,
    research_received: BTreeMap<String, research::Communication>,
    /// The meeting the panel was opened from; communication research focuses on it.
    pub(super) focus: Option<String>,
    /// Customer whose removal waits for confirmation.
    remove: Option<String>,
    cached: Option<Cached>,
    /// Fixed clock for tests.
    #[cfg(test)]
    pub(super) now: Option<chrono::DateTime<Local>>,
}

enum CardAction {
    Open(String),
    Pin(String),
    Exclude,
    Reset,
    Organization(Box<OrganizationAction>),
}

/// One click on an unconfirmed organization.
enum OrganizationAction {
    /// Save it as a customer, linked to the suggested MSX account when there is one.
    Confirm(model::Organization, Option<tomlook::msx::dataverse::AccountCandidate>),
    /// Add its domains to a saved customer or partner of the same name.
    AddTo(model::Organization, String),
    Choose(model::Organization),
    Save(model::Organization, model::Kind),
}

/// What the user sees for an organization: a saved one, an MSX customer of the territory, or
/// a local name.
fn kind_note(customer: &model::Customer) -> &'static str {
    match customer.kind {
        model::Kind::Partner => "Partner, not linked to MSX",
        model::Kind::Ignored => "Not tracked; its domains are not offered",
        model::Kind::Customer => "Local customer without an MSX link",
    }
}

fn names(people: &[String]) -> String {
    let mut shown = people
        .iter()
        .take(3)
        .map(|person| person.split('<').next().unwrap_or(person).trim().to_owned())
        .collect::<Vec<_>>();
    if people.len() > 3 {
        shown.push(format!("+{}", people.len() - 3));
    }
    shown.join(", ")
}

/// A weekday, day and month; the year too when it is not the current one.
fn day_label(date: NaiveDate, language: tomlook::i18n::Language) -> String {
    let year = date.year() != Local::now().year();
    match language {
        tomlook::i18n::Language::En if year => date.format("%a %-d %b %Y").to_string(),
        tomlook::i18n::Language::En => date.format("%a %-d %b").to_string(),
        tomlook::i18n::Language::Cs => {
            let day = language.date(date, false);
            if year {
                format!("{} {}. {}", day.trim(), date.month(), date.year())
            } else {
                format!("{} {}.", day.trim(), date.month())
            }
        }
    }
}

fn split_domains(text: &str) -> Vec<String> {
    text.split([',', ';', ' ', '\n'])
        .map(str::trim)
        .filter(|domain| !domain.is_empty())
        .map(str::to_owned)
        .collect()
}

fn month_kind(kind: model::MonthKind) -> &'static str {
    match kind {
        model::MonthKind::Actual => "Completed month",
        model::MonthKind::Partial => "Current month (partial)",
        model::MonthKind::Prediction => "Model prediction",
    }
}

fn evidence(item: &tomlook::msx::dataverse::Evidence) -> String {
    use tomlook::msx::dataverse::Evidence;
    match item {
        Evidence::WebsiteDomain { domain, website } => format!("Website {website} matches {domain}"),
        Evidence::ContactDomain { domain, contacts } => format!("{contacts} MSX contacts use @{domain}"),
        Evidence::NameSimilarity { name, similarity } => {
            format!("Name {}% similar to {name}", (similarity * 100.0).round())
        }
        Evidence::DomainMatchesName { domain } => format!("Domain {domain} matches the name"),
        Evidence::Territory => "In your MSX territory".into(),
        Evidence::TopParent { account, tpid } => format!("{account} has the same TPID {tpid}"),
    }
}

/// Bars of monthly Azure consumption: filled completed months, a lighter outlined current partial
/// month and dashed model predictions.
fn acr_chart(
    ui: &mut egui::Ui,
    bars: &[model::Bar],
    unit: &str,
    language: tomlook::i18n::Language,
    accent: Color32,
) {
    if bars.is_empty() {
        return;
    }
    let width = ui.available_width().clamp(260.0, 640.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, 150.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let weak = ui.visuals().weak_text_color();
    let plot = egui::Rect::from_min_max(rect.min + Vec2::new(0.0, 16.0), rect.max - Vec2::new(0.0, 16.0));
    let maximum = bars.iter().map(|bar| bar.value).fold(0.0_f64, f64::max);
    painter.text(
        rect.left_top(),
        egui::Align2::LEFT_TOP,
        format!("{} {unit}", model::amount(maximum, language)),
        egui::FontId::proportional(10.0),
        weak,
    );
    painter.line_segment(
        [plot.left_bottom(), plot.right_bottom()],
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
    let slot = plot.width() / bars.len() as f32;
    for (index, bar) in bars.iter().enumerate() {
        let left = plot.left() + slot * index as f32;
        let top = plot.bottom() - (plot.height() - 4.0) * bar.height.clamp(0.0, 1.0);
        let shape = egui::Rect::from_min_max(
            egui::pos2(left + slot * 0.18, top.min(plot.bottom() - 1.0)),
            egui::pos2(left + slot * 0.82, plot.bottom()),
        );
        match bar.kind {
            model::MonthKind::Actual => {
                painter.rect_filled(shape, 2, accent);
            }
            model::MonthKind::Partial => {
                painter.rect_filled(shape, 2, accent.gamma_multiply(0.3));
                painter.rect_stroke(shape, 2, Stroke::new(1.5, accent), egui::StrokeKind::Inside);
            }
            model::MonthKind::Prediction => {
                let corners = [
                    shape.left_bottom(),
                    shape.left_top(),
                    shape.right_top(),
                    shape.right_bottom(),
                ];
                painter.extend(egui::Shape::dashed_line(
                    &corners,
                    Stroke::new(1.2, weak),
                    3.0,
                    2.0,
                ));
            }
        }
        let first = index == 0 || bars[index - 1].month != bar.month;
        if first && (bars.len() <= 8 || index % 2 == 0) {
            painter.text(
                egui::pos2(left + slot * 0.5, plot.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                bar.month.format("%m/%y").to_string(),
                egui::FontId::proportional(10.0),
                weak,
            );
        }
    }
    if let Some(position) = response.hover_pos() {
        let index = ((position.x - plot.left()) / slot).floor().max(0.0) as usize;
        if let Some(bar) = bars.get(index) {
            response.on_hover_text(format!(
                "{}: {} {unit} · {}",
                bar.label,
                model::amount(bar.value, language),
                language.text(month_kind(bar.kind))
            ));
        }
    }
}

/// Legend entry of the consumption chart.
fn legend(ui: &mut egui::Ui, kind: model::MonthKind, accent: Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 10.0), egui::Sense::hover());
    let painter = ui.painter();
    match kind {
        model::MonthKind::Actual => {
            painter.rect_filled(rect, 2, accent);
        }
        model::MonthKind::Partial => {
            painter.rect_filled(rect, 2, accent.gamma_multiply(0.3));
            painter.rect_stroke(rect, 2, Stroke::new(1.5, accent), egui::StrokeKind::Inside);
        }
        model::MonthKind::Prediction => {
            painter.extend(egui::Shape::dashed_line(
                &[rect.left_bottom(), rect.left_top(), rect.right_top(), rect.right_bottom()],
                Stroke::new(1.2, ui.visuals().weak_text_color()),
                3.0,
                2.0,
            ));
        }
    }
    ui.label(RichText::new(label).small());
}

impl Tomlook {
    pub(super) fn customers_now(&self) -> chrono::DateTime<Local> {
        #[cfg(test)]
        if let Some(now) = self.customers.now {
            return now;
        }
        Local::now()
    }

    /// The calendar owner, or the fixed demo account in the preview.
    pub(super) fn customer_account(&self) -> Option<String> {
        self.feedback_account()
    }

    /// The published customers of the current account.
    pub(super) fn customer_book(&self) -> Arc<model::Book> {
        let book = self.activity.customers.clone();
        if book.account.is_some() && book.account == self.customer_account() {
            book
        } else {
            Arc::new(model::Book::default())
        }
    }

    pub(super) fn customer_detector(&self) -> model::Detector {
        let own = self.own_emails();
        model::Detector::new(
            own.iter().map(String::as_str),
            self.preview.then_some(tomlook::showcase::DEMO_DOMAIN),
        )
    }

    fn msx_connected(&self) -> bool {
        self.msx_status
            .service(tomlook::msx::Resource::Dataverse)
            .phase
            == tomlook::msx::Phase::Connected
    }

    pub(super) fn open_customers(&mut self, customer: Option<String>, focus: Option<String>) {
        self.customers.open = true;
        self.customers.raise = true;
        self.customers.selected = customer;
        self.customers.focus = focus;
        self.customers.remove = None;
    }

    /// Opens a company from meeting details: its customer, or the confirmation of its domain.
    pub(super) fn open_company(&mut self, domain: &str, people: Vec<String>, event: Option<String>) {
        let book = self.customer_book();
        if let Some(customer) = book.customer_for(domain) {
            self.open_customers(Some(customer.id.clone()), event);
        } else if self.customer_detector().external(domain) {
            self.open_customer_confirmation(domain, people);
        }
    }

    fn customer_notes_ready(&self, event: &Event, now: chrono::DateTime<chrono::FixedOffset>) -> bool {
        if self.preview {
            event.title == tomlook::showcase::ENDED_MEETING && event.end <= now
        } else {
            self.calendar_owner.as_deref().is_some_and(|account| {
                self.activity
                    .recaps
                    .record(account, event)
                    .is_some_and(|record| record.payload.is_some())
            })
        }
    }

    fn customer_notes_payload(&self, event: &Event) -> Option<serde_json::Value> {
        if self.preview {
            tomlook::showcase::notes(event).ok()
        } else {
            let account = self.calendar_owner.as_deref()?;
            self.activity.recaps.record(account, event)?.payload.clone()
        }
    }

    fn customer_insights_ready(&self, event: &Event) -> bool {
        if self.preview {
            event.extra.get("synthetic").and_then(serde_json::Value::as_bool) == Some(true)
                && event.briefing_status == "ready"
        } else {
            self.activity
                .queue
                .jobs
                .get(&event.id)
                .is_some_and(|job| job.state == tomlook::scheduler::State::Completed)
        }
    }

    /// The customer's timeline, notes and tasks, recomputed at most once a minute or when the
    /// calendar or the published local data change.
    pub(super) fn customer_overview(&mut self, customer: &str) -> Arc<Overview> {
        let now = self.customers_now();
        let minute = now.timestamp().div_euclid(60);
        if let Some(cached) = &self.customers.cached
            && Arc::ptr_eq(&cached.calendar, &self.calendar)
            && Arc::ptr_eq(&cached.activity, &self.activity)
            && cached.customer == customer
            && cached.minute == minute
        {
            return cached.view.clone();
        }
        let book = self.customer_book();
        let detector = self.customer_detector();
        let account = self.customer_account().unwrap_or_default();
        let fixed = now.fixed_offset();
        let timeline = model::timeline(
            &self.calendar,
            &book,
            &detector,
            &account,
            customer,
            fixed,
            &|event| self.customer_notes_ready(event, fixed),
            &|event| self.customer_insights_ready(event),
        );
        let payloads = timeline
            .past
            .iter()
            .filter(|entry| entry.notes)
            .take(NOTES_SHOWN * 2)
            .filter_map(|entry| {
                let event = &self.calendar.events[self.calendar.event_index(&entry.event_id)?];
                Some((entry.event_id.clone(), self.customer_notes_payload(event)?))
            })
            .collect::<BTreeMap<_, _>>();
        let notes = model::notes(
            &timeline,
            |id| payloads.get(id),
            self.preferences.language,
            NOTES_SHOWN,
        );
        let tasks = model::tasks(
            &self.activity.tasks,
            &account,
            &self.calendar,
            &timeline,
            chrono::Utc::now(),
            TASKS_SHOWN,
        )
        .into_iter()
        .cloned()
        .collect();
        let view = Arc::new(Overview {
            timeline,
            notes,
            tasks,
        });
        self.customers.cached = Some(Cached {
            calendar: self.calendar.clone(),
            activity: self.activity.clone(),
            customer: customer.into(),
            minute,
            view: view.clone(),
        });
        view
    }

    /// The newest MSX read of a customer's linked account: received now or saved earlier.
    fn customer_snapshot(&self, book: &model::Book, customer: &model::Customer) -> Option<model::MsxSnapshot> {
        let account = customer.msx.as_ref()?;
        let received = self
            .customers
            .msx_received
            .get(&customer.id)
            .filter(|snapshot| snapshot.account_id.eq_ignore_ascii_case(&account.id));
        match (received, book.snapshot(customer)) {
            (Some(received), Some(saved)) if saved.retrieved_at > received.retrieved_at => Some(saved.clone()),
            (Some(received), _) => Some(received.clone()),
            (None, saved) => saved.cloned(),
        }
    }

    fn customer_communication(&self, book: &model::Book, customer: &str) -> Option<research::Communication> {
        let received = self.customers.research_received.get(customer);
        match (received, book.communications.get(customer)) {
            (Some(received), Some(saved)) if saved.created_at > received.created_at => Some(saved.clone()),
            (Some(received), _) => Some(received.clone()),
            (None, saved) => saved.cloned(),
        }
    }

    /// Why MSX cannot be read for this customer now; `None` when a read can start.
    fn customer_msx_blocker(&self, customer: &model::Customer) -> Option<&'static str> {
        if self.preview {
            Some(tomlook::msx::DEMO)
        } else if !self.preferences.msx_enabled {
            Some(tomlook::msx::DISABLED)
        } else if customer.msx.is_none() {
            Some("Link an MSX customer first.")
        } else if !self.msx_connected() {
            Some("Sign in to MSX first.")
        } else if self.ai.is_none() {
            Some("The AI worker is unavailable. Restart Tomlook.")
        } else {
            None
        }
    }

    fn refresh_customer_msx(&mut self, customer: &model::Customer) {
        let Some(account) = customer.msx.clone() else {
            return;
        };
        self.customers.msx_refreshed.insert(customer.id.clone());
        self.customers.request_id += 1;
        let id = self.customers.request_id;
        if self.ai_command(ai::Command::CustomerMsx {
            id,
            customer: customer.id.clone(),
            account,
        }) {
            self.customers.msx_running = Some((customer.id.clone(), id));
            self.customers.msx_errors.remove(&customer.id);
        } else {
            let error = self
                .ai_error
                .take()
                .unwrap_or_else(|| "The AI worker is unavailable. Restart Tomlook.".into());
            self.customers.msx_errors.insert(customer.id.clone(), error);
        }
    }

    pub(super) fn receive_customer_msx(
        &mut self,
        id: u64,
        customer: String,
        result: Result<model::MsxSnapshot, String>,
    ) {
        if self
            .customers
            .msx_running
            .take_if(|(_, running)| *running == id)
            .is_none()
        {
            return;
        }
        match result.and_then(|snapshot| snapshot.validate().map(|()| snapshot)) {
            Ok(snapshot) => {
                if let Some(account) = self.customer_account() {
                    self.command(Command::Customer(CustomerCommand::SaveMsx {
                        account,
                        customer: customer.clone(),
                        snapshot: Box::new(snapshot.clone()),
                    }));
                }
                self.customers.msx_errors.remove(&customer);
                self.customers.msx_received.insert(customer, snapshot);
            }
            Err(error) => {
                self.customers.msx_errors.insert(customer, error);
            }
        }
    }

    /// Why communication research cannot start now; `None` when it can.
    fn customer_research_blocker(&self, customer: &str) -> Option<&'static str> {
        if self.preview {
            Some("Offline demo never sends AI requests.")
        } else if self.ai.is_none() || !self.ai_ready || !self.workiq_ready {
            Some("Connect GitHub Copilot and WorkIQ first.")
        } else if self.customer_account().is_none() {
            Some("Connect WorkIQ to your calendar account first.")
        } else if self
            .customers
            .research
            .as_ref()
            .is_some_and(|running| running.request.customer != customer)
        {
            Some("Another communication search is running.")
        } else {
            None
        }
    }

    fn request_customer_research(&mut self, customer: &model::Customer, view: &Overview) {
        let Some(account) = self.customer_account() else {
            return;
        };
        let focus = self
            .customers
            .focus
            .as_deref()
            .and_then(|id| self.calendar.event_index(id))
            .or_else(|| {
                view.timeline
                    .next
                    .as_ref()
                    .or(view.timeline.past.first())
                    .and_then(|entry| self.calendar.event_index(&entry.event_id))
            })
            .map(|index| self.calendar.events[index].clone());
        self.customers.request_id += 1;
        let id = self.customers.request_id;
        let today = self.customers_now().date_naive();
        match research::Request::new(
            id,
            &account,
            customer,
            &view.timeline.people,
            focus.as_ref(),
            self.preferences.language,
            today,
        ) {
            Ok(request) => {
                if self.ai_command(ai::Command::CustomerResearch(Box::new(request.clone()))) {
                    self.customers.research = Some(Research {
                        id,
                        request: Box::new(request),
                    });
                    self.customers.research_errors.remove(&customer.id);
                } else {
                    let error = self
                        .ai_error
                        .take()
                        .unwrap_or_else(|| "The AI worker is unavailable. Restart Tomlook.".into());
                    self.customers.research_errors.insert(customer.id.clone(), error);
                }
            }
            Err(error) => {
                self.customers.research_errors.insert(customer.id.clone(), error);
            }
        }
    }

    pub(super) fn receive_customer_research(
        &mut self,
        id: u64,
        result: Result<research::Communication, String>,
    ) {
        let Some(running) = self.customers.research.take_if(|running| running.id == id) else {
            return;
        };
        let customer = running.request.customer.clone();
        let saved = result.and_then(|saved| {
            saved.validate()?;
            if saved.customer != customer || saved.account != running.request.account {
                return Err("The communication summary belongs to a different customer; nothing was saved.".into());
            }
            Ok(saved)
        });
        match saved {
            Ok(saved) => {
                self.command(Command::Customer(CustomerCommand::SaveCommunication(Box::new(saved.clone()))));
                self.customers.research_errors.remove(&customer);
                self.customers.research_received.insert(customer, saved);
            }
            Err(error) => {
                self.customers.research_errors.insert(customer, error);
            }
        }
    }

    pub(super) fn stop_customer_research(&mut self) {
        if let Some(running) = self.customers.research.take() {
            self.customers.research_errors.insert(
                running.request.customer.clone(),
                "AI stopped without a completed answer; provider outcome may be unknown".into(),
            );
        }
    }

    fn cancel_customer_research(&mut self) {
        if let Some(running) = self.customers.research.take() {
            self.ai_command(ai::Command::Cancel(running.id | ai::CUSTOMER_BIT));
            self.customers.research_errors.insert(
                running.request.customer.clone(),
                "Communication search cancelled; nothing was saved.".into(),
            );
        }
    }

    /// A customer's cached company icon or its initial.
    fn customer_icon(&mut self, ui: &mut egui::Ui, customer: &model::Customer) {
        let domain = customer.icon_domain().map(str::to_owned);
        if let Some(image) = domain
            .as_ref()
            .and_then(|domain| self.companies.get(domain))
            .and_then(|icon| icon.image.as_ref())
        {
            egui::Frame::new()
                .fill(Color32::WHITE)
                .corner_radius(4)
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.add(egui::Image::new((image.id(), Vec2::splat(18.0))));
                });
        } else {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
            let accent = self.accent();
            ui.painter().circle_filled(rect.center(), 11.0, accent.gamma_multiply(0.25));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                customer.name.chars().next().map(|first| first.to_uppercase().to_string()).unwrap_or_default(),
                egui::FontId::proportional(12.0),
                ui.visuals().strong_text_color(),
            );
            if let Some(domain) = domain
                && !self.companies_busy
                && !self.companies.contains_key(&domain)
                && self.companies.len() < 128
                && self.ai_command(ai::Command::Companies(vec![domain]))
            {
                self.companies_busy = true;
            }
        }
    }

    // ---------- meeting details ----------

    /// The customers of a meeting: confirmed ones open the customer panel, partners are shown as
    /// such and unconfirmed external domains are grouped by organization with an MSX suggestion
    /// and one-click choices. Nothing is associated without the user.
    pub(super) fn customer_card(&mut self, ui: &mut egui::Ui, event: &Event) {
        let Some(account) = self.customer_account() else {
            return;
        };
        let book = self.customer_book();
        let detection = model::detect(&book, &self.customer_detector(), &account, event);
        if detection.parties.is_empty() && !detection.excluded {
            return;
        }
        let language = self.preferences.language;
        let accent = self.accent();
        let chosen = book.override_for(&account, event).is_some();
        let several = detection.customers().count() > 1;
        let organizations = model::organizations(&book, &detection.parties);
        let mut action = None;
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.strong(language.text("Customers in this meeting"));
                if detection.excluded {
                    ui.weak(language.text("Marked as not a customer meeting."));
                }
                for party in &detection.parties {
                    match party {
                        Party::Customer { id, people, pinned, .. } => {
                            let Some(customer) = book.customers.get(id) else {
                                continue;
                            };
                            ui.push_id(("meeting-customer", id), |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    self.customer_icon(ui, customer);
                                    if ui
                                        .add(egui::Button::new(RichText::new(&customer.name).strong()).frame(false))
                                        .on_hover_text(language.text("Open customer"))
                                        .clicked()
                                    {
                                        action = Some(CardAction::Open(id.clone()));
                                    }
                                    if *pinned {
                                        chip(ui, &language.text("Set for this meeting"), accent);
                                    }
                                    if customer.sample {
                                        chip(ui, &language.text("Sample data"), ui.visuals().weak_text_color());
                                    }
                                });
                                match &customer.msx {
                                    Some(msx) => ui.weak(language.text(&format!("MSX: {}", msx.name))),
                                    None => ui.weak(language.text(kind_note(customer))),
                                };
                                if !people.is_empty() {
                                    ui.label(RichText::new(names(people)).small());
                                }
                                ui.horizontal_wrapped(|ui| {
                                    if ui.small_button(language.text("Customer timeline")).clicked() {
                                        action = Some(CardAction::Open(id.clone()));
                                    }
                                    if several
                                        && !*pinned
                                        && ui.small_button(language.text("Only this customer")).clicked()
                                    {
                                        action = Some(CardAction::Pin(id.clone()));
                                    }
                                });
                            });
                        }
                        Party::Partner { id, people, .. } => {
                            let Some(partner) = book.customers.get(id) else {
                                continue;
                            };
                            ui.push_id(("meeting-partner", id), |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    self.customer_icon(ui, partner);
                                    if ui
                                        .add(egui::Button::new(RichText::new(&partner.name).strong()).frame(false))
                                        .on_hover_text(language.text("Open customer"))
                                        .clicked()
                                    {
                                        action = Some(CardAction::Open(id.clone()));
                                    }
                                    chip(ui, "Partner", ui.visuals().weak_text_color());
                                });
                                if !people.is_empty() {
                                    ui.label(RichText::new(names(people)).small().weak());
                                }
                            });
                        }
                        Party::Unconfirmed { .. } => {}
                    }
                }
                for organization in &organizations {
                    if let Some(chosen) = self.organization_row(ui, &book, organization, None) {
                        action = Some(CardAction::Organization(Box::new(chosen)));
                    }
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if chosen {
                        if ui.small_button(language.text("Detect automatically")).clicked() {
                            action = Some(CardAction::Reset);
                        }
                    } else if ui
                        .small_button(language.text("Not a customer meeting"))
                        .on_hover_text(language.text("Only this meeting; other meetings with these people are unchanged."))
                        .clicked()
                    {
                        action = Some(CardAction::Exclude);
                    }
                });
            });
        let now = chrono::Utc::now();
        match action {
            Some(CardAction::Open(id)) => self.open_customers(Some(id), Some(event.id.clone())),
            Some(CardAction::Organization(chosen)) => self.apply_organization(*chosen),
            Some(CardAction::Pin(id)) => self.command(Command::Customer(CustomerCommand::Choose(Box::new(
                model::Override::new(&account, event, Some(id), now),
            )))),
            Some(CardAction::Exclude) => self.command(Command::Customer(CustomerCommand::Choose(Box::new(
                model::Override::new(&account, event, None, now),
            )))),
            Some(CardAction::Reset) => {
                if let Some(choice) = book.override_for(&account, event) {
                    self.command(Command::Customer(CustomerCommand::Reset {
                        account: account.clone(),
                        occurrence: choice.occurrence.clone(),
                    }));
                }
            }
            None => {}
        }
        ui.add_space(6.0);
    }

    // ---------- unconfirmed organizations ----------

    /// Starts one background MSX suggestion at a time for an organization not looked up yet.
    fn request_tip(&mut self, organization: &model::Organization) {
        if self.preview
            || !self.preferences.msx_enabled
            || !self.msx_connected()
            || self.customers.tips.contains_key(&organization.key())
            || self.customers.tips.values().any(|tip| matches!(tip, Lookup::Running(_)))
        {
            return;
        }
        self.customers.request_id += 1;
        let id = self.customers.request_id;
        if self.ai_command(ai::Command::CustomerTip {
            id,
            domains: organization.domains.clone(),
            names: vec![organization.name.clone()],
        }) {
            self.customers.tips.insert(organization.key(), Lookup::Running(id));
        } else {
            self.ai_error = None;
        }
    }

    /// One unconfirmed organization with its suggestion and one-click choices.
    fn organization_row(
        &mut self,
        ui: &mut egui::Ui,
        book: &model::Book,
        organization: &model::Organization,
        meetings: Option<usize>,
    ) -> Option<OrganizationAction> {
        use tomlook::msx::dataverse::Selection;
        let language = self.preferences.language;
        let accent = self.accent();
        self.request_tip(organization);
        let tip = self.customers.tips.get(&organization.key()).cloned().unwrap_or_default();
        let related = organization.related.as_ref().and_then(|id| book.customers.get(id));
        let suggestion = match &tip {
            Lookup::Ready(lookup) => match &lookup.selection {
                Selection::Likely { account_id } => lookup.candidates.iter().find(|candidate| &candidate.id == account_id),
                _ => None,
            },
            _ => None,
        };
        let mut action = None;
        ui.push_id(("unconfirmed-organization", organization.key()), |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(language.text(&format!("Unconfirmed: {}", organization.name))).strong());
                ui.weak(organization.domains.join(", "));
                if let Some(meetings) = meetings {
                    ui.weak(language.text(&format!("Meetings: {meetings}")));
                }
            });
            if !organization.people.is_empty() {
                ui.label(RichText::new(names(&organization.people)).small().weak());
            }
            match (&tip, suggestion, related) {
                (_, _, Some(related)) => {
                    ui.weak(language.text(&format!("Probably another domain of {}", related.name)));
                }
                (_, Some(candidate), None) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(language.text(&format!("Suggestion: MSX customer {}", candidate.name)));
                        if let Some(tpid) = &candidate.tpid {
                            ui.weak(format!("TPID {tpid}"));
                        }
                        if candidate.evidence.contains(&tomlook::msx::dataverse::Evidence::Territory) {
                            chip(ui, &language.text("Your territory"), accent);
                        }
                    })
                    .response
                    .on_hover_text(
                        candidate
                            .evidence
                            .iter()
                            .map(|item| language.text(&evidence(item)).into_owned())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                (Lookup::Running(_), ..) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(language.text("Looking for the customer in MSX..."));
                    });
                }
                (Lookup::Ready(lookup), None, None) if !lookup.candidates.is_empty() => {
                    ui.weak(language.text("Several MSX customers are possible; choose one."));
                }
                (Lookup::Ready(_), None, None) => {
                    ui.weak(language.text("Not found in MSX: a partner, or a customer to choose yourself."));
                }
                _ => {}
            }
            ui.horizontal_wrapped(|ui| {
                if let Some(related) = related {
                    if ui
                        .button(RichText::new(language.text(&format!("Add to {}", related.name))).strong())
                        .clicked()
                    {
                        action = Some(OrganizationAction::AddTo(organization.clone(), related.id.clone()));
                    }
                } else if let Some(candidate) = suggestion {
                    if ui
                        .button(RichText::new(language.text(&format!("Confirm {} → {}", organization.name, candidate.name))).strong())
                        .on_hover_text(language.text("Saves the customer with this MSX link; you can change it later."))
                        .clicked()
                    {
                        action = Some(OrganizationAction::Confirm(organization.clone(), Some(candidate.clone())));
                    }
                } else if !matches!(&tip, Lookup::Running(_))
                    && !matches!(&tip, Lookup::Ready(lookup) if !lookup.candidates.is_empty())
                    && ui
                        .button(language.text(&format!("Confirm {}", organization.name)))
                        .on_hover_text(language.text("Saves a local customer; link MSX later."))
                        .clicked()
                {
                    action = Some(OrganizationAction::Confirm(organization.clone(), None));
                }
                if ui
                    .small_button(language.text("Choose customer..."))
                    .on_hover_text(language.text("Choose an MSX customer of your territory, search MSX or keep a local one; nothing is linked without you."))
                    .clicked()
                {
                    action = Some(OrganizationAction::Choose(organization.clone()));
                }
                if ui
                    .small_button(language.text("It's a partner"))
                    .on_hover_text(language.text("Remember it as a partner: no MSX link, never offered as a customer again."))
                    .clicked()
                {
                    action = Some(OrganizationAction::Save(organization.clone(), model::Kind::Partner));
                }
                if ui
                    .small_button(language.text("Don't track"))
                    .on_hover_text(language.text("These domains are never offered again; undo it in Customers."))
                    .clicked()
                {
                    action = Some(OrganizationAction::Save(organization.clone(), model::Kind::Ignored));
                }
            });
        });
        action
    }

    fn apply_organization(&mut self, action: OrganizationAction) {
        let Some(account) = self.customer_account() else {
            return;
        };
        let now = chrono::Utc::now();
        let saved = match action {
            OrganizationAction::Choose(organization) => {
                self.open_organization_confirmation(&organization);
                return;
            }
            OrganizationAction::AddTo(organization, id) => {
                let Some(mut customer) = self.customer_book().customers.get(&id).cloned() else {
                    return;
                };
                for domain in organization.domains {
                    if !customer.domains.contains(&domain) {
                        customer.domains.push(domain);
                    }
                }
                customer.updated_at = now.max(customer.created_at);
                Ok(customer)
            }
            OrganizationAction::Confirm(organization, candidate) => model::Customer::new(
                &account,
                &organization.name,
                &organization.domains,
                candidate.map(|candidate| model::MsxAccount {
                    id: candidate.id,
                    name: candidate.name,
                    tpid: candidate.tpid,
                    confidence: Some(candidate.confidence.clamp(0.0, 1.0)),
                }),
                now,
            ),
            OrganizationAction::Save(organization, kind) => {
                model::Customer::new(&account, &organization.name, &organization.domains, None, now).map(|mut customer| {
                    customer.kind = kind;
                    customer
                })
            }
        };
        match saved.and_then(|customer| customer.validate().map(|()| customer)) {
            Ok(customer) => {
                self.command(Command::Customer(CustomerCommand::Save(Box::new(customer))));
                self.customers.cached = None;
            }
            Err(error) => self.ai_error = Some(error),
        }
    }

    // ---------- confirmation ----------

    pub(super) fn open_customer_confirmation(&mut self, domain: &str, people: Vec<String>) {
        let Some(domain) = model::normalize_domain(domain) else {
            return;
        };
        self.open_organization_confirmation(&model::Organization {
            name: model::suggested_name(&domain),
            domains: vec![domain],
            people,
            related: None,
        });
    }

    fn open_organization_confirmation(&mut self, organization: &model::Organization) {
        let Some(domain) = organization.domains.first().cloned() else {
            return;
        };
        let tip = self.customers.tips.get(&organization.key()).cloned().unwrap_or_default();
        let mut draft = ConfirmDraft {
            name: organization.name.clone(),
            name_edited: organization.name != model::suggested_name(&domain),
            domains: organization.domains.join(", "),
            domain: Some(domain),
            group: organization.domains.clone(),
            people: organization.people.clone(),
            editing: None,
            existing: organization.related.clone(),
            kind: model::Kind::Customer,
            link: Link::Local,
            lookup: Lookup::Idle,
            filter: String::new(),
            error: None,
            focus: true,
        };
        if let Lookup::Ready(lookup) = tip {
            self.customers.request_id += 1;
            draft.lookup = Lookup::Running(self.customers.request_id);
            self.customers.confirm = Some(draft);
            self.receive_customer_accounts(self.customers.request_id, Ok(lookup));
        } else {
            self.customers.confirm = Some(draft);
            self.search_customer_accounts();
        }
    }

    pub(super) fn open_customer_editor(&mut self, customer: &model::Customer) {
        self.customers.confirm = Some(ConfirmDraft {
            domain: None,
            group: Vec::new(),
            people: Vec::new(),
            editing: Some(customer.id.clone()),
            existing: None,
            name: customer.name.clone(),
            name_edited: true,
            domains: customer.domains.join(", "),
            kind: customer.kind,
            link: if customer.msx.is_some() { Link::Keep } else { Link::Local },
            lookup: Lookup::Idle,
            filter: String::new(),
            error: None,
            focus: true,
        });
        self.search_customer_accounts();
    }

    /// Searches MSX for account candidates, only while MSX is already connected.
    fn search_customer_accounts(&mut self) {
        if self.preview || !self.preferences.msx_enabled || !self.msx_connected() {
            return;
        }
        let Some(draft) = &self.customers.confirm else {
            return;
        };
        let domains = split_domains(&draft.domains);
        let names = vec![draft.name.clone()];
        self.customers.request_id += 1;
        let id = self.customers.request_id;
        let admitted = self.ai_command(ai::Command::CustomerAccounts { id, domains, names });
        let error = (!admitted).then(|| {
            self.ai_error
                .take()
                .unwrap_or_else(|| "The AI worker is unavailable. Restart Tomlook.".into())
        });
        if let Some(draft) = &mut self.customers.confirm {
            draft.lookup = match error {
                None => Lookup::Running(id),
                Some(error) => Lookup::Failed(error),
            };
        }
    }

    pub(super) fn receive_customer_accounts(
        &mut self,
        id: u64,
        result: Result<tomlook::msx::dataverse::AccountLookup, String>,
    ) {
        if let Some(tip) = self
            .customers
            .tips
            .values_mut()
            .find(|tip| **tip == Lookup::Running(id))
        {
            *tip = match result {
                Ok(lookup) => Lookup::Ready(lookup),
                Err(error) => Lookup::Failed(error),
            };
            return;
        }
        let Some(draft) = &mut self.customers.confirm else {
            return;
        };
        if draft.lookup != Lookup::Running(id) {
            return;
        }
        match result {
            Ok(lookup) => {
                // A clearly leading candidate is preselected and shown with its confidence and
                // evidence; it is linked only after the user clicks Confirm.
                if let tomlook::msx::dataverse::Selection::Likely { account_id } = &lookup.selection
                    && draft.editing.is_none()
                    && draft.link == Link::Local
                {
                    draft.link = Link::Candidate(account_id.clone());
                    if !draft.name_edited
                        && let Some(candidate) = lookup.candidates.iter().find(|candidate| &candidate.id == account_id)
                    {
                        draft.name = candidate.name.clone();
                    }
                }
                draft.lookup = Lookup::Ready(lookup);
            }
            Err(error) => draft.lookup = Lookup::Failed(error),
        }
    }

    fn submit_customer(&mut self, draft: &ConfirmDraft) -> Result<(), String> {
        let account = self
            .customer_account()
            .ok_or("Connect WorkIQ to your calendar account first.")?;
        let book = self.customer_book();
        let now = chrono::Utc::now();
        let link = |current: Option<model::MsxAccount>| -> Result<Option<model::MsxAccount>, String> {
            Ok(match &draft.link {
                Link::Keep => current,
                Link::Local => None,
                Link::Candidate(id) => {
                    let Lookup::Ready(lookup) = &draft.lookup else {
                        return Err("Search MSX again before linking a customer".into());
                    };
                    let candidate = lookup
                        .candidates
                        .iter()
                        .chain(&lookup.territory)
                        .find(|candidate| &candidate.id == id)
                        .ok_or("Search MSX again before linking a customer")?;
                    Some(model::MsxAccount {
                        id: candidate.id.clone(),
                        name: candidate.name.clone(),
                        tpid: candidate.tpid.clone(),
                        confidence: Some(candidate.confidence.clamp(0.0, 1.0)),
                    })
                }
            })
        };
        let customer = if let Some(existing) = &draft.existing {
            let mut customer = book
                .customers
                .get(existing)
                .cloned()
                .ok_or("This customer no longer exists")?;
            for domain in draft.domain.iter().chain(&draft.group) {
                if !customer.domains.contains(domain) {
                    customer.domains.push(domain.clone());
                }
            }
            customer.confirmed = true;
            customer.updated_at = now.max(customer.created_at);
            customer
        } else if let Some(id) = &draft.editing {
            let mut customer = book
                .customers
                .get(id)
                .cloned()
                .ok_or("This customer no longer exists")?;
            customer.name = draft.name.split_whitespace().collect::<Vec<_>>().join(" ");
            customer.domains = model::normalize_domains(&split_domains(&draft.domains))?;
            customer.kind = draft.kind;
            customer.msx = if draft.kind.is_customer() { link(customer.msx.clone())? } else { None };
            customer.confirmed = true;
            customer.updated_at = now.max(customer.created_at);
            customer
        } else {
            let mut customer = model::Customer::new(
                &account,
                &draft.name,
                &split_domains(&draft.domains),
                if draft.kind.is_customer() { link(None)? } else { None },
                now,
            )?;
            customer.kind = draft.kind;
            customer
        };
        customer.validate()?;
        if customer.domains.is_empty() {
            return Err("Enter e-mail domains such as contoso.com".into());
        }
        self.command(Command::Customer(CustomerCommand::Save(Box::new(customer))));
        self.customers.cached = None;
        Ok(())
    }

    pub(super) fn confirm_customer_dialog(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.customers.confirm.take() else {
            return;
        };
        let language = self.preferences.language;
        let accent = self.accent();
        let book = self.customer_book();
        let connected = self.msx_connected() && self.preferences.msx_enabled && !self.preview;
        let current = draft
            .editing
            .as_ref()
            .and_then(|id| book.customers.get(id))
            .and_then(|customer| customer.msx.clone());
        let (mut open, mut submit, mut cancel, mut search, mut open_msx) = (true, false, false, false, false);
        egui::Window::new(language.text(if draft.editing.is_some() { "Edit customer" } else { "Confirm customer" }))
            .id(Id::new("customer-confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(540.0)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 90.0])
            .show(context, |ui| {
                if draft.group.len() > 1 {
                    ui.label(format!("{}: {}", language.text("E-mail domains"), draft.group.join(", ")));
                } else if let Some(domain) = &draft.domain {
                    ui.label(format!("{}: {domain}", language.text("E-mail domain")));
                }
                if !draft.people.is_empty() {
                    ui.weak(format!("{}: {}", language.text("People in this meeting"), names(&draft.people)));
                }
                let others = book.sorted().into_iter().filter(|customer| Some(&customer.id) != draft.editing.as_ref()).collect::<Vec<_>>();
                if draft.domain.is_some() && !others.is_empty() {
                    egui::ComboBox::from_label(language.text("Add to customer"))
                        .selected_text(
                            draft
                                .existing
                                .as_ref()
                                .and_then(|id| book.customers.get(id))
                                .map(|customer| customer.name.clone())
                                .unwrap_or_else(|| language.text("New customer").into_owned()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.existing, None, language.text("New customer"));
                            for customer in others {
                                ui.selectable_value(&mut draft.existing, Some(customer.id.clone()), &customer.name);
                            }
                        });
                }
                if draft.existing.is_none() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.radio_value(&mut draft.kind, model::Kind::Customer, language.text("Customer organization"));
                        ui.radio_value(&mut draft.kind, model::Kind::Partner, language.text("Partner organization"))
                            .on_hover_text(language.text("Remember it as a partner: no MSX link, never offered as a customer again."));
                        ui.radio_value(&mut draft.kind, model::Kind::Ignored, language.text("Don't track"))
                            .on_hover_text(language.text("These domains are never offered again; undo it in Customers."));
                    });
                    ui.add_space(4.0);
                    ui.label(language.text("Customer name"));
                    let name = ui.add(
                        egui::TextEdit::singleline(&mut draft.name)
                            .id(Id::new("customer-name"))
                            .desired_width(360.0),
                    );
                    if std::mem::take(&mut draft.focus) {
                        name.request_focus();
                    }
                    if name.changed() {
                        draft.name_edited = true;
                    }
                    ui.label(language.text("E-mail domains"));
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.domains)
                            .id(Id::new("customer-domains"))
                            .hint_text("contoso.com, contoso.cz")
                            .desired_width(360.0),
                    );
                    ui.add_space(6.0);
                }
                if draft.existing.is_none() && draft.kind.is_customer() {
                    ui.strong(language.text("MSX customer"));
                    if let Some(msx) = &current {
                        ui.radio_value(&mut draft.link, Link::Keep, language.text(&format!("Keep the current MSX link: {}", msx.name)));
                    }
                    ui.radio_value(&mut draft.link, Link::Local, language.text("Local customer only (no MSX link)"));
                    match &draft.lookup {
                        Lookup::Idle if self.preview => {
                            ui.weak(language.text(tomlook::msx::DEMO));
                        }
                        Lookup::Idle if !connected => {
                            ui.weak(language.text("MSX is not connected; you can confirm a local customer now and link MSX later."));
                            if self.preferences.msx_enabled && ui.button(language.text("Open MSX connection")).clicked() {
                                open_msx = true;
                            }
                        }
                        Lookup::Idle => {
                            if ui.button(language.text("Search MSX")).clicked() {
                                search = true;
                            }
                        }
                        Lookup::Running(_) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.weak(language.text("Searching MSX for matching customers..."));
                            });
                        }
                        Lookup::Failed(error) => {
                            ui.colored_label(ui.visuals().warn_fg_color, language.text(&format!("MSX search failed: {error}")));
                            if connected && ui.button(language.text("Search MSX again")).clicked() {
                                search = true;
                            }
                        }
                        Lookup::Ready(lookup) => {
                            let likely = match &lookup.selection {
                                tomlook::msx::dataverse::Selection::Likely { account_id } => Some(account_id.clone()),
                                _ => None,
                            };
                            if lookup.candidates.is_empty() {
                                ui.weak(language.text("No MSX candidates found. Confirm a local customer or search again later."));
                            } else if likely.is_none() {
                                ui.weak(language.text("Several candidates are possible; choose the right one."));
                            }
                            let mut chosen = None;
                            for candidate in lookup.candidates.iter().take(CANDIDATES_SHOWN) {
                                ui.push_id(("msx-candidate", &candidate.id), |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.radio(draft.link == Link::Candidate(candidate.id.clone()), &candidate.name).clicked() {
                                            chosen = Some(candidate.clone());
                                        }
                                        if likely.as_ref() == Some(&candidate.id) {
                                            chip(ui, &language.text("Likely match"), accent);
                                        }
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        ui.add_space(24.0);
                                        ui.weak(language.text(&format!("Confidence {}%", (candidate.confidence * 100.0).round())));
                                        if let Some(tpid) = &candidate.tpid {
                                            ui.weak(format!("TPID {tpid}"));
                                        }
                                        if let Some(owner) = candidate.owner.as_ref().and_then(|owner| owner.name.as_deref()) {
                                            ui.weak(language.text(&format!("Owner: {owner}")));
                                        }
                                    });
                                    for item in &candidate.evidence {
                                        ui.horizontal(|ui| {
                                            ui.add_space(24.0);
                                            ui.label(RichText::new(language.text(&evidence(item))).small());
                                        });
                                    }
                                });
                            }
                            if let Some(candidate) = chosen {
                                draft.link = Link::Candidate(candidate.id.clone());
                                if !draft.name_edited {
                                    draft.name = candidate.name.clone();
                                }
                            }
                            if !lookup.territory.is_empty() {
                                ui.add_space(6.0);
                                ui.strong(language.text("Your MSX territory"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut draft.filter)
                                        .id(Id::new("customer-territory-filter"))
                                        .hint_text(language.text("Filter customers..."))
                                        .desired_width(260.0),
                                );
                                let query = draft.filter.trim().to_lowercase();
                                let mut picked = None;
                                egui::ScrollArea::vertical()
                                    .id_salt("customer-territory")
                                    .max_height(220.0)
                                    .show(ui, |ui| {
                                        for account in lookup.territory.iter().filter(|account| {
                                            query.is_empty() || account.name.to_lowercase().contains(&query)
                                        }) {
                                            ui.push_id(("msx-territory", &account.id), |ui| {
                                                ui.horizontal_wrapped(|ui| {
                                                    if ui.radio(draft.link == Link::Candidate(account.id.clone()), &account.name).clicked() {
                                                        picked = Some(account.id.clone());
                                                    }
                                                    if let Some(tpid) = &account.tpid {
                                                        ui.weak(format!("TPID {tpid}"));
                                                    }
                                                });
                                            });
                                        }
                                    });
                                if let Some(id) = picked {
                                    draft.link = Link::Candidate(id);
                                }
                            }
                            if connected && ui.button(language.text("Search MSX again")).clicked() {
                                search = true;
                            }
                        }
                    }
                    ui.weak(language.text("Inferred matches are never linked without your confirmation."));
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(ui.visuals().error_fg_color, language.text(error));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new(language.text(if draft.editing.is_some() { "Save" } else { "Confirm" })).strong())
                        .clicked()
                    {
                        submit = true;
                    }
                    if ui.button(language.text("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if open_msx {
            self.connection_panel = Some(ConnectionPanel::Msx);
        }
        if submit {
            match self.submit_customer(&draft) {
                Ok(()) => return,
                Err(error) => draft.error = Some(error),
            }
        }
        if cancel || !open {
            return;
        }
        self.customers.confirm = Some(draft);
        if search {
            self.search_customer_accounts();
        }
    }

    // ---------- panel ----------

    pub(super) fn customers_window(&mut self, context: &egui::Context) {
        self.confirm_customer_dialog(context);
        if !self.customers.open {
            return;
        }
        let language = self.preferences.language;
        let mut open = true;
        let shown = egui::Window::new(language.text(if self.customers.selected.is_some() { "Customer timeline" } else { "Customers" }))
            .id(Id::new("customers-window"))
            .open(&mut open)
            .default_pos(egui::pos2(NAVIGATION_WIDTH + 60.0, 70.0))
            .default_size(Vec2::new(1040.0, 820.0))
            .min_width(440.0)
            .resizable(true)
            .show(context, |ui| self.customers_panel(ui));
        if std::mem::take(&mut self.customers.raise)
            && let Some(shown) = shown
        {
            context.move_to_top(shown.response.layer_id);
        }
        if !open {
            self.customers.open = false;
        }
    }

    pub(super) fn customers_panel(&mut self, ui: &mut egui::Ui) {
        let book = self.customer_book();
        match self
            .customers
            .selected
            .as_ref()
            .and_then(|id| book.customers.get(id))
            .cloned()
        {
            Some(customer) => self.customer_detail(ui, &book, customer),
            None => {
                self.customers.selected = None;
                self.customer_list(ui, &book);
            }
        }
    }

    /// Organizations of recent and upcoming meetings whose domains no saved organization covers,
    /// with the number of meetings each attended.
    fn unconfirmed_organizations(&self, book: &model::Book) -> Vec<(model::Organization, usize)> {
        let Some(account) = self.customer_account() else {
            return Vec::new();
        };
        let detector = self.customer_detector();
        let today = self.customers_now().date_naive();
        let first = today - chrono::Duration::days(model::RECENT_DAYS);
        let last = today + chrono::Duration::days(30);
        let mut seen = BTreeSet::new();
        let mut groups = BTreeMap::<String, (model::Organization, BTreeSet<String>, BTreeSet<String>, usize)>::new();
        for index in self
            .calendar
            .days
            .range(first..=last)
            .flat_map(|(_, day)| day.events.iter().copied())
        {
            let event = &self.calendar.events[index];
            if !seen.insert(index) || !model::counted(event) {
                continue;
            }
            let detection = model::detect(book, &detector, &account, event);
            for organization in model::organizations(book, &detection.parties) {
                let stem = model::organization_stem(&organization.domains[0]);
                let entry = groups
                    .entry(stem)
                    .or_insert_with(|| (organization.clone(), BTreeSet::new(), BTreeSet::new(), 0));
                entry.1.extend(organization.domains);
                entry.2.extend(organization.people);
                entry.3 += 1;
            }
        }
        let mut organizations = groups
            .into_values()
            .map(|(mut organization, domains, people, meetings)| {
                organization.domains = domains.into_iter().collect();
                organization.people = people.into_iter().collect();
                (organization, meetings)
            })
            .collect::<Vec<_>>();
        organizations.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.name.cmp(&right.0.name)));
        organizations.truncate(8);
        organizations
    }

    fn customer_list(&mut self, ui: &mut egui::Ui, book: &model::Book) {
        let language = self.preferences.language;
        let accent = self.accent();
        if self.customer_account().is_none() {
            ui.weak(language.text("Connect WorkIQ to your calendar account first."));
            return;
        }
        if self.preview {
            ui.weak(language.text("Offline demo - fictional calendar"));
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.customers.filter)
                .id(Id::new("customer-filter"))
                .hint_text(language.text("Search customers..."))
                .desired_width(320.0),
        );
        ui.add_space(6.0);
        let query = self.customers.filter.trim().to_lowercase();
        let customers = book
            .sorted()
            .into_iter()
            .filter(|customer| {
                query.is_empty()
                    || customer.name.to_lowercase().contains(&query)
                    || customer.domains.iter().any(|domain| domain.contains(&query))
            })
            .cloned()
            .collect::<Vec<_>>();
        if book.customers.is_empty() {
            ui.weak(language.text("No confirmed customers yet. Confirm one from a meeting with external attendees."));
        } else if customers.is_empty() {
            ui.weak(language.text("No matching customers"));
        }
        let mut open = None;
        for customer in &customers {
            ui.push_id(("customer-row", &customer.id), |ui| {
                ui.horizontal_wrapped(|ui| {
                    self.customer_icon(ui, customer);
                    if ui
                        .add(egui::Button::new(RichText::new(&customer.name).strong()).frame(false))
                        .on_hover_text(language.text("Open customer"))
                        .clicked()
                    {
                        open = Some(customer.id.clone());
                    }
                    ui.weak(customer.domains.join(", "));
                    match &customer.msx {
                        Some(msx) => {
                            ui.weak(language.text(&format!("MSX: {}", msx.name)));
                        }
                        None => {
                            ui.weak(language.text(kind_note(customer)));
                        }
                    }
                    match customer.kind {
                        model::Kind::Partner => chip(ui, "Partner", ui.visuals().weak_text_color()),
                        model::Kind::Ignored => chip(ui, &language.text("Don't track"), ui.visuals().weak_text_color()),
                        model::Kind::Customer => {}
                    }
                    if customer.sample {
                        chip(ui, &language.text("Sample data"), ui.visuals().weak_text_color());
                    }
                });
            });
        }
        if let Some(id) = open {
            self.customers.selected = Some(id);
            self.customers.remove = None;
        }
        let unconfirmed = self.unconfirmed_organizations(book);
        if !unconfirmed.is_empty() {
            ui.add_space(10.0);
            let mut chosen = None;
            card(ui, accent, &language.text("Unconfirmed customers in your meetings"), Some(unconfirmed.len()), |ui| {
                for (organization, meetings) in &unconfirmed {
                    if let Some(action) = self.organization_row(ui, book, organization, Some(*meetings)) {
                        chosen = Some(action);
                    }
                }
            });
            if let Some(action) = chosen {
                self.apply_organization(action);
            }
        }
    }

    fn customer_detail(&mut self, ui: &mut egui::Ui, book: &model::Book, customer: model::Customer) {
        let language = self.preferences.language;
        let accent = self.accent();
        let snapshot = self.customer_snapshot(book, &customer);
        let running = self
            .customers
            .msx_running
            .as_ref()
            .is_some_and(|(id, _)| *id == customer.id);
        let blocker = self.customer_msx_blocker(&customer);
        // One quiet refresh per session when the panel shows a stale or missing read.
        if blocker.is_none()
            && !running
            && !self.customers.msx_refreshed.contains(&customer.id)
            && snapshot.as_ref().is_none_or(|snapshot| {
                chrono::Utc::now() - snapshot.retrieved_at > chrono::Duration::minutes(MSX_STALE_MINUTES)
            })
        {
            self.refresh_customer_msx(&customer);
        }
        let running = self
            .customers
            .msx_running
            .as_ref()
            .is_some_and(|(id, _)| *id == customer.id);
        ui.horizontal_wrapped(|ui| {
            if ui.button(language.text("All customers")).clicked() {
                self.customers.selected = None;
                self.customers.remove = None;
            }
            if self.preview {
                ui.weak(language.text("Offline demo - fictional calendar"));
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            self.customer_icon(ui, &customer);
            ui.heading(&customer.name);
            if customer.sample {
                chip(ui, &language.text("Sample data"), ui.visuals().weak_text_color());
            }
        });
        if !customer.domains.is_empty() {
            ui.weak(customer.domains.join(", "));
        }
        ui.horizontal_wrapped(|ui| match &customer.msx {
            Some(msx) => {
                ui.label(language.text(&format!("MSX customer: {}", msx.name)));
                if let Some(tpid) = &msx.tpid {
                    ui.weak(format!("TPID {tpid}"));
                }
                if let Some(url) = customer.msx_url() {
                    ui.hyperlink_to(language.text("Open in MSX"), url);
                }
            }
            None => {
                ui.weak(language.text(kind_note(&customer)));
            }
        });
        let mut edit = false;
        let mut unlink = false;
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            match &snapshot {
                Some(snapshot) => ui.weak(language.text(&format!(
                    "Last refreshed {}",
                    snapshot.retrieved_at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
                ))),
                None => ui.weak(language.text("MSX not read yet")),
            };
            if running {
                ui.spinner();
                ui.weak(language.text("Loading MSX..."));
            } else if customer.msx.is_some() {
                refresh = ui
                    .add_enabled(blocker.is_none(), egui::Button::new(language.text("Refresh")))
                    .on_hover_text(language.text("Reads opportunities, milestones, your MSX tasks and Azure consumption again (read-only)."))
                    .on_disabled_hover_text(language.text(blocker.unwrap_or_default()))
                    .clicked();
            }
            edit = ui.button(language.text("Edit")).clicked();
            unlink = customer.msx.is_some() && ui.button(language.text("Unlink MSX")).clicked();
            if ui.button(language.text("Remove customer")).clicked() {
                self.customers.remove = Some(customer.id.clone());
            }
        });
        if refresh {
            self.refresh_customer_msx(&customer);
        }
        if edit {
            self.open_customer_editor(&customer);
        }
        if unlink {
            let mut unlinked = customer.clone();
            unlinked.msx = None;
            unlinked.updated_at = chrono::Utc::now().max(unlinked.created_at);
            self.command(Command::Customer(CustomerCommand::Save(Box::new(unlinked))));
        }
        if self.customers.remove.as_ref() == Some(&customer.id) {
            let mut decided = None;
            egui::Frame::new()
                .stroke(Stroke::new(1.0, ui.visuals().warn_fg_color))
                .corner_radius(6)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.label(language.text("Remove this customer with its domains and meeting choices? Meetings, notes and tasks stay."));
                    ui.horizontal(|ui| {
                        if ui.button(language.text("Remove customer")).clicked() {
                            decided = Some(true);
                        }
                        if ui.button(language.text("Cancel")).clicked() {
                            decided = Some(false);
                        }
                    });
                });
            match decided {
                Some(true) => {
                    if let Some(account) = self.customer_account() {
                        self.command(Command::Customer(CustomerCommand::Delete {
                            account,
                            id: customer.id.clone(),
                        }));
                    }
                    self.customers.remove = None;
                    self.customers.selected = None;
                    return;
                }
                Some(false) => self.customers.remove = None,
                None => {}
            }
        }
        if customer.sample {
            ui.weak(language.text(model::SAMPLE_NOTE));
        }
        ui.add_space(8.0);
        let view = self.customer_overview(&customer.id);
        egui::ScrollArea::vertical()
            .id_salt("customer-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ui.available_width() >= 900.0 {
                    ui.columns(2, |columns| {
                        let (left, right) = columns.split_at_mut(1);
                        self.customer_meetings(&mut left[0], &view, accent);
                        self.customer_notes_section(&mut left[0], &view, accent);
                        self.customer_tasks_section(&mut left[0], &view, accent);
                        self.customer_msx_section(&mut right[0], &customer, snapshot.as_ref(), running, accent);
                        self.customer_consumption_section(&mut right[0], &customer, snapshot.as_ref(), accent);
                        self.customer_communication_section(&mut right[0], book, &customer, &view, accent);
                    });
                } else {
                    self.customer_meetings(ui, &view, accent);
                    self.customer_notes_section(ui, &view, accent);
                    self.customer_tasks_section(ui, &view, accent);
                    self.customer_msx_section(ui, &customer, snapshot.as_ref(), running, accent);
                    self.customer_consumption_section(ui, &customer, snapshot.as_ref(), accent);
                    self.customer_communication_section(ui, book, &customer, &view, accent);
                }
            });
    }

    fn open_customer_meeting(&mut self, id: &str) {
        let Some(index) = self.calendar.event_index(id) else {
            self.error = Some("Meeting is no longer in the current calendar.".into());
            return;
        };
        let event = self.calendar.events[index].clone();
        self.open_recommended_meeting(&event);
        self.insights_expanded = false;
        if !self.assistant_detached {
            self.assistant = false;
        }
    }

    fn customer_meeting_row(&mut self, ui: &mut egui::Ui, entry: &model::Entry, accent: Color32) {
        let language = self.preferences.language;
        ui.push_id(("customer-meeting", &entry.event_id), |ui| {
            ui.horizontal_wrapped(|ui| {
                let date = entry.start.with_timezone(&Local);
                ui.weak(format!(
                    "{} {}",
                    day_label(date.date_naive(), language),
                    if entry.all_day { language.text("All day").into_owned() } else { date.format("%H:%M").to_string() }
                ));
                if ui
                    .add(egui::Button::new(&entry.title).frame(false).wrap_mode(egui::TextWrapMode::Wrap))
                    .on_hover_text(language.text("Open meeting details"))
                    .clicked()
                {
                    self.open_customer_meeting(&entry.event_id);
                }
                if entry.notes {
                    chip(ui, &language.text("Notes ready"), accent);
                }
                if entry.insights {
                    chip(ui, &language.text("Insights ready"), accent);
                }
            });
        });
    }

    fn customer_meetings(&mut self, ui: &mut egui::Ui, view: &Overview, accent: Color32) {
        let language = self.preferences.language;
        let timeline = &view.timeline;
        card(ui, accent, &language.text("Meetings with this customer"), None, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (value, label) in [
                    (timeline.recent.to_string(), "Meetings in the last 90 days"),
                    (tomlook::weekly::hours(timeline.recent_minutes, language), "Hours together in the last 90 days"),
                    (
                        timeline
                            .next
                            .as_ref()
                            .map(|entry| day_label(entry.start.with_timezone(&Local).date_naive(), language))
                            .unwrap_or_else(|| language.text("None scheduled").into_owned()),
                        "Next meeting",
                    ),
                ] {
                    egui::Frame::new()
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(6)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(value).strong().size(17.0));
                                ui.label(RichText::new(language.text(label)).small());
                            });
                        });
                }
            });
            if timeline.past.is_empty() && timeline.upcoming.is_empty() {
                ui.weak(language.text("No meetings with this customer in the local calendar."));
                return;
            }
            if !timeline.upcoming.is_empty() {
                ui.add_space(4.0);
                ui.strong(language.text("Upcoming"));
                for entry in timeline.upcoming.iter().take(UPCOMING_SHOWN) {
                    self.customer_meeting_row(ui, entry, accent);
                }
                if timeline.upcoming.len() > UPCOMING_SHOWN {
                    ui.weak(language.text(&format!("More meetings: {}", timeline.upcoming.len() - UPCOMING_SHOWN)));
                }
            }
            if !timeline.past.is_empty() {
                ui.add_space(4.0);
                ui.strong(language.text("Earlier meetings"));
                for entry in timeline.past.iter().take(PAST_SHOWN) {
                    self.customer_meeting_row(ui, entry, accent);
                }
                if timeline.past.len() > PAST_SHOWN {
                    ui.weak(language.text(&format!("More meetings: {}", timeline.past.len() - PAST_SHOWN)));
                }
            }
            ui.weak(language.text("From the local calendar cache: one month back and six months ahead."));
        });
    }

    fn customer_notes_section(&mut self, ui: &mut egui::Ui, view: &Overview, accent: Color32) {
        let language = self.preferences.language;
        let mut open = None;
        card(ui, accent, &language.text("Notes and decisions"), Some(view.notes.len()), |ui| {
            if view.notes.is_empty() {
                ui.weak(language.text("No saved meeting notes for this customer yet."));
            }
            for digest in &view.notes {
                ui.push_id(("customer-notes", &digest.event_id), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.weak(day_label(digest.start.with_timezone(&Local).date_naive(), language));
                        if ui
                            .add(egui::Button::new(RichText::new(&digest.title).strong()).frame(false))
                            .on_hover_text(language.text("Open meeting details"))
                            .clicked()
                        {
                            open = Some(digest.event_id.clone());
                        }
                    });
                    if !digest.summary.is_empty() {
                        ui.label(&digest.summary);
                    }
                    for (title, items) in [
                        ("Decisions", &digest.decisions),
                        ("Discussed", &digest.points),
                        ("My tasks", &digest.my_tasks),
                    ] {
                        if items.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(language.text(title)).small().strong());
                        for item in items {
                            // Under "My tasks" the repeated "My task:" heading adds nothing.
                            let item = if title == "My tasks" {
                                tomlook::tasks::without_label(item)
                            } else {
                                item.clone()
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(10.0);
                                ui.label(RichText::new(format!("• {item}")).small());
                            });
                        }
                    }
                    ui.add_space(4.0);
                });
            }
        });
        if let Some(id) = open {
            self.open_customer_meeting(&id);
        }
    }

    fn customer_tasks_section(&mut self, ui: &mut egui::Ui, view: &Overview, accent: Color32) {
        let language = self.preferences.language;
        let Some(account) = self.customer_account() else {
            return;
        };
        let open = view
            .tasks
            .iter()
            .filter(|task| task.status == tomlook::tasks::Status::Open)
            .count();
        let mut changes = Vec::new();
        card(ui, accent, &language.text("Tasks from these meetings"), Some(open), |ui| {
            if view.tasks.is_empty() {
                ui.weak(language.text("No open tasks from these meetings."));
            }
            for task in &view.tasks {
                ui.push_id(("customer-task", task.id), |ui| {
                    let mut done = task.status == tomlook::tasks::Status::Done;
                    let title = if done {
                        RichText::new(&task.title).strikethrough().weak()
                    } else {
                        RichText::new(&task.title)
                    };
                    ui.horizontal_wrapped(|ui| {
                        if ui.checkbox(&mut done, title).changed() {
                            changes.push((task.id, done));
                        }
                        if let Some(due) = task.due.filter(|_| !done) {
                            ui.weak(format!("{} {}", language.text("Due"), day_label(due, language)));
                        }
                        if let Some(meeting) = &task.meeting {
                            ui.label(RichText::new(&meeting.title).small().weak());
                        }
                    });
                });
            }
        });
        for (id, done) in changes {
            self.task_command(TaskCommand::SetDone {
                account: account.clone(),
                id,
                done,
            });
            self.customers.cached = None;
        }
    }

    fn customer_msx_section(
        &mut self,
        ui: &mut egui::Ui,
        customer: &model::Customer,
        snapshot: Option<&model::MsxSnapshot>,
        running: bool,
        accent: Color32,
    ) {
        let language = self.preferences.language;
        let state = model::msx_state(
            self.preferences.msx_enabled,
            self.msx_connected(),
            customer,
            snapshot,
            running,
            self.customers.msx_errors.get(&customer.id).map(String::as_str),
        );
        let mut open_msx = false;
        let mut link = false;
        card(ui, accent, "MSX", None, |ui| {
            let warn = ui.visuals().warn_fg_color;
            match &state {
                model::MsxState::Off => {
                    ui.weak(language.text(tomlook::msx::DISABLED));
                }
                model::MsxState::Sample => {
                    ui.weak(language.text(model::SAMPLE_NOTE));
                }
                model::MsxState::NoAssociation => {
                    ui.weak(language.text("This customer is not linked to MSX."));
                    link = ui.button(language.text("Link MSX customer")).clicked();
                }
                model::MsxState::NotSignedIn => {
                    ui.label(language.text("Not signed in to MSX. Sign in to read opportunities, milestones and consumption."));
                    open_msx = ui.button(language.text("Open MSX connection")).clicked();
                }
                model::MsxState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(language.text("Loading MSX..."));
                    });
                }
                model::MsxState::Unavailable(reason) => {
                    ui.colored_label(warn, language.text(&format!("MSX unavailable: {reason}")));
                    ui.weak(language.text("A failed read is not evidence that nothing exists."));
                }
                model::MsxState::Ready => {
                    ui.weak(language.text("MSX not read yet"));
                }
                model::MsxState::Partial(gaps) => {
                    ui.colored_label(warn, language.text("Partial MSX data:"));
                    for gap in gaps {
                        ui.label(RichText::new(language.text(gap)).small().color(warn));
                    }
                }
                model::MsxState::Complete => {}
            }
            let Some(snapshot) = snapshot.filter(|_| state != model::MsxState::NoAssociation) else {
                return;
            };
            if matches!(state, model::MsxState::NotSignedIn | model::MsxState::Unavailable(_) | model::MsxState::Loading | model::MsxState::Off) {
                ui.weak(language.text(&format!(
                    "Showing the saved read from {}.",
                    snapshot.retrieved_at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
                )));
            }
            match &snapshot.dataverse {
                model::Dataverse::Loaded {
                    opportunities,
                    opportunities_truncated,
                    milestones,
                    milestones_truncated,
                    tasks,
                    tasks_note,
                    sources,
                } => {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.strong(language.text("Open opportunities"));
                        ui.weak(opportunities.len().to_string());
                    });
                    for row in opportunities.iter().take(10) {
                        ui.horizontal_wrapped(|ui| {
                            match row.url.as_deref().filter(|url| tomlook::evidence::safe_source_url(url)) {
                                Some(url) => {
                                    ui.hyperlink_to(&row.name, url);
                                }
                                None => {
                                    ui.label(&row.name);
                                }
                            }
                            if let Some(stage) = &row.stage {
                                ui.weak(stage);
                            }
                            if let Some(close) = row.close {
                                ui.weak(format!("{} {}", language.text("Estimated close"), day_label(close, language)));
                            }
                            if let Some(owner) = &row.owner {
                                ui.weak(language.text(&format!("Owner: {owner}")));
                            }
                        });
                    }
                    if let Some(note) = model::list_note(opportunities.len(), *opportunities_truncated) {
                        ui.weak(language.text(note));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.strong(language.text("Active future milestones"));
                        ui.weak(milestones.len().to_string());
                    });
                    for row in milestones.iter().take(12) {
                        ui.horizontal_wrapped(|ui| {
                            match row.url.as_deref().filter(|url| tomlook::evidence::safe_source_url(url)) {
                                Some(url) => {
                                    ui.hyperlink_to(&row.name, url);
                                }
                                None => {
                                    ui.label(&row.name);
                                }
                            }
                            let (label, color) = match row.my_task {
                                Some(true) => ("My task: yes", accent),
                                Some(false) => ("My task: no", ui.visuals().weak_text_color()),
                                None => ("My task: unknown", ui.visuals().warn_fg_color),
                            };
                            chip(ui, &language.text(label), color);
                            if let Some(status) = &row.status {
                                ui.weak(status);
                            }
                            if let Some(due) = row.due {
                                ui.weak(format!("{} {}", language.text("Due"), day_label(due, language)));
                            }
                            if let Some(owner) = &row.owner {
                                ui.weak(language.text(&format!("Owner: {owner}")));
                            }
                        });
                    }
                    if let Some(note) = model::list_note(milestones.len(), *milestones_truncated) {
                        ui.weak(language.text(if milestones.is_empty() && !milestones_truncated {
                            "No active future milestones in a complete read."
                        } else {
                            note
                        }));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.strong(language.text("My MSX tasks"));
                        ui.weak(tasks.len().to_string());
                    });
                    for row in tasks.iter().take(10) {
                        ui.horizontal_wrapped(|ui| {
                            match row.url.as_deref().filter(|url| tomlook::evidence::safe_source_url(url)) {
                                Some(url) => {
                                    ui.hyperlink_to(&row.subject, url);
                                }
                                None => {
                                    ui.label(&row.subject);
                                }
                            }
                            if let Some(due) = row.due {
                                ui.weak(format!("{} {}", language.text("Due"), day_label(due, language)));
                            }
                            if let Some(milestone) = &row.milestone {
                                ui.weak(milestone);
                            }
                        });
                    }
                    if let Some(note) = tasks_note {
                        ui.colored_label(ui.visuals().warn_fg_color, language.text(note));
                    } else if let Some(note) = model::list_note(tasks.len(), false) {
                        ui.weak(language.text(note));
                    }
                    egui::CollapsingHeader::new(language.text(&format!("Sources ({})", sources.len())))
                        .id_salt("customer-msx-sources")
                        .default_open(false)
                        .show(ui, |ui| {
                            for source in sources.iter().take(8) {
                                let rows = if source.truncated {
                                    format!("Rows: {}, truncated", source.rows)
                                } else {
                                    format!("Rows: {}", source.rows)
                                };
                                ui.label(
                                    RichText::new(format!(
                                        "{}: {} · {} · {} · {}",
                                        language.text("Source"),
                                        source.system,
                                        source.table,
                                        language.text(&rows),
                                        source.retrieved_at.with_timezone(&Local).format("%Y-%m-%d %H:%M")
                                    ))
                                    .small()
                                    .weak(),
                                );
                            }
                        });
                }
                model::Dataverse::Unavailable { reason } => {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        language.text(&format!("Opportunities and milestones unavailable: {reason}")),
                    );
                }
            }
        });
        if open_msx {
            self.connection_panel = Some(ConnectionPanel::Msx);
        }
        if link {
            self.open_customer_editor(customer);
        }
    }

    fn customer_consumption_section(
        &mut self,
        ui: &mut egui::Ui,
        customer: &model::Customer,
        snapshot: Option<&model::MsxSnapshot>,
        accent: Color32,
    ) {
        let language = self.preferences.language;
        let scene = self.preview
            && self.demo_screenshot.is_some()
            && self.demo_scene == tomlook::showcase::Scene::Customer;
        let response = ui.scope(|ui| card(ui, accent, &language.text("Azure consumption"), None, |ui| {
            let warn = ui.visuals().warn_fg_color;
            match snapshot.map(|snapshot| &snapshot.consumption) {
                None if customer.msx.is_none() => {
                    ui.weak(language.text("Link an MSX customer with a TPID to see Azure consumption."));
                }
                None if customer.msx.as_ref().is_some_and(|msx| msx.tpid.is_none()) => {
                    ui.weak(language.text("No TPID is linked; Azure consumption needs the customer's TPID."));
                }
                None => {
                    ui.weak(language.text("MSX not read yet"));
                }
                Some(model::Consumption::NoTpid) => {
                    ui.weak(language.text("No TPID is linked; Azure consumption needs the customer's TPID."));
                }
                Some(model::Consumption::NoData { reason }) => {
                    ui.weak(language.text(&format!("No consumption data: {reason}")));
                }
                Some(model::Consumption::Unavailable { reason }) => {
                    ui.colored_label(warn, language.text(&format!("Azure consumption unavailable: {reason}")));
                }
                Some(model::Consumption::Available {
                    months,
                    unit,
                    dataset,
                    latest_month,
                    prediction_note,
                    ..
                }) => {
                    acr_chart(ui, &model::chart(months, usize::from(model::ACR_MONTHS)), unit, language, accent);
                    ui.horizontal_wrapped(|ui| {
                        legend(ui, model::MonthKind::Actual, accent, &language.text("Completed month"));
                        legend(ui, model::MonthKind::Partial, accent, &language.text("Current month (partial)"));
                        legend(ui, model::MonthKind::Prediction, accent, &language.text("Model prediction"));
                    });
                    ui.weak(language.text(&format!("Unit: {unit}")));
                    ui.weak(language.text(&format!("Dataset: {dataset}")));
                    let latest = match latest_month {
                        Some(month) => format!("Latest month with data: {}", month.format("%Y-%m")),
                        None => "No month has data yet.".to_owned(),
                    };
                    ui.weak(language.text(&latest));
                    if months.iter().any(|month| month.kind == model::MonthKind::Prediction) {
                        ui.weak(language.text(prediction_note));
                    }
                    ui.weak(language.text("Consumption is technical evidence, not business intent."));
                }
            }
        }))
        .response;
        // The demo screenshot shows the chart and the communication summary.
        if scene && self.demo_frames < 3 {
            response.scroll_to_me(Some(egui::Align::Min));
        }
    }

    fn customer_communication_section(
        &mut self,
        ui: &mut egui::Ui,
        book: &model::Book,
        customer: &model::Customer,
        view: &Overview,
        accent: Color32,
    ) {
        let language = self.preferences.language;
        let saved = self.customer_communication(book, &customer.id);
        let running = self
            .customers
            .research
            .as_ref()
            .is_some_and(|running| running.request.customer == customer.id);
        let blocker = self.customer_research_blocker(&customer.id);
        let mut request = false;
        let mut cancel = false;
        card(ui, accent, &language.text("Recent communication"), None, |ui| {
            ui.horizontal_wrapped(|ui| {
                if running {
                    ui.spinner();
                    ui.weak(language.text("Searching mail, Teams and meetings..."));
                    cancel = ui.small_button(language.text("Cancel")).clicked();
                } else {
                    request = ui
                        .add_enabled(blocker.is_none(), egui::Button::new(language.text("Find related communication")))
                        .on_hover_text(language.text("One Thorough AI request with read-only mail, Teams and meeting search over the last 60 days; it counts toward the daily AI limit."))
                        .on_disabled_hover_text(language.text(blocker.unwrap_or_default()))
                        .clicked();
                }
            });
            if let Some(reason) = blocker.filter(|_| !running) {
                ui.weak(language.text(reason));
            }
            if let Some(error) = self.customers.research_errors.get(&customer.id) {
                ui.colored_label(ui.visuals().warn_fg_color, language.text(error));
            }
            let Some(saved) = &saved else {
                ui.weak(language.text("Not searched yet. Tomlook searches only when you ask, and before preparing a confirmed customer meeting it reuses the last summary."));
                return;
            };
            ui.weak(language.text(&format!(
                "Searched {} · last {} days",
                saved.created_at.with_timezone(&Local).format("%Y-%m-%d %H:%M"),
                saved.days
            )));
            if let Some(topic) = &saved.topic {
                ui.weak(language.text(&format!("Focus: {topic}")));
            }
            if saved.sample {
                ui.weak(language.text("Sample communication summary: fictional, not produced by AI."));
            }
            if saved.items.is_empty() {
                ui.weak(language.text("No related communication found in the searched scope."));
            }
            for (index, item) in saved.items.iter().enumerate() {
                ui.push_id(("customer-communication", index), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        chip(ui, &language.text(item.kind.label()), accent);
                        if let Some(date) = item.date {
                            ui.weak(day_label(date, language));
                        }
                        ui.strong(&item.subject);
                    });
                    ui.label(&item.summary);
                    if !item.why_relevant.is_empty() {
                        ui.label(RichText::new(language.text(&format!("Why it matters: {}", item.why_relevant))).small());
                    }
                    ui.horizontal_wrapped(|ui| {
                        if !item.people.is_empty() {
                            ui.label(RichText::new(item.people.join(", ")).small().weak());
                        }
                        if let Some(url) = item.url.as_deref().filter(|url| tomlook::evidence::safe_source_url(url)) {
                            ui.hyperlink_to(language.text("Open source"), url);
                        } else if !item.verified && !saved.sample {
                            ui.label(RichText::new(language.text("Source not among the retrieved results")).small().color(ui.visuals().warn_fg_color));
                        }
                    });
                    ui.add_space(3.0);
                });
            }
            for (title, items) in [("Open questions", &saved.open_questions), ("Risks", &saved.risks), ("Coverage gaps", &saved.gaps)] {
                if items.is_empty() {
                    continue;
                }
                ui.label(RichText::new(language.text(title)).strong());
                for item in items {
                    ui.label(format!("• {item}"));
                }
            }
            if !saved.commitments.is_empty() {
                ui.label(RichText::new(language.text("Commitments")).strong());
                for item in &saved.commitments {
                    let text = match item.by {
                        research::Side::Me => format!("My commitment: {}", item.text),
                        research::Side::Them => format!("Customer commitment: {}", item.text),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("• {}", language.text(&text)));
                        if let Some(due) = item.due {
                            ui.weak(format!("{} {}", language.text("Due"), day_label(due, language)));
                        }
                    });
                }
            }
            if saved.dropped_sources > 0 {
                ui.weak(language.text(&format!(
                    "{} cited sources were removed because they were not retrieved or not safe.",
                    saved.dropped_sources
                )));
            }
        });
        if request {
            self.request_customer_research(customer, view);
        }
        if cancel {
            self.cancel_customer_research();
        }
    }
}
