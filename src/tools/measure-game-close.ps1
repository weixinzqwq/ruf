<#
.SYNOPSIS
Measures one normal close of the packaged Aola player after a game session.
.EXAMPLE
.\src\tools\measure-game-close.ps1 -Check
.EXAMPLE
.\src\tools\measure-game-close.ps1 -GameProcessId 1234
.EXAMPLE
.\measure-game-close.ps1 -Check
.NOTES
Run without -Check when you intend to close the game. A timeout leaves it running.
The result is displayed and saved in a new close-timing-*.json in aola-data\diagnostics.
The packaged copy targets the executable beside it. The source copy targets the long-run package.
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 2147483647)][int]$GameProcessId,
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$package = $PSScriptRoot
$parentDirectory = Split-Path -Parent $PSScriptRoot
if ((Split-Path -Leaf $PSScriptRoot) -eq 'tools' -and (Split-Path -Leaf $parentDirectory) -eq 'src') {
    $package = Join-Path (Split-Path -Parent $parentDirectory) 'dist\aola-long-run-20261123'
}
$expectedExe = [IO.Path]::GetFullPath((Join-Path $package 'aola-native-test.exe'))
$diagnostics = Join-Path $package 'aola-data\diagnostics'

function Find-SessionLog([string]$Prefix, [int]$TargetProcessId, [long]$StartUnixMs, [long]$LastUnixMs) {
    # A PID can be reused. The nanosecond timestamp in a matching filename must
    # also fall within this process's lifetime. Never open other diagnostic logs.
    $result = @{ status = 'no-current-log'; path = $null }
    if (-not [IO.Directory]::Exists($diagnostics)) { return $result }
    $suffix = '-' + $TargetProcessId + '.jsonl'
    $files = @(foreach ($entry in [IO.Directory]::EnumerateFiles($diagnostics, "$Prefix-*$suffix")) {
        $name = [IO.Path]::GetFileName($entry)
        if ($name -match ('^' + [regex]::Escape($Prefix) + '-([0-9]{1,19})' + [regex]::Escape($suffix) + '$')) {
            $timestamp = [decimal]$Matches[1]
            if ($timestamp -ge ([decimal]$StartUnixMs * 1000000) -and
                $timestamp -lt ([decimal]($LastUnixMs + 1) * 1000000)) {
                $entry
            }
        }
    })
    if ($files.Count -eq 0) { return $result }
    if ($files.Count -ne 1) { $result.status = 'ambiguous-current-log'; return $result }
    return @{ status = 'found'; path = $files[0] }
}

function Test-NonnegativeInteger($Value) {
    return ($Value -is [long] -or $Value -is [int]) -and $Value -ge 0
}

function Get-WallElapsedMs($StartMs, $StopMs) {
    if ((Test-NonnegativeInteger $StartMs) -and (Test-NonnegativeInteger $StopMs)) {
        return $StopMs - $StartMs
    }
    return $null
}

function Read-FinalSample($Process, [long]$StartUnixMs, [long]$LastUnixMs) {
    $log = Find-SessionLog 'performance' $Process.Id $StartUnixMs $LastUnixMs
    $result = @{ status = $log.status; unixMs = $null }
    if ($log.status -ne 'found') { return $result }
    $result.status = 'no-final-row'
    $line = Get-Content -LiteralPath $log.path -Tail 1 -Encoding UTF8
    if ($line) {
        $row = $line | ConvertFrom-Json
        if ($row.final -is [bool] -and $row.final -and $row.processId -eq $Process.Id -and
            (Test-NonnegativeInteger $row.unixMs)) {
            $result.status = 'found'
            $result.unixMs = [long]$row.unixMs
        }
    }
    return $result
}

function Read-PhaseSummary([string]$Path, [int]$TargetProcessId) {
    $summary = [ordered]@{
        completed = $false
        workerBoundaryEvents = $null
        finishUnixMs = $null
        playerDropMs = $null; playerDropInstances = $null
        gcDropMs = $null; gcDropInstances = $null
        slowestMainThreadSelf = $null
        gpuResourceDrops = $null
        timingScope = 'Player/GC sum inclusive main-thread instances. The ranking sums exclusive time by name. Worker boundary events count unpaired edges at the recording limits. GPU counters sum completed drop calls across all threads and may overlap wall time.'
    }
    $begins = @{}
    $top = @{}
    $openIds = [Collections.Generic.HashSet[long]]::new()
    $alreadyRunning = [Collections.Generic.List[string]]::new()
    $ends = [Collections.Generic.List[object]]::new()
    $started = $false
    $finish = $null
    $mainThread = $null
    $shutdownBegin = $false
    try {
        # The process can still hold the file for writing when the external wait times out.
        $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
        try {
            $reader = [IO.StreamReader]::new($stream)
            while ($null -ne ($line = $reader.ReadLine())) {
                $row = ConvertFrom-Json -InputObject $line
                if ($row.schema -ne 1 -or $row.processId -ne $TargetProcessId -or $finish) { return $summary }
                if ($row.event -ceq 'start' -and -not $started) { $started = $true; continue }
                if (-not $started) { return $summary }
                if ($row.event -ceq 'finish') {
                    if (-not (Test-NonnegativeInteger $row.unixMs) -or -not (Test-NonnegativeInteger $row.openPhases) -or
                        $row.openPhases -ne $openIds.Count -or -not $mainThread -or $top.ContainsKey($mainThread) -or
                        $alreadyRunning.Contains($mainThread)) { return $summary }
                    if ($row.PSObject.Properties['openWorkerPhaseIds']) {
                        $ids = $row.openWorkerPhaseIds
                        if ($ids -isnot [array] -or $ids.Count -ne $openIds.Count) { return $summary }
                        foreach ($workerId in $ids) {
                            if (-not (Test-NonnegativeInteger $workerId) -or -not $openIds.Remove($workerId)) { return $summary }
                        }
                    } elseif ($openIds.Count -or $alreadyRunning.Count) {
                        # Earlier schema-1 logs did not describe any worker boundaries.
                        return $summary
                    }
                    $finish = $row
                    continue
                }
                $thread = [string]$row.thread
                if (-not $thread -or -not $row.phase) { return $summary }
                if ($row.event -ceq 'worker-already-running') {
                    if ($top.ContainsKey($thread) -or $row.phase -ceq 'shutdown' -or $row.PSObject.Properties['id'] -or
                        $row.PSObject.Properties['parent'] -or $row.PSObject.Properties['durationUs'] -or
                        $row.PSObject.Properties['selfUs']) { return $summary }
                    $alreadyRunning.Add($thread)
                    continue
                }
                $id = [string]$row.id
                if (-not (Test-NonnegativeInteger $row.id) -or $row.id -eq 0 -or -not $row.PSObject.Properties['parent'] -or
                    ($null -ne $row.parent -and -not (Test-NonnegativeInteger $row.parent))) {
                    return $summary
                }
                if ($row.event -ceq 'begin') {
                    if ($begins.ContainsKey($id) -or [string]$row.parent -cne [string]$top[$thread]) { return $summary }
                    if ($row.phase -ceq 'shutdown') {
                        if ($shutdownBegin -or $null -ne $row.parent) { return $summary }
                        $shutdownBegin = $true
                    }
                    $begins[$id] = $row
                    $top[$thread] = $id
                    $null = $openIds.Add($row.id)
                    continue
                }
                $begin = $begins[$id]
                if ($row.event -cne 'end' -or $id -cne $top[$thread] -or $begin.thread -cne $thread -or $begin.phase -cne $row.phase -or
                    [string]$begin.parent -cne [string]$row.parent -or
                    -not (Test-NonnegativeInteger $row.selfUs) -or -not (Test-NonnegativeInteger $row.durationUs) -or
                    $row.selfUs -gt $row.durationUs) { return $summary }
                $null = $openIds.Remove($row.id)
                if ($null -eq $row.parent) { $top.Remove($thread) } else { $top[$thread] = [string]$row.parent }
                if ($row.phase -ceq 'shutdown') {
                    if ($mainThread -or $null -ne $row.parent) { return $summary }
                    $mainThread = $thread
                }
                $ends.Add($row)
            }
        } finally { $stream.Dispose() }
    } catch { return $summary }
    if (-not $finish -or -not $mainThread) { return $summary }

    $gpu = $null
    if ($finish.PSObject.Properties['gpuResourceDrops']) {
        $drops = $finish.gpuResourceDrops
        if ($drops -isnot [pscustomobject]) { return $summary }
        $gpu = [ordered]@{}
        $kinds = @('buffer', 'texture', 'textureView', 'bindGroup')
        foreach ($field in $drops.PSObject.Properties) {
            if ($field.Name -cnotin $kinds) { return $summary }
        }
        foreach ($kind in $kinds) {
            # The recorder omits resource kinds whose count did not increase.
            if (-not $drops.PSObject.Properties[$kind]) {
                $gpu[$kind] = [ordered]@{ count = 0; totalNs = 0; totalMs = 0 }
                continue
            }
            $metric = $drops.$kind
            if ($metric -isnot [pscustomobject] -or -not $metric.PSObject.Properties['count'] -or
                -not (Test-NonnegativeInteger $metric.count) -or -not (Test-NonnegativeInteger $metric.totalNs)) { return $summary }
            $gpu[$kind] = [ordered]@{ count = $metric.count; totalNs = $metric.totalNs; totalMs = $metric.totalNs / 1000000.0 }
        }
    }
    $byName = @{}
    foreach ($edge in $ends) {
        if ($edge.thread -cne $mainThread) { continue }
        $name = $edge.phase
        if (-not $byName.ContainsKey($name)) {
            $byName[$name] = [pscustomobject]@{ name = $name; calls = 0; durationUs = 0L; selfUs = 0L }
        }
        $byName[$name].calls++
        $byName[$name].durationUs += $edge.durationUs
        $byName[$name].selfUs += $edge.selfUs
    }
    $summary.completed = $true
    $summary.workerBoundaryEvents = $alreadyRunning.Count + $finish.openPhases
    $summary.finishUnixMs = $finish.unixMs
    $summary.gpuResourceDrops = $gpu
    if ($byName.ContainsKey('player-drop')) {
        $summary.playerDropMs = $byName['player-drop'].durationUs / 1000.0
        $summary.playerDropInstances = $byName['player-drop'].calls
    }
    if ($byName.ContainsKey('gc-drop-all')) {
        $summary.gcDropMs = $byName['gc-drop-all'].durationUs / 1000.0
        $summary.gcDropInstances = $byName['gc-drop-all'].calls
    }
    $summary.slowestMainThreadSelf = @($byName.Values |
        Sort-Object -Property @{ Expression = 'selfUs'; Descending = $true }, name |
        Select-Object -First 3 | ForEach-Object {
            [ordered]@{ name = $_.name; selfMs = $_.selfUs / 1000.0; instances = $_.calls }
        })
    return $summary
}

$explicitPid = $PSBoundParameters.ContainsKey('GameProcessId')
$processes = if ($explicitPid) {
    @(Get-Process -Id $GameProcessId -ErrorAction SilentlyContinue)
} else {
    @(Get-Process -Name 'aola-native-test' -ErrorAction SilentlyContinue)
}
$unknown = 0
$matching = @(foreach ($item in $processes) {
    try {
        # Keep a handle to the original process through validation and timing.
        $null = $item.Handle
        $image = $item.Path
        if (-not $image) { $unknown++; continue }
        if ([string]::Equals([IO.Path]::GetFullPath($image), $expectedExe, [StringComparison]::OrdinalIgnoreCase)) {
            $item
        }
    } catch {
        $unknown++
    }
})
if ($matching.Count -ne 1 -or $unknown -gt 0) {
    $status = if ($unknown) { 'unverified-process' } elseif ($matching.Count) { 'ambiguous-process' } else { 'no-matching-process' }
    [ordered]@{ status = $status; matchingProcessIds = @($matching | ForEach-Object { $_.Id }); closePosted = $false } | ConvertTo-Json
    return
}

if (-not ('AolaCloseWindow' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AolaCloseWindow {
    [StructLayout(LayoutKind.Sequential)]
    struct Rect { public int Left, Top, Right, Bottom; }
    delegate bool EnumCallback(IntPtr window, IntPtr value);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumCallback visit, IntPtr value);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", EntryPoint="PostMessageW", SetLastError=true)]
    static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    public static bool Visible(IntPtr window, int process) {
        uint owner;
        return window != IntPtr.Zero && GetWindowThreadProcessId(window, out owner) != 0
            && owner == (uint)process && IsWindowVisible(window);
    }
    public static IntPtr SingleWindow(int process) {
        IntPtr found = IntPtr.Zero;
        int count = 0;
        bool completed = EnumWindows(delegate(IntPtr window, IntPtr unused) {
            Rect rect;
            if (Visible(window, process) && GetWindow(window, 4 /* GW_OWNER */) == IntPtr.Zero
                && GetClientRect(window, out rect) && rect.Right > rect.Left && rect.Bottom > rect.Top) {
                found = window;
                count++;
            }
            return true;
        }, IntPtr.Zero);
        return completed && count == 1 ? found : IntPtr.Zero;
    }
    public static bool Close(IntPtr window, int process) {
        return window != IntPtr.Zero && SingleWindow(process) == window
            && Visible(window, process) && PostMessage(window, 0x0010 /* WM_CLOSE */, IntPtr.Zero, IntPtr.Zero);
    }
}
'@
}

$game = $matching[0]
try {
    $window = [IntPtr]::Zero
    try {
        $startedMs = ([DateTimeOffset]$game.StartTime).ToUnixTimeMilliseconds()
        $game.Refresh()
        if (-not $game.HasExited) { $window = [AolaCloseWindow]::SingleWindow($game.Id) }
    } catch [InvalidOperationException] {
        # It exited between enumeration and window validation.
    }
    if ($window -eq [IntPtr]::Zero) {
        [ordered]@{ status = 'no-unique-visible-game-window'; processId = $game.Id; closePosted = $false } | ConvertTo-Json
        return
    }
    if ($Check) {
        [ordered]@{ status = 'ready'; processId = $game.Id; closePosted = $false } | ConvertTo-Json
        return
    }

    $postUnixMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $timer = [Diagnostics.Stopwatch]::StartNew()
    if (-not [AolaCloseWindow]::Close($window, $game.Id)) {
        [ordered]@{ status = 'close-not-posted'; processId = $game.Id; closePosted = $false } | ConvertTo-Json
        return
    }
    $goneMs = $null
    $endMs = $null
    $exitUnixMs = $null
    while ($true) {
        if ($null -eq $goneMs -and -not [AolaCloseWindow]::Visible($window, $game.Id)) {
            $goneMs = [Math]::Round($timer.Elapsed.TotalMilliseconds, 2)
        }
        if ($game.WaitForExit(0)) {
            $endMs = [Math]::Round($timer.Elapsed.TotalMilliseconds, 2)
            if ($null -eq $goneMs) { $goneMs = $endMs }
            $exitUnixMs = ([DateTimeOffset]$game.ExitTime).ToUnixTimeMilliseconds()
            break
        }
        $remaining = 15000 - $timer.Elapsed.TotalMilliseconds
        if ($remaining -le 0) { break }
        $null = $game.WaitForExit([int][Math]::Min(10, [Math]::Ceiling($remaining)))
    }
    $lastUnixMs = if ($null -ne $exitUnixMs) { $exitUnixMs } else { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
    try { $final = Read-FinalSample $game $startedMs $lastUnixMs }
    catch { $final = @{ status = 'unreadable-final-row'; unixMs = $null } }
    try { $phases = Find-SessionLog 'close-phases' $game.Id $startedMs $lastUnixMs }
    catch { $phases = @{ status = 'unreadable-directory'; path = $null } }
    $phaseSummary = if ($phases.path) { Read-PhaseSummary $phases.path $game.Id } else { $null }
    $result = [ordered]@{
        status = if ($null -ne $endMs) { 'exited' } else { 'timeout' }
        closePosted = $true
        processId = $game.Id
        closeToWindowGoneMs = $goneMs
        closeToProcessExitMs = $endMs
        pollIntervalMs = 10
        waitLimitMs = 15000
        closeUnixMs = $postUnixMs
        processExitUnixMs = $exitUnixMs
        finalSampleStatus = $final.status
        finalSampleUnixMs = $final.unixMs
        closePhasesStatus = $phases.status
        closePhasesFile = if ($phases.path) { 'aola-data\diagnostics\' + [IO.Path]::GetFileName($phases.path) } else { $null }
        phaseSummary = $phaseSummary
        closeToFinalWallMs = Get-WallElapsedMs $postUnixMs $final.unixMs
        finalToProcessExitWallMs = Get-WallElapsedMs $final.unixMs $exitUnixMs
        finalToPhaseFinishWallMs = Get-WallElapsedMs $final.unixMs $phaseSummary.finishUnixMs
        phaseFinishToProcessExitWallMs = Get-WallElapsedMs $phaseSummary.finishUnixMs $exitUnixMs
        clockNote = 'Window/process durations are monotonic observations. Wall splits use cross-process timestamps recorded in whole milliseconds, subject to Windows clock resolution and adjustments. Final and finish timestamps precede writing their JSON records.'
    }
    $json = $result | ConvertTo-Json -Depth 6
    try {
        $null = [IO.Directory]::CreateDirectory($diagnostics)
        $fileName = "close-timing-$postUnixMs-$($game.Id)-$([Guid]::NewGuid().ToString('N')).json"
        $outFile = [IO.File]::Open((Join-Path $diagnostics $fileName), 'CreateNew', 'Write', 'Read')
        try {
            $bytes = [Text.Encoding]::UTF8.GetBytes($json + [Environment]::NewLine)
            $outFile.Write($bytes, 0, $bytes.Length)
        } finally { $outFile.Dispose() }
    } catch { Write-Warning 'Could not save the timing report. The JSON follows.' }
    $json
} finally {
    $game.Dispose()
}
