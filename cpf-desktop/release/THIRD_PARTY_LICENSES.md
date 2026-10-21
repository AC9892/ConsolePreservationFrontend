# Third party components

- The packaged emulation backend is based on melonDS DS libretro / melonDS code, under GNU GPL version 3. The full GPLv3 text is in `licenses/melonDS-GPL-3.0.txt`. The matching local modified source is in the separately prepared source archive. Its availability and license notices must be reviewed before publication.
- The native UI uses Tauri 2 and Rust crates with their respective licenses. The frontend uses `@tauri-apps/api` and Vite during build. Rust package license files are under `licenses/rust/`, with hashes in `MANIFEST.sha256`; `OVERRIDE_PROVENANCE.md` explains six source-revision overrides. `DEPENDENCY_INVENTORY.md` lists package versions and declared license expressions. A final dependency notice review is still required before publication.
- Windows WebView2 is supplied by Microsoft or the host system, not bundled here.

CPF-owned source code is licensed under GPL-3.0-only; the root `LICENSE` contains the full text. This declaration does not relicense third-party components or user-supplied console data. The public package uses original, unbranded shell art and a generated icon; development-only device artwork is excluded.
