import ApplicationServices
import Foundation
import Network

/// Owns the active Quartz event tap and TCP client for either the CLI or the menu-bar app.
public final class SenderController {
    public private(set) var mode: BridgeMode = .local
    public var statusChanged: ((BridgeMode) -> Void)?

    private var state = InputStateMachine()
    private var client: SenderClient?
    private var settings: BridgeSettings?
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?

    public init() {}

    public func configure(settings: BridgeSettings, pairingSecret: String) throws {
        self.settings = try settings.validated()
        client?.close()
        client = try SenderClient(pairingSecret: pairingSecret)
        client?.onReady = { [weak self] in
            DispatchQueue.main.async { self?.connectionReady() }
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

    private func connectionReady() {
        state.connectionReady()
        updateMode()
    }

    private func connectionLost() {
        state.connectionLost()
        updateMode()
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

private final class SenderClient {
    private static let maximumReceiveLength = SessionFramer.maximumFrameSize + 4

    private let queue = DispatchQueue(label: "MacDeckInputBridge.transport")
    private let pairing: PairingSecret
    private var connection: NWConnection?
    private var decoder = FrameDecoder()
    private var session: SessionFramer?
    private var isReady = false
    private var nextGeneration: UInt64 = 0
    private var activeGeneration: UInt64?
    var onReady: (() -> Void)?
    var onFailure: (() -> Void)?

    init(pairingSecret: String) throws {
        pairing = try PairingSecret(base64: pairingSecret)
    }

    func connect(host: String, port: UInt16) {
        queue.async { [weak self] in self?.startConnection(host: host, port: port) }
    }

    func send(_ packet: BridgePacket) {
        queue.async { [weak self] in self?.sendApplicationPacket(packet) }
    }

    func close() {
        queue.async { [weak self] in self?.invalidateActiveConnection() }
    }

    private func startConnection(host: String, port: UInt16) {
        invalidateActiveConnection()
        nextGeneration &+= 1
        let generation = nextGeneration
        activeGeneration = generation
        decoder = FrameDecoder()
        session = nil
        isReady = false

        let connection = NWConnection(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: port)!,
            using: .tcp
        )
        self.connection = connection
        connection.stateUpdateHandler = { [weak self] state in
            self?.handleConnectionState(state, generation: generation)
        }
        connection.start(queue: queue)
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
        guard packet == .serverReady else { throw ProtocolError.invalidFrame }
        isReady = true
        onReady?()
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
        let connection = self.connection
        activeGeneration = nil
        self.connection = nil
        session = nil
        isReady = false
        connection?.cancel()
        onFailure?()
    }

    private func invalidateActiveConnection() {
        let connection = self.connection
        activeGeneration = nil
        self.connection = nil
        decoder = FrameDecoder()
        session = nil
        isReady = false
        connection?.cancel()
    }
}
