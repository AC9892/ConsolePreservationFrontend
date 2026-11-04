"""Stage an allowlisted public backend preview; never copy an entire build folder."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('--source', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    project = Path(__file__).resolve().parent.parent
    source = args.source.resolve(strict=True)
    output = args.output.resolve()
    patches = [project / 'patches' / name for name in (
        'cpf-dashboard.patch', 'cpf-gamer-tile.patch', 'cpf-default-system-image.patch')]
    subprocess.run(['python', str(project / 'tools/prepare_xenia.py'), str(source),
                    *map(str, patches), '--check', '--require-complete'], check=True)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source).decode().strip()
    if revision != '2372a54105d9d598ab5bc7ba94a72ef1bea4b271':
        raise ValueError('Wrong upstream source revision')
    exe = source / 'build/bin/Windows/Release/xenia_canary.exe'
    data = exe.read_bytes()
    if len(data) < 256 or data[:2] != b'MZ':
        raise ValueError('Missing Windows executable')
    offset = struct.unpack_from('<I', data, 0x3c)[0]
    if offset + 24 > len(data) or data[offset:offset+4] != b'PE\0\0':
        raise ValueError('Invalid PE header')
    if struct.unpack_from('<H', data, offset + 4)[0] != 0x8664:
        raise ValueError('Expected Windows x64 build')
    if output.exists():
        raise ValueError('Output already exists; preserve it and choose a fresh path')
    if output == source or source in output.parents:
        raise ValueError('Keep package output outside the source checkout')
    output.mkdir(parents=True)
    shutil.copyfile(exe, output / 'xenia_canary.exe')
    shutil.copyfile(project / 'xenia/LICENSE.xenia', output / 'LICENSE.xenia')
    shutil.copyfile(project / 'PUBLIC-PREVIEW.md', output / 'START-HERE.md')
    shutil.copyfile(project / 'THIRD_PARTY.md', output / 'THIRD_PARTY.md')
    dependencies = []
    tree = subprocess.check_output(['git', 'ls-tree', '-rz', 'HEAD'], cwd=source)
    for entry in tree.split(b'\0'):
        if not entry:
            continue
        metadata, relative = entry.split(b'\t', 1)
        mode, kind, commit = metadata.split()
        if mode != b'160000':
            continue
        relative = relative.decode()
        dependency = source / relative
        if not dependency.is_dir():
            raise ValueError('Missing pinned dependency: ' + relative)
        dependencies.append({'path': relative, 'revision': commit.decode()})
        for notice in dependency.iterdir():
            if (notice.is_file() and not notice.is_symlink() and
                    notice.name.upper().startswith(('LICENSE', 'COPYING', 'COPYRIGHT', 'NOTICE'))):
                dest = output / 'licenses' / dependency.name / notice.name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(notice, dest)
    (output / 'DEPENDENCIES.json').write_text(json.dumps(dependencies, indent=2) + '\n', encoding='utf-8')
    (output / 'portable.txt').touch()
    (output / 'xenia-canary.config.toml').write_text(
        '# Public preview: provide your own dashboard files.\n'
        '[Kernel]\ndashboard_local_uri = true\ndashboard_disc_crypto = true\n'
        'allow_avatar_initialization = false\n'
        'dashboard_default_system_image = ""\n'
        '[General]\ncontroller_hotkeys = false\n', encoding='utf-8')
    records = [{'file': f.relative_to(output).as_posix(), 'bytes': f.stat().st_size,
                'sha256': hashlib.sha256(f.read_bytes()).hexdigest()}
               for f in sorted(output.rglob("*")) if f.is_file()]
    manifest = {'upstream_revision': revision,
                'patches': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in patches},
                'files': records,
                'status': 'staged; manual binary/privacy and runtime validation required before publishing'}
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    print('Staged preview with explicit file allowlist. No profiles, dashboards, logs or configs copied from development.')


if __name__ == '__main__':
    main()
