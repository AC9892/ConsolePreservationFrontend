# CPF DSi

Experimental Windows build of CPF's Nintendo DSi target. It uses the C++ melonDS emulation core through a libretro backend; the CPF desktop application and runtime worker are written in Rust, with a Tauri interface. The original DSi Menu comes from the user's own system files. The unfinished work to show library games as DSi Menu tiles is **not** in this public build.

## Preview

[Watch the DSi preview video](https://github.com/AC9892/ConsolePreservationFrontend/blob/main/image/preivew.mp4)

## Download and run

For a first-time installation, use the complete Windows ZIP from [Releases](https://github.com/AC9892/ConsolePreservationFrontend/releases) when available. Extract the whole ZIP to a permanent folder, then run `DSi.exe`. Do not run it from inside the ZIP or download only the EXE: it also needs `DSi Backend.exe` beside it and `backend/melondsds_system_compat.dll` at that relative path. Windows x64 and WebView2 are required. This alpha is unsigned, so Windows may show **Unknown Publisher**.

The setup window asks for files dumped from your own console:

| File | Expected size |
| --- | ---: |
| `bios9.bin` | 4 KiB |
| `bios7.bin` | 16 KiB |
| `dsi_bios9.bin` | 64 KiB |
| `dsi_bios7.bin` | 64 KiB |
| DSi firmware | 128, 256, or 512 KiB |
| DSi NAND | 240 or 256 MiB, with a supported layout/footer |

Choose the folder containing those files, or select them individually, then choose **Import and start DSi**. Filenames and sizes alone do not prove that dumps are valid or belong to the same console. NAND setup can take several minutes.

CPF reads and hashes the selected originals in place. It creates a separate writable NAND for the machine and another runtime working copy; it does not write to the selected originals. Keep the original files at their selected paths. Private machine data and logs are stored under `%LOCALAPPDATA%\CPF\Nintendo DSi`. Logs may contain identifying information.

## Controls and customization

- **F8:** DSi settings. Saving settings restarts the emulated DSi.
- **F9:** adjust clickable button positions to match a custom shell image.
- **Esc:** close the DSi window.
- Keyboard controls are enabled by default. See [KEYBOARD.md](cpf-desktop/release/KEYBOARD.md) for remapping and [BUTTON_LAYOUT.md](cpf-desktop/release/BUTTON_LAYOUT.md) for button placement.

The Windows package also includes setup and customization guides. Source is in [cpf-desktop](cpf-desktop), [melonds-ds-libretro](melonds-ds-libretro), and [melonDS-core](melonDS-core). `melonds-ds-libretro` is the adapter that connects the melonDS emulation core to CPF; it is not another app to launch. See [BUILD_CLEAN_RELEASE.md](BUILD_CLEAN_RELEASE.md) to build this source snapshot.

## Playing games right now

To play a user-supplied `.nds` game with the current DSi build, use the separate [Pseudo Game Mounter](https://github.com/AC9892/ConsolePreservationFrontend/blob/main/pseudo-game-mounter/README.md). The DSi app does not currently add games as tiles to the original DSi Menu.

## Current limits

This is an alpha, not a claim of full DSi compatibility. Built-in applications may freeze, white-screen, or return to the boot screen. DSi Sound has shown performance and audio problems. Network-dependent system features have not been fully tested. Virtual cartridge support is experimental; Pseudo Game Mounter is a separate application.

This repository does not include Nintendo BIOS, firmware, NAND, system titles, games, saves, or personal console data. Licenses and third-party notices are in the source directories and complete Windows package.
