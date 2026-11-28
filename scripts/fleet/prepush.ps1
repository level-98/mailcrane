<#
.SYNOPSIS
The light check every dev seat runs before each push (#564, docs/fleet.md).

.DESCRIPTION
Runs ktlint on the Kotlin files the branch changes, and `cargo fmt --check` when Rust changed. No Gradle
and no cargo build, so it runs in seconds and needs no heavy-job lock. scripts/fleet/prepush.sh is the
POSIX twin.

ktlint is the standalone CLI at the same version as the Gradle plugin's (the ktlint block in
android/app/build.gradle.kts). `cargo xtask ci` checks that the three pins match. It is downloaded once
from Maven Central into %LOCALAPPDATA%\mailcrane\tools and checked against its SHA-256 before every run.

.EXAMPLE
scripts/fleet/prepush.ps1

.EXAMPLE
scripts/fleet/prepush.ps1 -Base origin/main
#>
[CmdletBinding()]
param(
    # What the branch is compared with.
    [string]$Base = 'origin/main'
)
$ErrorActionPreference = 'Stop'

$KtlintVersion = '1.5.0'
$KtlintSha256 = 'a27854622198800f50971a049dcdba38f2105d47e7e7258786c7d28045c5735d'
$KtlintUrl = "https://repo1.maven.org/maven2/com/pinterest/ktlint/ktlint-cli/$KtlintVersion/ktlint-cli-$KtlintVersion-all.jar"

$root = (git rev-parse --show-toplevel).Trim()
Set-Location $root

# The branch's changes against the base, plus anything not committed yet:
# changed files and new files git doesn't track yet (#596).
$changed = @(git diff --name-only --diff-filter=ACMR "$Base...HEAD") + @(git diff --name-only --diff-filter=ACMR HEAD) +
    @(git ls-files --others --exclude-standard) |
    Where-Object { $_ } | Sort-Object -Unique
$kotlin = @($changed | Where-Object { $_ -match '^android/.*\.kts?$' -and $_ -notmatch '/build/' })
$rust = @($changed | Where-Object { $_ -match '^core/' })
$failed = $false

function Get-Sha256([string]$Path) {
    (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

if ($kotlin.Count -gt 0) {
    $cache = Join-Path $env:LOCALAPPDATA 'mailcrane\tools'
    $jar = Join-Path $cache "ktlint-cli-$KtlintVersion-all.jar"
    if (-not (Test-Path $jar) -or (Get-Sha256 $jar) -ne $KtlintSha256) {
        New-Item -ItemType Directory -Force $cache | Out-Null
        Write-Host "prepush: downloading ktlint $KtlintVersion"
        $part = "$jar.part"
        Invoke-WebRequest -UseBasicParsing -Uri $KtlintUrl -OutFile $part
        if ((Get-Sha256 $part) -ne $KtlintSha256) {
            Remove-Item -Force $part
            Write-Error "prepush: the downloaded ktlint doesn't match its pinned SHA-256; not running it."
            exit 2
        }
        Move-Item -Force $part $jar
    }
    $java = 'java'
    if ($env:JAVA_HOME) {
        $java = Join-Path $env:JAVA_HOME 'bin\java.exe'
    } elseif (-not (Get-Command java -ErrorAction SilentlyContinue)) {
        # Android Studio's bundled runtime, where most seats have one.
        $studio = 'C:\Program Files\Android\Android Studio\jbr\bin\java.exe'
        if (Test-Path $studio) { $java = $studio }
    }
    & $java -jar $jar --relative @kotlin
    if ($LASTEXITCODE -eq 0) {
        Write-Host "prepush: ktlint ok ($($kotlin.Count) file(s))"
    } else {
        Write-Host 'prepush: ktlint failed. Fix the lines above (or run ktlint --format on them).'
        $failed = $true
    }
}

if ($rust.Count -gt 0) {
    Push-Location (Join-Path $root 'core')
    try {
        cargo fmt --all --check
        if ($LASTEXITCODE -eq 0) {
            Write-Host 'prepush: cargo fmt ok'
        } else {
            Write-Host "prepush: cargo fmt failed. Run 'cargo fmt --all' in core/."
            $failed = $true
        }
    } finally {
        Pop-Location
    }
}

if ($kotlin.Count -eq 0 -and $rust.Count -eq 0) {
    Write-Host "prepush: no Kotlin or Rust changes against $Base"
}
if ($failed) { exit 1 }
exit 0
