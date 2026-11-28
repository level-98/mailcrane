#!/usr/bin/env sh
# Runs one heavy command (Gradle, or cargo build/test/clippy/xtask ci) at a time
# on this machine (#602). The Git Bash twin of scripts/fleet/heavy.ps1: on
# Windows it hands over to heavy.ps1, so both share the same machine-wide
# mutex; elsewhere it takes an flock on a lock file. See heavy.ps1 for details.
#
# Usage: scripts/fleet/heavy.sh ./gradlew ktlintCheck
set -eu

if [ "$#" -eq 0 ]; then
    echo "usage: scripts/fleet/heavy.sh <command...>" >&2
    exit 2
fi

here=$(cd "$(dirname "$0")" && pwd)
if command -v powershell.exe >/dev/null 2>&1; then
    script=$here/heavy.ps1
    if command -v cygpath >/dev/null 2>&1; then script=$(cygpath -w "$script"); fi
    exec powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$script" "$@"
fi

lock=${XDG_RUNTIME_DIR:-/tmp}/mailcrane-heavy-job.lock
exec 9>"$lock"
if ! flock -n 9; then
    echo "heavy: waiting for another run"
    flock 9
fi

code=0
"$@" || code=$?

for arg in "$@"; do
    case "$arg" in
        *gradlew*)
            # Daemons keep their memory after the build (rule 52).
            "$arg" --stop >/dev/null 2>&1 || true
            break
            ;;
    esac
done
exit "$code"
