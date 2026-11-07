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
    private let queue = DispatchQueue(label: "MacDeckInputBridge.transport")
    private let framer: SecureFramer
    private var connection: NWConnection?
    var onReady: (() -> Void)?
    var onFailure: (() -> Void)?

    init(pairingSecret: String) throws {
        framer = try SecureFramer(pairing: PairingSecret(base64: pairingSecret))
    }

    func connect(host: String, port: UInt16) {
        close()
        let connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!, using: .tcp)
        self.connection = connection
        connection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready: self?.onReady?()
            case .failed, .cancelled: self?.onFailure?()
            default: break
            }
        }
        connection.start(queue: queue)
    }

    func send(_ packet: BridgePacket) {
        guard let connection, let frame = try? framer.seal(packet) else { onFailure?(); return }
        connection.send(content: frame, completion: .contentProcessed { [weak self] error in
            if error != nil { self?.onFailure?() }
        })
    }

    func close() {
        connection?.cancel()
        connection = nil
    }
}
