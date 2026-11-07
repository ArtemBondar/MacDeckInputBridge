import AppKit
import BridgeCore
import MacDeckMenuBarUI
import SwiftUI

@main
struct MacDeckMenuBarApp: App {
    @StateObject private var model = MenuBarModel()

    var body: some Scene {
        MenuBarExtra {
            MenuContentView(model: model)
        } label: {
            Image(systemName: model.status.symbolName)
                .accessibilityLabel(model.status.title)
        }
    }
}

@MainActor
private final class MenuBarModel: ObservableObject {
    @Published var host: String
    @Published var port: String
    @Published var pairingSecret: String
    @Published var status: SenderStatus = .local
    @Published var message: String?

    private let store: BridgeSettingsStore
    private let sender = SenderController()
    private var started = false

    init(store: BridgeSettingsStore = BridgeSettingsStore()) {
        self.store = store
        let settings = store.loadSettings()
        host = settings.host
        port = String(settings.port)
        do {
            pairingSecret = try store.loadPairingSecret() ?? ""
        } catch {
            pairingSecret = ""
            message = "Could not read the pairing secret from Keychain: \(error.localizedDescription)"
        }
        sender.statusChanged = { [weak self] mode in
            self?.status = SenderStatus(mode: mode)
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

    var needsConfiguration: Bool {
        host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || pairingSecret.isEmpty
    }

    var primaryActionTitle: String {
        if needsConfiguration { return "Configure Steam Deck…" }
        switch status {
        case .local: return "Send keyboard to Steam Deck"
        case .connecting: return "Cancel connection"
        case .remote: return "Return keyboard to Mac"
        }
    }

    func primaryAction() {
        if needsConfiguration {
            showSettings()
        } else {
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
            draft: SettingsDraft(host: host, port: port, pairingSecret: pairingSecret)
        ) { draft in
            self.saveSettings(draft)
        }
    }

    private func saveSettings(_ draft: SettingsDraft) -> String? {
        do {
            host = draft.host
            port = draft.port
            pairingSecret = draft.pairingSecret
            let settings = try settingsFromFields()
            try store.saveSettings(settings)
            try store.savePairingSecret(pairingSecret)
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

    var symbolName: String {
        switch self {
        case .local: return "keyboard"
        case .connecting: return "keyboard.badge.ellipsis"
        case .remote: return "keyboard.fill"
        }
    }
}

private struct MenuContentView: View {
    @ObservedObject var model: MenuBarModel

    var body: some View {
        Text(model.status.title)
        Button(model.primaryActionTitle) {
            model.primaryAction()
        }
        Text("Shortcut: Control + Shift + F12")
            .font(.caption)
            .foregroundStyle(.secondary)
        if let message = model.message {
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
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

