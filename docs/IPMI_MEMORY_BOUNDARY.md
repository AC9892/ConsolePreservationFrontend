# Original IPMI client input boundary

`src/ipmi_memory.rs` copies and decodes the measured operation-2 input using a
caller-supplied memory reader. It contains no raw-pointer access, registry
mutation, guest output writes or backend-specific code. The native component
harness now uses this same path for original IPMI client creation.

The request remains 24 bytes with three opaque 64-bit addresses: original
client implementation, service name and descriptor. The decoder reads a
336-byte descriptor, validates its existing structural diagnostics, checks
16 readable bytes of implementation storage, and reads the service name one
byte at a time through its NUL, with a maximum of 24 content bytes. It checks
null addresses, arithmetic overflow, complete returned lengths and UTF-8.
The caller must reject memory outside its current guest/worker context.

The native caller uses the existing owned-region/current-worker-stack checks
and Windows page-access checks. The implementation object is now checked for
readability as well as ownership. The shared decoder keeps its address opaque;
it does not interpret object contents or treat that address as a host pointer.
Read checks explicitly permit readable protection values and reject execute-only
pages; a focused owned-page test covers rejection and executable-readable access.
The existing hash-pinned module gates, registry and result-write path remain
in the component harness. Client creation still does not mean a server exists.

Four synthetic tests cover high opaque addresses without host dereferences,
terminal NUL and maximum-length names, missing terminators, invalid UTF-8,
empty names, partial reads, wrong request length, overflow, descriptor errors
and unreadable implementation storage. They run with the component Rust tests.

This is an input-side extraction for a future adapter, not a shadPS4 integration.
The backend still needs a length-aware guest-memory provider, validated writable
results, process identity, kernel-scoped object ownership and both transport
error and operation-result channels. Unknown operations must remain unsupported.
The read callback alone does not establish an atomic guest-memory snapshot;
a backend caller must coordinate mappings and concurrent guest writes as needed.
No process authorization or authentic service behavior is supplied here.

## Synthetic client-result transaction

The component harness now uses `Kernel::create_with_output` for operation 2.
It validates the result range before decoding inputs and revalidates it at
the final handle write. If the writer refuses, the registry removes the new
client and restores the unconsumed handle. A rejected registry creation never
invokes the writer. Tests cover commit, rollback/reuse and registry rejection.

The writer must leave the output untouched when returning an error. This
callback contract does not make arbitrary guest writes atomic or protect a
real backend mapping against concurrent changes. Transport errno remains
separate from the four-byte operation result; unknown guest error mappings
remain unsupported. The original logger still stops at its missing server.
