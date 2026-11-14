//! Passive Desktop Mode errors, with a bounded fallback if D-Bus is unavailable.
use eframe::egui::Context;
use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

// The notification call is optional: do not let an unresponsive desktop
// service leave the only actionable error hidden indefinitely.
const NOTIFICATION_CALL_TIMEOUT: Duration = Duration::from_secs(3);
const NOTIFICATION_FALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

struct Pending {
    result: Receiver<bool>,
    message: &'static str,
    deadline: Instant,
    timed_out: bool,
}

#[derive(Default)]
pub struct NotificationBridge {
    pending: Option<Pending>,
}

impl NotificationBridge {
    #[cfg(test)]
    pub fn resolved_for_test(message: &'static str, success: bool) -> Self {
        let (sender, result) = mpsc::channel();
        sender.send(success).unwrap();
        Self {
            pending: Some(Pending {
                result,
                message,
                deadline: Instant::now() + NOTIFICATION_FALLBACK_TIMEOUT,
                timed_out: false,
            }),
        }
    }

    #[cfg(test)]
    pub fn pending_message(&self) -> Option<&'static str> {
        self.pending.as_ref().map(|pending| pending.message)
    }

    pub fn send(&mut self, context: &Context, message: &'static str) -> bool {
        self.send_with(context, message, deliver)
    }

    fn send_with(
        &mut self,
        context: &Context,
        message: &'static str,
        deliver: impl FnOnce(&'static str) -> bool + Send + 'static,
    ) -> bool {
        // Never create unbounded threads if the desktop service hangs.
        if self.pending.is_some() {
            return false;
        }
        context.request_repaint_after(NOTIFICATION_FALLBACK_TIMEOUT);
        let (sender, result) = mpsc::channel();
        let context = context.clone();
        std::thread::spawn(move || {
            let success = deliver(message);
            let _ = sender.send(success);
            context.request_repaint();
        });
        self.pending = Some(Pending {
            result,
            message,
            deadline: Instant::now() + NOTIFICATION_FALLBACK_TIMEOUT,
            timed_out: false,
        });
        true
    }

    /// Returns the safe diagnostic when delivery failed or timed out.
    pub fn poll(&mut self, now: Instant) -> Option<&'static str> {
        let pending = self.pending.as_mut()?;
        match pending.result.try_recv() {
            Ok(success) => {
                let failed = !success && !pending.timed_out;
                let message = pending.message;
                self.pending = None;
                failed.then_some(message)
            }
            Err(TryRecvError::Disconnected) => {
                let timed_out = pending.timed_out;
                let message = pending.message;
                self.pending = None;
                (!timed_out).then_some(message)
            }
            Err(TryRecvError::Empty) if !pending.timed_out && now >= pending.deadline => {
                pending.timed_out = true;
                Some(pending.message)
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    pub fn request_deadline_repaint(&self, context: &Context, now: Instant) {
        if let Some(pending) = &self.pending {
            if !pending.timed_out {
                context.request_repaint_after(pending.deadline.saturating_duration_since(now));
            }
        }
    }
}

fn deliver(message: &'static str) -> bool {
    fn notify(message: &str) -> zbus::Result<()> {
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(NOTIFICATION_CALL_TIMEOUT)
            .build()?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )?;
        let actions: Vec<&str> = Vec::new();
        let hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let _: u32 = proxy.call(
            "Notify",
            &(
                "MacDeck",
                0u32,
                "mac-deck-input-bridge",
                "MacDeck needs attention",
                message,
                actions,
                hints,
                -1i32,
            ),
        )?;
        Ok(())
    }
    notify(message).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeNotificationService(std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl FakeNotificationService {
        // The argument list is defined by the desktop notification protocol.
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app_name: &str,
            _replaces_id: u32,
            _app_icon: &str,
            summary: &str,
            body: &str,
            _actions: Vec<String>,
            _hints: HashMap<String, zbus::zvariant::OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            assert_eq!(app_name, "MacDeck");
            self.0
                .lock()
                .unwrap()
                .push((summary.to_owned(), body.to_owned()));
            1
        }
    }

    #[test]
    fn notification_dbus_protocol_and_missing_service() {
        const CHILD_MARKER: &str = "MAC_DECK_TEST_NOTIFICATION_BUS";
        if std::env::var_os(CHILD_MARKER).is_none() {
            if std::process::Command::new("dbus-run-session")
                .arg("--version")
                .output()
                .is_err()
            {
                eprintln!(
                    "dbus-run-session unavailable; notification protocol integration test skipped"
                );
                return;
            }
            let result = std::process::Command::new("dbus-run-session")
                .arg("--")
                .arg(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "notification::tests::notification_dbus_protocol_and_missing_service",
                    "--nocapture",
                ])
                .env(CHILD_MARKER, "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "isolated D-Bus test failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        assert!(!deliver("No service available"));
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let connection = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                FakeNotificationService(observed.clone()),
            )
            .unwrap()
            .build()
            .unwrap();
        assert!(deliver("Clipboard is unavailable. Reconnect."));
        assert_eq!(
            *observed.lock().unwrap(),
            [(
                "MacDeck needs attention".to_owned(),
                "Clipboard is unavailable. Reconnect.".to_owned()
            )]
        );
        connection.close().unwrap();
    }

    #[test]
    fn failed_or_timed_out_delivery_is_reported_once_and_keeps_worker_bounded() {
        let context = Context::default();
        let (sender, receiver) = mpsc::channel();
        let now = Instant::now();
        let mut bridge = NotificationBridge {
            pending: Some(Pending {
                result: receiver,
                message: "safe error",
                deadline: now + NOTIFICATION_FALLBACK_TIMEOUT,
                timed_out: false,
            }),
        };
        assert_eq!(bridge.poll(now), None);
        assert!(!bridge.send_with(&context, "another error", |_| true));
        assert_eq!(
            bridge.poll(now + NOTIFICATION_FALLBACK_TIMEOUT),
            Some("safe error")
        );
        assert_eq!(bridge.poll(now + NOTIFICATION_FALLBACK_TIMEOUT), None);
        sender.send(false).unwrap();
        assert_eq!(bridge.poll(now + NOTIFICATION_FALLBACK_TIMEOUT), None);
        assert!(bridge.pending.is_none());
    }

    #[test]
    fn successful_delivery_stays_silent_and_failure_is_actionable() {
        let now = Instant::now();
        for (success, expected) in [(true, None), (false, Some("safe error"))] {
            let (sender, receiver) = mpsc::channel();
            sender.send(success).unwrap();
            let mut bridge = NotificationBridge {
                pending: Some(Pending {
                    result: receiver,
                    message: "safe error",
                    deadline: now + NOTIFICATION_FALLBACK_TIMEOUT,
                    timed_out: false,
                }),
            };
            assert_eq!(bridge.poll(now), expected);
            assert!(bridge.pending.is_none());
        }
    }
}
