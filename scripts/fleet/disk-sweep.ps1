<#
.SYNOPSIS
Finds stale build output under .worktrees and, with -Apply, deletes it. A dry
run by default.

.DESCRIPTION
The seats' cargo target directories grow by about 20 GB a day each: every
source change leaves a new hash directory behind (#615). This reports, in GB
per category, and with -Apply deletes:

1. Cargo: in each .worktrees/target-*/<profile>/ (and <triple>/<profile>/),
   the hash directories under build/<crate>/ and incremental/ that belong to
   this workspace's own crates and targets (read from `cargo metadata`), and
   that nothing has written for over 24 hours, keeping the 8 newest of each
   crate or target. Third-party crates are never touched.
2. QA: *.apk files in .worktrees/qa-artifacts/<pr>-*/ whose PR is merged or
   closed, and main-* APKs older than the newest. Screenshots and logs stay.
3. Report only, never deleted: AVDs and system images, the Gradle caches,
   Temp, and qa-target.

-Apply needs `locks/heavy-job` (docs/fleet.md rule 11), so no build is
writing: pass -HeavyJobHeld to say you hold it, and the script also refuses
while a cargo, rustc or Java process is running. It never deletes outside
.worktrees/, and never a worktree, a branch or a source file.

.EXAMPLE
scripts/fleet/disk-sweep.ps1
What would go, in GB per category.

.EXAMPLE
scripts/fleet/disk-sweep.ps1 -Apply -HeavyJobHeld
Deletes it, while you hold locks/heavy-job.
#>
[CmdletBinding(PositionalBinding = $false)]
param(
    [switch] $Apply,
    # You hold locks/heavy-job (rule 11). Needed with -Apply.
    [switch] $HeavyJobHeld,
    # Hash directories younger than this are kept.
    [double] $KeepHours = 24,
    # How many of the newest hash directories of each crate or target are kept, whatever their age.
    [int] $KeepNewest = 8,
    # For the tests: another .worktrees folder, and the crate and target names to treat as the workspace's.
    [string] $WorktreesDir,
    [string[]] $Crates,
    [string[]] $Targets,
    # For the tests: no QA, report-only or process checks (they look at this machine, not the fixture).
    [switch] $CargoOnly
)

# Not 'Stop': in Windows PowerShell that turns any stderr line from a native
# tool into a terminating error. Exit codes are checked instead.

function Stop-Sweep([string] $Message) {
    Write-Host "disk-sweep: $Message" -ForegroundColor Red
    exit 1
}

function Format-GB([double] $Bytes) { '{0:n1} GB' -f ($Bytes / 1GB) }

function Get-Size([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return 0 }
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue |
            Measure-Object -Property Length -Sum).Sum
    if ($sum) { return [double]$sum } else { return 0 }
}

# When anything last wrote to a hash directory: the directory itself or its
# direct children (cargo rewrites fingerprints and outputs there).
function Get-LastWrite([IO.DirectoryInfo] $Dir) {
    $last = $Dir.LastWriteTimeUtc
    foreach ($child in (Get-ChildItem -LiteralPath $Dir.FullName -Force -ErrorAction SilentlyContinue)) {
        if ($child.LastWriteTimeUtc -gt $last) { $last = $child.LastWriteTimeUtc }
    }
    return $last
}

# Of a group of hash directories, the ones to delete: not among the newest
# $Keep, and older than $Cutoff.
function Select-Stale($Dirs, [int] $Keep, [datetime] $Cutoff) {
    $dated = @($Dirs | ForEach-Object { [pscustomobject]@{ dir = $_; last = Get-LastWrite $_ } } |
            Sort-Object last -Descending)
    return @($dated | Select-Object -Skip $Keep | Where-Object { $_.last -lt $Cutoff } | ForEach-Object { $_.dir })
}

# --- Where ---------------------------------------------------------------------

# 'powershell -File' passes '-Crates a,b' as one string; split it.
$Crates = @($Crates | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$Targets = @($Targets | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

if ($WorktreesDir) {
    $worktrees = (Resolve-Path -LiteralPath $WorktreesDir -ErrorAction SilentlyContinue).Path
    if (-not $worktrees) { Stop-Sweep "$WorktreesDir doesn't exist." }
    $root = $null
} else {
    $common = & git rev-parse --path-format=absolute --git-common-dir 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $common) { Stop-Sweep 'run this inside the mailcrane repository.' }
    $root = Split-Path ($common -replace '/', '\') -Parent
    $worktrees = Join-Path $root '.worktrees'
    if (-not (Test-Path -LiteralPath $worktrees)) { Stop-Sweep "$worktrees doesn't exist." }
}

# The workspace's crate names (build/<crate>/) and target names (incremental/<target>-<hash>).
if ($Crates.Count -eq 0 -or $Targets.Count -eq 0) {
    if (-not $root) { Stop-Sweep 'with -WorktreesDir, give -Crates and -Targets too.' }
    $meta = & cargo metadata --manifest-path (Join-Path $root 'core\Cargo.toml') --no-deps --offline --format-version 1 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $meta) { Stop-Sweep "cargo metadata failed, so the workspace's crates are unknown." }
    $packages = (($meta -join "`n") | ConvertFrom-Json).packages
    if ($Crates.Count -eq 0) { $Crates = @($packages | ForEach-Object { $_.name }) }
    if ($Targets.Count -eq 0) { $Targets = @($packages | ForEach-Object { $_.targets } | ForEach-Object { $_.name -replace '-', '_' }) }
}
if ($Crates.Count -eq 0 -or $Targets.Count -eq 0) { Stop-Sweep 'no workspace crates found.' }
$crateSet = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($c in $Crates) { [void]$crateSet.Add($c) }
$targetSet = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($t in $Targets) { [void]$targetSet.Add($t) }

# --- Before deleting -----------------------------------------------------------

if ($Apply) {
    if (-not $HeavyJobHeld) {
        Stop-Sweep '-Apply deletes build output: take locks/heavy-job first (docs/fleet.md rule 11), then add -HeavyJobHeld.'
    }
    if (-not $CargoOnly) {
        $busy = @(Get-Process -Name cargo, rustc, java -ErrorAction SilentlyContinue)
        if ($busy.Count -gt 0) {
            $names = ($busy | ForEach-Object { "$($_.ProcessName) ($($_.Id))" }) -join ', '
            Stop-Sweep "a build may be writing: $names. Wait for it (and run ./gradlew --stop), then try again."
        }
    }
}

$cutoff = [datetime]::UtcNow.AddHours(-$KeepHours)
$report = [ordered]@{}
$deleted = 0.0
$failures = 0

# Deletes (or only measures) $Dirs, counting them under $Category.
function Remove-Swept([string] $Category, $Items) {
    foreach ($item in $Items) {
        $full = $item.FullName
        # Never outside .worktrees, whatever a path contains.
        if (-not $full.StartsWith($worktrees + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { continue }
        $size = if ($item -is [IO.FileInfo]) { [double]$item.Length } else { Get-Size $full }
        if (-not $report.Contains($Category)) { $report[$Category] = [pscustomobject]@{ count = 0; bytes = 0.0 } }
        $report[$Category].count++
        $report[$Category].bytes += $size
        if ($Apply) {
            try {
                Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction Stop
                $script:deleted += $size
            } catch {
                $script:failures++
                Write-Verbose "couldn't delete $full`: $_"
            }
        }
    }
}

# --- 1. Cargo ------------------------------------------------------------------

$profiles = foreach ($target in (Get-ChildItem -LiteralPath $worktrees -Directory -Filter 'target-*' -ErrorAction SilentlyContinue)) {
    foreach ($p in 'debug', 'release') {
        $direct = Join-Path $target.FullName $p
        if (Test-Path -LiteralPath $direct) { $direct }
    }
    # Cross builds: <triple>/debug (cargo-ndk's aarch64-linux-android, x86_64-linux-android).
    foreach ($triple in (Get-ChildItem -LiteralPath $target.FullName -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*-*-*' })) {
        foreach ($p in 'debug', 'release') {
            $nested = Join-Path $triple.FullName $p
            if (Test-Path -LiteralPath $nested) { $nested }
        }
    }
}

foreach ($profile in $profiles) {
    # build/<crate>/<hash>/: only this workspace's crates.
    $build = Join-Path $profile 'build'
    foreach ($crate in (Get-ChildItem -LiteralPath $build -Directory -ErrorAction SilentlyContinue)) {
        if (-not $crateSet.Contains($crate.Name)) { continue }
        $hashes = @(Get-ChildItem -LiteralPath $crate.FullName -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match '^[0-9a-f]{16}$' })
        Remove-Swept 'cargo build/' (Select-Stale $hashes $KeepNewest $cutoff)
    }
    # incremental/<target>-<hash>/: only this workspace's targets, grouped by target.
    $incremental = Join-Path $profile 'incremental'
    $groups = Get-ChildItem -LiteralPath $incremental -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^(.+)-[0-9a-z]{10,16}$' -and $targetSet.Contains(($_.Name -replace '-[0-9a-z]{10,16}$', '')) } |
        Group-Object { $_.Name -replace '-[0-9a-z]{10,16}$', '' }
    foreach ($g in $groups) {
        Remove-Swept 'cargo incremental/' (Select-Stale $g.Group $KeepNewest $cutoff)
    }
}

# --- 2. QA APKs ------------------------------------------------------------------

if (-not $CargoOnly) {
    $qa = Join-Path $worktrees 'qa-artifacts'
    $prState = @{}
    $mainApks = @()
    foreach ($dir in (Get-ChildItem -LiteralPath $qa -Directory -ErrorAction SilentlyContinue)) {
        $apks = @(Get-ChildItem -LiteralPath $dir.FullName -File -Filter '*.apk' -ErrorAction SilentlyContinue)
        if ($apks.Count -eq 0) { continue }
        if ($dir.Name -like 'main-*') { $mainApks += $apks; continue }
        if ($dir.Name -notmatch '^(\d+)(-|$)') { continue }
        $pr = $Matches[1]
        if (-not $prState.ContainsKey($pr)) {
            $state = & gh pr view $pr --json state --jq .state 2>$null
            $prState[$pr] = if ($LASTEXITCODE -eq 0) { "$state".Trim() } else { 'UNKNOWN' }
        }
        if ($prState[$pr] -in 'MERGED', 'CLOSED') { Remove-Swept 'QA APKs (closed PRs)' $apks }
    }
    if ($mainApks.Count -gt 1) {
        Remove-Swept 'QA APKs (older main)' @($mainApks | Sort-Object LastWriteTimeUtc -Descending | Select-Object -Skip 1)
    }
}

# --- Report ----------------------------------------------------------------------

$verb = if ($Apply) { 'Deleted' } else { 'Would delete' }
Write-Output "disk-sweep: $verb (keeping the $KeepNewest newest of each, and anything under $KeepHours h):"
$total = 0.0
foreach ($k in $report.Keys) {
    Write-Output ('  {0,-24} {1,10}  ({2} items)' -f $k, (Format-GB $report[$k].bytes), $report[$k].count)
    $total += $report[$k].bytes
}
if ($report.Count -eq 0) { Write-Output '  nothing' }
Write-Output ('  {0,-24} {1,10}' -f 'total', (Format-GB $total))
if ($Apply -and $failures -gt 0) { Write-Output "  $failures item(s) couldn't be deleted (in use?); run again later." }

if (-not $CargoOnly) {
    Write-Output ''
    Write-Output 'Not touched (report only):'
    $sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } elseif ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
    $others = [ordered]@{
        'AVDs' = Join-Path $HOME '.android\avd'
        'system images' = Join-Path $sdk 'system-images'
        'Gradle caches' = Join-Path $HOME '.gradle\caches'
        'Temp' = [IO.Path]::GetTempPath()
        'qa-target' = Join-Path $worktrees 'qa-target'
    }
    foreach ($k in $others.Keys) {
        if (Test-Path -LiteralPath $others[$k]) { Write-Output ('  {0,-24} {1,10}' -f $k, (Format-GB (Get-Size $others[$k]))) }
    }
    $drive = Get-PSDrive -Name ([IO.Path]::GetPathRoot($worktrees).Substring(0, 1)) -ErrorAction SilentlyContinue
    if ($drive) { Write-Output ('  free on {0}:{1,23}' -f $drive.Name, (Format-GB $drive.Free)) }
}
