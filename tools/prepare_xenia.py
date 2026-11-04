"""Validate a known public patch prefix before applying remaining patches."""
import argparse
from pathlib import Path
import re
import subprocess
import tempfile


def run(args, cwd, check=True):
    result = subprocess.run(args, cwd=cwd, capture_output=True)
    if check and result.returncode:
        raise ValueError(result.stderr.decode(errors="replace").strip())
    return result


def normalized(data):
    return None if data is None else data.removeprefix(b"\xef\xbb\xbf").replace(b"\r\n", b"\n")


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("patches", nargs="+", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--require-complete", action="store_true")
    args = parser.parse_args()
    source = args.source.resolve()
    patches = [p.resolve(strict=True) for p in args.patches]
    affected = set()
    for patch in patches:
        for name in re.findall(rb"^\+\+\+ b/(.+)$", patch.read_bytes(), re.M):
            name = name.decode().rstrip("\r")
            if ":" in name or ".." in Path(name).parts or name.startswith(("/", "\\")):
                raise ValueError("Unsafe patch path")
            affected.add(name)
    actual = {}
    for name in affected:
        path = source / name
        if path.is_symlink():
            raise ValueError("Linked source file refused")
        actual[name] = normalized(path.read_bytes()) if path.exists() else None
    matches = []
    with tempfile.TemporaryDirectory(prefix="cpf-xenia-check-", dir=source.parent) as temp:
        temp = Path(temp).resolve()
        if temp.parent != source.parent:
            raise ValueError("Temporary staging escaped the intended parent")
        for name in affected:
            result = run(["git", "show", "HEAD:" + name], source, check=False)
            if result.returncode == 0:
                dest = temp / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(result.stdout)
        for applied in range(len(patches) + 1):
            expected = {name: normalized((temp / name).read_bytes())
                        if (temp / name).exists() else None for name in affected}
            if actual == expected:
                matches.append(applied)
            if applied < len(patches):
                run(["git", "apply", str(patches[applied])], temp)
    if len(matches) != 1:
        raise ValueError("Source differs from all reviewed patch states; nothing changed")
    pending = patches[matches[0]:]
    if args.require_complete:
        changed = run(["git", "-c", "diff.ignoreSubmodules=all", "diff", "--name-only", "HEAD"], source).stdout.decode().splitlines()
        unknown = run(["git", "ls-files", "--others", "--exclude-standard"], source).stdout.decode().splitlines()
        if (set(changed) | set(unknown)) - affected:
            raise ValueError("Unreviewed source files present; package a clean public checkout")
    if args.require_complete and pending:
        raise ValueError("All public patches must be applied before packaging")
    if not args.check:
        for patch in pending:
            run(["git", "apply", "--check", str(patch)], source)
            run(["git", "apply", str(patch)], source)
    print(f"Public source verified: {matches[0]} present, {len(pending)} pending; "
          + ("no source changes." if args.check else "preparation complete."))


if __name__ == "__main__":
    main()
