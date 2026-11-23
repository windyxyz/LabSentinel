param(
    [Parameter(Mandatory)][string]$PidFile,
    [Parameter(Mandatory)][string]$OutputCsv,
    [ValidateRange(1, 60)][int]$IntervalSeconds = 5,
    [ValidateRange(1, 1800)][int]$StartTimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$deadline = [DateTime]::UtcNow.AddSeconds($StartTimeoutSeconds)
while (-not (Test-Path -LiteralPath $PidFile)) {
    if ([DateTime]::UtcNow -ge $deadline) { throw 'Benchmark did not reach the sampling window.' }
    Start-Sleep -Milliseconds 500
}

$testProcessId = [int](Get-Content -LiteralPath $PidFile -Raw)
$testProcess = Get-Process -Id $testProcessId
if ($testProcess.ProcessName -notmatch '^labsentinel_server-') {
    throw 'The sample PID does not belong to the LabSentinel test harness.'
}

$previousCpu = @{}
$lastSample = [DateTime]::UtcNow
$sampled = 0
while (-not $testProcess.HasExited) {
    $parents = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$testProcessId")
    $descendants = foreach ($child in $parents) {
        Get-CimInstance Win32_Process -Filter "ParentProcessId=$($child.ProcessId)"
    }
    $processIds = @($testProcessId) + @($parents | ForEach-Object ProcessId) + @($descendants | ForEach-Object ProcessId)
    $samples = @(Get-Process -Id $processIds -ErrorAction SilentlyContinue)
    $nextCpu = @{}
    $deltaCpu = 0.0
    foreach ($item in $samples) {
        $nextCpu[$item.Id] = $item.TotalProcessorTime.TotalSeconds
        if ($previousCpu.ContainsKey($item.Id)) {
            $deltaCpu += [Math]::Max(0, $nextCpu[$item.Id] - $previousCpu[$item.Id])
        }
    }
    $sampledAt = [DateTime]::UtcNow
    $wallSeconds = ($sampledAt - $lastSample).TotalSeconds
    $gpu = Get-CimInstance -ClassName Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction SilentlyContinue
    $gpuEngines = @($gpu | Where-Object {
        $_.Name -match '^pid_(\d+)_' -and $processIds -contains [int]$Matches[1]
    })
    $gpuTotal = ($gpuEngines | Measure-Object UtilizationPercentage -Sum).Sum
    $workingSet = ($samples | Measure-Object WorkingSet64 -Sum).Sum
    $privateBytes = ($samples | Measure-Object PrivateMemorySize64 -Sum).Sum
    $cpuPercent = if ($sampled -gt 0 -and $wallSeconds -gt 0) {
        [Math]::Round(100 * $deltaCpu / $wallSeconds / [Environment]::ProcessorCount, 2)
    }
    [pscustomobject]@{
        sampled_at_utc = $sampledAt.ToString('o')
        cpu_percent_machine = $cpuPercent
        gpu_engine_percent_sum = if ($gpuEngines.Count) { $gpuTotal }
        working_set_mib = [Math]::Round($workingSet / 1MB, 1)
        private_mib = [Math]::Round($privateBytes / 1MB, 1)
        processes = $samples.Count
    } | Export-Csv -LiteralPath $OutputCsv -NoTypeInformation -Append
    $previousCpu = $nextCpu
    $lastSample = $sampledAt
    $sampled++
    Start-Sleep -Seconds $IntervalSeconds
}

if ($sampled -eq 0) { throw 'The benchmark exited before a resource sample was collected.' }
Write-Output "Resource samples: $sampled"
