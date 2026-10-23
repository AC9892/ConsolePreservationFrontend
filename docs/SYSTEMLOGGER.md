# Original SystemLogger component checkpoint

This development utility executes a hash-pinned original SystemLogger client library supplied locally by its owner. It is not an authentic SystemLogger server, ShellCore execution, or a working dashboard.

## Loader and native evidence

The bounded loader accepts only the researched original IPMI and SystemLogger hashes. It holds the original IPMI mapping alive while the logger mapping is used, links only the three allowlisted full SCE function-import keys, and validates module entry points against file-backed executable load segments. A module-relative entry offset of zero is valid for these pinned files.

Original SystemLogger module initialization returns zero. Its public initialize export then encounters unresolved dependencies. Three opt-in stages preserve the evidence:

- `--logger-initialize`: explicit stop at missing malloc (`gQX+4GDQjpM`).
- `--logger-allocate`: services one 4096-byte malloc request, then explicitly stops at missing operator new (`fJnpuVVBbKk`).
- `--logger-new`: services the 4096-byte malloc and an 8-byte operator-new request, then explicitly stops at missing strncpy (`6sJWiWSRuqk`).

These modes exit 86 at their deliberate unsupported-import boundary; producing a report is not successful initialization. Source module mappings and allocator outputs are private process-owned copies. Original dump files are never opened for writing.

## Diagnostic allocator limits

The allocator is restricted to the pinned logger and opted-in modes. Each request is checked and page-rounded, with at most 32 live allocations and 1 MiB of total charged storage. Zero-sized requests receive a distinct minimal allocation. malloc failure returns null and sets diagnostic errno 12.

The original throwing operator-new caller passes the returned pointer directly to its constructor. The bridge therefore supplies successful owned allocations only. On failure it explicitly stops the child: it neither returns null to original code nor fabricates guest exception unwinding. `--logger-new-failure` exercises this unsupported exception boundary synthetically after IPMI module initialization; it does not execute the logger library.

Allocation mappings are retained until worker exit. Matching free/delete and original logger cleanup have not been established. This is a bounded diagnostic allocator, not a PS4 heap implementation. Earlier synthetic raw/generic IPMI tests remain independent transport checks, not authentic service implementations.

## Next work

Resolve the measured strncpy dependency with bounded pointer/length semantics, then capture the next concrete failure. Establish matching allocation/deallocation ownership and mutex attributes before claiming successful logger cleanup. Independently review each added contract and preserve deliberate failure probes. Backend integration and authentic service providers remain unresolved.
