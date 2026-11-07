import Foundation

public struct ModifierState: OptionSet, Sendable {
    public let rawValue: UInt
    public init(rawValue: UInt) { self.rawValue = rawValue }
    public static let control = ModifierState(rawValue: 1 << 0)
    public static let shift = ModifierState(rawValue: 1 << 1)
    public static let command = ModifierState(rawValue: 1 << 2)
    public static let option = ModifierState(rawValue: 1 << 3)
    public static let capsLock = ModifierState(rawValue: 1 << 4)
}

public enum CapturedEventKind: Equatable, Sendable { case keyDown, keyUp, flagsChanged }

public struct CapturedKeyEvent: Sendable {
    public let kind: CapturedEventKind
    public let keyCode: UInt16
    public let modifiers: ModifierState
    public let isRepeat: Bool
    public init(kind: CapturedEventKind, keyCode: UInt16, modifiers: ModifierState = [], isRepeat: Bool = false) {
        self.kind = kind; self.keyCode = keyCode; self.modifiers = modifiers; self.isRepeat = isRepeat
    }
}

public enum BridgeMode: Equatable, Sendable { case local, connecting, remote }
public enum InputAction: Equatable, Sendable { case passThrough, suppress(BridgePacket), beginConnection, releaseAndDisconnect }

/// Policy-only state machine. The event-tap wrapper performs the requested I/O.
public struct InputStateMachine: Sendable {
    public private(set) var mode: BridgeMode = .local
    private var localOnlyKeys: Set<UInt16> = []
    private let f12: UInt16 = 111

    public init() {}

    public mutating func connectionReady() { if mode == .connecting { mode = .remote } }
    public mutating func connectionLost() { mode = .local; localOnlyKeys.removeAll() }

    public mutating func handle(_ event: CapturedKeyEvent) -> [InputAction] {
        // Modifiers held before REMOTE is entered were delivered locally. Their later
        // key-up must also stay local, even if the TCP connection becomes ready first.
        if event.kind == .flagsChanged, mode != .remote, isModifier(event.keyCode) {
            if modifierIsDown(event.keyCode, modifiers: event.modifiers) {
                localOnlyKeys.insert(event.keyCode)
            } else {
                localOnlyKeys.remove(event.keyCode)
            }
        }
        if isHotkey(event) {
            localOnlyKeys.insert(event.keyCode)
            switch mode {
            case .local:
                mode = .connecting
                return [.passThrough, .beginConnection]
            case .connecting:
                return [.passThrough]
            case .remote:
                mode = .local
                return [.passThrough, .releaseAndDisconnect]
            }
        }

        if localOnlyKeys.contains(event.keyCode) {
            if event.kind == .keyUp ||
                (event.kind == .flagsChanged && !modifierIsDown(event.keyCode, modifiers: event.modifiers)) {
                localOnlyKeys.remove(event.keyCode)
            }
            return [.passThrough]
        }
        guard mode == .remote, let code = KeyMapping.linuxCode(for: event.keyCode) else { return [.passThrough] }
        switch event.kind {
        case .keyDown:
            return [.suppress(.key(code: code, value: event.isRepeat ? 2 : 1))]
        case .keyUp:
            return [.suppress(.key(code: code, value: 0))]
        case .flagsChanged:
            // Quartz carries the modifier's new state in flagsChanged.
            let value: UInt8 = modifierIsDown(event.keyCode, modifiers: event.modifiers) ? 1 : 0
            return [.suppress(.key(code: code, value: value))]
        }
    }

    private func isHotkey(_ event: CapturedKeyEvent) -> Bool {
        event.kind == .keyDown && !event.isRepeat && event.keyCode == f12 &&
            event.modifiers.contains(.control) && event.modifiers.contains(.shift)
    }

    private func modifierIsDown(_ keyCode: UInt16, modifiers: ModifierState) -> Bool {
        switch keyCode {
        case 55: return modifiers.contains(.command)
        case 59, 62: return modifiers.contains(.control)
        case 56, 60: return modifiers.contains(.shift)
        case 57: return modifiers.contains(.capsLock)
        case 58, 61: return modifiers.contains(.option)
        default: return false
        }
    }

    private func isModifier(_ keyCode: UInt16) -> Bool {
        switch keyCode {
        case 55...62: return true
        default: return false
        }
    }
}
