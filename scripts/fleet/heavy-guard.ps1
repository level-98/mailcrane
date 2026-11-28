<#
.SYNOPSIS
PreToolUse hook (#602): refuses a Bash or PowerShell command that runs Gradle or a heavy cargo command
without scripts/fleet/heavy.ps1 (or heavy.sh).

.DESCRIPTION
Claude Code passes the tool call as JSON on stdin. A command is heavy when one of its parts runs
`gradlew` (any path), or `cargo build`, `cargo test`, `cargo clippy` or `cargo xtask ci`. Parts are split
at `;`, `&&`, `||`, `|` and newlines outside quotes, and leading `timeout N`, `env` and `NAME=value` are
skipped, so a commit message or a PR body that mentions those commands isn't refused. A command that
goes through heavy.ps1 or heavy.sh passes. `cargo fmt`, `cargo check`, the prepush script and git are
never refused.

Exit code 2 refuses the call, with the reason on stderr for the agent to read; 0 lets it run.
#>
$ErrorActionPreference = 'Stop'

function Get-CommandParts([string]$Text) {
    $parts = [System.Collections.Generic.List[string]]::new()
    $current = [System.Text.StringBuilder]::new()
    $quote = [char]0
    for ($i = 0; $i -lt $Text.Length; $i++) {
        $c = $Text[$i]
        if ($quote -ne [char]0) {
            if ($c -eq $quote) { $quote = [char]0 }
            [void]$current.Append($c)
            continue
        }
        if ($c -eq "'" -or $c -eq '"') {
            $quote = $c
            [void]$current.Append($c)
            continue
        }
        if ($c -eq ';' -or $c -eq '&' -or $c -eq '|' -or $c -eq "`n" -or $c -eq "`r") {
            $parts.Add($current.ToString())
            [void]$current.Clear()
            continue
        }
        [void]$current.Append($c)
    }
    $parts.Add($current.ToString())
    return $parts
}

function Test-HeavyPart([string]$Part) {
    $words = @($Part.Trim() -split '\s+' | Where-Object { $_ })
    $i = 0
    while ($i -lt $words.Count) {
        $w = $words[$i]
        if ($w -eq 'timeout' -and ($i + 1) -lt $words.Count) { $i += 2; continue }
        if ($w -eq 'env' -or $w -eq '&' -or $w -match '^[A-Za-z_][A-Za-z0-9_]*=') { $i += 1; continue }
        break
    }
    if ($i -ge $words.Count) { return $false }
    $first = $words[$i].Trim("'", '"')
    if ($first -match 'heavy\.(ps1|sh)$') { return $false }
    if ($first -match '(^|[\\/])gradlew(\.bat)?$') { return $true }
    if ($first -match '(^|[\\/])cargo(\.exe)?$' -and ($i + 1) -lt $words.Count) {
        $sub = $words[$i + 1]
        if ($sub -in @('build', 'test', 'clippy')) { return $true }
        if ($sub -eq 'xtask' -and ($i + 2) -lt $words.Count -and $words[$i + 2] -eq 'ci') { return $true }
    }
    return $false
}

function Test-HeavyCommand([string]$Text) {
    foreach ($part in (Get-CommandParts $Text)) {
        if (Test-HeavyPart $part) { return $true }
    }
    return $false
}

# Tests call the functions above directly (scripts/fleet/test-heavy.ps1).
if ($env:MAILCRANE_HEAVY_GUARD_LIB -eq '1') { return }

$raw = [Console]::In.ReadToEnd()
if (-not $raw) { exit 0 }
try {
    $call = $raw | ConvertFrom-Json
} catch {
    exit 0
}
$text = $call.tool_input.command
if (-not $text) { exit 0 }
if (Test-HeavyCommand $text) {
    [Console]::Error.WriteLine(
        'Refused (#602, rule 59): Gradle and cargo build/test/clippy/xtask ci run only through the heavy-job ' +
        'wrapper, which lets one heavy command run at a time on this machine. Take locks/heavy-job, then ' +
        'run it as: scripts/fleet/heavy.ps1 <command...>   (Git Bash: scripts/fleet/heavy.sh <command...>). ' +
        'cargo fmt, cargo check and scripts/fleet/prepush are not affected.')
    exit 2
}
exit 0
