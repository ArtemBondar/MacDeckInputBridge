import AppKit

/// Solid Steam Deck-inspired artwork. Local and connecting images are
/// templates, so macOS supplies the menu bar foreground.
public enum MenuBarDeckIcon {
    public static let connectingFrameCount = 42
    public static let connectingFrameInterval: TimeInterval = 1.4 / Double(connectingFrameCount)

    public static func image(
        for status: MenuBarStatus,
        connectingPhase: Int = 0,
        isDark: Bool = false,
        style: MenuBarIconStyle = .logo
    ) -> NSImage {
        if style == .classic { return ClassicDeckIcon.image(for: status, connectingPhase: connectingPhase) }
        let size = NSSize(width: 22, height: 22)
        let image = NSImage(size: size, flipped: false) { _ in
            NSGraphicsContext.saveGraphicsState()
            let scale = NSAffineTransform()
            scale.scale(by: 1.1)
            scale.concat()
            defer { NSGraphicsContext.restoreGraphicsState() }
            let ink: NSColor = status == .remote && !isDark ? .black : .white
            ink.setStroke()
            ink.setFill()

            // Coordinates reproduce the 18 × 18 SVGs, inset by one point.
            let center = NSPoint(x: 7.7, y: 10)
            let outer: CGFloat = 7.8
            let inner: CGFloat = 4.6
            let ring = NSBezierPath()
            ring.move(to: NSPoint(x: center.x, y: center.y - outer))
            ring.appendArc(withCenter: center, radius: outer, startAngle: -90, endAngle: 90)
            ring.line(to: NSPoint(x: center.x, y: center.y + inner))
            ring.appendArc(withCenter: center, radius: inner, startAngle: 90, endAngle: -90, clockwise: true)
            ring.close()
            ring.fill()

            let radius: CGFloat = 3.2
            let circle = NSBezierPath(ovalIn: NSRect(
                x: center.x - radius, y: center.y - radius,
                width: radius * 2, height: radius * 2
            ))
            switch status {
            case .local:
                circle.fill()
            case .connecting:
                let phase = ((connectingPhase % connectingFrameCount) + connectingFrameCount) % connectingFrameCount
                // Pulse the filled center while retaining the same solid logo.
                let opacity = 0.25 + 0.75 * (0.5 - 0.5 * cos(2 * .pi * Double(phase) / Double(connectingFrameCount)))
                NSColor.white.withAlphaComponent(opacity).setFill()
                circle.fill()
            case .remote:
                NSColor(calibratedRed: 26 / 255, green: 159 / 255, blue: 1, alpha: 1).setFill()
                circle.fill()
            }
            return true
        }
        image.isTemplate = status != .remote
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


/// A compact, template-safe Steam Deck status image. Its screen is the primary
/// state signal: empty for LOCAL, dots for CONNECTING, and `ABC` only when the
/// Steam Deck has captured the Mac keyboard in REMOTE mode.
enum ClassicDeckIcon {
    /// Empty screen, then one, two, and three dots.
    public static let connectingFrameCount = 4

    public static func image(for status: MenuBarStatus, connectingPhase: Int = 0) -> NSImage {
        // A compact 30 × 20 layout retains the original height and reflows the
        // controls rather than squeezing a wider silhouette horizontally.
        let size = NSSize(width: 30, height: 20)
        let image = NSImage(size: size, flipped: false) { _ in
            NSColor.white.setFill()
            NSColor.white.setStroke()

            let deck = NSBezierPath(roundedRect: NSRect(x: 1, y: 3, width: 28, height: 14), xRadius: 5.2, yRadius: 5.2)
            deck.lineWidth = 1.25
            deck.stroke()

            // The screen takes priority. The stick centres are geometrically at
            // the midpoint of each remaining side region, not merely near it.
            let screen = NSRect(x: 7.4, y: 6.0, width: 15.2, height: 8.0)
            let screenPath = NSBezierPath(roundedRect: screen, xRadius: 0.8, yRadius: 0.8)
            screenPath.lineWidth = 0.85
            screenPath.stroke()
            switch status {
            case .local:
                break
            case .connecting:
                let visibleDotCount = (connectingPhase / 10) % connectingFrameCount
                for x in [10.8, 14.15, 17.5].prefix(visibleDotCount) {
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
            NSBezierPath(ovalIn: NSRect(x: 2.76, y: 8.25, width: 3.5, height: 3.5)).fill()
            NSBezierPath(ovalIn: NSRect(x: 23.74, y: 8.25, width: 3.5, height: 3.5)).fill()
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
