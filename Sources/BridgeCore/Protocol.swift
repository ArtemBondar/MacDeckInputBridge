import CryptoKit
import Foundation

public enum BridgePacket: Equatable, Sendable {
    case key(code: UInt16, value: UInt8)
    case releaseAll
    case ping
    case clientReady
    case serverReady
    /// The receiver has opted into layout synchronization for this session.
    case serverReadyWithLayoutSync
    case keyboardLayout(identifier: String)

    public func encoded() -> Data {
        switch self {
        case let .key(code, value): return Data([1, UInt8(code >> 8), UInt8(code & 0xff), value])
        case .releaseAll: return Data([2])
        case .ping: return Data([3])
        case .clientReady: return Data([4])
        case .serverReady: return Data([5])
        case .serverReadyWithLayoutSync: return Data([6])
        case let .keyboardLayout(identifier):
            let identifierBytes = Array(identifier.utf8)
            precondition((1...128).contains(identifierBytes.count))
            return Data([7, UInt8(identifierBytes.count)] + identifierBytes)
        }
    }

    public init?(encoded: Data) {
        let bytes = Array(encoded)
        if bytes.count == 4, bytes[0] == 1, bytes[3] <= 2 {
            self = .key(code: UInt16(bytes[1]) << 8 | UInt16(bytes[2]), value: bytes[3])
        } else if bytes == [2] { self = .releaseAll
        } else if bytes == [3] { self = .ping
        } else if bytes == [4] { self = .clientReady
        } else if bytes == [5] { self = .serverReady
        } else if bytes == [6] { self = .serverReadyWithLayoutSync
        } else if bytes.count >= 3, bytes[0] == 7, bytes[1] <= 128,
                  bytes.count == 2 + Int(bytes[1]),
                  let identifier = String(bytes: bytes.dropFirst(2), encoding: .utf8),
                  Self.isValidKeyboardLayoutIdentifier(identifier) {
            self = .keyboardLayout(identifier: identifier)
        } else { return nil }
    }

    private static func isValidKeyboardLayoutIdentifier(_ identifier: String) -> Bool {
        !identifier.isEmpty && identifier.utf8.allSatisfy { byte in
            (48...57).contains(byte) || (65...90).contains(byte) ||
                (97...122).contains(byte) || byte == 45 || byte == 46 || byte == 95
        }
    }
}

public enum ProtocolError: Error, Equatable {
    case invalidPairingSecret, invalidFrame, frameTooLarge, authenticationFailed
    case invalidServerHello, unexpectedSequence
}

public struct PairingSecret: Sendable {
    public let key: SymmetricKey
    public init(base64: String) throws {
        guard let data = Data(base64Encoded: base64), data.count == 32 else { throw ProtocolError.invalidPairingSecret }
        key = SymmetricKey(data: data)
    }
}

/// Unencrypted, length-framed server greeting. The fresh challenge derives a key
/// unique to this TCP session; it is not itself trusted until the ready exchange.
public struct ServerHello: Equatable, Sendable {
    public static let type: UInt8 = 0x10
    public static let version: UInt8 = 0x01
    public static let challengeLength = 32
    public let challenge: Data

    public init(challenge: Data) throws {
        guard challenge.count == Self.challengeLength else { throw ProtocolError.invalidServerHello }
        self.challenge = challenge
    }

    public func encoded() -> Data { Data([Self.type, Self.version]) + challenge }

    public init(encoded: Data) throws {
        guard encoded.count == 2 + Self.challengeLength,
              encoded[encoded.startIndex] == Self.type,
              encoded[encoded.index(after: encoded.startIndex)] == Self.version else {
            throw ProtocolError.invalidServerHello
        }
        try self.init(challenge: Data(encoded.dropFirst(2)))
    }
}

/// A directional session framer. Every ciphertext authenticates a u64 big-endian
/// sequence number. The ready packets consume sequence zero, so input starts at one.
public struct SessionFramer: Sendable {
    public static let maximumFrameSize = 4_096
    private let key: SymmetricKey
    private var nextOutboundSequence: UInt64 = 0
    private var nextInboundSequence: UInt64 = 0

    public init(pairing: PairingSecret, challenge: Data) throws {
        guard challenge.count == ServerHello.challengeLength else { throw ProtocolError.invalidServerHello }
        key = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: pairing.key,
            salt: challenge,
            info: Data("MacDeckInputBridge/session/v1".utf8),
            outputByteCount: 32
        )
    }

    public mutating func seal(_ packet: BridgePacket) throws -> Data {
        var sequence = nextOutboundSequence.bigEndian
        var plaintext = Data(bytes: &sequence, count: MemoryLayout<UInt64>.size)
        plaintext.append(packet.encoded())
        let combined = try ChaChaPoly.seal(plaintext, using: key).combined
        guard combined.count <= Self.maximumFrameSize else { throw ProtocolError.frameTooLarge }
        nextOutboundSequence &+= 1
        return Self.lengthFrame(combined)
    }

    public mutating func open(combined: Data) throws -> BridgePacket {
        guard combined.count <= Self.maximumFrameSize else { throw ProtocolError.frameTooLarge }
        do {
            let plaintext = try ChaChaPoly.open(ChaChaPoly.SealedBox(combined: combined), using: key)
            guard plaintext.count > MemoryLayout<UInt64>.size else { throw ProtocolError.invalidFrame }
            let sequence = plaintext.prefix(MemoryLayout<UInt64>.size).reduce(UInt64(0)) { ($0 << 8) | UInt64($1) }
            guard sequence == nextInboundSequence else { throw ProtocolError.unexpectedSequence }
            guard let packet = BridgePacket(encoded: Data(plaintext.dropFirst(MemoryLayout<UInt64>.size))) else {
                throw ProtocolError.invalidFrame
            }
            nextInboundSequence &+= 1
            return packet
        } catch let error as ProtocolError { throw error
        } catch { throw ProtocolError.authenticationFailed }
    }

    public static func lengthFrame(_ payload: Data) -> Data {
        var length = UInt32(payload.count).bigEndian
        var frame = Data(bytes: &length, count: MemoryLayout<UInt32>.size)
        frame.append(payload)
        return frame
    }
}

public struct FrameDecoder: Sendable {
    private var pending = Data()
    public init() {}

    public mutating func append(_ chunk: Data) throws -> [Data] {
        pending.append(chunk)
        var frames: [Data] = []
        while pending.count >= 4 {
            let length = pending.prefix(4).reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
            guard length > 0, length <= SessionFramer.maximumFrameSize else { throw ProtocolError.frameTooLarge }
            let total = 4 + Int(length)
            guard pending.count >= total else { break }
            let start = pending.startIndex
            let payloadStart = pending.index(start, offsetBy: 4)
            let end = pending.index(start, offsetBy: total)
            frames.append(Data(pending[payloadStart..<end]))
            pending.removeSubrange(start..<end)
        }
        return frames
    }
}
