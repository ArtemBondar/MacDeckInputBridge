import ApplicationServices
import BridgeCore
import Foundation
import Network

private final class SenderClient {
    private let queue = DispatchQueue(label: "MacDeckInputBridge.transport")
    private let framer: SecureFramer
    private var connection: NWConnection?
    var onReady: (() -> Void)?
    var onFailure: (() -> Void)?

    init(pairingSecret: String) throws { framer = try SecureFramer(pairing: PairingSecret(base64: pairingSecret)) }

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

    func close() { connection?.cancel(); connection = nil }
}

private final class EventTapController {
    private var state = InputStateMachine()
    private let sender: SenderClient
    private let host: String
    private let port: UInt16
    private var source: CFRunLoopSource?

    init(sender: SenderClient, host: String, port: UInt16) {
        self.sender = sender; self.host = host; self.port = port
        sender.onReady = { [weak self] in
            DispatchQueue.main.async { self?.state.connectionReady() }
        }
        sender.onFailure = { [weak self] in
            DispatchQueue.main.async { self?.state.connectionLost() }
        }
    }

    func start() throws {
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        guard AXIsProcessTrustedWithOptions(options) else {
            throw SenderError.accessibilityPermissionRequired
        }
        let mask = (1 << CGEventType.keyDown.rawValue) | (1 << CGEventType.keyUp.rawValue) | (1 << CGEventType.flagsChanged.rawValue)
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap,
            eventsOfInterest: CGEventMask(mask), callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                return Unmanaged<EventTapController>.fromOpaque(context).takeUnretainedValue().handle(type: type, event: event)
            }, userInfo: context
        ) else { throw SenderError.eventTapUnavailable }
        source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
    }

    private func handle(type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        guard type == .keyDown || type == .keyUp || type == .flagsChanged else {
            if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
                state.connectionLost()
                sender.close()
            }
            return Unmanaged.passUnretained(event)
        }
        let rawFlags = event.flags
        var modifiers: ModifierState = []
        if rawFlags.contains(.maskControl) { modifiers.insert(.control) }
        if rawFlags.contains(.maskShift) { modifiers.insert(.shift) }
        if rawFlags.contains(.maskCommand) { modifiers.insert(.command) }
        if rawFlags.contains(.maskAlternate) { modifiers.insert(.option) }
        if rawFlags.contains(.maskAlphaShift) { modifiers.insert(.capsLock) }
        let captured = CapturedKeyEvent(
            kind: type == .keyDown ? .keyDown : type == .keyUp ? .keyUp : .flagsChanged,
            keyCode: UInt16(event.getIntegerValueField(.keyboardEventKeycode)),
            modifiers: modifiers,
            isRepeat: event.getIntegerValueField(.keyboardEventAutorepeat) != 0
        )
        let actions = state.handle(captured)
        var suppress = false
        for action in actions {
            switch action {
            case .passThrough: break
            case let .suppress(packet): sender.send(packet); suppress = true
            case .beginConnection: sender.connect(host: host, port: port)
            case .releaseAndDisconnect: sender.send(.releaseAll); sender.close()
            }
        }
        return suppress ? nil : Unmanaged.passUnretained(event)
    }
}

private enum SenderError: LocalizedError {
    case usage, accessibilityPermissionRequired, eventTapUnavailable, invalidPort
    var errorDescription: String? {
        switch self {
        case .usage: return "Usage: mac-deck-sender --host <Steam Deck IP> --pairing-secret <base64-32-byte-secret> [--port 4242]"
        case .accessibilityPermissionRequired: return "Accessibility permission is required. Enable this terminal/app under System Settings > Privacy & Security > Accessibility, then relaunch."
        case .eventTapUnavailable: return "Could not create the active keyboard event tap."
        case .invalidPort: return "Port must be between 1 and 65535."
        }
    }
}

private func argument(_ name: String, from arguments: [String]) -> String? {
    guard let index = arguments.firstIndex(of: name), arguments.indices.contains(index + 1) else { return nil }
    return arguments[index + 1]
}

do {
    let arguments = Array(CommandLine.arguments.dropFirst())
    guard let host = argument("--host", from: arguments), let secret = argument("--pairing-secret", from: arguments) else { throw SenderError.usage }
    let port = UInt16(argument("--port", from: arguments) ?? "4242") ?? 0
    guard port > 0 else { throw SenderError.invalidPort }
    let client = try SenderClient(pairingSecret: secret)
    let controller = EventTapController(sender: client, host: host, port: port)
    try controller.start()
    print("MacDeckInputBridge is LOCAL. Press Control + Shift + F12 to connect; press it again to return LOCAL.")
    CFRunLoopRun()
} catch {
    fputs("mac-deck-sender: \(error.localizedDescription)\n", stderr)
    exit(2)
}
