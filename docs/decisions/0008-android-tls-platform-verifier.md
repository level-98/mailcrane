# 0008: TLS certificate checks on Android

Status: accepted
Date: 2026-11-27

## Context

Every connection is TLS ([AGENTS.md §4.5](../../AGENTS.md#4-privacy-and-hostile-content)): IMAP and SMTP from the first versions (#122, #114), then ManageSieve, JMAP, autoconfig and unsubscribe. Certificates are checked with `rustls-platform-verifier`, so the phone's own trust store decides, not a list compiled into the app. On Android that needs Android-specific code: the verifier asks the system's trust manager through the JVM, so it needs the application `Context` before the first handshake (#243, #244). The app calls `TlsSetup.init(context)` from `Application.onCreate()`, which reaches `mailcrane-ffi/src/android_tls.rs`. The TLS code in `mailcrane-imap`, `mailcrane-smtp`, `mailcrane-sieve`, `mailcrane-jmap` and `mailcrane-core` (autoconfig, unsubscribe) has `#[cfg(target_os = "android")]` branches.

[docs/architecture.md](../architecture.md#portability) asks for a decision record before platform-specific code goes into the core crates, and listed this as a known exception without one. The fleet lead made the decision under docs/fleet.md, "Who decides" (#479).

## Decision

- On Android, certificates are checked by `rustls-platform-verifier` against Android's trust store: the system CAs, and CA certificates the user installed. [`cargo xtask dev-mail`](../dev-mail.md) relies on the user-installed CA on emulators.
- The Android branches stay behind `#[cfg(target_os = "android")]`. On other platforms (tests, the Windows CI check), the same verifier uses that OS's store, and tests may add the testkit's root. That adds trust and can't turn checking off.
- **TLS always:** there is no API to skip verification, no setting to accept a bad certificate, and a server without STARTTLS is an error, never plain text.

## Why

- The phone's trust store is what the user and their organisation control: an updated root, a removed CA or a company CA works with no app update.
- A bundled root list (`webpki-roots`) would ignore all that and go stale between releases.
- `rustls` stays the TLS library (never OpenSSL, [ADR 0001](0001-tech-stack.md)); only certificate checking is delegated to the platform.

Options we didn't pick:

- **A trait in the core with a per-platform implementation**, as architecture.md suggests for platform code. The verifier crate already is that abstraction, with an implementation for every platform the core builds on. A second layer would add code without changing behaviour.
- **Bundled roots only:** see above.

## Consequences

- The app must call `TlsSetup.init` before the first connection (done in `Application.onCreate()` since #244). Without it every handshake fails: it fails closed, with a TLS error, never open.
- A user-installed CA is trusted, as in Android's other apps that use the system store. That's what lets dev-mail work on an emulator, and why its CA must never go on a phone in use.
- The cfg branches are an accepted exception to the portability rule; new platform-specific code still needs its own decision.
