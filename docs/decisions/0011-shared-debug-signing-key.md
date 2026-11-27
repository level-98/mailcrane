# 0011: A shared signing key for debug builds

Status: accepted
Date: 2026-11-27

## Context

Android installs an update only if it has the same signature as the installed app. Gradle signs debug builds with a key it creates in each machine's `~/.android/debug.keystore`, so every machine and every CI runner signs with a different key. A debug APK from CI can't replace one built elsewhere, or one from another CI run: QA had to uninstall first (losing the app's state), or build locally with Gradle, a heavy job (docs/fleet.md rule 11). On the maintainer's phone, seats may never uninstall, so the phone could only take local builds from one machine.

The maintainer delegated the decision to the fleet lead, who approved this on 2026-11-27 (#545).

## Decision

- `android/debug.keystore` is committed: PKCS12, alias `androiddebugkey`, passwords `android`, `CN=Android Debug,O=Android,C=US`, RSA 2048, valid until 2076. `signingConfigs.debug` in `android/app/build.gradle.kts` points at it, and the `debug` build type (`app.mailcrane.debug`) uses it. Every debug APK, local or from CI, has the same signature.
- **It signs debug builds only.** The `release` build type has no signing config, so `assembleRelease` stays unsigned for F-Droid and the release workflow to sign ([docs/fdroid.md](../fdroid.md), [docs/releasing.md](../releasing.md)). A `finalizeDsl` check fails the build if the release build type is ever given a signing config. Nightly builds are signed with the development key (#70) by `nightly.yml`, and releases with the release key by `release.yml`, both outside Gradle. Neither ever uses this key.
- QA installs CI's debug APK with `adb install -r`, on emulators and on the maintainer's phone (docs/fleet.md).

## Why

- Debug keys are public by convention: Android's own tools create them with the same well-known passwords. Committing one discloses nothing that protects users.
- The debug app is a separate package (`app.mailcrane.debug`, #405) that holds only synthetic test data. Its key can't update the release app `app.mailcrane`, because Android ties updates to the package as well as the key.
- One signature lets every seat test CI's builds without a local Gradle build.

Options we didn't pick:

- **The key as a CI secret:** local builds would still differ from CI's, and debug builds would need secrets to build, which forks and F-Droid don't have.
- **Uninstalling before each install:** that loses the app's state, and seats may never uninstall on the maintainer's phone.

## Consequences

- **The key is public, so anyone can sign an `app.mailcrane.debug`.** Android accepts it as an update to an installed debug app, and the update keeps that app's data: its database, its stored passwords and OAuth tokens, everything. So the debug app must only ever hold test data. README and CONTRIBUTING say plainly: never add a real account to a debug build. Installing a debug APK from anywhere other than this project's CI or your own build is just as unsafe. The release app `app.mailcrane` isn't affected: Android ties updates to the package as well as the key, and release builds never use this key.
- When the key first lands, an existing debug install can't be updated over once. The maintainer uninstalls `app.mailcrane.debug` on their phone themselves; emulator snapshots are refreshed.
- F-Droid's scanner might flag the committed keystore. If it does, the F-Droid build recipe can delete it before building, since release builds don't use it.
