import XCTest
@testable import BridgeCore

final class AccessibilityPermissionTests: XCTestCase {
    func testTrustedProcessDoesNotPromptOrOpenSettings() {
        XCTAssertTrue(AccessibilityPermission.checkAndRequest(
            isTrusted: { true },
            settingsIsRunning: { XCTFail("Should not inspect Settings when already trusted"); return true },
            showPrompt: { XCTFail("Should not prompt"); return false },
            openSettings: { XCTFail("Should not open Settings") }
        ))
    }

    func testOpenSettingsAreReusedOnEveryUntrustedAttempt() {
        var settingsOpenCount = 0
        for _ in 0..<3 {
            XCTAssertFalse(AccessibilityPermission.checkAndRequest(
                isTrusted: { false },
                settingsIsRunning: { true },
                showPrompt: { XCTFail("Should not stack a prompt over Settings"); return false },
                openSettings: { settingsOpenCount += 1 }
            ))
        }
        XCTAssertEqual(settingsOpenCount, 3)
    }

    func testInitialRequestStillUsesSystemPromptWhenSettingsAreNotRunning() {
        var promptCount = 0
        XCTAssertFalse(AccessibilityPermission.checkAndRequest(
            isTrusted: { false },
            settingsIsRunning: { false },
            showPrompt: { promptCount += 1; return false },
            openSettings: { XCTFail("The initial system prompt handles navigation") }
        ))
        XCTAssertEqual(promptCount, 1)
    }
}
