import Foundation

/// Reassembles one authenticated clipboard update without retaining previous
/// values. Protocol errors are rejected before the system clipboard is touched.
struct IncomingClipboardTransfer {
    private let transferID: UInt64
    private let totalBytes: Int
    private let chunkCount: UInt16
    private var nextIndex: UInt16 = 0
    private var bytes = Data()

    init?(transferID: UInt64, totalBytes: UInt32, chunkCount: UInt16) {
        let size = Int(totalBytes)
        guard size <= BridgePacket.maximumClipboardTextBytes else { return nil }
        let expected = size == 0 ? 0 : (size + BridgePacket.maximumClipboardChunkBytes - 1) / BridgePacket.maximumClipboardChunkBytes
        guard expected == Int(chunkCount) else { return nil }
        self.transferID = transferID
        self.totalBytes = size
        self.chunkCount = chunkCount
        bytes.reserveCapacity(size)
    }

    mutating func append(transferID: UInt64, index: UInt16, chunk: Data) -> Bool {
        guard transferID == self.transferID, index == nextIndex, index < chunkCount,
              !chunk.isEmpty, chunk.count <= BridgePacket.maximumClipboardChunkBytes,
              bytes.count + chunk.count <= totalBytes else { return false }
        bytes.append(chunk)
        nextIndex += 1
        return true
    }

    func finish(transferID: UInt64) -> String? {
        guard transferID == self.transferID, nextIndex == chunkCount, bytes.count == totalBytes else { return nil }
        return String(data: bytes, encoding: .utf8)
    }
}
