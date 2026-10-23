param(
    [Parameter(Mandatory=$true)][string]$DumpRoot,
    [string]$ToolPath = (Join-Path $PSScriptRoot '..\dist\PS4ModuleTest.exe'),
    [string]$ReportParent = (Join-Path $PSScriptRoot '..\local\sandboxes'),
    [switch]$Allocator,
    [switch]$New,
    [switch]$NewFailure,
    [switch]$Copy
)
$ErrorActionPreference = 'Stop'
$selectedModes = @(@($Allocator.IsPresent, $New.IsPresent, $NewFailure.IsPresent, $Copy.IsPresent) | Where-Object { $_ })
if ($selectedModes.Count -gt 1) { throw 'Choose at most one logger mode switch.' }
$mode = if ($NewFailure) {'--logger-new-failure'}elseif ($Copy) {'--logger-copy'}elseif ($New) {'--logger-new'}elseif ($Allocator) {'--logger-allocate'}else{'--logger-initialize'}
$hasNew = $New -or $Copy
$path = & $ToolPath $DumpRoot $ReportParent $mode
if ($LASTEXITCODE -ne 0 -or @($path).Count -ne 1) { throw 'Logger harness failed to create its report' }
$report = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
$events = @(Get-Content -LiteralPath $report.events | ForEach-Object { $_ | ConvertFrom-Json })
if ($report.exit_code -ne 86 -or $report.timed_out -or $report.authentic_dashboard_verified) { throw 'Unexpected logger worker result' }
$modules = @($events | Where-Object event -eq 'module-loaded')
$init = @($events | Where-Object event -eq 'original-logger-module-init-returned')
$call = @($events | Where-Object event -eq 'calling-original-logger-initialize')
$fail = @($events | Where-Object event -eq 'failure')
$moduleInit = @($events | Where-Object event -eq 'module-init-returned')
if ($moduleInit.Count -ne 1 -or $moduleInit[0].status -ne 0) { throw 'Original IPMI module initialization did not return success' }
$allocations = @($events | Where-Object event -eq 'logger-owned-allocation')
$allocationFailures = @($events | Where-Object event -eq 'logger-owned-allocation-failed')
$operatorNews = @($events | Where-Object event -eq 'logger-owned-operator-new')
$operatorNewFailures = @($events | Where-Object event -eq 'logger-operator-new-failure-unsupported')
$copies = @($events | Where-Object event -eq 'logger-bounded-strncpy')
if ($NewFailure) {
    if ($modules.Count -ne 1 -or $modules[0].sha256 -ne 'c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5' -or $modules[0].linked_original_function_nids.Count -ne 0) { throw 'Failure probe must load only original IPMI' }
    if ($init.Count -ne 0 -or $call.Count -ne 0 -or @($events | Where-Object event -eq 'calling-original-logger-module-init').Count -ne 0) { throw 'Failure probe must not load or execute SystemLogger' }
    if ($allocations.Count -ne 0 -or $operatorNews.Count -ne 0 -or $copies.Count -ne 0 -or $allocationFailures.Count -ne 1 -or $allocationFailures[0].requested_bytes -ne [UInt64]::MaxValue -or $allocationFailures[0].posix_errno -ne 12) { throw 'Bounded allocation failure evidence missing or unexpected' }
    if ($operatorNewFailures.Count -ne 1 -or $operatorNewFailures[0].requested_bytes -ne [UInt64]::MaxValue -or $operatorNewFailures[0].guest_exception_thrown -or $operatorNewFailures[0].null_returned_to_guest) { throw 'Throwing-new failure was not safely fail-stopped' }
    if ($fail.Count -ne 1 -or $fail[0].reason -ne 'Throwing operator new failed; guest exception handling is unsupported') { throw 'Unexpected throwing-new failure boundary' }
} else {
    if ($modules.Count -ne 2 -or $modules[0].sha256 -ne 'c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5' -or $modules[1].sha256 -ne '817f9b8efa23e9693e712f5e5818a30505f81aa986a1e96870c3c3cec8f71ab0') { throw 'Expected original module profiles missing' }
    if (($modules[1].linked_original_function_nids -join ',') -ne '0zsTiDhM0nU,O1lQ2+do5r4,Rih7HnIM3JQ') { throw 'Original IPMI provider links differ' }
    if ($init.Count -ne 1 -or $init[0].status -ne 0 -or $call.Count -ne 1 -or $call[0].original_shell_service -or $call[0].authentic_dashboard_verified) { throw 'Original logger initialization evidence missing or mislabeled' }
    $expectedImport = if ($Copy) {'F8bUHwAG284'}elseif ($New) {'6sJWiWSRuqk'}elseif ($Allocator) {'fJnpuVVBbKk'}else{'gQX+4GDQjpM'}
    if ($fail.Count -ne 1 -or $fail[0].reason -ne ('Unimplemented native import: ' + $expectedImport)) { throw 'Expected missing native import was not observed' }
    if (@($events | Where-Object event -eq 'original-logger-initialize-returned').Count -ne 0) { throw 'Logger initialization must stop before returning success' }
    if ($hasNew) {
        if ($allocations.Count -ne 2 -or $allocations[0].requested_bytes -ne 4096 -or $allocations[0].charged_bytes -ne 4096 -or $allocations[0].live_allocations -ne 1 -or !$allocations[0].synthetic_host_allocator -or $allocations[1].requested_bytes -ne 8 -or $allocations[1].charged_bytes -ne 4096 -or $allocations[1].live_allocations -ne 2 -or !$allocations[1].synthetic_host_allocator) { throw 'Expected bounded logger malloc/new allocations missing' }
        if ($operatorNews.Count -ne 1 -or $operatorNews[0].requested_bytes -ne 8 -or !$operatorNews[0].synthetic_host_allocator -or $operatorNews[0].guest_exception_thrown -or $allocationFailures.Count -ne 0 -or $operatorNewFailures.Count -ne 0) { throw 'Successful operator-new evidence missing or failure event unexpected' }
        if ($Copy) {
            if ($copies.Count -ne 1 -or $copies[0].count -ne 32 -or $copies[0].source_bytes_examined -ne 27 -or !$copies[0].bounded_host_bridge) { throw 'Expected bounded strncpy evidence missing or unexpected' }
        } elseif ($copies.Count -ne 0) { throw 'strncpy bridge must remain disabled before -Copy mode' }
    } else {
        $expectedAllocations = if ($Allocator) {1}else{0}
        if ($allocations.Count -ne $expectedAllocations -or $allocationFailures.Count -ne 0 -or $operatorNews.Count -ne 0 -or $operatorNewFailures.Count -ne 0) { throw 'Unexpected allocation events in baseline logger mode' }
        if (@($events | Where-Object event -eq 'logger-bounded-strncpy').Count -ne 0) { throw 'strncpy bridge must remain disabled in earlier modes' }
        if ($Allocator -and ($allocations[0].requested_bytes -ne 4096 -or $allocations[0].charged_bytes -ne 4096 -or $allocations[0].live_allocations -ne 1 -or !$allocations[0].synthetic_host_allocator)) { throw 'Expected bounded owned logger allocation missing' }
    }
}
$path
