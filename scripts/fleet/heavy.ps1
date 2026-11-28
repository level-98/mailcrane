<#
.SYNOPSIS
Runs one heavy command (Gradle, or cargo build/test/clippy/xtask ci) at a time on this machine (#602).

.DESCRIPTION
Takes the machine-wide named mutex Global\mailcrane-heavy-job, runs the command, and releases the mutex
when it ends, even on failure or Ctrl+C. A run whose process was killed leaves the mutex abandoned, which
the next run takes over, so a lock is never left stale. While another run holds it, this waits and says
who has it (from a small info file next to the mutex), rather than failing.

After a Gradle command it runs `gradlew --stop`, so no daemon keeps its memory (docs/fleet.md rule 52).

The Synapse lock locks/heavy-job is still taken too, so other seats can see the queue (docs/fleet.md).
A PreToolUse hook (scripts/fleet/heavy-guard.ps1, .claude/settings.json) refuses heavy commands that
don't go through this wrapper. scripts/fleet/heavy.sh is the Git Bash twin and shares the same mutex.

A second slot for targeted cargo runs (cargo test/clippy/build with -p or --package) is available
behind MAILCRANE_HEAVY_CARGO_SLOT=1. It is off by default: until it's turned on, every heavy command
shares the one mutex.

.EXAMPLE
scripts/fleet/heavy.ps1 ./gradlew ktlintCheck

.EXAMPLE
scripts/fleet/heavy.ps1 cargo test -p mailcrane-store
#>
# No param block: every argument is passed through as it is, so the wrapped command's own options
# (`-p`, `-Command`, `--release`) are never taken as this script's.
$ErrorActionPreference = 'Stop'
$Command = @($args)
if ($Command.Count -eq 0) {
    Write-Host 'usage: scripts/fleet/heavy.ps1 <command...>'
    exit 2
}

$exe = $Command[0]
$arguments = @($Command | Select-Object -Skip 1)
$isGradle = [bool]($Command | Where-Object { $_ -match 'gradlew' })
$isTargetedCargo = $exe -match '(^|[\\/])cargo(\.exe)?$' -and
    [bool]($arguments | Where-Object { $_ -eq '-p' -or $_ -like '--package*' })

$name = 'Global\mailcrane-heavy-job'
if ($isTargetedCargo -and $env:MAILCRANE_HEAVY_CARGO_SLOT -eq '1') {
    $name = 'Global\mailcrane-heavy-cargo'
}
$infoDir = Join-Path $env:LOCALAPPDATA 'mailcrane'
New-Item -ItemType Directory -Force $infoDir | Out-Null
$info = Join-Path $infoDir (($name -replace '^Global\\', '') + '.info')

$mutex = [System.Threading.Mutex]::new($false, $name)
$held = $false
$code = 1
try {
    while (-not $held) {
        try {
            $held = $mutex.WaitOne([TimeSpan]::FromSeconds(15))
        } catch [System.Threading.AbandonedMutexException] {
            # Its holder was killed: the mutex is ours now, and nothing is left stale.
            $held = $true
        }
        if (-not $held) {
            $who = if (Test-Path $info) { (Get-Content -Raw $info).Trim() } else { 'another run' }
            Write-Host "heavy: waiting for $who"
        }
    }
    $seat = if ($env:DEV_SYNC_SEAT) { $env:DEV_SYNC_SEAT } else { Split-Path -Leaf (Get-Location) }
    $label = ($Command -join ' ')
    if ($label.Length -gt 120) { $label = $label.Substring(0, 120) + '...' }
    "$seat (pid $PID) since $(Get-Date -Format HH:mm:ss): $label" | Set-Content -Encoding utf8 $info

    & $exe @arguments
    $code = $LASTEXITCODE
    if ($null -eq $code) { $code = 0 }

    if ($isGradle) {
        # Daemons keep their memory after the build (rule 52): stop them before letting go.
        $gradlew = $Command | Where-Object { $_ -match 'gradlew' } | Select-Object -First 1
        & $gradlew --stop | Out-Null
    }
} finally {
    if ($held) {
        Remove-Item -Force $info -ErrorAction SilentlyContinue
        $mutex.ReleaseMutex()
    }
    $mutex.Dispose()
}
exit $code
