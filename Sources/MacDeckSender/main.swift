import BridgeCore
import Foundation

private enum CLIError: LocalizedError {
    case usage
    case invalidPort

    var errorDescription: String? {
        switch self {
        case .usage:
            return "Usage: mac-deck-sender --host <Steam Deck IP> --pairing-secret <base64-32-byte-secret> [--port 4242]"
        case .invalidPort:
            return "Port must be between 1 and 65535."
        }
    }
}

private func argument(_ name: String, from arguments: [String]) -> String? {
    guard let index = arguments.firstIndex(of: name), arguments.indices.contains(index + 1) else { return nil }
    return arguments[index + 1]
}

do {
    let arguments = Array(CommandLine.arguments.dropFirst())
    guard let host = argument("--host", from: arguments), let secret = argument("--pairing-secret", from: arguments) else {
        throw CLIError.usage
    }
    let port = UInt16(argument("--port", from: arguments) ?? "4242") ?? 0
    guard port > 0 else { throw CLIError.invalidPort }

    let sender = SenderController()
    try sender.configure(settings: BridgeSettings(host: host, port: port), pairingSecret: secret)
    try sender.start()
    print("MacDeckInputBridge is LOCAL. Press Control + Shift + F12 to connect; press it again to return LOCAL.")
    CFRunLoopRun()
} catch {
    fputs("mac-deck-sender: \(error.localizedDescription)\n", stderr)
    exit(2)
}
