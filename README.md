# ConsolePreservation Framework (CPV)

CPV is an experimental framework concept for preserving and reconstructing console environments, rather than only emulating game execution. This directory currently contains design documentation; no portable package format or restore tool is claimed to work.

## Purpose and core concept

Preservation records would describe console storage, firmware information, hardware profiles, configuration, metadata, saves, snapshots, and other reconstructable system state. Records should identify provenance and required external materials so a future restore tool can explain what it can and cannot reconstruct.

## Example use cases

- Archive a console's configuration and save history for later research.
- Recreate a documented hardware and software profile with compatible tools.
- Transfer a preservation package between tools that support the same format.

These are proposed use cases, not implemented workflows.

## High-level architecture

A capture/import layer would collect permitted state and metadata. A versioned manifest would describe the environment and file integrity. An export/restore layer would supply compatible tools with the data they can consume. A future `.cpv` container format may bundle portable preservation packages, with optional adapters for existing emulators.

## Status and planned features

**Status: experimental design.** The `.cpv` format, capture pipeline, and restore pipeline have not been specified or implemented here.

Planned work includes a manifest schema, integrity checks, storage and save capture, snapshot metadata, import/export adapters, and a documented container specification.

## Relationship to the other frameworks

CPV owns preservation records, not game translation, VM execution, or emulator runtime behavior. Optional integrations may exchange metadata or state with GameBridge, VMBridge (VME), EmuFrame, or existing emulators through adapters.

CPV must not contain copyrighted firmware, games, keys, or other proprietary content. Users must supply any required material through lawful channels supported by the relevant tools.

