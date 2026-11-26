# 0005: No iOS app before Mailcrane 1.0

Status: accepted (replaces [0002](0002-android-first-ios-after-release.md))
Date: 2026-11-26

## Context

[ADR 0002](0002-android-first-ios-after-release.md) put Android first and kept an iOS shell (`ios/`) waiting for a milestone M7 right after the Android release. In practice the shell only cost effort: it needed a macOS runner to check, fell behind every FFI change, and kept iOS rules, CI steps and design outputs in a repository that ships one app. A small team has one platform to get right, and the maintainer has decided there is no iOS work planned before 1.0.

## Decision

- **iOS is not planned before Mailcrane 1.0.** It may be looked at again only after the 1.0 release. Milestone M7 is closed and its issues are closed as "not planned".
- The iOS shell, its CI workflow (`ios.yml`), the iOS step in `cargo xtask ci`, and the iOS app icon and splash outputs in `design/` are removed.
- **The door stays open:**
  - the Rust core stays platform-neutral, and the core crates make no Android-only assumptions;
  - `mailcrane-ffi` and UniFFI stay, so Swift bindings can be generated again;
  - the licence rules stay (no GPL, LGPL or AGPL), so App Store distribution remains possible;
  - the shell as it was is kept at the git tag `archive/ios-shell` (commit `ff2b582`). To restore it: `git checkout archive/ios-shell -- ios .github/workflows/ios.yml design/app-icon/ios design/splash/ios`, then bring back the iOS parts of `design/build.py` and `core/xtask` from the same tag.

## Why

- Nothing in the 1.0 plan needs iOS, and every iOS file was something to keep in step that nobody could build without a Mac.
- Removing it makes CI one path (no skipped steps, no hand-started macOS job), and the docs describe the app that exists.
- Keeping the core neutral, UniFFI and the tag costs almost nothing and keeps a later iOS app possible.

Options we didn't pick:

- **Keep the shell paused (0002).** It kept costing review time and drifting from the core for no user benefit before 1.0.
- **Delete without a tag.** Losing the work entirely would make a later start slower for no gain.

## Consequences

- The README, roadmap, contributing guide and website say Android only, with "iPhone: not before 1.0".
- Contributors need no Xcode or macOS, and `cargo xtask ci` runs every step on Windows, Linux and macOS.
- Old changelog fragments and closed issues keep their history; they aren't rewritten.
- A future iOS app starts from the tag and a new decision that replaces this one.
