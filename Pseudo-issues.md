# Pseudo Game Mounter issues

Source: [Pseudo Game Mounter on main](https://github.com/AC9892/ConsolePreservationFrontend/tree/main/pseudo-game-mounter). User reports below need a fresh test of the exact packaged version.

## Session and cartridge behavior

| ID | Status | Problem and next check |
| --- | --- | --- |
| PSEUDO-1 | **Confirmed cross-app issue** | After a live insert, right-click Power / reset in DSi can leave Pseudo showing a mounted game while emulated Slot-1 is empty. Root cause: [DSI-1](DSI-issues.md#runtime-and-compatibility). Fix DSi reset handling, then press **Check slot** immediately after reset to verify both apps agree. |
| PSEUDO-2 | **User reported; retest needed** | Pseudo once showed **No running console apps found** while public `DSi.exe` was open. The current source [searches DSi session descriptors](https://github.com/AC9892/ConsolePreservationFrontend/blob/main/pseudo-game-mounter/src-tauri/src/dev_dsi.rs#L52), but discovery has not been verified against the exact public ZIP. Start Pseudo both before and after DSi. |
| PSEUDO-3 | **User reported; retest needed** | Live eject followed by insertion of a different `.nds` sometimes lagged, opened melonDS menus, or failed. Test repeated insert → eject → insert cycles on public DSi and standalone melonDS, recording both apps' Slot-1 state after every step. |
| PSEUDO-4 | **User reported once; not reproduced** | **Clear selection** appeared to crash Pseudo on first use, then worked on later attempts. Repeat from a fresh process with and without an active DSi session; retain the error/log if it recurs. |
| PSEUDO-5 | **Open verification** | Cartridge save preservation and Slot-1 reporting after reset, app exit, and abrupt termination need a complete regression run. Use a disposable save and compare hashes before and after each operation. A mounted label alone does not prove the DSi sees the cartridge. |

## Scope and release

| ID | Status | Problem and next check |
| --- | --- | --- |
| PSEUDO-6 | **Current limitation** | The released Cartridge UI does not install games as tiles in the original DSi Menu. Experimental Games library/Menu-tile work is excluded from this release, as stated in the [current README](https://github.com/AC9892/ConsolePreservationFrontend/blob/main/pseudo-game-mounter/README.md). Keep this separate from Slot-1 mounting. |
| PSEUDO-7 | **Open verification** | The exact packaged build has not been verified against every supported DSi build variant and a clean Windows installation. Test public DSi first, then development DSi and standalone melonDS where supported; record the versions used. |

Pseudo is a separate desktop application. Its released scope is choosing a user-supplied `.nds` file, mounting it in a selected running session, checking Slot-1, and ejecting it. No ROMs or Nintendo system data belong in the repository or release.
