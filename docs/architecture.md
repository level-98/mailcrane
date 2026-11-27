# Architecture

> Status: **accepted**, see [ADR 0001](decisions/0001-tech-stack.md).

## Overview

Mailcrane has one shared engine and two native apps.

- **`core/` (Rust)** does all the mail work: accounts, IMAP/JMAP sync, SMTP sending, MIME parsing, HTML sanitising, the SQLite store and search.
- **`android/` (Kotlin, Jetpack Compose, Material 3)** is the user interface. It calls the core and draws screens; it doesn't speak mail protocols itself. There's no iOS app before 1.0 ([decision 0005](decisions/0005-no-ios-before-1-0.md)), but the core and its FFI stay platform-neutral so one could be added later without core changes.
- **UniFFI** generates the Kotlin bindings from `mailcrane-ffi` (and could generate Swift ones again).

## Why this design

- **Write the hard part once.** Sync, threading, MIME and search are most of the work in an email client, and they are the same on both platforms.
- **Native UI.** Email is used all day, so it should feel right: Material 3 on Android.
- **Rust is memory-safe.** Parsing untrusted email is a classic place for security bugs.
- **Tested precedent.** Other apps already ship Rust cores with UniFFI on both mobile platforms.

## Crates

| Crate | Job | Main libraries |
|---|---|---|
| `mailcrane-core` | Accounts, sync scheduling, the public API | `tokio`, `hyper` (autoconfig, unsubscribe), `quick-xml`, `url` |
| `mailcrane-imap` | IMAP client (our own), IDLE, CONDSTORE/QRESYNC, server quirks | `rustls` |
| `mailcrane-smtp` | Sending (our own SMTP client) | `rustls` |
| `mailcrane-sieve` | ManageSieve client (our own) and Sieve scripts for server-side rules ([ADR 0004](decisions/0004-server-side-rules-managesieve.md)) | `rustls` |
| `mailcrane-jmap` | JMAP client (our own). Not yet connected to `mailcrane-core` | `hyper`, `serde_json` |
| `mailcrane-mime` | Parsing, building and sanitising messages | `mail-parser`, `ammonia` |
| `mailcrane-store` | SQLite storage, migrations, FTS5 search | `rusqlite` |
| `mailcrane-ffi` | The only API the apps see | `uniffi` |
| `mailcrane-testkit` | Fake IMAP, SMTP, JMAP and ManageSieve servers for tests, able to misbehave on purpose | `rcgen`, `hyper` |

[ADR 0001](decisions/0001-tech-stack.md#library-choices) records which of these were decided and where.

## Portability

The same core code runs on every platform. It is compiled separately for each: a `.so` for Android and a `.dll` for Windows. There is no Windows app and none is planned, but the core must keep building and passing its tests on Windows so that one (or another desktop app) could be added later without changing the core. CI checks this on every PR (task M0-17).

To keep it portable, the core never reaches into the operating system for anything that differs between platforms. The app passes it in instead:

- **File locations:** the app gives the core the folders for the database and cache.
- **Secrets:** the core defines one interface for storing credentials, and each app implements it with the Android Keystore (tests use an in-memory one).
- **OAuth sign-in:** the core builds the sign-in URL and does the PKCE exchange. The app opens the system browser and returns the redirect.
- **Background work and notifications:** the core exposes "sync now" and change events. Each app decides when to call it and how to notify.

Platform-specific code (`#[cfg(target_os = …)]`, `std::os::unix`, crates that only work on mobile) stays out of the core crates. If it is unavoidable, it goes behind a feature or a trait with an implementation for every platform, and it needs a decision record first.

**Known exception, not yet recorded:** TLS certificate checking uses `rustls-platform-verifier`, which needs Android-specific setup. The TLS code in `mailcrane-imap`, `mailcrane-smtp`, `mailcrane-sieve`, `mailcrane-jmap`, `mailcrane-core` (autoconfig and unsubscribe) and `mailcrane-ffi` (`android_tls`) has `#[cfg(target_os = "android")]` branches. It needs a decision record, or to move behind a trait as described above.

## Data flow

1. The app asks the core to sync an account.
2. The core talks to the server, writes changes to SQLite, and emits change events.
3. The app watches those events (as a Kotlin `Flow`) and updates the screen.
4. User actions (archive, send) go to the core, which applies them locally straight away, queues them, and replays them on the server when online.

## Open questions

- **Network calls beyond the user's mail server.** AGENTS.md §4.7 allows them only with an accepted decision. The bundled ISPDB has one ([ADR 0003](decisions/0003-bundled-ispdb.md)). These don't yet: one-click unsubscribe (M4-10) sends an HTTPS POST to the sender's unsubscribe URL, and automatic setup fetches autoconfig files from the domain of the address's MX provider.
- **JMAP storage.** The store is built around IMAP (integer UIDs, one folder per message). Connecting `mailcrane-jmap` needs a maintainer decision on the schema before its migration ships, because migrations can't be undone once released.
