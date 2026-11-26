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
#>
[CmdletBinding()]
param(
    [string[]] $Seats = @('lead', 'dev1', 'dev2', 'review', 'qa'),
    [string] $PermissionMode = 'auto',
    [string] $Model = ''
)

$ErrorActionPreference = 'Stop'

$roles = @{
    lead   = 'mailcrane-lead'
    dev1   = 'mailcrane-dev'
    dev2   = 'mailcrane-dev'
    review = 'mailcrane-review'
    qa     = 'mailcrane-qa'
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

# 'powershell -File' passes '-Seats a,b' as one string; split it.
$Seats = @($Seats | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

foreach ($s in $Seats) {
    if (-not $roles.ContainsKey($s)) { Stop-Fleet "unknown seat '$s'. Seats: $($roles.Keys -join ', ')." }
}

$wiring = @('.mcp.json', '.synapse\config.json', '.claude\settings.local.json')
foreach ($f in $wiring) {
    if (-not (Test-Path (Join-Path $root $f))) { Stop-Fleet "$f is missing. Run 'synapse add-project' for this repository first." }
}

foreach ($tool in @('claude', 'gh', 'git')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Stop-Fleet "'$tool' is not on PATH." }
}

$login = (& gh api user --jq .login 2>$null)
if (-not $login) { Stop-Fleet "gh isn't signed in. Run 'gh auth login' first." }
Write-Host "Fleet will act on GitHub as $login."

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
        Copy-Item (Join-Path $root $f) $dest -Force
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
    if ($Model) { $claudeArgs += @('--model', $Model) }
    $argText = ($claudeArgs | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ' '

    $command = @"
`$host.UI.RawUI.WindowTitle = 'mailbot.$seat'
`$env:DEV_SYNC_SEAT = '$seat'
`$env:CARGO_TARGET_DIR = '$target'
claude $argText '$($loop -replace "'", "''")'
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    Start-Process powershell -WorkingDirectory $dir -ArgumentList @('-NoExit', '-EncodedCommand', $encoded) | Out-Null
    Write-Host "  started mailbot.$seat in $dir"
}

Write-Host ''
Write-Host 'First launch in each window: accept the folder trust, the "synapse" MCP server and the development-channels warning.'
Write-Host 'Close a window to stop that seat.'
