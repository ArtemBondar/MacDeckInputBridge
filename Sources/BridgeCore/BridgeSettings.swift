import Foundation
import Security

public struct BridgeSettings: Equatable, Sendable {
    public var host: String
    public var port: UInt16
    /// Explicitly opt in to Mac → Steam Deck plain-text clipboard transfers.
    /// Clipboard contents themselves are never persisted.
    public var syncClipboardFromMac: Bool

    public init(host: String, port: UInt16 = 4242, syncClipboardFromMac: Bool = false) {
        self.host = host.trimmingCharacters(in: .whitespacesAndNewlines)
        self.port = port
        self.syncClipboardFromMac = syncClipboardFromMac
    }

    public func validated() throws -> BridgeSettings {
        guard !host.isEmpty else { throw BridgeSettingsError.missingHost }
        guard port > 0 else { throw BridgeSettingsError.invalidPort }
        return self
    }
}

public enum BridgeSettingsError: LocalizedError, Equatable {
    case missingHost
    case invalidPort
    case invalidPairingSecret
    case keychain(OSStatus)
    case localSecretStorage

    public var errorDescription: String? {
        switch self {
        case .missingHost: return "Enter the Steam Deck host name or IP address."
        case .invalidPort: return "Port must be between 1 and 65535."
        case .invalidPairingSecret: return "The pairing secret must be Base64-encoded 32 bytes."
        case let .keychain(status): return "Could not update the macOS Keychain (status \(status))."
        case .localSecretStorage: return "Could not securely store the pairing secret in this Mac’s private app-data folder."
        }
    }
}

public enum PairingSecretPersistence: Equatable, Sendable {
    case keychain
    case localPrivateFile
    case memoryOnly
}

public protocol PairingSecretStoring: AnyObject {
    func readSecret() throws -> String?
    func saveSecret(_ secret: String) throws
    func deleteSecret() throws
}

public final class KeychainPairingSecretStore: PairingSecretStoring {
    private let service = "com.artembondar.MacDeckInputBridge"
    private let account = "pairing-secret"

    public init() {}

    public func readSecret() throws -> String? {
        var query = baseQuery
        query[kSecMatchLimit] = kSecMatchLimitOne
        query[kSecReturnData] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data, let secret = String(data: data, encoding: .utf8) else {
            throw BridgeSettingsError.keychain(status)
        }
        return secret
    }

    public func saveSecret(_ secret: String) throws {
        guard (try? PairingSecret(base64: secret)) != nil else { throw BridgeSettingsError.invalidPairingSecret }
        let data = Data(secret.utf8)
        let status = SecItemUpdate(baseQuery as CFDictionary, [kSecValueData: data] as CFDictionary)
        if status == errSecItemNotFound {
            var addQuery = baseQuery
            addQuery[kSecValueData] = data
            let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
            guard addStatus == errSecSuccess else { throw BridgeSettingsError.keychain(addStatus) }
        } else if status != errSecSuccess {
            throw BridgeSettingsError.keychain(status)
        }
    }

    public func deleteSecret() throws {
        let status = SecItemDelete(baseQuery as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw BridgeSettingsError.keychain(status) }
    }

    private var baseQuery: [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]
    }
}

public final class InMemoryPairingSecretStore: PairingSecretStoring {
    private var secret: String?
    public init(secret: String? = nil) { self.secret = secret }
    public func readSecret() throws -> String? { secret }
    public func saveSecret(_ secret: String) throws {
        guard (try? PairingSecret(base64: secret)) != nil else { throw BridgeSettingsError.invalidPairingSecret }
        self.secret = secret
    }
    public func deleteSecret() throws { secret = nil }
}

/// An explicitly opt-in fallback for a Keychain the user has denied or that is
/// unavailable. The secret is never written to UserDefaults: this store uses an
/// owner-only file in Application Support instead.
public final class LocalPairingSecretStore: PairingSecretStoring {
    private let fileURL: URL
    private let fileManager: FileManager

    public init(fileURL: URL? = nil, fileManager: FileManager = .default) {
        self.fileManager = fileManager
        self.fileURL = fileURL ?? Self.defaultFileURL(fileManager: fileManager)
    }

    public func readSecret() throws -> String? {
        guard fileManager.fileExists(atPath: fileURL.path) else { return nil }
        do {
            let values = try fileURL.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
            guard values.isRegularFile == true, values.isSymbolicLink != true else {
                throw BridgeSettingsError.localSecretStorage
            }
            let data = try Data(contentsOf: fileURL)
            guard let secret = String(data: data, encoding: .utf8),
                  (try? PairingSecret(base64: secret)) != nil else {
                throw BridgeSettingsError.localSecretStorage
            }
            return secret
        } catch {
            throw BridgeSettingsError.localSecretStorage
        }
    }

    public func saveSecret(_ secret: String) throws {
        guard (try? PairingSecret(base64: secret)) != nil else { throw BridgeSettingsError.invalidPairingSecret }
        let directory = fileURL.deletingLastPathComponent()
        do {
            try fileManager.createDirectory(
                at: directory,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
            let directoryValues = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
            guard directoryValues.isDirectory == true, directoryValues.isSymbolicLink != true else {
                throw BridgeSettingsError.localSecretStorage
            }
            try fileManager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
            if fileManager.fileExists(atPath: fileURL.path) {
                let fileValues = try fileURL.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
                guard fileValues.isRegularFile == true, fileValues.isSymbolicLink != true else {
                    throw BridgeSettingsError.localSecretStorage
                }
            }
            try Data(secret.utf8).write(to: fileURL, options: .atomic)
            try fileManager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: fileURL.path)
        } catch {
            throw BridgeSettingsError.localSecretStorage
        }
    }

    public func deleteSecret() throws {
        guard fileManager.fileExists(atPath: fileURL.path) else { return }
        do {
            try fileManager.removeItem(at: fileURL)
        } catch {
            throw BridgeSettingsError.localSecretStorage
        }
    }

    private static func defaultFileURL(fileManager: FileManager) -> URL {
        let applicationSupport = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return applicationSupport
            .appendingPathComponent("MacDeckInputBridge", isDirectory: true)
            .appendingPathComponent("pairing-secret")
    }
}

public final class BridgeSettingsStore {
    private enum Key {
        static let host = "deckHost"
        static let port = "deckPort"
        static let syncClipboardFromMac = "syncClipboardFromMac"
        static let allowLocalSecretFallback = "allowLocalSecretFallback"
    }
    private let defaults: UserDefaults
    private let secretStore: PairingSecretStoring
    private let localSecretStore: PairingSecretStoring

    public init(
        defaults: UserDefaults = .standard,
        secretStore: PairingSecretStoring = KeychainPairingSecretStore(),
        localSecretStore: PairingSecretStoring = LocalPairingSecretStore()
    ) {
        self.defaults = defaults
        self.secretStore = secretStore
        self.localSecretStore = localSecretStore
    }

    public func loadSettings() -> BridgeSettings {
        let storedPort = defaults.integer(forKey: Key.port)
        let port = (1...Int(UInt16.max)).contains(storedPort) ? UInt16(storedPort) : 4242
        return BridgeSettings(
            host: defaults.string(forKey: Key.host) ?? "",
            port: port,
            syncClipboardFromMac: defaults.bool(forKey: Key.syncClipboardFromMac)
        )
    }

    public func saveSettings(_ settings: BridgeSettings) throws {
        let validated = try settings.validated()
        defaults.set(validated.host, forKey: Key.host)
        defaults.set(Int(validated.port), forKey: Key.port)
        defaults.set(validated.syncClipboardFromMac, forKey: Key.syncClipboardFromMac)
    }

    public var allowsLocalSecretFallback: Bool {
        defaults.bool(forKey: Key.allowLocalSecretFallback)
    }

    public func setAllowsLocalSecretFallback(_ allowed: Bool) throws {
        if !allowed { try localSecretStore.deleteSecret() }
        defaults.set(allowed, forKey: Key.allowLocalSecretFallback)
    }

    public func loadPairingSecret() throws -> String? {
        do {
            if let secret = try secretStore.readSecret() { return secret }
        } catch {
            guard allowsLocalSecretFallback, isKeychainUnavailable(error) else { throw error }
            return try localSecretStore.readSecret()
        }
        return allowsLocalSecretFallback ? try localSecretStore.readSecret() : nil
    }

    @discardableResult
    public func savePairingSecret(_ secret: String) throws -> PairingSecretPersistence {
        guard (try? PairingSecret(base64: secret)) != nil else { throw BridgeSettingsError.invalidPairingSecret }
        do {
            try secretStore.saveSecret(secret)
            try? localSecretStore.deleteSecret()
            return .keychain
        } catch {
            guard isKeychainUnavailable(error) else { throw error }
            guard allowsLocalSecretFallback else { return .memoryOnly }
            try localSecretStore.saveSecret(secret)
            return .localPrivateFile
        }
    }

    public func deletePairingSecret() throws {
        try localSecretStore.deleteSecret()
        try secretStore.deleteSecret()
    }

    private func isKeychainUnavailable(_ error: Error) -> Bool {
        guard case let BridgeSettingsError.keychain(status) = error else { return false }
        return [errSecAuthFailed, errSecInteractionNotAllowed, errSecNotAvailable, errSecUserCanceled].contains(status)
    }
}
