@testable import BridgeCore
import Security
import XCTest

final class BridgeCoreTests: XCTestCase {
    private let pairing = try! PairingSecret(base64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
    private let expectedMappings: [UInt16: UInt16] = [
        0: 30, 1: 31, 2: 32, 3: 33, 4: 35, 5: 34, 6: 44, 7: 45, 8: 46, 9: 47,
        10: 86, 11: 48, 12: 16, 13: 17, 14: 18, 15: 19, 16: 21, 17: 20, 18: 2,
        19: 3, 20: 4, 21: 5, 22: 7, 23: 6, 24: 13, 25: 10, 26: 8, 27: 12,
        28: 9, 29: 11, 30: 27, 31: 24, 32: 22, 33: 26, 34: 23, 35: 25, 36: 28,
        37: 38, 38: 36, 39: 40, 40: 37, 41: 39, 42: 43, 43: 51, 44: 53, 45: 49,
        46: 50, 47: 52, 48: 15, 49: 57, 50: 41, 51: 14, 53: 1, 54: 126, 55: 125,
        56: 42, 57: 58, 58: 56, 59: 29, 60: 54, 61: 100, 62: 97, 64: 187, 65: 83,
        67: 55, 69: 78, 71: 69, 72: 115, 73: 114, 74: 113, 75: 98, 76: 96,
        78: 74, 79: 188, 80: 189, 81: 117, 82: 82, 83: 79, 84: 80, 85: 81,
        86: 75, 87: 76, 88: 77, 89: 71, 90: 190, 91: 89, 92: 90, 96: 63, 97: 64,
        98: 65, 99: 61, 100: 66, 101: 67, 103: 87, 105: 183, 106: 186, 107: 184,
        109: 68, 110: 127, 111: 88, 113: 185, 114: 110, 115: 102, 116: 104,
        117: 111, 118: 62, 119: 107, 120: 60, 121: 109, 122: 59, 123: 105,
        124: 106, 125: 108, 126: 103
    ]

    private final class KeychainDeniedStore: PairingSecretStoring {
        func readSecret() throws -> String? { throw BridgeSettingsError.keychain(errSecInteractionNotAllowed) }
        func saveSecret(_: String) throws { throw BridgeSettingsError.keychain(errSecInteractionNotAllowed) }
        func deleteSecret() throws { throw BridgeSettingsError.keychain(errSecInteractionNotAllowed) }
    }

    private func remoteMachine() -> InputStateMachine {
        var machine = InputStateMachine()
        _ = machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 59, modifiers: [.control]))
        _ = machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 56, modifiers: [.control, .shift]))
        _ = machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift]))
        machine.connectionReady()
        _ = machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: 111))
        _ = machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 56, modifiers: [.control]))
        _ = machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: 59))
        return machine
    }

    func testReceiverStartupRetryPolicyIsBounded() {
        let policy = ConnectionRetryPolicy.deckReceiverStartup
        XCTAssertEqual(policy.maximumAttempts, 8)
        XCTAssertEqual(policy.retryDelay, 0.5)
        XCTAssertEqual(policy.handshakeTimeout, 2)

        for attempt in 1..<policy.maximumAttempts {
            XCTAssertTrue(policy.shouldRetry(afterFailedAttempt: attempt), "attempt \(attempt) should retry")
        }
        XCTAssertFalse(policy.shouldRetry(afterFailedAttempt: policy.maximumAttempts))
        XCTAssertFalse(policy.shouldRetry(afterFailedAttempt: 0))
    }

    func testSessionRoundTripThroughAuthenticatedFrame() throws {
        let challenge = Data(repeating: 7, count: ServerHello.challengeLength)
        var sender = try SessionFramer(pairing: pairing, challenge: challenge)
        var receiver = try SessionFramer(pairing: pairing, challenge: challenge)
        let frame = try sender.seal(.clientReady)
        var decoder = FrameDecoder()
        let encrypted = try XCTUnwrap(decoder.append(frame).first)
        XCTAssertEqual(try receiver.open(combined: encrypted), .clientReady)
    }

    func testServerHelloIsStrictlyVersionedAndLengthFramed() throws {
        let hello = try ServerHello(challenge: Data(repeating: 9, count: ServerHello.challengeLength))
        var decoder = FrameDecoder()
        let frames = try decoder.append(SessionFramer.lengthFrame(hello.encoded()))
        XCTAssertEqual(try ServerHello(encoded: try XCTUnwrap(frames.first)), hello)
        XCTAssertThrowsError(try ServerHello(encoded: Data([0x10, 2]) + Data(repeating: 0, count: 32)))
    }

    func testDecoderHandlesFragmentedAndCoalescedFrames() throws {
        let challenge = Data(repeating: 8, count: ServerHello.challengeLength)
        var sender = try SessionFramer(pairing: pairing, challenge: challenge)
        var receiver = try SessionFramer(pairing: pairing, challenge: challenge)
        let first = try sender.seal(.clientReady)
        let second = try sender.seal(.ping)
        var decoder = FrameDecoder()
        XCTAssertTrue(try decoder.append(first.prefix(2)).isEmpty)
        let frames = try decoder.append(first.dropFirst(2) + second)
        XCTAssertEqual(frames.count, 2)
        XCTAssertEqual(try receiver.open(combined: frames[0]), .clientReady)
        XCTAssertEqual(try receiver.open(combined: frames[1]), .ping)
    }

    func testFramingWorksAtEveryTCPFragmentBoundary() throws {
        let challenge = Data(repeating: 6, count: ServerHello.challengeLength)
        var sender = try SessionFramer(pairing: pairing, challenge: challenge)
        let frame = try sender.seal(.clientReady)
        for splitIndex in 0...frame.count {
            var receiver = try SessionFramer(pairing: pairing, challenge: challenge)
            var decoder = FrameDecoder()
            let firstFrames = try decoder.append(frame.prefix(splitIndex))
            let secondFrames = try decoder.append(frame.dropFirst(splitIndex))
            let frames = firstFrames + secondFrames
            let encrypted = try XCTUnwrap(frames.first)
            XCTAssertEqual(frames.count, 1)
            XCTAssertEqual(try receiver.open(combined: encrypted), .clientReady)
        }
    }

    func testSessionRejectsReplayedAndOutOfOrderFrames() throws {
        let challenge = Data(repeating: 5, count: ServerHello.challengeLength)
        var sender = try SessionFramer(pairing: pairing, challenge: challenge)
        var receiver = try SessionFramer(pairing: pairing, challenge: challenge)
        let ready = try sender.seal(.clientReady)
        let key = try sender.seal(.key(code: 30, value: 1))
        var decoder = FrameDecoder()
        let frames = try decoder.append(ready + key)
        XCTAssertEqual(try receiver.open(combined: frames[0]), .clientReady)
        XCTAssertThrowsError(try receiver.open(combined: frames[0])) { error in
            XCTAssertEqual(error as? ProtocolError, .unexpectedSequence)
        }
        XCTAssertEqual(try receiver.open(combined: frames[1]), .key(code: 30, value: 1))
    }

    func testInvalidPacketsAreRejected() {
        XCTAssertNil(BridgePacket(encoded: Data()))
        XCTAssertNil(BridgePacket(encoded: Data([1, 0, 30, 3])))
        XCTAssertNil(BridgePacket(encoded: Data([9])))
        XCTAssertNil(BridgePacket(encoded: Data([7, 2, 0x75])))
        XCTAssertEqual(BridgePacket(encoded: Data([4])), .clientReady)
        XCTAssertEqual(BridgePacket(encoded: Data([5])), .serverReady)
        XCTAssertEqual(BridgePacket(encoded: Data([6])), .serverReadyWithLayoutSync)
        XCTAssertEqual(
            BridgePacket(encoded: Data([7, 22] + Array("com.apple.keylayout.US".utf8))),
            .keyboardLayout(identifier: "com.apple.keylayout.US")
        )
    }

    func testLayoutSynchronizationSendsOnEachSourceChangeAndAfterReconnect() {
        var synchronization = LayoutSynchronizationState()
        let us = "com.apple.keylayout.US"
        let ukrainian = "com.apple.keylayout.Ukrainian"

        XCTAssertNil(synchronization.nextPacket(for: us, enabled: false))
        XCTAssertEqual(
            synchronization.nextPacket(for: us, enabled: true),
            .keyboardLayout(identifier: us)
        )
        XCTAssertNil(synchronization.nextPacket(for: us, enabled: true))
        XCTAssertEqual(
            synchronization.nextPacket(for: ukrainian, enabled: true),
            .keyboardLayout(identifier: ukrainian)
        )
        XCTAssertEqual(
            synchronization.nextPacket(for: us, enabled: true),
            .keyboardLayout(identifier: us)
        )

        synchronization.reset()
        XCTAssertEqual(
            synchronization.nextPacket(for: us, enabled: true),
            .keyboardLayout(identifier: us)
        )
        XCTAssertNil(synchronization.nextPacket(for: nil, enabled: true))
    }

    func testLayoutSourceTransitionConfirmsOldOldNewBeforeSending() {
        var synchronization = LayoutSynchronizationState()
        let us = "com.apple.keylayout.US"
        let ukrainian = "com.apple.keylayout.Ukrainian"

        XCTAssertEqual(
            synchronization.nextPacket(for: us, enabled: true),
            .keyboardLayout(identifier: us)
        )

        // A TIS notification can arrive while its source query still reports
        // US. Two stale reads must not send US again; only the observed new
        // source can produce a packet.
        let generation = synchronization.beginSourceChange(currentIdentifier: us)
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: generation, for: us, enabled: true),
            .retry(after: LayoutSynchronizationState.sourceObservationBackoff[0])
        )
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: generation, for: us, enabled: true),
            .retry(after: LayoutSynchronizationState.sourceObservationBackoff[1])
        )
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: generation, for: ukrainian, enabled: true),
            .send(.keyboardLayout(identifier: ukrainian))
        )
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: generation, for: ukrainian, enabled: true),
            .none
        )
    }

    func testLayoutSourceTransitionCoalescesRapidChangesByGeneration() {
        var synchronization = LayoutSynchronizationState()
        let us = "com.apple.keylayout.US"
        let ukrainian = "com.apple.keylayout.Ukrainian"
        let polish = "com.apple.keylayout.Polish"
        _ = synchronization.nextPacket(for: us, enabled: true)

        let firstGeneration = synchronization.beginSourceChange(currentIdentifier: us)
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: firstGeneration, for: us, enabled: true),
            .retry(after: LayoutSynchronizationState.sourceObservationBackoff[0])
        )
        // A later notification replaces the pending generation. An observation
        // from the old work item is ignored, and only the final source is sent.
        let finalGeneration = synchronization.beginSourceChange(currentIdentifier: ukrainian)
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: firstGeneration, for: ukrainian, enabled: true),
            .none
        )
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: finalGeneration, for: polish, enabled: true),
            .send(.keyboardLayout(identifier: polish))
        )
    }

    func testLayoutSourceTransitionReturningToOriginalTimesOutWithoutSending() {
        var synchronization = LayoutSynchronizationState()
        let us = "com.apple.keylayout.US"
        _ = synchronization.nextPacket(for: us, enabled: true)

        // The user can cycle away from US and back before TIS exposes the
        // intermediate source. The Deck is already US, so timing out without a
        // packet is safer than guessing that a change happened.
        let generation = synchronization.beginSourceChange(currentIdentifier: us)
        for delay in LayoutSynchronizationState.sourceObservationBackoff {
            XCTAssertEqual(
                synchronization.observeSourceChange(generation: generation, for: us, enabled: true),
                .retry(after: delay)
            )
        }
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: generation, for: us, enabled: true),
            .timedOut
        )
    }

    func testLayoutSourceTransitionTimesOutAndCancellationSendsNothing() {
        var synchronization = LayoutSynchronizationState()
        let us = "com.apple.keylayout.US"
        _ = synchronization.nextPacket(for: us, enabled: true)

        let generation = synchronization.beginSourceChange(currentIdentifier: us)
        for delay in LayoutSynchronizationState.sourceObservationBackoff {
            XCTAssertEqual(
                synchronization.observeSourceChange(generation: generation, for: nil, enabled: true),
                .retry(after: delay)
            )
        }
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: generation, for: nil, enabled: true),
            .timedOut
        )

        let cancelledGeneration = synchronization.beginSourceChange(currentIdentifier: us)
        synchronization.reset()
        XCTAssertEqual(
            synchronization.observeSourceChange(generation: cancelledGeneration, for: us, enabled: true),
            .none
        )
    }

    func testSettingsStorePersistsEndpointAndKeychainSecret() throws {
        let suite = "MacDeckInputBridgeTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let secretStore = InMemoryPairingSecretStore()
        let store = BridgeSettingsStore(defaults: defaults, secretStore: secretStore)
        let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

        try store.saveSettings(BridgeSettings(host: " 192.168.1.5 ", port: 4242))
        try store.savePairingSecret(secret)

        XCTAssertEqual(store.loadSettings(), BridgeSettings(host: "192.168.1.5", port: 4242))
        XCTAssertEqual(try store.loadPairingSecret(), secret)
    }

    func testSettingsValidationRejectsEmptyHostAndInvalidSecret() {
        XCTAssertThrowsError(try BridgeSettings(host: "", port: 4242).validated()) { error in
            XCTAssertEqual(error as? BridgeSettingsError, .missingHost)
        }
        let store = InMemoryPairingSecretStore()
        XCTAssertThrowsError(try store.saveSecret("not-a-pairing-secret"))
    }

    func testSettingsStoreFallsBackWhenPersistedPortIsCorrupt() throws {
        let suite = "MacDeckInputBridgeTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(99_999, forKey: "deckPort")
        let store = BridgeSettingsStore(defaults: defaults, secretStore: InMemoryPairingSecretStore())
        XCTAssertEqual(store.loadSettings().port, 4242)
    }

    func testDeniedKeychainKeepsSecretInMemoryUnlessLocalFallbackIsExplicitlyEnabled() throws {
        let suite = "MacDeckInputBridgeTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let localStore = InMemoryPairingSecretStore()
        let store = BridgeSettingsStore(
            defaults: defaults,
            secretStore: KeychainDeniedStore(),
            localSecretStore: localStore
        )
        let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

        XCTAssertEqual(try store.savePairingSecret(secret), .memoryOnly)
        XCTAssertNil(try localStore.readSecret())

        try store.setAllowsLocalSecretFallback(true)
        XCTAssertEqual(try store.savePairingSecret(secret), .localPrivateFile)
        XCTAssertEqual(try store.loadPairingSecret(), secret)

        try store.setAllowsLocalSecretFallback(false)
        XCTAssertNil(try localStore.readSecret())
    }

    func testLocalFallbackUsesAnOwnerOnlyRegularFile() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("MacDeckInputBridgeTests.\(UUID().uuidString)", isDirectory: true)
        let secretURL = directory.appendingPathComponent("pairing-secret")
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = LocalPairingSecretStore(fileURL: secretURL)
        let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

        try store.saveSecret(secret)
        let attributes = try FileManager.default.attributesOfItem(atPath: secretURL.path)
        XCTAssertEqual(attributes[.posixPermissions] as? Int, 0o600)
        let directoryAttributes = try FileManager.default.attributesOfItem(atPath: directory.path)
        XCTAssertEqual(directoryAttributes[.posixPermissions] as? Int, 0o700)
        XCTAssertEqual(try store.readSecret(), secret)
    }

    func testPhysicalKeyMapping() {
        XCTAssertEqual(KeyMapping.supportedMappings, expectedMappings)
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

    func testANSIAlphabetMapsToMatchingLinuxKeys() {
        let alphabet: [(UInt16, UInt16)] = [
            (0, 30), (11, 48), (8, 46), (2, 32), (14, 18), (3, 33), (5, 34),
            (4, 35), (34, 23), (38, 36), (40, 37), (37, 38), (46, 50), (45, 49),
            (31, 24), (35, 25), (12, 16), (15, 19), (1, 31), (17, 20), (32, 22),
            (9, 47), (13, 17), (7, 45), (16, 21), (6, 44)
        ]
        for (macKeyCode, linuxCode) in alphabet {
            XCTAssertEqual(KeyMapping.linuxCode(for: macKeyCode), linuxCode, "macOS keycode \(macKeyCode)")
        }
    }

    func testEveryMappedKeyForwardsDownUpAndRepeat() {
        for (macKeyCode, linuxCode) in expectedMappings {
            var machine = remoteMachine()
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: macKeyCode)),
                [.suppress(.key(code: linuxCode, value: 1))],
                "down for macOS keycode \(macKeyCode)"
            )
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: macKeyCode, isRepeat: true)),
                [.suppress(.key(code: linuxCode, value: 2))],
                "repeat for macOS keycode \(macKeyCode)"
            )
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: macKeyCode)),
                [.suppress(.key(code: linuxCode, value: 0))],
                "up for macOS keycode \(macKeyCode)"
            )
        }
    }

    func testConnectingNeverSuppressesAndRemoteToggleFailsSafe() {
        var machine = InputStateMachine()
        let hotkey = CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift])
        XCTAssertEqual(machine.handle(hotkey), [.passThrough, .beginConnection])
        XCTAssertEqual(machine.mode, .connecting)
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)), [.passThrough])
        machine.connectionReady()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: 0)), [.passThrough])
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)), [.suppress(.key(code: 30, value: 1))])
        XCTAssertEqual(machine.handle(hotkey), [.passThrough, .releaseAndDisconnect])
        XCTAssertEqual(machine.mode, .local)
    }

    func testSecondHotkeyCancelsConnecting() {
        var machine = InputStateMachine()
        let hotkey = CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift])
        XCTAssertEqual(machine.handle(hotkey), [.passThrough, .beginConnection])
        XCTAssertEqual(machine.handle(hotkey), [.passThrough, .releaseAndDisconnect])
        XCTAssertEqual(machine.mode, .local)
    }

    func testPartialAndRepeatedHotkeysStayLocal() {
        var machine = InputStateMachine()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control])), [.passThrough])
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.shift])), [.passThrough])
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift], isRepeat: true)), [.passThrough])
        XCTAssertEqual(machine.mode, .local)
    }

    func testConnectionLossAlwaysReturnsToLocal() {
        var machine = remoteMachine()
        machine.connectionLost()
        XCTAssertEqual(machine.mode, .local)
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)), [.passThrough])
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

    func testOrdinaryKeyPressedLocallyBeforeRemoteKeepsItsKeyUpLocal() {
        var machine = InputStateMachine()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)), [.passThrough])
        _ = machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift]))
        machine.connectionReady()
        XCTAssertEqual(machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: 0)), [.passThrough])
        XCTAssertEqual(
            machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0)),
            [.suppress(.key(code: 30, value: 1))]
        )
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

    func testEveryModifierFlagsTransitionForwardsCorrectState() {
        let modifiers: [(UInt16, ModifierState, UInt16)] = [
            (54, .command, 126), (55, .command, 125), (56, .shift, 42), (57, .capsLock, 58),
            (58, .option, 56), (59, .control, 29), (60, .shift, 54), (61, .option, 100),
            (62, .control, 97)
        ]
        for (macKeyCode, flag, linuxCode) in modifiers {
            var machine = remoteMachine()
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: macKeyCode, modifiers: [flag])),
                [.suppress(.key(code: linuxCode, value: 1))],
                "down for modifier \(macKeyCode)"
            )
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .flagsChanged, keyCode: macKeyCode)),
                [.suppress(.key(code: linuxCode, value: 0))],
                "up for modifier \(macKeyCode)"
            )
        }
    }

    func testOrdinaryKeyForwardsUnderEveryModifierCombination() {
        let flags: [ModifierState] = [.control, .shift, .command, .option, .capsLock]
        for rawValue in 0..<(1 << flags.count) {
            let modifiers = flags.enumerated().reduce(into: ModifierState()) { result, item in
                if rawValue & (1 << item.offset) != 0 { result.insert(item.element) }
            }
            var machine = remoteMachine()
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .keyDown, keyCode: 0, modifiers: modifiers)),
                [.suppress(.key(code: 30, value: 1))],
                "modifier mask \(rawValue)"
            )
            XCTAssertEqual(
                machine.handle(CapturedKeyEvent(kind: .keyUp, keyCode: 0, modifiers: modifiers)),
                [.suppress(.key(code: 30, value: 0))],
                "modifier mask \(rawValue)"
            )
        }
    }

    func testClipboardProtocolChunksBoundedUTF8TextAndKeepsDirectionExplicit() throws {
        let text = String(repeating: "🙂", count: 1_100)
        let packets = try XCTUnwrap(
            BridgePacket.clipboardPackets(origin: .mac, transferID: 42, text: text)
        )
        XCTAssertEqual(
            packets.first,
            .clipboardStart(
                origin: .mac,
                transferID: 42,
                totalBytes: UInt32(text.utf8.count),
                chunkCount: 2
            )
        )
        XCTAssertEqual(packets.last, .clipboardEnd(origin: .mac, transferID: 42))
        for packet in packets {
            XCTAssertEqual(BridgePacket(encoded: packet.encoded()), packet)
            if case let .clipboardChunk(_, _, _, bytes) = packet {
                XCTAssertFalse(bytes.isEmpty)
                XCTAssertLessThanOrEqual(bytes.count, BridgePacket.maximumClipboardChunkBytes)
            }
        }
    }

    func testClipboardProtocolRejectsMalformedAndOversizedTransfers() {
        XCTAssertNil(BridgePacket(encoded: Data([8, ClipboardOrigin.mac.rawValue] + Array(repeating: 0, count: 12) + [0, 1])))
        XCTAssertNil(BridgePacket(encoded: Data([9, ClipboardOrigin.mac.rawValue] + Array(repeating: 0, count: 10))))
        XCTAssertNil(BridgePacket.clipboardPackets(
            origin: .mac,
            transferID: 1,
            text: String(repeating: "x", count: BridgePacket.maximumClipboardTextBytes + 1)
        ))
        XCTAssertNil(BridgePacket(encoded: Data([10, ClipboardOrigin.steamDeck.rawValue] + Array(repeating: 0, count: 7))))
        XCTAssertNil(BridgePacket(encoded: Data([13, ClipboardOrigin.mac.rawValue] + Array(repeating: 0, count: 8) + [99])))
    }

    func testClipboardDeliveryAcknowledgementKeepsContentOutOfTheProtocol() {
        let packet = BridgePacket.clipboardDelivery(
            origin: .mac,
            transferID: 42,
            delivery: .written
        )
        XCTAssertEqual(
            packet.encoded(),
            Data([13, ClipboardOrigin.mac.rawValue] + Array(repeating: 0, count: 7) + [42, ClipboardDelivery.written.rawValue])
        )
        XCTAssertEqual(BridgePacket(encoded: packet.encoded()), packet)
    }

    func testClipboardSynchronizationDoesNotSendExistingDataOrAfterCancellation() {
        var synchronization = ClipboardSynchronizationState()
        XCTAssertNil(synchronization.packetsForChange(changeCount: 1, text: "private", origin: .mac))
        synchronization.start(changeCount: 1)
        XCTAssertNil(synchronization.packetsForChange(changeCount: 1, text: "private", origin: .mac))
        let packets = synchronization.packetsForChange(changeCount: 2, text: "new text", origin: .mac)
        XCTAssertEqual(packets?.first, .clipboardStart(origin: .mac, transferID: 1, totalBytes: 8, chunkCount: 1))
        synchronization.reset()
        XCTAssertNil(synchronization.packetsForChange(changeCount: 3, text: "after disconnect", origin: .mac))
    }

    func testClipboardSynchronizationIsDisabledByDefaultAndPersistsOnlyTheChoice() throws {
        let suite = "MacDeckInputBridgeTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = BridgeSettingsStore(defaults: defaults, secretStore: InMemoryPairingSecretStore())
        XCTAssertFalse(store.loadSettings().syncClipboardFromMac)
        try store.saveSettings(BridgeSettings(host: "deck.local", syncClipboardFromMac: true))
        XCTAssertTrue(store.loadSettings().syncClipboardFromMac)
    }
}

private extension Data {
    init(hexadecimal: String) {
        precondition(hexadecimal.count.isMultiple(of: 2))
        self.init((0..<hexadecimal.count).compactMap { offset in
            guard offset.isMultiple(of: 2) else { return nil }
            let start = hexadecimal.index(hexadecimal.startIndex, offsetBy: offset)
            let end = hexadecimal.index(start, offsetBy: 2)
            return UInt8(hexadecimal[start..<end], radix: 16)
        })
    }
}
