import XCTest
@testable import BridgeCore

final class AccessibilityPermissionTests: XCTestCase {
    func testTrustedProcessDoesNotPromptOrOpenSettings() {
        var permission = AccessibilityPermission()
        XCTAssertTrue(permission.checkAndRequest(
            isTrusted: { true },
            showPrompt: { XCTFail("Should not prompt"); return false },
            openSettings: { XCTFail("Should not open Settings") }
        ))
    }

    func testInitialRequestRegistersAppEvenWhenSettingsAreAlreadyOpen() {
        var permission = AccessibilityPermission()
        var promptCount = 0
        var settingsOpenCount = 0
        for _ in 0..<3 {
            XCTAssertFalse(permission.checkAndRequest(
                isTrusted: { false },
                showPrompt: { promptCount += 1; return false },
                openSettings: { settingsOpenCount += 1 }
            ))
        }
        XCTAssertEqual(promptCount, 1)
        XCTAssertEqual(settingsOpenCount, 2)
    }

    func testFreshProcessRequestsAgainAfterResetOrReinstall() {
        var promptCount = 0
        // No persisted "already requested" flag may skip registration on launch.
        for _ in 0..<2 {
            var permission = AccessibilityPermission()
            XCTAssertFalse(permission.checkAndRequest(
                isTrusted: { false },
                showPrompt: { promptCount += 1; return false },
                openSettings: { XCTFail("New process must request registration") }
            ))
        }
        XCTAssertEqual(promptCount, 2)
    }

    func testRevokedAccessCanBeRequestedAgainAfterTrustWasObserved() {
        var permission = AccessibilityPermission()
        var promptCount = 0
        let request = { promptCount += 1; return false }
        let unexpectedOpen = { XCTFail("Should request registration after revocation") }
        XCTAssertFalse(permission.checkAndRequest(isTrusted: { false }, showPrompt: request, openSettings: unexpectedOpen))
        XCTAssertTrue(permission.checkAndRequest(isTrusted: { true }, showPrompt: request, openSettings: unexpectedOpen))
        XCTAssertFalse(permission.checkAndRequest(isTrusted: { false }, showPrompt: request, openSettings: unexpectedOpen))
        XCTAssertEqual(promptCount, 2)
    }
}
