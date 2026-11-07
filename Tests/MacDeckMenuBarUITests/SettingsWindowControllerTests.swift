import AppKit
import MacDeckMenuBarUI
import XCTest

@MainActor
final class SettingsWindowControllerTests: XCTestCase {
    func testSettingsWindowBecomesKeyAndHostFieldAcceptsTyping() throws {
        let controller = SettingsWindowController()
        controller.show(draft: SettingsDraft(host: "", port: "4242", pairingSecret: "")) { _ in nil }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()

        XCTAssertTrue(window.isVisible)
        XCTAssertEqual(NSApp.activationPolicy(), .regular)
        XCTAssertTrue(controller.hostField.isEditable)
        XCTAssertTrue(controller.hostField.isSelectable)
        XCTAssertTrue(
            window.firstResponder === controller.hostField || window.firstResponder is NSTextView,
            "Host field must be the initial key responder"
        )

        window.makeFirstResponder(controller.hostField)
        let editor = try XCTUnwrap(window.fieldEditor(true, for: controller.hostField) as? NSTextView)
        editor.insertText("192.168.1.42", replacementRange: NSRange(location: 0, length: 0))
        XCTAssertEqual(controller.hostField.stringValue, "192.168.1.42")
        window.close()
        XCTAssertEqual(NSApp.activationPolicy(), .regular)
    }

    func testPairingFieldAcceptsPasteAndSaveUsesEnteredValues() throws {
        let controller = SettingsWindowController()
        var saved: SettingsDraft?
        controller.show(draft: SettingsDraft(host: "", port: "4242", pairingSecret: "")) {
            saved = $0
            return nil
        }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()

        controller.hostField.stringValue = "192.168.1.42"
        let pasteboard = NSPasteboard.general
        let originalText = pasteboard.string(forType: .string)
        defer {
            pasteboard.clearContents()
            if let originalText { pasteboard.setString(originalText, forType: .string) }
            window.close()
        }
        pasteboard.clearContents()
        pasteboard.setString("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", forType: .string)

        window.makeFirstResponder(controller.pairingSecretField)
        let editor = try XCTUnwrap(window.fieldEditor(true, for: controller.pairingSecretField) as? NSTextView)
        editor.paste(nil)
        XCTAssertEqual(controller.pairingSecretField.stringValue, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")

        controller.saveButton.performClick(nil)
        XCTAssertEqual(saved, SettingsDraft(
            host: "192.168.1.42",
            port: "4242",
            pairingSecret: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ))
    }

    func testSaveValidationErrorKeepsWindowOpenAndVisible() throws {
        let controller = SettingsWindowController()
        controller.show(draft: SettingsDraft(host: "", port: "4242", pairingSecret: "")) { _ in
            "Enter a Steam Deck host."
        }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()

        controller.saveButton.performClick(nil)
        XCTAssertTrue(window.isVisible)
        XCTAssertEqual(controller.messageLabel.stringValue, "Enter a Steam Deck host.")
        window.close()
    }

    func testReopeningSettingsReplacesAllFieldsAndClearsPreviousError() throws {
        let controller = SettingsWindowController()
        controller.show(draft: SettingsDraft(host: "", port: "0", pairingSecret: "old")) { _ in "Invalid port." }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()
        controller.saveButton.performClick(nil)
        XCTAssertEqual(controller.messageLabel.stringValue, "Invalid port.")

        controller.show(draft: SettingsDraft(host: "deck.local", port: "4242", pairingSecret: "new")) { _ in nil }
        drainMainQueue()
        XCTAssertEqual(controller.hostField.stringValue, "deck.local")
        XCTAssertEqual(controller.portField.stringValue, "4242")
        XCTAssertEqual(controller.pairingSecretField.stringValue, "new")
        XCTAssertEqual(controller.messageLabel.stringValue, "")
        window.close()
    }

    func testLongNoticeWrapsInsideFixedWidthSettingsWindow() throws {
        let controller = SettingsWindowController()
        let notice = "Accessibility permission is required. Enable the app in System Settings, then relaunch MacDeckInputBridge before sending the keyboard to Steam Deck."
        controller.show(
            draft: SettingsDraft(host: "deck.local", port: "4242", pairingSecret: "secret"),
            notice: notice
        ) { _ in nil }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()

        XCTAssertEqual(window.frame.width, 560, accuracy: 1)
        XCTAssertEqual(controller.messageLabel.stringValue, notice)
        XCTAssertEqual(controller.messageLabel.lineBreakMode, .byWordWrapping)
        XCTAssertTrue(controller.messageLabel.cell?.wraps ?? false)
        window.close()
    }

    func testEveryMenuBarPrimaryActionStateHasAnExplicitLabel() {
        XCTAssertEqual(MenuBarPresentation.attentionTitle, "Needs attention — open Settings…")
        XCTAssertEqual(
            MenuBarPresentation.primaryAction(status: .local, host: "", pairingSecret: "secret"),
            .configure
        )
        XCTAssertEqual(
            MenuBarPresentation.primaryAction(status: .remote, host: "deck", pairingSecret: ""),
            .configure
        )
        XCTAssertEqual(
            MenuBarPresentation.primaryAction(status: .local, host: "deck", pairingSecret: "secret"),
            .send
        )
        XCTAssertEqual(
            MenuBarPresentation.primaryAction(status: .connecting, host: "deck", pairingSecret: "secret"),
            .cancelConnection
        )
        XCTAssertEqual(
            MenuBarPresentation.primaryAction(status: .remote, host: "deck", pairingSecret: "secret"),
            .returnToMac
        )
    }

    func testDockCanStayHiddenWhileSettingsIsOpenAndAfterClose() throws {
        let controller = SettingsWindowController()
        controller.show(
            draft: SettingsDraft(host: "deck.local", port: "4242", pairingSecret: "secret", showInDock: false)
        ) { _ in nil }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()

        XCTAssertEqual(controller.showInDockCheckbox.state, .off)
        XCTAssertEqual(NSApp.activationPolicy(), .accessory)
        XCTAssertTrue(window.isVisible)
        XCTAssertTrue(
            window.firstResponder === controller.hostField || window.firstResponder is NSTextView,
            "An accessory app must still direct input into Settings."
        )
        window.close()
        XCTAssertEqual(NSApp.activationPolicy(), .accessory)
    }

    func testSavingDockChoiceDeliversTheUserSelection() throws {
        let controller = SettingsWindowController()
        var saved: SettingsDraft?
        controller.show(
            draft: SettingsDraft(host: "deck.local", port: "4242", pairingSecret: "secret", showInDock: true)
        ) {
            saved = $0
            return nil
        }
        let window = try XCTUnwrap(controller.window)
        drainMainQueue()

        controller.showInDockCheckbox.state = .off
        controller.saveButton.performClick(nil)
        XCTAssertEqual(saved?.showInDock, false)
        XCTAssertFalse(window.isVisible)
        XCTAssertEqual(NSApp.activationPolicy(), .accessory)
    }

    func testDockVisibilityPreferenceDefaultsToShownAndPersistsChoice() {
        let suite = "MacDeckMenuBarUITests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { UserDefaults.standard.removePersistentDomain(forName: suite) }

        XCTAssertTrue(DockVisibilityPreference.load(from: defaults))
        DockVisibilityPreference.save(showInDock: false, to: defaults)
        XCTAssertFalse(DockVisibilityPreference.load(from: defaults))
    }

    func testMenuBarDeckIconIsTemplateAccessibleAndExposesEveryConnectionState() {
        let expectedDescriptions: [(MenuBarStatus, String)] = [
            (.local, "Steam Deck keyboard input, local"),
            (.connecting, "Steam Deck keyboard input, connecting"),
            (.remote, "Steam Deck keyboard input, sending to Steam Deck")
        ]
        for (status, description) in expectedDescriptions {
            let image = MenuBarDeckIcon.image(for: status)
            XCTAssertTrue(image.isTemplate)
            XCTAssertEqual(image.accessibilityDescription, description)
            XCTAssertGreaterThan(image.size.width, 0)
            XCTAssertGreaterThan(image.size.height, 0)
        }

        XCTAssertNotEqual(
            MenuBarDeckIcon.image(for: .local).tiffRepresentation,
            MenuBarDeckIcon.image(for: .connecting).tiffRepresentation,
            "CONNECTING must render a visible in-progress state."
        )
        XCTAssertNotEqual(
            MenuBarDeckIcon.image(for: .local).tiffRepresentation,
            MenuBarDeckIcon.image(for: .remote).tiffRepresentation,
            "REMOTE must render a visibly captured keyboard state."
        )
        XCTAssertEqual(MenuBarDeckIcon.connectingFrameCount, 3)
        XCTAssertNotEqual(
            MenuBarDeckIcon.image(for: .connecting, connectingPhase: 0).tiffRepresentation,
            MenuBarDeckIcon.image(for: .connecting, connectingPhase: 1).tiffRepresentation,
            "Connecting animation frames must visibly advance."
        )
        XCTAssertNotEqual(
            MenuBarDeckIcon.image(for: .connecting, connectingPhase: 1).tiffRepresentation,
            MenuBarDeckIcon.image(for: .connecting, connectingPhase: 2).tiffRepresentation,
            "Connecting animation must progress through all three dots."
        )
    }

    private func drainMainQueue() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
}
