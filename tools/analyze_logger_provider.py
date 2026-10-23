"""Read-only, hash-pinned ShellCore SystemLogger startup analysis.

JSON goes to stdout; keep raw instruction evidence private. This tool does not
load or execute guest code. Requires the same optional Capstone as the existing
service-reference tooling. Offsets apply only to the explicitly pinned input.
"""
import argparse
import hashlib
import json
import struct
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
    0xe24bd0: 'fJnpuVVBbKk#8#e',  # operator new
    0xe24cc0: 'tsvEmnenz48#8#e',  # destructor registration
}


def mapped_bytes(elf, address, size):
    """File-backed bytes only, including SCE RELRO; never synthesize BSS data."""
    if size < 0:
        raise ValueError('Negative mapped read')
    for kind, _, offset, start, _, length in elf.segments:
        if kind in (1, 0x61000010) and start <= address and address + size <= start + length:
            return elf.b[offset + address - start:offset + address - start + size]
    raise ValueError(f'Unbacked analysis address {address:#x}')


def constructor_state(elf):
    dynamic = next(h for h in elf.segments if h[0] == 2)
    base = next(h for h in elf.segments if h[0] == 0x61000000)[2]
    tags = {}
    for offset in range(dynamic[2], dynamic[2] + dynamic[5], 16):
        tag, value = struct.unpack_from('<QQ', elf.b, offset)
        if not tag:
            break
        tags[tag] = value
    if tags.get(0xc) != 0x20 or tags.get(0x1b) != 0:
        raise ValueError('Unexpected module initialization profile')
    if tags.get(0x61000033) != 24 or tags[0x61000031] % 24:
        raise ValueError('Unexpected relocation stride')
    relocations = {}
    first = base + tags[0x6100002f]
    end = first + tags[0x61000031]
    if end > len(elf.b):
        raise ValueError('Relocation table exceeds input')
    for offset in range(first, end, 24):
        address, info, addend = struct.unpack_from('<QQq', elf.b, offset)
        relocations[address] = (info, addend)
    # The original DT_INIT walks this legacy constructor table backwards.
    # Finding its entry proves static linkage, not that preceding calls return.
    slots = []
    for address in range(0x13fba40, 0x13f0000 - 1, -8):
        value = struct.unpack('<Q', mapped_bytes(elf, address, 8))[0]
        if value == 0xffffffffffffffff:
            break
        slots.append((address, value))
    else:
        raise ValueError('Constructor sentinel missing within bounded segment')
    if (address, len(slots)) != (0x13fb210, 262):
        raise ValueError('Constructor table bounds changed')
    if slots[111] != (0x13fb6c8, 0x4f3230):
        raise ValueError('Logger constructor entry changed')
    table = list(struct.unpack('<7Q', mapped_bytes(elf, 0x13b15f8, 56)))
    expected = [0x4f32c0, 0x4f3480, 0x4f34a0, 0x4f3720,
                0x4f3820, 0x4f3c70, 0x4f4100]
    if table != expected:
        raise ValueError('Logger callback table changed')
    required_relocations = {0x13fb6c8: 0x4f3230}
    required_relocations.update({0x13b15f8 + i * 8: entry for i, entry in enumerate(table)})
    if any(relocations.get(slot) != (8, target)
           for slot, target in required_relocations.items()):
        raise ValueError('Expected relative constructor/callback relocation missing')
    return {
        'module_init': '0x20',
        'legacy_constructor_table_slots': len(slots),
        'logger_constructor_slot': '0x13fb6c8',
        'logger_constructor': '0x4f3230',
        'callback_constructor': '0x4f3270',
        'callback_object': '0x1431608',
        'callback_vtable': '0x13b15f8',
        'local_callback_entries': [hex(entry) for entry in table],
        'relative_relocations_verified': len(required_relocations),
        'constructor_allocation_request': 48,
        'worker_context': {
            'address': '0x1431648', 'server_pointer_offset': 0,
            'working_memory_pointer_offset': 8, 'working_memory_size_offset': 16,
        },
        'constructor_executed': False,
    }


def analyze(path):
    if path.stat().st_size > 128 * 1024 * 1024:
        raise ValueError('Input exceeds analysis bound')
    if hashlib.sha256(path.read_bytes()).hexdigest() != EXPECTED:
        raise ValueError('Unsupported ShellCore hash; offsets not validated')
    elf = Elf(path)
    if hashlib.sha256(elf.b).hexdigest() != EXPECTED:
        raise ValueError('ShellCore changed while reading')
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    slices = {
        'server_setup': (0x4f28ac, 0xee),
        'worker_entry': (0x4f2ea0, 0xc0),
        'thread_setup': (0xc92160, 0x155),
        'module_init': (0x20, 0x5b),
        'callback_init': (0x4f3230, 0x82),
    }
    decoded = {
        name: list(md.disasm(elf.at(start, size), start))
        for name, (start, size) in slices.items()
    }
    indexed = {i.address: (i.mnemonic, i.op_str)
               for instructions in decoded.values() for i in instructions}
    required = {
        0x4f28b0: ('call', '0xe24c50'),
        0x4f28b5: ('vmovups', 'xmm0, xmmword ptr [rip + 0xa5fcca]'),
        0x4f28bd: ('vmovups', 'xmm1, xmmword ptr [rip + 0xa5fcb9]'),
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
        0x55: ('lea', 'rbx, [rip + 0x13fb9e4]'),
        0x60: ('add', 'rbx, -8'),
        0x64: ('mov', 'rax, qword ptr [rbx]'),
        0x6c: ('cmp', 'rax, -1'),
        0x72: ('call', 'rax'),
        0x4f3236: ('lea', 'rbx, [rip + 0xf3e3cb]'),
        0x4f3240: ('call', '0x4f3270'),
        0x4f325c: ('jmp', '0xe24cc0'),
        0x4f3276: ('lea', 'rax, [rip + 0xebe37b]'),
        0x4f3284: ('mov', 'qword ptr [rdi], rax'),
        0x4f328c: ('mov', 'edi, 0x30'),
        0x4f3291: ('call', '0xe24bd0'),
        0x4f3296: ('mov', 'qword ptr [rax], rax'),
        0x4f3299: ('mov', 'qword ptr [rax + 8], rax'),
        0x4f329d: ('mov', 'qword ptr [rax + 0x10], rax'),
        0x4f32a1: ('mov', 'word ptr [rax + 0x18], 0x101'),
        0x4f32a7: ('mov', 'qword ptr [rbx + 0x10], rax'),
        0x4f28ca: ('lea', 'rax, [rip + 0xf3ed37]'),
        0x4f28db: ('mov', 'qword ptr [rbp - 0x50], rax'),
        0x4f294c: ('lea', 'rcx, [rip + 0xf3ecf5]'),
        0x4f2974: ('mov', 'qword ptr [rip + 0xf3ecd5], rax'),
        0x4f2982: ('mov', 'qword ptr [rip + 0xf3eccf], rbx'),
        0x4f2989: ('mov', 'qword ptr [rip + 0xf3ecb8], rax'),
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
        'provider_state': constructor_state(elf),
        'imports': {hex(k): v for k, v in IMPORTS.items()},
        'evidence': {
            name: [{'address': hex(i.address),
                    'instruction': f'{i.mnemonic} {i.op_str}'.strip()}
                   for i in instructions]
            for name, instructions in decoded.items()
        },
        'limitations': [
            'Static slices do not establish reachability from process entry.',
            'Constructor table membership does not prove earlier constructors complete.',
            'Callback entries are structural targets; their service semantics remain unverified.',
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
