import AppKit

public struct SettingsDraft: Equatable {
    public var host: String
    public var port: String
    public var pairingSecret: String
    public var showInDock: Bool
    public var iconStyle: MenuBarIconStyle
    public var useKeychainForPairingSecret: Bool
    public var syncClipboardFromMac: Bool
    public var syncClipboardFromDeck: Bool
    public var currentInputSourceID: String?

    public init(
        host: String,
        port: String,
        pairingSecret: String,
        showInDock: Bool = true,
        iconStyle: MenuBarIconStyle = .logo,
        useKeychainForPairingSecret: Bool = false,
        syncClipboardFromMac: Bool = false,
        syncClipboardFromDeck: Bool = false,
        currentInputSourceID: String? = nil
    ) {
        self.host = host
        self.port = port
        self.pairingSecret = pairingSecret
        self.showInDock = showInDock
        self.iconStyle = iconStyle
        self.useKeychainForPairingSecret = useKeychainForPairingSecret
        self.syncClipboardFromMac = syncClipboardFromMac
        self.syncClipboardFromDeck = syncClipboardFromDeck
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
    public let showInDockCheckbox = NSButton(checkboxWithTitle: "Show MacDeck in Dock", target: nil, action: nil)
    public let iconStylePopup = NSPopUpButton()
    public let iconPreviews = [NSImageView(), NSImageView(), NSImageView()]
    public let useKeychainForPairingSecretCheckbox = NSButton(
        checkboxWithTitle: "Store pairing secret in macOS Keychain",
        target: nil,
        action: nil
    )
    public let syncClipboardCheckbox = NSButton(
        checkboxWithTitle: "Sync plain-text clipboard to Steam Deck",
        target: nil,
        action: nil
    )
    public let syncClipboardFromDeckCheckbox = NSButton(
        checkboxWithTitle: "Accept plain-text clipboard from Steam Deck",
        target: nil,
        action: nil
    )
    public let currentInputSourceLabel = NSTextField(wrappingLabelWithString: "")
    public let messageLabel = NSTextField(wrappingLabelWithString: "")
    public let saveButton = NSButton(title: "Save", target: nil, action: nil)

    private var onSave: ((SettingsDraft) -> String?)?
    private var previewTimer: Timer?
    private(set) var previewPhase = 0
    var isPreviewAnimationRunning: Bool { previewTimer != nil }

    public init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 660),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "MacDeck Settings"
        window.isReleasedWhenClosed = false
        window.hidesOnDeactivate = false
        super.init(window: window)
        window.delegate = self
        buildContent(in: window)
    }

    required init?(coder: NSCoder) { nil }

    deinit { previewTimer?.invalidate() }

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
        iconStylePopup.selectItem(at: MenuBarIconStyle.allCases.firstIndex(of: draft.iconStyle) ?? 0)
        updateIconPreviews(nil)
        useKeychainForPairingSecretCheckbox.state = draft.useKeychainForPairingSecret ? .on : .off
        syncClipboardCheckbox.state = draft.syncClipboardFromMac ? .on : .off
        syncClipboardFromDeckCheckbox.state = draft.syncClipboardFromDeck ? .on : .off
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
        startPreviewAnimation()
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
        stopPreviewAnimation()
        onSave = nil
    }

    public func windowDidMiniaturize(_ notification: Notification) {
        stopPreviewAnimation()
    }

    public func windowDidDeminiaturize(_ notification: Notification) {
        startPreviewAnimation()
    }

    private func startPreviewAnimation() {
        guard previewTimer == nil, let window, window.isVisible, !window.isMiniaturized else { return }
        let timer = Timer(timeInterval: MenuBarDeckIcon.connectingFrameInterval, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.advancePreviewAnimation() }
        }
        RunLoop.main.add(timer, forMode: .common)
        previewTimer = timer
    }

    private func stopPreviewAnimation() {
        previewTimer?.invalidate()
        previewTimer = nil
    }

    func advancePreviewAnimation() {
        guard previewTimer != nil else { return }
        guard let window, window.isVisible, !window.isMiniaturized else {
            stopPreviewAnimation()
            return
        }
        previewPhase = (previewPhase + 1) % MenuBarDeckIcon.connectingFrameCount
        updateConnectingPreview()
    }

    @objc private func save(_ sender: Any?) {
        let draft = SettingsDraft(
            host: hostField.stringValue,
            port: portField.stringValue,
            pairingSecret: pairingSecretField.stringValue,
            showInDock: showInDockCheckbox.state == .on,
            iconStyle: selectedIconStyle,
            useKeychainForPairingSecret: useKeychainForPairingSecretCheckbox.state == .on,
            syncClipboardFromMac: syncClipboardCheckbox.state == .on,
            syncClipboardFromDeck: syncClipboardFromDeckCheckbox.state == .on
        )
        if let error = onSave?(draft) {
            messageLabel.stringValue = error
            return
        }
        close()
        DockVisibilityPreference.apply(showInDock: draft.showInDock)
    }

    @objc private func secretStorageSelectionChanged(_ sender: Any?) {
        guard useKeychainForPairingSecretCheckbox.state == .off,
              isKeychainRelatedNotice(messageLabel.stringValue) else { return }
        messageLabel.stringValue = "After you click Save, the pairing secret will be stored in this Mac’s private local file."
        messageLabel.textColor = .secondaryLabelColor
    }

    private var selectedIconStyle: MenuBarIconStyle {
        let index = iconStylePopup.indexOfSelectedItem
        return MenuBarIconStyle.allCases.indices.contains(index) ? MenuBarIconStyle.allCases[index] : .logo
    }

    @objc private func updateIconPreviews(_ sender: Any?) {
        let isDark = window?.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        for (index, status) in [MenuBarStatus.local, .connecting, .remote].enumerated() {
            iconPreviews[index].image = MenuBarDeckIcon.image(for: status, connectingPhase: previewPhase, isDark: isDark, style: selectedIconStyle)
        }
    }

    private func updateConnectingPreview() {
        let isDark = window?.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        iconPreviews[1].image = MenuBarDeckIcon.image(for: .connecting, connectingPhase: previewPhase, isDark: isDark, style: selectedIconStyle)
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
        let guidance = NSTextField(wrappingLabelWithString: "By default, the secret is stored in a private local file accessible to your account. Keychain offers stronger protection and may request permission. Typed keys are never stored or logged.")
        guidance.textColor = .secondaryLabelColor
        guidance.font = .systemFont(ofSize: 12)
        let clipboardGuidance = NSTextField(wrappingLabelWithString: "Clipboard sharing is off by default. New text may contain sensitive information; it is encrypted and is not logged or saved by MacDeck.")
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
        useKeychainForPairingSecretCheckbox.target = self
        useKeychainForPairingSecretCheckbox.action = #selector(secretStorageSelectionChanged(_:))
        iconStylePopup.addItems(withTitles: MenuBarIconStyle.allCases.map(\.title))
        iconStylePopup.target = self
        iconStylePopup.action = #selector(updateIconPreviews(_:))
        let previewRow = NSStackView()
        previewRow.orientation = .horizontal
        previewRow.alignment = .centerY
        previewRow.spacing = 14
        for (name, image) in zip(["Mac", "Connecting", "Steam Deck"], iconPreviews) {
            image.imageScaling = .scaleProportionallyUpOrDown
            image.widthAnchor.constraint(equalToConstant: 32).isActive = true
            image.heightAnchor.constraint(equalToConstant: 24).isActive = true
            previewRow.addArrangedSubview(image)
            previewRow.addArrangedSubview(label(name, font: .systemFont(ofSize: 12)))
        }

        let stack = NSStackView(views: [
            deckTitle,
            fieldRow("Host or IP address", hostField),
            fieldRow("Port", portField),
            separator(),
            pairingTitle,
            fieldRow("Pairing secret", pairingSecretField),
            guidance,
            useKeychainForPairingSecretCheckbox,
            syncClipboardCheckbox,
            syncClipboardFromDeckCheckbox,
            clipboardGuidance,
            inputSourceTitle,
            currentInputSourceLabel,
            showInDockCheckbox,
            fieldRow("Menu bar icon", iconStylePopup),
            previewRow,
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

    private func fieldRow(_ title: String, _ field: NSView) -> NSStackView {
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
