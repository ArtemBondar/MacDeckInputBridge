import AppKit

public struct SettingsDraft: Equatable {
    public var host: String
    public var port: String
    public var pairingSecret: String
    public var showInDock: Bool
    public var allowLocalSecretFallback: Bool
    public var syncClipboardFromMac: Bool
    public var currentInputSourceID: String?

    public init(
        host: String,
        port: String,
        pairingSecret: String,
        showInDock: Bool = true,
        allowLocalSecretFallback: Bool = false,
        syncClipboardFromMac: Bool = false,
        currentInputSourceID: String? = nil
    ) {
        self.host = host
        self.port = port
        self.pairingSecret = pairingSecret
        self.showInDock = showInDock
        self.allowLocalSecretFallback = allowLocalSecretFallback
        self.syncClipboardFromMac = syncClipboardFromMac
        self.currentInputSourceID = currentInputSourceID
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
    public let allowLocalSecretFallbackCheckbox = NSButton(
        checkboxWithTitle: "If Keychain is unavailable, store the secret in a private local file",
        target: nil,
        action: nil
    )
    public let syncClipboardCheckbox = NSButton(
        checkboxWithTitle: "Sync plain-text clipboard to Steam Deck",
        target: nil,
        action: nil
    )
    public let currentInputSourceLabel = NSTextField(wrappingLabelWithString: "")
    public let messageLabel = NSTextField(wrappingLabelWithString: "")
    public let saveButton = NSButton(title: "Save", target: nil, action: nil)

    private var onSave: ((SettingsDraft) -> String?)?

    public init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 520),
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
        allowLocalSecretFallbackCheckbox.state = draft.allowLocalSecretFallback ? .on : .off
        syncClipboardCheckbox.state = draft.syncClipboardFromMac ? .on : .off
        currentInputSourceLabel.stringValue = draft.currentInputSourceID ?? "Unavailable"
        messageLabel.stringValue = notice ?? ""
        messageLabel.textColor = isInformationalNotice(notice) ? .secondaryLabelColor : .systemRed
        guard let window else { return }

        // Accessory applications may still activate a window programmatically.
        // Keeping this in sync with the saved Dock choice avoids a Dock icon
        // briefly appearing only while Settings is open.
        DockVisibilityPreference.apply(showInDock: draft.showInDock)
        NSRunningApplication.current.activate(options: [.activateIgnoringOtherApps])
        NSApp.activate(ignoringOtherApps: true)
        if !window.isVisible {
            // A window constructed at (0, 0) otherwise appears in the lower
            // left corner on first use. Preserve a user-moved open window.
            window.center()
        }
        showWindow(nil)
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
            showInDock: showInDockCheckbox.state == .on,
            allowLocalSecretFallback: allowLocalSecretFallbackCheckbox.state == .on,
            syncClipboardFromMac: syncClipboardCheckbox.state == .on
        )
        if let error = onSave?(draft) {
            messageLabel.stringValue = error
            return
        }
        close()
        DockVisibilityPreference.apply(showInDock: draft.showInDock)
    }

    @objc private func localSecretFallbackSelectionChanged(_ sender: Any?) {
        guard allowLocalSecretFallbackCheckbox.state == .on,
              isKeychainRelatedNotice(messageLabel.stringValue) else { return }
        messageLabel.stringValue = "After you click Save, the pairing secret will be stored in this Mac’s private local file."
        messageLabel.textColor = .secondaryLabelColor
    }

    private func isKeychainRelatedNotice(_ notice: String) -> Bool {
        notice.contains("Keychain") || notice.hasPrefix("Could not load the pairing secret")
    }

    private func isInformationalNotice(_ notice: String?) -> Bool {
        guard let notice else { return false }
        return notice.hasPrefix("Keychain access is unavailable") ||
            notice.hasPrefix("After you click Save") ||
            notice.hasPrefix("Saved in this Mac’s private app-data folder")
    }

    private func buildContent(in window: NSWindow) {
        let root = NSView()
        root.translatesAutoresizingMaskIntoConstraints = false
        window.contentView = root

        let deckTitle = label("Steam Deck", font: .systemFont(ofSize: 20, weight: .semibold))
        let pairingTitle = label("Pairing", font: .systemFont(ofSize: 20, weight: .semibold))
        let guidance = NSTextField(wrappingLabelWithString: "Stored in this Mac’s Keychain. If Keychain access is denied, the optional local fallback is owner-only but less secure. Typed keys are never stored or logged.")
        guidance.textColor = .secondaryLabelColor
        guidance.font = .systemFont(ofSize: 12)
        let clipboardGuidance = NSTextField(wrappingLabelWithString: "Off by default. Sends only new plain-text clipboard changes while connected; contents are encrypted, never logged or saved, and are not sent back from Steam Deck.")
        clipboardGuidance.textColor = .secondaryLabelColor
        clipboardGuidance.font = .systemFont(ofSize: 12)
        clipboardGuidance.maximumNumberOfLines = 2
        let inputSourceTitle = label("Current Mac layout ID", font: .systemFont(ofSize: 13, weight: .medium))
        currentInputSourceLabel.textColor = .secondaryLabelColor
        currentInputSourceLabel.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        currentInputSourceLabel.maximumNumberOfLines = 2
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
        allowLocalSecretFallbackCheckbox.target = self
        allowLocalSecretFallbackCheckbox.action = #selector(localSecretFallbackSelectionChanged(_:))

        let stack = NSStackView(views: [
            deckTitle,
            fieldRow("Host or IP address", hostField),
            fieldRow("Port", portField),
            separator(),
            pairingTitle,
            fieldRow("Pairing secret", pairingSecretField),
            guidance,
            allowLocalSecretFallbackCheckbox,
            syncClipboardCheckbox,
            clipboardGuidance,
            inputSourceTitle,
            currentInputSourceLabel,
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
