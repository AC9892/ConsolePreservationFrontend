"""Build an allowlisted public preview without using development bin or user state."""
from pathlib import Path
from hashlib import sha256
import datetime, json, os, shutil, subprocess, sys, tomllib, zipfile

PROJECT = Path(__file__).resolve().parents[1]
ROOT = PROJECT / 'release-staging' / 'public' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
SOURCE = ROOT / 'source'
PACKAGE = ROOT / 'CPF-2DS-public-preview'
DENIED_EXT = {'.bin','.dat','.exefs','.cia','.cxi','.cfa','.cci','.3ds','.tik','.tmd','.app','.nds','.srl','.dsi','.ids','.mem','.firm','.sed','.sav','.key','.keys','.pem','.p12'}
DENIED_NAMES = {'boot9.bin','boot11.bin','nand.bin','otp.bin','otp.mem','movable.sed','essential.exefs','secureinfo_a','localfriendcodeseed_b','dectitlekeys.bin','enctitlekeys.bin'}

def digest(path):
    return sha256(path.read_bytes()).hexdigest()

def run(*args, cwd=PROJECT, env=None):
    subprocess.run(args, cwd=cwd, env=env, check=True)

def copy(source, dest):
    if source.is_symlink() or getattr(source, 'is_junction', lambda: False)():
        raise RuntimeError('Linked release input refused')
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, dest)

def audit(folder, vendor=False):
    count = 0
    roots = [str(PROJECT), str(PROJECT.parent), str(Path.home())]
    needles = []
    for root in roots:
        for value in {root, root.replace('\\','/'), root.replace('\\','\\\\')}:
            needles.extend([value.lower().encode(), value.lower().encode('utf-16le')])
    for path in folder.rglob('*'):
        if path.is_symlink() or getattr(path,'is_junction',lambda:False)():
            raise RuntimeError('Linked artifact refused')
        if not path.is_file(): continue
        relative = path.relative_to(folder)
        third_party = vendor and relative.parts[0] == 'vendor'
        if path.name.lower() in DENIED_NAMES or (not third_party and path.suffix.lower() in DENIED_EXT):
            raise RuntimeError(f'Forbidden artifact: {relative.as_posix()}')
        if (any(part.lower() in {'private','nand','roms','saves','logs','imgs','.git'} for part in relative.parts[:-1]) or relative.parts[0].lower() in {'bin','target'}) and not third_party:
            raise RuntimeError(f'Forbidden directory: {relative.as_posix()}')
        raw = path.read_bytes()
        data = raw.lower()
        if any(needle in data for needle in needles):
            raise RuntimeError(f'Local path detected: {relative.as_posix()}')
        # Compiled CPF must not contain development PNGs, even as encoded blobs.
        if path.suffix.lower() == '.exe':
            for image in (PROJECT/'imgs').glob('*.png'):
                if image.read_bytes() in raw:
                    raise RuntimeError('Development artwork found in executable')
        count += 1
    return count

def manifest(folder):
    entries = [f'{digest(p)}  {p.relative_to(folder).as_posix()}\n' for p in sorted(folder.rglob('*')) if p.is_file() and p.name != 'MANIFEST.sha256']
    (folder/'MANIFEST.sha256').write_text(''.join(entries), encoding='utf-8')

def archive(folder, output):
    with zipfile.ZipFile(output, 'x', zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as z:
        for path in sorted(folder.rglob('*')):
            if path.is_file(): z.write(path, path.relative_to(folder).as_posix())
    with zipfile.ZipFile(output) as z:
        if z.testzip() is not None: raise RuntimeError('Archive CRC validation failed')

def main():
    run(sys.executable, str(PROJECT/'release/test-release.py'))
    ROOT.mkdir(parents=True, exist_ok=False)
    SOURCE.mkdir(); PACKAGE.mkdir()
    for name in ['Cargo.toml','Cargo.lock','build.rs','LICENSE','.gitignore']:
        copy(PROJECT/name, SOURCE/name)
    for path in (PROJECT/'src').rglob('*.rs'):
        copy(path, SOURCE/path.relative_to(PROJECT))
    for name in ['build-public.ps1','prepare-public.py','generate-assets.py','test-release.py','PUBLIC_README.md','INSTALLATION.md','KNOWN_ISSUES.md','THIRD_PARTY_LICENSES.md']:
        copy(PROJECT/'release'/name, SOURCE/'release'/name)
    for name in ['slate.png','fold.png','icon.png']:
        copy(PROJECT/'release/assets'/name, SOURCE/'release/assets'/name)
    copy(PROJECT/'release/PUBLIC_README.md', SOURCE/'README.md')
    # Source candidates default to public artwork; the development checkout does not.
    cargo = SOURCE/'Cargo.toml'
    cargo_text = cargo.read_text()
    if 'default = ["public-release"]' not in cargo_text:
        cargo_text = cargo_text.replace('[features]\n','[features]\ndefault = ["public-release"]\n')
    cargo.write_text(cargo_text, encoding='utf-8')
    run('cargo','vendor','--offline','--locked',str(SOURCE/'vendor'))
    (SOURCE/'.cargo').mkdir()
    (SOURCE/'.cargo/config.toml').write_text('[source.crates-io]\nreplace-with = "vendored-sources"\n[source.vendored-sources]\ndirectory = "vendor"\n', encoding='utf-8')
    inventory = []
    for directory in sorted((SOURCE/'vendor').iterdir()):
        metadata = tomllib.loads((directory/'Cargo.toml').read_text(encoding='utf-8'))['package']
        inventory.append({'name': metadata['name'], 'version':metadata['version'], 'license':metadata.get('license','see license files'), 'repository':metadata.get('repository',''), 'source':'crates.io; checksums in Cargo.lock and .cargo-checksum.json'})
        for path in directory.rglob('*'):
            if path.is_file() and (path.name.upper().startswith(('LICENSE','COPYING','NOTICE','COPYRIGHT'))):
                copy(path, PACKAGE/'licenses'/directory.name/path.relative_to(directory))
    inventory_text = json.dumps(inventory, indent=2)+'\n'
    (SOURCE/'DEPENDENCIES.json').write_text(inventory_text,encoding='utf-8')
    (PACKAGE/'DEPENDENCIES.json').write_text(inventory_text,encoding='utf-8')
    env = os.environ.copy()
    env['CARGO_TARGET_DIR'] = str(PROJECT/'target/public')
    env.pop('RUSTFLAGS',None)
    env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(['--remap-path-prefix='+str(SOURCE)+'=/cpf-source','--remap-path-prefix='+str(PROJECT)+'=/cpf-build','--remap-path-prefix='+str(Path.home())+'=/build-user','-C','strip=symbols','-C','target-feature=+crt-static'])
    run('cargo','build','--release','--offline','--locked','--features','public-release','--bins',cwd=SOURCE,env=env)
    for source,name in [('cpf-2ds.exe','2DS.exe'),('cpf.exe','cpf.exe'),('cpf-preview.exe','cpf-preview.exe')]:
        copy(PROJECT/'target/public/release'/source, PACKAGE/name)
    for source,name in [('PUBLIC_README.md','README.md'),('INSTALLATION.md','INSTALLATION.md'),('KNOWN_ISSUES.md','KNOWN_ISSUES.md'),('THIRD_PARTY_LICENSES.md','THIRD_PARTY_LICENSES.md')]:
        copy(PROJECT/'release'/source, PACKAGE/name)
    copy(PROJECT/'LICENSE',PACKAGE/'LICENSE')
    counts = {'source_files':audit(SOURCE,True),'package_files':audit(PACKAGE),'development_bundle_used':False,'backend_bundled':False,'console_material_bundled':False,'public_setup':'Persistent labelled setup form with filename hints, per-row Browse controls, inline status, and optional local shell artwork','status':'limited preview; gameplay and HOME switching not verified complete'}
    for folder in [SOURCE,PACKAGE]: manifest(folder)
    audit(SOURCE,True); audit(PACKAGE)
    source_zip=ROOT/'CPF-2DS-public-preview-source.zip'
    binary_zip=ROOT/'CPF-2DS-public-preview-windows.zip'
    archive(SOURCE,source_zip); archive(PACKAGE,binary_zip)
    (ROOT/'AUDIT.json').write_text(json.dumps(counts,indent=2)+'\n',encoding='utf-8')
    (ROOT/'SHA256SUMS.txt').write_text(''.join(f'{digest(p)}  {p.name}\n' for p in [source_zip,binary_zip,ROOT/'AUDIT.json']),encoding='utf-8')
    print('Public preview candidate prepared: '+str(ROOT))

if __name__ == "__main__":
    main()
