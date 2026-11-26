<#
.SYNOPSIS
Starts the Mailcrane agent fleet: one Claude Code window per seat, wired to Synapse and the bot account.

.DESCRIPTION
See docs/fleet.md. For each seat this:
  - creates a git worktree under .worktrees/<seat> (the lead uses the main checkout),
  - copies this machine's Synapse wiring (.mcp.json, .synapse/config.json, .claude/settings.local.json) into it,
  - points the worktree's git at the bot (commit name and email, pushes through `gh` with the bot token),
  - opens a PowerShell window running Claude Code as that seat's role, in /loop dynamic mode.

The bot token is read from the MAILCRANE_BOT_GH_TOKEN environment variable inside each new window. It is never
written to disk, put on a command line or printed by this script.

.EXAMPLE
scripts/fleet/start-fleet.ps1 -BotLogin mailcrane-bot -BotUserId 123456789

.EXAMPLE
scripts/fleet/start-fleet.ps1 -BotLogin mailcrane-bot -BotUserId 123456789 -Seats lead,dev1
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $BotLogin,
    [Parameter(Mandatory = $true)] [string] $BotUserId,
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

foreach ($s in $Seats) {
    if (-not $roles.ContainsKey($s)) { Stop-Fleet "unknown seat '$s'. Seats: $($roles.Keys -join ', ')." }
}

if (-not $env:MAILCRANE_BOT_GH_TOKEN) {
    Stop-Fleet 'MAILCRANE_BOT_GH_TOKEN is not set in this terminal. Set it for your user (see docs/fleet.md) and open a new terminal.'
}

$wiring = @('.mcp.json', '.synapse\config.json', '.claude\settings.local.json')
foreach ($f in $wiring) {
    if (-not (Test-Path (Join-Path $root $f))) { Stop-Fleet "$f is missing. Run 'synapse add-project' for this repository first." }
}

foreach ($tool in @('claude', 'gh', 'git')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Stop-Fleet "'$tool' is not on PATH." }
}

# Confirm the token belongs to the bot, without printing it.
$env:GH_TOKEN = $env:MAILCRANE_BOT_GH_TOKEN
$login = (& gh api user --jq .login 2>$null)
Remove-Item Env:GH_TOKEN
if ($login -ne $BotLogin) { Stop-Fleet "MAILCRANE_BOT_GH_TOKEN signs in as '$login', not '$BotLogin'." }

# --- Worktrees ---------------------------------------------------------------

& git -C $root config extensions.worktreeConfig true
& git -C $root fetch origin --quiet
if ($LASTEXITCODE -ne 0) { Stop-Fleet 'git fetch origin failed.' }

$botEmail = "$BotUserId+$BotLogin@users.noreply.github.com"

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
    & git -C $wt config --worktree user.name $BotLogin
    & git -C $wt config --worktree user.email $botEmail
    # An empty helper clears the system's credential manager, so pushes use only the bot token via gh.
    # ('""' because Windows PowerShell 5.1 drops a bare empty string argument.)
    & git -C $wt config --worktree --replace-all credential.helper '""'
    & git -C $wt config --worktree --add credential.helper '!gh auth git-credential'
    return $wt
}

# --- Launch ------------------------------------------------------------------

$loop = '/loop Follow your role instructions and docs/fleet.md: do one wake-up now, then pace yourself with ScheduleWakeup.'

foreach ($seat in $Seats) {
    $dir = if ($seat -eq 'lead') { $root } else { Initialize-Worktree $seat }
    $target = Join-Path $root ".worktrees\target-$seat"

    $claudeArgs = @(
        '--agent', $roles[$seat],
        '-n', "mailbot.$seat",
        '--permission-mode', $PermissionMode,
        '--dangerously-load-development-channels', 'server:synapse'
    )
    if ($Model) { $claudeArgs += @('--model', $Model) }
    $argText = ($claudeArgs | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ' '

    $command = @"
`$host.UI.RawUI.WindowTitle = 'mailbot.$seat'
`$env:DEV_SYNC_SEAT = '$seat'
`$env:GH_TOKEN = `$env:MAILCRANE_BOT_GH_TOKEN
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
