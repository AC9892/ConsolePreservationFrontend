# CPF Xbox 360 public preview

This backend runs the original Xbox 360 dashboard in Xenia's own window. It does not include Microsoft dashboard files. Metro 2.0.17559.0 is the primary test target.

## What to expect

Dashboard boot and some navigation have been observed locally. Personalized avatars, Game Details, Preferences, online services and seamless dashboard/game switching remain incomplete. On 2026-11-04, a clean public-source build booted Metro 17559 with a newly created disposable profile. Home/Social rendering, shoulder navigation, normal exit and one profile-persistence restart were observed. This is limited local validation, not complete console compatibility.

Requires Windows x64, the Microsoft Visual C++ x64 runtime and a compatible graphics driver. The Vulkan SDK is a build prerequisite, not a dashboard file.

## Using the preview folder

1. Extract the complete preview folder somewhere writable.
2. Keep your own extracted dashboard filesystem in a separate folder, preserving its accompanying files. A NAND image or a lone dash.xex is not a complete prepared filesystem.
3. Run xenia_canary.exe. Acknowledge the disclaimer normally if shown.
4. Use Xenia's existing Profile menu to create or select a local profile. No profile is supplied.
5. Choose File > Open and select your own dash.xex. Wait for initialization.
6. Click the dashboard window before using input. Exit normally before restarting or switching dashboard versions.

Keyboard defaults: Shift+W/A/S/D for D-pad, semicolon for A, apostrophe for B, and 1/3 for shoulder buttons. Physical-controller support depends on the selected input driver and your device.

If the dashboard fails, keep its log local. Do not post raw profiles, launch_data.bin, firmware, keys, logs or crash dumps. The optional Xbox.exe inspector/monitor is separate and is not required to execute the dashboard.

## Building from the public source branch

Install the pinned upstream project's Windows build prerequisites (Visual Studio 2022 C++, CMake/Ninja, Python 3.9 or newer, Git and Vulkan SDK). In PowerShell from this repository:

```powershell
.\tools\prepare-xenia.ps1 -SourceDirectory C:\CPF-Build\xenia
$env:_CL_ = '/Z7'
Set-Location C:\CPF-Build\xenia
python xenia-build.py build --config=release --build-tests --target xenia-kernel-tests --target xenia-app
.\build\bin\Windows\Release\xenia-kernel-tests.exe
```

From the CPF source folder, stage a clean preview using your completed public source build:

```powershell
python tools/package-public.py --source C:\CPF-Build\xenia --output C:\CPF-Preview
```

The package step uses an explicit allowlist, refuses existing output folders and verifies the pinned source/patch set. It does not publish anything or certify a binary's privacy. Review dependency licenses, binary contents and actual boot/input before distribution. Do not package an old private development executable or its runtime folder.

No public binary release has been created by these instructions. Supply required dashboard resources locally; none are downloaded or bundled automatically.
