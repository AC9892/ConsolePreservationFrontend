"""Read-only, hash-pinned ShellCore SystemLogger startup analysis.

JSON goes to stdout; keep raw instruction evidence private. This tool does not
load or execute guest code. Requires the same optional Capstone as the existing
service-reference tooling. Offsets apply only to the explicitly pinned input.
"""
import argparse
import hashlib
import json
from pathlib import Path

from map_shell_services import Elf
from capstone import Cs, CS_ARCH_X86, CS_MODE_64

EXPECTED = '805bfac472677f362eec6233012468becf50fa2ec23271eefccbca4f9e9455d1'
IMPORTS = {
    0xe24c50: 'vo2FiTXAekk#E#F',  # IPMI server configuration constructor
    0xe24c60: 'OXc1BksfEIg#E#F',  # temporary working-memory estimate
    0xe24c70: 'hoWSsRT0ktA#E#F',  # IPMI server creation
    0xe24c80: 'nsYoNRywwNg#7#U',  # thread attributes initialization
    0xe25990: 'UTXzJbWhhTE#7#U',  # stack size
    0xe2bdd0: 'FXPWHNk8Of0#7#U',  # read scheduling parameters
    0xe258d0: 'eXbUSpEaTsA#7#U',  # inherit-scheduling attribute
    0xe258c0: 'DzES9hQF4f4#7#U',  # scheduling parameters
    0xe25750: '-Wreprtu0Qs#7#U',  # detached-state attribute
    0xe24c90: '6UgtwV+0zb4#7#U',  # create thread
    0xe25560: '3kg7rT0NQIs#7#U',  # exit thread
}


def analyze(path):
    if path.stat().st_size > 128 * 1024 * 1024:
        raise ValueError('Input exceeds analysis bound')
    if hashlib.sha256(path.read_bytes()).hexdigest() != EXPECTED:
        raise ValueError('Unsupported ShellCore hash; offsets not validated')
    elf = Elf(path)
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    slices = {
        'server_setup': (0x4f28ac, 0xee),
        'worker_entry': (0x4f2ea0, 0xc0),
        'thread_setup': (0xc92160, 0x155),
    }
    decoded = {
        name: list(md.disasm(elf.at(start, size), start))
        for name, (start, size) in slices.items()
    }
    indexed = {i.address: (i.mnemonic, i.op_str)
               for instructions in decoded.values() for i in instructions}
    required = {
        0x4f28b0: ('call', '0xe24c50'),
        0x4f28c5: ('mov', 'edi, 0x100'),
        0x4f28fc: ('mov', 'qword ptr [rbp - 0x58], 0x10000'),
        0x4f2904: ('call', '0xe24c60'),
        0x4f2938: ('call', '0xe24c70'),
        0x4f2961: ('lea', 'rdx, [rip + 0x538]'),
        0x4f2968: ('mov', 'r8d, 0x8000'),
        0x4f296e: ('mov', 'r9d, 0x2fe'),
        0x4f2990: ('call', '0xc92160'),
        0x4f2eab: ('mov', 'rax, qword ptr [rdi]'),
        0x4f2eae: ('mov', 'rsi, qword ptr [rdi + 8]'),
        0x4f2eb2: ('mov', 'rdx, qword ptr [rdi + 0x10]'),
        0x4f2ebc: ('call', 'qword ptr [rcx + 0x10]'),
        0x4f2f59: ('call', '0xe25560'),
        0xc92196: ('mov', 'r12, r8'),
        0xc92199: ('mov', 'qword ptr [rbp - 0x70], rdx'),
        0xc9223f: ('mov', 'rsi, r12'),
        0xc92242: ('call', '0xe25990'),
        0xc922a4: ('call', '0xe24c90'),
    }
    for address, expected in required.items():
        if indexed.get(address) != expected:
            raise ValueError(f'Startup instruction mismatch at {address:#x}')
    relocations = elf.imports()
    for plt, expected in IMPORTS.items():
        instruction = elf.at(plt, 6)
        if len(instruction) != 6 or instruction[:2] != b'\xff\x25':
            raise ValueError(f'Expected indirect PLT jump at {plt:#x}')
        got = plt + 6 + int.from_bytes(instruction[2:], 'little', signed=True)
        if relocations.get(got) != expected:
            raise ValueError(f'Unexpected import at {plt:#x}')
    service_name = b'SceSystemLoggerService\0'
    if elf.at(0xf5257e, len(service_name)) != service_name:
        raise ValueError('Expected provider service name missing')
    return {
        'schema_version': 1,
        'analysis': 'static-only',
        'sha256': EXPECTED,
        'service': 'SceSystemLoggerService',
        'server_create_call': '0x4f2938',
        'worker_entry': '0x4f2ea0',
        'worker_server_vtable_slot': '0x10',
        'thread_wrapper': '0xc92160',
        'thread_stack_bytes': 0x8000,
        'thread_scheduling_parameter': 0x2fe,
        'imports': {hex(k): v for k, v in IMPORTS.items()},
        'evidence': {
            name: [{'address': hex(i.address),
                    'instruction': f'{i.mnemonic} {i.op_str}'.strip()}
                   for i in instructions]
            for name, instructions in decoded.items()
        },
        'limitations': [
            'Static slices do not establish reachability from process entry.',
            'No server registration, worker, callback or method handler executed.',
            'Thread attributes and process identity require backend support.',
            'No claim that extracting a function preserves ShellCore semantics.',
        ],
        'authentic_service_executed': False,
        'authentic_dashboard_verified': False,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('shellcore', type=Path)
    args = parser.parse_args()
    print(json.dumps(analyze(args.shellcore), indent=2))
