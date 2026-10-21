# Align the shell buttons

The shell image is visual artwork. CPF uses separate, invisible button areas for mouse clicks. You can move those areas to match the artwork without changing the DSi screens, keyboard controls, or gamepad controls.

1. Open the DSi app and press **F9**. In the public app, you can also press **F8 → Adjust buttons…**.
2. Colored markers show the current clickable areas. Drag each marker onto the matching button in your shell image. The markers include the D-pad, A/B/X/Y, L/R, Start, Select, and Power.
3. Select **Save** to keep the positions. Select **Cancel** or press **Esc** to discard the changes. **Defaults** previews the original positions for the current shell; select Save to keep them.
4. Test each button after closing calibration mode. Press F9 again for fine adjustment.

Only the **positions** change; the clickable areas keep their current sizes. The app stores positions as percentages, so they stay aligned when the window is displayed at a different scale. Calibration does not restart the emulated DSi and does not alter NAND, firmware, saves, or the shell image.

The public app stores the layout in `%LOCALAPPDATA%\CPF\Nintendo DSi\ui\button-layout.json`. The development app stores a separate `ui\development-button-layout.json` in its CPF workspace. Changing the custom shell image does not erase your saved positions; use **Defaults** or adjust the markers for the new image.

CPF currently uses manual alignment. It does not try to identify controls from arbitrary artwork automatically.
