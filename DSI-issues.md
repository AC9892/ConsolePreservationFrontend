# Nintendo DSi issues

Source: [DSI branch](https://github.com/AC9892/ConsolePreservationFrontend/tree/DSI). These items describe the checked-in branch and historical user tests; a local development fix is not assumed to be published.

## Runtime and compatibility

| ID | Status | Problem and next check |
| --- | --- | --- |
| DSI-1 | **Confirmed in source** | Right-click Power / reset can discard a cartridge inserted live. The [worker calls reset](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/crates/cpf-runtime-worker/src/main.rs#L196) without saving and reinserting it; the [backend rebuilds the console](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/melonds-ds-libretro/src/libretro/core/core.cpp#L356), while the worker keeps its Slot-1 record. Verify the cartridge and its save after reset. A local development worker was changed, but the fix is absent from this branch. |
| DSI-2 | **User reported; retest needed** | PictoChat and DS Download Play did not reliably return to the original DSi Menu. A workaround returned to the initial startup screen, which was accepted temporarily. Test each app's own exit path and verify the authentic Menu appears without white screens or a forced restart. |
| DSI-3 | **User reported; retest needed** | The **System** tab in Nintendo DSi Camera caused a black screen during system-storage operations. Reproduce only on a disposable working NAND; test save and return-to-Menu paths. |
| DSI-4 | **User reported; retest needed** | Nintendo DSi Sound showed severe lag and distorted or excessively loud audio. An earlier build also white-screened at launch. Retest the bird/sound app, recording, playback, and volume on the current build. |
| DSI-5 | **User reported; retest needed** | Moving from the DSi startup screen to the Menu sometimes lagged, and the development build occasionally had low frame rate. Capture frame timing to isolate this from the Sound problem. |
| DSI-6 | **Open verification** | Network-dependent DSi features have not been fully tested. Keep offline results separate from Wi-Fi results. This is also noted in the [published known issues](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/release/KNOWN_ISSUES.md). |
| DSI-7 | **Open verification** | The exact public package still needs a complete clean-Windows and built-in-app regression run: first-run import, Menu, Settings, Camera, Sound, PictoChat, Download Play, buttons/touch, and clean exit. [Release notes](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/release/RELEASE_NOTES.md) note incomplete validation. |

## Build and release

| ID | Status | Problem and next check |
| --- | --- | --- |
| DSI-8 | **Confirmed in source** | [build-local.ps1](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/build-local.ps1#L7) refers to `src-tauri/icons/create-dsi-icon.ps1` and `icons/dsi.ico`, but neither exists on the DSI branch. The local development build script will fail. This does not establish that the separate public build fails. |
| DSI-9 | **Confirmed in repository** | [MANIFEST.sha256](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/MANIFEST.sha256) omits root `LICENSE`, root `README.md`, and `backend/melondsds_system_compat.dll`; it cannot verify the entire branch. Regenerate it for the intended scope or label the exclusions. |
| DSI-10 | **Open verification** | The [third-party notices](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/release/THIRD_PARTY_LICENSES.md) and packaged backend's matching source need a final release audit. The [release notes](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/release/RELEASE_NOTES.md) still say the candidate was not approved for publication. |

## Snapshot storage

| ID | Status | Problem and fix |
| --- | --- | --- |
| DSI-11 | **Fixed in source; release update pending** | Automatic full NAND/firmware/SD snapshots previously had no retention limit and accumulated many gigabytes. The DSI branch now retains the newest three automatic snapshots per category, logs pruning, preserves manually named snapshots, and rejects linked snapshot directories. Development and clean public worker tests and release builds passed. A rebuilt public backend is staged locally; already downloaded release ZIPs remain affected. See [snapshot policy](https://github.com/AC9892/ConsolePreservationFrontend/blob/DSI/cpf-desktop/release/SNAPSHOTS.md). |

## Current scope

The public build is an alpha, not a claim of full DSi compatibility. Games do not currently become tiles in the original DSi Menu; the separate Pseudo Game Mounter handles cartridges. Do not commit or bundle Nintendo BIOS, firmware, NAND, titles, games, saves, keys, or personal console data.
