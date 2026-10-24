# Optional Xbox 360 monitor and preservation library

This is the existing CPF Xbox 360 Rust application source. It is a supporting utility, not the dashboard execution window or a replacement dashboard. The authentic dashboard runs separately in Xenia Canary. CPF Console Settings remains responsible for launching Xenia; that separate frontend integration is not included in this module.

## Files

- `src/bin/xbox.rs`: native Windows `Xbox.exe` window, Live monitor and Firmware tools tabs.
- `src/bin/xbox/monitor.rs`: process detection, launch/exit observations, CPU/RAM/thread metrics, bounded log-tail reading and diagnostic redaction.
- `src/lib.rs`: read-only inventory, bounded header inspection, duplicate hashes, change verification, preview restrictions, private catalog storage, optional router API and synthetic tests.
- `src/bin/validate.rs`: explicitly selected local-copy validation with aggregate output.
- `src/compatibility.json`: descriptive compatibility record; it is not an automatic execution test.
- `Cargo.toml` and `Cargo.lock`: build dependencies and pinned crate versions.
- `tools/build-release.ps1`: existing local Windows build script; its output is excluded from Git.

## Build and test

Use current stable Rust with the Windows MSVC target, Visual Studio C++ build tools and Windows SDK. From this directory:

```powershell
cargo fetch --locked
cargo test --offline --locked
cargo build --offline --locked --release --bin Xbox
```

The first command obtains the Cargo.lock dependencies from the Rust package registry. It is unnecessary if they are already cached. The build script also works after dependencies are cached:

```powershell
./tools/build-release.ps1
```

It produces a local `Xbox.exe`; no executable is committed or released. This source publication does not start the monitor or emulator automatically. The `Xbox` binary does not start an HTTP server. The library retains router functions for an external caller, but this checkout contains no server launcher or web frontend.

## Data handling and boundaries

Firmware tools reads explicitly selected originals; it does not decrypt or execute them. A private catalog is stored outside the source checkout under `%LOCALAPPDATA%/CPF`, or the directory supplied through `CPF_PRIVATE_LIBRARY`. The catalog includes source paths and hashes and must stay private. Parent imports skip the specially named restricted directory checked by the implementation; direct manual selection is required for it. Source tests exercise this rule using a newly created synthetic directory, not a real backup.

Log redaction is intended to reduce accidental disclosure in the display, not to certify every diagnostic as safe to share. Raw emulator logs, crash dumps, profiles, handoff files and private catalogs are never publication inputs. The monitor reports observed process behavior; an exit code alone does not prove the cause of a crash. Firmware header recognition is not full integrity, ECC, motherboard or operating-system compatibility verification.

The synthetic tests create their own fixtures under this crate's `target` directory. They do not require or read original console dumps. This copy preserves the existing implementation; the descriptive compatibility record and this README have been refreshed for publication. No UI redesign or new monitor features are included.
