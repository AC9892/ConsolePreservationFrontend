# Custom console shell image

You can use your own PNG around the live DSi screens. The image stays on your PC in `%LOCALAPPDATA%\CPF\Nintendo DSi\ui\custom-shell.png`; it is not uploaded or added to the CPF app files.

## Prepare the image

- Use a **PNG** file no larger than **4 MiB**.
- Both width and height must be **512–4096 pixels**. The width divided by height must be **0.9–1.1**.
- **1254×1254 pixels** is recommended because the original development shell and button layout use that coordinate system. Other accepted sizes are scaled to the same app window.
- Keep the two screen openings aligned with the rectangles below. CPF does not detect screen positions in the image.

| Opening | Left | Top | Width | Height |
| --- | ---: | ---: | ---: | ---: |
| Top screen | 26.8% | 7.35% | 46.4% | 32.9% |
| Touch screen | 26.8% | 55.75% | 46.4% | 33.9% |

On a 1254×1254 image, the top screen begins at approximately **(336, 92)** and covers **582×413** pixels. The touch screen begins at approximately **(336, 699)** and covers **582×425** pixels. Keep any important artwork outside those areas: the live screens cover them. Use the calibration steps in BUTTON_LAYOUT.md to move the clickable button areas onto your artwork.

## Load it

1. Start `DSi.exe` and finish the first-run console import.
2. Press **F8** to open DSi settings.
3. Under **Console shell**, select **Choose image…** and pick your PNG.
4. The shell changes immediately. If it does not display, CPF returns to the default shell and shows an error in settings.
5. Select **Use default** to remove the locally imported copy and return to the supplied shell.

The image selection does not restart the emulated DSi. It does not alter NAND, saves, firmware, or the original image file you selected. If you move or delete the original file later, the imported copy continues to work. To share a shell with others, share the image separately and have them select it themselves.
