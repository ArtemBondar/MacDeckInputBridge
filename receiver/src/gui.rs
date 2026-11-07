use eframe::egui::{self, Color32, RichText};
use get_if_addrs::{get_if_addrs, IfAddr};
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
    time::{Duration, Instant},
};

const APP_NAME: &str = "MacDeckInputBridge Receiver";
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

impl ReceiverProcess {
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

struct ReceiverApp {
    settings: ReceiverSettings,
    settings_path: PathBuf,
    process: ReceiverProcess,
    selected_tab: Tab,
    status: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Tab {
    Control,
    Settings,
}

impl ReceiverApp {
    fn new() -> Self {
        let settings_path = settings_path();
        let (settings, status) = match load_settings(&settings_path) {
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
        };
        Self {
            settings,
            settings_path,
            process: ReceiverProcess { child: None },
            selected_tab: Tab::Control,
            status,
        }
    }

    fn save_settings(&mut self) -> Result<(), String> {
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
                self.status = match self.apply_settings() {
                    Ok(()) => Some("Settings saved.".to_owned()),
                    Err(error) => Some(error),
                };
            }
            if ui.button("Copy secret").clicked() {
                context.copy_text(self.settings.pairing_secret.clone());
                self.status = Some("Pairing secret copied to the clipboard.".to_owned());
            }
            if ui.button("Generate new secret").clicked() {
                // Do not leave a receiver running with a secret different from the
                // one shown in the UI. Stopping it destroys its uinput device, so
                // Linux releases the virtual keyboard state before re-pairing.
                self.status = match self.process.stop() {
                    Err(error) => Some(error),
                    Ok(()) => {
                        self.settings.pairing_secret = PairingSecret::generate().to_base64();
                        match self.save_settings() {
                            Ok(()) => Some(
                                "New pairing secret generated. Mac input is disabled until you enable it."
                                    .to_owned(),
                            ),
                            Err(error) => Some(error),
                        }
                    }
                };
            }
        });
        ui.add_space(16.0);
        ui.heading("Steam Deck IP addresses");
        for address in local_ipv4_addresses() {
            ui.monospace(address.to_string());
        }
        ui.add_space(8.0);
        ui.small("Use one of these addresses as --host in the Mac sender settings.");
    }
}

impl eframe::App for ReceiverApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tabs").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_tab, Tab::Control, "Control");
                ui.selectable_value(&mut self.selected_tab, Tab::Settings, "Settings");
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
        context.request_repaint_after(std::time::Duration::from_millis(250));
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 360.0]),
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

    #[test]
    fn generated_default_settings_are_valid() {
        ReceiverSettings::default().validates().unwrap();
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
            status: None,
        };
        app.settings.bind = "127.0.0.1:4343".to_owned();
        app.apply_settings().unwrap();
        assert_eq!(load_settings(&path).unwrap().bind, "127.0.0.1:4343");
        fs::remove_dir_all(directory).unwrap();
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
