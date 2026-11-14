import AppKit

/// Solid Steam Deck-inspired artwork. Local and connecting images are
/// templates, so macOS supplies the menu bar foreground.
public enum MenuBarDeckIcon {
    public static let connectingFrameCount = 42
    public static let connectingFrameInterval: TimeInterval = 1.4 / Double(connectingFrameCount)

    public static func image(
        for status: MenuBarStatus,
        connectingPhase: Int = 0,
        isDark: Bool = false
    ) -> NSImage {
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
