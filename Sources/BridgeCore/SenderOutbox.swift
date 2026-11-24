import Foundation

/// A bounded handoff to the transport queue. Only adjacent, same-direction
/// unsent motion may be coalesced; other packets form ordering barriers.
final class SenderOutbox {
    enum EnqueueResult: Equatable {
        case inactive
        case queued
        case wake(UInt64)
        case overflow(UInt64)
    }

    private let lock = NSLock()
    private let capacity: Int
    private var generation: UInt64?
    private var packets: [BridgePacket] = []
    private var wakePending = false

    init(capacity: Int) { precondition(capacity > 0); self.capacity = capacity }

    func activate(generation: UInt64) {
        lock.lock(); defer { lock.unlock() }
        self.generation = generation
        packets.removeAll(keepingCapacity: true)
        wakePending = false
    }

    func deactivate() {
        lock.lock(); defer { lock.unlock() }
        generation = nil
        packets.removeAll(keepingCapacity: true)
        wakePending = false
    }

    func enqueue(_ packet: BridgePacket) -> EnqueueResult {
        lock.lock(); defer { lock.unlock() }
        guard let generation else { return .inactive }
        if case let .pointerMotion(dx, dy) = packet,
           let last = packets.last,
           case let .pointerMotion(px, py) = last,
           compatible(px, dx), compatible(py, dy),
           let x = Int16(exactly: Int(px) + Int(dx)),
           let y = Int16(exactly: Int(py) + Int(dy)) {
            packets[packets.count - 1] = .pointerMotion(dx: x, dy: y)
            return .queued
        }
        guard packets.count < capacity else {
            // Fail the session closed; never silently lose a key or button up.
            self.generation = nil
            packets.removeAll(keepingCapacity: true)
            wakePending = false
            return .overflow(generation)
        }
        packets.append(packet)
        guard !wakePending else { return .queued }
        wakePending = true
        return .wake(generation)
    }

    /// The wake stays reserved while the transport is busy, so producers
    /// cannot create one Dispatch closure per event during backpressure.
    func take(generation: UInt64) -> BridgePacket? {
        lock.lock(); defer { lock.unlock() }
        guard self.generation == generation else { return nil }
        guard !packets.isEmpty else { wakePending = false; return nil }
        return packets.removeFirst()
    }

    private func compatible(_ a: Int16, _ b: Int16) -> Bool {
        a == 0 || b == 0 || (a > 0) == (b > 0)
    }
}
