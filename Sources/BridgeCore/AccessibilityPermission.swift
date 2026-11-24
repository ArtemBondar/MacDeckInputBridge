import AppKit
import ApplicationServices

/// Reuse System Settings instead of stacking an asynchronous permission prompt
/// over the page where the user is already trying to grant access.
struct AccessibilityPermission {
    // Process-local only: a saved preference would become stale after a TCC reset
    // or reinstall and could prevent the app from registering again.
    private var hasRequestedAccess = false
    private static let settingsBundleIdentifier = "com.apple.systempreferences"
    private static let accessibilitySettingsURL = URL(
        string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
    )!

    mutating func checkAndRequest(
        isTrusted: () -> Bool = { AXIsProcessTrusted() },
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
        if isTrusted() {
            // If access is later revoked in this process, request registration
            // again rather than assuming the old TCC entry still exists.
            hasRequestedAccess = false
            return true
        }
        if !hasRequestedAccess {
            // Settings running does not mean this app is in its permission list.
            // Only the explicit AX request can register the initial request.
            hasRequestedAccess = true
            return showPrompt()
        }
        openSettings()
        return false
    }
}
