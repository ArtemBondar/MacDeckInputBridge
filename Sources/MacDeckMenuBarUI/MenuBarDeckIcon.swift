import AppKit

/// A compact, template-safe Steam Deck status image. Its screen is the primary
/// state signal: empty for LOCAL, dots for CONNECTING, and `ABC` only when the
/// Steam Deck has captured the Mac keyboard in REMOTE mode.
public enum MenuBarDeckIcon {
    /// Empty screen, then one, two, and three dots.
    public static let connectingFrameCount = 4
    public static let scale: CGFloat = 0.8

    public static func image(for status: MenuBarStatus, connectingPhase: Int = 0) -> NSImage {
        // Keep the proportions of the 36 × 20 vector design while reducing its
        // menu-bar footprint by 20% to sit alongside standard status items.
        let size = NSSize(width: 36 * scale, height: 20 * scale)
        let image = NSImage(size: size, flipped: false) { _ in
            let transform = NSAffineTransform()
            transform.scale(by: scale)
            transform.concat()
            NSColor.white.setFill()
            NSColor.white.setStroke()

            let deck = NSBezierPath(roundedRect: NSRect(x: 1, y: 3, width: 34, height: 14), xRadius: 5.2, yRadius: 5.2)
            deck.lineWidth = 1.25
            deck.stroke()

            // The screen takes priority. The stick centres are geometrically at
            // the midpoint of each remaining side region, not merely near it.
            let screen = NSRect(x: 8.7, y: 6.0, width: 18.6, height: 8.0)
            let screenPath = NSBezierPath(roundedRect: screen, xRadius: 0.8, yRadius: 0.8)
            screenPath.lineWidth = 0.85
            screenPath.stroke()
            switch status {
            case .local:
                break
            case .connecting:
                let visibleDotCount = connectingPhase % connectingFrameCount
                for x in [13.1, 17.3, 21.5].prefix(visibleDotCount) {
                    NSBezierPath(ovalIn: NSRect(x: x, y: 9.15, width: 1.7, height: 1.7)).fill()
                }
            case .remote:
                let label = "ABC" as NSString
                let attributes: [NSAttributedString.Key: Any] = [
                    .font: NSFont.monospacedSystemFont(ofSize: 7.0, weight: .bold),
                    .foregroundColor: NSColor.white
                ]
                let labelSize = label.size(withAttributes: attributes)
                label.draw(
                    at: NSPoint(x: screen.midX - labelSize.width / 2, y: screen.midY - labelSize.height / 2 + 0.2),
                    withAttributes: attributes
                )
            }

            // Unchanged stick size; only the geometry changes. Their centres
            // are halfway between the display and the rounded body's sides.
            NSBezierPath(ovalIn: NSRect(x: 3.1, y: 8.25, width: 3.5, height: 3.5)).fill()
            NSBezierPath(ovalIn: NSRect(x: 29.4, y: 8.25, width: 3.5, height: 3.5)).fill()
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
