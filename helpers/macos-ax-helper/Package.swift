// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "DuxNodeMacosAxHelper",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .executable(name: "dux-node-macos-ax-helper", targets: ["DuxNodeMacosAxHelper"]),
    ],
    targets: [
        .executableTarget(
            name: "DuxNodeMacosAxHelper",
            path: "Sources"
        ),
    ]
)
