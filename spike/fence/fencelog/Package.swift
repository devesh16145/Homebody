// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "fencelog",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "fencelog",
            path: "Sources"
        )
    ]
)
