use eframe::egui::{self, Color32, RichText};
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
    sync::Arc,
    time::{Duration, Instant},
};

const APP_NAME: &str = "MacDeck";
const APP_ID: &str = "mac-deck-input-bridge";
const CONFIG_DIRECTORY: &str = "mac-deck-input-bridge";
const CONFIG_FILE: &str = "receiver.toml";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ReceiverSettings {
    bind: String,
    pairing_secret: String,
}

impl Default for ReceiverSettings {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:4242".to_owned(),
            pairing_secret: PairingSecret::generate().to_base64(),
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
            })
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
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Tab {
    Control,
    Settings,
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

    fn control_ui(&mut self, ui: &mut egui::Ui) {
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
            if ui
                .add_sized([220.0, 42.0], egui::Button::new(toggle_label))
                .clicked()
            {
                self.toggle_receiver();
            }
            ui.add_space(12.0);
            ui.label("When disabled, the receiver stops and releases all injected keys.");
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.heading("Receiver settings");
        ui.add_space(8.0);
        ui.label("Listen address");
        ui.text_edit_singleline(&mut self.settings.bind);
        ui.add_space(12.0);
        ui.label("Pairing secret");
        ui.text_edit_singleline(&mut self.settings.pairing_secret);
        ui.horizontal(|ui| {
            if ui.button("Save settings").clicked() {
                self.apply_settings_from_ui();
            }
            if ui.button("Copy secret").clicked() {
                context.copy_text(self.settings.pairing_secret.clone());
                self.copied_secret_status();
            }
            if ui.button("Generate new secret").clicked() {
                // Do not leave a receiver running with a secret different from the
                // one shown in the UI. Stopping it destroys its uinput device, so
                // Linux releases the virtual keyboard state before re-pairing.
                self.regenerate_pairing_secret_from_ui();
            }
        });
        ui.add_space(16.0);
        ui.heading("Steam Deck IP addresses");
        for address in &self.ip_addresses {
            ui.monospace(address.to_string());
        }
        if ui.button("Refresh addresses").clicked() {
            self.refresh_ip_addresses();
        }
        ui.add_space(8.0);
        ui.small("Use one of these addresses as --host in the Mac sender settings.");
    }
}

impl eframe::App for ReceiverApp<ReceiverProcess> {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tabs").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_tab, Tab::Control, "Control");
                if ui
                    .selectable_value(&mut self.selected_tab, Tab::Settings, "Settings")
                    .changed()
                {
                    self.refresh_ip_addresses();
                }
            });
        });
        egui::CentralPanel::default().show(context, |ui| {
            match self.selected_tab {
                Tab::Control => self.control_ui(ui),
                Tab::Settings => self.settings_ui(ui, context),
            }
            if let Some(status) = &self.status {
                ui.add_space(16.0);
                ui.separator();
                ui.label(status);
            }
        });
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
        .with_inner_size([500.0, 360.0])
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
    fn bundled_dock_icon_is_a_decodable_png() {
        assert!(eframe::icon_data::from_png_bytes(include_bytes!(
            "../../assets/MacDeckInputBridgeIcon.png"
        ))
        .is_ok());
    }

    #[test]
    fn invalid_settings_are_rejected_before_starting_a_process() {
        let settings = ReceiverSettings {
            bind: "not an address".to_owned(),
            pairing_secret: "not base64".to_owned(),
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
