import Foundation
import Security

public struct BridgeSettings: Equatable, Sendable {
    public var host: String
    public var port: UInt16

    public init(host: String, port: UInt16 = 4242) {
        self.host = host.trimmingCharacters(in: .whitespacesAndNewlines)
        self.port = port
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

    public var errorDescription: String? {
        switch self {
        case .missingHost: return "Enter the Steam Deck host name or IP address."
        case .invalidPort: return "Port must be between 1 and 65535."
        case .invalidPairingSecret: return "The pairing secret must be Base64-encoded 32 bytes."
        case let .keychain(status): return "Could not update the macOS Keychain (status \(status))."
        }
    }
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

public final class BridgeSettingsStore {
    private enum Key { static let host = "deckHost"; static let port = "deckPort" }
    private let defaults: UserDefaults
    private let secretStore: PairingSecretStoring

    public init(defaults: UserDefaults = .standard, secretStore: PairingSecretStoring = KeychainPairingSecretStore()) {
        self.defaults = defaults
        self.secretStore = secretStore
    }

    public func loadSettings() -> BridgeSettings {
        let port = defaults.integer(forKey: Key.port)
        return BridgeSettings(host: defaults.string(forKey: Key.host) ?? "", port: UInt16(port == 0 ? 4242 : port))
    }

    public func saveSettings(_ settings: BridgeSettings) throws {
        let validated = try settings.validated()
        defaults.set(validated.host, forKey: Key.host)
        defaults.set(Int(validated.port), forKey: Key.port)
    }

    public func loadPairingSecret() throws -> String? { try secretStore.readSecret() }
    public func savePairingSecret(_ secret: String) throws { try secretStore.saveSecret(secret) }
    public func deletePairingSecret() throws { try secretStore.deleteSecret() }
}
