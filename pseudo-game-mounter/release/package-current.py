"""Stage the existing public DSi build and the current Pseudo cartridge UI.

This intentionally does not read development machines, ROM libraries, or probes.
"""
from hashlib import sha256
from pathlib import Path
import json
import subprocess
import zipfile

root = Path(__file__).resolve().parents[1]
cfp = root.parent
staging = cfp / "DSi Release Staging"
public_dsi = staging / "Current"
release = staging / "Release"
exe = root / "src-tauri" / "target" / "release" / "pseudo-game-mounter.exe"
license_file = public_dsi / "LICENSE"

required = [public_dsi / "DSi.exe", public_dsi / "DSi Backend.exe",
            public_dsi / "backend" / "melondsds_system_compat.dll", exe, license_file]
for path in required:
    if not path.is_file():
        raise SystemExit(f"Required release file missing: {path}")
release.mkdir(exist_ok=True)


def write_zip(name: str, entries: list[tuple[str, Path | bytes]]) -> Path:
    output = release / name
    if output.exists():
        raise SystemExit(f"Review existing release archive before replacing it: {output}")
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED,
                         compresslevel=9) as archive:
        for destination, source in entries:
            info = zipfile.ZipInfo(destination, date_time=(2026, 10, 20, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, source if isinstance(source, bytes) else source.read_bytes(),
                             compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    return output


dsi_files = sorted(p for p in public_dsi.rglob("*") if p.is_file())
for path in dsi_files:
    if path.suffix.lower() in {".nds", ".sav", ".bin", ".key", ".ticket"}:
        raise SystemExit(f"Console data in public DSi package: {path}")
dsi_zip = write_zip("CPF-DSi-v0.1.0-alpha-win64.zip", [
    (f"CPF-DSi/{p.relative_to(public_dsi).as_posix()}", p) for p in dsi_files
])

readme = b"""# Pseudo Game Mounter 0.1.0 alpha

Extract this ZIP and run `Pseudo Game Mounter.exe`. Start the current public
`DSi.exe` first or afterward, then choose its running session under Target app.
Choose your own `.nds` image to insert into Slot-1. Check slot and Eject cartridge
operate on the selected live session. This release shows the Cartridge controls;
the experimental Games library tab and DSi Menu tile work are not included.

No Nintendo files, games, saves, BIOS, firmware, or NAND are included. Pseudo is
a separate desktop application; it does not install games onto the DSi Menu.
For standalone melonDS configuration, launch with `--melon`.

CPF-owned code: GPL-3.0-only. See LICENSE and the separate source archive.
"""
exe_data = exe.read_bytes()
manifest_path = str(root / "src-tauri").encode("ascii")
if exe_data.count(manifest_path) != 1:
    raise SystemExit("Expected one Tauri build path in Pseudo executable")
replacement = b"/cpf-public-build/tauri".ljust(len(manifest_path), b"_")
exe_data = exe_data.replace(manifest_path, replacement)
for private in (str(root).encode("ascii"), str(Path.home()).encode("ascii")):
    if private.lower() in exe_data.lower():
        raise SystemExit("A private build path remains in Pseudo executable")
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--format-version", "1", "--filter-platform",
    "x86_64-pc-windows-msvc", "--offline"
], cwd=root / "src-tauri"))
dependencies = sorted((package for package in metadata["packages"]
                       if package["name"] != "pseudo-game-mounter"),
                      key=lambda package: (package["name"], package["version"]))
license_entries = []
inventory = ["# Pseudo Game Mounter Rust dependencies\n\n",
             "Names, versions, and declared licenses resolved for Windows x64.\n\n"]
for package in dependencies:
    key = f"{package['name']}-{package['version']}"
    inventory.append(f"- {key}: {package.get('license') or 'unspecified'}\n")
    cached = public_dsi / "licenses" / "rust" / key
    files = sorted(path for path in cached.iterdir() if path.is_file()) if cached.is_dir() else []
    if not files:
        source = Path(package["manifest_path"]).parent
        files = sorted(path for path in source.iterdir() if path.is_file()
                       and path.name.lower().startswith(("license", "licence", "copying", "notice")))
    if not files:
        raise SystemExit(f"Missing third-party license text for {key}")
    license_entries.extend((f"Pseudo Game Mounter/licenses/rust/{key}/{path.name}", path)
                           for path in files)
license_notes = ("# Third-party notices\n\nPseudo Game Mounter uses Tauri and the Rust "
                 "packages listed in DEPENDENCIES.md. Their license texts are in licenses/rust. "
                 "The standalone melonDS executable and Nintendo console data are not bundled.\n").encode()
pseudo_zip = write_zip("Pseudo-Game-Mounter-v0.1.0-alpha-win64.zip", [
    ("Pseudo Game Mounter/Pseudo Game Mounter.exe", exe_data),
    ("Pseudo Game Mounter/README.md", readme),
    ("Pseudo Game Mounter/LICENSE", license_file),
    ("Pseudo Game Mounter/THIRD_PARTY_LICENSES.md", license_notes),
    ("Pseudo Game Mounter/DEPENDENCIES.md", "".join(inventory).encode()),
] + license_entries)

source_files = [root / "ui" / "index.html"]
source_files += [root / "src-tauri" / name for name in
                 ("Cargo.toml", "Cargo.lock", "tauri.conf.json", "build.rs")]
source_files += sorted(p for p in (root / "src-tauri" / "src").rglob("*")
                       if p.is_file() and "bin" not in p.relative_to(root / "src-tauri" / "src").parts
                       and p.name != "menu_tile.rs")
source_files += sorted(p for p in (root / "src-tauri" / "icons").rglob("*") if p.is_file())
source_files += [Path(__file__)]
source_zip = write_zip("Pseudo-Game-Mounter-v0.1.0-alpha-source.zip", [
    (f"pseudo-game-mounter/{p.relative_to(root).as_posix()}", p) for p in source_files
] + [("pseudo-game-mounter/LICENSE", license_file),
     ("pseudo-game-mounter/README.md", readme)])

archives = [dsi_zip, pseudo_zip, source_zip]
checksums = "".join(f"{sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in archives)
(release / "SHA256SUMS.txt").write_text(checksums, encoding="ascii")
for path in archives:
    with zipfile.ZipFile(path) as archive:
        bad = archive.testzip()
        if bad:
            raise SystemExit(f"Corrupt archive entry: {path.name}: {bad}")
        names = archive.namelist()
        if any(name.lower().endswith((".nds", ".sav", "nand.bin", "firmware.bin", "bios7i.bin", "bios9i.bin"))
               for name in names):
            raise SystemExit(f"Console data name found in {path.name}")
    print(f"{path.name}: {path.stat().st_size} bytes, SHA-256 {sha256(path.read_bytes()).hexdigest()}")
