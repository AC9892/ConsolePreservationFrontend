# Original SystemLogger component checkpoint

This development utility executes a hash-pinned original SystemLogger client library supplied locally by its owner. It is not an authentic SystemLogger server, ShellCore execution, or a working dashboard.

## Loader and native evidence

The bounded loader accepts only the researched original IPMI and SystemLogger hashes. It holds the original IPMI mapping alive while the logger mapping is used, links only the three allowlisted full SCE function-import keys, and validates module entry points against file-backed executable load segments. A module-relative entry offset of zero is valid for these pinned files.

Original SystemLogger module initialization returns zero. Its public initialize export then encounters unresolved dependencies. Separate opt-in stages preserve the evidence:

- `--logger-initialize`: explicit stop at missing malloc (`gQX+4GDQjpM`).
- `--logger-allocate`: services one 4096-byte malloc request, then explicitly stops at missing operator new (`fJnpuVVBbKk`).
- `--logger-new`: services the 4096-byte malloc and an 8-byte operator-new request, then explicitly stops at missing strncpy (`6sJWiWSRuqk`).

- `--logger-copy`: additionally completes one bounded 32-byte strncpy call (27 source bytes examined), then explicitly stops at missing scePthreadMutexattrInit (`F8bUHwAG284`).
- `--logger-attr-probe`: captures the writable eight-byte output slot on the worker stack, then stops without creating an attribute.
- `--logger-attr-init`: verifies the exact reference-kernel hash, creates a registered diagnostic 12-byte attribute object containing structural words 1/0/0, then stops before attributed mutex creation. The reference kernel is not executed and these values are not claimed as verified live semantics.
- `--logger-mutex-create`: validates the attribute identity and creates an unnamed owned Windows mutex, then stops at missing attribute destruction (`smWEktiyyG0`).
- `--logger-mutex-lifecycle`: releases only the registered attribute mapping and clears its original output slot. Original logger then creates an original IPMI client for SceSystemLoggerService. No server is registered; connection operation `0x400` explicitly stops without writing callback-status or transport-result outputs.

These modes exit 86 at their deliberate unsupported-import boundary; producing a report is not successful initialization. Source module mappings and allocator outputs are private process-owned copies. Original dump files are never opened for writing.

## Diagnostic allocator limits

The allocator is restricted to the pinned logger and opted-in modes. Each request is checked and page-rounded, with at most 32 live allocations and 1 MiB of total charged storage. Zero-sized requests receive a distinct minimal allocation. malloc failure returns null and sets diagnostic errno 12.

The original throwing operator-new caller passes the returned pointer directly to its constructor. The bridge therefore supplies successful owned allocations only. On failure it explicitly stops the child: it neither returns null to original code nor fabricates guest exception unwinding. `--logger-new-failure` exercises this unsupported exception boundary synthetically after IPMI module initialization; it does not execute the logger library.

General allocation mappings are retained until worker exit. The mutex-lifecycle mode releases only its precisely registered attribute mapping; the other two allocations and separately owned mutex remain until worker exit. Matching general free/delete and original logger cleanup have not been established. This is a bounded diagnostic allocator, not a PS4 heap implementation. Earlier synthetic raw/generic IPMI tests remain independent transport checks, not authentic service implementations.

## Bounded string copy

The opt-in strncpy bridge accepts at most 256 bytes, validates the complete writable destination, reads only through the source NUL or count, implements truncation and zero padding, and rejects overlapping copies. Inputs are checked before any destination writes. Zero count performs no pointer access. Guard, invalid-memory, padding and mapping-boundary tests cover the bridge.

## Next work

The current concrete boundary is a missing authentic service provider, not a successful connection. All nine logger probes and 32 transport workers passed against the client-result transaction checkpoint using the existing independently authored validation scripts; 29 component tests and all-target Clippy passed. No successful logger initialize return or original logger teardown is claimed.

`tools/analyze_logger_provider.py ORIGINAL_SHELLCORE` records hash-pinned static provider startup evidence as JSON on stdout. It verifies server creation at module-relative `0x4f2938`, worker entry `0x4f2ea0` calling server vtable slot `+0x10`, and the internal thread-setup wrapper. The original setup requests a 32768-byte stack and scheduling parameter 766 and imports PS4 thread attribute, scheduling, creation and exit APIs. Raw instruction reports belong outside the dump and must remain private.

Establish callback registration, prerequisite global state and process-entry reachability before attempting an original provider probe. Preserve process identity and backend IPC boundaries; a synthetic server or separately started host process does not establish an authentic provider. Kernel mutex semantics, backend integration, service execution and dashboard execution remain unresolved.

## Provider construction evidence

The analyzer now follows the original module initialization table to the logger callback constructor. A bounded legacy table has 262 slots before its sentinel; one entry points to the constructor that installs the callback table and requests a 48-byte allocation. Seven local callback targets and the constructor entry have eight verified relative relocations. Table membership does not prove that preceding constructors complete. The worker separately requires a valid server pointer, temporary working-memory pointer and its size.

The analyzer verifies the parsed snapshot's hash as well as the initial read, refusing changed input between reads. It checks both service-name fragment loads and reads file-backed SCE RELRO without inventing BSS contents. Six synthetic checks cover these integrity and memory-boundary properties without console fixtures:

```powershell
python -m unittest discover -s tools -p test_logger_provider.py
```

The optional Capstone analysis dependency is required. Original server, constructor and method execution remain unverified. The next backend investigation should retain the original IPMI library and target its measured `ipmimgr_call` kernel boundary (NID `Hk7iHmGxB18`), with explicit ownership, guest-memory validation and separate transport/error-result channels. Missing HLE C++ APIs alone do not justify replacing the original client/server library.

The operation-2 input decoder is now shared with the native component harness through an explicit memory-reader interface; see [memory boundary](IPMI_MEMORY_BOUNDARY.md). Static callback analysis also identifies an authorization-based classification gate before method routing. Its caller identity and authentic authorization semantics remain unresolved. The backend's fixed PID and capability stub cannot establish authentic classification. No original method handler has executed successfully.
