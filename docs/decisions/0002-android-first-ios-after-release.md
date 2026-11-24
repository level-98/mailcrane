# 0002: Android first; the iOS app follows after the Android release

Status: accepted
Date: 2026-11-24

## Context

[ADR 0001](0001-tech-stack.md) chose a shared Rust core with two native apps, Android and iOS. Building and testing the iOS app needs macOS: Xcode, the simulator and code signing only run there. The maintainers don't have a Mac, and relying on GitHub's macOS runners for every change is slow (each PR waits for a macOS job) and makes iOS work hard to iterate on.

Building both apps at once also splits a small team's time across two UIs before either is usable.

## Decision

- Ship **Android first**. Milestones M1–M6 in [ROADMAP.md](../../ROADMAP.md) cover the core and the Android app, up to the Android 1.0 release.
- The iOS app stays at its M0 shell (`ios/`) until after that release. Its work moves to a new milestone, **M7 — iOS app**: screens, sign-in, push, widgets, iPad, accessibility, signing, TestFlight and the App Store.
- The iOS CI workflow (`.github/workflows/ios.yml`) runs only when started by hand, and Dependabot's Swift updates are switched off, until M7-02 turns them back on.
- The core and `mailcrane-ffi` stay platform-neutral and usable from Swift, so iOS can be added later without reworking the core. ADR 0001 is unchanged.
- Roadmap task IDs are kept when tasks move (for example `M1-11` and `M6-06` now sit under M7), so their issues still match.

## Why

- Android can be built, tested and released entirely from Windows or Linux, so the project can reach 1.0 without a Mac.
- The shared core means the hardest work (sync, MIME, sanitising, storage, search) is already done for iOS when M7 starts. Only the SwiftUI layer and Apple integrations remain.
- PRs no longer wait on a macOS runner, and nobody is asked to keep Swift code in step with changes they can't build.

Options we didn't pick:

- **Keep building both, testing iOS only in CI.** Every iOS change would be a slow push-and-wait cycle, and the shell would still need fixing on every FFI change.
- **Drop iOS.** Mailcrane is meant for both platforms; the App Store licensing rules (no GPL) and the shared core are kept so iOS stays possible.
- **Cross-platform UI (Flutter, Compose Multiplatform).** Already rejected in ADR 0001; switching now would not remove the need for macOS to build and sign an iOS app.

## Consequences

- The iOS shell may fall behind the core's FFI until M7-02 brings it up to date.
- iOS users wait longer. The website and README say Android first, iOS after release.
- Decisions that affect iOS (push relay, Apple sign-in flows, App Store listing) are postponed to M7, but anything in the core must not rule them out.
- When M7 starts, someone needs regular macOS access: an own Mac, a rented cloud Mac, or CI-only builds (task M7-01).
