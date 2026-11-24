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

## Data flow

1. The app asks the core to sync an account.
2. The core talks to the server, writes changes to SQLite, and emits change events.
3. The app watches those events (as a Kotlin `Flow` or Swift `AsyncSequence`) and updates the screen.
4. User actions (archive, send) go to the core, which applies them locally straight away, queues them, and replays them on the server when online.

## Open questions

- **Push notifications on iOS.** iOS doesn't allow a permanent IMAP connection in the background. Options are background fetch only, or an optional self-hostable push relay. Needs an ADR (M4-05).
- **Licence check.** MPL-2.0 is proposed because it works with the App Store, unlike GPL.
