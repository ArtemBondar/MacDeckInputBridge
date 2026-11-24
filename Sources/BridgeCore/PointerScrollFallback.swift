import AppKit
import CoreGraphics

/// Local AppKit monitors only see events delivered to this app. This is a
/// fallback for scrolls not filtered by the Quartz session tap, not a global
/// monitor and not a way to suppress system gestures.
enum PointerScrollFallback {
    static func route(_ event: NSEvent, forwarding: Bool, send: (CGEvent) -> Void) -> NSEvent? {
        guard forwarding, let cgEvent = event.cgEvent else { return event }
        send(cgEvent)
        return nil
    }
}
