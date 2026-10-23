"""Read-only static generic IPMI dispatch evidence; no native execution.

Requires optional local Capstone. JSON goes to stdout; keep reports private.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path

from map_shell_services import Elf
from capstone import Cs, CS_ARCH_X86, CS_MODE_64

EXPECTED = 'c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5'


def analyze(path):
    elf = Elf(path)
    if hashlib.sha256(elf.b).hexdigest() != EXPECTED:
        raise ValueError('Unsupported IPMI hash; static offsets not validated')
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    ranges = [(0x1e04, 0x95), (0x1f3c, 0x1d), (0x25e0, 0x1e9)]
    instructions = [i for start, size in ranges for i in md.disasm(elf.at(start, size), start)]
    indexed = {i.address: (i.mnemonic, i.op_str) for i in instructions}
    required = {
        0x1e76: ('cmp', 'eax, 0x8041'),
        0x1e8e: ('call', '0x25e0'),
        0x1f3c: ('cmp', 'eax, 0x8051'),
        0x1f54: ('call', '0x2de0'),
        0x2606: ('mov', 'r9d, dword ptr [rdx + 0x10]'),
        0x260e: ('mov', 'r8d, dword ptr [rdx + 0x14]'),
        0x2612: ('cmp', 'r9, 0xa'),
        0x261c: ('cmp', 'r8d, 0xb'),
        0x2630: ('lea', 'r13, [rdx + 0x18]'),
        0x2650: ('movsxd', 'rax, dword ptr [r13]'),
        0x2664: ('lea', 'r13, [r13 + rax + 4]'),
        0x26cd: ('add', 'r13, 4'),
        0x2789: ('call', 'qword ptr [rax + 0x10]'),
    }
    for address, expected in required.items():
        if indexed.get(address) != expected:
            raise ValueError(f'Dispatch instruction mismatch at {address:#x}')
    table = struct.unpack('<19i', elf.at(0x7580, 76))
    if 0x7580 + table[0] != 0x1e81 or 0x7580 + table[0x10] != 0x1f47:
        raise ValueError('Generic/raw low packet-type dispatch mismatch')
    return {
        'schema_version': 1,
        'analysis': 'static-only; not a captured kernel packet',
        'sha256': EXPECTED,
        'generic_packet_types': ['0x41', '0x8041'],
        'generic_dispatch': '0x25e0',
        'generic_handler_slot': '0x10',
        'raw_packet_types': ['0x51', '0x8051'],
        'raw_dispatch': '0x2de0',
        'generic_counts_maximum': 10,
        'generic_header': {'0x00': 'session object pointer', '0x08': 'process identity',
                           '0x0c': 'method', '0x10': 'input count', '0x14': 'output count'},
        'body': 'At 0x18: repeated u32 length + input bytes; then repeated u32 output capacity',
        'callback_abi_candidate': 'handler, session, method, input-descriptor pointer, input count, output-descriptor pointer, output count',
        'limitations': ['Static offsets apply only to the pinned module.',
                       'Packet flags/sign-bit behavior and output backing selection need runtime verification.',
                       'Reply encoding is not established by this slice.',
                       'The bridge must enforce aggregate and per-buffer bounds before native parsing.'],
        'evidence': [{'address': hex(i.address), 'instruction': f'{i.mnemonic} {i.op_str}'.strip()}
                     for i in instructions],
        'authentic_service_executed': False,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('module', type=Path)
    print(json.dumps(analyze(parser.parse_args().module), indent=2))
