"""Read-only analysis of the pinned IPMI ELF; prints private research evidence.

Requires the optional Capstone installation in local/python-tools. Never executes
guest instructions and never writes to the input or original dump.
"""
import argparse
import hashlib
import json
import pathlib
import re
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'local/python-tools'))
from capstone import Cs, CS_ARCH_X86, CS_MODE_64

EXPECTED = 'c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5'


def analyze(module, trace, kernel=None):
    b = module.read_bytes()
    if hashlib.sha256(b).hexdigest() != EXPECTED:
        raise ValueError('Unsupported IPMI module hash; offsets have not been validated')
    if not b.startswith(b'\x7fELF\x02\x01'):
        raise ValueError('Expected ELF64 little endian')
    ph = struct.unpack_from('<Q', b, 32)[0]
    stride, count = struct.unpack_from('<HH', b, 54)
    segments = []
    for i in range(count):
        p = ph + i * stride
        kind, flags, offset, va, _, size = struct.unpack_from('<IIQQQQ', b, p)
        if offset + size > len(b):
            raise ValueError('Segment exceeds file')
        segments.append((kind, flags, offset, va, size))

    def at(va, size):
        for kind, _, offset, start, length in segments:
            if kind == 1 and start <= va and va + size <= start + length:
                return b[offset + va - start:offset + va - start + size]
        raise ValueError('Address not file-backed')

    # Resolve the PLT jump through its SCE relocation, rather than trusting a
    # disassembler label or assuming that the logged callee is this import.
    dyn = next(s for s in segments if s[0] == 2)
    data = next(s for s in segments if s[0] == 0x61000000)
    tags = {}
    for pos in range(dyn[2], dyn[2] + dyn[4], 16):
        tag, value = struct.unpack_from('<QQ', b, pos)
        if not tag:
            break
        tags[tag] = value
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    plt = list(md.disasm(at(0x208, 6), 0x208))[0]
    if plt.mnemonic != 'jmp':
        raise ValueError('Unexpected PLT')
    raw = at(0x208, 6)
    if raw[:2] != b'\xff\x25':
        raise ValueError('Expected RIP-relative indirect PLT jump')
    got = 0x20e + struct.unpack_from('<i', raw, 2)[0]
    reloc = None
    for pos in range(data[2] + tags[0x61000029], data[2] + tags[0x61000029] + tags[0x6100002d], 24):
        address, info, _ = struct.unpack_from('<QQq', b, pos)
        if address == got:
            reloc = info >> 32
            break
    if reloc is None:
        raise ValueError('No relocation for IPMI PLT')
    sym = data[2] + tags[0x61000039] + reloc * tags.get(0x6100003b, 24)
    nameoff = struct.unpack_from('<I', b, sym)[0]
    start = data[2] + tags[0x61000035] + nameoff
    name = b[start:b.index(0, start)].decode()
    if name.split('#')[0] != 'Hk7iHmGxB18':
        raise ValueError('PLT does not resolve to ipmimgr_call NID')
    lines = [f'Module SHA256={EXPECTED}', f'PLT 0x208 -> GOT {got:#x} -> {name}',
             'All addresses below are module-relative virtual addresses.']
    instructions = []
    for kind, flags, _, va, size in segments:
        if kind == 1 and flags & 1:
            instructions.extend(md.disasm(at(va, size), va))
    calls = []
    operations = []
    for i, ins in enumerate(instructions):
        if ins.mnemonic == 'call' and ins.op_str == '0x208':
            calls.append(ins.address)
            setup = instructions[max(0, i - 24):i]
            op = next((int(item.op_str.split(', ')[1], 0) for item in reversed(setup)
                       if item.mnemonic == 'mov' and re.fullmatch(r'edi, (0x[0-9a-f]+|[0-9]+)', item.op_str)), None)
            operations.append((ins.address, op))
            lines.append(f'\nTransport call at {ins.address:#x}; return={ins.address + ins.size:#x}')
            for item in instructions[max(0, i - 18):i + 13]:
                lines.append(f'{item.address:#08x} {item.mnemonic:8} {item.op_str}')
    lines.append('\nOperation selectors (static nearest immediate EDI assignment; not a full dataflow analysis):')
    for call, op in operations:
        lines.append(f'call={call:#x} operation={op:#x}' if op is not None else f'call={call:#x} operation=unknown')
    lines.append('\nDirect callers of the operation-2 wrapper at 0x3fd0:')
    for i, ins in enumerate(instructions):
        if ins.mnemonic == 'call' and ins.op_str == '0x3fd0':
            for item in instructions[max(0, i - 26):i + 17]:
                lines.append(f'{item.address:#08x} {item.mnemonic:8} {item.op_str}')
    if trace:
        result = json.loads(trace.read_text(encoding='utf-8-sig'))
        log = pathlib.Path(result['stdout']).read_text(errors='replace')
        bases = re.findall(r'Loading module libSceIpmi\.sprx to (0x[0-9a-f]+)', log)
        returns = re.findall(r'Stub: ipmimgr_call .*returning zero to (0x[0-9a-f]+)', log)
        if not bases or not returns:
            raise ValueError('Trace does not contain native IPMI transport calls')
        base = int(bases[-1], 16)
        relative = sorted({int(value, 16) - base for value in returns})
        if any(address - 5 not in calls for address in relative):
            raise ValueError('Logged call does not match validated direct CALL')
        lines.append(f'\nRuntime base={base:#x}; {len(returns)} logged calls; return offsets={relative}')
    if kernel:
        kb = kernel.read_bytes()
        if not kb.startswith(b'\x7fELF\x02\x01'):
            raise ValueError('Expected readable ELF64 kernel library')
        headers = []
        kp = struct.unpack_from('<Q', kb, 32)[0]
        ks, kn = struct.unpack_from('<HH', kb, 54)
        for i in range(kn):
            headers.append(struct.unpack_from('<IIQQQQ', kb, kp + i * ks))
        kd = next(h for h in headers if h[0] == 2)
        kbase = next(h for h in headers if h[0] == 0x61000000)[2]
        kt = {}
        for pos in range(kd[2], kd[2] + kd[5], 16):
            tag, value = struct.unpack_from('<QQ', kb, pos)
            if not tag:
                break
            kt[tag] = value
        found = False
        for pos in range(kbase + kt[0x61000039], kbase + kt[0x61000039] + kt[0x6100003f], kt.get(0x6100003b, 24)):
            name, _, _, section, va, size = struct.unpack_from('<IBBHQQ', kb, pos)
            start = kbase + kt[0x61000035] + name
            symbol = kb[start:kb.index(0, start)].decode()
            if section and symbol.startswith('Hk7iHmGxB18#'):
                segment = next(h for h in headers if h[0] == 1 and h[3] <= va and va + size <= h[3] + h[5])
                offset = segment[2] + va - segment[3]
                lines.append(f'\nKernel SHA256={hashlib.sha256(kb).hexdigest()}; {symbol} at {va:#x}')
                for item in md.disasm(kb[offset:offset + size], va):
                    lines.append(f'{item.address:#08x} {item.mnemonic:8} {item.op_str}')
                found = True
        if not found:
            raise ValueError('Kernel library does not export ipmimgr_call')
    return '\n'.join(lines)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('module', type=pathlib.Path)
    parser.add_argument('--trace', type=pathlib.Path)
    parser.add_argument('--kernel', type=pathlib.Path, help='Read-only syscall wrapper analysis')
    args = parser.parse_args()
    print(analyze(args.module, args.trace, args.kernel))
