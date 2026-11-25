# 0001: Shared Rust core with native UIs, bound through UniFFI

Status: accepted
Date: 2026-11-24

## Context

Most of the work in an email client is sync, MIME parsing, threading, search and storage. That logic is identical on Android and iOS; only the screens differ.

The client also parses hostile input all day: server responses, message bytes, attachments and HTML written by strangers. Memory-safety bugs in mail parsers are a classic route to remote exploits.

The team is small and leans on AI agents to send PRs, so the code base must be one that strict tooling (compiler, lints, `cargo deny`, one `cargo xtask ci` command) can keep honest without a large review team.

The apps must ship on the App Store, Google Play and F-Droid. That rules out GPL-family dependencies (App Store) and anything that needs Google Play Services or proprietary SDKs (F-Droid).

## Decision

- **Core:** one Rust workspace in `core/`, with the crates listed in [`docs/architecture.md`](../architecture.md#crates): `mailcrane-core`, `-imap`, `-smtp`, `-jmap`, `-mime`, `-store`, `-ffi` and `-testkit`. Stable Rust, edition 2021, async on `tokio`, errors with `thiserror`.
- **Android:** Kotlin with Jetpack Compose and Material 3 Expressive (`MaterialExpressiveTheme`), `minSdk 26`.
- **iOS:** Swift with SwiftUI, iOS 17+. The core ships as an XCFramework.
- **Bindings:** UniFFI generates the Kotlin and Swift bindings from `mailcrane-ffi`, the only crate the apps see.
- **Storage:** SQLite through `rusqlite`, with FTS5 for offline search and numbered migrations.
- **TLS:** `rustls` only. Never OpenSSL or the platform TLS stack from inside the core.
- **Licences:** dependencies must be MPL-2.0-compatible (MIT, Apache-2.0, BSD, ISC, MPL-2.0, Zlib and the other licences in `core/deny.toml`). No GPL, LGPL or AGPL. `cargo deny` enforces this.
- **Portability:** the core must keep building and passing its tests on Windows as well as the mobile targets, and never reaches into the OS for things that differ between platforms; the apps pass in file locations, secret storage, OAuth redirects and scheduling, as described in [`docs/architecture.md`](../architecture.md#portability). CI checks the Windows build on every PR (task M0-17).

## Alternatives considered

**Kotlin Multiplatform (shared Kotlin core).** KMP would let Android share code with no FFI layer, and Compose Multiplatform could even share UI. But the iOS side would run Kotlin/Native, whose ecosystem for IMAP, MIME, HTML sanitising and TLS is thin, so we would write most of those libraries ourselves on a less mature toolchain. Kotlin is memory-safe, but Rust's mail and TLS crates are more mature, and Rust gives tighter control over memory and binary size on iOS.

**Flutter.** One UI code base, but the UI is drawn by Flutter rather than the platform, so it never quite feels like Material 3 on Android or like iOS on an iPhone, and it lags behind new platform features (Material 3 Expressive, iOS widgets and system integration). The mail engine would still need to be written in Dart or bridged to native code, so we would not avoid an FFI boundary anyway.

**React Native.** Same UI trade-off as Flutter, plus a JavaScript runtime between the user and their mail. Heavy parsing and sync in JavaScript is slow and battery-hungry, so the engine would end up in native modules: an FFI boundary again, with a large dependency tree to audit on top.

**Two fully native apps, no shared core.** Best possible platform feel and the simplest build, but every hard part (IMAP sync, QRESYNC, MIME, threading, search, sanitising) would be written and debugged twice, in Kotlin and in Swift. With a small team that halves our speed and guarantees the two apps drift apart in behaviour and security fixes.

## Consequences

Good:

- The hard parts are written, tested and fuzzed once, and both apps get every fix at the same time.
- Rust's memory safety (`#![forbid(unsafe_code)]` outside `mailcrane-ffi`) and the no-panic lints suit a program that parses hostile input.
- Each app uses its own platform's UI toolkit, so it looks and behaves like a native app.
- Strict compiler and lint rules give agents fast, unambiguous feedback.
- The core can later power a desktop app without changes, because it stays portable.

Bad:

- The FFI boundary must be designed and maintained: types kept simple, async and callbacks mapped through UniFFI, and generated bindings rebuilt on every core change.
- Building needs three toolchains (Rust with Android and iOS targets, Gradle with the NDK, Xcode), which makes CI and first-time setup slower.
- Each app carries a native library, so download size is larger than a pure Kotlin or Swift app.
- Contributors who only know Kotlin or Swift need some Rust to work on mail logic.
- UniFFI and the Material 3 Expressive APIs are still pre-1.0 or alpha, so upgrades may need code changes.

## Library choices

From the "likely libraries" column in [`docs/architecture.md`](../architecture.md#crates):

| Library | Status |
|---|---|
| `tokio`, `thiserror` | Confirmed by this decision. |
| `rustls` (with `tokio-rustls`) | Confirmed by this decision. |
| `rusqlite` (SQLite with FTS5) | Confirmed by this decision. |
| `uniffi` | Confirmed by this decision. |
| `async-imap` or our own IMAP client | Our own client in `mailcrane-imap` (#122). `async-imap` is not used. |
| `lettre` | Not used. `mailcrane-smtp` has its own SMTP client (#114). |
| `jmap-client` | Not used. `mailcrane-jmap` has its own client on `hyper` and `serde_json` (#233). |
| `mail-parser`, `mail-builder` | `mail-parser` is used (#121). `mail-builder` is not; messages are built in our own code. |
| `ammonia` | Used for the HTML sanitiser (#127). |

These were chosen in the PRs listed, not in decision records. Later dependencies not covered by this table (for example `hyper`, `quick-xml`, `rustls-platform-verifier`, `url`, `zeroize`, and on Android Glance, WorkManager and `androidx.biometric`) were also added in their PRs. New choices follow the "Decisions that only maintainers make" rules in [CONTRIBUTING.md](../../CONTRIBUTING.md#decisions-that-only-maintainers-make).
