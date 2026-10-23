import sys,json,struct,pathlib
root=pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0,str(root/'local/python-tools'))
import pefile
from capstone import Cs,CS_ARCH_X86,CS_MODE_64
report_path=pathlib.Path((root/'reports/v011-trace-path.txt').read_text().strip())
report=json.loads(report_path.read_text())
pe=pefile.PE(str(root/'local/backend-v0.11.0/runtime/shadPS4.exe'))
rip=int(report['unhandled_fault']['registers']['rip'],16)
base=pe.OPTIONAL_HEADER.ImageBase
entries=[e for e in pe.DIRECTORY_ENTRY_EXCEPTION if e.struct.BeginAddress <= rip-base < e.struct.EndAddress]
imports={i.address:(i.name or b'ordinal').decode() for dll in pe.DIRECTORY_ENTRY_IMPORT for i in dll.imports}
md=Cs(CS_ARCH_X86,CS_MODE_64);md.detail=True
lines=[f'PE image base={base:#x}; fault RIP={rip:#x}; RVA={rip-base:#x}']
for e in entries:
    begin=e.struct.BeginAddress;end=e.struct.EndAddress
    lines.append(f'Unwind function range: {base+begin:#x}..{base+end:#x}')
    for ins in md.disasm(pe.get_data(begin,end-begin),base+begin):
        text=f'{ins.address:#x} {ins.mnemonic:8} {ins.op_str}'
        for op in ins.operands:
            if op.type==3 and ins.reg_name(op.mem.base)=='rip':
                addr=ins.address+ins.size+op.mem.disp
                if addr in imports:text+=' ; IAT '+imports[addr]
                elif 0<=addr-base<pe.OPTIONAL_HEADER.SizeOfImage:
                    data=pe.get_data(addr-base,100).split(b'\0')[0]
                    if data and all(32<=x<127 for x in data):text+=' ; '+data.decode()
        if ins.address==rip:text+=' ; FAULT'
        lines.append(text)
(root/'reports/v011-native-fault-disassembly.txt').write_text('\n'.join(lines))
print('\n'.join(lines))
