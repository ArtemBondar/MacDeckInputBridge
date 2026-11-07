// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "MacDeckInputBridge",
    platforms: [.macOS(.v13)],
    products: [
        .library(name: "BridgeCore", targets: ["BridgeCore"]),
        .executable(name: "mac-deck-sender", targets: ["MacDeckSender"]),
        .executable(name: "mac-deck-menu-bar", targets: ["MacDeckMenuBar"])
    ],
    targets: [
        .target(name: "BridgeCore"),
        .executableTarget(name: "MacDeckSender", dependencies: ["BridgeCore"]),
        .executableTarget(name: "MacDeckMenuBar", dependencies: ["BridgeCore"]),
        .testTarget(name: "BridgeCoreTests", dependencies: ["BridgeCore"])
    ]
)
