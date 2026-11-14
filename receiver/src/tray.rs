//! Plasma's StatusNotifier tray. Callbacks only enqueue commands; the UI thread
//! owns the receiver process and applies each command through its normal paths.
use eframe::egui::Context;
use ksni::{blocking::TrayMethods, menu::StandardItem};
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Show,
    Settings,
    Toggle,
    SendClipboard,
    Quit,
    Offline,
    Online,
}

struct Tray {
    sender: Sender<Action>,
    context: Context,
    running: bool,
    clipboard_enabled: bool,
}

impl Tray {
    fn send(&self, action: Action) {
        let _ = self.sender.send(action);
        self.context.request_repaint();
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "mac-deck-input-bridge".into()
    }
    fn icon_name(&self) -> String {
        "mac-deck-input-bridge".into()
    }
    fn overlay_icon_name(&self) -> String {
        if self.running { "emblem-default" } else { "" }.into()
    }
    fn title(&self) -> String {
        "MacDeck".into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "MacDeck".into(),
            description: if self.running {
                "Mac input is enabled"
            } else {
                "Mac input is disabled"
            }
            .into(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _: i32, _: i32) {
        self.send(Action::Show);
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.send(Action::Offline);
        true
    }
    fn watcher_online(&self) {
        self.send(Action::Online);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open MacDeck".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::Show)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Settings…".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::Settings)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: if self.running {
                    "Disable Mac input"
                } else {
                    "Enable Mac input"
                }
                .into(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::Toggle)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Send Deck clipboard now".into(),
                enabled: self.running && self.clipboard_enabled,
                activate: Box::new(|tray: &mut Self| tray.send(Action::SendClipboard)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit MacDeck".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct TrayBridge {
    actions: Receiver<Action>,
    startup: Receiver<Result<ksni::blocking::Handle<Tray>, ()>>,
    handle: Option<ksni::blocking::Handle<Tray>>,
    available: bool,
    previous_state: Option<(bool, bool)>,
}

impl TrayBridge {
    pub fn start(context: Context) -> Self {
        let (sender, actions) = mpsc::channel();
        let (startup_sender, startup) = mpsc::channel();
        std::thread::spawn(move || {
            let tray = Tray {
                sender,
                context: context.clone(),
                running: false,
                clipboard_enabled: false,
            };
            let result = tray.spawn().map_err(|_| ());
            let _ = startup_sender.send(result);
            context.request_repaint();
        });
        Self {
            actions,
            startup,
            handle: None,
            available: false,
            previous_state: None,
        }
    }

    pub fn poll(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Ok(result) = self.startup.try_recv() {
            match result {
                Ok(handle) => {
                    self.handle = Some(handle);
                    self.available = true;
                }
                Err(()) => actions.push(Action::Offline),
            }
        }
        actions.extend(self.actions.try_iter());
        if self
            .handle
            .as_ref()
            .is_some_and(|handle| handle.is_closed())
            && self.available
        {
            actions.push(Action::Offline);
        }
        for action in &actions {
            match action {
                Action::Offline => self.available = false,
                Action::Online => self.available = true,
                _ => {}
            }
        }
        actions
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub fn update(&mut self, running: bool, clipboard_enabled: bool) {
        if self.previous_state == Some((running, clipboard_enabled)) {
            return;
        }
        if let Some(handle) = &self.handle {
            if handle
                .update(|tray| {
                    tray.running = running;
                    tray.clipboard_enabled = clipboard_enabled;
                })
                .is_some()
            {
                self.previous_state = Some((running, clipboard_enabled));
            }
        }
    }
}

impl Drop for TrayBridge {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.shutdown().wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray as _;

    #[test]
    fn tray_menu_tracks_input_state_and_routes_actions() {
        let (sender, actions) = mpsc::channel();
        let mut tray = Tray {
            sender,
            context: Context::default(),
            running: false,
            clipboard_enabled: true,
        };
        let menu = tray.menu();
        let ksni::MenuItem::Standard(toggle) = &menu[3] else {
            panic!("toggle missing")
        };
        assert_eq!(toggle.label, "Enable Mac input");
        let ksni::MenuItem::Standard(send) = &menu[4] else {
            panic!("clipboard action missing")
        };
        assert!(!send.enabled);
        (toggle.activate)(&mut tray);
        assert_eq!(actions.try_recv(), Ok(Action::Toggle));

        tray.running = true;
        let menu = tray.menu();
        let ksni::MenuItem::Standard(toggle) = &menu[3] else {
            panic!("toggle missing")
        };
        let ksni::MenuItem::Standard(send) = &menu[4] else {
            panic!("clipboard action missing")
        };
        assert_eq!(toggle.label, "Disable Mac input");
        assert!(send.enabled);
        assert_eq!(tray.tool_tip().description, "Mac input is enabled");
        (send.activate)(&mut tray);
        assert_eq!(actions.try_recv(), Ok(Action::SendClipboard));
    }
}
