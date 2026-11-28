<#
.SYNOPSIS
Starts the Mailcrane agent fleet: one Claude Code window per seat, wired to Synapse.

.DESCRIPTION
See docs/fleet.md. For each seat this:
  - creates a git worktree under .worktrees/<seat> (the lead uses the main checkout),
  - copies this machine's Synapse wiring (.mcp.json, .synapse/config.json, .claude/settings.local.json) into it,
  - opens a PowerShell window running Claude Code as that seat's role, in /loop dynamic mode.

Every seat acts on GitHub as you, through your existing `gh` and git sign-in, and labels its PRs and issues
`fleet` (docs/fleet.md).

.EXAMPLE
scripts/fleet/start-fleet.ps1

.EXAMPLE
scripts/fleet/start-fleet.ps1 -Seats lead,dev1

.EXAMPLE
scripts/fleet/start-fleet.ps1 -Seats spec,docs,coach
Starts the spec writer, the docs seat and the coach alongside a running fleet.

.EXAMPLE
scripts/fleet/start-fleet.ps1 -Seats spec -DryRun
Checks everything and prints what it would start, without creating worktrees or opening windows.
#>
# No positional binding: under 'powershell -File', '-Seats lead, dev1' would
# otherwise bind the stray 'dev1' to -PermissionMode.
[CmdletBinding(PositionalBinding = $false)]
param(
    [string[]] $Seats = @('lead', 'dev1', 'dev2', 'dev3', 'review', 'qa'),
    [string] $PermissionMode = 'auto',
    [string] $Model = '',
    # Check and print the plan only: no fetch, no worktrees, no windows.
    [switch] $DryRun
)

$ErrorActionPreference = 'Stop'

$roles = @{
    lead   = 'mailcrane-lead'
    dev1   = 'mailcrane-dev'
    dev2   = 'mailcrane-dev'
    dev3   = 'mailcrane-dev'
    review = 'mailcrane-review'
    qa     = 'mailcrane-qa'
    # Not in the default -Seats (#582).
    spec   = 'mailcrane-spec'
    docs   = 'mailcrane-docs'
    coach  = 'mailcrane-coach'
}

# Per-seat default model (#507, #474 rule 32): emulator checks don't need
# the larger model. Seats not listed here use the account default (dev3
# moved to it with #595). -Model overrides every seat.
$seatModels = @{
    qa = 'sonnet'
}

function Get-SeatModel([string] $Seat) {
    if ($Model) { return $Model }
    return $seatModels[$Seat]
}

function Stop-Fleet([string] $Message) {
    Write-Host "start-fleet: $Message" -ForegroundColor Red
    exit 1
}

# --- Checks ------------------------------------------------------------------

$root = (& git rev-parse --show-toplevel 2>$null)
if (-not $root) { Stop-Fleet 'run this from inside the mailcrane repository.' }
$root = $root -replace '/', '\'
$common = (& git -C $root rev-parse --path-format=absolute --git-common-dir) -replace '/', '\'
if ((Split-Path $common -Parent) -ne $root) { Stop-Fleet "run this from the main checkout ($((Split-Path $common -Parent))), not a worktree." }

# 'powershell -File' passes '-Seats a,b' as one string; split it. Seat names
# are the Synapse identity and window name, so use them lower-cased, once each:
# 'Dev1' would be a different identity, and 'dev1,dev1' two windows sharing
# one worktree.
$Seats = @($Seats | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLowerInvariant() } | Where-Object { $_ } | Select-Object -Unique)
if ($Seats.Count -eq 0) { Stop-Fleet "no seats given. Seats: $($roles.Keys -join ', ')." }

foreach ($s in $Seats) {
    if (-not $roles.ContainsKey($s)) { Stop-Fleet "unknown seat '$s'. Seats: $($roles.Keys -join ', ')." }
}

$wiring = @('.mcp.json', '.synapse\config.json', '.claude\settings.local.json')
foreach ($f in $wiring) {
    if (-not (Test-Path (Join-Path $root $f))) { Stop-Fleet "$f is missing. Run 'synapse add-project' for this repository first." }
}

# The main checkout's settings for a worktree, minus Synapse's own hooks: the Synapse registry binds the project to the
# main checkout only, so they always refuse in a worktree and just print noise. Other hooks, the MCP server and env stay.
# Read and checked here, before any window opens.
function Get-WorktreeSettings([string] $Path) {
    try { $settings = Get-Content -Raw -Encoding UTF8 $Path | ConvertFrom-Json } catch { $settings = $null }
    if ($null -eq $settings -or $settings -isnot [System.Management.Automation.PSCustomObject]) {
        Stop-Fleet ".claude\settings.local.json isn't a JSON object. Run 'synapse add-project' for this repository again."
    }
    $hooks = $settings.PSObject.Properties['hooks']
    if ($hooks -and $hooks.Value -is [System.Management.Automation.PSCustomObject]) {
        foreach ($event in @($hooks.Value.PSObject.Properties.Name)) {
            $groups = @(foreach ($group in @($hooks.Value.$event)) {
                $kept = @(@($group.hooks) | Where-Object {
                    $_ -and -not (((@($_.command) + @($_.args)) -join ' ') -match '[\\/]\.synapse[\\/]app[\\/]hooks[\\/]')
                })
                if ($kept.Count -gt 0) { $group.hooks = $kept; $group }
            })
            if ($groups.Count -gt 0) { $hooks.Value.$event = $groups } else { $hooks.Value.PSObject.Properties.Remove($event) }
        }
        if (@($hooks.Value.PSObject.Properties).Count -eq 0) { $settings.PSObject.Properties.Remove('hooks') }
    }
    return ($settings | ConvertTo-Json -Depth 32)
}
$worktreeSettings = Get-WorktreeSettings (Join-Path $root '.claude\settings.local.json')

foreach ($tool in @('claude', 'gh', 'git')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Stop-Fleet "'$tool' is not on PATH." }
}

$login = (& gh api user --jq .login 2>$null)
if (-not $login) { Stop-Fleet "gh isn't signed in. Run 'gh auth login' first." }
Write-Host "Fleet will act on GitHub as $login."

# One compiler cache for every seat (#531), so a new worktree doesn't rebuild
# every dependency. Only when sccache is installed; otherwise nothing changes.
$sccacheDir = Join-Path $root '.worktrees\sccache'
$useSccache = [bool] (Get-Command sccache -ErrorAction SilentlyContinue)
if ($useSccache) {
    Write-Host "Seats will share sccache in $sccacheDir (up to 10G)."
} else {
    Write-Host 'sccache is not installed, so each seat builds its own dependencies. To share them: cargo install sccache --locked'
}

if ($DryRun) {
    foreach ($seat in $Seats) {
        $dir = if ($seat -eq 'lead') { $root } else { Join-Path $root ".worktrees\$seat" }
        $state = if ($seat -eq 'lead' -or (Test-Path $dir)) { 'exists' } else { 'would be created' }
        $seatModel = Get-SeatModel $seat
        $modelText = if ($seatModel) { $seatModel } else { 'account default' }
        Write-Host "  would start mailbot.$seat ($($roles[$seat]), model: $modelText) in $dir ($state)"
    }
    exit 0
}

# --- Worktrees ---------------------------------------------------------------

& git -C $root fetch origin --quiet
if ($LASTEXITCODE -ne 0) { Stop-Fleet 'git fetch origin failed.' }

function Initialize-Worktree([string] $Seat) {
    $wt = Join-Path $root ".worktrees\$Seat"
    if (-not (Test-Path $wt)) {
        & git -C $root worktree add --detach $wt origin/main --quiet
        if ($LASTEXITCODE -ne 0) { Stop-Fleet "could not create the worktree for $Seat." }
        Write-Host "  created worktree .worktrees\$Seat"
    }
    foreach ($f in $wiring) {
        $dest = Join-Path $wt $f
        New-Item -ItemType Directory -Force (Split-Path $dest -Parent) | Out-Null
        if ($f -eq '.claude\settings.local.json') {
            # Without Synapse's hooks (see Get-WorktreeSettings).
            [IO.File]::WriteAllText($dest, $worktreeSettings, (New-Object Text.UTF8Encoding $false))
        } else {
            Copy-Item (Join-Path $root $f) $dest -Force
        }
    }
    return $wt
}

# --- Launch ------------------------------------------------------------------

$loop = '/loop Follow your role instructions and docs/fleet.md: do one wake-up now, then pace yourself with ScheduleWakeup.'

foreach ($seat in $Seats) {
    $dir = if ($seat -eq 'lead') { $root } else { Initialize-Worktree $seat }
    $target = Join-Path $root ".worktrees\target-$seat"

    # --dangerously-load-development-channels takes a list of values, so it
    # would swallow the /loop prompt after it and claude would exit. It comes
    # first, and an ordinary option always follows it to end the list.
    $claudeArgs = @(
        '--dangerously-load-development-channels', 'server:synapse',
        '--agent', $roles[$seat],
        '-n', "mailbot.$seat",
        '--permission-mode', $PermissionMode
    )
    $seatModel = Get-SeatModel $seat
    if ($seatModel) { $claudeArgs += @('--model', $seatModel) }
    $argText = ($claudeArgs | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ' '

    # Every value goes into the window's script single-quoted, so escape '.
    $seatText = $seat -replace "'", "''"
    $targetText = $target -replace "'", "''"
    $cacheText = if ($useSccache) {
        "`$env:RUSTC_WRAPPER = 'sccache'; `$env:SCCACHE_DIR = '$($sccacheDir -replace "'", "''")'; `$env:SCCACHE_CACHE_SIZE = '10G'"
    } else { '' }
    $command = @"
`$host.UI.RawUI.WindowTitle = 'mailbot.$seatText'
`$env:DEV_SYNC_SEAT = '$seatText'
`$env:CARGO_TARGET_DIR = '$targetText'
$cacheText
claude $argText '$($loop -replace "'", "''")'
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    Start-Process powershell -WorkingDirectory $dir -ArgumentList @('-NoExit', '-EncodedCommand', $encoded) | Out-Null
    Write-Host "  started mailbot.$seat in $dir"
}

Write-Host ''
Write-Host 'First launch in each window: accept the folder trust, the "synapse" MCP server and the development-channels warning.'
Write-Host 'Close a window to stop that seat.'
