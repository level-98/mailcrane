#!/usr/bin/env sh
# The light check every dev seat runs before each push (#564, docs/fleet.md):
# ktlint on the Kotlin files the branch changes, and `cargo fmt --check` when
# Rust changed. No Gradle and no cargo build, so it runs in seconds and needs
# no heavy-job lock. scripts/fleet/prepush.ps1 is the PowerShell twin.
#
# ktlint is the standalone CLI at the same version as the Gradle plugin's
# (the ktlint block in android/app/build.gradle.kts). `cargo xtask ci` checks
# that the three pins match. It is downloaded once from Maven Central into a
# per-user cache and checked against its SHA-256 before every run.
#
# Usage: scripts/fleet/prepush.sh [base]   (base defaults to origin/main)
set -eu

KTLINT_VERSION=1.5.0
KTLINT_SHA256=a27854622198800f50971a049dcdba38f2105d47e7e7258786c7d28045c5735d
KTLINT_URL="https://repo1.maven.org/maven2/com/pinterest/ktlint/ktlint-cli/$KTLINT_VERSION/ktlint-cli-$KTLINT_VERSION-all.jar"

base=${1:-origin/main}
root=$(git rev-parse --show-toplevel)
cd "$root"

# The branch's changes against the base, plus anything not committed yet:
# changed files and new files git doesn't track yet (#596).
changed=$( {
    git diff --name-only --diff-filter=ACMR "$base"...HEAD
    git diff --name-only --diff-filter=ACMR HEAD
    git ls-files --others --exclude-standard
} | sort -u)

kotlin=$(printf '%s\n' "$changed" | grep -E '^android/.*\.kts?$' | grep -v '/build/' || true)
rust=$(printf '%s\n' "$changed" | grep -E '^core/' || true)
failed=0

if [ -n "$kotlin" ]; then
    cache=${XDG_CACHE_HOME:-$HOME/.cache}/mailcrane
    if [ -n "${LOCALAPPDATA:-}" ]; then cache=$LOCALAPPDATA/mailcrane/tools; fi
    jar=$cache/ktlint-cli-$KTLINT_VERSION-all.jar
    if command -v sha256sum >/dev/null 2>&1; then hasher="sha256sum"; else hasher="shasum -a 256"; fi
    # From stdin, so the output is only the hash (no escaped file name).
    sum() { $hasher < "$1" | cut -d' ' -f1; }
    if [ ! -f "$jar" ] || [ "$(sum "$jar")" != "$KTLINT_SHA256" ]; then
        mkdir -p "$cache"
        echo "prepush: downloading ktlint $KTLINT_VERSION"
        curl -sfL "$KTLINT_URL" -o "$jar.part"
        if [ "$(sum "$jar.part")" != "$KTLINT_SHA256" ]; then
            rm -f "$jar.part"
            echo "prepush: the downloaded ktlint doesn't match its pinned SHA-256; not running it." >&2
            exit 2
        fi
        mv "$jar.part" "$jar"
    fi
    java=java
    if [ -n "${JAVA_HOME:-}" ]; then
        java=$JAVA_HOME/bin/java
    elif ! command -v java >/dev/null 2>&1; then
        # Android Studio's bundled runtime, where most seats have one.
        for candidate in \
            "/c/Program Files/Android/Android Studio/jbr/bin/java.exe" \
            "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java"; do
            if [ -x "$candidate" ]; then java=$candidate; break; fi
        done
    fi
    # shellcheck disable=SC2086 # one path per word: repo paths have no spaces
    if "$java" -jar "$jar" --relative $kotlin; then
        echo "prepush: ktlint ok ($(printf '%s\n' "$kotlin" | wc -l | tr -d ' ') file(s))"
    else
        echo "prepush: ktlint failed. Fix the lines above (or run ktlint --format on them)." >&2
        failed=1
    fi
fi

if [ -n "$rust" ]; then
    if (cd core && cargo fmt --all --check); then
        echo "prepush: cargo fmt ok"
    else
        echo "prepush: cargo fmt failed. Run 'cargo fmt --all' in core/." >&2
        failed=1
    fi
fi

if [ -z "$kotlin" ] && [ -z "$rust" ]; then
    echo "prepush: no Kotlin or Rust changes against $base"
fi
exit $failed
