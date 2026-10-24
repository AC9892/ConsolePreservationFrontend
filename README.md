# CPF Xbox 360 - experimental Xenia Canary integration

**Known limitations first: Game Details does not work, original avatar rendering/customization remains unresolved, and seamless dashboard/game switching is not implemented. Preferences also remains unsupported. This is experimental source, not a complete Xbox 360 operating-system emulator or a release.**

This Xbox360 branch contains a source patch for Xenia Canary, build instructions, and a record of locally observed behavior. It does not contain the CPF inspector, a standalone frontend, firmware, dashboard assets, games, profiles, keys, logs, or compiled executables. You must supply your own lawful local research inputs separately.

## What has been observed locally

- The authentic Xbox 360 Metro dashboard 2.0.17559.0 boots and renders. This uses original dashboard software supplied locally, not a recreated interface.
- Keyboard navigation reaches Home, Social, Games, Apps and Settings. Some original system settings pages open. This does not establish every setting works.
- With `dashboard_local_uri = true`, My Games opens its original empty-library page, My Apps renders six built-in tiles, and Recents opens its original page. Recents showed a Groove Music tile, but did not show the tested game; history enumeration is not established as correct. Individual built-in applications are not verified.
- One existing local test profile was reused across launches. Profile sign-in/selection is partial; full console-profile compatibility is not claimed.
- File > Mount game disc... mounts a supported Xbox 360 ISO read-only while the dashboard runs. File > Eject game disc restores the empty-tray state. The Home tile changed between Close Tray and Play Game during controlled tests.
- A Play Game request can prepare a persistent handoff. Closing and reopening Xenia then booted Halo: Reach to its original welcome/setup screen; manual selection movement was confirmed. Campaign gameplay, save persistence, and a seamless return to the dashboard were not tested.
- A limited `XeKeysGetMediaID` implementation supplies the mounted disc's 16-byte XGD2 ID to dashboard source-1 queries. The test eliminated repeated undefined-export errors. It does not make Game Details work.

These are development observations from a limited test setup, not a compatibility guarantee.

## What remains broken or incomplete

- **Game Details:** an experimental local route did not render reliably and was removed. A later keyboard action in that experiment was followed by a launch-handoff notice; its precise cause remains unresolved. The published patch excludes that route.
- **Avatars:** original avatar rendering and customization remain unresolved. Existing incomplete avatar APIs have diagnostic probes; they are not proven to be the sole cause.
- **Title switching:** dashboard-to-disc execution requires a manual backend restart. Safe in-process title replacement is not enabled.
- **Preferences:** the original system-UI request has no complete implementation. No replacement screen or fake success was added.
- **Recents/content:** the tested ISO does not appear in Recents or My Games automatically. A mounted ISO is not an installed content package. No synthetic game-library entries are created.
- **DASHUSER:** the package mounts, but two theme/effect child resources remain absent after retries. Their visible impact has not been established.
- **Disc metadata:** missing `XexTransformImageKey` and `XeCryptAesCreateKeySchedule` functions were identified in the guest disc-resource extraction path. Their semantics and impact require further investigation.
- **Media queries:** `XeKeysGetMediaID` supports only the observed dashboard/source-1 case. Other sources and non-dashboard title contexts return an error.
- **Ejection:** logical eject removes drive aliases, but old disc devices remain alive until shutdown to protect open guest handles. It does not immediately release every ISO mapping/file handle. The dashboard's own controller Y-eject action was not reliably verified.
- Online Xbox services and general Xbox 360 system-application compatibility are not provided by this patch.

## Source and attribution

The patch applies to [Xenia Canary](https://github.com/xenia-canary/xenia-canary), upstream revision `2372a54105d9d598ab5bc7ba94a72ef1bea4b271` from `canary_experimental`.

Xenia is credited to its upstream authors and contributors. The patch retains upstream notices and is distributed under the same BSD 3-clause terms; see [LICENSE.xenia](integrations/xenia-canary/LICENSE.xenia). This work is an independent CPF experiment, not an upstream Xenia release or endorsement.

The publication contains [cpf-dashboard.patch](integrations/xenia-canary/cpf-dashboard.patch), affecting 18 C++ source/header files. It includes dashboard routing, virtual-disc handling, restart handoff, keyboard event handling, content-root initialization, and targeted diagnostics. No proprietary binary, disassembly, key value, or private test fixture is included.

## Apply and build

Use a separate source checkout. Do not apply the patch to console dumps or replace preserved originals.

```powershell
git clone https://github.com/xenia-canary/xenia-canary.git xenia-cpf
cd xenia-cpf
git checkout 2372a54105d9d598ab5bc7ba94a72ef1bea4b271
git submodule update --init --recursive
git apply --check <path-to-cpf-dashboard.patch>
git apply <path-to-cpf-dashboard.patch>
```

Follow the upstream build prerequisites. Local development used Windows, Visual Studio 2022 and the Vulkan SDK. In a PowerShell session with the required tools available:

```powershell
$env:_CL_ = '/Z7'
python xenia-build.py build --config=release
```

`/Z7` was used to avoid an MSVC C1902 debug-information issue encountered in this setup. It is not a claim that every compiler/SDK combination requires that override. The release-build output is `build/bin/Windows/Release/xenia_canary.exe`; no executable is published here.

## Local testing

1. Keep original dumps and game images unchanged. Put any user-supplied working copies, configuration, generated profiles and logs outside the publication checkout.
2. Reuse the existing Xenia storage/configuration and profile when testing persistence. Do not run multiple instances against the same storage.
3. Launch your locally supplied `dash.xex` with its accompanying extracted flash filesystem. Merely opening the XEX does not prove the dashboard works; verify rendering and input.
4. The new library routes are disabled by default. After Xenia has generated its configuration, set `dashboard_local_uri = true` in its `[Kernel]` section while Xenia is closed. Test My Games, My Apps, Recents and Back.
5. Use File > Mount game disc... for a supported ISO. Confirm Play Game appears. Use File > Eject game disc for the tested eject path.
6. To test the restart handoff, select Play Game, then close and reopen the same Xenia executable with the same storage root. This launches the prepared disc instead of seamlessly replacing the dashboard. Do not repeatedly launch extra instances.

The handoff file is `launch_data.bin` under Xenia's storage root and contains local launch information. Keep it private, along with profiles and runtime logs. The patch adds limited redacted diagnostics, but **does not sanitize all upstream logging or crash reporting**. Do not upload raw logs or memory dumps.

## Validation and publication scope

The local source built successfully with Visual Studio 2022, the Vulkan SDK and `/Z7`; `git diff --check` passed. Rendering/input observations above came from controlled local tests. The final metadata-only build was compiled after removing the unsuccessful Game Details route; it was not relaunched after that removal. No stronger final-build runtime claim is made.

The publication patch passed the apply check against the pinned upstream index; all 18 reconstructed sources match the locally built working sources after normalizing line endings. No CI workflow, binary artifact, tag or GitHub release is part of this source-only upload.
