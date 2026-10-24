# Xenia Canary dashboard modifications

**Game Details, original avatars and seamless title switching remain incomplete.** See the root README for the tested behavior and limitations.

## Browse versus build

The `source` directory makes the changes readable on GitHub: 17 complete modified upstream source/header files and a standalone source view of the CPF media-ID handler. It is not a complete emulator checkout. Keep using `cpf-dashboard.patch` to reproduce the tested changes on the pinned upstream revision; do not compile this view in addition to the patched upstream files.

`xboxkrnl_crypt.cc` is deliberately not copied in full because upstream key material would conflict with CPF's publishing rules. Its newly added handler is available as `source/extensions/xboxkrnl_media_id.cc`, and the canonical patch contains the actual integration changes. No key bytes or proprietary guest code are included in that view.

[Source manifest](source-manifest.json) records the pinned revision, patch checksum and SHA-256 of each published source view. The complete upstream source and dependencies are obtained separately from [Xenia Canary](https://github.com/xenia-canary/xenia-canary). The original project and contributors retain their notices; redistributed Xenia files and CPF modifications use the upstream [BSD 3-clause license](LICENSE.xenia).

## Prepare a build checkout

From the CPF repository root:

```powershell
./tools/prepare-xenia.ps1
```

This clones upstream into the ignored `build/xenia-canary` directory, checks out revision `2372a54105d9d598ab5bc7ba94a72ef1bea4b271`, initializes its pinned submodules and applies the patch. It does not download dashboard binaries, games, console keys or user profiles. It obtains only upstream emulator/dependency source.

To check an existing checkout without changing it:

```powershell
./tools/prepare-xenia.ps1 -SourceDirectory <existing-xenia-source> -CheckOnly
```

The helper rejects other upstream revisions, checks whether all patch hunks are already present, and does not reset, clean, overwrite conflicting files, or switch the revision of an existing checkout. Follow the root README for the tested MSVC/Vulkan build command and `/Z7` workaround. Private runtime inputs and logs belong outside this publication tree.

## Modification index

| Upstream file | Published source view |
| --- | --- |
| [src/xenia/app/emulator_window.cc](source/src/xenia/app/emulator_window.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/app/emulator_window.h](source/src/xenia/app/emulator_window.h) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/app/xenia_main.cc](source/src/xenia/app/xenia_main.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/cpu/backend/x64/x64_emitter.cc](source/src/xenia/cpu/backend/x64/x64_emitter.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/emulator.cc](source/src/xenia/emulator.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/emulator.h](source/src/xenia/emulator.h) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/hid/keyboard/keyboard_input_driver.cc](source/src/xenia/hid/keyboard/keyboard_input_driver.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/hid/keyboard/keyboard_input_driver.h](source/src/xenia/hid/keyboard/keyboard_input_driver.h) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/smc.cc](source/src/xenia/kernel/smc.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/smc.h](source/src/xenia/kernel/smc.h) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/xam/xam_avatar.cc](source/src/xenia/kernel/xam/xam_avatar.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/xam/xam_content.cc](source/src/xenia/kernel/xam/xam_content.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/xam/xam_info.cc](source/src/xenia/kernel/xam/xam_info.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/xam/xam_module.cc](source/src/xenia/kernel/xam/xam_module.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/xam/xam_module.h](source/src/xenia/kernel/xam/xam_module.h) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/kernel/xboxkrnl/xboxkrnl_crypt.cc](source/extensions/xboxkrnl_media_id.cc) | New media-ID handler only; upstream cryptography constants omitted. |
| [src/xenia/kernel/xboxkrnl/xboxkrnl_io.cc](source/src/xenia/kernel/xboxkrnl/xboxkrnl_io.cc) | Complete modified source/header file; upstream notices retained. |
| [src/xenia/vfs/devices/disc_image_device.h](source/src/xenia/vfs/devices/disc_image_device.h) | Complete modified source/header file; upstream notices retained. |
