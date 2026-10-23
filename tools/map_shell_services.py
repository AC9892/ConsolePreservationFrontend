"""Map observed service strings to original ELF code references, read-only.

String presence is not server registration. Candidate RIP-relative LEAs are
reported with nearby calls; instruction-boundary status is not assumed.
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
from capstone.x86 import X86_OP_MEM, X86_REG_RIP

SERVICES = ['SceSystemLoggerService', 'SceDbPreparationServer', 'SceAppDbIpc', 'SceMorpheusUpdService']


class Elf:
    def __init__(self, path):
        if path.stat().st_size > 128 * 1024 * 1024:
            raise ValueError('ELF exceeds research parser bound')
        self.b = path.read_bytes()
        if not self.b.startswith(b'\x7fELF\x02\x01'):
            raise ValueError('Expected readable ELF64')
        ph = struct.unpack_from('<Q', self.b, 32)[0]
        stride, count = struct.unpack_from('<HH', self.b, 54)
        if stride < 56 or count > 4096:
            raise ValueError('Invalid program header table')
        self.segments = [struct.unpack_from('<IIQQQQ', self.b, ph + i * stride) for i in range(count)]
        if any(h[2] + h[5] > len(self.b) for h in self.segments):
            raise ValueError('Segment outside file')

    def at(self, va, size):
        for kind, _, offset, start, _, length in self.segments:
            if kind == 1 and start <= va and va + size <= start + length:
                return self.b[offset + va - start:offset + va - start + size]
        return b''

    def file_va(self, offset):
        for kind, _, start, va, _, size in self.segments:
            if kind == 1 and start <= offset < start + size:
                return va + offset - start
        return None

    def imports(self):
        dynamic = next(h for h in self.segments if h[0] == 2)
        base = next(h for h in self.segments if h[0] == 0x61000000)[2]
        tags = {}
        for pos in range(dynamic[2], dynamic[2] + dynamic[5], 16):
            tag, value = struct.unpack_from('<QQ', self.b, pos)
            if not tag:
                break
            tags[tag] = value
        symbols = []
        for pos in range(base + tags[0x61000039], base + tags[0x61000039] + tags[0x6100003f], tags.get(0x6100003b, 24)):
            name = struct.unpack_from('<I', self.b, pos)[0]
            start = base + tags[0x61000035] + name
            symbols.append(self.b[start:self.b.index(0, start)].decode())
        result = {}
        for pos in range(base + tags.get(0x61000029, 0), base + tags.get(0x61000029, 0) + tags.get(0x6100002d, 0), 24):
            address, info, _ = struct.unpack_from('<QQq', self.b, pos)
            result[address] = symbols[info >> 32]
        return result


def map_file(path, hints, deep=False):
    elf = Elf(path)
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    imports = elf.imports()
    strings = {}
    for service in SERVICES:
        offsets = [m.start() for m in re.finditer(re.escape(service.encode() + b'\0'), elf.b)]
        for offset in offsets:
            va = elf.file_va(offset)
            if va is not None:
                strings[va] = service
    if not strings:
        return None
    references = []
    for kind, flags, offset, va, _, size in elf.segments:
        if kind != 1 or not flags & 1:
            continue
        code = elf.b[offset:offset + size]
        # Common 64-bit RIP-relative LEA encodings, then validate the target.
        matches = []
        for match in re.finditer(rb'[\x48\x4c]\x8d[\x05\x0d\x15\x1d\x25\x2d\x35\x3d].{4}', code, re.DOTALL):
            address = va + match.start()
            target = address + 7 + struct.unpack_from('<i', match.group(), 3)[0]
            if target in strings:
                matches.append((address, target, 'lea-byte-candidate'))
        if deep:
            scan = Cs(CS_ARCH_X86, CS_MODE_64)
            scan.detail = True
            scan.skipdata = True
            for ins in scan.disasm(code, va):
                if not ins.id:
                    continue
                for operand in ins.operands:
                    if operand.type != X86_OP_MEM or operand.mem.base != X86_REG_RIP:
                        continue
                    target = ins.address + ins.size + operand.mem.disp
                    for string_va, service in strings.items():
                        if target <= string_va < target + max(1, operand.size) or string_va <= target < string_va + len(service):
                            matches.append((ins.address, string_va, 'linear-disassembly-memory-reference'))
        seen = set()
        for address, target, method in matches:
            if (address, target) in seen:
                continue
            seen.add((address, target))
            position = address - va
            instructions = list(md.disasm(code[position:position + 192], address))
            context = []
            for ins in instructions:
                record = {'address': hex(ins.address), 'instruction': f'{ins.mnemonic} {ins.op_str}'}
                if ins.mnemonic == 'call' and ins.op_str.startswith('0x'):
                    dest = int(ins.op_str, 16)
                    stub = elf.at(dest, 6)
                    if stub[:2] == b'\xff\x25':
                        got = dest + 6 + struct.unpack_from('<i', stub, 2)[0]
                        symbol = imports.get(got)
                        if symbol:
                            record['import'] = symbol
                            record['name_hint'] = hints.get(symbol.split('#')[0])
                context.append(record)
                if ins.mnemonic == 'ret':
                    break
            references.append({'service': strings[target], 'string_va': hex(target), 'reference_va': hex(address), 'method': method, 'context': context})
    return {'sha256': hashlib.sha256(elf.b).hexdigest(), 'strings': {hex(k): v for k, v in strings.items()}, 'candidate_references': references}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('dump', type=pathlib.Path)
    parser.add_argument('backend_source', type=pathlib.Path)
    parser.add_argument('files', nargs='+', help='Dump-relative ELF files; no recursive scan')
    parser.add_argument('--deep', action='store_true', help='Scan decoded RIP-relative memory operands, including SIMD string copies')
    args = parser.parse_args()
    dump = args.dump.resolve()
    text = (args.backend_source / 'src/core/aerolib/aerolib.inl').read_text()
    hints = dict(re.findall(r'STUB\("([^"]+)",\s*([^\)]+)\)', text))
    report = {'files': {}, 'diagnostics': [], 'limitations': ['LEA candidates need instruction-boundary and control-flow validation; strings alone do not establish service ownership.']}
    for relative in args.files:
        path = (dump / relative).resolve()
        if not path.is_relative_to(dump):
            raise ValueError('Input must remain inside the dump')
        try:
            report['files'][relative] = map_file(path, hints, args.deep)
        except (ValueError, KeyError, StopIteration, struct.error) as error:
            report['diagnostics'].append(f'{relative}: {error}')
    print(json.dumps(report, indent=2))
