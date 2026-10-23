# PS4 â€” experimental system software host

Independent Windows application named **PS4.exe**, built with Rust and Tauri. The objective is incremental execution of authentic PlayStation 4 system software. No recreated dashboard or centralized console launcher is included.

**Authentic dashboard execution and rendering remain unverified.** This source snapshot is experimental development work, not a working PS4 system emulator.

## Current behavior

- Select local dump and pinned external shadPS4 executable paths; prepare a fresh writable copy and attempt a bounded ShellUI launch.
- Check backend hashes and Windows memory availability; collect structured execution diagnostics.
- Inspect local firmware read-only through PS4Inspector: SHA-256, file formats, SFO metadata, SCE imports/exports and dependency candidates.
- Capture bounded debugger traces with PS4Trace.
- Exercise original IPMI library code using PS4ModuleTest with explicitly synthetic diagnostic endpoints. Connection, session lifecycle, separate-thread raw and generic multi-buffer request/reply, guarded client buffers, error propagation and cleanup are checked. The generic test uses three synthetic inputs and one four-byte output; it does not execute the original SystemLogger library or service.

Original ShellUI startup instructions have executed in controlled external-backend experiments, but missing IPMI transport and system services prevent a verified dashboard. The component IPMI transport is not integrated into shadPS4. A synthetic test handler is not an authentic ShellCore service.

## Build

Windows 11, Rust MSVC, Visual Studio C++ Build Tools and WebView2 are required.

```powershell
./build-release.ps1
cargo clippy --release --all-targets --locked -- -D warnings
```

The build script builds/tests and copies PS4.exe, PS4Inspector.exe, PS4Trace.exe and PS4ModuleTest.exe to dist. No Node development server is needed. Source code and dependency declarations are provided; third-party runtimes and backend executables must be obtained separately under their licenses.

## Local use

Open dist/PS4.exe and choose your dump root, verified external backend and writable sandbox parent outside the dump. Dumps are opened for reading; execution uses copied files. This copy boundary is not an OS security sandbox. Prepared networking flags are disabled, but this is not OS-enforced network isolation. Attempts are bounded to 20 seconds.

```powershell
./dist/PS4Inspector.exe DUMP_ROOT --output NEW_REPORT_PATH
./tools/test-native-ipmi.ps1 -DumpRoot DUMP_ROOT
python tools/analyze_logger_contract.py PATH_TO_LOCAL_SYSTEMLOGGER_MODULE
```

The output parent must exist and reports must remain outside the dump. Native component experiments accept an exact researched module hash and execute private native mappings; they are not safe isolation for hostile code. Optional Python research tools use separately installed Capstone/pefile from local/python-tools.

## Validation and remaining work

The current component build, 17 component tests and all-target Clippy passed. The transport regression suite passed 32 controlled workers; four logger probes passed their exact expected boundaries (36 controlled workers total). Separate logger probes record exact initialization dependency boundaries. Firmware-dependent evidence remains private and is not bundled.

The original SystemLogger library now loads alongside the original IPMI provider, completes its module entry and begins public initialization. An opt-in bounded malloc/operator-new bridge advances to missing strncpy; initialization has not succeeded. Next work: resolve further bounded libc/mutex dependencies, identify authentic service providers and evaluate backend process/service integration. The current generic transport supports only the tested synthetic request shape. No successful firmware boot is claimed.

See [SystemLogger execution limits](docs/SYSTEMLOGGER.md) for the original-library checkpoint and allocator failure policy.

## Publication boundary

Only original source, configuration, synthetic fixtures and sanitized documentation are included. Firmware, games, keys, accounts, raw logs, dump inventories, private checkpoints, Agent Talk, backend sources and binaries are excluded. No compiled release is published with this snapshot. See [third-party notes](THIRD_PARTY.md) and [dependency metadata](docs/DEPENDENCY_LICENSES.tsv).
