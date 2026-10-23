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
$modes += '--threaded-multibuffer-request'
$modes += '--threaded-multibuffer-dispatch'
$modes += '--threaded-multibuffer-reply-probe'
for ($index=0; $index -lt $RpcRepetitions; $index++) { $modes += '--threaded-rpc' }
$modes += '--threaded-rpc-error'
for ($index=0; $index -lt $RpcRepetitions; $index++) {
    $modes += '--threaded-multibuffer-rpc'
    $modes += '--threaded-multibuffer-rpc-error'
}
foreach ($mode in $modes) {
    $arguments = @($DumpRoot,$ReportParent)
    if ($mode) { $arguments += $mode }
    $reportPath = & $ToolPath @arguments
    if ($LASTEXITCODE -ne 0 -or @($reportPath).Count -ne 1) { throw "Harness failed: $mode" }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $events = @(Get-Content -LiteralPath $report.events | ForEach-Object { $_ | ConvertFrom-Json })
    $expected = if ($mode -in @('', '--server','--handshake','--threaded-handshake','--threaded-rpc','--threaded-rpc-error','--threaded-multibuffer-rpc','--threaded-multibuffer-rpc-error')) { 0 } else { 86 }
    if ($report.exit_code -ne $expected -or $report.timed_out -or $report.authentic_dashboard_verified) { throw "Unexpected worker result: $mode" }
    if ($mode -eq '--connect') {
        $lookup = @($events | Where-Object event -eq 'native-connection-server-lookup')
        $unsupported = @($events | Where-Object event -eq 'unsupported-ipmi-operation')
        $failure = @($events | Where-Object event -eq 'failure')
        if (@($events | Where-Object event -eq 'ipmi-server-registered').Count -ne 0 -or $lookup.Count -ne 1 -or $lookup[0].lookup_error -ne 'ServerNotFound' -or $lookup[0].server_handle -ne $null -or $lookup[0].session_created) { throw 'Client-only connect must stop at the missing-server lookup' }
        if ($unsupported.Count -ne 1 -or $unsupported[0].operation -ne 0x400 -or $failure.Count -ne 1 -or $failure[0].reason -ne 'Unsupported IPMI operation 0x400; no success response fabricated' -or @($events | Where-Object event -eq 'native-connection-completion-accepted').Count -ne 0) { throw 'Client-only connect must not fabricate connection completion' }
    }
    if ($mode -in @('--server','--server-connect','--server-dispatch','--server-session')) {
        $registered = @($events | Where-Object event -eq 'ipmi-server-registered')
        if ($registered.Count -ne 1 -or $registered[0].service -ne 'PS4IpcTest' -or !$registered[0].synthetic_test_endpoint -or $registered[0].original_shell_service) { throw "Expected synthetic test server registration missing: $mode" }
    }
    if ($mode -eq '--server') {
        $destroyed = @($events | Where-Object event -eq 'native-server-destroy-returned')
        if ($destroyed.Count -ne 1 -or $destroyed[0].status -ne 0 -or !$destroyed[0].registry_empty -or !$destroyed[0].mutex_registry_empty -or !$destroyed[0].native_handle_invalidated -or @($events | Where-Object event -eq 'server-component-test-complete').Count -ne 1 -or @($events | Where-Object event -eq 'ipmi-client-created').Count -ne 0) { throw 'Server-only probe must register and clean up its synthetic endpoint' }
    }
    if ($mode -eq '--server-connect') {
        $lookup = @($events | Where-Object event -eq 'native-connection-server-lookup')
        $unsupported = @($events | Where-Object event -eq 'unsupported-ipmi-operation')
        $failure = @($events | Where-Object event -eq 'failure')
        if ($lookup.Count -ne 1 -or $lookup[0].lookup_error -ne $null -or $lookup[0].server_handle -ne $registered[0].handle -or $lookup[0].session_created) { throw 'Server-connect probe must find its synthetic server without creating a session' }
        if ($unsupported.Count -ne 1 -or $unsupported[0].operation -ne 0x400 -or $failure.Count -ne 1 -or $failure[0].reason -ne 'Unsupported IPMI operation 0x400; no success response fabricated' -or @($events | Where-Object event -eq 'native-connection-completion-accepted').Count -ne 0) { throw 'Server-connect probe must stop before connection completion' }
    }
    if ($mode -eq '--server-dispatch') {
        $lookup = @($events | Where-Object event -eq 'native-connection-server-lookup')
        $packet = @($events | Where-Object event -eq 'native-server-received-connection-packet')
        $callback = @($events | Where-Object event -eq 'native-server-connection-callback-reached')
        $failure = @($events | Where-Object event -eq 'failure')
        if ($lookup.Count -ne 1 -or $lookup[0].lookup_error -ne $null -or $lookup[0].server_handle -ne $registered[0].handle -or $lookup[0].session_created -or $packet.Count -ne 1 -or $packet[0].packet_type -ne 1 -or $packet[0].synthetic_test_endpoint -ne $true) { throw 'Server-dispatch probe must receive only the synthetic connection packet' }
        if ($callback.Count -ne 1 -or !$callback[0].synthetic_test_endpoint -or $callback[0].original_shell_service_verified -or $callback[0].session_created -or $failure.Count -ne 1 -or $failure[0].reason -ne 'Original server dispatch reached the test callback; native session allocation is not implemented') { throw 'Server-dispatch probe crossed its explicit session-allocation boundary' }
    }
    if ($mode -eq '--server-session') {
        $callback = @($events | Where-Object event -eq 'native-server-connection-callback-reached')
        $created = @($events | Where-Object event -eq 'native-session-create-returned')
        $destroyed = @($events | Where-Object event -eq 'ipmi-session-destroyed')
        $failure = @($events | Where-Object event -eq 'failure')
        if ($callback.Count -ne 1 -or !$callback[0].synthetic_test_endpoint -or $callback[0].original_shell_service_verified -or $created.Count -ne 1 -or $created[0].status -ne 0 -or !$created[0].registry_matches_native_object -or !$created[0].synthetic_test_endpoint -or $created[0].connection_completed -or $destroyed.Count -ne 1 -or $destroyed[0].handle -ne $created[0].handle) { throw 'Server-session probe must create and destroy only its synthetic session' }
        if ($failure.Count -ne 1 -or $failure[0].reason -ne 'Native session lifecycle verified; connection completion and concurrent dispatch remain unsupported' -or @($events | Where-Object event -eq 'native-connection-completion-accepted').Count -ne 0) { throw 'Server-session probe must stop before connection completion' }
    }
    if ($mode -in @('--handshake','--threaded-handshake','--threaded-request','--threaded-multibuffer-request','--threaded-multibuffer-dispatch','--threaded-multibuffer-reply-probe','--threaded-rpc','--threaded-rpc-error','--threaded-multibuffer-rpc','--threaded-multibuffer-rpc-error')) {
        $connect = @($events | Where-Object event -eq 'native-connect-returned')
        $complete = @($events | Where-Object event -eq 'native-connection-completion-accepted')
        if ($connect.Count -ne 1 -or $connect[0].status -ne 0 -or $connect[0].callback_status -ne 0 -or $complete.Count -ne 1) { throw "Native completion evidence missing: $mode" }
        if ($mode -notin @('--threaded-request','--threaded-multibuffer-request','--threaded-multibuffer-dispatch','--threaded-multibuffer-reply-probe')) {
            $cleanup = @($events | Where-Object event -eq 'native-server-destroy-returned')
            if ($cleanup.Count -ne 1 -or !$cleanup[0].registry_empty -or !$cleanup[0].mutex_registry_empty -or $cleanup[0].status -ne 0) { throw "Native cleanup evidence missing: $mode" }
        }
    }
    if ($mode -in @('--threaded-handshake','--threaded-request','--threaded-multibuffer-request','--threaded-multibuffer-dispatch','--threaded-multibuffer-reply-probe','--threaded-rpc','--threaded-rpc-error','--threaded-multibuffer-rpc','--threaded-multibuffer-rpc-error')) {
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
    if ($mode -in @('--threaded-multibuffer-rpc','--threaded-multibuffer-rpc-error')) {
        $generic = @($events | Where-Object event -eq 'native-generic-dispatch-verified')
        $roundtrip = @($events | Where-Object event -eq 'native-generic-roundtrip-verified')
        $reply = @($events | Where-Object event -eq 'native-generic-reply-captured')
        $accepted = @($events | Where-Object event -eq 'native-sync-response-accepted')
        $auth = @($events | Where-Object event -eq 'synthetic-process-authinfo-unavailable')
        $failures = @($events | Where-Object event -eq 'failure')
        $sessionDestroyed = @($events | Where-Object event -eq 'ipmi-session-destroyed')
        $connectionDestroyed = @($events | Where-Object event -eq 'connected-session-destroyed')
        $clientDestroyed = @($events | Where-Object event -eq 'native-client-destroy-returned')
        $serverDestroyed = @($events | Where-Object event -eq 'native-server-destroy-returned')
        $expectedMethodStatus = if ($mode -eq '--threaded-multibuffer-rpc-error') {-123}else{0}
        $expectedLength = if ($mode -eq '--threaded-multibuffer-rpc-error') {0}else{4}
        if ($generic.Count -ne 1 -or !$generic[0].exact_payload_match -or $generic[0].method_id -ne 0x03299000 -or $generic[0].callback_slot -ne 0x10 -or ($generic[0].input_bytes -join ',') -ne '8,16,100' -or $generic[0].output_capacity -ne 4 -or $generic[0].response_written -or $generic[0].original_logger_library_executed -or $generic[0].original_shell_service -or $generic[0].authentic_dashboard_verified) { throw 'Generic callback evidence missing or incorrect' }
        if ($roundtrip.Count -ne 1 -or $roundtrip[0].transport_status -ne 0 -or $roundtrip[0].method_status -ne $expectedMethodStatus -or $roundtrip[0].response_bytes -ne $expectedLength -or !$roundtrip[0].exact_payload_match -or !$roundtrip[0].guard_bytes_intact -or !$roundtrip[0].empty_response_buffer_unchanged -or $roundtrip[0].original_shell_service -or $roundtrip[0].original_logger_library_executed -or $roundtrip[0].authentic_dashboard_verified) { throw 'Generic native RPC result, length, payload, guards or evidence boundary mismatch' }
        if ($reply.Count -ne 1 -or $reply[0].method_status -ne $expectedMethodStatus -or $reply[0].capacity -ne 4 -or $reply[0].length -ne $expectedLength -or !$reply[0].exact_payload_match -or $reply[0].operation -ne 0x232 -or $reply[0].flags -ne 1 -or $reply[0].count -ne 1 -or $reply[0].client_outputs_written -or $reply[0].transport_result_written -or $reply[0].original_shell_service -or $reply[0].authentic_dashboard_verified) { throw 'Generic native response capture evidence missing or incorrect' }
        if ($accepted.Count -ne 1 -or $accepted[0].method_status -ne $expectedMethodStatus -or $accepted[0].response_bytes -ne $expectedLength -or !$accepted[0].synthetic_test_endpoint) { throw 'Generic native response delivery evidence missing or incorrect' }
        if ($auth.Count -ne 1 -or $auth[0].posix_errno -ne 78 -or $auth[0].authorization_data_written -or $failures.Count -ne 0) { throw 'Generic response delivery or unsupported authorization evidence missing' }
        if ($sessionDestroyed.Count -ne 1 -or $connectionDestroyed.Count -ne 1 -or $connectionDestroyed[0].status -ne 0 -or !$connectionDestroyed[0].native_handle_invalidated -or $clientDestroyed.Count -ne 1 -or $clientDestroyed[0].status -ne 0 -or !$clientDestroyed[0].native_handle_invalidated -or $serverDestroyed.Count -ne 1 -or $serverDestroyed[0].status -ne 0 -or !$serverDestroyed[0].registry_empty -or !$serverDestroyed[0].mutex_registry_empty) { throw 'Generic RPC session, client or server teardown evidence missing' }
    }
    if ($mode -eq '--threaded-multibuffer-request') {
        $call = @($events | Where-Object event -eq 'calling-native-multibuffer-method')
        $decoded = @($events | Where-Object event -eq 'native-sync-request-decoded')
        $unsupported = @($events | Where-Object event -eq 'unsupported-ipmi-operation')
        if ($call.Count -ne 1 -or $call[0].original_logger_library_executed -or $call[0].original_shell_service -or $decoded.Count -ne 1 -or $unsupported.Count -ne 1) { throw 'Multi-buffer evidence missing or mislabeled' }
        if ($decoded[0].request.method -ne 0x03299000 -or $decoded[0].request.flags -ne 1 -or $decoded[0].request.input_count -ne 3 -or $decoded[0].request.output_count -ne 1 -or ($decoded[0].input_bytes -join ',') -ne '8,16,100' -or ($decoded[0].output_capacities -join ',') -ne '4') { throw 'Original generic multi-buffer contract differs from expected layout' }
        if ($decoded[0].method_result_written -or $decoded[0].response_written -or $unsupported[0].operation -ne 0x320 -or $unsupported[0].parameter_bytes -ne 48) { throw 'Multi-buffer probe must stop without writing a response' }
    }
    if ($mode -eq '--threaded-multibuffer-dispatch') {
        $generic = @($events | Where-Object event -eq 'native-generic-dispatch-verified')
        $packet = @($events | Where-Object event -eq 'native-server-received-sync-packet')
        $failures = @($events | Where-Object event -eq 'failure')
        if ($generic.Count -ne 1 -or !$generic[0].exact_payload_match -or $generic[0].method_id -ne 0x03299000 -or $generic[0].callback_slot -ne 0x10 -or ($generic[0].input_bytes -join ',') -ne '8,16,100' -or $generic[0].output_capacity -ne 4 -or $generic[0].response_written -or $generic[0].original_logger_library_executed -or $generic[0].original_shell_service -or $generic[0].authentic_dashboard_verified) { throw 'Generic callback evidence missing or incorrect' }
        if ($packet.Count -ne 1 -or $packet[0].packet_type -ne 0x8041 -or $packet[0].packet_bytes -ne 164 -or $packet[0].input_size -ne 160 -or $failures.Count -ne 1 -or $failures[0].reason -ne 'Generic native dispatch verified; reply ABI remains unsupported') { throw 'Generic dispatch must stop at the exact unsupported reply boundary' }
    }
    if ($mode -eq '--threaded-multibuffer-reply-probe') {
        $generic = @($events | Where-Object event -eq 'native-generic-dispatch-verified')
        $reply = @($events | Where-Object event -eq 'native-generic-reply-captured')
        $failure = @($events | Where-Object event -eq 'failure')
        if ($generic.Count -ne 1 -or !$generic[0].exact_payload_match -or $reply.Count -ne 1 -or $reply[0].operation -ne 0x232 -or $reply[0].parameter_bytes -ne 24 -or $reply[0].flags -ne 1 -or $reply[0].count -ne 1 -or $reply[0].method_status -ne 0 -or $reply[0].capacity -ne 4 -or $reply[0].length -ne 4 -or !$reply[0].exact_payload_match -or $reply[0].client_outputs_written -or $reply[0].transport_result_written -or $reply[0].original_shell_service -or $reply[0].authentic_dashboard_verified) { throw 'Native generic response capture is missing or incorrect' }
        if ($failure.Count -ne 1 -or $failure[0].reason -ne 'Generic native reply captured; delivery remains unsupported') { throw 'Generic reply probe must stop before delivering a fabricated response' }
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
