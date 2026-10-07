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
    pub exiting: Arc<AtomicBool>,
}

fn restore(context: &Context) {
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
        let pause = MenuItem::new("Pause / resume preparation", commands.is_some(), None);
        menu.append_items(&[&show, &pause, &exit])
            .map_err(|e| format!("Create tray menu: {e}"))?;
        let mut rgba = vec![0; 32 * 32 * 4];
        for y in 4..28 {
            for x in 4..28 {
                let stroke = (y < 9 && (7..25).contains(&x)) || ((13..19).contains(&x) && y < 26);
                let pixel = (y * 32 + x) * 4;
                rgba[pixel..pixel + 4].copy_from_slice(if stroke {
                    &[235, 235, 235, 255]
                } else {
                    &[35, 35, 35, 255]
                });
            }
        }
        let icon = Icon::from_rgba(rgba, 32, 32).map_err(|e| format!("Create tray icon: {e}"))?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("Tomlook - open calendar; preparation status in Settings")
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
            exiting,
        })
    }

    pub fn exit(&self, context: &Context) {
        self.exiting.store(true, Ordering::Release);
        context.send_viewport_cmd(ViewportCommand::Close);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    }
}
