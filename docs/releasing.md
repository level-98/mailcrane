# Releasing the Android app

How a maintainer makes a signed release (M6-03, issue #70). The workflow is [.github/workflows/release.yml](../.github/workflows/release.yml). Google Play upload comes later (#72) and isn't covered yet.

## What happens on a tag

Pushing a tag such as `v0.1.0` runs two jobs:

1. **build** (no secrets): checks that the tag is on `main` and matches the version in Gradle (`cargo xtask release-check`), runs `cargo xtask ci --strict`, builds the unsigned release APK and AAB (with `checkFreeDependencies`, so nothing non-free gets in) and the release notes from `docs/changelog.d/`.
2. **sign** (the `release` environment): waits for a maintainer to approve it, signs the APK (`apksigner`) and AAB (`jarsigner`), writes `SHA256SUMS`, and makes a **draft** GitHub Release with the APK, AAB, checksums and notes. You read the draft and press Publish.

The signing key is only ever on the runner of the sign job, which builds nothing, so no build script or library code runs next to it. The keystore is written to a temporary file and deleted at the end, even if signing fails. GitHub masks the secrets in logs, and the workflow never prints them.

If the secrets are missing (a fork, or before the setup below), the sign job says "Release not signed", keeps the unsigned files as a workflow artifact, and makes no GitHub Release. Pull requests that change the workflow run only the build job, never signing.

**Run workflow** (Actions → Release) builds and signs the chosen branch the same way and keeps the files as a workflow artifact for 30 days, without a GitHub Release. Useful for testing a signed build before tagging.

## One-time setup (maintainer)

Do these once. None of it goes in the repository.

### 1. Decide on Play App Signing

Google Play re-signs apps with its own **app signing key** unless you give it yours. If Google holds a key of its own, copies from Google Play and copies from GitHub or F-Droid have different signatures, and Android won't update one with the other: users would have to uninstall (and lose their local data) to switch. So when you enrol in Play App Signing (#72), choose **"Use my own key"** and upload the key made below, so every channel shares one signature. This is a maintainer decision; the rest of this guide works either way.

### 2. Create the key

On your own computer, with the JDK's `keytool` (Android Studio includes one, under `jbr\bin`):

```powershell
keytool -genkeypair -v -keystore mailcrane-release.p12 -storetype PKCS12 -alias mailcrane -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Mailcrane"
```

It asks for a password. A PKCS12 keystore uses the same password for the store and the key, so `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_PASSWORD` below are the same value. Use a long random password from a password manager.

**Back it up now.** Losing this file or its password means no more updates for anyone who installed the app: Android only accepts updates signed with the same key. Keep at least two encrypted copies offline (for example an encrypted USB stick and your password manager's file storage), with the password stored separately. Never commit it, email it or put it in a chat.

### 3. Create the `release` environment

GitHub → the repository → **Settings → Environments → New environment**, named `release`:

- **Required reviewers:** the maintainers. Each signing run then waits for one of you to approve it.
- **Deployment branches and tags:** "Selected branches and tags", then add a **tag** rule `v*` and a **branch** rule `main`. Runs from other branches can't reach the secrets.
- Optional but recommended: a repository ruleset (Settings → Rules → Rulesets) that lets only maintainers create tags matching `v*`.

### 4. Add the secrets to the environment

In the `release` environment, under **Environment secrets** (not repository secrets, which every workflow can read), add:

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | the keystore file, base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD` | the keystore password |
| `ANDROID_KEY_ALIAS` | `mailcrane` (the `-alias` above) |
| `ANDROID_KEY_PASSWORD` | the key password (the same as the keystore password for PKCS12) |

With the GitHub CLI, from the folder holding the keystore. The base64 text goes straight from the file to GitHub, and the password prompts don't show what you type:

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("$PWD\mailcrane-release.p12")) | gh secret set ANDROID_KEYSTORE_BASE64 --env release --repo level-98/mailcrane
```

```powershell
gh secret set ANDROID_KEYSTORE_PASSWORD --env release --repo level-98/mailcrane
```

```powershell
gh secret set ANDROID_KEY_ALIAS --env release --repo level-98/mailcrane --body mailcrane
```

```powershell
gh secret set ANDROID_KEY_PASSWORD --env release --repo level-98/mailcrane
```

Or paste them in the web page. Then delete any base64 copy you made; keep only the backed-up keystore.

### 5. Try it

Actions → Release → **Run workflow** on `main`, approve the sign job, and check that its log ends with "Signed and verified." and shows the certificate's SHA-256 digest. Write that digest down: it's the app's public fingerprint, which F-Droid's reproducible-build check and careful users compare against. It's safe to publish.

## Version numbers

Tags are `vMAJOR.MINOR.PATCH`, with an optional pre-release suffix: `v0.1.0`, `v0.1.0-rc.1`. In `android/app/build.gradle.kts`:

- `versionName` is the tag without the `v` (`0.1.0-rc.1`).
- `versionCode` is `MAJOR × 10000 + MINOR × 100 + PATCH` (`v1.2.3` → `10203`), so MINOR and PATCH stay below 100. A pre-release and its final release share a `versionCode`, so install a release candidate only on test devices: Android won't replace it with the final build of the same code. Use a new PATCH if a candidate went to real users.

`cargo xtask release-check v0.1.0` checks both locally. A tag with Gradle values that don't match fails at once, before the long build.

## Making a release

1. Make a PR that sets `versionName` and `versionCode`, runs `python scripts/release_notes.py <version> --apply --fastlane <versionCode>` (writes `CHANGELOG.md`, removes the used fragments, and writes the store notes), and updates the F-Droid recipe versions in `docs/fdroid.md` if needed. Merge it.
2. Tag that merge commit on `main` and push the tag:

   ```powershell
   git tag -a v0.1.0 -m "Mailcrane 0.1.0"
   ```

   ```powershell
   git push origin v0.1.0
   ```

3. Watch Actions → Release. When the build job has passed, approve the sign job.
4. Open the draft release, read the notes, check that the APK, AAB and `SHA256SUMS` are attached, and publish it. Tags with a `-` are marked as pre-releases.
5. Optional check on your own computer: `sha256sum -c SHA256SUMS`, and `apksigner verify --print-certs mailcrane-0.1.0.apk` shows the digest from step 5 of the setup.

If the release job fails, fix the cause on `main` and tag again with a new PATCH, or delete the tag and the draft and push the tag again. Re-running a failed sign job replaces the files on the existing draft and keeps its text.

## Nightly test builds

[.github/workflows/nightly.yml](../.github/workflows/nightly.yml) (#425) makes an unofficial test build of `main` every night at 02:30 UTC, and on **Run workflow** (Actions → Nightly test build). No agent publishes anything: GitHub Actions does it, once you've done the setup below.

1. **check** skips the night if `main` hasn't changed since the newest `test-build-*` release.
2. **build** is the build job of `release.yml`, called with `build_only`: the same `cargo xtask ci --strict` and Gradle release build as a release.
3. **sign** runs in the `nightly` environment with the **development key** that the earlier test builds used, never the release key. It only downloads the APK, then zipaligns and signs it. Without the secrets it prints "No test build published" and stops.
4. **publish** (no secrets, `main` only) makes a pre-release named `test-build-<YYYYMMDD>-nightly` with the APK, its SHA-256 and the commits since the last test build. It's never a `v*` tag, a normal release or "latest". Then it deletes test builds beyond the newest 7 (`scripts/prune_test_builds.py`, which can only delete `test-build-*` pre-releases; `--dry-run` lists what it would delete).

A development-key build installs over earlier test builds, not over a store or release version.

### Setup (maintainer, once)

1. **Settings → Environments → New environment**, named `nightly`. Under **Deployment branches and tags**, choose **Selected branches and tags** and add `main`. No required reviewers: that's the point of a nightly. The first run on `main` creates the environment automatically if it doesn't exist, without the branch rule, so add the rule either way.
2. Add these environment secrets for the development keystore (the standard Android debug keystore, alias `androiddebugkey`; QA can tell you which file the earlier test builds used):
   - `NIGHTLY_KEYSTORE_B64`: the keystore file, base64-encoded (`[Convert]::ToBase64String([IO.File]::ReadAllBytes("debug.keystore"))` in PowerShell);
   - `NIGHTLY_KEYSTORE_PASSWORD` and `NIGHTLY_KEY_PASSWORD`: `android` for the standard debug keystore;
   - `NIGHTLY_KEY_ALIAS`: `androiddebugkey`.
3. Start **Run workflow** once on `main` and check that a `test-build-…-nightly` pre-release appears.

To stop nightly builds, disable the workflow (Actions → Nightly test build → ⋯ → Disable workflow) or delete the environment's secrets.

## Checklist

- [ ] Version PR merged; `cargo xtask release-check <tag>` passes on `main`
- [ ] Tag pushed from `main`
- [ ] Build job green, sign job approved and green ("Signed and verified.")
- [ ] Certificate digest in the log matches the recorded one
- [ ] Draft release has the APK, AAB, `SHA256SUMS` and readable notes; published
- [ ] Store uploads (Play internal track, F-Droid recipe), once #72 and #71 are set up
