/// Tracks local and remote button ownership across a forwarding boundary.
/// Buttons held when forwarding starts remain local until their release.
struct PointerButtonState {
    enum Action: Equatable {
        case pass
        case send(button: UInt8, pressed: Bool)
        case suppress
    }

    private var physical: Set<UInt8> = []
    private var localOwnership: Set<UInt8> = []
    private var remoteOwnership: Set<UInt8> = []
    private var suppressUntilUp: Set<UInt8> = []
    private(set) var forwarding = false

    /// Reconcile events missed while the event tap was not installed.
    mutating func reconcile(pressed: Set<UInt8>) {
        physical = pressed
        localOwnership.formIntersection(pressed)
        suppressUntilUp.formIntersection(pressed)
        remoteOwnership.formIntersection(pressed)
    }

    mutating func start() {
        guard !forwarding else { return }
        localOwnership = physical.subtracting(suppressUntilUp)
        forwarding = true
    }

    /// Returns buttons that need releasing on the receiver. Their physical
    /// releases must still be suppressed locally, preventing orphaned ups.
    mutating func stop() -> [UInt8] {
        guard forwarding else { return [] }
        forwarding = false
        let releases = remoteOwnership.sorted()
        suppressUntilUp.formUnion(remoteOwnership)
        remoteOwnership.removeAll()
        return releases
    }

    mutating func handle(button: UInt8, pressed: Bool) -> Action {
        guard button <= 1 else { return .pass }
        if pressed { physical.insert(button) } else { physical.remove(button) }
        if suppressUntilUp.contains(button) {
            if !pressed { suppressUntilUp.remove(button) }
            return .suppress
        }
        if localOwnership.contains(button) {
            if !pressed { localOwnership.remove(button) }
            return .pass
        }
        if remoteOwnership.contains(button) {
            if !pressed { remoteOwnership.remove(button) }
            return forwarding ? .send(button: button, pressed: pressed) : .suppress
        }
        guard forwarding else { return .pass }
        guard pressed else { return .suppress }
        remoteOwnership.insert(button)
        return .send(button: button, pressed: true)
    }
}

/// Coalesces consecutive events without reordering motion and scrolling.
struct PointerEventBuffer {
    private enum Kind { case motion, scroll }
    private var kind: Kind?
    private var x = 0
    private var y = 0
    private var sentSinceIdle = false

    mutating func appendMotion(x: Int, y: Int) -> BridgePacket? { append(.motion, x: x, y: y) }
    mutating func appendScroll(x: Int, y: Int) -> BridgePacket? { append(.scroll, x: x, y: y) }

    private mutating func append(_ next: Kind, x: Int, y: Int) -> BridgePacket? {
        let limitedX = max(Int(Int16.min), min(Int(Int16.max), x))
        let limitedY = max(Int(Int16.min), min(Int(Int16.max), y))
        let range = Int(Int16.min)...Int(Int16.max)
        // Flush before an accumulated axis would exceed the wire format,
        // preserving both packets instead of silently clipping fast movement.
        let overflows = !range.contains(self.x + limitedX) || !range.contains(self.y + limitedY)
        let prior = kind != nil && (kind != next || overflows) ? drain() : nil
        kind = next
        self.x += limitedX
        self.y += limitedY
        return prior
    }

    /// An idle stream need not wait for the next timer tick.
    mutating func drainIfIdle() -> BridgePacket? {
        guard !sentSinceIdle, let packet = drain() else { return nil }
        sentSinceIdle = true
        return packet
    }

    /// During a burst, at most one coalesced packet is sent per timer tick.
    mutating func drainForTick() -> BridgePacket? {
        let packet = drain()
        sentSinceIdle = packet != nil
        return packet
    }

    mutating func drain() -> BridgePacket? {
        defer { kind = nil; x = 0; y = 0 }
        guard let kind, x != 0 || y != 0 else { return nil }
        switch kind {
        case .motion: return .pointerMotion(dx: Int16(x), dy: Int16(y))
        case .scroll: return .pointerScroll(horizontal: Int16(x), vertical: Int16(y))
        }
    }

    /// A keyboard transition must not overtake a queued pointer event.
    mutating func drain(before packet: BridgePacket) -> [BridgePacket] {
        if let pending = drain() { return [pending, packet] }
        return [packet]
    }
}

/// Converts precise fractional wheel deltas into the receiver's whole steps.
struct ScrollDeltaRemainder {
    private var x = 0.0
    private var y = 0.0

    mutating func append(x: Double, y: Double) -> (Int, Int) {
        func step(_ previous: inout Double, _ delta: Double) -> Int {
            guard delta.isFinite else { return 0 }
            previous = max(Double(Int16.min), min(Double(Int16.max), previous + delta))
            let whole = Int(previous)
            previous -= Double(whole)
            return whole
        }
        return (step(&self.x, x), step(&self.y, y))
    }
}
