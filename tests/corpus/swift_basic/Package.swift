// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Demo",
    targets: [
        .target(name: "Support"),
        .target(name: "Demo", dependencies: ["Support"]),
    ]
)
