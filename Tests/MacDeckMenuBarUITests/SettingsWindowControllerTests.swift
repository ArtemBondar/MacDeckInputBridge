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
        XCTAssertEqual(NSApp.activationPolicy(), .accessory)
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

    private func drainMainQueue() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
}
