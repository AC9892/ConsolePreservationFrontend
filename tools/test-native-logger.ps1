param(
    [Parameter(Mandatory=$true)][string]$DumpRoot,
    [string]$ToolPath = (Join-Path $PSScriptRoot '..\dist\PS4ModuleTest.exe'),
    [string]$ReportParent = (Join-Path $PSScriptRoot '..\local\sandboxes'),
    [switch]$Allocator,
    [switch]$New,
    [switch]$NewFailure,
    [switch]$Copy,
    [switch]$AttrProbe,
    [switch]$AttrInit,
    [switch]$MutexCreate,
    [switch]$MutexLifecycle
)
$ErrorActionPreference = 'Stop'
$selectedModes = @(@($Allocator.IsPresent, $New.IsPresent, $NewFailure.IsPresent, $Copy.IsPresent, $AttrProbe.IsPresent, $AttrInit.IsPresent, $MutexCreate.IsPresent, $MutexLifecycle.IsPresent) | Where-Object { $_ })
if ($selectedModes.Count -gt 1) { throw 'Choose at most one logger mode switch.' }
$mode = if ($NewFailure) {'--logger-new-failure'}elseif ($MutexCreate) {'--logger-mutex-create'}elseif ($MutexLifecycle) {'--logger-mutex-lifecycle'}elseif ($AttrProbe) {'--logger-attr-probe'}elseif ($AttrInit) {'--logger-attr-init'}elseif ($Copy) {'--logger-copy'}elseif ($New) {'--logger-new'}elseif ($Allocator) {'--logger-allocate'}else{'--logger-initialize'}
$hasMutex = $MutexCreate -or $MutexLifecycle
$hasAttributes = $AttrProbe -or $AttrInit -or $hasMutex
$hasNew = $New -or $Copy -or $hasAttributes
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
$attributesCaptured = @($events | Where-Object event -eq 'logger-mutexattr-output-captured')
$attributesInitialized = @($events | Where-Object event -eq 'logger-owned-mutexattr-initialized')
$attributeReference = @($events | Where-Object event -eq 'logger-attribute-reference-verified')
$attributedMutex = @($events | Where-Object event -eq 'logger-attributed-mutex-create-deferred')
$mutexesInitialized = @($events | Where-Object event -eq 'native-mutex-initialized')
$attributedMutexes = @($events | Where-Object event -eq 'logger-attributed-mutex-created')
$attributesDestroyed = @($events | Where-Object event -eq 'logger-owned-mutexattr-destroyed')
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
    if (@($events | Where-Object event -eq 'original-logger-initialize-returned').Count -ne 0) { throw 'Logger initialization must stop before returning success' }
    if ($AttrProbe) {
        $expectedFailure = 'Original mutex-attribute output captured; attribute creation remains unsupported'
    } elseif ($AttrInit) {
        $expectedFailure = 'Logger mutex creation with attributes remains unsupported'
    } elseif ($MutexCreate) {
        $expectedFailure = 'Unimplemented native import: smWEktiyyG0'
    } elseif ($MutexLifecycle) {
        $expectedFailure = 'Unsupported IPMI operation 0x400; no success response fabricated'
    } else {
        $expectedImport = if ($Copy) {'F8bUHwAG284'}elseif ($New) {'6sJWiWSRuqk'}elseif ($Allocator) {'fJnpuVVBbKk'}else{'gQX+4GDQjpM'}
        $expectedFailure = 'Unimplemented native import: ' + $expectedImport
    }
    if ($fail.Count -ne 1 -or $fail[0].reason -ne $expectedFailure) { throw 'Expected logger failure boundary was not observed' }
    if ($hasNew) {
        $expectedAllocationCount = if ($hasAttributes -and !$AttrProbe) {3}else{2}
        if ($allocations.Count -ne $expectedAllocationCount -or $allocations[0].requested_bytes -ne 4096 -or $allocations[0].charged_bytes -ne 4096 -or $allocations[0].live_allocations -ne 1 -or !$allocations[0].synthetic_host_allocator -or $allocations[1].requested_bytes -ne 8 -or $allocations[1].charged_bytes -ne 4096 -or $allocations[1].live_allocations -ne 2 -or !$allocations[1].synthetic_host_allocator) { throw 'Expected bounded logger malloc/new allocations missing' }
        if ($operatorNews.Count -ne 1 -or $operatorNews[0].requested_bytes -ne 8 -or !$operatorNews[0].synthetic_host_allocator -or $operatorNews[0].guest_exception_thrown -or $allocationFailures.Count -ne 0 -or $operatorNewFailures.Count -ne 0) { throw 'Successful operator-new evidence missing or failure event unexpected' }
        if ($hasAttributes) {
            if ($copies.Count -ne 1 -or $copies[0].count -ne 32 -or $copies[0].source_bytes_examined -ne 27 -or !$copies[0].bounded_host_bridge) { throw 'Expected bounded strncpy evidence missing before attribute probe' }
            if ($attributesCaptured.Count -ne 1 -or $attributesCaptured[0].output_slot_bytes -ne 8 -or !$attributesCaptured[0].output_slot_writable -or !$attributesCaptured[0].output_on_worker_stack) { throw 'Expected writable worker-stack attribute pointer slot was not verified' }
            if ($AttrProbe) {
                if ($attributeReference.Count -ne 0 -or $attributesInitialized.Count -ne 0 -or $attributedMutex.Count -ne 0 -or $allocations.Count -ne 2 -or $attributesCaptured[0].attribute_written) { throw 'Attribute probe must capture only the slot and leave it unwritten' }
            } elseif ($hasMutex -or $AttrInit) {
                if ($attributeReference.Count -ne 1 -or $attributeReference[0].sha256 -ne '188b440550403ae149a733f3da2ce5fc905ae6528a2379f87ff4c4b798171899' -or $attributeReference[0].kernel_executed) { throw 'Pinned reference kernel verification missing or mislabeled' }
                if ($allocations[2].requested_bytes -ne 12 -or $allocations[2].charged_bytes -ne 4096 -or $allocations[2].live_allocations -ne 3 -or !$allocations[2].synthetic_host_allocator) { throw 'Expected bounded 12-byte diagnostic attribute allocation missing' }
                if ($attributesInitialized.Count -ne 1 -or $attributesInitialized[0].object_bytes -ne 12 -or ($attributesInitialized[0].default_words -join ',') -ne '1,0,0' -or !$attributesInitialized[0].pointer_matches_registry -or !$attributesInitialized[0].synthetic_host_attribute -or $attributesInitialized[0].native_kernel_implementation_executed) { throw 'Synthetic attribute initialization contract mismatch' }
                if ($MutexCreate -or $MutexLifecycle) {
                    if ($attributedMutex.Count -ne 0 -or $mutexesInitialized.Count -ne 1 -or $mutexesInitialized[0].name -ne 'SceSysLogExternalIpcClient' -or $attributedMutexes.Count -ne 1 -or $attributedMutexes[0].name -ne 'SceSysLogExternalIpcClient' -or !$attributedMutexes[0].attribute_identity_verified -or !$attributedMutexes[0].owned_host_mutex -or $attributedMutexes[0].native_kernel_implementation_executed) { throw 'Owned attributed-mutex creation evidence missing or unexpected' }
                    if ($MutexCreate) {
                        if ($attributesDestroyed.Count -ne 0 -or @($events | Where-Object event -eq 'ipmi-client-created').Count -ne 0 -or $fail[0].reason -ne 'Unimplemented native import: smWEktiyyG0') { throw 'Mutex-create mode must stop before attribute destruction' }
                    } else {
                        if ($attributesDestroyed.Count -ne 1 -or !$attributesDestroyed[0].pointer_slot_cleared -or !$attributesDestroyed[0].object_mapping_released -or $attributesDestroyed[0].remaining_attribute_objects -ne 0 -or $attributesDestroyed[0].remaining_logger_allocations -ne 2 -or !$attributesDestroyed[0].synthetic_host_attribute -or $attributesDestroyed[0].native_kernel_implementation_executed) { throw 'Owned attribute destroy/release evidence missing or unexpected' }
                        $clients = @($events | Where-Object event -eq 'ipmi-client-created')
                        $requests = @($events | Where-Object event -eq 'native-connection-request')
                        $lookups = @($events | Where-Object event -eq 'native-connection-server-lookup')
                        $operations = @($events | Where-Object event -eq 'unsupported-ipmi-operation')
                        if ($clients.Count -ne 1 -or $clients[0].service -ne 'SceSystemLoggerService' -or $clients[0].connection_state -ne 'not-connected' -or $clients[0].service_running -or $clients[0].handle -ne 1) { throw 'Expected disconnected original logger IPMI client evidence missing' }
                        if ($requests.Count -ne 1 -or !$requests[0].callback_status_writable -or $requests[0].callback_status_written -or $requests[0].transport_result_written) { throw 'Connection request must not fabricate a response' }
                        if ($lookups.Count -ne 1 -or $lookups[0].lookup_error -ne 'ServerNotFound' -or $lookups[0].server_handle -ne $null -or $lookups[0].session_created) { throw 'Expected absent-server lookup boundary missing' }
                        if ($operations.Count -ne 1 -or $operations[0].operation -ne 0x400 -or $operations[0].parameter_bytes -ne 32 -or $fail[0].reason -ne 'Unsupported IPMI operation 0x400; no success response fabricated') { throw 'Expected unsupported IPMI operation boundary missing' }
                    }
                } elseif ($attributedMutex.Count -ne 1 -or !$attributedMutex[0].attribute_identity_verified -or !$attributedMutex[0].output_writable -or $attributedMutex[0].mutex_created -or !$attributedMutex[0].synthetic_host_attribute) { throw 'Deferred attributed-mutex boundary evidence missing or unexpected' }
            }
        } elseif ($Copy) {
            if ($copies.Count -ne 1 -or $copies[0].count -ne 32 -or $copies[0].source_bytes_examined -ne 27 -or !$copies[0].bounded_host_bridge) { throw 'Expected bounded strncpy evidence missing or unexpected' }
        } elseif ($copies.Count -ne 0) { throw 'strncpy bridge must remain disabled before -Copy mode' }
    } else {
        $expectedAllocations = if ($Allocator) {1}else{0}
        if ($allocations.Count -ne $expectedAllocations -or $allocationFailures.Count -ne 0 -or $operatorNews.Count -ne 0 -or $operatorNewFailures.Count -ne 0) { throw 'Unexpected allocation events in baseline logger mode' }
        if (@($events | Where-Object event -eq 'logger-bounded-strncpy').Count -ne 0) { throw 'strncpy bridge must remain disabled in earlier modes' }
        if ($Allocator -and ($allocations[0].requested_bytes -ne 4096 -or $allocations[0].charged_bytes -ne 4096 -or $allocations[0].live_allocations -ne 1 -or !$allocations[0].synthetic_host_allocator)) { throw 'Expected bounded owned logger allocation missing' }
    }
}
if (!$hasAttributes -and ($attributesCaptured.Count -ne 0 -or $attributesInitialized.Count -ne 0 -or $attributeReference.Count -ne 0 -or $attributedMutex.Count -ne 0)) { throw 'Mutex-attribute bridges must remain disabled outside attribute modes' }
if (!$MutexCreate -and !$MutexLifecycle -and ($mutexesInitialized.Count -ne 0 -or $attributedMutexes.Count -ne 0 -or $attributesDestroyed.Count -ne 0)) { throw 'Mutex lifecycle bridge must remain disabled outside its modes' }
$path
