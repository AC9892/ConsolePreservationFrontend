# Build the public DSi release

Build on Windows x64 with Rust, Node.js, MSYS2 UCRT64 CMake/Ninja/GCC, and the Tauri Windows prerequisites. Fetch dependencies through Cargo, npm, and CMake, or point CMake FetchContent at local matching checkouts.

1. Configure melonDS DS with `cmake -S melonds-ds-libretro -B build-core -G Ninja -DCMAKE_BUILD_TYPE=Release -DFETCHCONTENT_SOURCE_DIR_MELONDS=<absolute path to melonDS-core> -DENABLE_JIT=OFF -DENABLE_OPENGL=OFF -DENABLE_NETWORKING=ON -DBUILD_TESTING=OFF`, then run `cmake --build build-core -j 4`.
2. In cpf-desktop, run `npm ci`; set `CPF_PUBLIC_BUILD=1`; run `npm run build`. Confirm dist contains public-shell.svg and not dsi-off.png.
3. Set `TAURI_CONFIG` to `{"productName":"DSi","identifier":"org.consolepreservation.dsi","version":"0.1.0-alpha","bundle":{"icon":["icons/public-dsi.ico"]}}`. Run `cargo build -p cpf-desktop --bin cpf-dsi-public --release` and `cargo build -p cpf-runtime-worker --bin cpf-runtime-worker --release`.
4. Package the resulting DSi executable, worker, and libretro DLL with the documentation, GPL text, and dependency notices. Verify no BIOS, firmware, NAND, games, saves, keys, or personal paths are present.
