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

    private func drainMainQueue() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
}
