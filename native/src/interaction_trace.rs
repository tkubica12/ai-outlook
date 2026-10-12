use eframe::egui::{self, Event, Key, Modifiers};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const INTERACTIONS: usize = 1000;

#[derive(Default, serde::Serialize)]
pub struct RenderProfile {
    pub ui_passes: usize,
    pub ui_total_ms: f64,
    pub cards: usize,
    pub culled_cards: usize,
    pub card_time_ms: f64,
    pub card_widget_ms: f64,
    pub card_paint_ms: f64,
    pub title_layout_ms: f64,
}

struct Action {
    name: &'static str,
    event: Event,
    expected: (&'static str, Value),
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn action(index: usize, state: &Value) -> Action {
    let field = |name: &'static str, event, property, value| Action {
        name,
        event,
        expected: (property, value),
    };
    let palette = |label: &str, property, value| {
        [
            field(
                "open-command-palette",
                key(Key::P, Modifiers::CTRL | Modifiers::SHIFT),
                "palette",
                json!(true),
            ),
            field(
                "search-command",
                Event::Text(label.into()),
                "palette_query",
                json!(label),
            ),
            field(
                "execute-command",
                key(Key::Enter, Modifiers::NONE),
                property,
                value,
            ),
        ]
    };
    let mut actions = Vec::new();
    for (label, view) in [
        ("Show day", "day"),
        ("Show work week", "work-week"),
        ("Show week", "week"),
        ("Show month", "month"),
    ] {
        actions.extend(palette(label, "view", json!(view)));
    }
    actions.extend(palette("Go to today", "anchor", state["today"].clone()));
    actions.extend(palette(
        "Toggle dark / light",
        "dark",
        json!(!state["dark"].as_bool().unwrap_or(false)),
    ));
    actions.extend([
        field(
            "previous-period",
            key(Key::ArrowLeft, Modifiers::ALT),
            "anchor",
            state["previous"].clone(),
        ),
        field(
            "next-period",
            key(Key::ArrowRight, Modifiers::ALT),
            "anchor",
            state["next"].clone(),
        ),
        field(
            "focus-calendar-search",
            key(Key::F, Modifiers::CTRL),
            "search_focused",
            json!(true),
        ),
        field(
            "search-czech-text",
            Event::Text("žluť".into()),
            "query",
            json!("žluť"),
        ),
        field(
            "clear-search",
            key(Key::Escape, Modifiers::NONE),
            "query",
            json!(""),
        ),
        field(
            "keyboard-open-meeting",
            key(Key::ArrowDown, Modifiers::NONE),
            "selected",
            json!(true),
        ),
        field(
            "close-meeting",
            key(Key::Escape, Modifiers::NONE),
            "selected",
            json!(false),
        ),
        field(
            "open-assistant",
            key(Key::Space, Modifiers::CTRL),
            "assistant",
            json!(true),
        ),
        field(
            "close-assistant",
            key(Key::Escape, Modifiers::NONE),
            "assistant",
            json!(false),
        ),
    ]);
    actions.swap_remove(index % actions.len())
}

pub struct Trace {
    expected_events: usize,
    started: Instant,
    warmed: usize,
    pending: Option<(Action, Instant, Option<Value>)>,
    samples: Vec<Value>,
    stopped: Option<String>,
    native_pixels_per_point: Option<f32>,
}

impl Trace {
    pub fn new(expected_events: usize) -> Self {
        Self {
            expected_events,
            started: Instant::now(),
            warmed: 0,
            pending: None,
            samples: Vec::with_capacity(INTERACTIONS),
            stopped: None,
            native_pixels_per_point: None,
        }
    }

    pub fn input(&mut self, input: &mut egui::RawInput, state: &Value) {
        self.native_pixels_per_point = input
            .viewports
            .get(&egui::ViewportId::ROOT)
            .and_then(|viewport| viewport.native_pixels_per_point);
        if self.stopped.is_some() || self.samples.len() == INTERACTIONS {
            return;
        }
        if let Some(event) = input.events.iter().find(|event| {
            matches!(
                event,
                Event::Key { pressed: true, .. }
                    | Event::Text(_)
                    | Event::PointerButton { pressed: true, .. }
                    | Event::MouseWheel { .. }
            )
        }) {
            let kind = match event {
                Event::Key { .. } => "key press",
                Event::Text(_) => "text",
                Event::PointerButton { .. } => "pointer press",
                Event::MouseWheel { .. } => "wheel",
                _ => unreachable!(),
            };
            self.stopped = Some(format!(
                "Physical {kind} input interrupted the synthetic trial"
            ));
            return;
        }
        if self.deadline_reached() {
            return;
        }
        if let Some((action, dispatched, observed)) = self.pending.take() {
            let Some(observed) = observed else {
                self.pending = Some((action, dispatched, None));
                return;
            };
            let passed = observed[action.expected.0] == action.expected.1;
            if let Event::Key { key, modifiers, .. } = action.event {
                input.events.push(Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                });
            }
            self.samples.push(json!({
                "action": action.name,
                "latency_ms": dispatched.elapsed().as_secs_f64() * 1000.0,
                "expected": { action.expected.0: action.expected.1 },
                "observed": observed,
                "passed": passed,
            }));
            if !passed {
                self.stopped = Some(format!("Task-specific end state failed: {}", action.name));
                return;
            }
        }
        if state["error"].is_string() {
            self.stopped = Some(format!("Native UI error: {}", state["error"]));
            return;
        }
        if state["loading"] != false || state["events"] != self.expected_events {
            return;
        }
        if self.warmed < 30 {
            self.warmed += 1;
            return;
        }
        if self.samples.len() < INTERACTIONS {
            let action = action(self.samples.len(), state);
            let modifiers = match &action.event {
                Event::Key { modifiers, .. } => *modifiers,
                _ => Modifiers::NONE,
            };
            input.events.push(Event::ModifiersChanged(modifiers));
            input.focused = true;
            if let Some(viewport) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
                viewport.focused = Some(true);
            }
            input.events.push(Event::WindowFocused(true));
            input.events.push(action.event.clone());
            self.pending = Some((action, Instant::now(), None));
        }
    }

    pub fn rendered(&mut self, state: Value) {
        if let Some((_, _, observed)) = &mut self.pending {
            *observed = Some(state);
        }
    }

    pub fn previous_frame_cpu(&mut self, seconds: Option<f32>) {
        if let Some(sample) = self.samples.last_mut() {
            sample["frame_cpu_ms"] = seconds.map(|seconds| f64::from(seconds) * 1000.0).into();
        }
    }

    pub fn deadline_reached(&mut self) -> bool {
        if self.samples.len() < INTERACTIONS && self.started.elapsed() >= Duration::from_secs(120) {
            self.stopped
                .get_or_insert_with(|| "Trial exceeded its 120-second deadline".into());
            true
        } else {
            false
        }
    }

    pub fn finished(&self) -> bool {
        self.stopped.is_some() || self.samples.len() == INTERACTIONS
    }

    pub fn report(&self) -> Value {
        let mut latencies: Vec<f64> = self
            .samples
            .iter()
            .filter_map(|sample| sample["latency_ms"].as_f64())
            .collect();
        latencies.sort_by(f64::total_cmp);
        let p95 = if latencies.is_empty() {
            None
        } else {
            latencies.get((latencies.len() * 95).div_ceil(100) - 1)
        };
        json!({
            "schema": "tomlook-native-interaction-trace-v1",
            "landmark": "Injected eframe raw input to the start of the following native frame after task-state verification at the end of UI construction",
            "limits": "Not an independent GPU-display timestamp, OS input-dispatch measurement, physical keyboard acceptance, or active-provider interference test",
            "fixture": "tomlook-frozen-fixture-v1",
            "expected_events": self.expected_events,
            "native_pixels_per_point": self.native_pixels_per_point,
            "requested_interactions": INTERACTIONS,
            "completed_interactions": self.samples.len(),
            "p95_ms": p95,
            "maximum_ms": latencies.last(),
            "error": self.stopped,
            "threshold_pass": self.samples.len() == INTERACTIONS
                && self.stopped.is_none()
                && p95.is_some_and(|ms| *ms <= 50.0)
                && latencies.last().is_some_and(|ms| *ms <= 100.0),
            "samples": self.samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_trials_do_not_pass() {
        assert_eq!(Trace::new(200).report()["threshold_pass"], false);
    }

    #[test]
    fn physical_input_stops_injection() {
        let mut trace = Trace::new(200);
        let mut input = egui::RawInput {
            events: vec![Event::Text("owner input".into())],
            ..Default::default()
        };
        trace.input(&mut input, &json!({"loading": false, "events": 200}));
        assert!(trace.finished());
        assert_eq!(input.events.len(), 1);
        assert_eq!(trace.report()["completed_interactions"], 0);
    }

    #[test]
    fn focus_loss_key_release_does_not_look_like_owner_input() {
        let mut trace = Trace::new(200);
        let mut release = key(Key::P, Modifiers::NONE);
        if let Event::Key { pressed, .. } = &mut release {
            *pressed = false;
        }
        let mut input = egui::RawInput {
            events: vec![release],
            ..Default::default()
        };
        trace.input(&mut input, &json!({"loading": true, "events": 0}));
        assert!(!trace.finished());
    }

    #[test]
    fn wrong_task_state_fails_instead_of_counting_a_paint() {
        let mut trace = Trace::new(200);
        trace.warmed = 30;
        let state = json!({"loading": false, "events": 200});
        trace.input(&mut egui::RawInput::default(), &state);
        trace.rendered(json!({"palette": false}));
        trace.input(&mut egui::RawInput::default(), &state);
        assert!(trace.finished());
        assert_eq!(trace.report()["samples"][0]["passed"], false);
    }

    #[test]
    fn renderer_cpu_belongs_to_the_just_presented_action() {
        let mut trace = Trace::new(200);
        trace.warmed = 30;
        let state = json!({"loading": false, "events": 200, "dark": true});
        trace.input(&mut egui::RawInput::default(), &state);
        trace.rendered(json!({"palette": true}));
        trace.input(&mut egui::RawInput::default(), &state);
        trace.previous_frame_cpu(Some(0.003));
        let report = trace.report();
        assert_eq!(report["samples"][0]["action"], "open-command-palette");
        let cpu = report["samples"][0]["frame_cpu_ms"].as_f64().unwrap();
        assert!((cpu - 3.0).abs() < 0.000001);
    }

    #[test]
    fn deadline_does_not_require_a_rendered_frame() {
        let mut trace = Trace::new(200);
        trace.started = Instant::now()
            .checked_sub(Duration::from_secs(121))
            .unwrap();
        assert!(trace.deadline_reached());
        assert!(trace.finished());
        assert_eq!(trace.report()["threshold_pass"], false);
        assert_eq!(
            trace.report()["error"],
            "Trial exceeded its 120-second deadline"
        );
    }
}
