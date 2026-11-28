<#
.SYNOPSIS
Finds repeated friction in the fleet's pipeline over a time window and prints
it as short Markdown, repeated items first.

.DESCRIPTION
The lead runs it at every wake-up (#577) and turns each repeated item into a
rule in #474. It reports, for fleet pull requests in the window:

- CI failures by cause (ktlint, rustfmt, clippy, compile errors, test
  failures...), read from the failed job's log: the summary table that
  `cargo xtask ci` prints, then the Gradle task or Rust error that failed.
- Merges delayed by open threads: the review seat cleared the PR (a
  `**mailbot.review:**` comment saying no findings, no blockers or cleared),
  but its review threads were answered more than 10 minutes later.
- Commits pushed after the review cleared (fix rounds), and more than 2 CI
  rounds on one PR.
- /claim comments on issues that didn't have `agent-ready` at the time.
- Merge-queue ejections.

Read-only: it only reads from GitHub, through your `gh` sign-in, and never
writes there. The only file it writes is a small local cache of what it has
read from finished CI runs (their jobs and logs never change), so a repeat run
doesn't fetch them again. It makes a few REST and GraphQL calls plus one per
uncached failed run and one per uncached failed job, at most 4 at a time, well
within GitHub's rate limits (#474 rule 39 is about writes; this makes none).
The default window takes about 5 s; a day takes about 15 s once cached.

Limits: GitHub's API doesn't say when a review thread was resolved, so a
thread counts as closed at its last reply (rule 45: the reply and the resolve
go in the same push). A commit's time is its commit date, which a rebase
resets.

.EXAMPLE
scripts/fleet/retro.ps1
The last 3 hours.

.EXAMPLE
scripts/fleet/retro.ps1 -Since (Get-Date).Date
Today so far.
#>
[CmdletBinding(PositionalBinding = $false)]
param(
    # Hours to look back, when -Since isn't given.
    [double] $Hours = 3,
    # Start of the window (local time unless it says otherwise).
    [datetime] $Since,
    [string] $Repo = 'mailcrane/mailcrane',
    # Minutes a cleared PR may wait for its threads before it counts as delayed.
    [int] $ThreadMinutes = 10,
    # Don't read or write the local cache of finished CI runs.
    [switch] $NoCache
)

# Not 'Stop': in Windows PowerShell that turns any stderr line from gh into a
# terminating error. Exit codes are checked instead.

function Stop-Retro([string] $Message) {
    Write-Host "retro: $Message" -ForegroundColor Red
    exit 1
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Stop-Retro "'gh' is not on PATH." }

$started = Get-Date
$nowUtc = [datetime]::UtcNow
if ($PSBoundParameters.ContainsKey('Since')) { $fromUtc = $Since.ToUniversalTime() }
else { $fromUtc = $nowUtc.AddHours(-$Hours) }
$fromIso = $fromUtc.ToString('yyyy-MM-ddTHH:mm:ssZ')
$owner, $name = $Repo.Split('/', 2)
if (-not $name) { Stop-Retro "-Repo must be owner/name." }

function ConvertTo-Utc($Iso) {
    if (-not $Iso) { return $null }
    # Some ConvertFrom-Json versions already turn ISO strings into dates.
    if ($Iso -is [datetime]) { return $Iso.ToUniversalTime() }
    return [DateTimeOffset]::Parse([string]$Iso, [Globalization.CultureInfo]::InvariantCulture).UtcDateTime
}

function Format-Local([datetime] $Utc) { return $Utc.ToLocalTime().ToString('HH:mm') }

function Format-Prs($Numbers) {
    return (($Numbers | Sort-Object -Unique | ForEach-Object { "#$_" }) -join ', ')
}

# gh api, parsed from JSON. Windows PowerShell mangles double quotes in native
# arguments, so queries take their strings as variables and never quote.
function Invoke-GhJson([string[]] $Arguments) {
    $out = & gh api @Arguments 2>$null
    if ($LASTEXITCODE -ne 0) { Stop-Retro "gh api $($Arguments[0]) failed." }
    return ($out -join "`n") | ConvertFrom-Json
}

# --- Fleet pull requests touched in the window -----------------------------

$prQuery = @'
query($q: String!, $after: String) {
  search(query: $q, type: ISSUE, first: 25, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        number headRefName
        comments(last: 60) { nodes { createdAt body } }
        reviewThreads(first: 60) {
          nodes {
            isResolved
            opened: comments(first: 1) { nodes { createdAt } }
            answered: comments(last: 1) { nodes { createdAt } }
          }
        }
        commits(last: 60) { nodes { commit { abbreviatedOid committedDate parents { totalCount } } } }
        timelineItems(last: 30, itemTypes: [REMOVED_FROM_MERGE_QUEUE_EVENT]) {
          nodes { ... on RemovedFromMergeQueueEvent { createdAt reason } }
        }
      }
    }
  }
}
'@ -replace '\s+', ' '

$prs = @()
$cursor = $null
do {
    $ghArgs = @('graphql', '-f', "query=$prQuery", '-f', "q=repo:$Repo is:pr label:fleet updated:>=$fromIso")
    if ($cursor) { $ghArgs += @('-f', "after=$cursor") }
    $page = (Invoke-GhJson $ghArgs).data.search
    $prs += @($page.nodes | Where-Object { $_.number })
    $cursor = $page.pageInfo.endCursor
} while ($page.pageInfo.hasNextPage)

$prByBranch = @{}
foreach ($pr in $prs) { $prByBranch[$pr.headRefName] = $pr.number }

# --- CI runs on those PRs ---------------------------------------------------

# Runs gh api for each key at most 4 at a time; returns key -> output text
# ('' when the call failed).
$tmp = Join-Path ([IO.Path]::GetTempPath()) "mailcrane-retro-$PID"
function Invoke-GhParallel([hashtable] $Calls) {
    $results = @{}
    New-Item -ItemType Directory -Force $tmp | Out-Null
    $pending = New-Object System.Collections.Queue
    foreach ($k in $Calls.Keys) { $pending.Enqueue($k) }
    $running = @()
    $n = 0
    while ($pending.Count -gt 0 -or $running.Count -gt 0) {
        while ($pending.Count -gt 0 -and $running.Count -lt 4) {
            $k = $pending.Dequeue()
            $n++
            $file = Join-Path $tmp "$n.out"
            $proc = Start-Process gh -NoNewWindow -PassThru -RedirectStandardOutput $file `
                -RedirectStandardError "$file.err" -ArgumentList (@('api') + $Calls[$k])
            # Without reading the handle now, ExitCode stays empty after exit.
            $null = $proc.Handle
            $running += [pscustomobject]@{ proc = $proc; key = $k; file = $file }
        }
        $done = @($running | Where-Object { $_.proc.HasExited })
        if ($done.Count -eq 0) { Start-Sleep -Milliseconds 100; continue }
        foreach ($d in $done) {
            # The redirect handles close a moment after the exit is seen.
            $d.proc.WaitForExit()
            $text = ''
            for ($try = 0; $try -lt 20 -and (Test-Path $d.file); $try++) {
                try { $text = [IO.File]::ReadAllText($d.file); break }
                catch [IO.IOException] { Start-Sleep -Milliseconds 100 }
            }
            if ($d.proc.ExitCode -ne 0) { $text = '' }
            $results[$d.key] = $text
        }
        $running = @($running | Where-Object { -not $_.proc.HasExited })
    }
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    return $results
}

# Only the fields used, as tab-separated lines: whole run objects are large
# and slow to parse in Windows PowerShell.
function ConvertFrom-Rows([string[]] $Lines, [string[]] $Fields) {
    $rows = New-Object System.Collections.Generic.List[object]
    foreach ($line in $Lines) {
        if (-not $line) { continue }
        $cells = $line -split "`t"
        $row = [ordered]@{}
        for ($i = 0; $i -lt $Fields.Count; $i++) { $row[$Fields[$i]] = $cells[$i] }
        $rows.Add([pscustomobject]$row)
    }
    return $rows
}

function Get-GhRows([string] $Endpoint, [string] $Jq, [string[]] $Fields) {
    $rows = @()
    for ($page = 1; $page -le 10; $page++) {
        $out = @(& gh api "$Endpoint&per_page=100&page=$page" --jq $Jq 2>$null)
        if ($LASTEXITCODE -ne 0) { Stop-Retro "gh api $Endpoint failed." }
        $rows += @(ConvertFrom-Rows $out $Fields)
        if ($out.Count -lt 100) { break }
    }
    return $rows
}

# The runs API answers slowly, so all pages are fetched at once (at most 10).
$runsUrl = "repos/$Repo/actions/runs?created=%3E%3D$fromIso"
$total = & gh api "$runsUrl&per_page=1" --jq .total_count 2>$null
if ($LASTEXITCODE -ne 0) { Stop-Retro "gh api $runsUrl failed." }
$runJq = '.workflow_runs[] | [.id, .name, .event, .head_branch, .head_sha, .conclusion] | @tsv'
$runCalls = @{}
for ($page = 1; $page -le [Math]::Min(10, [Math]::Ceiling([int]$total / 100)); $page++) {
    # Start-Process joins its arguments with spaces, so the filter is quoted here.
    $runCalls["$page"] = @("$runsUrl&per_page=100&page=$page", '--jq', "`"$runJq`"")
}
$runPages = Invoke-GhParallel $runCalls
if (@($runPages.Values | Where-Object { -not $_ }).Count -gt 0) { Stop-Retro "gh api $runsUrl failed." }
$runs = @(ConvertFrom-Rows (@($runPages.Values) -join "`n" -split "`r?`n") @('id', 'name', 'event', 'head_branch', 'head_sha', 'conclusion'))

function Get-RunPr($Run) {
    if ($Run.event -eq 'merge_group' -and $Run.head_branch -match '/pr-(\d+)-') { return [int]$Matches[1] }
    if ($prByBranch.ContainsKey($Run.head_branch)) { return $prByBranch[$Run.head_branch] }
    return $null
}

$fleetRuns = @($runs | Where-Object { Get-RunPr $_ } | ForEach-Object {
        $_ | Add-Member -NotePropertyName pr -NotePropertyValue (Get-RunPr $_) -PassThru
    })

# What made a failed job fail, from its log (escape sequences stripped).
function Get-LogCauses([string] $Log) {
    $causes = New-Object System.Collections.Generic.List[string]
    $compile = $Log -match 'error\[E\d{4}\]'
    $summary = [regex]::Matches($Log, '(?m)^\S*\s+(design|fmt|lint|test|deny|android)\s+FAILED')
    foreach ($m in $summary) {
        switch ($m.Groups[1].Value) {
            'design' { $causes.Add('design check') }
            'fmt' { $causes.Add('rustfmt') }
            'deny' { $causes.Add('cargo deny / licences') }
            'lint' {
                if ($compile) { $causes.Add('Rust compile error') }
                elseif ($Log -match '(?m)^\S*\s+error: |clippy::') { $causes.Add('clippy') }
                else { $causes.Add('lint (strings or release guard)') }
            }
            'test' {
                if ($compile) { $causes.Add('Rust compile error') }
                else { $causes.Add('Rust test failure') }
            }
            'android' {
                $before = $causes.Count
                if ($Log -match 'Task :\S*ktlint\S* FAILED') { $causes.Add('ktlint') }
                if ($Log -match 'Task :\S*(compile|ksp)\S*Kotlin FAILED') { $causes.Add('Kotlin compile error') }
                if ($Log -match 'Task :\S*:lint\w* FAILED') { $causes.Add('Android lint') }
                if ($Log -match 'Task :\S*UnitTest FAILED') { $causes.Add('Android unit test') }
                if ($causes.Count -eq $before) { $causes.Add('android build (other)') }
            }
        }
    }
    return @($causes | Select-Object -Unique)
}

# Finished runs and their jobs never change, so what was read from them is
# cached: "run:<id>" is a run's failed jobs, "job:<id>" a job's causes.
$cacheFile = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'mailcrane\retro-cache.json' }
else { Join-Path $HOME '.cache/mailcrane/retro-cache.json' }
$cache = @{}
if (-not $NoCache -and (Test-Path $cacheFile)) {
    try {
        $saved = Get-Content $cacheFile -Raw | ConvertFrom-Json
        foreach ($p in $saved.PSObject.Properties) { $cache[$p.Name] = @($p.Value | ForEach-Object { $_ }) }
    } catch { $cache = @{} }
}

# Failed jobs of each failed run (ci-ok only repeats the others' result, so it
# counts only when it failed alone).
$failedRuns = @($fleetRuns | Where-Object { $_.conclusion -in 'failure', 'timed_out' })
$jobCalls = @{}
foreach ($run in $failedRuns) {
    if (-not $cache.ContainsKey("run:$($run.id)")) { $jobCalls["$($run.id)"] = @("repos/$Repo/actions/runs/$($run.id)/jobs?per_page=100") }
}
$jobLists = Invoke-GhParallel $jobCalls
foreach ($id in $jobLists.Keys) {
    if (-not $jobLists[$id]) { continue }
    $jobs = @(($jobLists[$id] | ConvertFrom-Json).jobs | Where-Object { $_.conclusion -in 'failure', 'timed_out', 'cancelled' })
    $real = @($jobs | Where-Object { $_.name -ne 'ci-ok' })
    if ($real.Count -eq 0) { $real = $jobs }
    $cache["run:$id"] = @($real | ForEach-Object {
            [pscustomobject]@{
                id = $_.id; name = $_.name; conclusion = $_.conclusion
                step = (@($_.steps | Where-Object { $_.conclusion -eq 'failure' } | ForEach-Object { $_.name }) + '')[0]
            }
        })
}

$failedJobs = @()
foreach ($run in $failedRuns) {
    foreach ($job in @($cache["run:$($run.id)"] | Where-Object { $_ })) {
        $failedJobs += [pscustomobject]@{ run = $run; job = $job; causes = @() }
    }
}

# Causes: from the failed step, or from the log when `cargo xtask ci` failed.
$logCalls = @{}
foreach ($f in $failedJobs) {
    $key = "job:$($f.job.id)"
    if ($f.job.conclusion -eq 'timed_out') { $f.causes = @('timed out') }
    elseif ($f.job.conclusion -eq 'cancelled') { $f.causes = @('cancelled') }
    elseif ($cache.ContainsKey($key)) { $f.causes = $cache[$key] }
    elseif ($f.job.step -match 'cargo xtask ci') {
        $logCalls["$($f.job.id)"] = @("repos/$Repo/actions/jobs/$($f.job.id)/logs", '--allow-escape-sequences')
    } elseif ($f.job.step) { $f.causes = @("step: $($f.job.step)") }
    else { $f.causes = @("job: $($f.job.name)") }
}
$logs = Invoke-GhParallel $logCalls
foreach ($f in @($failedJobs | Where-Object { $logCalls.ContainsKey("$($_.job.id)") })) {
    $log = $logs["$($f.job.id)"] -replace "\x1b\[[0-9;]*[A-Za-z]", ''
    $causes = @(Get-LogCauses $log)
    if ($causes.Count -gt 0) {
        $f.causes = $causes
        $cache["job:$($f.job.id)"] = $causes
    } elseif ($log) { $f.causes = @('cargo xtask ci (cause not found in the log)') }
    else { $f.causes = @('log unavailable') }
}
if (-not $NoCache) {
    try {
        New-Item -ItemType Directory -Force (Split-Path $cacheFile) | Out-Null
        $cache | ConvertTo-Json -Depth 4 -Compress | Set-Content -Encoding utf8 $cacheFile
    } catch { Write-Verbose "cache not saved: $_" }
}

$byCause = @{}
foreach ($f in $failedJobs) {
    foreach ($c in $f.causes) {
        if (-not $byCause.ContainsKey($c)) { $byCause[$c] = New-Object System.Collections.ArrayList }
        [void]$byCause[$c].Add($f.run.pr)
    }
}

# CI rounds: distinct head commits that ran the CI workflow on the PR.
$ciRounds = @($fleetRuns | Where-Object { $_.name -eq 'CI' -and $_.event -eq 'pull_request' } |
        Group-Object pr | ForEach-Object {
            [pscustomobject]@{ pr = [int]$_.Name; rounds = @($_.Group | ForEach-Object { $_.head_sha } | Sort-Object -Unique).Count }
        } | Where-Object { $_.rounds -gt 2 } | Sort-Object rounds -Descending)

# --- Reviews: clearance, open threads, fix rounds, queue ejections ----------

$clearPattern = '(?i)no findings|no blockers|nothing (that )?blocks|\bcleared\b'
$delayed = @()
$fixRounds = @()
$ejections = @()
foreach ($pr in $prs) {
    $clears = @($pr.comments.nodes | Where-Object {
            $_.body -match '^\s*\*\*mailbot\.review:\*\*' -and $_.body -match $clearPattern -and
            $_.body -notmatch '(?i)not a clearance'
        } | ForEach-Object { ConvertTo-Utc $_.createdAt } | Sort-Object)
    if ($clears.Count -gt 0) {
        # Threads opened before the last clearance and answered over the limit after it.
        $cleared = $clears[-1]
        if ($cleared -ge $fromUtc) {
            $late = @()
            foreach ($t in $pr.reviewThreads.nodes) {
                $opened = ConvertTo-Utc $t.opened.nodes[0].createdAt
                if (-not $opened -or $opened -gt $cleared) { continue }
                $closed = if ($t.isResolved) { ConvertTo-Utc $t.answered.nodes[0].createdAt } else { $nowUtc }
                if (($closed - $cleared).TotalMinutes -gt $ThreadMinutes) {
                    $late += [pscustomobject]@{ closed = $closed; open = -not $t.isResolved }
                }
            }
            if ($late.Count -gt 0) {
                $last = ($late | Sort-Object closed)[-1]
                $delayed += [pscustomobject]@{
                    pr = $pr.number; cleared = $cleared; threads = $late.Count
                    minutes = [int]($last.closed - $cleared).TotalMinutes
                    stillOpen = @($late | Where-Object { $_.open }).Count
                }
            }
        }
        # Commits (not merges of main) after the first clearance, inside the window.
        $after = @($pr.commits.nodes | Where-Object {
                $d = ConvertTo-Utc $_.commit.committedDate
                $_.commit.parents.totalCount -lt 2 -and $d -gt $clears[0] -and $d -ge $fromUtc
            } | ForEach-Object { $_.commit.abbreviatedOid })
        if ($after.Count -gt 0) { $fixRounds += [pscustomobject]@{ pr = $pr.number; commits = $after } }
    }
    foreach ($e in $pr.timelineItems.nodes) {
        if (-not $e.createdAt -or $e.reason -match '(?i)^merged$') { continue }
        if ((ConvertTo-Utc $e.createdAt) -ge $fromUtc) {
            $ejections += [pscustomobject]@{ pr = $pr.number; reason = $e.reason; at = ConvertTo-Utc $e.createdAt }
        }
    }
}

# --- /claim on issues without agent-ready ------------------------------------

$claims = @()
$comments = Get-GhRows "repos/$Repo/issues/comments?since=$fromIso" `
    '.[] | [.issue_url, .created_at, (.body | .[0:20])] | @tsv' @('issue_url', 'created_at', 'body')
foreach ($c in $comments) {
    # @tsv writes a newline in the body as \n.
    if ($c.body -match '^\s*/claim(\[rn]|\s)*$' -and (ConvertTo-Utc $c.created_at) -ge $fromUtc -and
        $c.issue_url -match '/issues/(\d+)$') {
        $claims += [pscustomobject]@{ issue = [int]$Matches[1]; at = ConvertTo-Utc $c.created_at }
    }
}

$unlabelled = @()
$claimed = @($claims | ForEach-Object { $_.issue } | Sort-Object -Unique)
if ($claimed.Count -gt 0) {
    $fields = ($claimed | ForEach-Object {
            "i$($_): issueOrPullRequest(number: $_) { ... on Issue { timelineItems(last: 50, itemTypes: [LABELED_EVENT, UNLABELED_EVENT]) { nodes { __typename ... on LabeledEvent { createdAt label { name } } ... on UnlabeledEvent { createdAt label { name } } } } } }"
        }) -join ' '
    $labelQuery = "query(`$o: String!, `$n: String!) { repository(owner: `$o, name: `$n) { $fields } }"
    $repoData = (Invoke-GhJson @('graphql', '-f', "query=$labelQuery", '-f', "o=$owner", '-f', "n=$name")).data.repository
    foreach ($c in $claims) {
        $events = @($repoData."i$($c.issue)".timelineItems.nodes | Where-Object { $_.label.name -eq 'agent-ready' })
        $ready = $false
        foreach ($e in ($events | Sort-Object { ConvertTo-Utc $_.createdAt })) {
            if ((ConvertTo-Utc $e.createdAt) -gt $c.at) { break }
            $ready = $e.__typename -eq 'LabeledEvent'
        }
        if (-not $ready) { $unlabelled += $c }
    }
}

# --- Report -------------------------------------------------------------------

$repeated = New-Object System.Collections.Generic.List[string]
foreach ($c in ($byCause.Keys | Sort-Object { -$byCause[$_].Count }, { $_ })) {
    if ($byCause[$c].Count -ge 2) {
        $repeated.Add("- **CI failures: $c**: $($byCause[$c].Count) ($(Format-Prs $byCause[$c]))")
    }
}
if ($delayed.Count -ge 2) { $repeated.Add("- **Merges delayed by open threads**: $($delayed.Count) ($(Format-Prs ($delayed | ForEach-Object { $_.pr })))") }
if ($fixRounds.Count -ge 2) { $repeated.Add("- **Pushes after the review cleared**: $($fixRounds.Count) PRs ($(Format-Prs ($fixRounds | ForEach-Object { $_.pr })))") }
if ($ciRounds.Count -ge 2) { $repeated.Add("- **More than 2 CI rounds**: $($ciRounds.Count) PRs ($(Format-Prs ($ciRounds | ForEach-Object { $_.pr })))") }
if ($unlabelled.Count -ge 2) { $repeated.Add("- **Claims without agent-ready**: $($unlabelled.Count) ($(Format-Prs ($unlabelled | ForEach-Object { $_.issue })))") }
foreach ($g in ($ejections | Group-Object reason | Where-Object { $_.Count -ge 2 })) {
    $repeated.Add("- **Merge-queue ejections ($($g.Name))**: $($g.Count) ($(Format-Prs ($g.Group | ForEach-Object { $_.pr })))")
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("# Pipeline retro, $($fromUtc.ToLocalTime().ToString('yyyy-MM-dd HH:mm')) to $(Format-Local $nowUtc)")
$lines.Add('')
$lines.Add("$($prs.Count) fleet PRs active, $($fleetRuns.Count) workflow runs on them, $($failedJobs.Count) failed jobs.")
$lines.Add('')
$lines.Add('## Repeated (2 or more)')
$lines.Add('')
if ($repeated.Count -eq 0) { $lines.Add('Nothing repeated.') } else { $lines.AddRange($repeated) }
$lines.Add('')

$lines.Add('## CI failures by cause')
$lines.Add('')
if ($byCause.Count -eq 0) { $lines.Add('None.') }
foreach ($c in ($byCause.Keys | Sort-Object { -$byCause[$_].Count }, { $_ })) {
    $lines.Add("- $c`: $($byCause[$c].Count) ($(Format-Prs $byCause[$c]))")
}
$lines.Add('')

$lines.Add("## Merges delayed by open threads (> $ThreadMinutes min after clearance)")
$lines.Add('')
if ($delayed.Count -eq 0) { $lines.Add('None.') }
foreach ($d in ($delayed | Sort-Object minutes -Descending)) {
    $open = if ($d.stillOpen) { ", $($d.stillOpen) still open" } else { '' }
    $lines.Add("- #$($d.pr): cleared $(Format-Local $d.cleared), $($d.threads) thread(s) answered up to $($d.minutes) min later$open")
}
$lines.Add('')

$lines.Add('## Pushes after the review cleared')
$lines.Add('')
if ($fixRounds.Count -eq 0) { $lines.Add('None.') }
foreach ($r in $fixRounds) { $lines.Add("- #$($r.pr): $($r.commits.Count) commit(s) ($($r.commits -join ', '))") }
$lines.Add('')

$lines.Add('## CI rounds per PR above 2')
$lines.Add('')
if ($ciRounds.Count -eq 0) { $lines.Add('None.') }
foreach ($r in $ciRounds) { $lines.Add("- #$($r.pr): $($r.rounds) rounds") }
$lines.Add('')

$lines.Add('## Claims on issues without agent-ready')
$lines.Add('')
if ($unlabelled.Count -eq 0) { $lines.Add('None.') }
foreach ($u in $unlabelled) { $lines.Add("- #$($u.issue), claimed $(Format-Local $u.at)") }
$lines.Add('')

$lines.Add('## Merge-queue ejections')
$lines.Add('')
if ($ejections.Count -eq 0) { $lines.Add('None.') }
foreach ($e in $ejections) { $lines.Add("- #$($e.pr) at $(Format-Local $e.at): $($e.reason)") }
$lines.Add('')
$lines.Add("_Read-only; took $([int]((Get-Date) - $started).TotalSeconds) s._")

$lines -join "`n"
