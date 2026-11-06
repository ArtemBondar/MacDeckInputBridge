import CryptoKit
import Foundation

public enum BridgePacket: Equatable, Sendable {
    case key(code: UInt16, value: UInt8)
    case releaseAll
    case ping

    public func encoded() -> Data {
        switch self {
        case let .key(code, value):
            return Data([1, UInt8(code >> 8), UInt8(code & 0xff), value])
        case .releaseAll:
            return Data([2])
        case .ping:
            return Data([3])
        }
    }

    public init?(encoded: Data) {
        let bytes = Array(encoded)
        if bytes.count == 4, bytes[0] == 1, bytes[3] <= 2 {
            self = .key(code: UInt16(bytes[1]) << 8 | UInt16(bytes[2]), value: bytes[3])
        } else if bytes == [2] {
            self = .releaseAll
        } else if bytes == [3] {
            self = .ping
        } else {
            return nil
        }
    }
}

public enum ProtocolError: Error, Equatable {
    case invalidPairingSecret
    case invalidFrame
    case frameTooLarge
    case authenticationFailed
}

public struct PairingSecret: Sendable {
    public let key: SymmetricKey

    public init(base64: String) throws {
        guard let data = Data(base64Encoded: base64), data.count == 32 else {
            throw ProtocolError.invalidPairingSecret
        }
        key = SymmetricKey(data: data)
    }
}

/// Wire format: a big-endian 32-bit frame size followed by CryptoKit's ChaChaPoly
/// combined box (12-byte random nonce, ciphertext, 16-byte authentication tag).
public struct SecureFramer: Sendable {
    public static let maximumFrameSize = 4_096
    private let key: SymmetricKey

    public init(pairing: PairingSecret) { key = pairing.key }

    public func seal(_ packet: BridgePacket) throws -> Data {
        let sealed = try ChaChaPoly.seal(packet.encoded(), using: key)
        let combined = sealed.combined
        guard combined.count <= Self.maximumFrameSize else {
            throw ProtocolError.frameTooLarge
        }
        var length = UInt32(combined.count).bigEndian
        var frame = Data(bytes: &length, count: MemoryLayout<UInt32>.size)
        frame.append(combined)
        return frame
    }

    public func open(combined: Data) throws -> BridgePacket {
        guard combined.count <= Self.maximumFrameSize else { throw ProtocolError.frameTooLarge }
        do {
            let plaintext = try ChaChaPoly.open(ChaChaPoly.SealedBox(combined: combined), using: key)
            guard let packet = BridgePacket(encoded: plaintext) else { throw ProtocolError.invalidFrame }
            return packet
        } catch let error as ProtocolError {
            throw error
        } catch {
            throw ProtocolError.authenticationFailed
        }
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
            guard length > 0, length <= SecureFramer.maximumFrameSize else {
                throw ProtocolError.frameTooLarge
            }
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
