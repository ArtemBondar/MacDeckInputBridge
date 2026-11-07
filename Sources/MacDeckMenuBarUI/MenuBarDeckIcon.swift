import AppKit

/// A compact, template-safe Steam Deck status image. Its screen is the primary
/// state signal: empty for LOCAL, dots for CONNECTING, and `ABC` only when the
/// Steam Deck has captured the Mac keyboard in REMOTE mode.
public enum MenuBarDeckIcon {
    public static func image(for status: MenuBarStatus) -> NSImage {
        let size = NSSize(width: 32, height: 20)
        let image = NSImage(size: size, flipped: false) { _ in
            NSColor.white.setFill()
            NSColor.white.setStroke()

            let deck = NSBezierPath(roundedRect: NSRect(x: 1, y: 2, width: 30, height: 16), xRadius: 6, yRadius: 6)
            deck.lineWidth = 1.8
            deck.stroke()

            let screen = NSRect(x: 10, y: 7, width: 12, height: 6)
            let screenPath = NSBezierPath(roundedRect: screen, xRadius: 0.8, yRadius: 0.8)
            screenPath.lineWidth = 1.3
            screenPath.stroke()
            switch status {
            case .local:
                break
            case .connecting:
                for x in [12.2, 15.1, 18.0] {
                    NSBezierPath(ovalIn: NSRect(x: x, y: 9.45, width: 1.4, height: 1.4)).fill()
                }
            case .remote:
                let label = "ABC" as NSString
                let attributes: [NSAttributedString.Key: Any] = [
                    .font: NSFont.monospacedSystemFont(ofSize: 5.6, weight: .bold),
                    .foregroundColor: NSColor.white
                ]
                let labelSize = label.size(withAttributes: attributes)
                label.draw(
                    at: NSPoint(x: screen.midX - labelSize.width / 2, y: screen.midY - labelSize.height / 2 + 0.2),
                    withAttributes: attributes
                )
            }

            // Two large controls keep the Steam Deck silhouette legible without
            // competing with the screen's connection status.
            NSBezierPath(ovalIn: NSRect(x: 4.2, y: 8.0, width: 3.5, height: 3.5)).fill()
            NSBezierPath(ovalIn: NSRect(x: 24.3, y: 8.0, width: 3.5, height: 3.5)).fill()
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
