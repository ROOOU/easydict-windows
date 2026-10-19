param(
    [string]$Windows = '0,1,5',
    [string]$Scenarios = 'idle',
    [ValidateRange(1, 600)][int]$Seconds = 60,
    [ValidateRange(0, 60)][int]$SettleSeconds = 10,
    [ValidateRange(1, 10)][int]$Repetitions = 3,
    [string]$OutputPath,
    [switch]$SelfTest
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# `-File` hands `-Windows 0,1` over as the single literal string "0,1", and
# binding that to [int[]] quietly keeps only the last number — which would
# measure one window count while reporting another. Parse the list here, and
# accept the space-separated form a real [int[]] argument stringifies to.
$windowCounts = @($Windows -split '[,\s]+' | Where-Object { $_ } | ForEach-Object {
    $value = 0
    if (-not [int]::TryParse($_, [ref]$value) -or $value -notin 0, 1, 5) {
        throw "Unsupported WebView count: $_"
    }
    $value
})
if ($windowCounts.Count -eq 0) { throw 'At least one WebView count is required' }

# `idle` leaves the hidden windows alone; `suspend` has the probe suspend each
# WebView the way closing a window does, so the two can be compared directly.
$scenarioList = @($Scenarios -split '[,\s]+' | Where-Object { $_ } | ForEach-Object {
    if ($_ -notin 'idle', 'suspend') { throw "Unsupported scenario: $_" }
    $_
})
if ($scenarioList.Count -eq 0) { throw 'At least one scenario is required' }

function Get-TreeIds($Rows, [int]$RootId, [datetime]$Started) {
    $byId = @{}
    foreach ($row in $Rows) { $byId[[int]$row.Id] = $row }
    if (-not $byId.ContainsKey($RootId) -or
        [math]::Abs(($byId[$RootId].Created - $Started).TotalMilliseconds) -gt 2) {
        throw 'Root process exited or its PID was reused'
    }
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($RootId)
    do {
        $changed = $false
        foreach ($row in $Rows) {
            # A reused parent PID cannot adopt a child older than that parent.
            if ($ids.Contains([int]$row.Parent) -and -not $ids.Contains([int]$row.Id) -and
                $row.Created -ge $byId[[int]$row.Parent].Created) {
                [void]$ids.Add([int]$row.Id)
                $changed = $true
            }
        }
    } while ($changed)
    return ,$ids
}

function Get-TreeSnapshot([Diagnostics.Process]$Root) {
    $rows = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate |
        ForEach-Object {
            if ($null -ne $_.CreationDate) {
                [pscustomobject]@{ Id = [int]$_.ProcessId; Parent = [int]$_.ParentProcessId; Created = $_.CreationDate.ToUniversalTime() }
            }
        })
    $ids = Get-TreeIds $rows $Root.Id $Root.StartTime.ToUniversalTime()
    $captured = @()
    $captureErrors = @()
    foreach ($id in $ids) {
        $proc = $null
        try {
            $proc = [Diagnostics.Process]::GetProcessById($id)
            $proc.Refresh()
            $created = $proc.StartTime.ToUniversalTime()
            $row = $rows | Where-Object Id -EQ $id | Select-Object -First 1
            if ([math]::Abs(($created - $row.Created).TotalMilliseconds) -gt 2) { throw 'PID reused during capture' }
            $captured += [pscustomobject]@{
                pid = $id; parentPid = $row.Parent; name = $proc.ProcessName
                identity = "$id/$($created.Ticks)"; startedUtc = $created.ToString('o')
                cpuSeconds = $proc.TotalProcessorTime.TotalSeconds
                privateBytes = $proc.PrivateMemorySize64; workingSetBytes = $proc.WorkingSet64
                handles = $proc.HandleCount
            }
        } catch {
            $captureErrors += "$id`: $($_.Exception.Message)"
        } finally {
            if ($null -ne $proc) { $proc.Dispose() }
        }
    }
    [pscustomobject]@{ processes = $captured; errors = $captureErrors }
}

# `[IO.Path]::GetRelativePath` and `ProcessStartInfo.ArgumentList` are .NET Core
# only, and `Process.Kill(bool)` needs .NET Core 3. A machine with no PowerShell 7
# still has to be able to reproduce these measurements, so the four helpers below
# avoid them instead of requiring a `pwsh` that may not be installed.
function Get-PathUnder([string]$Root, [string]$Full) {
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $path = [IO.Path]::GetFullPath($Full)
    if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path is outside $Root`: $Full"
    }
    return $path.Substring($prefix.Length)
}

function Save-Report([string]$Path, $Report) {
    # `Set-Content -Encoding utf8` writes a BOM on Windows PowerShell 5.1, which
    # would make the report depend on the host that measured it.
    [IO.File]::WriteAllText($Path, ($Report | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
}

function Test-KillTreeSupported {
    return [bool]([Diagnostics.Process].GetMethods() | Where-Object {
        $_.Name -eq 'Kill' -and $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType -eq [bool]
    })
}

function Stop-ProcessTree([Diagnostics.Process]$Process) {
    if ($Process.HasExited) { return }
    if ($killTreeSupported) { $Process.Kill($true) } else { & taskkill.exe /PID $Process.Id /T /F 2>&1 | Out-Null }
    $Process.WaitForExit()
}

function Get-Stats($Values) {
    $sorted = @($Values | Sort-Object)
    $n = $sorted.Count
    if ($n -eq 0) { throw 'No samples' }
    $middle = [int][math]::Floor($n / 2)
    $median = if ($n % 2) { $sorted[$middle] } else { ($sorted[$middle - 1] + $sorted[$middle]) / 2.0 }
    [pscustomobject]@{
        median = $median; p95 = $sorted[[math]::Ceiling($n * 0.95) - 1]; max = $sorted[-1]
    }
}

function Get-TraceSummary($Samples, [int]$LogicalProcessors) {
    if ($Samples.Count -lt 2 -or $LogicalProcessors -lt 1) { throw 'At least two samples and one logical processor required' }
    $first = $Samples[0]
    $last = $Samples[-1]
    $members = ($first.processes.identity | Sort-Object) -join ','
    $stable = $true
    foreach ($sample in $Samples) {
        if ($sample.errors.Count -ne 0 -or (($sample.processes.identity | Sort-Object) -join ',') -ne $members) {
            $stable = $false
        }
    }
    $wall = $last.elapsedSeconds - $first.elapsedSeconds
    if ($wall -le 0) { throw 'Nonpositive measurement interval' }
    $cpuDelta = ($last.processes.cpuSeconds | Measure-Object -Sum).Sum - ($first.processes.cpuSeconds | Measure-Object -Sum).Sum
    if ($cpuDelta -lt 0) { $stable = $false }
    [pscustomobject]@{
        samples = $Samples.Count; elapsedSeconds = $wall; stableProcessTree = $stable
        # Do not turn process churn or an unreadable process into a false CPU zero.
        cpuSeconds = $(if ($stable) { $cpuDelta } else { $null })
        cpuPercentOneCore = $(if ($stable) { 100 * $cpuDelta / $wall } else { $null })
        cpuPercentMachine = $(if ($stable) { 100 * $cpuDelta / $wall / $LogicalProcessors } else { $null })
        processCount = Get-Stats @($Samples | ForEach-Object { $_.processes.Count })
        privateBytes = Get-Stats @($Samples | ForEach-Object { ($_.processes.privateBytes | Measure-Object -Sum).Sum })
        workingSetBytesSum = Get-Stats @($Samples | ForEach-Object { ($_.processes.workingSetBytes | Measure-Object -Sum).Sum })
        handles = Get-Stats @($Samples | ForEach-Object { ($_.processes.handles | Measure-Object -Sum).Sum })
    }
}

if ($SelfTest) {
    $t = [datetime]::UtcNow
    $rows = @(
        [pscustomobject]@{ Id = 10; Parent = 1; Created = $t },
        [pscustomobject]@{ Id = 20; Parent = 10; Created = $t.AddSeconds(1) },
        [pscustomobject]@{ Id = 30; Parent = 20; Created = $t.AddSeconds(2) },
        [pscustomobject]@{ Id = 40; Parent = 10; Created = $t.AddSeconds(-1) },
        [pscustomobject]@{ Id = 50; Parent = 999; Created = $t.AddSeconds(1) }
    )
    $ids = Get-TreeIds $rows 10 $t
    if (($ids | Sort-Object) -join ',' -ne '10,20,30') { throw 'Tree filtering failed' }
    $rejected = $false
    try { Get-TreeIds $rows 10 $t.AddSeconds(9) | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw 'PID reuse was not rejected' }
    $p1 = [pscustomobject]@{ identity = '1/a'; cpuSeconds = 1.0; privateBytes = 100; workingSetBytes = 200; handles = 3 }
    $p2 = [pscustomobject]@{ identity = '1/a'; cpuSeconds = 2.0; privateBytes = 200; workingSetBytes = 300; handles = 5 }
    $samples = @(
        [pscustomobject]@{ elapsedSeconds = 0.0; processes = @($p1); errors = @() },
        [pscustomobject]@{ elapsedSeconds = 10.0; processes = @($p2); errors = @() }
    )
    $summary = Get-TraceSummary $samples 4
    if ($summary.cpuPercentMachine -ne 2.5 -or $summary.privateBytes.median -ne 150) { throw 'Summary math failed' }
    $p2.identity = '1/reused'
    if ($null -ne (Get-TraceSummary $samples 4).cpuPercentMachine) { throw 'Churn must invalidate CPU measurement' }
    $p2.identity = '1/a'
    $samples[1].errors = @('access denied')
    if ($null -ne (Get-TraceSummary $samples 4).cpuPercentMachine) { throw 'Read failure must invalidate CPU measurement' }
    'resource-probe self-test passed'
    return
}

$appRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$killTreeSupported = Test-KillTreeSupported
$exe = (Get-Item -LiteralPath (Join-Path $appRoot 'src-tauri/target/release/examples/resource-probe.exe')).FullName
if (-not $OutputPath) {
    $OutputPath = Join-Path $appRoot "../docs/resource-probe-$(Get-Date -Format yyyyMMdd-HHmmss).json"
}
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
if (Test-Path -LiteralPath $OutputPath) { throw 'Refusing to overwrite an existing measurement' }
$os = Get-CimInstance Win32_OperatingSystem
$cpus = @(Get-CimInstance Win32_Processor)
$report = [ordered]@{
    schema = 1; status = 'running'; startedUtc = [datetime]::UtcNow.ToString('o')
    executable = $exe; sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
    distFilesAtMeasurement = @(Get-ChildItem -LiteralPath (Join-Path $appRoot 'dist') -File -Recurse | ForEach-Object {
        [ordered]@{ name = (Get-PathUnder (Join-Path $appRoot 'dist') $_.FullName); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    })
    windows = $os.Caption; osVersion = $os.Version; cpu = @($cpus.Name)
    logicalProcessors = [int]($cpus | Measure-Object NumberOfLogicalProcessors -Sum).Sum
    samplerAvailableProcessors = [Environment]::ProcessorCount
    memoryBytes = [long]$os.TotalVisibleMemorySize * 1024
    seconds = $Seconds; settleSeconds = $SettleSeconds; repetitions = $Repetitions
    scope = 'Isolated Release probe: production plugins/IPC/AppState/React; no tray, hooks, shortcuts, translation or production settings.'
    caveats = @('Summed working set double-counts shared pages.', 'Snapshots can miss processes that start and exit between samples.', 'Fresh WebView profiles do not imply cold OS caches.', 'Hidden idle windows do not represent visible interactions or continuous selection.')
    trials = @()
}
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
for ($repeat = 1; $repeat -le $Repetitions; $repeat++) {
    # Rotate the deterministic order so neither a window count nor a scenario is
    # always measured last.
    $order = @(for ($i = 0; $i -lt $windowCounts.Count; $i++) { $windowCounts[($i + $repeat - 1) % $windowCounts.Count] })
    $runs = @(for ($s = 0; $s -lt $scenarioList.Count; $s++) {
        $scenario = $scenarioList[($s + $repeat - 1) % $scenarioList.Count]
        foreach ($count in $order) { [pscustomobject]@{ Scenario = $scenario; Count = $count } }
    })
    foreach ($run in $runs) {
        $scenario = $run.Scenario
        $count = $run.Count
        $profile = Join-Path $tempRoot "easydict-resource-$([guid]::NewGuid())"
        [void][IO.Directory]::CreateDirectory($profile)
        $startInfo = [Diagnostics.ProcessStartInfo]::new($exe)
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $startInfo.RedirectStandardInput = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        # Quote the profile argument: a TEMP path may contain spaces, and
        # `ProcessStartInfo.ArgumentList` is unavailable on .NET Framework.
        $startInfo.Arguments = '"{0}" {1} {2}' -f $profile.Replace('"', '\"'), $count, $scenario
        $proc = [Diagnostics.Process]::new()
        $proc.StartInfo = $startInfo
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $ready = $null
        $samples = @()
        $startedProcess = $false
        try {
            [void]$proc.Start()
            $startedProcess = $true
            $stderr = $proc.StandardError.ReadToEndAsync()
            $line = $proc.StandardOutput.ReadLineAsync()
            while (-not $ready) {
                if ($proc.HasExited) { throw "Probe exited $($proc.ExitCode) before readiness: $($stderr.GetAwaiter().GetResult())" }
                if ($clock.Elapsed.TotalSeconds -gt 60) { throw 'Probe failed to become ready within 60 seconds' }
                if ($line.IsCompleted) {
                    $text = $line.GetAwaiter().GetResult()
                    if ($null -eq $text) { throw 'Probe output closed before readiness' }
                    if ($text.StartsWith('RESOURCE_PROBE ')) { $ready = $text.Substring(15) | ConvertFrom-Json }
                    else { $line = $proc.StandardOutput.ReadLineAsync() }
                } else { Start-Sleep -Milliseconds 50 }
            }
            $readyWall = $clock.Elapsed.TotalMilliseconds
            if (@($ready.windows).Count -ne $count -or $ready.profile -ne "\\?\$profile") {
                # Rust canonical paths may be verbatim UNC paths; compare normalized paths.
                if (@($ready.windows).Count -ne $count -or $ready.profile.Replace('\\?\', '') -ne $profile) { throw 'Probe readiness/profile mismatch' }
            }
            Start-Sleep -Seconds $SettleSeconds
            $clock.Restart()
            do {
                if ($proc.HasExited) { throw 'Probe exited during measurement' }
                $snapshot = Get-TreeSnapshot $proc
                $snapshot | Add-Member -NotePropertyName elapsedSeconds -NotePropertyValue $clock.Elapsed.TotalSeconds
                $samples += $snapshot
                if ($snapshot.errors.Count) { throw "Incomplete process capture: $($snapshot.errors -join '; ')" }
                if ($clock.Elapsed.TotalSeconds -ge $Seconds) { break }
                Start-Sleep -Milliseconds ([int][math]::Max(0, [math]::Min(1000, ($Seconds - $clock.Elapsed.TotalSeconds) * 1000)))
            } while ($true)
            $proc.StandardInput.WriteLine('quit')
            $proc.StandardInput.Flush()
            if (-not $proc.WaitForExit(15000)) { throw 'Probe did not exit' }
            if ($proc.ExitCode -ne 0) { throw "Probe exit $($proc.ExitCode): $($stderr.GetAwaiter().GetResult())" }
            $exitClock = [Diagnostics.Stopwatch]::StartNew()
            do {
                $remaining = @()
                foreach ($member in $samples[-1].processes) {
                    $child = $null
                    try {
                        $child = [Diagnostics.Process]::GetProcessById($member.pid)
                        $identity = "$($child.Id)/$($child.StartTime.ToUniversalTime().Ticks)"
                        if (-not $child.HasExited -and $identity -eq $member.identity) { $remaining += $member.pid }
                    } catch [ArgumentException], [InvalidOperationException] {
                        # This exact child is gone, not a same-PID replacement.
                    } finally {
                        if ($null -ne $child) { $child.Dispose() }
                    }
                }
                if ($remaining.Count -eq 0) { break }
                if ($exitClock.Elapsed.TotalSeconds -gt 10) { throw "Probe descendants did not exit: $($remaining -join ',')" }
                Start-Sleep -Milliseconds 100
            } while ($true)
            $summary = Get-TraceSummary $samples $report.logicalProcessors
            $report.trials += [ordered]@{ repetition = $repeat; scenario = $scenario; webviews = $count; ready = $ready; readyWallMillis = $readyWall; childExitWaitMillis = $exitClock.Elapsed.TotalMilliseconds; summary = $summary; trace = $samples }
            Save-Report $OutputPath $report
            Write-Output ("run={0} scenario={1} webviews={2} privateMiB={3:N2} RSSsumMiB={4:N2} CPUmachine={5:N4}% stable={6}" -f $repeat, $scenario, $count, ($summary.privateBytes.median / 1MB), ($summary.workingSetBytesSum.median / 1MB), $summary.cpuPercentMachine, $summary.stableProcessTree)
        } catch {
            $report.status = 'failed'
            $report.trials += [ordered]@{ repetition = $repeat; scenario = $scenario; webviews = $count; failure = $_.Exception.Message; trace = $samples }
            Save-Report $OutputPath $report
            throw
        } finally {
            if ($startedProcess -and -not $proc.HasExited) { Stop-ProcessTree $proc }
            $proc.Dispose()
            # Only this newly created profile is removable; never enumerate/delete older probes.
            $target = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $profile).Path)
            if ([IO.Path]::GetDirectoryName($target) -ne $tempRoot -or
                -not [IO.Path]::GetFileName($target).StartsWith('easydict-resource-') -or
                (Get-Item -LiteralPath $target).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
                throw 'Unsafe profile cleanup path'
            }
            for ($attempt = 0; $attempt -lt 100; $attempt++) {
                try { Remove-Item -LiteralPath $target -Recurse -Force; break }
                catch { if ($attempt -eq 99) { throw }; Start-Sleep -Milliseconds 100 }
            }
        }
    }
}
$report.status = 'completed'
Save-Report $OutputPath $report
Write-Output "Report: $OutputPath"
