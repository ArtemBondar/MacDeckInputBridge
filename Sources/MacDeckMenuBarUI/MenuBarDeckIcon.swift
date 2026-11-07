import AppKit

/// A compact, template-safe Steam Deck status image. Its screen is the primary
/// state signal: empty for LOCAL, dots for CONNECTING, filled for REMOTE.
/// The `ABC` label keeps the input purpose readable at menu-bar scale.
public enum MenuBarDeckIcon {
    public static func image(for status: MenuBarStatus) -> NSImage {
        let size = NSSize(width: 27, height: 19)
        let image = NSImage(size: size, flipped: false) { _ in
            NSColor.white.setFill()
            NSColor.white.setStroke()

            let deck = NSBezierPath(roundedRect: NSRect(x: 1, y: 7, width: 25, height: 9), xRadius: 4, yRadius: 4)
            deck.lineWidth = 1.5
            deck.stroke()

            let screen = NSRect(x: 9, y: 10, width: 9, height: 3.5)
            let screenPath = NSBezierPath(roundedRect: screen, xRadius: 0.8, yRadius: 0.8)
            switch status {
            case .local:
                screenPath.lineWidth = 1.2
                screenPath.stroke()
            case .connecting:
                screenPath.lineWidth = 1.2
                screenPath.stroke()
                for x in [10.7, 13.0, 15.3] {
                    NSBezierPath(ovalIn: NSRect(x: x, y: 11.1, width: 1.1, height: 1.1)).fill()
                }
            case .remote:
                screenPath.fill()
            }

            // Minimal controls retain the Steam Deck silhouette without making
            // the status item busy at the approximately 18-point menu-bar size.
            NSBezierPath(ovalIn: NSRect(x: 4.1, y: 10.4, width: 2.1, height: 2.1)).fill()
            NSBezierPath(ovalIn: NSRect(x: 20.8, y: 10.4, width: 2.1, height: 2.1)).fill()

            let label = "ABC" as NSString
            let attributes: [NSAttributedString.Key: Any] = [
                .font: NSFont.monospacedSystemFont(ofSize: 6.5, weight: .bold),
                .foregroundColor: NSColor.white
            ]
            let labelSize = label.size(withAttributes: attributes)
            label.draw(at: NSPoint(x: (size.width - labelSize.width) / 2, y: 0.2), withAttributes: attributes)
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "Steam Deck keyboard input, ABC, \(statusDescription(status))"
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
