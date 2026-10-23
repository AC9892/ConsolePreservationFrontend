import sys,json,struct,pathlib,re,hashlib
root=pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0,str(root/'local/python-tools'))
from capstone import Cs,CS_ARCH_X86,CS_MODE_64
trace_path=pathlib.Path((root/'reports/v011-hle-libc-trace-path.txt').read_text().strip())
trace=json.loads(trace_path.read_text())
path=root/'local/shellui-v011-hle-libc/app0/eboot.bin'
b=path.read_bytes();base=0x800000000
rip=int(trace['unhandled_fault']['registers']['rip'],16)
ph=struct.unpack_from('<Q',b,32)[0];stride,count=struct.unpack_from('<HH',b,54)
segments=[]
for i in range(count):
    p=ph+i*stride;t=struct.unpack_from('<I',b,p)[0];off,va=struct.unpack_from('<QQ',b,p+8);size=struct.unpack_from('<Q',b,p+32)[0]
    if t==1:segments.append((va,off,size))
def at(addr,size):
    relative=addr-base
    for va,offset,length in segments:
        if va<=relative and relative+size<=va+length:
            return b[offset+relative-va:offset+relative-va+size],offset+relative-va
    raise ValueError('Address outside file-backed load segment')
stdout=pathlib.Path(trace['stdout']).read_text(errors='replace')
match=re.findall(r'Stub: _Znwm.*returning zero to (0x[0-9a-f]+)',stdout)
return_address=int(match[-1],16)
start=return_address-5
code,file_offset=at(start,64)
assert code[0]==0xe8,'Expected direct CALL ending at logged return address'
fault_bytes,fault_offset=at(rip,3)
assert fault_bytes==bytes.fromhex(trace['unhandled_fault']['instruction_bytes'])[:3]
md=Cs(CS_ARCH_X86,CS_MODE_64)
lines=[f'Original entry point SHA256={hashlib.sha256(b).hexdigest()}',f'Guest image base={base:#x}; fault RIP={rip:#x}; ELF file offset={fault_offset:#x}',f'Logged operator-new return site={return_address:#x}; validated CALL at {start:#x}']
for ins in md.disasm(code,start):
    lines.append(f'{ins.address:#x} {ins.mnemonic:8} {ins.op_str}'+(' ; FAULT' if ins.address==rip else ''))
(root/'reports/v011-hle-libc-guest-fault.txt').write_text('\n'.join(lines))
print('\n'.join(lines))
