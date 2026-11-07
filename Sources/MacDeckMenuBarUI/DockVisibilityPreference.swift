import AppKit

/// The Dock is optional for this menu-bar utility. The preference is intentionally
/// separate from connection settings and secrets, which are stored elsewhere.
public enum DockVisibilityPreference {
    public static let userDefaultsKey = "showDockIcon"

    public static func load(from defaults: UserDefaults = .standard) -> Bool {
        guard defaults.object(forKey: userDefaultsKey) != nil else { return true }
        return defaults.bool(forKey: userDefaultsKey)
    }

    public static func save(showInDock: Bool, to defaults: UserDefaults = .standard) {
        defaults.set(showInDock, forKey: userDefaultsKey)
    }

    @discardableResult
    public static func apply(showInDock: Bool) -> Bool {
        NSApp.setActivationPolicy(showInDock ? .regular : .accessory)
    }
}
