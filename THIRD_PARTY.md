# Third-party notes

This source snapshot does not bundle console firmware, proprietary SDK material, shadPS4 or RPCSX source, downloaded Python packages, or compiled executables.

Rust dependency versions and source locations are locked in Cargo.lock. docs/DEPENDENCY_LICENSES.tsv records license expressions and repositories from Cargo metadata; it is metadata rather than a replacement for required license texts. Notable direct dependencies include Tauri (MIT OR Apache-2.0), rfd (MIT), serde/serde_json/sha2 (MIT OR Apache-2.0), walkdir (Unlicense OR MIT), regex (MIT OR Apache-2.0), and windows-sys (MIT OR Apache-2.0). Before binary redistribution, collect all applicable dependency notices and review WebView2 distribution requirements.

shadPS4 is a separately obtained external backend: https://github.com/shadps4-emu/shadPS4 . Its GPL-2.0-or-later notices and licensing files must be reviewed before integration or redistribution. No backend changes are distributed here.

RPCSX was consulted as a transport research reference, not copied into this implementation: https://github.com/RPCSX/rpcsx . The researched revision was e8ae1481ab7ba04d5c6bef89dd852aabba2c88ff. Its GPLv2 root license and separate historical orbis-kernel MIT provenance require per-file verification before any future reuse. No RPCSX implementation source is compiled or distributed here.

Optional Python analysis tools import separately installed Capstone 5.0.6 and pefile 2024.8.26. Their packages and notices are not included. The publication icon is an original solid-color geometric asset generated for this snapshot, containing no console artwork.

No project-wide open-source license grant is asserted by this snapshot; the owner has not selected one. Dependency licenses remain applicable to their respective projects.
