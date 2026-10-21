# ConsolePreservationFrontend (CPF)

[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-666666)](LICENSE) [![Status: Experimental](https://img.shields.io/badge/status-experimental-666666)](https://github.com/AC9892/ConsolePreservationFrontend#current-status) [![DSi: Alpha](https://img.shields.io/badge/DSi-alpha-426487)](https://github.com/AC9892/ConsolePreservationFrontend/tree/DSI) [![Pseudo Game Mounter: Alpha](https://img.shields.io/badge/Pseudo_Game_Mounter-alpha-426487)](https://github.com/AC9892/ConsolePreservationFrontend/tree/main/pseudo-game-mounter) [![Discord: Join](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/vjHt6w49eY)

CPF is an experimental framework for preserving and reconstructing console environments. This repository contains the framework overview and early console-specific tools. The full preservation workflow and portable package format are still in development.

## Purpose

Give users a way to inspect, organize, and eventually reconstruct the recorded state of a console environment, including context beyond a single game executable.

## Core concept

The frontend would present records for console storage, firmware information, hardware profiles, configuration, metadata, saves, snapshots, and other reconstructable state. It should distinguish captured data, descriptive metadata, and references to material the user must supply separately.

## Example use cases

- Review an archived console configuration and save history.
- Compare a preserved hardware or firmware profile with a target environment.
- Prepare a preservation package for import by a compatible tool.

These describe the broader CPF goal; support varies by target and remains incomplete.

## High-level architecture

CPF would be a user interface above a separate preservation data layer. The data layer would define manifests, provenance, integrity checks, and import/export behavior; CPF would display and validate those records and guide compatible restore workflows. Optional adapters could connect preservation records to existing emulators. A portable container format may be specified later; no container schema or extension is established by this repository.

## Current status

**Experimental.** The DSi target and Pseudo Game Mounter are alpha projects. Their current capabilities and limits are documented separately; the framework-wide workflows above are not yet complete.

- [Nintendo DSi target](https://github.com/AC9892/ConsolePreservationFrontend/tree/DSI) — DSi source, setup, and known limits.
- [Pseudo Game Mounter](pseudo-game-mounter/README.md) — separate application for mounting a user-supplied `.nds` cartridge in a running DSi session.

## Planned features

- Inspectable manifests for hardware, firmware information, configuration, and storage.
- Save and snapshot browsing with provenance and integrity information.
- Import/export and package validation interfaces.
- Compatibility checks for optional emulator adapters.
- Clear reporting of missing user-supplied materials during reconstruction.

## Relationship to the other frameworks

CPF concerns console preservation workflows. [GameBridge](https://github.com/AC9892/GameBridge-Framework) concerns game-to-game translation, [VME/VMBridge](https://github.com/AC9892/VM-EmbedingFramework) concerns embedded virtual machines, and EmuFrame concerns emulator integration. Any connection would be optional and use explicit adapters or documented interfaces.

CPF must not include copyrighted firmware, games, keys, or other proprietary content. Users must supply any required material through lawful channels supported by the relevant tools.

