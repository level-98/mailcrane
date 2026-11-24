# iOS app

> **Paused until after the Android 1.0 release.** This is the M0 shell: it builds the core as an XCFramework and calls it, nothing more. The iOS app catches up in milestone **M7** of [ROADMAP.md](../ROADMAP.md) (see [decision 0002](../docs/decisions/0002-android-first-ios-after-release.md)). Until then CI only runs the iOS check when a maintainer starts the `iOS` workflow by hand, and the shell may fall behind the core.

## Prerequisites

- macOS with Xcode 15 or later
- Rust with the Apple iOS targets
- XcodeGen
- SwiftLint

Install the tools:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
brew install xcodegen swiftlint
```

## Build and test

From the repository root:

```bash
cd ios
sh scripts/build-xcframework.sh
xcodegen
swiftlint --strict
xcodebuild test -project Mailcrane.xcodeproj -scheme Mailcrane \
  -destination 'platform=iOS Simulator,name=iPhone 17'
```

The XCFramework is generated at `ios/build/MailcraneFFI.xcframework`. Generated
Swift bindings and the Xcode project are build artifacts and are not committed.
