# Mailcrane for Android

Kotlin and Jetpack Compose (Material 3 Expressive) app that calls the Rust core through UniFFI bindings.

## Prerequisites

- JDK 17 or newer
- Android SDK (platform 37.2) and NDK 30.0.16248370 (`sdkmanager "ndk;30.0.16248370"`); set `ANDROID_HOME`. The build uses that NDK whatever `ANDROID_NDK_HOME` says.
- Rust via `rustup`: `rust-toolchain.toml` picks the version (1.100.0) and installs it with the Android targets on the first `cargo` run
- `cargo install cargo-ndk --version 4.1.2 --locked`

## Build

```bash
cd android
./gradlew ktlintCheck lintDebug testDebugUnitTest assembleDebug
./gradlew installDebug   # with an emulator running
```

The `buildRust` Gradle task (run by `preBuild`) builds `mailcrane-ffi` for arm64-v8a and x86_64 into
`app/build/rustJniLibs/debug` or `app/build/rustJniLibs/release` and generates the Kotlin bindings into
`app/build/generated/uniffi`. Release builds use the Rust release profile. Each build type packages only its
own folder, and one Gradle run builds one profile: build debug and release variants in separate runs
(`./gradlew assembleRelease`, then `./gradlew assembleDebug`). Nothing generated is committed.

