use eframe::egui::{self, Color32, Id, RichText};
use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};
use get_if_addrs::{get_if_addrs, IfAddr, Interface};
use mac_deck_receiver::PairingSecret;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    env,
    fs::{self, OpenOptions},
    io,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const APP_NAME: &str = "MacDeck";
const APP_ID: &str = "mac-deck-input-bridge";
const CONFIG_DIRECTORY: &str = "mac-deck-input-bridge";
const CONFIG_FILE: &str = "receiver.toml";
const GAMING_MODE_PREFERRED_SIZE: [f32; 2] = [960.0, 720.0];
const MINIMUM_WINDOW_SIZE: [f32; 2] = [480.0, 360.0];

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ReceiverSettings {
    bind: String,
    pairing_secret: String,
    #[serde(default)]
    sync_layout_from_mac: bool,
    #[serde(default)]
    sync_clipboard_from_mac: bool,
    #[serde(default)]
    layout_mappings: String,
}

impl Default for ReceiverSettings {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:4242".to_owned(),
            pairing_secret: PairingSecret::generate().to_base64(),
            sync_layout_from_mac: false,
            sync_clipboard_from_mac: false,
            layout_mappings: String::new(),
        }
    }
}

impl ReceiverSettings {
    fn validates(&self) -> Result<(), String> {
        PairingSecret::parse(&self.pairing_secret)
            .map(|_| ())
            .map_err(|error| error.to_string())?;
        self.bind
            .parse::<std::net::SocketAddr>()
            .map(|_| ())
            .map_err(|_| {
                "Bind address must be an IP address and port, for example 0.0.0.0:4242".to_owned()
            })?;
        // Mappings are optional overrides. With none present, the receiver
        // automatically matches the Mac source name against configured Plasma
        // layouts; validate user-supplied overrides whenever they exist.
        if !self.layout_mappings.trim().is_empty() {
            parse_layout_mappings(&self.layout_mappings)?;
        }
        Ok(())
    }
}

struct ReceiverProcess {
    child: Option<Child>,
}

trait ReceiverSupervisor {
    fn is_running(&mut self) -> bool {
        false
    }
    fn start(&mut self, settings: &ReceiverSettings) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
}

impl ReceiverSupervisor for ReceiverProcess {
    fn is_running(&mut self) -> bool {
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(Ok(Some(_))) | Some(Err(_)) | None => {
                self.child = None;
                false
            }
        }
    }

    fn start(&mut self, settings: &ReceiverSettings) -> Result<(), String> {
        settings.validates()?;
        if self.is_running() {
            return Ok(());
        }
        let executable = receiver_executable()?;
        let child = Command::new(executable)
            .arg("serve")
            .arg("--bind")
            .arg(&settings.bind)
            .env("MAC_DECK_PAIRING_SECRET", &settings.pairing_secret)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .args(layout_sync_arguments(settings)?)
            .args(clipboard_sync_arguments(settings))
            .spawn()
            .map_err(|error| format!("Could not start receiver: {error}"))?;
        self.child = Some(child);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        match child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                request_graceful_shutdown(&mut child)?;
                Ok(())
            }
            Err(error) => Err(format!("Could not inspect receiver: {error}")),
        }
    }
}

fn request_graceful_shutdown(child: &mut Child) -> Result<(), String> {
    #[cfg(unix)]
    {
        let status = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
        if status == -1 {
            return Err(format!(
                "Could not request receiver shutdown: {}",
                io::Error::last_os_error()
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(error) => return Err(format!("Could not inspect receiver shutdown: {error}")),
            }
        }
    }

    // A damaged or older receiver binary might not handle SIGTERM. Keep a short,
    // bounded fallback so disabling input cannot leave an unresponsive child alive.
    child
        .kill()
        .map_err(|error| format!("Could not force-stop receiver: {error}"))?;
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("Could not wait for receiver shutdown: {error}"))
}

impl Drop for ReceiverProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct ReceiverApp<P = ReceiverProcess> {
    settings: ReceiverSettings,
    settings_path: PathBuf,
    process: P,
    selected_tab: Tab,
    ip_addresses: Vec<Ipv4Addr>,
    status: Option<String>,
    gamepad: Option<GamepadNavigation>,
    gamepad_start_attempted: bool,
}

/// Reads the Steam Deck controls directly from Linux evdev. Steam does
/// not promise to translate Gaming Mode controller input into keyboard arrows
/// for a non-Steam desktop window, so relying on eframe keyboard events alone
/// leaves the UI inaccessible in Gaming Mode.
struct GamepadNavigation {
    keys: Receiver<egui::Key>,
}

#[derive(Default)]
struct StickState {
    x_active: bool,
    y_active: bool,
}

impl StickState {
    fn key_for_axis(&mut self, axis: AbsoluteAxisCode, value: f32) -> Option<egui::Key> {
        const ENGAGE_THRESHOLD: f32 = 0.70;
        const RELEASE_THRESHOLD: f32 = 0.35;
        let (active, negative, positive) = match axis {
            AbsoluteAxisCode::ABS_X | AbsoluteAxisCode::ABS_HAT0X => (
                &mut self.x_active,
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
            ),
            AbsoluteAxisCode::ABS_Y | AbsoluteAxisCode::ABS_HAT0Y => {
                (&mut self.y_active, egui::Key::ArrowUp, egui::Key::ArrowDown)
            }
            _ => return None,
        };
        if value.abs() < RELEASE_THRESHOLD {
            *active = false;
            return None;
        }
        if *active || value.abs() < ENGAGE_THRESHOLD {
            return None;
        }
        *active = true;
        Some(if value.is_sign_negative() {
            negative
        } else {
            positive
        })
    }
}

fn gamepad_button_key(button: KeyCode) -> Option<egui::Key> {
    match button {
        KeyCode::BTN_DPAD_UP => Some(egui::Key::ArrowUp),
        KeyCode::BTN_DPAD_DOWN => Some(egui::Key::ArrowDown),
        KeyCode::BTN_DPAD_LEFT => Some(egui::Key::ArrowLeft),
        KeyCode::BTN_DPAD_RIGHT => Some(egui::Key::ArrowRight),
        // South is the Steam Deck's A button. Enter activates the currently
        // focused egui button, checkbox, or tab.
        KeyCode::BTN_SOUTH => Some(egui::Key::Enter),
        _ => None,
    }
}

fn normalized_axis_value(value: i32, range: Option<(i32, i32)>) -> f32 {
    let Some((minimum, maximum)) = range.filter(|(minimum, maximum)| minimum < maximum) else {
        return value.signum() as f32;
    };
    (((value - minimum) as f32 / (maximum - minimum) as f32) * 2.0 - 1.0).clamp(-1.0, 1.0)
}

fn controller_axis_range(device: &Device, wanted: AbsoluteAxisCode) -> Option<(i32, i32)> {
    device
        .get_absinfo()
        .ok()?
        .find(|(axis, _)| *axis == wanted)
        .map(|(_, info)| (info.minimum(), info.maximum()))
}

fn is_controller(device: &Device) -> bool {
    let Some(keys) = device.supported_keys() else {
        return false;
    };
    keys.contains(KeyCode::BTN_SOUTH)
        && (keys.contains(KeyCode::BTN_DPAD_UP)
            || device
                .supported_absolute_axes()
                .is_some_and(|axes| axes.contains(AbsoluteAxisCode::ABS_X)))
}

fn controller_event_key(
    event: EventSummary,
    sticks: &mut StickState,
    x_range: Option<(i32, i32)>,
    y_range: Option<(i32, i32)>,
) -> Option<egui::Key> {
    match event {
        EventSummary::Key(_, button, 1) => gamepad_button_key(button),
        EventSummary::AbsoluteAxis(_, axis, value) => {
            let range = match axis {
                AbsoluteAxisCode::ABS_X => x_range,
                AbsoluteAxisCode::ABS_Y => y_range,
                AbsoluteAxisCode::ABS_HAT0X | AbsoluteAxisCode::ABS_HAT0Y => Some((-1, 1)),
                _ => None,
            };
            sticks.key_for_axis(axis, normalized_axis_value(value, range))
        }
        _ => None,
    }
}

impl GamepadNavigation {
    fn start(context: egui::Context) -> Option<Self> {
        let (sender, keys) = mpsc::channel();
        thread::Builder::new()
            .name("macdeck-gamepad".to_owned())
            .spawn(move || {
                // Missing device permissions must degrade safely to normal
                // keyboard/mouse use; the receiver never needs root solely
                // for controller navigation.
                let Some(mut device) = evdev::enumerate()
                    .find_map(|(_, device)| is_controller(&device).then_some(device))
                else {
                    return;
                };
                let x_range = controller_axis_range(&device, AbsoluteAxisCode::ABS_X);
                let y_range = controller_axis_range(&device, AbsoluteAxisCode::ABS_Y);
                let mut sticks = StickState::default();
                while let Ok(events) = device.fetch_events() {
                    for event in events {
                        if let Some(key) =
                            controller_event_key(event.destructure(), &mut sticks, x_range, y_range)
                        {
                            if sender.send(key).is_err() {
                                return;
                            }
                            // evdev is not part of eframe's event loop. Wake
                            // the UI only for a real controller action,
                            // avoiding a polling repaint loop and its battery
                            // cost.
                            context.request_repaint();
                        }
                    }
                }
            })
            .ok()?;
        Some(Self { keys })
    }

    fn drain_keys(&self) -> Vec<egui::Key> {
        self.keys.try_iter().collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Control,
    Settings,
}

/// Steam Input normally exposes the D-pad as arrows and the primary action as
/// Enter/Space. Egui makes widgets keyboard-focusable, but keeps focus order
/// dependent on the layout by default. The explicit order keeps the controller
/// path stable when the Settings page grows or becomes scrollable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FocusTarget {
    ControlTab,
    SettingsTab,
    ToggleReceiver,
    RefreshControlAddresses,
    ListenAddress,
    PairingSecret,
    SaveSettings,
    CopySecret,
    GenerateSecret,
    SyncLayout,
    SyncClipboard,
    LayoutMappings,
    CheckLayoutSync,
    RefreshSettingsAddresses,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FocusDirection {
    Previous,
    Next,
}

impl FocusTarget {
    const CONTROL_ORDER: [Self; 4] = [
        Self::ControlTab,
        Self::SettingsTab,
        Self::ToggleReceiver,
        Self::RefreshControlAddresses,
    ];
    const SETTINGS_ORDER: [Self; 12] = [
        Self::ControlTab,
        Self::SettingsTab,
        Self::ListenAddress,
        Self::PairingSecret,
        Self::SaveSettings,
        Self::CopySecret,
        Self::GenerateSecret,
        Self::SyncLayout,
        Self::SyncClipboard,
        Self::LayoutMappings,
        Self::CheckLayoutSync,
        Self::RefreshSettingsAddresses,
    ];

    fn order(tab: Tab, layout_sync_enabled: bool) -> Vec<Self> {
        match tab {
            Tab::Control => Self::CONTROL_ORDER.to_vec(),
            Tab::Settings if layout_sync_enabled => Self::SETTINGS_ORDER.to_vec(),
            Tab::Settings => Self::SETTINGS_ORDER
                .into_iter()
                .filter(|target| *target != Self::LayoutMappings)
                .collect(),
        }
    }

    fn is_text_entry(self) -> bool {
        matches!(
            self,
            Self::ListenAddress | Self::PairingSecret | Self::LayoutMappings
        )
    }
}

fn next_focus_target(
    current: Option<FocusTarget>,
    direction: FocusDirection,
    tab: Tab,
    layout_sync_enabled: bool,
) -> Option<FocusTarget> {
    let order = FocusTarget::order(tab, layout_sync_enabled);
    let current_index = current.and_then(|target| order.iter().position(|item| *item == target));
    let next_index = match (direction, current_index) {
        (FocusDirection::Previous, Some(0) | None) => order.len().checked_sub(1),
        (FocusDirection::Previous, Some(index)) => Some(index - 1),
        (FocusDirection::Next, Some(index)) => Some((index + 1) % order.len()),
        (FocusDirection::Next, None) => Some(0),
    }?;
    order.get(next_index).copied()
}

impl ReceiverApp<ReceiverProcess> {
    fn new() -> Self {
        let settings_path = settings_path();
        let (settings, status) = load_settings_or_default(&settings_path);
        Self {
            settings,
            settings_path,
            process: ReceiverProcess { child: None },
            selected_tab: Tab::Control,
            ip_addresses: local_ipv4_addresses(),
            status,
            gamepad: None,
            gamepad_start_attempted: false,
        }
    }
}

impl<P: ReceiverSupervisor> ReceiverApp<P> {
    #[cfg(test)]
    fn with_process(settings: ReceiverSettings, settings_path: PathBuf, process: P) -> Self {
        Self {
            settings,
            settings_path,
            process,
            selected_tab: Tab::Control,
            ip_addresses: Vec::new(),
            status: None,
            gamepad: None,
            gamepad_start_attempted: true,
        }
    }

    fn save_settings(&mut self) -> Result<(), String> {
        self.settings.validates()?;
        persist_settings(&self.settings_path, &self.settings)
            .map_err(|error| format!("Could not save settings: {error}"))
    }

    fn apply_settings(&mut self) -> Result<(), String> {
        if self.process.is_running() {
            // The receiver receives its settings only at process launch. Restart
            // it so the visible pairing secret and bind address are always the
            // values protecting the active listener.
            self.process.stop()?;
            self.save_settings()?;
            self.process.start(&self.settings)
        } else {
            self.save_settings()
        }
    }

    fn toggle_receiver(&mut self) {
        let result = if self.process.is_running() {
            self.process.stop()
        } else {
            self.save_settings()
                .and_then(|()| self.process.start(&self.settings))
        };
        self.status = result.err();
    }

    fn apply_settings_from_ui(&mut self) {
        self.status = match self.apply_settings() {
            Ok(()) => Some("Settings saved.".to_owned()),
            Err(error) => Some(error),
        };
    }

    fn copied_secret_status(&mut self) {
        self.status = Some("Pairing secret copied to the clipboard.".to_owned());
    }

    fn regenerate_pairing_secret_from_ui(&mut self) {
        self.status = match self.regenerate_pairing_secret() {
            Ok(()) => Some(
                "New pairing secret generated. Mac input is disabled until you enable it."
                    .to_owned(),
            ),
            Err(error) => Some(error),
        };
    }

    fn regenerate_pairing_secret(&mut self) -> Result<(), String> {
        // Stop first: the receiver owns the uinput device and its teardown
        // releases injected state before any pairing credential changes.
        self.process.stop()?;
        let previous_secret = self.settings.pairing_secret.clone();
        self.settings.pairing_secret = PairingSecret::generate().to_base64();
        if let Err(error) = self.save_settings() {
            // The receiver is safely stopped. Restore the value displayed in the
            // UI to the last persisted credential so the user never copies a key
            // that failed to save.
            self.settings.pairing_secret = previous_secret;
            return Err(error);
        }
        Ok(())
    }

    fn refresh_ip_addresses(&mut self) {
        self.ip_addresses = local_ipv4_addresses();
    }

    fn control_ui(&mut self, ui: &mut egui::Ui, focus_widgets: &mut Vec<(FocusTarget, Id)>) {
        let running = self.process.is_running();
        let (label, color) = if running {
            ("Mac input is enabled", Color32::from_rgb(55, 180, 100))
        } else {
            ("Mac input is disabled", Color32::from_rgb(170, 75, 75))
        };
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(label).size(22.0).color(color).strong());
            ui.add_space(12.0);
            let toggle_label = if running {
                "Disable Mac input"
            } else {
                "Enable Mac input"
            };
            let response = ui.add_sized([260.0, 48.0], egui::Button::new(toggle_label));
            track_focus(focus_widgets, FocusTarget::ToggleReceiver, &response);
            if response.clicked() {
                self.toggle_receiver();
            }
            ui.add_space(12.0);
            ui.label("When disabled, the receiver stops and releases all injected keys.");
            ui.add_space(20.0);
            ui.separator();
            ui.add_space(12.0);
            ui.label(RichText::new("Steam Deck IP addresses").strong());
            if self.ip_addresses.is_empty() {
                ui.label("No non-loopback IP address detected.");
            } else {
                for address in &self.ip_addresses {
                    ui.monospace(address.to_string());
                }
            }
            let response = ui.button("Refresh addresses");
            track_focus(
                focus_widgets,
                FocusTarget::RefreshControlAddresses,
                &response,
            );
            if response.clicked() {
                self.refresh_ip_addresses();
            }
        });
    }

    fn settings_ui(
        &mut self,
        ui: &mut egui::Ui,
        context: &egui::Context,
        focus_widgets: &mut Vec<(FocusTarget, Id)>,
    ) {
        ui.heading("Receiver settings");
        ui.add_space(8.0);
        ui.small(
            "Controller: use the D-pad to move focus and A/Enter to activate. ".to_owned()
                + "For a text field, use Steam + X to open the on-screen keyboard.",
        );
        ui.add_space(12.0);
        ui.label("Listen address");
        let response = ui.text_edit_singleline(&mut self.settings.bind);
        track_focus(focus_widgets, FocusTarget::ListenAddress, &response);
        ui.add_space(12.0);
        ui.label("Pairing secret");
        let response = ui.text_edit_singleline(&mut self.settings.pairing_secret);
        track_focus(focus_widgets, FocusTarget::PairingSecret, &response);
        ui.horizontal_wrapped(|ui| {
            let response = ui.button("Save settings");
            track_focus(focus_widgets, FocusTarget::SaveSettings, &response);
            if response.clicked() {
                self.apply_settings_from_ui();
            }
            let response = ui.button("Copy secret");
            track_focus(focus_widgets, FocusTarget::CopySecret, &response);
            if response.clicked() {
                context.copy_text(self.settings.pairing_secret.clone());
                self.copied_secret_status();
            }
            let response = ui.button("Generate new secret");
            track_focus(focus_widgets, FocusTarget::GenerateSecret, &response);
            if response.clicked() {
                // Do not leave a receiver running with a secret different from the
                // one shown in the UI. Stopping it destroys its uinput device, so
                // Linux releases the virtual keyboard state before re-pairing.
                self.regenerate_pairing_secret_from_ui();
            }
        });
        ui.add_space(16.0);
        let response = ui.checkbox(
            &mut self.settings.sync_layout_from_mac,
            "Sync active layout from Mac",
        );
        track_focus(focus_widgets, FocusTarget::SyncLayout, &response);
        ui.small("When enabled, the Mac chooses the active Deck layout before it sends keys.");
        if self.settings.sync_layout_from_mac {
            ui.add_space(6.0);
            ui.label("Manual overrides (optional; one per line: Mac input-source ID = Deck layout number)");
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.settings.layout_mappings)
                    .desired_rows(4)
                    .hint_text("com.apple.keylayout.US = 0\ncom.apple.keylayout.Ukrainian = 1"),
            );
            track_focus(focus_widgets, FocusTarget::LayoutMappings, &response);
            ui.small("Standard layouts are matched automatically. Use an override only for a custom or ambiguous layout; numbers start at 0 in Plasma Settings > Keyboard > Layouts.");
            let response = ui.button("Check Deck layout sync");
            track_focus(focus_widgets, FocusTarget::CheckLayoutSync, &response);
            if response.clicked() {
                self.status = Some(match deck_layout_sync_diagnostics(&self.settings) {
                    Ok(status) => status,
                    Err(error) => format!("Deck layout sync check failed: {error}"),
                });
            }
            ui.small("Reports the active Plasma layout and layouts MacDeck can inspect; it does not send any keys.");
        }
        ui.add_space(16.0);
        let response = ui.checkbox(
            &mut self.settings.sync_clipboard_from_mac,
            "Accept plain-text clipboard from Mac",
        );
        track_focus(focus_widgets, FocusTarget::SyncClipboard, &response);
        ui.small("Off by default. Desktop Mode only; accepts new plain-text clipboard changes while connected. Contents are encrypted, never logged or saved, and are not sent back to the Mac.");
        ui.add_space(16.0);
        ui.heading("Steam Deck IP addresses");
        for address in &self.ip_addresses {
            ui.monospace(address.to_string());
        }
        let response = ui.button("Refresh addresses");
        track_focus(
            focus_widgets,
            FocusTarget::RefreshSettingsAddresses,
            &response,
        );
        if response.clicked() {
            self.refresh_ip_addresses();
        }
        ui.add_space(8.0);
        ui.small("Use one of these addresses as --host in the Mac sender settings.");
    }
}

fn track_focus(
    focus_widgets: &mut Vec<(FocusTarget, Id)>,
    target: FocusTarget,
    response: &egui::Response,
) {
    // Requesting focus happens after the UI has been laid out. On the next
    // frame, bring the focused control into view so D-pad navigation also
    // scrolls the Settings page instead of leaving a focused control below
    // the visible viewport.
    if response.has_focus() {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    focus_widgets.push((target, response.id));
}

fn handle_gamepad_focus(
    context: &egui::Context,
    focus_widgets: &[(FocusTarget, Id)],
    tab: Tab,
    layout_sync_enabled: bool,
    controller_navigation_enabled: bool,
) {
    if !controller_navigation_enabled {
        return;
    }
    let focused_id = context.memory(|memory| memory.focused());
    let focused_target = focus_widgets
        .iter()
        .find_map(|(target, id)| (Some(*id) == focused_id).then_some(*target));

    let direction = context.input(|input| {
        if input.key_pressed(egui::Key::ArrowUp) {
            Some(FocusDirection::Previous)
        } else if input.key_pressed(egui::Key::ArrowDown) {
            Some(FocusDirection::Next)
        } else if focused_target.is_some_and(FocusTarget::is_text_entry) {
            // In Gaming Mode, vertical D-pad movement must be able to leave a
            // text field. Keep left/right available to Steam's on-screen
            // keyboard for caret movement.
            None
        } else if input.key_pressed(egui::Key::ArrowLeft) {
            Some(FocusDirection::Previous)
        } else if input.key_pressed(egui::Key::ArrowRight) {
            Some(FocusDirection::Next)
        } else {
            None
        }
    });

    if let Some(direction) = direction {
        if let Some(next) = next_focus_target(focused_target, direction, tab, layout_sync_enabled) {
            if let Some((_, id)) = focus_widgets.iter().find(|(target, _)| *target == next) {
                context.memory_mut(|memory| memory.request_focus(*id));
                context.request_repaint();
            }
        }
    }

    if let Some(focused_id) = context.memory(|memory| memory.focused()) {
        context.highlight_widget(focused_id);
    }
}

/// Gamescope is the SteamOS compositor used by Gaming Mode. Desktop Mode keeps
/// ordinary editor arrow-key behavior, while Gaming Mode reserves the D-pad
/// for controller focus navigation.
fn controller_navigation_enabled_from_environment(
    current_desktop: Option<&str>,
    session_desktop: Option<&str>,
    steam_app_id: Option<&str>,
    steam_game_id: Option<&str>,
) -> bool {
    current_desktop.is_some_and(|desktop| desktop.to_ascii_lowercase().contains("gamescope"))
        || session_desktop
            .is_some_and(|desktop| desktop.to_ascii_lowercase().contains("gamescope"))
        // Steam sets SteamAppId for applications launched from Gaming Mode.
        // SteamGameId is not set consistently for non-Steam shortcuts, so
        // accepting both is essential for the installed desktop entry.
        || steam_app_id.is_some_and(|id| !id.trim().is_empty())
        || steam_game_id.is_some_and(|id| !id.trim().is_empty())
}

fn controller_navigation_enabled() -> bool {
    controller_navigation_enabled_from_environment(
        env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        env::var("XDG_SESSION_DESKTOP").ok().as_deref(),
        env::var("SteamAppId")
            .or_else(|_| env::var("STEAM_APP_ID"))
            .ok()
            .as_deref(),
        env::var("SteamGameId").ok().as_deref(),
    )
}

fn inject_gamepad_key_presses(context: &egui::Context, keys: impl IntoIterator<Item = egui::Key>) {
    context.input_mut(|input| {
        for key in keys {
            // A press event is enough for egui's focus/navigation handling.
            // Add the matching release immediately so a controller action
            // cannot leave a synthetic key held down in the text editor.
            for pressed in [true, false] {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    });
}

impl eframe::App for ReceiverApp<ReceiverProcess> {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.gamepad_start_attempted {
            self.gamepad_start_attempted = true;
            self.gamepad = GamepadNavigation::start(context.clone());
        }
        let gamepad_keys = self
            .gamepad
            .as_ref()
            .map(GamepadNavigation::drain_keys)
            .unwrap_or_default();
        let received_gamepad_navigation = !gamepad_keys.is_empty();
        inject_gamepad_key_presses(context, gamepad_keys);
        let mut focus_widgets = Vec::new();
        egui::TopBottomPanel::top("tabs").show(context, |ui| {
            ui.horizontal(|ui| {
                let response = ui.selectable_value(&mut self.selected_tab, Tab::Control, "Control");
                track_focus(&mut focus_widgets, FocusTarget::ControlTab, &response);
                let response =
                    ui.selectable_value(&mut self.selected_tab, Tab::Settings, "Settings");
                track_focus(&mut focus_widgets, FocusTarget::SettingsTab, &response);
                if response.changed() {
                    self.refresh_ip_addresses();
                }
            });
        });
        if let Some(status) = &self.status {
            egui::TopBottomPanel::bottom("status").show(context, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Status:").strong());
                    ui.label(status);
                });
            });
        }
        egui::CentralPanel::default()
            // Keep normal breathing room around the content, but reserve no
            // empty strip on the right: the scroll bar should align with the
            // window's content edge.
            .frame(egui::Frame::default().inner_margin(egui::Margin {
                left: 8,
                right: 0,
                top: 8,
                bottom: 0,
            }))
            .show(context, |ui| {
                egui::ScrollArea::vertical()
                    // Fill the available panel width. With auto-shrink enabled,
                    // the area followed the narrow form content and left a wide
                    // blank strip before the scrollbar.
                    .auto_shrink([false, false])
                    // Explicitly bound the scroll area to the available panel.
                    // Without this bound, a Gamescope window can grow to its
                    // content height and leave the bottom of Settings clipped
                    // instead of scrollable.
                    .max_height(ui.available_height())
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .id_salt("main-content")
                    .show(ui, |ui| match self.selected_tab {
                        Tab::Control => self.control_ui(ui, &mut focus_widgets),
                        Tab::Settings => self.settings_ui(ui, context, &mut focus_widgets),
                    });
            });
        handle_gamepad_focus(
            context,
            &focus_widgets,
            self.selected_tab,
            self.settings.sync_layout_from_mac,
            received_gamepad_navigation || controller_navigation_enabled(),
        );
        // There is no animation. When disabled, egui can sleep until a real
        // input event arrives. Poll a running child only once per second so a
        // crash is eventually reflected in Control without waking the Deck at
        // a display-frame rate.
        if self.selected_tab == Tab::Control && self.process.is_running() {
            context.request_repaint_after(Duration::from_secs(1));
        }
    }
}

fn receiver_executable() -> Result<PathBuf, String> {
    let current =
        env::current_exe().map_err(|error| format!("Could not resolve UI path: {error}"))?;
    receiver_executable_beside(&current)
}

fn receiver_executable_beside(current: &Path) -> Result<PathBuf, String> {
    let sibling = current.with_file_name("mac-deck-receiver");
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(
            "The mac-deck-receiver binary must be installed beside mac-deck-receiver-ui."
                .to_owned(),
        )
    }
}

fn layout_sync_arguments(settings: &ReceiverSettings) -> Result<Vec<String>, String> {
    if !settings.sync_layout_from_mac {
        return Ok(Vec::new());
    }
    let mappings = parse_layout_mappings(&settings.layout_mappings)?;
    let mut arguments = vec!["--sync-layout-from-mac".to_owned()];
    for mapping in mappings {
        arguments.push("--layout-mapping".to_owned());
        arguments.push(mapping);
    }
    Ok(arguments)
}

fn clipboard_sync_arguments(settings: &ReceiverSettings) -> Vec<String> {
    settings
        .sync_clipboard_from_mac
        .then(|| "--sync-clipboard-from-mac".to_owned())
        .into_iter()
        .collect()
}

fn layout_status_arguments(settings: &ReceiverSettings) -> Result<Vec<String>, String> {
    let mappings = parse_layout_mappings(&settings.layout_mappings)?;
    let mut arguments = vec!["layout-status".to_owned()];
    for mapping in mappings {
        arguments.push("--layout-mapping".to_owned());
        arguments.push(mapping);
    }
    Ok(arguments)
}

fn deck_layout_sync_diagnostics(settings: &ReceiverSettings) -> Result<String, String> {
    let executable = receiver_executable()?;
    let output = Command::new(executable)
        .args(layout_status_arguments(settings)?)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not run the layout diagnostic: {error}"))?;
    if output.status.success() {
        let status = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if status.is_empty() {
            Err("the layout diagnostic returned no status".to_owned())
        } else {
            Ok(status)
        }
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if error.is_empty() {
            "the receiver rejected the layout diagnostic".to_owned()
        } else {
            error
        })
    }
}

fn parse_layout_mappings(text: &str) -> Result<Vec<String>, String> {
    let mut mappings = Vec::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((source, layout_index)) = line.split_once('=') else {
            return Err(format!(
                "Layout mapping on line {} must use source ID = Deck layout number.",
                line_number + 1
            ));
        };
        let source = source.trim();
        let layout_index = layout_index.trim();
        let valid_source = !source.is_empty()
            && source.len() <= 128
            && source
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid_source {
            return Err(format!(
                "Layout mapping on line {} has an invalid Mac input-source ID.",
                line_number + 1
            ));
        }
        let index = layout_index.parse::<u8>().map_err(|_| {
            format!(
                "Layout mapping on line {} needs a Deck layout number from 0 to 255.",
                line_number + 1
            )
        })?;
        mappings.push(format!("{source}={index}"));
    }
    Ok(mappings)
}

fn settings_path() -> PathBuf {
    let root = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    root.join(CONFIG_DIRECTORY).join(CONFIG_FILE)
}

fn load_settings(path: &Path) -> io::Result<ReceiverSettings> {
    let text = fs::read_to_string(path)?;
    toml::from_str(&text).map_err(io::Error::other)
}

fn load_settings_or_default(path: &Path) -> (ReceiverSettings, Option<String>) {
    match load_settings(path) {
        Ok(settings) => (settings, None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            (ReceiverSettings::default(), None)
        }
        Err(error) => (
            ReceiverSettings::default(),
            Some(format!(
                "Could not load settings; using a new pairing secret: {error}"
            )),
        ),
    }
}

fn persist_settings(path: &Path, settings: &ReceiverSettings) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    set_private_mode(parent, 0o700)?;
    let text = toml::to_string(settings).map_err(io::Error::other)?;
    let temporary = path.with_extension("toml.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    set_private_mode(&temporary, 0o600)?;
    use std::io::Write;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    set_private_mode(path, 0o600)
}

#[cfg(unix)]
fn set_private_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_private_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

fn local_ipv4_addresses() -> Vec<Ipv4Addr> {
    let Ok(interfaces) = get_if_addrs() else {
        return Vec::new();
    };
    local_ipv4_addresses_from(interfaces)
}

fn local_ipv4_addresses_from(interfaces: impl IntoIterator<Item = Interface>) -> Vec<Ipv4Addr> {
    interfaces
        .into_iter()
        .filter_map(|interface| match interface.addr {
            IfAddr::V4(address) if !address.ip.is_loopback() => Some(address.ip),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn main() -> eframe::Result<()> {
    // Wayland needs both a window icon and an application ID that matches the
    // desktop file. Without them Plasma uses eframe's default icon in the
    // task manager even when the launcher itself has a correctly installed
    // icon.
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../assets/MacDeckInputBridgeIcon.png"
    ));
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(GAMING_MODE_PREFERRED_SIZE)
        .with_min_inner_size(MINIMUM_WINDOW_SIZE)
        .with_app_id(APP_ID);
    let viewport = match icon {
        Ok(icon) => viewport.with_icon(Arc::new(icon)),
        Err(error) => {
            // The launcher still supplies the icon through its desktop entry.
            // Never prevent the receiver from starting merely because a future
            // packaging change omits an image decoder.
            eprintln!("Could not decode the bundled MacDeck icon: {error}");
            viewport
        }
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_context| Ok(Box::new(ReceiverApp::new()))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use get_if_addrs::Ifv4Addr;

    #[derive(Default)]
    struct FakeReceiverProcess {
        running: bool,
        calls: Vec<&'static str>,
        start_error: Option<String>,
        stop_error: Option<String>,
    }

    impl ReceiverSupervisor for FakeReceiverProcess {
        fn is_running(&mut self) -> bool {
            self.running
        }

        fn start(&mut self, _settings: &ReceiverSettings) -> Result<(), String> {
            self.calls.push("start");
            if let Some(error) = &self.start_error {
                return Err(error.clone());
            }
            self.running = true;
            Ok(())
        }

        fn stop(&mut self) -> Result<(), String> {
            self.calls.push("stop");
            if let Some(error) = &self.stop_error {
                return Err(error.clone());
            }
            self.running = false;
            Ok(())
        }
    }

    #[test]
    fn generated_default_settings_are_valid() {
        ReceiverSettings::default().validates().unwrap();
    }

    #[test]
    fn layout_mappings_support_any_safe_mac_source_identifier() {
        assert_eq!(
            parse_layout_mappings("com.apple.keylayout.British = 3\norg.example.Custom_42 = 12")
                .unwrap(),
            [
                "com.apple.keylayout.British=3".to_owned(),
                "org.example.Custom_42=12".to_owned()
            ]
        );
        assert!(parse_layout_mappings("com.apple.keylayout.US = not-a-number").is_err());
        assert!(parse_layout_mappings("not a mapping").is_err());
    }

    #[test]
    fn layout_status_uses_the_receiver_diagnostic_without_pairing_or_uinput() {
        let settings = ReceiverSettings {
            layout_mappings: "com.apple.keylayout.US = 0".to_owned(),
            ..ReceiverSettings::default()
        };
        assert_eq!(
            layout_status_arguments(&settings).unwrap(),
            [
                "layout-status".to_owned(),
                "--layout-mapping".to_owned(),
                "com.apple.keylayout.US=0".to_owned(),
            ]
        );
    }

    #[test]
    fn enabled_layout_sync_allows_automatic_matching_and_validates_manual_overrides() {
        let settings = ReceiverSettings {
            sync_layout_from_mac: true,
            ..ReceiverSettings::default()
        };
        assert!(settings.validates().is_ok());
        let settings = ReceiverSettings {
            layout_mappings: "com.apple.keylayout.US = invalid".to_owned(),
            ..settings
        };
        assert!(settings.validates().is_err());
    }

    #[test]
    fn clipboard_sync_is_off_by_default_and_only_adds_an_explicit_receiver_flag() {
        let disabled = ReceiverSettings::default();
        assert!(clipboard_sync_arguments(&disabled).is_empty());
        let enabled = ReceiverSettings {
            sync_clipboard_from_mac: true,
            ..disabled
        };
        assert_eq!(
            clipboard_sync_arguments(&enabled),
            vec!["--sync-clipboard-from-mac".to_owned()]
        );
        assert!(FocusTarget::order(Tab::Settings, false).contains(&FocusTarget::SyncClipboard));
    }

    #[test]
    fn bundled_dock_icon_is_a_decodable_png() {
        assert!(eframe::icon_data::from_png_bytes(include_bytes!(
            "../../assets/MacDeckInputBridgeIcon.png"
        ))
        .is_ok());
    }

    #[test]
    fn gaming_mode_uses_a_large_preferred_viewport_with_a_safe_minimum() {
        assert_eq!(GAMING_MODE_PREFERRED_SIZE, [960.0, 720.0]);
        assert_eq!(MINIMUM_WINDOW_SIZE, [480.0, 360.0]);
    }

    #[test]
    fn gamepad_focus_order_includes_every_visible_control() {
        assert_eq!(
            FocusTarget::order(Tab::Control, false),
            vec![
                FocusTarget::ControlTab,
                FocusTarget::SettingsTab,
                FocusTarget::ToggleReceiver,
                FocusTarget::RefreshControlAddresses,
            ]
        );
        let settings_without_mappings = FocusTarget::order(Tab::Settings, false);
        assert!(!settings_without_mappings.contains(&FocusTarget::LayoutMappings));
        let settings_with_mappings = FocusTarget::order(Tab::Settings, true);
        assert!(settings_with_mappings.contains(&FocusTarget::LayoutMappings));
        assert!(settings_with_mappings.contains(&FocusTarget::ListenAddress));
        assert!(settings_with_mappings.contains(&FocusTarget::PairingSecret));
        assert!(settings_with_mappings.contains(&FocusTarget::SyncLayout));
        assert!(settings_with_mappings.contains(&FocusTarget::CheckLayoutSync));
        assert!(settings_with_mappings.contains(&FocusTarget::RefreshSettingsAddresses));
    }

    #[test]
    fn controller_navigation_keeps_horizontal_text_editing_and_uses_vertical_dpad_to_exit() {
        assert!(FocusTarget::ListenAddress.is_text_entry());
        assert!(FocusTarget::PairingSecret.is_text_entry());
        assert!(FocusTarget::LayoutMappings.is_text_entry());
        assert!(!FocusTarget::ToggleReceiver.is_text_entry());
        assert!(!FocusTarget::SyncLayout.is_text_entry());
        assert_eq!(
            next_focus_target(
                Some(FocusTarget::ListenAddress),
                FocusDirection::Next,
                Tab::Settings,
                false,
            ),
            Some(FocusTarget::PairingSecret)
        );
    }

    #[test]
    fn controller_navigation_is_enabled_in_gaming_mode() {
        assert!(controller_navigation_enabled_from_environment(
            Some("gamescope"),
            None,
            None,
            None
        ));
        assert!(controller_navigation_enabled_from_environment(
            Some("KDE"),
            None,
            Some("480"),
            Some("123456")
        ));
        assert!(controller_navigation_enabled_from_environment(
            Some("KDE"),
            None,
            Some("480"),
            None
        ));
        assert!(controller_navigation_enabled_from_environment(
            Some("KDE"),
            Some("gamescope"),
            None,
            None
        ));
        assert!(!controller_navigation_enabled_from_environment(
            Some("KDE"),
            Some("KDE"),
            None,
            None
        ));
    }

    #[test]
    fn deck_gamepad_controls_map_to_navigation_and_activation_keys() {
        assert_eq!(
            gamepad_button_key(KeyCode::BTN_DPAD_UP),
            Some(egui::Key::ArrowUp)
        );
        assert_eq!(
            gamepad_button_key(KeyCode::BTN_DPAD_DOWN),
            Some(egui::Key::ArrowDown)
        );
        assert_eq!(
            gamepad_button_key(KeyCode::BTN_DPAD_LEFT),
            Some(egui::Key::ArrowLeft)
        );
        assert_eq!(
            gamepad_button_key(KeyCode::BTN_DPAD_RIGHT),
            Some(egui::Key::ArrowRight)
        );
        assert_eq!(
            gamepad_button_key(KeyCode::BTN_SOUTH),
            Some(egui::Key::Enter)
        );
        assert_eq!(gamepad_button_key(KeyCode::BTN_NORTH), None);
    }

    #[test]
    fn left_stick_emits_one_direction_until_released() {
        let mut sticks = StickState::default();
        assert_eq!(
            sticks.key_for_axis(AbsoluteAxisCode::ABS_Y, -0.8),
            Some(egui::Key::ArrowUp)
        );
        assert_eq!(sticks.key_for_axis(AbsoluteAxisCode::ABS_Y, -0.9), None);
        assert_eq!(sticks.key_for_axis(AbsoluteAxisCode::ABS_Y, 0.0), None);
        assert_eq!(
            sticks.key_for_axis(AbsoluteAxisCode::ABS_Y, 0.8),
            Some(egui::Key::ArrowDown)
        );
    }

    #[test]
    fn gamepad_focus_wraps_predictably_between_controls() {
        assert_eq!(
            next_focus_target(
                Some(FocusTarget::ControlTab),
                FocusDirection::Previous,
                Tab::Control,
                false,
            ),
            Some(FocusTarget::RefreshControlAddresses)
        );
        assert_eq!(
            next_focus_target(
                Some(FocusTarget::ToggleReceiver),
                FocusDirection::Next,
                Tab::Control,
                false,
            ),
            Some(FocusTarget::RefreshControlAddresses)
        );
        assert_eq!(
            next_focus_target(None, FocusDirection::Next, Tab::Settings, false),
            Some(FocusTarget::ControlTab)
        );
    }

    #[test]
    fn invalid_settings_are_rejected_before_starting_a_process() {
        let settings = ReceiverSettings {
            bind: "not an address".to_owned(),
            pairing_secret: "not base64".to_owned(),
            ..ReceiverSettings::default()
        };
        assert!(settings.validates().is_err());
    }

    #[test]
    fn invalid_settings_cannot_be_persisted_or_started_from_the_ui() {
        let directory = unique_test_directory("invalid-ui-settings");
        let path = directory.join(CONFIG_FILE);
        let mut app = ReceiverApp::with_process(
            ReceiverSettings {
                bind: "invalid address".to_owned(),
                pairing_secret: "not base64".to_owned(),
                ..ReceiverSettings::default()
            },
            path.clone(),
            FakeReceiverProcess::default(),
        );

        app.apply_settings_from_ui();
        assert!(app.status.as_deref().unwrap_or_default().contains("base64"));
        assert!(!path.exists());
        assert!(app.process.calls.is_empty());

        app.toggle_receiver();
        assert!(app.status.as_deref().unwrap_or_default().contains("base64"));
        assert!(app.process.calls.is_empty());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn missing_and_malformed_configuration_have_distinct_safe_states() {
        let directory = unique_test_directory("load-settings");
        let missing = directory.join("missing.toml");
        let (_, missing_status) = load_settings_or_default(&missing);
        assert!(missing_status.is_none());

        fs::create_dir_all(&directory).unwrap();
        let malformed = directory.join(CONFIG_FILE);
        fs::write(&malformed, "not = [valid").unwrap();
        let (_, malformed_status) = load_settings_or_default(&malformed);
        assert!(malformed_status
            .as_deref()
            .unwrap_or_default()
            .contains("Could not load settings"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn settings_round_trip_without_typed_content() {
        let settings = ReceiverSettings::default();
        let text = toml::to_string(&settings).unwrap();
        let decoded: ReceiverSettings = toml::from_str(&text).unwrap();
        assert_eq!(decoded.bind, settings.bind);
        assert_eq!(decoded.pairing_secret, settings.pairing_secret);
    }

    #[test]
    fn receiver_binary_must_be_installed_beside_the_ui() {
        let directory = unique_test_directory("receiver-binary");
        fs::create_dir_all(&directory).unwrap();
        let ui_path = directory.join("mac-deck-receiver-ui");
        fs::File::create(&ui_path).unwrap();
        assert!(receiver_executable_beside(&ui_path).is_err());

        let receiver_path = directory.join("mac-deck-receiver");
        fs::File::create(&receiver_path).unwrap();
        assert_eq!(receiver_executable_beside(&ui_path).unwrap(), receiver_path);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persisted_settings_can_be_loaded_without_a_typed_content_field() {
        let directory = unique_test_directory("settings");
        let path = directory.join(CONFIG_FILE);
        let settings = ReceiverSettings::default();
        persist_settings(&path, &settings).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("pairing_secret"));
        assert!(!text.contains("key_event"));
        let loaded = load_settings(&path).unwrap();
        assert_eq!(loaded.bind, settings.bind);
        assert_eq!(loaded.pairing_secret, settings.pairing_secret);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn apply_settings_persists_when_receiver_is_disabled() {
        let directory = unique_test_directory("apply-settings");
        let path = directory.join(CONFIG_FILE);
        let mut app = ReceiverApp {
            settings: ReceiverSettings::default(),
            settings_path: path.clone(),
            process: ReceiverProcess { child: None },
            selected_tab: Tab::Settings,
            ip_addresses: Vec::new(),
            status: None,
            gamepad: None,
            gamepad_start_attempted: true,
        };
        app.settings.bind = "127.0.0.1:4343".to_owned();
        app.apply_settings().unwrap();
        assert_eq!(load_settings(&path).unwrap().bind, "127.0.0.1:4343");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn control_toggle_starts_then_stops_receiver_without_spawning_a_real_process() {
        let directory = unique_test_directory("control-toggle");
        let path = directory.join(CONFIG_FILE);
        let mut app = ReceiverApp::with_process(
            ReceiverSettings::default(),
            path.clone(),
            FakeReceiverProcess::default(),
        );

        app.toggle_receiver();
        assert!(app.process.running);
        assert_eq!(app.process.calls, ["start"]);
        assert!(path.exists());
        assert!(app.status.is_none());

        app.toggle_receiver();
        assert!(!app.process.running);
        assert_eq!(app.process.calls, ["start", "stop"]);
        assert!(app.status.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn control_toggle_surfaces_safe_start_error() {
        let directory = unique_test_directory("toggle-error");
        let path = directory.join(CONFIG_FILE);
        let mut app = ReceiverApp::with_process(
            ReceiverSettings::default(),
            path,
            FakeReceiverProcess {
                start_error: Some("receiver unavailable".to_owned()),
                ..Default::default()
            },
        );

        app.toggle_receiver();
        assert_eq!(app.process.calls, ["start"]);
        assert_eq!(app.status.as_deref(), Some("receiver unavailable"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn control_toggle_surfaces_stop_error_without_claiming_input_is_disabled() {
        let directory = unique_test_directory("toggle-stop-error");
        let mut app = ReceiverApp::with_process(
            ReceiverSettings::default(),
            directory.join(CONFIG_FILE),
            FakeReceiverProcess {
                running: true,
                stop_error: Some("receiver would not stop".to_owned()),
                ..Default::default()
            },
        );

        app.toggle_receiver();
        assert!(app.process.running);
        assert_eq!(app.process.calls, ["stop"]);
        assert_eq!(app.status.as_deref(), Some("receiver would not stop"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn saving_active_settings_restarts_receiver_with_persisted_values() {
        let directory = unique_test_directory("active-settings");
        let path = directory.join(CONFIG_FILE);
        let mut app = ReceiverApp::with_process(
            ReceiverSettings {
                bind: "127.0.0.1:4343".to_owned(),
                pairing_secret: PairingSecret::generate().to_base64(),
                ..ReceiverSettings::default()
            },
            path.clone(),
            FakeReceiverProcess {
                running: true,
                ..Default::default()
            },
        );

        app.apply_settings().unwrap();
        assert!(app.process.running);
        assert_eq!(app.process.calls, ["stop", "start"]);
        assert_eq!(load_settings(&path).unwrap().bind, "127.0.0.1:4343");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_active_settings_stops_before_persist_failure_and_never_restarts() {
        let directory = unique_test_directory("active-save-error");
        fs::create_dir_all(&directory).unwrap();
        let parent_file = directory.join("not-a-directory");
        fs::write(&parent_file, "x").unwrap();
        let path = parent_file.join(CONFIG_FILE);
        let mut app = ReceiverApp::with_process(
            ReceiverSettings::default(),
            path,
            FakeReceiverProcess {
                running: true,
                ..Default::default()
            },
        );

        app.apply_settings_from_ui();
        assert_eq!(app.process.calls, ["stop"]);
        assert!(!app.process.running);
        assert!(app
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("Could not save settings"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_active_settings_reports_restart_failure_after_persisting() {
        let directory = unique_test_directory("active-start-error");
        let path = directory.join(CONFIG_FILE);
        let mut app = ReceiverApp::with_process(
            ReceiverSettings::default(),
            path.clone(),
            FakeReceiverProcess {
                running: true,
                start_error: Some("uinput unavailable".to_owned()),
                ..Default::default()
            },
        );

        app.apply_settings_from_ui();
        assert_eq!(app.process.calls, ["stop", "start"]);
        assert!(!app.process.running);
        assert_eq!(app.status.as_deref(), Some("uinput unavailable"));
        assert!(path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn generating_secret_stops_input_before_persisting_rotated_secret() {
        let directory = unique_test_directory("rotate-secret");
        let path = directory.join(CONFIG_FILE);
        let old_secret = PairingSecret::generate().to_base64();
        let mut app = ReceiverApp::with_process(
            ReceiverSettings {
                bind: "0.0.0.0:4242".to_owned(),
                pairing_secret: old_secret.clone(),
                ..ReceiverSettings::default()
            },
            path.clone(),
            FakeReceiverProcess {
                running: true,
                ..Default::default()
            },
        );

        app.regenerate_pairing_secret().unwrap();

        assert_eq!(app.process.calls, ["stop"]);
        assert!(!app.process.running);
        assert_ne!(app.settings.pairing_secret, old_secret);
        assert_eq!(
            load_settings(&path).unwrap().pairing_secret,
            app.settings.pairing_secret
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn generating_secret_keeps_the_old_value_when_stop_or_persist_fails() {
        let old_secret = PairingSecret::generate().to_base64();
        let stop_error_directory = unique_test_directory("rotate-stop-error");
        let mut stop_error_app = ReceiverApp::with_process(
            ReceiverSettings {
                bind: "0.0.0.0:4242".to_owned(),
                pairing_secret: old_secret.clone(),
                ..ReceiverSettings::default()
            },
            stop_error_directory.join(CONFIG_FILE),
            FakeReceiverProcess {
                running: true,
                stop_error: Some("receiver would not stop".to_owned()),
                ..Default::default()
            },
        );
        stop_error_app.regenerate_pairing_secret_from_ui();
        assert_eq!(stop_error_app.settings.pairing_secret, old_secret);
        assert_eq!(
            stop_error_app.status.as_deref(),
            Some("receiver would not stop")
        );

        let persist_error_directory = unique_test_directory("rotate-persist-error");
        fs::create_dir_all(&persist_error_directory).unwrap();
        let parent_file = persist_error_directory.join("not-a-directory");
        fs::write(&parent_file, "x").unwrap();
        let mut persist_error_app = ReceiverApp::with_process(
            ReceiverSettings {
                bind: "0.0.0.0:4242".to_owned(),
                pairing_secret: old_secret.clone(),
                ..ReceiverSettings::default()
            },
            parent_file.join(CONFIG_FILE),
            FakeReceiverProcess {
                running: true,
                ..Default::default()
            },
        );
        persist_error_app.regenerate_pairing_secret_from_ui();
        assert_eq!(persist_error_app.process.calls, ["stop"]);
        assert_eq!(persist_error_app.settings.pairing_secret, old_secret);
        assert!(persist_error_app
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("Could not save settings"));
        let _ = fs::remove_dir_all(stop_error_directory);
        fs::remove_dir_all(persist_error_directory).unwrap();
    }

    #[test]
    fn copy_secret_has_a_non_sensitive_confirmation_message() {
        let directory = unique_test_directory("copy-secret");
        let secret = PairingSecret::generate().to_base64();
        let mut app = ReceiverApp::with_process(
            ReceiverSettings {
                bind: "0.0.0.0:4242".to_owned(),
                pairing_secret: secret.clone(),
                ..ReceiverSettings::default()
            },
            directory.join(CONFIG_FILE),
            FakeReceiverProcess::default(),
        );
        app.copied_secret_status();
        assert_eq!(
            app.status.as_deref(),
            Some("Pairing secret copied to the clipboard.")
        );
        assert!(!app.status.as_deref().unwrap_or_default().contains(&secret));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn ip_display_filters_loopback_and_deduplicates_addresses() {
        let interfaces = vec![
            ipv4_interface("lo", [127, 0, 0, 1]),
            ipv4_interface("wlan0", [192, 168, 1, 42]),
            ipv4_interface("docker0", [192, 168, 1, 42]),
            ipv4_interface("eth0", [10, 0, 0, 5]),
        ];
        assert_eq!(
            local_ipv4_addresses_from(interfaces),
            vec![Ipv4Addr::new(10, 0, 0, 5), Ipv4Addr::new(192, 168, 1, 42)]
        );
    }

    #[test]
    fn ip_display_handles_no_usable_addresses() {
        assert!(local_ipv4_addresses_from(vec![ipv4_interface("lo", [127, 0, 0, 1])]).is_empty());
        assert!(local_ipv4_addresses_from(Vec::new()).is_empty());
    }

    fn ipv4_interface(name: &str, octets: [u8; 4]) -> Interface {
        Interface {
            name: name.to_owned(),
            addr: IfAddr::V4(Ifv4Addr {
                ip: Ipv4Addr::from(octets),
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                broadcast: None,
            }),
        }
    }

    fn unique_test_directory(label: &str) -> PathBuf {
        let nanoseconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mac-deck-input-bridge-{label}-{}-{nanoseconds}",
            std::process::id()
        ))
    }
}
