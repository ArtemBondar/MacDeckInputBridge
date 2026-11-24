import AppKit
import ApplicationServices

/// Reuse System Settings instead of stacking an asynchronous permission prompt
/// over the page where the user is already trying to grant access.
enum AccessibilityPermission {
    private static let settingsBundleIdentifier = "com.apple.systempreferences"
    private static let accessibilitySettingsURL = URL(
        string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
    )!

    static func checkAndRequest(
        isTrusted: () -> Bool = { AXIsProcessTrusted() },
        settingsIsRunning: () -> Bool = {
            !NSRunningApplication.runningApplications(withBundleIdentifier: settingsBundleIdentifier).isEmpty
        },
        showPrompt: () -> Bool = {
            let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
            return AXIsProcessTrustedWithOptions(options)
        },
        openSettings: () -> Void = {
            // Opening the pane URL reuses Settings and also restores a closed or
            // minimized window; activation alone would not select the pane.
            NSWorkspace.shared.open(accessibilitySettingsURL)
            NSRunningApplication.runningApplications(withBundleIdentifier: settingsBundleIdentifier)
                .first?.activate(options: [.activateIgnoringOtherApps])
        }
    ) -> Bool {
        if isTrusted() { return true }
        if settingsIsRunning() {
            openSettings()
            return false
        }
        return showPrompt()
    }
}
