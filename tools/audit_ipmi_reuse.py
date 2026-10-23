"""Compare recovered native IPMI operation selectors with pinned RPCSX source.

This is source-level candidate coverage, never a claim of emulator correctness.
"""
import argparse
import hashlib
import json
import pathlib
import re


def function_body(text, name):
    match = re.search(r'\borbis::' + re.escape(name) + r'\s*\([^;]*?\)\s*\{', text, re.S)
    if not match:
        return ''
    start = match.end()
    depth = 1
    for pos in range(start, len(text)):
        depth += (text[pos] == '{') - (text[pos] == '}')
        if depth == 0:
            return text[start:pos]
    raise ValueError('Unbalanced function body')


def audit(native_evidence, source):
    selectors = [(int(a, 16), int(op, 16)) for a, op in re.findall(
        r'^call=(0x[0-9a-f]+) operation=(0x[0-9a-f]+)$', native_evidence.read_text(), re.M)]
    if not selectors:
        raise ValueError('No native operation selectors found')
    dispatcher_path = source / 'kernel/orbis/src/sys/sys_sce.cpp'
    implementation_path = source / 'kernel/orbis/src/ipmi.cpp'
    dispatcher = dispatcher_path.read_text()
    implementation = implementation_path.read_text()
    body = function_body(dispatcher, 'sys_ipmimgr_call')
    handlers = {}
    for number, handler in re.findall(r'case\s+(0x[0-9a-f]+|\d+)\s*:\s*(?://[^\n]*\n\s*)?return\s+(\w+)\s*\(', body):
        handlers[int(number, 0)] = handler
    operations = []
    for call, op in selectors:
        handler = handlers.get(op)
        contents = function_body(implementation, handler) if handler else ''
        operations.append({
            'operation': hex(op), 'native_call_va': hex(call),
            'rpcsx_dispatch_target': handler,
            'implementation_body_found': bool(contents),
            'todo_marker': 'ORBIS_LOG_TODO' in contents,
            'abort_marker': 'std::abort' in contents,
            'status': ('no-explicit-dispatch-case' if not handler else
                       'direct-dispatch-helper' if not contents else
                       'implementation-with-todo-marker' if 'ORBIS_LOG_TODO' in contents else
                       'source-implementation-candidate'),
            'runtime_verified': False,
        })
    files = [dispatcher_path, implementation_path, source / 'kernel/orbis/include/orbis/ipmi.hpp']
    return {
        'rpcsx_revision': (source / 'REVISION').read_text().strip(),
        'native_call_sites': len(selectors),
        'explicit_dispatch_matches': sum(op['rpcsx_dispatch_target'] is not None for op in operations),
        'operations': operations,
        'source_sha256': {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files},
        'limitations': [
            'Native selector extraction is static; only operation 2 is correlated with the existing runtime trace.',
            'Explicit source handlers can be incomplete. Default dispatch writes success for unsupported operations.',
            'No source has been integrated into PS4.exe or shadPS4, and no RPCSX binary has been run.',
            'Licensing and Windows portability must be resolved before copying or distributing source.',
        ],
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('native_evidence', type=pathlib.Path)
    parser.add_argument('rpcsx_source', type=pathlib.Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.native_evidence, args.rpcsx_source), indent=2))
