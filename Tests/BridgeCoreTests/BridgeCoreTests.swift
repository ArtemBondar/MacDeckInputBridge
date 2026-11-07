import BridgeCore
import XCTest

final class BridgeCoreTests: XCTestCase {
    private let pairing = try! PairingSecret(base64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")

    func testPacketRoundTripThroughAuthenticatedFrame() throws {
        let framer = SecureFramer(pairing: pairing)
        let frame = try framer.seal(.key(code: 30, value: 1))
        var decoder = FrameDecoder()
        let encrypted = try XCTUnwrap(decoder.append(frame).first)
        XCTAssertEqual(try framer.open(combined: encrypted), .key(code: 30, value: 1))
    }

    func testDecoderHandlesFragmentedAndCoalescedFrames() throws {
        let framer = SecureFramer(pairing: pairing)
        let first = try framer.seal(.ping)
        let second = try framer.seal(.releaseAll)
        var decoder = FrameDecoder()
        XCTAssertTrue(try decoder.append(first.prefix(2)).isEmpty)
        let frames = try decoder.append(first.dropFirst(2) + second)
        XCTAssertEqual(frames.count, 2)
        XCTAssertEqual(try framer.open(combined: frames[0]), .ping)
        XCTAssertEqual(try framer.open(combined: frames[1]), .releaseAll)
    }

    func testPhysicalKeyMapping() {
        XCTAssertEqual(KeyMapping.linuxCode(for: 0), 30) // A
        XCTAssertEqual(KeyMapping.linuxCode(for: 111), 88) // F12
        XCTAssertEqual(KeyMapping.linuxCode(for: 59), 29) // left control
        XCTAssertNil(KeyMapping.linuxCode(for: 999))
    }

    func testTopNumberRowMapsByPhysicalKey() {
        let expected: [UInt16: UInt16] = [
            18: 2, 19: 3, 20: 4, 21: 5, 22: 7, 23: 6,
            24: 13, 25: 10, 26: 8, 27: 12, 28: 9, 29: 11
        ]
        for (macKeyCode, linuxCode) in expected {
            XCTAssertEqual(KeyMapping.linuxCode(for: macKeyCode), linuxCode, "macOS keycode \(macKeyCode)")
        }
    }

    func testConnectingNeverSuppressesAndRemoteToggleFailsSafe() {
        var machine = InputStateMachine()
        let hotkey = CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift])
        XCTAssertEqual(machine.handle(hotkey), [.passThrough, .beginConnection])
        XCTAssertEqual(machine.mode, .connecting)
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)), [.passThrough])
        machine.connectionReady()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)), [.suppress(.key(code: 30, value: 1))])
        XCTAssertEqual(machine.handle(hotkey), [.passThrough, .releaseAndDisconnect])
        XCTAssertEqual(machine.mode, .local)
    }

    func testHotkeyNeverBecomesForwardedKey() {
        var machine = InputStateMachine()
        _ = machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift]))
        machine.connectionReady()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: 111)), [.passThrough])
    }

    func testModifierPressedLocallyBeforeConnectKeepsItsKeyUpLocal() {
        var machine = InputStateMachine()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 59, modifiers: [.control])), [.passThrough])
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift])), [.passThrough, .beginConnection])
        machine.connectionReady()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: 59)), [.passThrough])
    }

    func testHotkeyShiftIsForwardedAfterItsLocalRelease() {
        var machine = InputStateMachine()
        _ = machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 56, modifiers: [.shift]))
        _ = machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift]))
        machine.connectionReady()

        XCTAssertEqual(
            machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 56)),
            [.passThrough]
        )
        XCTAssertEqual(
            machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 56, modifiers: [.shift])),
            [.suppress(.key(code: 42, value: 1))]
        )
    }

    func testRemoteFlagsChangedForOptionForwardsCorrectState() {
        var machine = InputStateMachine()
        _ = machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift]))
        machine.connectionReady()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 58, modifiers: [.option])), [.suppress(.key(code: 56, value: 1))])
    }
}
