import AppKit

public struct SettingsDraft: Equatable {
    public var host: String
    public var port: String
    public var pairingSecret: String
    public var showInDock: Bool

    public init(host: String, port: String, pairingSecret: String, showInDock: Bool = true) {
        self.host = host
        self.port = port
        self.pairingSecret = pairingSecret
        self.showInDock = showInDock
    }
}

/// An explicit AppKit window keeps Settings independent from MenuBarExtra's
/// transient menu lifecycle and makes the editable controls directly testable.
@MainActor
public final class SettingsWindowController: NSWindowController, NSWindowDelegate {
    public static let shared = SettingsWindowController()

    public let hostField = NSTextField(string: "")
    public let portField = NSTextField(string: "")
    public let pairingSecretField = NSSecureTextField(string: "")
    public let showInDockCheckbox = NSButton(checkboxWithTitle: "Show MacDeckInputBridge in Dock", target: nil, action: nil)
    public let messageLabel = NSTextField(wrappingLabelWithString: "")
    public let saveButton = NSButton(title: "Save", target: nil, action: nil)

    private var onSave: ((SettingsDraft) -> String?)?

    public init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 390),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "MacDeckInputBridge Settings"
        window.isReleasedWhenClosed = false
        window.hidesOnDeactivate = false
        super.init(window: window)
        window.delegate = self
        buildContent(in: window)
    }

    required init?(coder: NSCoder) { nil }

    public func show(
        draft: SettingsDraft,
        notice: String? = nil,
        onSave: @escaping (SettingsDraft) -> String?
    ) {
        self.onSave = onSave
        hostField.stringValue = draft.host
        portField.stringValue = draft.port
        pairingSecretField.stringValue = draft.pairingSecret
        showInDockCheckbox.state = draft.showInDock ? .on : .off
        messageLabel.stringValue = notice ?? ""

        // Accessory applications may still activate a window programmatically.
        // Keeping this in sync with the saved Dock choice avoids a Dock icon
        // briefly appearing only while Settings is open.
        DockVisibilityPreference.apply(showInDock: draft.showInDock)
        NSRunningApplication.current.activate(options: [.activateIgnoringOtherApps])
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        guard let window else { return }
        window.level = .normal
        window.makeKeyAndOrderFront(nil)
        window.makeMain()
        window.orderFrontRegardless()
        DispatchQueue.main.async { [weak self, weak window] in
            guard let self, let window else { return }
            DockVisibilityPreference.apply(showInDock: draft.showInDock)
            NSRunningApplication.current.activate(options: [.activateIgnoringOtherApps])
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(self.hostField)
        }
    }

    public func windowWillClose(_ notification: Notification) {
        onSave = nil
    }

    @objc private func save(_ sender: Any?) {
        let draft = SettingsDraft(
            host: hostField.stringValue,
            port: portField.stringValue,
            pairingSecret: pairingSecretField.stringValue,
            showInDock: showInDockCheckbox.state == .on
        )
        if let error = onSave?(draft) {
            messageLabel.stringValue = error
            return
        }
        close()
        DockVisibilityPreference.apply(showInDock: draft.showInDock)
    }

    private func buildContent(in window: NSWindow) {
        let root = NSView()
        root.translatesAutoresizingMaskIntoConstraints = false
        window.contentView = root

        let deckTitle = label("Steam Deck", font: .systemFont(ofSize: 20, weight: .semibold))
        let pairingTitle = label("Pairing", font: .systemFont(ofSize: 20, weight: .semibold))
        let guidance = NSTextField(wrappingLabelWithString: "Stored only in this Mac’s Keychain; typed keys are never stored or logged.")
        guidance.textColor = .secondaryLabelColor
        guidance.font = .systemFont(ofSize: 12)
        messageLabel.textColor = .systemRed
        messageLabel.font = .systemFont(ofSize: 12)
        messageLabel.maximumNumberOfLines = 3
        messageLabel.lineBreakMode = .byWordWrapping
        messageLabel.cell?.wraps = true

        [hostField, portField, pairingSecretField].forEach { field in
            field.translatesAutoresizingMaskIntoConstraints = false
            field.isEditable = true
            field.isSelectable = true
            field.isBezeled = true
            field.bezelStyle = .roundedBezel
        }
        hostField.placeholderString = "192.168.1.42"
        portField.placeholderString = "4242"
        pairingSecretField.placeholderString = "Base64 32-byte secret"

        saveButton.target = self
        saveButton.action = #selector(save(_:))
        saveButton.keyEquivalent = "\r"

        let stack = NSStackView(views: [
            deckTitle,
            fieldRow("Host or IP address", hostField),
            fieldRow("Port", portField),
            separator(),
            pairingTitle,
            fieldRow("Pairing secret", pairingSecretField),
            guidance,
            showInDockCheckbox,
            messageLabel,
            buttonRow(saveButton)
        ])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 28),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -28),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 24),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: root.bottomAnchor, constant: -24),
            hostField.widthAnchor.constraint(greaterThanOrEqualToConstant: 340),
            pairingSecretField.widthAnchor.constraint(greaterThanOrEqualToConstant: 340)
        ])
    }

    private func fieldRow(_ title: String, _ field: NSTextField) -> NSStackView {
        let titleLabel = label(title, font: .systemFont(ofSize: 13, weight: .medium))
        let row = NSStackView(views: [titleLabel, field])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 12
        titleLabel.widthAnchor.constraint(equalToConstant: 128).isActive = true
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return row
    }

    private func buttonRow(_ button: NSButton) -> NSStackView {
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let row = NSStackView(views: [spacer, button])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.distribution = .fill
        return row
    }

    private func label(_ text: String, font: NSFont) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = font
        return label
    }

    private func separator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        box.widthAnchor.constraint(equalToConstant: 500).isActive = true
        return box
    }
}
