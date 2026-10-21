# CPF Nintendo DSi v0.1.0 Alpha

This candidate adds a standalone `DSi.exe` first run importer to the existing Rust/Tauri DSi shell and C++ melonDS DS libretro runtime. It validates file sizes and a plain MBR or no$gba DSi footer, verifies SHA-256 copies, places private files in local app data, and starts the original DSi firmware from working copies. The development executable remains separate.
Press F8 to change virtual SD settings, select a local custom shell image, or reload the user-editable keyboard map. Press F9 to align clickable buttons with the shell artwork. See CUSTOM_SHELL.md, BUTTON_LAYOUT.md, and KEYBOARD.md for instructions. This public candidate uses the matched compatibility backend; the alternate development backend is not included.

CPF-owned code is GPL-3.0-only. This release candidate still needs clean environment validation and final dependency notice review. It is not approved for publication.
