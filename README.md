# ConsolePreservationFrontend (CPF)

CPF is a proposed frontend for preserving and reconstructing console environments. The main branch carries the separate experimental Pseudo Game Mounter and its source. The DSi application is on the DSI branch.

## Purpose

Give users a way to inspect, organize, and eventually reconstruct the recorded state of a console environment, including context beyond a single game executable.

## Core concept

The frontend would present records for console storage, firmware information, hardware profiles, configuration, metadata, saves, snapshots, and other reconstructable state. It should distinguish captured data, descriptive metadata, and references to material the user must supply separately.

## Example use cases

- Review an archived console configuration and save history.
- Compare a preserved hardware or firmware profile with a target environment.
- Prepare a preservation package for import by a compatible tool.

These are proposed workflows, not implemented features.

## High-level architecture

CPF would be a user interface above a separate preservation data layer. The data layer would define manifests, provenance, integrity checks, and import/export behavior; CPF would display and validate those records and guide compatible restore workflows. Optional adapters could connect preservation records to existing emulators. A portable container format may be specified later; no container schema or extension is established by this repository.

## Current status

**Alpha Pseudo Game Mounter build.** Download the Windows ZIP below. The Games tab is hidden in this version. The DSi app is published separately on the DSI branch.

## Planned features

- Inspectable manifests for hardware, firmware information, configuration, and storage.
- Save and snapshot browsing with provenance and integrity information.
- Import/export and package validation interfaces.
- Compatibility checks for optional emulator adapters.
- Clear reporting of missing user-supplied materials during reconstruction.

## Relationship to the other frameworks

CPF concerns console preservation workflows. [GameBridge](https://github.com/AC9892/GameBridge-Framework) concerns game-to-game translation, [VME/VMBridge](https://github.com/AC9892/VM-EmbedingFramework) concerns embedded virtual machines, and EmuFrame concerns emulator integration. Any connection would be optional and use explicit adapters or documented interfaces.

CPF must not include copyrighted firmware, games, keys, or other proprietary content. Users must supply any required material through lawful channels supported by the relevant tools.



## Downloads

- `Pseudo-Game-Mounter-v0.1.0-alpha-win64.zip` — Windows application.
- `Pseudo-Game-Mounter-v0.1.0-alpha-source.zip` — corresponding source.

SHA-256 checksums are in `SHA256SUMS.txt`.
