import AppKit
import BridgeCore
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
        Settings {
            SettingsView(model: model)
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
        pairingSecret = (try? store.loadPairingSecret()) ?? ""
        sender.statusChanged = { [weak self] mode in
            self?.status = SenderStatus(mode: mode)
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

    func saveSettings() {
        do {
            let settings = try settingsFromFields()
            try store.saveSettings(settings)
            try store.savePairingSecret(pairingSecret)
            if started {
                sender.stop()
                started = false
            }
            message = "Saved. The next toggle will use these settings."
        } catch {
            message = error.localizedDescription
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
        Button(model.status == .remote ? "Return keyboard to Mac" : "Send keyboard to Steam Deck") {
            model.toggle()
        }
        Text("Shortcut: Control + Shift + F12")
            .font(.caption)
            .foregroundStyle(.secondary)
        Divider()
        Button("Settings…") {
            NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
        }
        Button("Quit MacDeckInputBridge") {
            model.stop()
            NSApplication.shared.terminate(nil)
        }
    }
}

private struct SettingsView: View {
    @ObservedObject var model: MenuBarModel

    var body: some View {
        Form {
            Section("Steam Deck") {
                TextField("Host or IP address", text: $model.host)
                TextField("Port", text: $model.port)
                    .textFieldStyle(.roundedBorder)
            }
            Section("Pairing") {
                SecureField("Pairing secret", text: $model.pairingSecret)
                Text("Stored only in this Mac’s Keychain; typed keys are never stored or logged.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let message = model.message {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Button("Save") { model.saveSettings() }
        }
        .padding()
        .frame(width: 440)
    }
}
