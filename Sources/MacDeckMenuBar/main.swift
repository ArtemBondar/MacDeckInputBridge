import AppKit
import BridgeCore
import MacDeckMenuBarUI
import SwiftUI

@main
struct MacDeckMenuBarApp: App {
    @StateObject private var model = MenuBarModel()
    @Environment(\.colorScheme) private var colorScheme

    init() {
        let showInDock = DockVisibilityPreference.load()
        // The bundle starts as an LSUIElement agent so hidden Dock icons never
        // appear during launch. Promote it only if the saved setting requires it.
        // SwiftUI constructs App before AppKit has installed NSApp. Defer the
        // policy until the first main-loop turn; accessing NSApp here crashes
        // when launching the bundled application from Finder or the Dock.
        DispatchQueue.main.async {
            DockVisibilityPreference.apply(showInDock: showInDock)
        }
    }

    var body: some Scene {
        MenuBarExtra {
            MenuContentView(model: model)
        } label: {
            Image(nsImage: MenuBarDeckIcon.image(
                for: model.status.presentationStatus,
                connectingPhase: model.connectionAnimationPhase,
                isDark: colorScheme == .dark
            ))
                .accessibilityLabel(model.status.title)
        }
    }
}

@MainActor
private final class MenuBarModel: ObservableObject {
    @Published var host: String
    @Published var port: String
    @Published var pairingSecret: String
    @Published var showInDock: Bool
    @Published var allowLocalSecretFallback: Bool
    @Published var syncClipboardFromMac: Bool
    @Published var syncClipboardFromDeck: Bool
    @Published var status: SenderStatus = .local
    @Published var message: String?
    @Published var connectionAnimationPhase = 0

    private let store: BridgeSettingsStore
    private let sender = SenderController()
    private var started = false
    private var connectionAnimationTimer: Timer?
    private var hasOutstandingAccessibilityNotice = false

    init(store: BridgeSettingsStore = BridgeSettingsStore()) {
        self.store = store
        let settings = store.loadSettings()
        host = settings.host
        port = String(settings.port)
        showInDock = DockVisibilityPreference.load()
        allowLocalSecretFallback = store.allowsLocalSecretFallback
        syncClipboardFromMac = settings.syncClipboardFromMac
        syncClipboardFromDeck = settings.syncClipboardFromDeck
        do {
            pairingSecret = try store.loadPairingSecret() ?? ""
        } catch {
            pairingSecret = ""
            message = allowLocalSecretFallback
                ? "Keychain access is unavailable. Enter the pairing secret and save to use this Mac’s private local file."
                : "Could not load the pairing secret. Enter it again in Settings."
        }
        sender.statusChanged = { [weak self] mode in
            self?.setStatus(SenderStatus(mode: mode))
        }
        sender.connectionFailed = { [weak self] in
            self?.message = "Connection was not established. Check the Steam Deck address, pairing secret, and that Mac input is enabled on the Deck."
        }
        sender.layoutSynchronizationWarning = { [weak self] in
            self?.message = "Keyboard-layout change was not confirmed. The Steam Deck kept its previous layout; switch the Mac input source again."
        }
        sender.clipboardSynchronizationUnavailable = { [weak self] in
            self?.message = "Clipboard sync is unavailable. Enable it in MacDeck Settings while the Steam Deck is in Desktop Mode."
        }
        sender.deckClipboardUnavailable = { [weak self] in
            self?.message = "Steam Deck clipboard sharing is unavailable. Enable it on the Deck in Desktop Mode, then reconnect."
        }
        sender.deckClipboardAvailable = { [weak self] in
            let unavailable = "Steam Deck clipboard sharing is unavailable. Enable it on the Deck in Desktop Mode, then reconnect."
            if self?.message == unavailable { self?.message = nil }
        }
        sender.deckClipboardReceived = { [weak self] success in
            self?.message = success ? "Clipboard received from Steam Deck." : "Could not update the Mac clipboard."
        }
        sender.clipboardDeliveryResult = { [weak self] result in
            self?.message = switch result {
            case .written:
                "Clipboard received by Steam Deck."
            case .unavailable:
                "Steam Deck clipboard sync is unavailable. Open MacDeck in Desktop Mode and reconnect."
            case .failed:
                "Steam Deck could not update its clipboard. Open MacDeck in Desktop Mode and reconnect."
            }
        }
    }

    func toggle() {
        do {
            try startIfNeeded()
            sender.toggle()
        } catch {
            if let senderError = error as? SenderControllerError,
               case .accessibilityPermissionRequired = senderError {
                hasOutstandingAccessibilityNotice = true
            }
            message = error.localizedDescription
        }
    }

    var primaryAction: MenuBarPrimaryAction {
        MenuBarPresentation.primaryAction(
            status: status.presentationStatus,
            host: host,
            pairingSecret: pairingSecret
        )
    }

    func performPrimaryAction() {
        switch primaryAction {
        case .configure:
            showSettings()
        case .send, .cancelConnection, .returnToMac:
            toggle()
        }
    }

    func sendCurrentClipboard() {
        message = switch sender.sendCurrentClipboard() {
        case .sent:
            "Sending clipboard to Steam Deck…"
        case .notConnected:
            "Connect the keyboard to Steam Deck before sharing the clipboard."
        case .notEnabled:
            "Enable clipboard sync in both MacDeck Settings screens, then reconnect."
        case .noPlainText:
            "The Mac clipboard has no plain text to send."
        case .tooLarge:
            "The Mac clipboard text is too large to send (maximum 64 KiB)."
        }
    }

    /// Settings must always receive the Mac's physical keyboard. If forwarding is
    /// active, tear down the event tap first rather than letting it suppress this
    /// app's own text fields.
    func showSettings() {
        refreshAccessibilityNotice()
        if started {
            sender.stop()
            started = false
        }
        SettingsWindowController.shared.show(
            draft: SettingsDraft(
                host: host,
                port: port,
                pairingSecret: pairingSecret,
                showInDock: showInDock,
                allowLocalSecretFallback: allowLocalSecretFallback,
                syncClipboardFromMac: syncClipboardFromMac,
                syncClipboardFromDeck: syncClipboardFromDeck,
                currentInputSourceID: SenderController.currentKeyboardLayoutIdentifier()
            ),
            notice: message
        ) { draft in
            self.saveSettings(draft)
        }
    }

    private func saveSettings(_ draft: SettingsDraft) -> String? {
        do {
            host = draft.host
            port = draft.port
            pairingSecret = draft.pairingSecret
            showInDock = draft.showInDock
            allowLocalSecretFallback = draft.allowLocalSecretFallback
            syncClipboardFromMac = draft.syncClipboardFromMac
            syncClipboardFromDeck = draft.syncClipboardFromDeck
            let settings = try settingsFromFields()
            try store.saveSettings(settings)
            try store.setAllowsLocalSecretFallback(allowLocalSecretFallback)
            let persistence = try store.savePairingSecret(pairingSecret)
            DockVisibilityPreference.save(showInDock: showInDock)
            message = switch persistence {
            case .keychain:
                "Saved. The next toggle will use these settings."
            case .localPrivateFile:
                "Saved in this Mac’s private app-data folder because Keychain access is unavailable."
            case .memoryOnly:
                "Keychain access is unavailable. The secret will be used only until this app quits."
            }
            return nil
        } catch {
            message = error.localizedDescription
            return message
        }
    }

    func stop() {
        sender.stop()
        started = false
    }

    private func setStatus(_ newStatus: SenderStatus) {
        status = newStatus
        if newStatus == .connecting {
            startConnectionAnimation()
        } else {
            stopConnectionAnimation()
        }
    }

    private func refreshAccessibilityNotice() {
        guard hasOutstandingAccessibilityNotice, SenderController.isAccessibilityTrusted() else { return }
        hasOutstandingAccessibilityNotice = false
        message = nil
    }

    private func startConnectionAnimation() {
        guard connectionAnimationTimer == nil else { return }
        connectionAnimationPhase = 0
        connectionAnimationTimer = Timer.scheduledTimer(withTimeInterval: 0.32, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.advanceConnectionAnimation()
            }
        }
    }

    private func advanceConnectionAnimation() {
        connectionAnimationPhase = (connectionAnimationPhase + 1) % MenuBarDeckIcon.connectingFrameCount
    }

    private func stopConnectionAnimation() {
        connectionAnimationTimer?.invalidate()
        connectionAnimationTimer = nil
        connectionAnimationPhase = 0
    }

    deinit {
        connectionAnimationTimer?.invalidate()
    }

    private func startIfNeeded() throws {
        guard !started else { return }
        let settings = try settingsFromFields()
        try store.saveSettings(settings)
        let persistence = try store.savePairingSecret(pairingSecret)
        if persistence == .memoryOnly {
            message = "Keychain access is unavailable. The secret will be used only until this app quits."
        }
        try sender.configure(settings: settings, pairingSecret: pairingSecret)
        try sender.start()
        started = true
    }

    private func settingsFromFields() throws -> BridgeSettings {
        guard let portValue = UInt16(port) else { throw BridgeSettingsError.invalidPort }
        return try BridgeSettings(
            host: host,
            port: portValue,
            syncClipboardFromMac: syncClipboardFromMac,
            syncClipboardFromDeck: syncClipboardFromDeck
        ).validated()
    }
}

private enum SenderStatus: Equatable {
    case local, connecting, remote

    init(mode: BridgeMode) {
        switch mode {
        case .local: self = .local
        case .connecting: self = .connecting
        case .remote: self = .remote
        }
    }

    var title: String {
        switch self {
        case .local: return "Mac keyboard: Local"
        case .connecting: return "Mac keyboard: Connecting"
        case .remote: return "Mac keyboard: Steam Deck"
        }
    }

    var presentationStatus: MenuBarStatus {
        switch self {
        case .local: return .local
        case .connecting: return .connecting
        case .remote: return .remote
        }
    }
}

private struct MenuContentView: View {
    @ObservedObject var model: MenuBarModel

    var body: some View {
        Text(model.status.title)
            .font(.headline)
        Button(model.primaryAction.title) {
            model.performPrimaryAction()
        }
        Text("Shortcut: ⌃⇧F12")
            .font(.caption)
            .foregroundStyle(.primary.opacity(0.78))
        if model.status == .remote, model.syncClipboardFromMac {
            Button("Send Mac clipboard now") {
                model.sendCurrentClipboard()
            }
        }
        if let message = model.message, model.status == .remote {
            Text(message)
        }
        Divider()
        Button("Settings…") {
            model.showSettings()
        }
        Button("Quit MacDeckInputBridge") {
            model.stop()
            NSApplication.shared.terminate(nil)
        }
    }
}

