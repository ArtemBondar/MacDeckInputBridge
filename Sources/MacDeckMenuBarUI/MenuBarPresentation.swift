import Foundation

/// The complete visible action state of the menu-bar control. Keeping it pure
/// makes every UI state testable without a running event tap or network peer.
public enum MenuBarStatus: Equatable, Sendable {
    case local
    case connecting
    case remote
}

public enum MenuBarPrimaryAction: Equatable, Sendable {
    case configure
    case send
    case cancelConnection
    case returnToMac

    public var title: String {
        switch self {
        case .configure: return "Configure Steam Deck…"
        case .send: return "Send keyboard to Steam Deck"
        case .cancelConnection: return "Cancel connection"
        case .returnToMac: return "Return keyboard to Mac"
        }
    }
}

public enum MenuBarPresentation {
    public static func primaryAction(
        status: MenuBarStatus,
        host: String,
        pairingSecret: String
    ) -> MenuBarPrimaryAction {
        guard !host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !pairingSecret.isEmpty else {
            return .configure
        }
        switch status {
        case .local: return .send
        case .connecting: return .cancelConnection
        case .remote: return .returnToMac
        }
    }
}
