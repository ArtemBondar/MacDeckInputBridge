import AppKit
import BridgeCore
import MacDeckMenuBarUI
import SwiftUI

@main
struct MacDeckMenuBarApp: App {
    @StateObject private var model = MenuBarModel()

    init() {
        let showInDock = DockVisibilityPreference.load()
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
                connectingPhase: model.connectionAnimationPhase
            ))
                .renderingMode(.template)
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
    @Published var status: SenderStatus = .local
    @Published var message: String?
    @Published var connectionAnimationPhase = 0

    private let store: BridgeSettingsStore
    private let sender = SenderController()
    private var started = false
    private var connectionAnimationTimer: Timer?

    init(store: BridgeSettingsStore = BridgeSettingsStore()) {
        self.store = store
        let settings = store.loadSettings()
        host = settings.host
        port = String(settings.port)
        showInDock = DockVisibilityPreference.load()
        do {
            pairingSecret = try store.loadPairingSecret() ?? ""
        } catch {
            pairingSecret = ""
            message = "Could not read the pairing secret from Keychain: \(error.localizedDescription)"
        }
        sender.statusChanged = { [weak self] mode in
            self?.setStatus(SenderStatus(mode: mode))
        }
        sender.connectionFailed = { [weak self] in
            self?.message = "Connection was not established. Check the Steam Deck address, pairing secret, and that Mac input is enabled on the Deck."
        }
    }

    func toggle() {
        do {
            try startIfNeeded()
            sender.toggle()
        } catch {
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

    /// Settings must always receive the Mac's physical keyboard. If forwarding is
    /// active, tear down the event tap first rather than letting it suppress this
    /// app's own text fields.
    func showSettings() {
        if started {
            sender.stop()
            started = false
        }
        SettingsWindowController.shared.show(
            draft: SettingsDraft(host: host, port: port, pairingSecret: pairingSecret, showInDock: showInDock),
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
            let settings = try settingsFromFields()
            try store.saveSettings(settings)
            try store.savePairingSecret(pairingSecret)
            DockVisibilityPreference.save(showInDock: showInDock)
            message = "Saved. The next toggle will use these settings."
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
        try store.savePairingSecret(pairingSecret)
        try sender.configure(settings: settings, pairingSecret: pairingSecret)
        try sender.start()
        started = true
    }

    private func settingsFromFields() throws -> BridgeSettings {
        guard let portValue = UInt16(port) else { throw BridgeSettingsError.invalidPort }
        return try BridgeSettings(host: host, port: portValue).validated()
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

