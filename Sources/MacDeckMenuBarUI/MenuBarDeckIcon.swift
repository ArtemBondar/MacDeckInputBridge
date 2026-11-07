import AppKit

/// A small, template-safe status image: Steam Deck controls above the letters
/// “ABC”, which communicate that this status item controls keyboard input.
public enum MenuBarDeckIcon {
    public static func image(for status: MenuBarStatus) -> NSImage {
        let size = NSSize(width: 29, height: 21)
        let image = NSImage(size: size, flipped: false) { rect in
            NSColor.white.setFill()

            let deck = NSRect(x: 2, y: 7, width: 25, height: 10)
            NSBezierPath(roundedRect: deck, xRadius: 4, yRadius: 4).fill()

            // Cut out the central display and controls using transparency. As a
            // template image, macOS supplies the final foreground color.
            let graphicsContext = NSGraphicsContext.current
            let previousOperation = graphicsContext?.compositingOperation
            graphicsContext?.compositingOperation = .clear
            NSBezierPath(roundedRect: NSRect(x: 10, y: 9, width: 9, height: 5), xRadius: 1, yRadius: 1).fill()
            NSBezierPath(ovalIn: NSRect(x: 5, y: 10, width: 3, height: 3)).fill()
            NSBezierPath(ovalIn: NSRect(x: 21, y: 10, width: 3, height: 3)).fill()
            graphicsContext?.compositingOperation = previousOperation ?? .sourceOver

            let statusDot = NSRect(x: 23.5, y: 15.5, width: 2.5, height: 2.5)
            switch status {
            case .local:
                NSBezierPath(ovalIn: statusDot).stroke()
            case .connecting:
                NSBezierPath(ovalIn: statusDot).fill()
            case .remote:
                NSBezierPath(ovalIn: statusDot.insetBy(dx: -0.75, dy: -0.75)).fill()
            }

            let label = "ABC" as NSString
            let attributes: [NSAttributedString.Key: Any] = [
                .font: NSFont.monospacedSystemFont(ofSize: 5.5, weight: .semibold),
                .foregroundColor: NSColor.white
            ]
            let labelSize = label.size(withAttributes: attributes)
            label.draw(at: NSPoint(x: (size.width - labelSize.width) / 2, y: 0), withAttributes: attributes)
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