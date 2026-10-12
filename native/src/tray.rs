use eframe::egui::{Context, ViewportCommand};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

pub struct Tray {
    _icon: TrayIcon,
    show: MenuItem,
    pause: MenuItem,
    exit: MenuItem,
    pub exiting: Arc<AtomicBool>,
}

pub(crate) fn restore(context: &Context) {
    context.send_viewport_cmd(ViewportCommand::Visible(true));
    context.send_viewport_cmd(ViewportCommand::Minimized(false));
    context.send_viewport_cmd(ViewportCommand::Focus);
    context.request_repaint();
}

impl Tray {
    pub fn new(
        context: Context,
        commands: Option<std::sync::mpsc::SyncSender<tomlook::worker::Command>>,
    ) -> Result<Self, String> {
        let menu = Menu::new();
        let show = MenuItem::new("Open Tomlook", true, None);
        let exit = MenuItem::new("Exit Tomlook (stop AI)", true, None);
        let pause = MenuItem::new("Pause / resume Insights", commands.is_some(), None);
        menu.append_items(&[&show, &pause, &exit])
            .map_err(|e| format!("Create tray menu: {e}"))?;
        let rgba = tomlook::brand::rgba(32);
        let icon = Icon::from_rgba(rgba, 32, 32).map_err(|e| format!("Create tray icon: {e}"))?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("Tomlook - calendar and Insights")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| format!("Start Windows tray: {e}"))?;
        let exiting = Arc::new(AtomicBool::new(false));
        let exit_flag = exiting.clone();
        let show_id = show.id().clone();
        let exit_id = exit.id().clone();
        let pause_id = pause.id().clone();
        let menu_context = context.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                restore(&menu_context);
            } else if event.id == pause_id
                && let Some(commands) = &commands
            {
                if commands
                    .try_send(tomlook::worker::Command::TogglePause)
                    .is_err()
                {
                    menu_context.data_mut(|data| data.insert_temp(eframe::egui::Id::new("tray-error"), "Tray pause command could not be submitted; preparation state is unchanged".to_string()));
                }
                restore(&menu_context);
            } else if event.id == exit_id {
                exit_flag.store(true, Ordering::Release);
                restore(&menu_context);
                menu_context.send_viewport_cmd(ViewportCommand::Close);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                restore(&context);
            }
        }));
        Ok(Self {
            _icon: icon,
            show,
            pause,
            exit,
            exiting,
        })
    }

    pub fn exit(&self, context: &Context) {
        self.exiting.store(true, Ordering::Release);
        context.send_viewport_cmd(ViewportCommand::Close);
    }

    pub fn set_color(&self, color: [u8; 3]) -> Result<(), String> {
        let icon = Icon::from_rgba(tomlook::brand::rgba_with_color(32, color), 32, 32)
            .map_err(|error| format!("Create Focus tray icon: {error}"))?;
        self._icon
            .set_icon(Some(icon))
            .map_err(|error| format!("Update Focus tray icon: {error}"))
    }

    pub fn set_language(&self, language: tomlook::i18n::Language) -> Result<(), String> {
        self.show.set_text(language.text("Open Tomlook"));
        self.pause
            .set_text(language.text("Pause / resume Insights"));
        self.exit.set_text(language.text("Exit Tomlook (stop AI)"));
        self._icon
            .set_tooltip(Some(language.text("Tomlook - calendar and Insights")))
            .map_err(|error| format!("Update tray language: {error}"))
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_requests_visibility_unminimizing_and_focus_without_closing() {
        let context = Context::default();
        let mut output = context.run_ui(Default::default(), |ui| restore(ui.ctx()));
        output.textures_delta.clear();
        let commands = &output.viewport_output[&eframe::egui::ViewportId::ROOT].commands;
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::Visible(true)))
        );
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::Minimized(false)))
        );
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::Focus))
        );
        assert!(
            commands
                .iter()
                .all(|command| !matches!(command, ViewportCommand::Close))
        );
    }
}
