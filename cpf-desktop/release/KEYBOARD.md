# Keyboard controls and remapping

The shell buttons are optional. The public DSi app also supports a keyboard, a gamepad, and mouse or touch input on the lower screen.

| DSi button | Default key |
| --- | --- |
| D-pad | Arrow keys |
| A / B / X / Y | X / Z / S / A |
| L / R | Q / W |
| Start / Select | Enter / Right Shift |

The first configured launch creates `%LOCALAPPDATA%\CPF\Nintendo DSi\input.json`. The default file looks like this:

```json
{
  "version": 1,
  "buttons": {
    "A": "KeyX",
    "B": "KeyZ",
    "DOWN": "ArrowDown",
    "L": "KeyQ",
    "LEFT": "ArrowLeft",
    "R": "KeyW",
    "RIGHT": "ArrowRight",
    "SELECT": "ShiftRight",
    "START": "Enter",
    "UP": "ArrowUp",
    "X": "KeyS",
    "Y": "KeyA"
  }
}
```

To change a key, close the JSON file in your editor after saving it, press **F8** in the DSi app, and select **Reload bindings** under **Keyboard**. The DSi does not restart. Changes also load on the next app start. The JSON file stays on your PC; it is not added to the app folder, source repo, or release package.

Use [KeyboardEvent.code](https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/code) names, such as `KeyC`, `Space`, `Digit1`, `Numpad0`, or `ShiftLeft`. These identify the physical key position; `"A"` or `"x"` are not valid key codes. Each of the 12 DSi buttons must appear exactly once, and each key code may be used only once. `Escape`, `F5`, `F6`, `F8`, and `F9` are reserved for app controls. Invalid JSON leaves the previous working bindings active and shows an error in F8 settings. To restore defaults, delete `input.json` while the app is closed; CPF recreates it on the next start.

App controls: **F5** resets the DSi, **F6** pauses or resumes, **F8** opens settings, **F9** adjusts shell button positions, and **Esc** closes settings or the window. Shell button and gamepad input continue to work independently of `input.json`.
