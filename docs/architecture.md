# Architecture

> Status: **proposed**. To be confirmed in `docs/decisions/0001-tech-stack.md` (task M0-12).

## Overview

Mailcrane has one shared engine and two native apps.

- **`core/` (Rust)** does all the mail work: accounts, IMAP/JMAP sync, SMTP sending, MIME parsing, HTML sanitising, the SQLite store and search.
- **`android/` (Kotlin, Jetpack Compose, Material 3)** and **`ios/` (Swift, SwiftUI)** are the user interfaces. They call the core and draw screens. They don't speak mail protocols themselves.
- **UniFFI** generates the Kotlin and Swift bindings from `mailcrane-ffi`.

## Why this design

- **Write the hard part once.** Sync, threading, MIME and search are most of the work in an email client, and they are the same on both platforms.
- **Native UI on each platform.** Email is used all day, so it should feel right: Material 3 on Android, SwiftUI on iOS.
- **Rust is memory-safe.** Parsing untrusted email is a classic place for security bugs.
- **Tested precedent.** Other apps already ship Rust cores with UniFFI on both mobile platforms.

## Crates

| Crate | Job | Likely libraries |
|---|---|---|
| `mailcrane-core` | Accounts, sync scheduling, the public API | `tokio` |
| `mailcrane-imap` | IMAP client, IDLE, CONDSTORE/QRESYNC, server quirks | `async-imap` or own, `rustls` |
| `mailcrane-smtp` | Sending | `lettre` |
| `mailcrane-jmap` | JMAP client | `jmap-client` |
| `mailcrane-mime` | Parsing, building and sanitising messages | `mail-parser`, `mail-builder`, `ammonia` |
| `mailcrane-store` | SQLite storage, migrations, FTS5 search | `rusqlite` |
| `mailcrane-ffi` | The only API the apps see | `uniffi` |
| `mailcrane-testkit` | Fake IMAP, SMTP and JMAP servers for tests, able to misbehave on purpose | — |

Library choices are suggestions until an ADR confirms them.

## Portability

The same core code runs on every platform. It is compiled separately for each: a `.so` for Android, an XCFramework for iOS, and a `.dll` for Windows. There is no Windows app and none is planned, but the core must keep building and passing its tests on Windows so that one (or another desktop app) could be added later without changing the core. CI checks this on every PR (task M0-17).

To keep it portable, the core never reaches into the operating system for anything that differs between platforms. The app passes it in instead:

- **File locations:** the app gives the core the folders for the database and cache.
- **Secrets:** the core defines one interface for storing credentials, and each app implements it with the Android Keystore, the iOS Keychain, or the Windows Credential Manager.
- **OAuth sign-in:** the core builds the sign-in URL and does the PKCE exchange. The app opens the system browser and returns the redirect.
- **Background work and notifications:** the core exposes "sync now" and change events. Each app decides when to call it and how to notify.

Platform-specific code (`#[cfg(target_os = …)]`, `std::os::unix`, crates that only work on mobile) stays out of the core crates. If it is unavoidable, it goes behind a feature or a trait with an implementation for every platform, and it needs a decision record first.

## Data flow

1. The app asks the core to sync an account.
2. The core talks to the server, writes changes to SQLite, and emits change events.
3. The app watches those events (as a Kotlin `Flow` or Swift `AsyncSequence`) and updates the screen.
4. User actions (archive, send) go to the core, which applies them locally straight away, queues them, and replays them on the server when online.

## Open questions

- **Push notifications on iOS.** iOS doesn't allow a permanent IMAP connection in the background. Options are background fetch only, or an optional self-hostable push relay. Needs an ADR (M4-05).
- **Licence check.** MPL-2.0 is proposed because it works with the App Store, unlike GPL.
