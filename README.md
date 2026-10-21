# CPF DSi (alpha)

This branch contains the unpacked CPF DSi source and Windows binaries. The public build excludes the unfinished DSi Menu game-tile work.

## Windows files

Keep `DSI.exe`, `DSi Backend.exe`, and `backend/melondsds_system_compat.dll` in the shown relative locations. The user must supply their own DSi BIOS, firmware, and NAND.

## Source

- `cpf-desktop/` — CPF Rust/Tauri application and runtime worker.
- `melonds-ds-libretro/` and `melonDS-core/` — emulation backend source.
- `BUILD_CLEAN_RELEASE.md` — build notes for this source snapshot.

No Nintendo system files, games, saves, or personal console data are in this branch. See the source directories for license and third-party notices.
