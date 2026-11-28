<#
.SYNOPSIS
Tests heavy.ps1 and heavy-guard.ps1 (#602): two runs started at once take turns, the guard refuses bare
heavy commands and lets the rest through, and a killed run leaves no stale lock.

.DESCRIPTION
Uses short sleeps, never a real build. Exits 1 on the first failure.
#>
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$heavy = Join-Path $here 'heavy.ps1'
$failed = $false
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "ok   $what" } else { Write-Host "FAIL $what"; $script:failed = $true }
}

# 1. Two runs started at once run one after the other.
$log = Join-Path ([IO.Path]::GetTempPath()) "heavy-test-$PID.log"
Remove-Item -Force $log -ErrorAction SilentlyContinue
# A step script, so no command text has to survive Start-Process's argument joining.
$stepFile = Join-Path ([IO.Path]::GetTempPath()) "heavy-test-step-$PID.ps1"
@'
param($Name, $Log)
Add-Content -Path $Log -Value ("start $Name " + [DateTime]::UtcNow.Ticks)
Start-Sleep -Milliseconds 1500
Add-Content -Path $Log -Value ("end $Name " + [DateTime]::UtcNow.Ticks)
'@ | Set-Content -Encoding utf8 $stepFile
$runs = foreach ($name in 'a', 'b') {
    Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $heavy,
        'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $stepFile, $name, $log
    )
}
$runs | Wait-Process -Timeout 60
Remove-Item -Force $stepFile -ErrorAction SilentlyContinue
$events = Get-Content $log | ForEach-Object { $kind, $name, $ticks = $_ -split ' '; [pscustomobject]@{ Kind = $kind; Name = $name; Ticks = [long]$ticks } } |
    Sort-Object Ticks
$order = ($events | ForEach-Object { $_.Kind }) -join ','
Check ($order -eq 'start,end,start,end') "two runs at once take turns (saw $order)"
Remove-Item -Force $log -ErrorAction SilentlyContinue

# 1b. The wrapped command's own options pass through untouched (-p, -Command).
$out = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $heavy powershell.exe -NoProfile -Command 'Write-Output passed-through'
Check ($LASTEXITCODE -eq 0 -and "$out" -match 'passed-through') "options like -Command pass through to the wrapped command"

# 2. The guard refuses bare heavy commands and lets the rest through.
$env:MAILCRANE_HEAVY_GUARD_LIB = '1'
. (Join-Path $here 'heavy-guard.ps1')
Remove-Item Env:\MAILCRANE_HEAVY_GUARD_LIB
foreach ($case in @(
        @{ Cmd = './gradlew ktlintCheck'; Heavy = $true },
        @{ Cmd = 'cd android && ./gradlew lintDebug'; Heavy = $true },
        @{ Cmd = 'timeout 900 cargo test -p mailcrane-store'; Heavy = $true },
        @{ Cmd = 'cargo clippy --all-targets -- -D warnings'; Heavy = $true },
        @{ Cmd = 'cargo xtask ci'; Heavy = $true },
        @{ Cmd = 'RUST_LOG=debug cargo build'; Heavy = $true },
        @{ Cmd = '& .\gradlew.bat assembleDebug'; Heavy = $true },
        @{ Cmd = 'scripts/fleet/heavy.ps1 ./gradlew ktlintCheck'; Heavy = $false },
        @{ Cmd = 'cd android; scripts/fleet/heavy.sh cargo test -p x'; Heavy = $false },
        @{ Cmd = 'cargo fmt --all --check'; Heavy = $false },
        @{ Cmd = 'cargo check -p mailcrane-core'; Heavy = $false },
        @{ Cmd = 'sh scripts/fleet/prepush.sh'; Heavy = $false },
        @{ Cmd = "git commit -m `"fix: cargo test passes; ./gradlew ktlintCheck too`""; Heavy = $false },
        @{ Cmd = "gh pr create --body 'Ran cargo clippy && cargo test'"; Heavy = $false }
    )) {
    Check ((Test-HeavyCommand $case.Cmd) -eq $case.Heavy) "guard: '$($case.Cmd)' heavy=$($case.Heavy)"
}

# The hook end to end: JSON on stdin, exit 2 to refuse. In Windows PowerShell a native program's stderr
# is an error under 'Stop', and the refusal message is on stderr by design.
$ErrorActionPreference = 'Continue'
$guard = Join-Path $here 'heavy-guard.ps1'
$refuse = '{"tool_name":"Bash","tool_input":{"command":"./gradlew ktlintCheck"}}' |
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File $guard 2>$null
Check ($LASTEXITCODE -eq 2) 'hook: a bare ./gradlew is refused (exit 2)'
$allow = '{"tool_name":"Bash","tool_input":{"command":"scripts/fleet/heavy.ps1 ./gradlew ktlintCheck"}}' |
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File $guard 2>$null
Check ($LASTEXITCODE -eq 0) 'hook: the wrapped command runs (exit 0)'

# 3. A killed run leaves no stale lock: the next run gets the mutex at once.
$holder = Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $heavy,
    'powershell.exe', '-NoProfile', '-Command', 'Start-Sleep -Seconds 60'
)
Start-Sleep -Seconds 3
# Kill the wrapper first, as a crash or a closed window would, so it never releases the mutex; then its
# child.
$children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($holder.Id)")
Stop-Process -Id $holder.Id -Force
$children | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
$started = Get-Date
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $heavy powershell.exe -NoProfile -Command 'exit 0'
$waited = ((Get-Date) - $started).TotalSeconds
Check ($LASTEXITCODE -eq 0 -and $waited -lt 20) "a killed run's lock is taken over at once (waited $([int]$waited) s)"

if ($failed) { exit 1 }
Write-Host 'all heavy.ps1 tests passed'
exit 0
