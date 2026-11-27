<#
.SYNOPSIS
Puts an issue or pull request on the Mailcrane project board and sets its fields.

.DESCRIPTION
See docs/fleet.md ("The project board"). The lead runs it on dispatch and at
hand-offs, so the board shows who has what. GitHub stays the record and the
Synapse board stays the live layer; this only mirrors the state onto the
project. Adding an item that's already on the board is harmless.

Uses your existing `gh` sign-in, which needs the `project` scope
(`gh auth refresh -s project`, done once by the maintainer). No secrets.

.EXAMPLE
scripts/fleet/project-sync.ps1 -Item 538 -Status 'In progress' -Seat dev3
Adds #538 (an issue or a pull request) to the board, in progress, held by dev3.

.EXAMPLE
scripts/fleet/project-sync.ps1 -Item 522 -Status 'In review' -Group G6 -Phase M4
#>
[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Mandatory)] [int] $Item,
    [ValidateSet('Backlog', 'Ready', 'In progress', 'In review', 'QA', 'Done')]
    [string] $Status,
    [ValidateSet('dev1', 'dev2', 'dev3', 'review', 'qa', 'lead', 'windows', 'human')]
    [string] $Seat,
    # Grouped dispatch, for example G6 (docs/fleet.md). '-' clears it.
    [string] $Group,
    [ValidateSet('M3', 'M4', 'M5', 'M6', 'M7', 'M8', 'post-1.0')]
    [string] $Phase,
    [ValidateSet('P1', 'P2', 'P3', 'P4')]
    [string] $Priority,
    [string] $Repo = 'level-98/mailcrane',
    [string] $Owner = 'level-98',
    [int] $Project = 1
)

$ErrorActionPreference = 'Stop'

function Stop-Sync([string] $Message) {
    Write-Host "project-sync: $Message" -ForegroundColor Red
    exit 1
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Stop-Sync "'gh' is not on PATH." }

# The issue API answers for pull requests too, with the pull request's URL.
$url = & gh api "repos/$Repo/issues/$Item" --jq .html_url 2>$null
if ($LASTEXITCODE -ne 0 -or -not $url) { Stop-Sync "#$Item not found in $Repo." }

& gh project item-add $Project --owner $Owner --url $url --format json --jq .id | Out-Null
if ($LASTEXITCODE -ne 0) {
    Stop-Sync "could not add $url to project $Project. Does gh have the 'project' scope? (gh auth status)"
}

# One field per call: gh edits a single field of an issue's item at a time.
$fields = [ordered]@{ Status = $Status; Seat = $Seat; Phase = $Phase; Priority = $Priority }
foreach ($name in $fields.Keys) {
    $value = $fields[$name]
    if (-not $value) { continue }
    & gh project item-edit $Project --owner $Owner --url $url --field $name --value $value | Out-Null
    if ($LASTEXITCODE -ne 0) { Stop-Sync "could not set $name to '$value' on #$Item." }
}
if ($Group -eq '-') {
    & gh project item-edit $Project --owner $Owner --url $url --field Group --clear | Out-Null
} elseif ($Group) {
    & gh project item-edit $Project --owner $Owner --url $url --field Group --text $Group | Out-Null
}
if ($LASTEXITCODE -ne 0) { Stop-Sync "could not set Group on #$Item." }

$set = @($fields.Keys | Where-Object { $fields[$_] } | ForEach-Object { "$_=$($fields[$_])" })
if ($Group) { $set += "Group=$Group" }
Write-Host "project-sync: #$Item on the board$(if ($set) { ': ' + ($set -join ', ') })"
