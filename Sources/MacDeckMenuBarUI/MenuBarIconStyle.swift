import Foundation

public enum MenuBarIconStyle: String, CaseIterable {
    case logo, classic

    public var title: String {
        switch self {
        case .logo: return "Steam Deck logo"
        case .classic: return "Steam Deck silhouette"
        }
    }

    public static func load(from defaults: UserDefaults = .standard) -> Self {
        defaults.string(forKey: "menuBarIconStyle").flatMap(Self.init(rawValue:)) ?? .logo
    }

    public func save(to defaults: UserDefaults = .standard) {
        defaults.set(rawValue, forKey: "menuBarIconStyle")
    }
}
