<#
.SYNOPSIS
Tests disk-sweep.ps1's cargo rules on a fixture (#615): it keeps the 8 newest
of each crate or target and anything under 24 hours, and never touches a
third-party crate or a target that isn't the workspace's.

.EXAMPLE
scripts/fleet/disk-sweep.test.ps1
Prints "ok" and exits 0, or the failed checks and exits 1.
#>
$sweep = Join-Path $PSScriptRoot 'disk-sweep.ps1'
$fixture = Join-Path ([IO.Path]::GetTempPath()) "disk-sweep-test-$PID"
$failed = 0

function Assert-Equal($Expected, $Actual, [string] $What) {
    if ("$Expected" -ne "$Actual") {
        Write-Host "FAIL: $What`: expected $Expected, got $Actual" -ForegroundColor Red
        $script:failed++
    }
}

# A hash directory with one file, last written $HoursAgo hours ago.
function New-Unit([string] $Path, [double] $HoursAgo) {
    New-Item -ItemType Directory -Force $Path | Out-Null
    $file = Join-Path $Path 'output'
    Set-Content -LiteralPath $file -Value ('x' * 1024) -Encoding ascii
    $when = (Get-Date).AddHours(-$HoursAgo)
    (Get-Item -LiteralPath $file).LastWriteTime = $when
    (Get-Item -LiteralPath $Path).LastWriteTime = $when
}

function Get-Count([string] $Path) { @(Get-ChildItem -LiteralPath $Path -Directory -ErrorAction SilentlyContinue).Count }

try {
    $debug = Join-Path $fixture 'target-seat\debug'
    $build = Join-Path $debug 'build'
    $inc = Join-Path $debug 'incremental'
    # Ours: 12 old (2 to 13 days) and 2 new (1 and 2 hours).
    for ($i = 0; $i -lt 12; $i++) { New-Unit (Join-Path $build ("mailcrane-x\{0:x16}" -f (100 + $i))) (48 + 24 * $i) }
    New-Unit (Join-Path $build 'mailcrane-x\00000000000000aa') 1
    New-Unit (Join-Path $build 'mailcrane-x\00000000000000ab') 2
    # Ours, but only 5 hash directories, all old: fewer than 8, so all kept.
    for ($i = 0; $i -lt 5; $i++) { New-Unit (Join-Path $build ("mailcrane-y\{0:x16}" -f (200 + $i))) 100 }
    # Third-party: 12 old ones, never touched.
    for ($i = 0; $i -lt 12; $i++) { New-Unit (Join-Path $build ("serde\{0:x16}" -f (300 + $i))) 500 }
    # Incremental: our target (10 old), and a target that isn't ours (10 old).
    for ($i = 0; $i -lt 10; $i++) { New-Unit (Join-Path $inc ("mailcrane_x-{0}abcdefghij" -f $i)) (30 + $i) }
    for ($i = 0; $i -lt 10; $i++) { New-Unit (Join-Path $inc ("other_target-{0}abcdefghij" -f $i)) 300 }
    # A cross build is swept too.
    for ($i = 0; $i -lt 10; $i++) { New-Unit (Join-Path $fixture ("target-seat\aarch64-linux-android\debug\build\mailcrane-x\{0:x16}" -f (400 + $i))) 72 }

    $crates = 'mailcrane-x,mailcrane-y'

    # A dry run deletes nothing.
    $dry = & powershell -NoProfile -File $sweep -WorktreesDir $fixture -Crates $crates -Targets mailcrane_x -CargoOnly
    Assert-Equal 14 (Get-Count (Join-Path $build 'mailcrane-x')) 'dry run keeps everything'
    Assert-Equal $true (($dry -join "`n") -match 'cargo build/\s+[\d.,]+ GB\s+\(8 items\)') 'dry run counts 6 + 2 build dirs'

    # -Apply without -HeavyJobHeld refuses.
    & powershell -NoProfile -File $sweep -WorktreesDir $fixture -Crates $crates -Targets mailcrane_x -CargoOnly -Apply | Out-Null
    Assert-Equal 1 $LASTEXITCODE '-Apply needs -HeavyJobHeld'
    Assert-Equal 14 (Get-Count (Join-Path $build 'mailcrane-x')) 'refused -Apply deletes nothing'

    & powershell -NoProfile -File $sweep -WorktreesDir $fixture -Crates $crates -Targets mailcrane_x -CargoOnly -Apply -HeavyJobHeld | Out-Null
    Assert-Equal 0 $LASTEXITCODE '-Apply runs'
    # 14 of ours: the 8 newest stay (2 new + 6 old), 6 old go.
    Assert-Equal 8 (Get-Count (Join-Path $build 'mailcrane-x')) 'keeps the 8 newest of a crate'
    Assert-Equal $true (Test-Path (Join-Path $build 'mailcrane-x\00000000000000aa')) 'keeps the newest'
    Assert-Equal $false (Test-Path (Join-Path $build ('mailcrane-x\{0:x16}' -f 111))) 'deletes the oldest'
    Assert-Equal 5 (Get-Count (Join-Path $build 'mailcrane-y')) 'fewer than 8: all kept'
    Assert-Equal 12 (Get-Count (Join-Path $build 'serde')) 'third-party crate untouched'
    $ours = @(Get-ChildItem -LiteralPath $inc -Directory | Where-Object { $_.Name -like 'mailcrane_x-*' }).Count
    $theirs = @(Get-ChildItem -LiteralPath $inc -Directory | Where-Object { $_.Name -like 'other_target-*' }).Count
    Assert-Equal 8 $ours 'incremental: keeps the 8 newest of our target'
    Assert-Equal 10 $theirs 'incremental: another target untouched'
    Assert-Equal 8 (Get-Count (Join-Path $fixture 'target-seat\aarch64-linux-android\debug\build\mailcrane-x')) 'cross build swept the same way'

    # Nothing younger than 24 h goes, even past the newest 8.
    for ($i = 0; $i -lt 12; $i++) { New-Unit (Join-Path $build ("mailcrane-z\{0:x16}" -f (500 + $i))) (1 + $i) }
    & powershell -NoProfile -File $sweep -WorktreesDir $fixture -Crates mailcrane-z -Targets none -CargoOnly -Apply -HeavyJobHeld | Out-Null
    Assert-Equal 12 (Get-Count (Join-Path $build 'mailcrane-z')) 'under 24 h: all kept'
} finally {
    Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failed -gt 0) {
    Write-Host "$failed check(s) failed" -ForegroundColor Red
    exit 1
}
Write-Output 'ok'
