// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "MailcraneCore",
    platforms: [
        .iOS(.v17)
    ],
    products: [
        .library(
            name: "MailcraneCore",
            targets: ["MailcraneCore"]
        )
    ],
    targets: [
        .binaryTarget(
            name: "MailcraneCoreFFI",
            path: "../build/MailcraneFFI.xcframework"
        ),
        .target(
            name: "MailcraneCore",
            dependencies: ["MailcraneCoreFFI"],
            path: "Sources/MailcraneCore"
        )
    ]
)
