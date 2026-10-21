# Private system data

The portable ZIP must contain code, backend binaries, and application artwork only. It must never include BIOS, firmware, NAND, keys, tickets, console IDs, saves, screenshots, private machine profiles, or runtime logs.

Imported files are copied into `%LOCALAPPDATA%\CPF\Nintendo DSi`; the original selected files are read only. A separate generated NAND and runtime working NAND receive emulation writes. CPF does not upload imported system files.

Treat local app data backups and logs as private. Before any public release, audit both the exact ZIP and the Git history. A `.gitignore` rule does not remove secrets or copyrighted files from prior commits.
