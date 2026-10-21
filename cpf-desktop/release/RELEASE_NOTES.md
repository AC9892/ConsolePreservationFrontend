# CPF Nintendo DSi v0.1.0-alpha.2

This alpha update limits automatic snapshot history so repeated DSi sessions no longer retain an unlimited number of full NAND, firmware, and SD copies.

## Changes

- Keep the three newest automatic snapshots per category: NAND, firmware, and SD.
- Apply retention before boot and after a successful shutdown, including cleanup after interrupted sessions.
- Preserve manually named snapshots, original console dumps, capture metadata, save backups, and current working storage.
- Log each retired snapshot and its rollback availability. Historical modification hashes remain intact.

## Upgrading

Use the complete alpha.2 package, including its updated `DSi Backend.exe`. Older automatic history is trimmed when that worker next starts. See [SNAPSHOTS.md](SNAPSHOTS.md) for the exact policy.

Snapshots remain complete copies. Compression and changed-block snapshots are not implemented; SD history size depends on the SD image size.

## Verification and limits

Retention tests and release builds passed for the development worker and a clean public worker. Tests cover keeping the newest three snapshots, preserving manual snapshots and current storage, and recording pruning events. A complete live DSi compatibility regression has not been performed for this update. Existing application, audio, network, and clean-Windows limitations remain; see [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

CPF-owned code is GPL-3.0-only. Third-party components retain their respective licenses. No Nintendo system data is included.
