# Installation

1. Extract the entire portable ZIP to a folder you can keep. Do not run the executable from inside the ZIP.
2. Run `DSi.exe` on Windows x64. Windows WebView2 is required by the Tauri window.
   This local alpha is unsigned, so Windows may show **Unknown Publisher**. Verify the ZIP against `SHA256SUMS.txt` and review its source before deciding whether to run it. No publisher certificate is bundled.
3. In the setup window, choose a folder containing your own dumps, or choose each file separately: `bios9.bin` (4 KiB), `bios7.bin` (16 KiB), `dsi_bios9.bin` (64 KiB), `dsi_bios7.bin` (64 KiB), DSi firmware (128/256/512 KiB), and NAND (240/256 MiB with either a plain MBR or a recognized no$gba DSi footer). Filenames alone are not validation.
4. Select **Import and start DSi**. CPF hashes your selected files in place and creates a separate writable NAND copy; NAND setup can take several minutes. Keep your original files at the selected paths after setup.

CPF stores its private machine and writable NAND in `%LOCALAPPDATA%\CPF\Nintendo DSi`. It references your selected BIOS, firmware, and original NAND files in their existing locations; it does not copy them into `system-files`. The runtime creates another writable NAND under the machine's runtime directory. CPF never writes the files you selected for import. Older setups that already copied files into `system-files` continue to work; this update does not automatically delete those copies.

Do not place Nintendo system dumps in the application folder. To troubleshoot, check the machine's `logs` folder under local app data. Keep those logs private because they may contain identifying information.

During normal operation, press **F8** for DSi settings. Saving settings restarts the emulated DSi. Press **Esc** to close the DSi window.

To use a locally supplied shell image, see CUSTOM_SHELL.md. Choosing or resetting the shell takes effect immediately and does not restart the DSi.

Keyboard controls are enabled by default. To change them, edit the generated `input.json` as described in KEYBOARD.md, then press **F8 → Reload bindings**.

If clickable shell buttons do not line up with the image, press **F9** and follow BUTTON_LAYOUT.md. The public F8 settings panel also has an **Adjust buttons…** control.

Use your own legally obtained console dumps. This package does not fetch firmware or system software.
