import AppKit

/// Status artwork based on the supplied Steam Deck logo SVGs. Local and
/// connecting images are templates, so macOS supplies the menu bar foreground.
public enum MenuBarDeckIcon {
    public static let connectingFrameCount = 4

    public static func image(
        for status: MenuBarStatus,
        connectingPhase: Int = 0
    ) -> NSImage {
        let size = NSSize(width: 20, height: 20)
        let image = NSImage(size: size, flipped: false) { _ in
            NSColor.white.setStroke()
            NSColor.white.setFill()

            // Coordinates reproduce the 18 × 18 SVGs, inset by one point.
            let center = NSPoint(x: 7.7, y: 10)
            let outline = status != .remote
            let outer: CGFloat = outline ? 7.3 : 7.8
            let inner: CGFloat = outline ? 5.1 : 4.6
            let ring = NSBezierPath()
            ring.move(to: NSPoint(x: center.x, y: center.y - outer))
            ring.appendArc(withCenter: center, radius: outer, startAngle: -90, endAngle: 90)
            ring.line(to: NSPoint(x: center.x, y: center.y + inner))
            ring.appendArc(withCenter: center, radius: inner, startAngle: 90, endAngle: -90, clockwise: true)
            ring.close()
            if outline {
                ring.lineWidth = 1
                ring.stroke()
            } else {
                ring.fill()
            }

            let radius: CGFloat = outline ? 2.7 : 3.2
            let circle = NSBezierPath(ovalIn: NSRect(
                x: center.x - radius, y: center.y - radius,
                width: radius * 2, height: radius * 2
            ))
            if outline {
                circle.lineWidth = 1
                circle.stroke()
                // The dot grows and shrinks independently of the outline, so
                // connecting never resembles the fully filled remote state.
                if status == .connecting {
                    let phase = ((connectingPhase % connectingFrameCount) + connectingFrameCount) % connectingFrameCount
                    let radii: [CGFloat] = [0.0, 0.8, 1.5, 0.8]
                    let dotRadius = radii[phase]
                    if dotRadius > 0 {
                        NSBezierPath(ovalIn: NSRect(
                            x: center.x - dotRadius, y: center.y - dotRadius,
                            width: dotRadius * 2, height: dotRadius * 2
                        )).fill()
                    }
                }
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "Steam Deck keyboard input, \(statusDescription(status))"
        return image
    }

    /// A separate, non-template layer preserves the blue accent while macOS
    /// tints the surrounding logo for the actual menu bar appearance.
    public static func remoteAccent() -> NSImage {
        let image = NSImage(size: NSSize(width: 20, height: 20), flipped: false) { _ in
            NSColor(calibratedRed: 26 / 255, green: 159 / 255, blue: 1, alpha: 1).setFill()
            NSBezierPath(ovalIn: NSRect(x: 4.5, y: 6.8, width: 6.4, height: 6.4)).fill()
            return true
        }
        image.isTemplate = false
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
