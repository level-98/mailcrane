# F-Droid

Mailcrane is meant to be on F-Droid, built from source by F-Droid with no proprietary parts (M6-04, issue #71). This page covers what the build already guarantees, what reproducible builds still need, and a draft recipe for the `fdroiddata` merge request. Submitting it and answering the reviewers is a maintainer task.

## What the build guarantees

- **Only free dependencies.** `./gradlew checkFreeDependencies` (run by `cargo xtask ci` and by `./gradlew check`) fails if the release classpath contains Google Play Services, Firebase, ML Kit, Play Core or billing, or a known closed analytics, ads or crash SDK. It fails too if the app's sources contain a prebuilt `.jar`, `.aar`, `.so`, `.dex` or `.apk`. Native code comes only from the Rust core, built from source by `cargo-ndk`, and Java libraries only from Maven Central and Google's Maven repository (AndroidX, Hilt).
- **Free native libraries from Maven.** JNA and rustls-platform-verifier ship native code in their Maven artifacts. Both are open source (JNA: Apache-2.0/LGPL-2.1; rustls-platform-verifier: Apache-2.0/MIT), which F-Droid accepts.
- **No Google signing block.** `dependenciesInfo` is off, so AGP doesn't add its encrypted dependency-metadata block, which F-Droid's scanner flags.
- **No git data in the APK.** `vcsInfo` is off for release builds.
- **Only the ABIs the core is built for.** `abiFilters` limits the APK to arm64-v8a and x86_64, the targets `cargo-ndk` builds. Before, JNA added armeabi-v7a, armeabi and mips folders, so the app would have installed on 32-bit phones and crashed there.
- **Unsigned release builds work.** `./gradlew assembleRelease` needs no signing secrets: the release build type has no signing config, so it produces an unsigned APK for F-Droid to sign, or to compare with ours. Our signed builds are signed outside Gradle, by the release workflow ([docs/releasing.md](releasing.md)), so this stays true.
- **No anti-features.** No tracking, no ads, no non-free network services: the app talks only to the user's own mail servers. The OAuth sign-in planned in M3 will use the providers' own servers, which F-Droid's NonFreeNet rule doesn't cover for a user's own account.

## Reproducible builds

F-Droid can publish our own signed APK if its build from the same tag produces the same bytes. Done so far:

- Release builds remap the paths Rust would embed (the checkout and the cargo registry) to fixed prefixes (`--remap-path-prefix`, in `cargoNdkBuild`).
- No git commit or build time in the APK.
- Gradle and every library are pinned (`gradle/wrapper`, `gradle/libs.versions.toml`); Rust crates are pinned by `core/Cargo.lock`.
- The tools that make the native code are pinned too:

| What | Version | Where it's set | Also update |
|---|---|---|---|
| Rust | 1.100.0 | `rust-toolchain.toml` (rustup reads it for every `cargo` run) | `toolchain:` in `.github/workflows/ci.yml` (both jobs); the recipe's `--default-toolchain` |
| Android NDK | 30.0.16248370 | `ndkVersion` in `android/app/build.gradle.kts`; `cargoNdkBuild` passes the same NDK to cargo-ndk | `NDK_VERSION` in `ci.yml`; the recipe's `ndk:`; `android/README.md` |
| cargo-ndk | 4.1.2 | `cargo-ndk@4.1.2` in `ci.yml` | the recipe's `cargo install`; `android/README.md` |

Then build twice on different machines (or once locally and once with `fdroid build`) and compare with `apksigcopier compare`, or `diffoscope` for detail. Record the result in issue #71.

### Updating a pin

Change every place in its row in one PR, so CI, developers and F-Droid keep building with the same tools. After a Rust update, run `cargo xtask ci` and fix any new clippy warnings in the same PR. Developers need nothing more for Rust: rustup installs the new version on the next `cargo` run. For a new NDK they run `sdkmanager "ndk;<version>"`; until they do, Gradle stops and names the missing NDK (or downloads it, where the SDK licences are accepted). On F-Droid a pin change takes effect with the next release's recipe entry, so update the recipe when that release is submitted.

## Draft recipe for fdroiddata

`metadata/app.mailcrane.yml`, to adapt when the merge request is opened. The versions match the pins above; keep them in step.

```yaml
Categories:
  - Email
License: MPL-2.0
AuthorName: Mailcrane contributors
WebSite: https://mailcrane.app
SourceCode: https://github.com/level-98/mailcrane
IssueTracker: https://github.com/level-98/mailcrane/issues
Changelog: https://github.com/level-98/mailcrane/blob/main/CHANGELOG.md

AutoName: Mailcrane

RepoType: git
Repo: https://github.com/level-98/mailcrane.git

Builds:
  - versionName: 0.1.0
    versionCode: 1
    commit: v0.1.0
    subdir: android/app
    sudo:
      - apt-get update
      - apt-get install -y build-essential
    gradle:
      - yes
    srclibs:
      - rustup@1.27.1
    prebuild:
      - $$rustup$$/rustup-init.sh -y --default-toolchain 1.100.0 --profile minimal --target aarch64-linux-android,x86_64-linux-android
      - source $HOME/.cargo/env
      - cargo install cargo-ndk --version 4.1.2 --locked
    ndk: 30.0.16248370
    output: build/outputs/apk/release/app-release-unsigned.apk

AutoUpdateMode: Version
UpdateCheckMode: Tags
CurrentVersion: 0.1.0
CurrentVersionCode: 1
```

The store listing (title, descriptions, icon, per-version changelogs) is read from `fastlane/metadata/android/en-US/` in this repository; fdroiddata doesn't need a copy.

## Store listing files

- `fastlane/metadata/android/en-US/title.txt`, `short_description.txt` (80 characters at most), `full_description.txt` (4000 at most), `images/icon.png` (512 px, from `design/app-icon/android/playstore-icon-512.png`).
- `changelogs/<versionCode>.txt` (500 characters at most): for each release, `python scripts/release_notes.py <version> --fastlane <versionCode>` writes it from the changelog fragments, with internal ones left out, before `--apply` removes them. The first release's notes were written by hand.
- `images/phoneScreenshots/`: still to add, from a phone or the emulator with test data only. A maintainer task.

## Maintainer steps

1. Tag the first release (see the release process, #70).
2. Add screenshots.
3. Run `fdroid lint` and `fdroid build -v -l app.mailcrane` with the recipe (F-Droid's server tools in Docker), and install the result.
4. Open the merge request to `fdroid/fdroiddata`, and answer the reviewers.
5. Record the reproducibility result in #71.
