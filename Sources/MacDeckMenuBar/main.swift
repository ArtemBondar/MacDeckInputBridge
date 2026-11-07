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
            SettingsWindowPresenter.show()
        } else {
            toggle()
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
            SettingsWindowPresenter.show()
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
        VStack(alignment: .leading, spacing: 16) {
            Text("Steam Deck")
                .font(.title2.weight(.semibold))
            field("Host or IP address") {
                TextField("192.168.1.42", text: $model.host)
            }
            field("Port") {
                TextField("4242", text: $model.port)
            }
            Divider()
            Text("Pairing")
                .font(.title2.weight(.semibold))
            field("Pairing secret") {
                SecureField("Base64 32-byte secret", text: $model.pairingSecret)
            }
            Text("Stored only in this Mac’s Keychain; typed keys are never stored or logged.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let message = model.message {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Save") { model.saveSettings() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 520)
    }

    @ViewBuilder
    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.headline)
            content()
                .textFieldStyle(.roundedBorder)
        }
    }
}

@MainActor
private enum SettingsWindowPresenter {
    static func show() {
        NSApp.activate(ignoringOtherApps: true)
        if let window = settingsWindow() {
            bringToFront(window)
            return
        }

        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
        DispatchQueue.main.async {
            NSApp.activate(ignoringOtherApps: true)
            if let window = settingsWindow() {
                bringToFront(window)
            }
        }
    }

    private static func settingsWindow() -> NSWindow? {
        NSApp.windows.first { window in
            window.title.localizedCaseInsensitiveContains("settings")
        }
    }

    private static func bringToFront(_ window: NSWindow) {
        window.level = .normal
        window.hidesOnDeactivate = false
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
    }
}
