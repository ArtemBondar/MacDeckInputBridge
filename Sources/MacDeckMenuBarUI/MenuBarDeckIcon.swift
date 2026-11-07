import AppKit

/// A compact, template-safe Steam Deck status image. Its screen is the primary
/// state signal: empty for LOCAL, dots for CONNECTING, and `ABC` only when the
/// Steam Deck has captured the Mac keyboard in REMOTE mode.
public enum MenuBarDeckIcon {
    /// Empty screen, then one, two, and three dots.
    public static let connectingFrameCount = 4

    public static func image(for status: MenuBarStatus, connectingPhase: Int = 0) -> NSImage {
        let size = NSSize(width: 32, height: 20)
        let image = NSImage(size: size, flipped: false) { _ in
            NSColor.white.setFill()
            NSColor.white.setStroke()

            let deck = NSBezierPath(roundedRect: NSRect(x: 1, y: 3, width: 30, height: 14), xRadius: 5.2, yRadius: 5.2)
            deck.lineWidth = 1.25
            deck.stroke()

            let screen = NSRect(x: 8.8, y: 6.4, width: 14.4, height: 7.2)
            let screenPath = NSBezierPath(roundedRect: screen, xRadius: 0.8, yRadius: 0.8)
            screenPath.lineWidth = 0.85
            screenPath.stroke()
            switch status {
            case .local:
                break
            case .connecting:
                let visibleDotCount = connectingPhase % connectingFrameCount
                for x in [11.2, 15.0, 18.8].prefix(visibleDotCount) {
                    NSBezierPath(ovalIn: NSRect(x: x, y: 9.4, width: 1.7, height: 1.7)).fill()
                }
            case .remote:
                let label = "ABC" as NSString
                let attributes: [NSAttributedString.Key: Any] = [
                    .font: NSFont.monospacedSystemFont(ofSize: 6.5, weight: .bold),
                    .foregroundColor: NSColor.white
                ]
                let labelSize = label.size(withAttributes: attributes)
                label.draw(
                    at: NSPoint(x: screen.midX - labelSize.width / 2, y: screen.midY - labelSize.height / 2 + 0.2),
                    withAttributes: attributes
                )
            }

            // The sticks sit halfway between the display and the outside of the
            // thinner body, centered on the display's vertical axis.
            NSBezierPath(ovalIn: NSRect(x: 4.9, y: 8.25, width: 3.5, height: 3.5)).fill()
            NSBezierPath(ovalIn: NSRect(x: 23.6, y: 8.25, width: 3.5, height: 3.5)).fill()
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "Steam Deck keyboard input, \(statusDescription(status))"
        return image
    }

    private static func statusDescription(_ status: MenuBarStatus) -> String {
        switch status {
        case .local: return "local"
        case .connecting: return "connecting"
        case .remote: return "sending to Steam Deck"
        }
    }
}
