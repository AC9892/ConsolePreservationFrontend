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

The current component build, 29 component tests and all-target Clippy passed. The client-result transaction checkpoint passed 32 transport workers and nine logger probes at their exact expected boundaries (41 controlled workers total, no timeouts), using the existing independently authored validation scripts. Firmware-dependent evidence remains private and is not bundled.

Provider research additionally has six synthetic analyzer regression checks. The latest transport-script update adds explicit early-stage failure and cleanup assertions and passed 16 focused worker runs against the unchanged component binary. These checks do not establish an authentic running service.

The original SystemLogger library now loads alongside original IPMI, completes its module entry and begins public initialization. Opt-in bounded allocation, string-copy and diagnostic mutex-attribute bridges advance to an original logger client connection request. It finds no SceSystemLoggerService server and stops at unsupported connection operation 0x400 without a fabricated response. Initialization has not succeeded. A read-only hash-pinned analysis tool records original ShellCore provider creation, worker dispatch and thread dependencies. Next work: establish callback registration and startup prerequisites, then evaluate backend process/service integration. No successful firmware boot is claimed.

See [SystemLogger execution limits](docs/SYSTEMLOGGER.md) for the original-library checkpoint and allocator failure policy.

The original client-create path now uses a reusable [memory input decoder](docs/IPMI_MEMORY_BOUNDARY.md) with opaque guest addresses and bounded copies. This prepares the kernel-call adapter; it does not integrate that adapter into shadPS4 or supply process authorization.

## Publication boundary

Only original source, configuration, synthetic fixtures and sanitized documentation are included. Firmware, games, keys, accounts, raw logs, dump inventories, private checkpoints, Agent Talk, backend sources and binaries are excluded. No compiled release is published with this snapshot. See [third-party notes](THIRD_PARTY.md) and [dependency metadata](docs/DEPENDENCY_LICENSES.tsv).
