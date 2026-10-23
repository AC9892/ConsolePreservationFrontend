param(
    [Parameter(Mandatory=$true)][string]$DumpRoot,
    [string]$ToolPath = (Join-Path $PSScriptRoot '..\dist\PS4ModuleTest.exe'),
    [string]$ReportParent = (Join-Path $PSScriptRoot '..\local\sandboxes'),
    [ValidateRange(1,20)][int]$ThreadedRepetitions = 5,
    [ValidateRange(1,20)][int]$RpcRepetitions = 5
)
$ErrorActionPreference = 'Stop'
$runs = [Collections.Generic.List[object]]::new()
$modes = @('', '--connect', '--server', '--server-connect', '--server-dispatch', '--server-session', '--handshake')
for ($index=0; $index -lt $ThreadedRepetitions; $index++) { $modes += '--threaded-handshake' }
$modes += '--threaded-request'
for ($index=0; $index -lt $RpcRepetitions; $index++) { $modes += '--threaded-rpc' }
$modes += '--threaded-rpc-error'
foreach ($mode in $modes) {
    $arguments = @($DumpRoot,$ReportParent)
    if ($mode) { $arguments += $mode }
    $reportPath = & $ToolPath @arguments
    if ($LASTEXITCODE -ne 0 -or @($reportPath).Count -ne 1) { throw "Harness failed: $mode" }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $events = @(Get-Content -LiteralPath $report.events | ForEach-Object { $_ | ConvertFrom-Json })
    $expected = if ($mode -in @('', '--server','--handshake','--threaded-handshake','--threaded-rpc','--threaded-rpc-error')) { 0 } else { 86 }
    if ($report.exit_code -ne $expected -or $report.timed_out -or $report.authentic_dashboard_verified) { throw "Unexpected worker result: $mode" }
    if ($mode -in @('--handshake','--threaded-handshake','--threaded-request','--threaded-rpc','--threaded-rpc-error')) {
        $connect = @($events | Where-Object event -eq 'native-connect-returned')
        $complete = @($events | Where-Object event -eq 'native-connection-completion-accepted')
        if ($connect.Count -ne 1 -or $connect[0].status -ne 0 -or $connect[0].callback_status -ne 0 -or $complete.Count -ne 1) { throw "Native completion evidence missing: $mode" }
        if ($mode -ne '--threaded-request') {
            $cleanup = @($events | Where-Object event -eq 'native-server-destroy-returned')
            if ($cleanup.Count -ne 1 -or !$cleanup[0].registry_empty -or !$cleanup[0].mutex_registry_empty -or $cleanup[0].status -ne 0) { throw "Native cleanup evidence missing: $mode" }
        }
    }
    if ($mode -in @('--threaded-handshake','--threaded-request','--threaded-rpc','--threaded-rpc-error')) {
        $client = @($events | Where-Object event -eq 'native-client-waiting-for-server')
        $server = @($events | Where-Object event -eq 'calling-native-server-try-dispatch')
        if ($client.Count -ne 1 -or $server.Count -ne 1 -or !$client[0].host_thread -or !$server[0].host_thread -or $client[0].host_thread -eq $server[0].host_thread) { throw 'Distinct native client/server thread evidence missing' }
    }
    if ($mode -in @('--threaded-rpc','--threaded-rpc-error')) {
        $roundtrip = @($events | Where-Object event -eq 'native-sync-roundtrip-verified')
        $reply = @($events | Where-Object event -eq 'native-sync-respond-returned')
        $auth = @($events | Where-Object event -eq 'synthetic-process-authinfo-unavailable')
        $expectedMethodStatus = if ($mode -eq '--threaded-rpc-error') {-123}else{0}
        $expectedLength = if ($mode -eq '--threaded-rpc-error') {0}else{16}
        if ($roundtrip.Count -ne 1 -or !$roundtrip[0].exact_payload_match -or !$roundtrip[0].guard_bytes_intact -or $roundtrip[0].method_status -ne $expectedMethodStatus -or $roundtrip[0].response_bytes -ne $expectedLength -or $roundtrip[0].original_shell_service -or $roundtrip[0].authentic_dashboard_verified) { throw 'Native diagnostic RPC evidence missing or mislabeled' }
        if ($reply.Count -ne 1 -or $reply[0].status -ne 0 -or $auth.Count -ne 1 -or $auth[0].posix_errno -ne 78 -or $auth[0].authorization_data_written) { throw 'Native response or unsupported authorization evidence missing' }
    }
    if ($mode -eq '--server-session') {
        $created = @($events | Where-Object event -eq 'native-session-create-returned')
        $destroyed = @($events | Where-Object event -eq 'native-session-destroy-returned')
        if ($created.Count -ne 1 -or $created[0].status -ne 0 -or !$created[0].registry_matches_native_object -or $destroyed.Count -ne 1 -or $destroyed[0].status -ne 0 -or !$destroyed[0].native_handle_invalidated -or !$destroyed[0].session_registry_entry_removed) { throw 'Native session lifecycle evidence missing' }
    }
    if ($mode -eq '--threaded-request') {
        $decoded = @($events | Where-Object event -eq 'native-sync-request-decoded')
        $unsupported = @($events | Where-Object event -eq 'unsupported-ipmi-operation')
        if ($decoded.Count -ne 1 -or $decoded[0].method_result_written -or $decoded[0].response_written -or $unsupported.Count -ne 1 -or $unsupported[0].operation -ne 0x320 -or $unsupported[0].parameter_bytes -ne 48) { throw 'Expected bounded synchronous request failure missing' }
        if (@($decoded[0].input_bytes).Count -ne 1 -or $decoded[0].input_bytes[0] -ne 0 -or @($decoded[0].output_capacities).Count -ne 1 -or $decoded[0].output_capacities[0] -ne 16) { throw 'Native method buffer descriptors differ from requested bounds' }
    }
    $runs.Add([pscustomobject]@{mode=$mode;child_exit=$report.exit_code;timed_out=$report.timed_out;report=$reportPath})
}
$summaryPath = Join-Path $ReportParent ('native-ipmi-validation-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + '.json')
[pscustomobject]@{authentic_dashboard_verified=$false;original_shell_service_verified=$false;threaded_repetitions=$ThreadedRepetitions;rpc_repetitions=$RpcRepetitions;runs=$runs} |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath
$summaryPath
