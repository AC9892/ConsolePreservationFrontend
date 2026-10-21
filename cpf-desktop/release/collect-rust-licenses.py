"""Copy locally cached Rust package license files into a release candidate."""
from hashlib import sha256
from pathlib import Path
import json
import subprocess
import sys

project = Path(sys.argv[1]).resolve()
candidate = Path(sys.argv[2]).resolve()
output = candidate / "licenses" / "rust"
if output.exists():
    raise SystemExit("Rust license output already exists; use a new candidate directory")
overrides = project / "release" / "license-overrides"
expected_revisions = {
    "alloc-stdlib-0.3.0": "0a81fd6928ea3b33c8cd484aa4575d50ffb98012",
    "defmt-parser-1.0.0": "4a8cdb44891ed57b8ff5a023b6bec7137c48708f",
    "selectors-0.38.0": "572ecba2d1600e7c3d490586692a209faf703baa",
    "webview2-com-0.39.1": "edc2caf886175ccaebe86078c9cfe1ae2a187328",
    "webview2-com-macros-0.8.1": "dffa41a8a46d3f5565eefbff2de57d38d399f158",
    "webview2-com-sys-0.39.1": "edc2caf886175ccaebe86078c9cfe1ae2a187328",
}

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--format-version", "1", "--filter-platform",
    "x86_64-pc-windows-msvc", "--offline"
], cwd=project).decode("utf-8"))
packages = sorted((package for package in metadata["packages"]
                   if package["name"] not in {"cpf-core", "cpf-desktop", "cpf-runtime-worker"}),
                  key=lambda package: (package["name"], package["version"]))
missing = []
manifest = []
for package in packages:
    source = Path(package["manifest_path"]).parent.resolve()
    key = f"{package['name']}-{package['version']}"
    files = sorted(path for path in source.iterdir()
                   if path.is_file() and not path.is_symlink()
                   and path.name.lower().startswith(("license", "licence", "copying", "notice")))
    if not files:
        override = overrides / key
        if override.is_dir():
            vcs = source / ".cargo_vcs_info.json"
            if not vcs.is_file() or json.loads(vcs.read_text(encoding="utf-8"))["git"]["sha1"] != expected_revisions.get(key):
                raise SystemExit(f"Cached source revision changed for {key}; review license override")
            files = sorted(path for path in override.iterdir()
                           if path.is_file() and not path.is_symlink()
                           and path.name.lower().startswith(("license", "licence", "copying", "notice")))
        if not files:
            missing.append(f"- {package['name']} {package['version']}: {package.get('license') or 'license unspecified'}")
    for path in files:
        if path.stat().st_size > 1024 * 1024:
            raise SystemExit(f"Unexpectedly large license file for {package['name']}")
        target = output / key / path.name
        target.parent.mkdir(parents=True, exist_ok=True)
        data = path.read_bytes()
        target.write_bytes(data)
        manifest.append((target.relative_to(candidate).as_posix(), sha256(data).hexdigest()))

readme = ("# Rust package license files\n\n"
          "These files came from the locally cached packages resolved for Windows x64. "
          "Six crates without bundled license files use reviewed source-revision overrides; see OVERRIDE_PROVENANCE.md. "
          "See the dependency inventory for package versions and declared license expressions. "
          "This collection does not decide the CPF project license or replace a final notices review.\n\n"
          "Packages without a license-like file in their cached package root:\n\n"
          + "\n".join(missing) + "\n")
readme_path = output / "README.md"
readme_path.write_text(readme, encoding="utf-8")
manifest.append((readme_path.relative_to(candidate).as_posix(), sha256(readme_path.read_bytes()).hexdigest()))
provenance_path = output / "OVERRIDE_PROVENANCE.md"
provenance_path.write_bytes((overrides / "PROVENANCE.md").read_bytes())
manifest.append((provenance_path.relative_to(candidate).as_posix(), sha256(provenance_path.read_bytes()).hexdigest()))
manifest_path = output / "MANIFEST.sha256"
manifest_path.write_text("".join(f"{digest}  {name}\n" for name, digest in sorted(manifest)), encoding="ascii")
print(f"Collected {len(manifest)-2} Rust license files; {len(missing)} packages have no license text; final notice review remains")
