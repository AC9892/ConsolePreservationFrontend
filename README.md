# CPF Xbox 360 - experimental Xenia Canary integration

https://github.com/user-attachments/assets/5b367fa9-cada-45bc-a720-1e2fb7f3cf26

**Known limitations first: Game Details remains unverified, original avatars/customization and Preferences remain unresolved, and seamless dashboard/game switching is not implemented. This is development source, not a public executable release or a complete console operating-system emulator.**

This branch contains the existing optional CPF Windows monitor, browsable Xenia modifications, synthetic tests and a reproducible upstream patch. It contains no dashboard binaries/assets, firmware, games, profiles, keys, private logs or compiled executables. Supply your own lawful local research inputs separately.

## Gamer-picture checkpoint (2026-10-30)

The additive [gamer-tile patch](patches/cpf-gamer-tile.patch) corrects RGBA-to-ARGB row placement for padded texture pitch, rejects undersized destinations and handles image decoder failure. Apply it after `cpf-dashboard.patch` to the pinned upstream checkout. This is a source checkpoint, not a release.

Validation: incremental development build passed with `/Z7`; syntax check passed; two synthetic tests passed with eight assertions. The modified executable was subsequently tested with dashboard12581 and synthetic32/64 PNGs in a disposable profile copy: the original Welcome and My Xbox screens displayed the expected picture and accepted navigation. This validates stored-picture rendering, not the original picture-selection/save flow, persistence, or execution of the changed texture-copy function. The stride fix is covered separately by unit tests. The working profile remains unchanged.

Historical dashboard tests confirmed original builds 1888, 7371 and 12581 render and accept keyboard gamepad navigation. These bounded tests do not establish full system functionality. The isolated test profile had no stored picture or avatar manifest. A controlled 12581 avatar-initialization test reached the existing zero-size/incomplete asset handlers and still displayed a gray figure. Original personalized 3D avatars remain unresolved; supplying original assets requires a real asset/animation pipeline. No dashboard resources or profile data accompany these sources.

Read-only avatar-pack research now identifies unique bounded candidate index layouts for the tested NXE and Kinect inputs, including the legacy pack. All populated candidate ranges were checked for bounds and overlap. The legacy file demonstrates that the header version alone cannot choose the index layout. Guest asset identifiers and output/GPU buffer contracts remain unresolved; no avatar loader has been integrated and no personalized 3D rendering is claimed.

## Metro default-image API experiment

Apply [cpf-default-system-image.patch](patches/cpf-default-system-image.patch) after the canonical dashboard patch. It adds an opt-in `dashboard_default_system_image` path in `[Kernel]` for your separately supplied original 64x64 PNG. No image is distributed. The loader caps input at 1 MiB, checks PNG decoding/dimensions and keeps a mutex-protected guest cache owned by XAM until module teardown. Restart after changing the path.

The candidate built with `/Z7` and booted original Metro17559 in isolated storage. Home/Social rendering and shoulder navigation were observed; the default-image export is registered and its undefined-call warning was absent. A visible picture improvement and successful cache return were not established. Gray avatars remain. Missing/invalid input, concurrent calls and cache teardown need focused verification. This API experiment is not a completed picture or 3D-avatar fix.

## Project layout

| Path | Contents |
| --- | --- |
| [monitor](monitor) | Existing Rust monitor/preservation module, Cargo files and synthetic tests. |
| [xenia](xenia) | 23 complete changed/new source views and two explicitly marked helper-only views. |
| [patches/cpf-dashboard.patch](patches/cpf-dashboard.patch) | Canonical changes to 25 files in the pinned upstream tree. |
| [tools/prepare-xenia.ps1](tools/prepare-xenia.ps1) | Prepare/check a separate pinned upstream checkout. |
| [xenia/source-manifest.json](xenia/source-manifest.json) | Source-view hashes, upstream paths and patch identity. |

The optional Xbox.exe monitor observes Xenia externally; the authentic dashboard runs in Xenia's own window. The separate CPF Console Settings launcher is not included here. This update adds no standalone application or central launcher.

## Observed locally

- Original Metro dashboard 2.0.17559.0 boots and renders. Keyboard navigation reaches the main tabs and some original settings pages; this is original dashboard execution, not a recreation.
- With `dashboard_local_uri = true`, My Games opens an empty library, My Apps shows six built-in entries, and Recents opens. Correct history enumeration and individual app functionality remain unverified.
- Existing test-profile selection/sign-in is partial. Tests reused local profile copies; full console-profile compatibility is not established.
- File > Mount game disc mounts the selected supported ISO read-only. Home changes to Play Game. File > Eject game disc is the tested eject path.
- Restart-based disc handoff reached Halo: Reach's original welcome/setup screen; manual selection movement was confirmed. Gameplay and same-process title switching are not established.
- The opt-in disc-crypto experiment completed an image-key transform during a mounted-disc dashboard test. Synthetic AES tests cover known ciphertext, in-place decryption, IV continuity and invalid requests. These observations do not establish the entire resource extraction pipeline.
- A candidate with controller hotkeys enabled rendered the dashboard, accepted the tested navigation sequence, and closed through the normal exit path without a worker-wait failure. This is a smoke test, not concurrency stress coverage or complete backend teardown.

## Changes in this update

- Added default-off `dashboard_disc_crypto`: narrowly scoped retail image-key transformation and AES schedule/CBC handlers for the observed dashboard resource path. Unsupported key variants and inputs remain rejected. It reuses existing upstream key handling without distributing key constants. It does not implement console authorization or a complete avatar/resource service.
- Added cooperative controller-hotkey worker stop/join, explicit failure handling, generation-cancelled queued UI actions and normal-shutdown integration. Title selection and its recent-title/dialog access now run on the UI thread. Other backend workers and callback lifetimes still require work before safe title replacement.
- Added a dashboard-only avatar initialization diagnostic. The tested configuration returned the initialization-disabled path. Defaults and profile data are unchanged; asset and animation handlers remain incomplete.
- Corrected the experimental Game Details destination from the generic content library to the original embedded `gamedetails` scene. The old mapping reproducibly opened empty My Games. The corrected build booted the dashboard, but native file-picker automation blocked the subsequent disc selection, so the corrected details action was not exercised. Do not describe Game Details as working.

## Remaining limitations

- **Game Details:** corrected scene mapping needs a controlled mounted-disc rendering/input test. Route dispatch alone is insufficient.
- **Avatars:** initialization is disabled by default; enabling it does not supply the missing original asset/animation pipeline. Rendering and customization remain unresolved.
- **Preferences:** the original system-UI request lacks a complete implementation.
- **Title switching:** the supported experiment uses a backend restart. Main-thread completion, GPU/audio quiescence, guest object retirement and remaining UI callbacks need explicit coordination for same-process switching.
- **Content:** a mounted ISO is not an installed package and does not automatically appear in My Games or Recents. No synthetic entries are added.
- **DASHUSER:** the package mounts with a profile, but two theme/effect child requests remain unresolved after retry; visible impact is not established.
- **Media queries/eject:** media-ID handling is limited to the observed dashboard/source-1 case. Old disc devices remain alive until shutdown to protect guest handles. The dashboard's controller Y-eject path is not verified.
- Online Xbox services and general system-application compatibility are not provided.

## Apply and build

Upstream: [Xenia Canary](https://github.com/xenia-canary/xenia-canary), revision `2372a54105d9d598ab5bc7ba94a72ef1bea4b271` (`canary_experimental`). See [LICENSE.xenia](xenia/LICENSE.xenia) and [THIRD_PARTY.md](THIRD_PARTY.md). This is an independent CPF experiment, not an upstream release or endorsement.

Use a separate source checkout, never console dumps:

```powershell
git clone https://github.com/xenia-canary/xenia-canary.git xenia-cpf
cd xenia-cpf
git checkout 2372a54105d9d598ab5bc7ba94a72ef1bea4b271
git submodule update --init --recursive
git apply --check <path-to-cpf-dashboard.patch>
git apply <path-to-cpf-dashboard.patch>
$env:_CL_ = '/Z7'
python xenia-build.py build --config=release --build-tests --target xenia-kernel-tests --target xenia-app
.\build\bin\Windows\Release\xenia-kernel-tests.exe
```

Follow upstream prerequisites. Local builds used Visual Studio 2022 and the Vulkan SDK; `/Z7` avoided an MSVC C1902 issue. Output: `build/bin/Windows/Release/xenia_canary.exe`. Here **Release means compiler configuration**, not a published release. Apply the canonical patch; do not additionally compile the browsable helper-only views.

## Local testing and privacy

Keep all supplied console/game data, profiles, configs and logs outside this publication checkout. Use isolated working copies and one emulator instance per storage root. Reuse an existing profile rather than creating duplicates.

After Xenia generates its config, close it before enabling experiments in `[Kernel]`:

```toml
dashboard_local_uri = true
dashboard_disc_crypto = true
```

Both options default to false in source. Do not change `allow_avatar_initialization` merely to hide an initialization failure. Launch your supplied dashboard with its accompanying filesystem, mount the selected supported ISO, wait for Play Game, and use controller X for Game Details. Verify original page rendering and input/Back; this is still a pending compatibility test. For restart handoff, Play Game prepares the next launch: close and reopen the same emulator with the same storage root.

`launch_data.bin` contains local launch information. Keep it private along with profiles, runtime logs and crash reports. Targeted diagnostics do not sanitize all upstream logging; never upload raw logs or memory dumps.

## Validation boundaries

The October 29 local Xenia builds succeeded with `/Z7`; kernel tests passed 8 cases / 288 assertions. The lifecycle/avatar checkpoint received a dashboard/mount/navigation/normal-close smoke test. The subsequent corrected Game Details build received dashboard boot observation only; its mounted-disc details action remains unverified. No new gameplay, avatar-rendering or seamless-switching claim follows.

The canonical patch is checked against the pinned upstream index and reconstructed source is compared with the development tree after line-ending/BOM normalization. Manifest hashes describe publication bytes. The unchanged Rust monitor previously passed 13 library and 4 monitor tests plus its Windows release build; those earlier results are retained, not claimed as a new run for this update.

No GitHub Actions workflow, binary, tag or public release is added. Private agent notes, disassembly, firmware inputs, keys, profiles and runtime evidence are excluded.
