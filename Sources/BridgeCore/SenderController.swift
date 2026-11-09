import ApplicationServices
import Carbon
import Foundation
import Network

/// Owns the active Quartz event tap and TCP client for either the CLI or the menu-bar app.
public final class SenderController {
    public private(set) var mode: BridgeMode = .local
    public var statusChanged: ((BridgeMode) -> Void)?
    public var connectionFailed: (() -> Void)?

    private var state = InputStateMachine()
    private var client: SenderClient?
    private var settings: BridgeSettings?
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var layoutSyncEnabled = false
    private var layoutSynchronization = LayoutSynchronizationState()
    private var inputSourceObserver: NSObjectProtocol?

    public init() {
        // Text Input Services publishes this documented distributed notification
        // whenever the user changes the selected keyboard input source. Sending
        // the source at that point avoids a race where the next key is delivered
        // using the Deck's previous layout.
        inputSourceObserver = DistributedNotificationCenter.default().addObserver(
            forName: Notification.Name(kTISNotifySelectedKeyboardInputSourceChanged as String),
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.inputSourceDidChange()
        }
    }

    deinit {
        if let inputSourceObserver {
            DistributedNotificationCenter.default().removeObserver(inputSourceObserver)
        }
    }

    /// Checks current trust without opening System Settings or showing a prompt.
    /// The menu-bar UI uses this to avoid displaying a stale permission error
    /// after the user has granted Accessibility access.
    public static func isAccessibilityTrusted() -> Bool {
        AXIsProcessTrusted()
    }

    public func configure(settings: BridgeSettings, pairingSecret: String) throws {
        self.settings = try settings.validated()
        client?.close()
        client = try SenderClient(pairingSecret: pairingSecret)
        client?.onReady = { [weak self] layoutSyncEnabled in
            DispatchQueue.main.async { self?.connectionReady(layoutSyncEnabled: layoutSyncEnabled) }
        }
        client?.onFailure = { [weak self] in
            DispatchQueue.main.async { self?.connectionLost() }
        }
    }

    public func start() throws {
        guard client != nil, settings != nil else { throw SenderControllerError.notConfigured }
        guard eventTap == nil else { return }
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        guard AXIsProcessTrustedWithOptions(options) else { throw SenderControllerError.accessibilityPermissionRequired }

        let mask = (1 << CGEventType.keyDown.rawValue) |
            (1 << CGEventType.keyUp.rawValue) |
            (1 << CGEventType.flagsChanged.rawValue)
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: CGEventMask(mask),
            callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                return Unmanaged<SenderController>.fromOpaque(context).takeUnretainedValue().handle(type: type, event: event)
            },
            userInfo: context
        ) else { throw SenderControllerError.eventTapUnavailable }

        eventTap = tap
        runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        CFRunLoopAddSource(CFRunLoopGetMain(), runLoopSource, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
    }

    public func toggle() {
        process(CapturedKeyEvent(kind: .keyDown, keyCode: 111, modifiers: [.control, .shift]))
    }

    public func stop() {
        if mode == .remote { client?.send(.releaseAll) }
        client?.close()
        if let eventTap { CFMachPortInvalidate(eventTap) }
        if let runLoopSource { CFRunLoopRemoveSource(CFRunLoopGetMain(), runLoopSource, .commonModes) }
        eventTap = nil
        runLoopSource = nil
        state.connectionLost()
        layoutSyncEnabled = false
        layoutSynchronization.reset()
        updateMode()
    }

    private func handle(type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        guard type == .keyDown || type == .keyUp || type == .flagsChanged else {
            if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
                state.connectionLost()
                client?.close()
                // Quartz disables a slow tap. Re-enable it only after returning to
                // LOCAL, so an event-tap failure can never keep suppressing input.
                if let eventTap { CGEvent.tapEnable(tap: eventTap, enable: true) }
                updateMode()
            }
            return Unmanaged.passUnretained(event)
        }

        let captured = CapturedKeyEvent(
            kind: type == .keyDown ? .keyDown : type == .keyUp ? .keyUp : .flagsChanged,
            keyCode: UInt16(event.getIntegerValueField(.keyboardEventKeycode)),
            modifiers: modifierState(from: event.flags),
            isRepeat: event.getIntegerValueField(.keyboardEventAutorepeat) != 0
        )
        return process(captured) ? nil : Unmanaged.passUnretained(event)
    }

    @discardableResult
    private func process(_ event: CapturedKeyEvent) -> Bool {
        var suppress = false
        for action in state.handle(event) {
            switch action {
            case .passThrough: break
            case let .suppress(packet):
                sendCurrentLayoutIfNeeded()
                client?.send(packet)
                suppress = true
            case .beginConnection:
                guard let settings else { break }
                client?.connect(host: settings.host, port: settings.port)
            case .releaseAndDisconnect:
                client?.send(.releaseAll)
                client?.close()
            }
        }
        updateMode()
        return suppress
    }

    private func connectionReady(layoutSyncEnabled: Bool) {
        self.layoutSyncEnabled = layoutSyncEnabled
        layoutSynchronization.reset()
        state.connectionReady()
        sendCurrentLayoutIfNeeded()
        updateMode()
    }

    private func connectionLost() {
        let wasConnectingOrRemote = state.mode != .local
        state.connectionLost()
        layoutSyncEnabled = false
        layoutSynchronization.reset()
        updateMode()
        if wasConnectingOrRemote { connectionFailed?() }
    }

    private func updateMode() {
        guard mode != state.mode else { return }
        mode = state.mode
        statusChanged?(mode)
    }

    private func modifierState(from flags: CGEventFlags) -> ModifierState {
        var modifiers: ModifierState = []
        if flags.contains(.maskControl) { modifiers.insert(.control) }
        if flags.contains(.maskShift) { modifiers.insert(.shift) }
        if flags.contains(.maskCommand) { modifiers.insert(.command) }
        if flags.contains(.maskAlternate) { modifiers.insert(.option) }
        if flags.contains(.maskAlphaShift) { modifiers.insert(.capsLock) }
        return modifiers
    }

    private func sendCurrentLayoutIfNeeded() {
        guard let packet = layoutSynchronization.nextPacket(
            for: Self.currentKeyboardLayoutIdentifier(),
            enabled: layoutSyncEnabled
        ) else { return }
        client?.send(packet)
    }

    private func inputSourceDidChange() {
        // Notification delivery uses the main queue above, the same queue that
        // owns SenderClient and the event-tap state.
        sendCurrentLayoutIfNeeded()
    }

    /// A stable, non-sensitive identifier used only for Deck layout mapping.
    /// It never represents typed content.
    public static func currentKeyboardLayoutIdentifier() -> String? {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
              let rawIdentifier = TISGetInputSourceProperty(source, kTISPropertyInputSourceID) else {
            return nil
        }
        let identifier = Unmanaged<CFString>.fromOpaque(rawIdentifier).takeUnretainedValue() as String
        let allowed = identifier.utf8.allSatisfy {
            (48...57).contains($0) || (65...90).contains($0) ||
                (97...122).contains($0) || $0 == 45 || $0 == 46 || $0 == 95
        }
        return allowed && !identifier.isEmpty && identifier.utf8.count <= 128 ? identifier : nil
    }
}

/// Tracks the last source sent on a session. Keeping this separate from the
/// event tap makes source-change and reconnect behavior deterministic to test.
struct LayoutSynchronizationState {
    private(set) var lastSentIdentifier: String?

    mutating func reset() {
        lastSentIdentifier = nil
    }

    mutating func nextPacket(for identifier: String?, enabled: Bool) -> BridgePacket? {
        guard enabled, let identifier, identifier != lastSentIdentifier else { return nil }
        lastSentIdentifier = identifier
        return .keyboardLayout(identifier: identifier)
    }
}

public enum SenderControllerError: LocalizedError {
    case notConfigured
    case accessibilityPermissionRequired
    case eventTapUnavailable

    public var errorDescription: String? {
        switch self {
        case .notConfigured: return "Configure the Steam Deck host and pairing secret first."
        case .accessibilityPermissionRequired: return "Accessibility permission is required. Enable the app in System Settings > Privacy & Security > Accessibility, then relaunch."
        case .eventTapUnavailable: return "Could not create the active keyboard event tap."
        }
    }
}

/// A short, bounded connection window that lets the Deck receiver finish
/// starting after the user has already requested REMOTE mode. Retrying is only
/// used before a session is authenticated; an established session still fails
/// closed immediately when it disconnects.
public struct ConnectionRetryPolicy: Equatable, Sendable {
    /// One initial attempt plus up to seven retries gives the Deck receiver
    /// enough time to launch without leaving the menu bar in CONNECTING forever.
    public static let deckReceiverStartup = ConnectionRetryPolicy(
        maximumAttempts: 8,
        retryDelay: 0.5,
        handshakeTimeout: 2
    )

    public let maximumAttempts: Int
    public let retryDelay: TimeInterval
    public let handshakeTimeout: TimeInterval

    public init(maximumAttempts: Int, retryDelay: TimeInterval, handshakeTimeout: TimeInterval) {
        precondition(maximumAttempts > 0)
        precondition(retryDelay >= 0)
        precondition(handshakeTimeout > 0)
        self.maximumAttempts = maximumAttempts
        self.retryDelay = retryDelay
        self.handshakeTimeout = handshakeTimeout
    }

    /// `attempt` is one-based and includes the attempt that just failed.
    public func shouldRetry(afterFailedAttempt attempt: Int) -> Bool {
        attempt > 0 && attempt < maximumAttempts
    }
}

private final class SenderClient {
    private static let maximumReceiveLength = SessionFramer.maximumFrameSize + 4

    private struct Endpoint {
        let host: String
        let port: UInt16
    }

    private let queue = DispatchQueue(label: "MacDeckInputBridge.transport")
    private let pairing: PairingSecret
    private let retryPolicy: ConnectionRetryPolicy
    private var connection: NWConnection?
    private var decoder = FrameDecoder()
    private var session: SessionFramer?
    private var isReady = false
    private var nextGeneration: UInt64 = 0
    private var activeGeneration: UInt64?
    private var nextPlan: UInt64 = 0
    private var activePlan: UInt64?
    private var endpoint: Endpoint?
    private var attempt = 0
    private var retryWorkItem: DispatchWorkItem?
    private var handshakeTimeoutWorkItem: DispatchWorkItem?
    var onReady: ((Bool) -> Void)?
    var onFailure: (() -> Void)?

    init(pairingSecret: String, retryPolicy: ConnectionRetryPolicy = .deckReceiverStartup) throws {
        pairing = try PairingSecret(base64: pairingSecret)
        self.retryPolicy = retryPolicy
    }

    func connect(host: String, port: UInt16) {
        queue.async { [weak self] in self?.beginConnection(host: host, port: port) }
    }

    func send(_ packet: BridgePacket) {
        queue.async { [weak self] in self?.sendApplicationPacket(packet) }
    }

    func close() {
        queue.async { [weak self] in self?.invalidateConnectionPlan() }
    }

    private func beginConnection(host: String, port: UInt16) {
        invalidateConnectionPlan()
        nextPlan &+= 1
        let plan = nextPlan
        activePlan = plan
        endpoint = Endpoint(host: host, port: port)
        attempt = 0
        startNextAttempt(plan: plan)
    }

    private func startNextAttempt(plan: UInt64) {
        guard activePlan == plan, let endpoint else { return }
        invalidateActiveConnection()
        attempt += 1
        nextGeneration &+= 1
        let generation = nextGeneration
        activeGeneration = generation
        decoder = FrameDecoder()
        session = nil
        isReady = false

        let connection = NWConnection(
            host: NWEndpoint.Host(endpoint.host),
            port: NWEndpoint.Port(rawValue: endpoint.port)!,
            using: .tcp
        )
        self.connection = connection
        connection.stateUpdateHandler = { [weak self] state in
            self?.handleConnectionState(state, generation: generation)
        }
        connection.start(queue: queue)
        armHandshakeTimeout(generation: generation, plan: plan)
    }

    private func handleConnectionState(_ state: NWConnection.State, generation: UInt64) {
        guard activeGeneration == generation else { return }
        switch state {
        case .ready:
            receiveNext(generation: generation)
        case .failed, .cancelled:
            failActiveConnection(generation: generation)
        default:
            break
        }
    }

    private func receiveNext(generation: UInt64) {
        guard activeGeneration == generation, let connection else { return }
        connection.receive(
            minimumIncompleteLength: 1,
            maximumLength: Self.maximumReceiveLength
        ) { [weak self] data, _, complete, error in
            self?.handleReceived(data: data, complete: complete, error: error, generation: generation)
        }
    }

    private func handleReceived(
        data: Data?,
        complete: Bool,
        error: NWError?,
        generation: UInt64
    ) {
        guard activeGeneration == generation else { return }
        do {
            if let data, !data.isEmpty {
                for frame in try decoder.append(data) {
                    try processReceivedFrame(frame, generation: generation)
                }
            }
            if error != nil || complete {
                failActiveConnection(generation: generation)
            } else {
                receiveNext(generation: generation)
            }
        } catch {
            failActiveConnection(generation: generation)
        }
    }

    private func processReceivedFrame(_ frame: Data, generation: UInt64) throws {
        guard activeGeneration == generation else { return }
        guard var session else {
            let hello = try ServerHello(encoded: frame)
            self.session = try SessionFramer(pairing: pairing, challenge: hello.challenge)
            try sendPacket(.clientReady, generation: generation)
            return
        }

        let packet = try session.open(combined: frame)
        self.session = session
        guard !isReady else { throw ProtocolError.invalidFrame }
        let layoutSyncEnabled: Bool
        switch packet {
        case .serverReady:
            layoutSyncEnabled = false
        case .serverReadyWithLayoutSync:
            layoutSyncEnabled = true
        default:
            throw ProtocolError.invalidFrame
        }
        isReady = true
        handshakeTimeoutWorkItem?.cancel()
        handshakeTimeoutWorkItem = nil
        onReady?(layoutSyncEnabled)
    }

    private func sendApplicationPacket(_ packet: BridgePacket) {
        guard isReady, let generation = activeGeneration else { return }
        do {
            try sendPacket(packet, generation: generation)
        } catch {
            failActiveConnection(generation: generation)
        }
    }

    private func sendPacket(_ packet: BridgePacket, generation: UInt64) throws {
        guard activeGeneration == generation, let connection, var session else { throw ProtocolError.invalidFrame }
        let frame = try session.seal(packet)
        self.session = session
        connection.send(content: frame, completion: .contentProcessed { [weak self] error in
            if error != nil { self?.failActiveConnection(generation: generation) }
        })
    }

    private func failActiveConnection(generation: UInt64) {
        guard activeGeneration == generation else { return }
        let wasReady = isReady
        guard let plan = activePlan else {
            invalidateActiveConnection()
            return
        }
        invalidateActiveConnection()

        if !wasReady, retryPolicy.shouldRetry(afterFailedAttempt: attempt) {
            scheduleRetry(plan: plan)
        } else {
            finishConnectionPlan(plan: plan)
        }
    }

    private func armHandshakeTimeout(generation: UInt64, plan: UInt64) {
        handshakeTimeoutWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.handshakeTimedOut(generation: generation, plan: plan)
        }
        handshakeTimeoutWorkItem = workItem
        queue.asyncAfter(deadline: .now() + retryPolicy.handshakeTimeout, execute: workItem)
    }

    private func handshakeTimedOut(generation: UInt64, plan: UInt64) {
        guard activePlan == plan, activeGeneration == generation, !isReady else { return }
        failActiveConnection(generation: generation)
    }

    private func scheduleRetry(plan: UInt64) {
        retryWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.startNextAttempt(plan: plan)
        }
        retryWorkItem = workItem
        queue.asyncAfter(deadline: .now() + retryPolicy.retryDelay, execute: workItem)
    }

    private func finishConnectionPlan(plan: UInt64) {
        guard activePlan == plan else { return }
        retryWorkItem?.cancel()
        retryWorkItem = nil
        handshakeTimeoutWorkItem?.cancel()
        handshakeTimeoutWorkItem = nil
        activePlan = nil
        endpoint = nil
        attempt = 0
        onFailure?()
    }

    private func invalidateConnectionPlan() {
        retryWorkItem?.cancel()
        retryWorkItem = nil
        handshakeTimeoutWorkItem?.cancel()
        handshakeTimeoutWorkItem = nil
        activePlan = nil
        endpoint = nil
        attempt = 0
        invalidateActiveConnection()
    }

    private func invalidateActiveConnection() {
        handshakeTimeoutWorkItem?.cancel()
        handshakeTimeoutWorkItem = nil
        let connection = self.connection
        activeGeneration = nil
        self.connection = nil
        decoder = FrameDecoder()
        session = nil
        isReady = false
        connection?.cancel()
    }
}
