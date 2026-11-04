# Installation â€” guided private setup

Use 64-bit Windows 10 or 11. Extract the Windows ZIP and keep 2DS.exe, cpf.exe,
and cpf-preview.exe together. The public download contains CPF only.

Separately extract an official portable Azahar build into a fresh, writable
private runtime folder, outside your preservation archive and upload folder.
The tested backend is Azahar 2126.1.2:
https://github.com/azahar-emu/azahar/releases/tag/2126.1.2
Its executable and dependencies are not bundled by CPF.

Open 2DS.exe. The setup window shows the required files together, with a Browse button and filename hint for each row:

1. **Original HOME Menu CXI**: your decrypted USA HOME Menu export, title
   `0004003000008F02`, from your original 6.0.0-12U preservation set. A typical
   export name contains `CTR-N-HMME`. Do not select a raw NAND or a game.
2. **Separate matching HOME Menu backup CXI**: a verified backup copy of the
   same HOME Menu, in a different source file. CPF compares complete SHA-256
   hashes. It refuses a mismatch without preparing runtime state.
3. **azahar.exe**: the executable in the separately extracted private runtime.

No handwritten profile, NAND image, keys, or game is needed for these setup
steps. This is not a full NAND import or complete firmware boot. Version remains
profile metadata, not authenticated proof of firmware version.

CPF leaves both source files where they are. For a fresh runtime it creates a
private `user` directory beside Azahar, with Old 3DS/USA settings, virtual SD,
default portable storage, and update checks disabled. It prepares the verified
HOME return copy in that writable runtime and generates
`user/cpf-setup/original-2ds.cpf-profile.toml` automatically. You do not select or
edit this internal profile. Repeated matching setup reuses it. Existing settings,
profiles, installed titles, and saves are not overwritten. An incompatible
existing runtime gives an error; choose a fresh extracted runtime folder.

These steps write to your private runtime, so close any other process using it
first. Source and backup files must remain outside its writable user directory.
Keep the runtime and generated profile private; never upload them to the repo.
The setup window keeps your selections visible if validation fails. Cancelling a Browse dialog keeps the current selection; Cancel closes setup before emulation starts.

Right-click the device for cartridge insertion/ejection or base CIA installation.
Save before confirming a restart. Initial cartridge support requires decrypted
.cci/.3ds images; CIA installation accepts decrypted base CTR applications and
excludes updates, DLC, system titles, and replacement of installed titles.
No games or other system applications are included or automatically acquired.
DS/DSi support is deferred.

The built-in public shell has a rounded dark body, blue edging, circle pad, D-pad, labelled face buttons, START/SELECT, HOME, and Power. The optional Device artwork row accepts your own RGBA 2DS PNG at 1536 × 1024 pixels. It is read locally and is never added to the public package. Leave it blank to use the built-in shell. Keyboard input follows your selected backend profile.
Esc closes CPF. Read KNOWN_ISSUES.md, especially the unresolved game HOME crash.
