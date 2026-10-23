"""Read-only, hash-pinned SystemLogger call-contract evidence; JSON to stdout.

Uses the optional local Capstone installation. No guest execution or writes to
the input. Redirect stdout to a private report outside the original dump.
"""
import argparse
import hashlib
import json
from pathlib import Path

from map_shell_services import Elf
from capstone import Cs, CS_ARCH_X86, CS_MODE_64

EXPECTED = '817f9b8efa23e9693e712f5e5818a30505f81aa986a1e96870c3c3cec8f71ab0'


def analyze(path):
    if path.stat().st_size != 52104:
        raise ValueError('Unsupported SystemLogger size')
    elf = Elf(path)
    if hashlib.sha256(elf.b).hexdigest() != EXPECTED:
        raise ValueError('Unsupported SystemLogger hash; offsets not validated')
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    instructions = list(md.disasm(elf.at(0x200, 0x96), 0x200))
    indexed = {i.address: (i.mnemonic, i.op_str) for i in instructions}
    # Exact instructions establish the structural contract. Do not infer service
    # semantics, an executed call, or transport flags from this static slice.
    required = {
        0x21b: ('imul', 'eax, edx, 0x64'),
        0x226: ('mov', 'qword ptr [rbp - 0x38], 8'),
        0x232: ('mov', 'qword ptr [rbp - 0x28], 0x10'),
        0x249: ('mov', 'qword ptr [rbp - 0x58], 4'),
        0x25d: ('mov', 'esi, 0x3299000'),
        0x262: ('mov', 'ecx, 3'),
        0x26e: ('mov', 'dword ptr [rsp], 1'),
        0x275: ('call', 'qword ptr [rax + 0x58]'),
    }
    for address, expected in required.items():
        if indexed.get(address) != expected:
            raise ValueError(f'Contract instruction mismatch at {address:#x}')
    strings = ['SceSystemLoggerService', 'SceSysLogExternalIpcClient']
    if any(s.encode() + b'\0' not in elf.b for s in strings):
        raise ValueError('Expected service/client strings missing')
    return {
        'schema_version': 1,
        'analysis': 'static-only',
        'sha256': EXPECTED,
        'module_bytes': len(elf.b),
        'method_id': '0x03299000',
        'client_vtable_slot': '0x58',
        'input_count': 3,
        'input_layout': ['8-byte count', '16-byte name', 'count * 100-byte rows'],
        'output_count': 1,
        'output_capacity': 4,
        'transport_flags': 'not established by this analysis',
        'service_strings': strings,
        'imports': sorted(set(elf.imports().values())),
        'evidence': [{'address': hex(i.address), 'instruction': f'{i.mnemonic} {i.op_str}'.strip()}
                     for i in instructions],
        'authentic_service_executed': False,
        'authentic_dashboard_verified': False,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('module', type=Path)
    args = parser.parse_args()
    print(json.dumps(analyze(args.module), indent=2))
