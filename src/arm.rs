//! A deliberately bounded A32/T16 execution probe. Unsupported behavior stops execution.
use crate::ProcessImage;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    StepLimit,
    DeterministicLoop,
    InstructionFetchFault,
    UnsupportedInstruction,
    UnknownConditionFlags,
    SupervisorCall,
    MemoryFault,
    ThumbStateRequired,
}

impl StopReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::StepLimit => "step limit",
            Self::DeterministicLoop => "repeated CPU state",
            Self::InstructionFetchFault => "instruction fetch fault",
            Self::UnsupportedInstruction => "unsupported instruction",
            Self::UnknownConditionFlags => "condition flags unavailable",
            Self::SupervisorCall => "supervisor call (no kernel service connected)",
            Self::MemoryFault => "mapped memory fault",
            Self::ThumbStateRequired => "Thumb state required (not implemented)",
        }
    }
}

/// Broad encoding class for aggregate diagnostics; it does not expose code bytes.
pub fn instruction_family(word: u32) -> &'static str {
    if word >> 28 == 15 {
        if word & 0x0fff_ff00 == 0x057f_f000 {
            return match (word >> 4) & 15 {
                1 => "CLREX unconditional",
                4 => "DSB unconditional",
                5 => "DMB unconditional",
                6 => "ISB unconditional",
                _ => "other unconditional synchronization encoding",
            };
        }
        if word & 0x0d70_f000 == 0x0550_f000 {
            return "PLD prefetch hint";
        }
        if (word >> 25) & 7 == 5 {
            return "BLX immediate";
        }
        return "unconditional encoding";
    }
    if word & 0x0fff_fff0 == 0x012f_ff10 {
        return "branch and exchange";
    }
    if word & 0x0fc0_00f0 == 0x0000_0090 {
        return "multiply or multiply-accumulate";
    }
    if word & 0x0e00_00f0 == 0x0000_0090 {
        let op = (word >> 20) & 0xff;
        if 0x18 <= op && op <= 0x1f {
            return match (op >> 1 & 3, op & 1) {
                (0, 1) => "LDREX word",
                (0, 0) => "STREX word",
                (1, 1) => "LDREXD doubleword",
                (1, 0) => "STREXD doubleword",
                (2, 1) => "LDREXB byte",
                (2, 0) => "STREXB byte",
                (3, 1) => "LDREXH halfword",
                _ => "STREXH halfword",
            };
        }
        return "swap or other exclusive-memory operation";
    }
    if word & 0x0e00_0090 == 0x0000_0090 && word & 0x60 != 0 {
        return match (word >> 5) & 3 {
            1 => "halfword data transfer",
            2 => "signed byte data transfer",
            _ => "signed halfword data transfer",
        };
    }
    if word & 0x0fff_0ff0 == 0x016f_0f10 {
        return "count leading zeros";
    }
    if word & 0x0e00_0090 == 0x0000_0010 {
        return "register-specified shift or miscellaneous";
    }
    if word & 0x0c00_0000 == 0x0400_0000 {
        if word & (1 << 25) != 0 {
            return "register-offset single data transfer";
        }
        if word & (1 << 24) == 0 {
            return "post-indexed single data transfer";
        }
        if word & (1 << 21) != 0 {
            return "writeback single data transfer";
        }
        if word & 0x0000_f000 == 0x0000_f000 {
            return "PC-target single data transfer";
        }
        return "immediate single data transfer";
    }
    if word & 0x0e00_0000 == 0x0800_0000 {
        let preindexed = word & (1 << 24) != 0;
        let add = word & (1 << 23) != 0;
        let load = word & (1 << 20) != 0;
        return match (preindexed, add, load) {
            (true, false, false) => "STMDB block store",
            (false, true, false) => "STMIA block store",
            (true, true, false) => "STMIB block store",
            (false, false, false) => "STMDA block store",
            (false, true, true) => "LDMIA block load",
            (_, _, true) => "other block load",
        };
    }
    if word & 0x0fff_ffff == 0x0320_f000 {
        return "NOP hint";
    }
    if word & 0x0fff_ffff == 0x0320_f001 {
        return "YIELD hint";
    }
    if word & 0x0fff_ffff == 0x0320_f002 {
        return "WFE hint";
    }
    if word & 0x0fff_ffff == 0x0320_f003 {
        return "WFI hint";
    }
    if word & 0x0fff_ffff == 0x0320_f004 {
        return "SEV hint";
    }
    if word & 0x0fb0_f000 == 0x0320_f000 {
        let fields = (word >> 16) & 15;
        return match (word & (1 << 22) != 0, fields) {
            (false, 8) => "MSR CPSR flags-only immediate",
            (false, 0) => "other MSR-space hint or reserved encoding",
            (false, _) => "MSR CPSR control/status immediate",
            (true, _) => "MSR SPSR immediate",
        };
    }
    if word & 0x0e00_0000 == 0x0200_0000 {
        return match (word >> 21) & 15 {
            0 if word & (1 << 20) != 0 => "ANDS immediate",
            0 => "AND immediate",
            1 => "EOR immediate",
            2 => "SUB immediate",
            3 => "RSB immediate",
            4 => "ADD immediate",
            5 => "ADC immediate",
            6 => "SBC immediate",
            7 => "RSC immediate",
            8 => "TST immediate",
            9 if word & (1 << 20) == 0 => "TEQ immediate without S bit",
            9 if (word >> 12) & 15 != 0 => "TEQ immediate with reserved field",
            9 => "TEQ immediate",
            10 => "CMP immediate",
            11 => "CMN immediate",
            12 => "ORR immediate",
            13 => "MOV immediate",
            14 => "BIC immediate",
            _ if word & (1 << 20) != 0 => "MVNS immediate",
            _ => "MVN immediate",
        };
    }
    if word & 0x0e00_0010 == 0 {
        if (word >> 21) & 15 == 13 {
            if (word >> 12) & 15 == 15 {
                return "MOV register to PC";
            }
            if word & (1 << 20) != 0 {
                return match ((word >> 5) & 3, (word >> 7) & 31) {
                    (0, 0) => "MOVS unshifted register",
                    (3, 0) => "MOVS register with RRX",
                    _ => "MOVS shifted register",
                };
            }
            if (word >> 5) & 3 == 3 && (word >> 7) & 31 == 0 {
                return "MOV register with RRX";
            }
        }
        return match (word >> 21) & 15 {
            0 => "AND register",
            1 => "EOR register",
            2 => "SUB register",
            3 => "RSB register",
            4 => "ADD register",
            5 => "ADC register",
            6 => "SBC register",
            7 => "RSC register",
            8 => "TST register",
            9 => "TEQ register",
            10 => "CMP register",
            11 => "CMN register",
            12 => "ORR register",
            13 => "MOV register",
            14 => "BIC register",
            _ => "MVN register",
        };
    }
    if word & 0x0f00_0010 == 0x0e00_0010 {
        let coprocessor = (word >> 8) & 15;
        return match (coprocessor, word & (1 << 20) != 0) {
            (15, true)
                if (word >> 21) & 7 == 0
                    && (word >> 16) & 15 == 13
                    && word & 15 == 0
                    && (word >> 5) & 7 == 2 =>
            {
                "CP15 user thread register read"
            }
            (15, true)
                if (word >> 21) & 7 == 0
                    && (word >> 16) & 15 == 13
                    && word & 15 == 0
                    && (word >> 5) & 7 == 3 =>
            {
                "CP15 read-only thread register read"
            }
            (15, true) => "other CP15 register read",
            (15, false) => "CP15 register write",
            (10 | 11, true) if (word >> 21) & 7 == 7 => "VMRS status-register read",
            (10, true) if (word >> 21) & 7 == 0 => "VMOV scalar to ARM register",
            (10 | 11, true) => "other VFP register read",
            (10, false) if word & 0x0fff_0fff == 0x0ee1_0a10 => "VMSR FPSCR write",
            (10 | 11, false) if (word >> 21) & 7 == 7 => "other VMSR status-register write",
            (10, false) if (word >> 21) & 7 == 0 => "VMOV ARM register to scalar",
            (10 | 11, false) => "other VFP register write",
            _ => "other coprocessor register transfer",
        };
    }
    match (word >> 25) & 7 {
        0 | 1 => "data processing or miscellaneous",
        2 | 3 => "single data transfer",
        4 => "block data transfer",
        5 => "branch",
        6 => "coprocessor",
        _ => "system or coprocessor",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunReport {
    pub executed: u32,
    pub stop: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelStop {
    Cpu(StopReason),
    ServiceUnavailable,
    ServiceNotAllowed,
    CapabilityMaskUnavailable,
    HandleLimit,
}

impl KernelStop {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu(reason) => reason.label(),
            Self::ServiceUnavailable => "kernel service unavailable",
            Self::ServiceNotAllowed => "SVC not allowed by exheader",
            Self::CapabilityMaskUnavailable => "SVC access mask unavailable",
            Self::HandleLimit => "in-memory handle limit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelRunReport {
    pub executed: u32,
    pub serviced: u32,
    pub stop: KernelStop,
}

/// Small local kernel model. Handles are opaque and have no host resources.
pub struct KernelProbe {
    next_handle: u32,
    address_arbiters: HashSet<u32>,
}

impl KernelProbe {
    pub fn new() -> Self {
        Self {
            next_handle: 0x100,
            address_arbiters: HashSet::new(),
        }
    }

    pub fn address_arbiter_count(&self) -> usize {
        self.address_arbiters.len()
    }

    fn service_pending(
        &mut self,
        cpu: &mut ArmProbe,
        process: &ProcessImage,
    ) -> Result<(), KernelStop> {
        let number = cpu
            .pending_supervisor_call(process)
            .ok_or(KernelStop::ServiceUnavailable)?;
        match process.svc_allowed(number) {
            Some(true) => {}
            Some(false) => return Err(KernelStop::ServiceNotAllowed),
            None => return Err(KernelStop::CapabilityMaskUnavailable),
        }
        match number {
            0x01 => {
                // Model only zero-size MEMOP_FREE results that do not change
                // the process address space. Real frees need a VM manager.
                let operation = cpu.registers[0];
                let address = cpu.registers[1];
                let other_address = cpu.registers[2];
                let size = cpu.registers[3];
                let permissions = cpu.registers[4];
                if operation != 1
                    || address & 0xfff != 0
                    || other_address & 0xfff != 0
                    || size != 0
                    || permissions > 3
                {
                    return Err(KernelStop::ServiceUnavailable);
                }
                cpu.registers[0] = if address == 0 {
                    0xe0e0_1bf5 // ResultInvalidAddress
                } else if (0x0800_0000..0x1000_0000).contains(&address) {
                    0 // ResultSuccess, no memory change for a zero-size free
                } else {
                    return Err(KernelStop::ServiceUnavailable);
                };
            }
            0x21 => {
                if self.address_arbiters.len() >= 64 {
                    return Err(KernelStop::HandleLimit);
                }
                let handle = self.next_handle;
                self.next_handle += 1;
                self.address_arbiters.insert(handle);
                cpu.registers[0] = 0; // ResultSuccess
                cpu.registers[1] = handle;
            }
            _ => return Err(KernelStop::ServiceUnavailable),
        }
        cpu.registers[15] = cpu.registers[15].wrapping_add(4);
        cpu.exclusive = None;
        cpu.states.clear();
        Ok(())
    }
}

impl Default for KernelProbe {
    fn default() -> Self {
        Self::new()
    }
}

/// Executes a small A32 subset in private memory, never invoking host services.
/// The initial condition flags are unknown; conditional code cannot run until
/// a supported flag-setting instruction establishes them.
pub struct ArmProbe {
    registers: [u32; 16],
    flags: Option<Flags>,
    visited: HashSet<u32>,
    states: HashSet<([u32; 16], Option<Flags>, Option<(u32, u32)>, u32, bool)>,
    exclusive: Option<(u32, u32)>,
    fpscr: u32,
    writes: u32,
    thumb: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Flags {
    n: bool,
    z: bool,
    c: bool,
    v: bool,
}

impl ArmProbe {
    pub fn new(process: &ProcessImage) -> Self {
        let mut registers = [0; 16];
        registers[13] = process.stack_range().map_or(0, |(_, top)| top);
        registers[15] = process.layout().text.address;
        Self {
            registers,
            flags: None,
            visited: HashSet::new(),
            states: HashSet::new(),
            exclusive: None,
            fpscr: 0,
            writes: 0,
            thumb: false,
        }
    }

    pub fn register(&self, index: usize) -> Option<u32> {
        self.registers.get(index).copied()
    }
    pub fn pending_instruction_family(&self, process: &ProcessImage) -> Option<&'static str> {
        let pc = self.registers[15];
        if self.thumb {
            process.fetch_u16(pc).ok().map(thumb_instruction_family)
        } else {
            process.fetch_u32(pc).ok().map(instruction_family)
        }
    }
    pub fn distinct_instruction_addresses(&self) -> usize {
        self.visited.len()
    }
    pub fn memory_writes(&self) -> u32 {
        self.writes
    }
    /// Reports only whether a pending A32 BLX immediate reaches mapped Thumb text.
    pub fn pending_blx_thumb_entry_readable(&self, process: &ProcessImage) -> Option<bool> {
        if self.thumb {
            return None;
        }
        let pc = self.registers[15];
        let word = process.fetch_u32(pc).ok()?;
        if word >> 28 != 15 || (word >> 25) & 7 != 5 {
            return None;
        }
        let target = blx_immediate_target(pc, word);
        Some(process.fetch_u16(target).is_ok())
    }
    pub fn pending_supervisor_call(&self, process: &ProcessImage) -> Option<u32> {
        if self.thumb {
            return None;
        }
        let word = process.fetch_u32(self.registers[15]).ok()?;
        (word & 0x0f00_0000 == 0x0f00_0000).then_some(word & 0x00ff_ffff)
    }

    /// Continue through supported in-memory kernel calls within a total step bound.
    pub fn run_with_kernel(
        &mut self,
        process: &mut ProcessImage,
        kernel: &mut KernelProbe,
        max_steps: u32,
    ) -> KernelRunReport {
        let limit = max_steps.min(1_000_000);
        let mut executed = 0;
        let mut serviced = 0;
        loop {
            if executed >= limit {
                return KernelRunReport {
                    executed,
                    serviced,
                    stop: KernelStop::Cpu(StopReason::StepLimit),
                };
            }
            let report = self.run(process, limit - executed);
            executed += report.executed;
            if report.stop != StopReason::SupervisorCall {
                return KernelRunReport {
                    executed,
                    serviced,
                    stop: KernelStop::Cpu(report.stop),
                };
            }
            if let Err(stop) = kernel.service_pending(self, process) {
                return KernelRunReport {
                    executed,
                    serviced,
                    stop,
                };
            }
            executed += 1;
            serviced += 1;
        }
    }

    pub fn run(&mut self, process: &mut ProcessImage, max_steps: u32) -> RunReport {
        let max_steps = max_steps.min(1_000_000);
        for executed in 0..max_steps {
            if !self.states.insert((
                self.registers,
                self.flags,
                self.exclusive,
                self.fpscr,
                self.thumb,
            )) {
                return RunReport {
                    executed,
                    stop: StopReason::DeterministicLoop,
                };
            }
            let pc = self.registers[15];
            self.visited.insert(pc);
            if self.thumb {
                let halfword = match process.fetch_u16(pc) {
                    Ok(value) => value,
                    Err(_) => {
                        return RunReport {
                            executed,
                            stop: StopReason::InstructionFetchFault,
                        }
                    }
                };
                if halfword == 0xbf00 {
                    self.registers[15] = pc.wrapping_add(2); // T16 NOP.
                    continue;
                }
                if halfword & 0xf800 == 0x4800 {
                    // LDR Rt,[PC,#imm8*4] uses the word-aligned T16 PC value.
                    let rt = ((halfword >> 8) & 7) as usize;
                    let address = pc.wrapping_add(4) & !3;
                    let address = address.wrapping_add(u32::from(halfword & 0xff) * 4);
                    let mut value = [0; 4];
                    if process.read(address, &mut value).is_err() {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    self.registers[rt] = u32::from_le_bytes(value);
                    self.registers[15] = pc.wrapping_add(2);
                    continue;
                }
                if halfword & 0xfe00 == 0xb400 {
                    // PUSH {low registers, optional LR} is STMDB SP!.
                    let list = (halfword & 0xff) as u32;
                    let save_lr = halfword & 0x100 != 0;
                    let count = list.count_ones() + u32::from(save_lr);
                    if count == 0 {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        };
                    }
                    let start = self.registers[13].wrapping_sub(count * 4);
                    if start & 3 != 0 {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    let mut values = Vec::with_capacity((count * 4) as usize);
                    for register in 0..8 {
                        if list & (1 << register) != 0 {
                            values.extend_from_slice(&self.registers[register].to_le_bytes());
                        }
                    }
                    if save_lr {
                        values.extend_from_slice(&self.registers[14].to_le_bytes());
                    }
                    if process.write(start, &values).is_err() {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    self.registers[13] = start;
                    self.registers[15] = pc.wrapping_add(2);
                    self.writes += count;
                    self.states.clear();
                    continue;
                }
                if halfword & 0xfe00 == 0xbc00 {
                    // POP {low registers, optional PC} is LDMIA SP!.
                    let list = (halfword & 0xff) as u32;
                    let load_pc = halfword & 0x100 != 0;
                    let count = list.count_ones() + u32::from(load_pc);
                    if count == 0 {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        };
                    }
                    let start = self.registers[13];
                    if start & 3 != 0 {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    let mut values = vec![0; (count * 4) as usize];
                    if process.read(start, &mut values).is_err() {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    let target = if load_pc {
                        Some(u32::from_le_bytes(
                            values[values.len() - 4..].try_into().unwrap(),
                        ))
                    } else {
                        None
                    };
                    if target.is_some_and(|address| address & 1 == 0 && address & 3 != 0) {
                        return RunReport {
                            executed,
                            stop: StopReason::InstructionFetchFault,
                        };
                    }
                    let mut offset = 0;
                    for register in 0..8 {
                        if list & (1 << register) != 0 {
                            self.registers[register] =
                                u32::from_le_bytes(values[offset..offset + 4].try_into().unwrap());
                            offset += 4;
                        }
                    }
                    self.registers[13] = start.wrapping_add(count * 4);
                    if let Some(address) = target {
                        self.thumb = address & 1 != 0;
                        self.registers[15] = address & !1;
                    } else {
                        self.registers[15] = pc.wrapping_add(2);
                    }
                    continue;
                }
                if halfword & 0xff87 == 0x4700 {
                    let rm = ((halfword >> 3) & 15) as usize;
                    let target = if rm == 15 {
                        pc.wrapping_add(4)
                    } else {
                        self.registers[rm]
                    };
                    if target & 1 == 0 && target & 3 != 0 {
                        return RunReport {
                            executed,
                            stop: StopReason::InstructionFetchFault,
                        };
                    }
                    self.thumb = target & 1 != 0;
                    self.registers[15] = target & !1;
                    continue;
                }
                if halfword & 0xff00 == 0x4400 {
                    // T16 ADD (high register), excluding PC as destination.
                    let rd = (((halfword >> 7) & 1) << 3 | (halfword & 7)) as usize;
                    let rm = ((halfword >> 3) & 15) as usize;
                    if rd == 15 {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        };
                    }
                    let operand = if rm == 15 {
                        pc.wrapping_add(4)
                    } else {
                        self.registers[rm]
                    };
                    self.registers[rd] = self.registers[rd].wrapping_add(operand);
                    self.registers[15] = pc.wrapping_add(2);
                    continue;
                }
                if halfword & 0xffc0 == 0x4280 {
                    // T16 CMP (register), low registers only.
                    let rn = (halfword & 7) as usize;
                    let rm = ((halfword >> 3) & 7) as usize;
                    let left = self.registers[rn];
                    let right = self.registers[rm];
                    let (value, borrow) = left.overflowing_sub(right);
                    self.flags = Some(Flags {
                        n: value >> 31 != 0,
                        z: value == 0,
                        c: !borrow,
                        v: (left ^ right) & (left ^ value) & 0x8000_0000 != 0,
                    });
                    self.registers[15] = pc.wrapping_add(2);
                    continue;
                }
                if halfword & 0xf000 == 0xd000 {
                    let cond = u32::from((halfword >> 8) & 15);
                    if cond >= 14 {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        };
                    }
                    let Some(flags) = self.flags else {
                        return RunReport {
                            executed,
                            stop: StopReason::UnknownConditionFlags,
                        };
                    };
                    if condition_passes(cond, flags) {
                        let displacement = (((u32::from(halfword & 0xff) << 1) as i32) << 23) >> 23;
                        self.registers[15] = pc.wrapping_add(4).wrapping_add_signed(displacement);
                    } else {
                        self.registers[15] = pc.wrapping_add(2);
                    }
                    continue;
                }
                if halfword & 0xe000 == 0x6000 {
                    // T16 STR/LDR and STRB/LDRB with unsigned immediate offset.
                    let byte = halfword & 0x1000 != 0;
                    let load = halfword & 0x0800 != 0;
                    let offset = u32::from((halfword >> 6) & 31) * if byte { 1 } else { 4 };
                    let rn = ((halfword >> 3) & 7) as usize;
                    let rt = (halfword & 7) as usize;
                    let address = self.registers[rn].wrapping_add(offset);
                    if !byte && address & 3 != 0 {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    if load {
                        if byte {
                            let mut value = [0];
                            if process.read(address, &mut value).is_err() {
                                return RunReport {
                                    executed,
                                    stop: StopReason::MemoryFault,
                                };
                            }
                            self.registers[rt] = u32::from(value[0]);
                        } else {
                            let mut value = [0; 4];
                            if process.read(address, &mut value).is_err() {
                                return RunReport {
                                    executed,
                                    stop: StopReason::MemoryFault,
                                };
                            }
                            self.registers[rt] = u32::from_le_bytes(value);
                        }
                    } else {
                        let value = self.registers[rt];
                        let result = if byte {
                            process.write(address, &[value as u8])
                        } else {
                            process.write(address, &value.to_le_bytes())
                        };
                        if result.is_err() {
                            return RunReport {
                                executed,
                                stop: StopReason::MemoryFault,
                            };
                        }
                        self.writes += 1;
                        self.states.clear();
                    }
                    self.registers[15] = pc.wrapping_add(2);
                    continue;
                }
                if halfword & 0xf800 == 0xe000 {
                    // T16 B uses a signed 11-bit halfword displacement from PC+4.
                    let displacement = (((u32::from(halfword & 0x7ff) << 1) as i32) << 20) >> 20;
                    self.registers[15] = pc.wrapping_add(4).wrapping_add_signed(displacement);
                    continue;
                }
                return RunReport {
                    executed,
                    stop: StopReason::UnsupportedInstruction,
                };
            }
            let word = match process.fetch_u32(pc) {
                Ok(word) => word,
                Err(_) => {
                    return RunReport {
                        executed,
                        stop: StopReason::InstructionFetchFault,
                    }
                }
            };
            let cond = word >> 28;
            if cond == 15 {
                if (word >> 25) & 7 != 5 {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                let target = blx_immediate_target(pc, word);
                if process.fetch_u16(target).is_err() {
                    return RunReport {
                        executed,
                        stop: StopReason::InstructionFetchFault,
                    };
                }
                self.registers[14] = pc.wrapping_add(4);
                self.registers[15] = target;
                self.thumb = true;
                continue;
            }
            if cond != 14 {
                let Some(flags) = self.flags else {
                    return RunReport {
                        executed,
                        stop: StopReason::UnknownConditionFlags,
                    };
                };
                if !condition_passes(cond, flags) {
                    self.registers[15] = pc.wrapping_add(4);
                    continue;
                }
            }
            if word & 0x0fff_fff0 == 0x012f_ff10 {
                let rm = (word & 15) as usize;
                let target = if rm == 15 {
                    pc.wrapping_add(8)
                } else {
                    self.registers[rm]
                };
                if target & 1 != 0 {
                    self.thumb = true;
                    self.registers[15] = target & !1;
                    continue;
                }
                if target & 3 != 0 {
                    return RunReport {
                        executed,
                        stop: StopReason::InstructionFetchFault,
                    };
                }
                self.registers[15] = target;
                continue;
            }
            if word & 0x0f00_0000 == 0x0f00_0000 {
                return RunReport {
                    executed,
                    stop: StopReason::SupervisorCall,
                };
            }
            if word & 0x0fff_ffff == 0x0320_f000 {
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            // MSR immediate overlaps the data-processing immediate space. A
            // CPSR control/status write needs processor modes and banked state.
            if word & 0x0fb0_f000 == 0x0320_f000 {
                return RunReport {
                    executed,
                    stop: StopReason::UnsupportedInstruction,
                };
            }
            if word & 0x0fff_0fff == 0x0ee1_0a10 {
                let rt = ((word >> 12) & 15) as usize;
                if rt == 15 {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                self.fpscr = self.registers[rt];
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0fff_0fff == 0x0ef1_0a10 {
                let rt = ((word >> 12) & 15) as usize;
                if rt == 15 {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                self.registers[rt] = self.fpscr;
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0f00_0010 == 0x0e00_0010
                && word & (1 << 20) != 0
                && (word >> 8) & 15 == 15
                && (word >> 21) & 7 == 0
                && (word >> 16) & 15 == 13
                && word & 15 == 0
                && (word >> 5) & 7 == 3
            {
                let rd = ((word >> 12) & 15) as usize;
                if rd == 15 {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                self.registers[rd] = process.main_thread_tls_address();
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0ff0_0fff == 0x0190_0f9f {
                let rn = ((word >> 16) & 15) as usize;
                let rt = ((word >> 12) & 15) as usize;
                if rn == 15 || rt == 15 {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                let address = self.registers[rn];
                if address & 3 != 0 {
                    return RunReport {
                        executed,
                        stop: StopReason::MemoryFault,
                    };
                }
                let mut bytes = [0; 4];
                if process.read(address, &mut bytes).is_err() {
                    return RunReport {
                        executed,
                        stop: StopReason::MemoryFault,
                    };
                }
                let value = u32::from_le_bytes(bytes);
                self.registers[rt] = value;
                self.exclusive = Some((address, value));
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0ff0_0ff0 == 0x0180_0f90 {
                let rn = ((word >> 16) & 15) as usize;
                let rd = ((word >> 12) & 15) as usize;
                let rt = (word & 15) as usize;
                if rn == 15 || rd == 15 || rt == 15 || rd == rn || rd == rt {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                let address = self.registers[rn];
                if address & 3 != 0 {
                    return RunReport {
                        executed,
                        stop: StopReason::MemoryFault,
                    };
                }
                let mut bytes = [0; 4];
                if process.read(address, &mut bytes).is_err() {
                    return RunReport {
                        executed,
                        stop: StopReason::MemoryFault,
                    };
                }
                let current = u32::from_le_bytes(bytes);
                let success = self
                    .exclusive
                    .take()
                    .is_some_and(|(reserved_address, expected)| {
                        reserved_address == address && expected == current
                    });
                if success {
                    if process
                        .write(address, &self.registers[rt].to_le_bytes())
                        .is_err()
                    {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    self.writes += 1;
                    self.states.clear();
                }
                self.registers[rd] = u32::from(!success);
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0e00_0000 == 0x0a00_0000 {
                let displacement = (((word & 0x00ff_ffff) << 2) as i32) << 6 >> 6;
                if word & (1 << 24) != 0 {
                    self.registers[14] = pc.wrapping_add(4);
                }
                self.registers[15] = pc.wrapping_add(8).wrapping_add_signed(displacement);
                continue;
            }
            if word & 0x0e00_0000 == 0x0400_0000 {
                // Immediate-offset LDR/STR and LDRB/STRB, pre-indexed without writeback.
                let preindexed = word & (1 << 24) != 0;
                let add = word & (1 << 23) != 0;
                let byte = word & (1 << 22) != 0;
                let writeback = word & (1 << 21) != 0;
                let load = word & (1 << 20) != 0;
                let rn = ((word >> 16) & 15) as usize;
                let rd = ((word >> 12) & 15) as usize;
                if rd == 15
                    || (writeback && (!preindexed || rn == 15 || rn == rd))
                    || (!preindexed && (rn == 15 || load && rn == rd))
                {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                let base = if rn == 15 {
                    pc.wrapping_add(8)
                } else {
                    self.registers[rn]
                };
                let offset = word & 0xfff;
                let adjusted = if add {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let address = if preindexed { adjusted } else { base };
                if !byte && address & 3 != 0 {
                    return RunReport {
                        executed,
                        stop: StopReason::MemoryFault,
                    };
                }
                if load {
                    if byte {
                        let mut value = [0];
                        if process.read(address, &mut value).is_err() {
                            return RunReport {
                                executed,
                                stop: StopReason::MemoryFault,
                            };
                        }
                        self.registers[rd] = u32::from(value[0]);
                    } else {
                        let mut value = [0; 4];
                        if process.read(address, &mut value).is_err() {
                            return RunReport {
                                executed,
                                stop: StopReason::MemoryFault,
                            };
                        }
                        self.registers[rd] = u32::from_le_bytes(value);
                    }
                } else {
                    let value = self.registers[rd];
                    let result = if byte {
                        process.write(address, &[value as u8])
                    } else {
                        process.write(address, &value.to_le_bytes())
                    };
                    if result.is_err() {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    self.writes += 1;
                    self.states.clear();
                }
                if !preindexed || writeback {
                    self.registers[rn] = adjusted;
                }
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0e00_0000 == 0x0800_0000 {
                // STMDB/LDMIA without user-mode transfer. PC loads may branch.
                let preindexed = word & (1 << 24) != 0;
                let add = word & (1 << 23) != 0;
                let user_mode = word & (1 << 22) != 0;
                let writeback = word & (1 << 21) != 0;
                let load = word & (1 << 20) != 0;
                let rn = ((word >> 16) & 15) as usize;
                let list = word & 0xffff;
                if user_mode
                    || rn == 15
                    || list == 0
                    || (!load && list & (1 << 15) != 0)
                    || writeback && list & (1 << rn) != 0
                    || !((preindexed && !add && !load)
                        || (!preindexed && add && load)
                        || (!preindexed && add && !load))
                {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                let bytes = list.count_ones() * 4;
                let base = self.registers[rn];
                let start = if load || add {
                    base
                } else {
                    base.wrapping_sub(bytes)
                };
                if start & 3 != 0 {
                    return RunReport {
                        executed,
                        stop: StopReason::MemoryFault,
                    };
                }
                if load {
                    let mut values = vec![0; bytes as usize];
                    if process.read(start, &mut values).is_err() {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    let next_pc = if list & (1 << 15) != 0 {
                        let target =
                            u32::from_le_bytes(values[values.len() - 4..].try_into().unwrap());
                        if target & 1 != 0 {
                            return RunReport {
                                executed,
                                stop: StopReason::ThumbStateRequired,
                            };
                        }
                        if target & 3 != 0 {
                            return RunReport {
                                executed,
                                stop: StopReason::InstructionFetchFault,
                            };
                        }
                        Some(target)
                    } else {
                        None
                    };
                    let mut offset = 0;
                    for register in 0..15 {
                        if list & (1 << register) != 0 {
                            self.registers[register] =
                                u32::from_le_bytes(values[offset..offset + 4].try_into().unwrap());
                            offset += 4;
                        }
                    }
                    if writeback {
                        self.registers[rn] = base.wrapping_add(bytes);
                    }
                    self.registers[15] = next_pc.unwrap_or_else(|| pc.wrapping_add(4));
                } else {
                    let mut values = Vec::with_capacity(bytes as usize);
                    for register in 0..15 {
                        if list & (1 << register) != 0 {
                            values.extend_from_slice(&self.registers[register].to_le_bytes());
                        }
                    }
                    if process.write(start, &values).is_err() {
                        return RunReport {
                            executed,
                            stop: StopReason::MemoryFault,
                        };
                    }
                    self.writes += list.count_ones();
                    self.states.clear();
                    if writeback {
                        self.registers[rn] = if add { base.wrapping_add(bytes) } else { start };
                    }
                    self.registers[15] = pc.wrapping_add(4);
                }
                continue;
            }
            if word & 0x0e00_0000 == 0x0200_0000 {
                let opcode = (word >> 21) & 15;
                let set_flags = word & (1 << 20) != 0;
                let rn = ((word >> 16) & 15) as usize;
                let rd = ((word >> 12) & 15) as usize;
                let rotate = ((word >> 8) & 15) * 2;
                let immediate = (word & 255).rotate_right(rotate);
                // PC operands and flag-setting MOV require more architecture state.
                if rn == 15 && opcode != 13 || rd == 15 || opcode == 13 && set_flags {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                match opcode {
                    0 => {
                        // AND/ANDS
                        let old = if set_flags {
                            let Some(old) = self.flags else {
                                return RunReport {
                                    executed,
                                    stop: StopReason::UnknownConditionFlags,
                                };
                            };
                            Some(old)
                        } else {
                            None
                        };
                        let value = self.registers[rn] & immediate;
                        self.registers[rd] = value;
                        if let Some(old) = old {
                            self.flags = Some(Flags {
                                n: value >> 31 != 0,
                                z: value == 0,
                                c: if rotate == 0 {
                                    old.c
                                } else {
                                    immediate >> 31 != 0
                                },
                                v: old.v,
                            });
                        }
                    }
                    8 | 9 if set_flags && rd == 0 => {
                        // TST/TEQ
                        let Some(old) = self.flags else {
                            return RunReport {
                                executed,
                                stop: StopReason::UnknownConditionFlags,
                            };
                        };
                        let value = if opcode == 8 {
                            self.registers[rn] & immediate
                        } else {
                            self.registers[rn] ^ immediate
                        };
                        self.flags = Some(Flags {
                            n: value >> 31 != 0,
                            z: value == 0,
                            c: if rotate == 0 {
                                old.c
                            } else {
                                immediate >> 31 != 0
                            },
                            v: old.v,
                        });
                    }
                    13 if rn == 0 => self.registers[rd] = immediate, // MOV
                    15 if rn == 0 && !set_flags => self.registers[rd] = !immediate, // MVN
                    4 | 2 | 10 => {
                        // ADD, SUB, CMP
                        let left = self.registers[rn];
                        let (value, carry, overflow) = if opcode == 4 {
                            let (value, carry) = left.overflowing_add(immediate);
                            (
                                value,
                                carry,
                                (left ^ value) & (immediate ^ value) & 0x8000_0000 != 0,
                            )
                        } else {
                            let (value, borrow) = left.overflowing_sub(immediate);
                            (
                                value,
                                !borrow,
                                (left ^ immediate) & (left ^ value) & 0x8000_0000 != 0,
                            )
                        };
                        if opcode == 10 {
                            if !set_flags {
                                return RunReport {
                                    executed,
                                    stop: StopReason::UnsupportedInstruction,
                                };
                            }
                        } else {
                            self.registers[rd] = value;
                        }
                        if set_flags {
                            self.flags = Some(Flags {
                                n: value >> 31 != 0,
                                z: value == 0,
                                c: carry,
                                v: overflow,
                            });
                        }
                    }
                    _ => {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        }
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            if word & 0x0e00_0010 == 0 {
                let opcode = (word >> 21) & 15;
                let set_flags = word & (1 << 20) != 0;
                let rn = ((word >> 16) & 15) as usize;
                let rd = ((word >> 12) & 15) as usize;
                let rm = (word & 15) as usize;
                let shift = (word >> 7) & 31;
                let shift_type = (word >> 5) & 3;
                if rd == 15 || opcode == 13 && rn != 0 {
                    return RunReport {
                        executed,
                        stop: StopReason::UnsupportedInstruction,
                    };
                }
                let source = if rm == 15 {
                    pc.wrapping_add(8)
                } else {
                    self.registers[rm]
                };
                let operand = match (shift_type, shift) {
                    (0, bits) => source.wrapping_shl(bits),
                    (1, 0) => 0,
                    (1, bits) => source >> bits,
                    (2, 0) => (source as i32 >> 31) as u32,
                    (2, bits) => (source as i32 >> bits) as u32,
                    (3, 0) => {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        }
                    }
                    (3, bits) => source.rotate_right(bits),
                    _ => unreachable!(),
                };
                let left = if rn == 15 {
                    pc.wrapping_add(8)
                } else {
                    self.registers[rn]
                };
                match opcode {
                    13 => {
                        // MOV/MOVS
                        let old = if set_flags {
                            let Some(old) = self.flags else {
                                return RunReport {
                                    executed,
                                    stop: StopReason::UnknownConditionFlags,
                                };
                            };
                            Some(old)
                        } else {
                            None
                        };
                        self.registers[rd] = operand;
                        if let Some(old) = old {
                            let carry = match (shift_type, shift) {
                                (0, 0) => old.c,
                                (0, bits) => source & (1u32 << (32 - bits)) != 0,
                                (1 | 2, 0) => source >> 31 != 0,
                                (1 | 2, bits) => source & (1u32 << (bits - 1)) != 0,
                                (3, bits) => source.rotate_right(bits) >> 31 != 0,
                                _ => unreachable!(),
                            };
                            self.flags = Some(Flags {
                                n: operand >> 31 != 0,
                                z: operand == 0,
                                c: carry,
                                v: old.v,
                            });
                        }
                    }
                    4 | 2 | 10 => {
                        // ADD, SUB, CMP
                        if opcode == 10 && !set_flags {
                            return RunReport {
                                executed,
                                stop: StopReason::UnsupportedInstruction,
                            };
                        }
                        let (value, carry, overflow) = if opcode == 4 {
                            let (value, carry) = left.overflowing_add(operand);
                            (
                                value,
                                carry,
                                (left ^ value) & (operand ^ value) & 0x8000_0000 != 0,
                            )
                        } else {
                            let (value, borrow) = left.overflowing_sub(operand);
                            (
                                value,
                                !borrow,
                                (left ^ operand) & (left ^ value) & 0x8000_0000 != 0,
                            )
                        };
                        if opcode != 10 {
                            self.registers[rd] = value;
                        }
                        if set_flags {
                            self.flags = Some(Flags {
                                n: value >> 31 != 0,
                                z: value == 0,
                                c: carry,
                                v: overflow,
                            });
                        }
                    }
                    _ => {
                        return RunReport {
                            executed,
                            stop: StopReason::UnsupportedInstruction,
                        }
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                continue;
            }
            return RunReport {
                executed,
                stop: StopReason::UnsupportedInstruction,
            };
        }
        RunReport {
            executed: max_steps,
            stop: StopReason::StepLimit,
        }
    }
}

fn blx_immediate_target(pc: u32, word: u32) -> u32 {
    let displacement = (((word & 0x00ff_ffff) << 2) as i32) << 6 >> 6;
    let halfword_bit = ((word >> 24) & 1) * 2;
    pc.wrapping_add(8)
        .wrapping_add_signed(displacement)
        .wrapping_add(halfword_bit)
}

fn thumb_instruction_family(halfword: u16) -> &'static str {
    match halfword {
        0xbf00 => "T16 NOP",
        value if value & 0xff87 == 0x4700 => "T16 BX register",
        value if value & 0xfe00 == 0xb400 => "T16 PUSH",
        value if value & 0xfe00 == 0xbc00 => "T16 POP",
        value if value & 0xf800 == 0x2000 => "T16 MOV immediate",
        value if value & 0xf800 == 0x4800 => "T16 LDR literal",
        value if value & 0xfc00 == 0x4400 => match (value >> 8) & 3 {
            0 => "T16 ADD high-register",
            1 => "T16 CMP high-register",
            2 => "T16 MOV high-register",
            _ => "T16 BLX register or reserved branch exchange",
        },
        value if value & 0xfc00 == 0x4000 => match (value >> 6) & 15 {
            0 => "T16 AND register",
            1 => "T16 EOR register",
            2 => "T16 LSL register",
            3 => "T16 LSR register",
            4 => "T16 ASR register",
            5 => "T16 ADC register",
            6 => "T16 SBC register",
            7 => "T16 ROR register",
            8 => "T16 TST register",
            9 => "T16 RSB register",
            10 => "T16 CMP register",
            11 => "T16 CMN register",
            12 => "T16 ORR register",
            13 => "T16 MUL register",
            14 => "T16 BIC register",
            _ => "T16 MVN register",
        },
        value if value & 0xe000 == 0x0000 => "T16 shift or add/subtract",
        value if value & 0xe000 == 0x2000 => "T16 immediate data processing",
        value if value & 0xf000 == 0x5000 => "T16 register-offset load/store",
        value if value & 0xe000 == 0x6000 => "T16 immediate load/store",
        value if value & 0xf000 == 0x8000 => "T16 halfword or SP-relative load/store",
        value if value & 0xf000 == 0xa000 => "T16 PC/SP-relative address",
        value if value & 0xf000 == 0xb000 => "T16 miscellaneous",
        value if value & 0xf000 == 0xc000 => "T16 multiple load/store",
        value if value & 0xf000 == 0xd000 => "T16 conditional branch or SVC",
        value if value & 0xf800 == 0xe000 => "T16 B unconditional",
        value if value & 0xf800 >= 0xe800 => "T32 instruction prefix",
        _ => "other T16 instruction",
    }
}

fn condition_passes(cond: u32, f: Flags) -> bool {
    match cond {
        0 => f.z,
        1 => !f.z,
        2 => f.c,
        3 => !f.c,
        4 => f.n,
        5 => !f.n,
        6 => f.v,
        7 => !f.v,
        8 => f.c && !f.z,
        9 => !f.c || f.z,
        10 => f.n == f.v,
        11 => f.n != f.v,
        12 => !f.z && f.n == f.v,
        13 => f.z || f.n != f.v,
        14 => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeSegment, NcchExecutableLayout};

    fn fixture(words: &[u32]) -> ProcessImage {
        let layout = NcchExecutableLayout {
            text: CodeSegment {
                address: 0x1000,
                pages: 1,
                byte_size: 0x1000,
            },
            read_only: CodeSegment {
                address: 0x2000,
                pages: 0,
                byte_size: 0,
            },
            data: CodeSegment {
                address: 0x3000,
                pages: 0,
                byte_size: 0,
            },
            bss_size: 0,
            stack_size: 0x1000,
            compressed: false,
        };
        let mut code = vec![0; 0x1000];
        for (index, word) in words.iter().enumerate() {
            code[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        ProcessImage::from_parts(layout, code).unwrap()
    }

    #[test]
    fn moves_branches_and_stops_at_supervisor_call() {
        // MOV r0,#1; B over MOV r0,#2; MOV r0,#3; ADD r0,r0,#4; SVC #1.
        let mut image = fixture(&[
            0xe3a0_0001,
            0xea00_0000,
            0xe3a0_0002,
            0xe280_0004,
            0xef00_0001,
        ]);
        let mut cpu = ArmProbe::new(&image);
        let report = cpu.run(&mut image, 20);
        assert_eq!(
            report,
            RunReport {
                executed: 3,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.pending_supervisor_call(&image), Some(1));
        assert_eq!(cpu.register(0), Some(5));
        assert_eq!(cpu.register(13), Some(0x1000_0000));
    }

    #[test]
    fn and_immediate_masks_without_changing_flags() {
        // MOV r0,#0xff; AND r1,r0,#0x0f; SVC #0.
        let mut image = fixture(&[0xe3a0_00ff, 0xe200_100f, 0xef00_0000]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 2,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(1), Some(0x0f));
        let mut image = fixture(&[0xe3a0_00ff, 0xe210_100f]); // Old V/C flags unavailable.
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 1,
                stop: StopReason::UnknownConditionFlags
            }
        );
        assert_eq!(cpu.register(1), Some(0));
        // CMP leaves C clear; rotated immediate sets C; zero result sets Z.
        let mut image = fixture(&[
            0xe3a0_0000,
            0xe350_0001,
            0xe210_1480,
            0x23a0_2007,
            0x03a0_3009,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 5,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(1), Some(0));
        assert_eq!(cpu.register(2), Some(7));
        assert_eq!(cpu.register(3), Some(9));
    }

    #[test]
    fn teq_immediate_sets_logical_flags_without_destination_write() {
        // MOV r0,#0; CMP r0,#1; TEQ r0,#0x80000000; MOVCS r1,#7; MOVMI r2,#9.
        let mut image = fixture(&[
            0xe3a0_0000,
            0xe350_0001,
            0xe330_0480,
            0x23a0_1007,
            0x43a0_2009,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 5,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(0), Some(0));
        assert_eq!(cpu.register(1), Some(7));
        assert_eq!(cpu.register(2), Some(9));
    }

    #[test]
    fn msr_control_immediate_stops_before_modifying_cpu_state() {
        // The MSR form occupies a TEQ-without-S encoding; it is not a TEQ.
        let mut image = fixture(&[0xe321_f080, 0xef00_0000]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            instruction_family(0xe321_f080),
            "MSR CPSR control/status immediate"
        );
        assert_eq!(
            cpu.run(&mut image, 3),
            RunReport {
                executed: 0,
                stop: StopReason::UnsupportedInstruction
            }
        );
        assert_eq!(cpu.register(15), Some(0x1000));
    }

    #[test]
    fn only_exact_nop_hint_advances_without_changing_state() {
        let mut image = fixture(&[0xe320_f000, 0xe3a0_0007, 0xef00_0000]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 2,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(0), Some(7));

        let mut image = fixture(&[0xe320_f001]); // YIELD needs scheduling semantics.
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 1),
            RunReport {
                executed: 0,
                stop: StopReason::UnsupportedInstruction
            }
        );
    }

    #[test]
    fn main_thread_tls_pointer_reads_and_writes_private_slot() {
        // MRC p15,0,r0,c13,c0,3; MOV r1,#7; STR r1,[r0]; LDR r2,[r0]; SVC #0.
        let mut image = fixture(&[
            0xee1d_0f70,
            0xe3a0_1007,
            0xe580_1000,
            0xe590_2000,
            0xef00_0000,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 8),
            RunReport {
                executed: 4,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(0), Some(0x1ff8_2000));
        assert_eq!(cpu.register(2), Some(7));
        let mut value = [0u8; 4];
        image.read(0x1ff8_2000, &mut value).unwrap();
        assert_eq!(u32::from_le_bytes(value), 7);
        assert!(image.read(0x1ff8_21ff, &mut value).is_err());
    }

    #[test]
    fn mvn_immediate_inverts_operand_without_flags() {
        // MVN r0,#0xff; SVC #0.
        let mut image = fixture(&[0xe3e0_00ff, 0xef00_0000]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 1,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(0), Some(0xffff_ff00));
    }

    #[test]
    fn exclusive_word_pair_updates_private_memory_and_reports_failure_without_reservation() {
        // SUB r0,sp,#4; MOV r3,#7; STR r3,[r0]; LDREX r1,[r0];
        // ADD r1,r1,#1; STREX r2,r1,[r0]; LDR r3,[r0]; SVC #0.
        let mut image = fixture(&[
            0xe24d_0004,
            0xe3a0_3007,
            0xe580_3000,
            0xe190_1f9f,
            0xe281_1001,
            0xe180_2f91,
            0xe590_3000,
            0xef00_0000,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 10),
            RunReport {
                executed: 7,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(2), Some(0));
        assert_eq!(cpu.register(3), Some(8));

        // With no LDREX, STREX leaves the value unchanged and returns failure.
        let mut image = fixture(&[0xe24d_0004, 0xe3a0_1009, 0xe180_2f91, 0xe590_3000]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 4,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(2), Some(1));
        assert_eq!(cpu.register(3), Some(0));
    }

    #[test]
    fn fpscr_status_register_round_trip_without_vfp_arithmetic() {
        // MOV r0,#7; VMSR FPSCR,r0; VMRS r1,FPSCR; SVC #0.
        let mut image = fixture(&[0xe3a0_0007, 0xeee1_0a10, 0xeef1_1a10, 0xef00_0000]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 3,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(1), Some(7));
    }

    #[test]
    fn movs_shifted_register_updates_result_and_carry() {
        // LDR r0,[pc,#8]; CMP r0,#0; MOVS r1,r0,LSL #1; MOVCS r2,#7; literal.
        let mut image = fixture(&[
            0xe59f_0008,
            0xe350_0000,
            0xe1b0_1080,
            0x23a0_2007,
            0x8000_0000,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 4,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(1), Some(0));
        assert_eq!(cpu.register(2), Some(7));
        let mut image = fixture(&[0xe1b0_1080]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 1).stop,
            StopReason::UnknownConditionFlags
        );
        assert_eq!(cpu.register(1), Some(0));
    }

    #[test]
    fn stmia_stores_consecutive_registers_and_advances_base() {
        // MOV r0,#1; MOV r1,#2; SUB r2,sp,#8; STMIA r2!,{r0,r1}.
        let mut image = fixture(&[0xe3a0_0001, 0xe3a0_1002, 0xe24d_2008, 0xe8a2_0003]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 4,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(2), Some(0x1000_0000));
        let mut values = [0u8; 8];
        image.read(0x0fff_fff8, &mut values).unwrap();
        assert_eq!(u32::from_le_bytes(values[..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(values[4..].try_into().unwrap()), 2);
    }

    #[test]
    fn conditions_require_known_flags_and_step_limit_is_bounded() {
        let mut image = fixture(&[0x03a0_0001]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5).stop,
            StopReason::UnknownConditionFlags
        );
        let mut image = fixture(&[0xe350_0000, 0x03a0_0007, 0xeaff_fffe]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 3,
                stop: StopReason::DeterministicLoop
            }
        );
        assert_eq!(cpu.register(0), Some(7));
    }

    #[test]
    fn branch_exchange_to_unmapped_address_faults() {
        let mut image = fixture(&[0xe12f_ff1e]); // BX LR with an unmapped target
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 1,
                stop: StopReason::InstructionFetchFault
            }
        );
        assert_eq!(cpu.register(15), Some(0));
    }

    #[test]
    fn loads_literals_and_stores_only_to_writable_memory() {
        // LDR r0,[pc,#4]; STR r0,[sp,#-4]; LDR r1,[sp,#-4]; literal.
        let mut image = fixture(&[0xe59f_0004, 0xe50d_0004, 0xe51d_1004, 0x1234_5678]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 3),
            RunReport {
                executed: 3,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(1), Some(0x1234_5678));
        let mut bytes = [0; 4];
        image.read(0x0fff_fffc, &mut bytes).unwrap();
        assert_eq!(u32::from_le_bytes(bytes), 0x1234_5678);
        let mut image = fixture(&[0xe58f_0000]); // STR r0,[pc] targets read-only text.
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(cpu.run(&mut image, 1).stop, StopReason::MemoryFault);
    }

    #[test]
    fn register_comparison_sets_flags_for_conditional_execution() {
        // MOV r0,#3; MOV r1,#3; CMP r0,r1; MOVEQ r2,#7; SVC #0.
        let mut image = fixture(&[
            0xe3a0_0003,
            0xe3a0_1003,
            0xe150_0001,
            0x03a0_2007,
            0xef00_0000,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 8),
            RunReport {
                executed: 4,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(2), Some(7));
    }

    #[test]
    fn post_indexed_store_updates_base_after_access() {
        // SUB r1,sp,#4; MOV r0,#7; STR r0,[r1],#4; LDR r2,[r1,#-4].
        let mut image = fixture(&[0xe24d_1004, 0xe3a0_0007, 0xe481_0004, 0xe511_2004]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 4,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(1), Some(0x1000_0000));
        assert_eq!(cpu.register(2), Some(7));
    }

    #[test]
    fn branch_exchange_enters_thumb_and_stops_on_unsupported_instruction() {
        // MOV r0,#0x1000 is not representable as one A32 immediate, so load a literal.
        let mut image = fixture(&[0xe59f_0004, 0xe12f_ff10, 0xef00_0000, 0x0000_1001]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 2,
                stop: StopReason::UnsupportedInstruction
            }
        );
        assert!(cpu.thumb);
        assert_eq!(cpu.register(15), Some(0x1000));
    }

    #[test]
    fn blx_immediate_enters_thumb_and_returns_to_a32() {
        // BLX to 0x1008; T16 NOP; BX LR; A32 SVC at return address.
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0x4770_bf00]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(instruction_family(0xfa00_0000), "BLX immediate");
        assert_eq!(cpu.pending_blx_thumb_entry_readable(&image), Some(true));
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 3,
                stop: StopReason::SupervisorCall
            }
        );
        assert!(!cpu.thumb);
        assert_eq!(cpu.register(15), Some(0x1004));
        assert_eq!(cpu.register(14), Some(0x1004));

        let image = fixture(&[0xfaff_fffd]);
        let cpu = ArmProbe::new(&image);
        assert_eq!(cpu.pending_blx_thumb_entry_readable(&image), Some(false));
    }

    #[test]
    fn thumb_push_and_pop_restore_registers_and_return_to_a32() {
        // A32 BLX; T16 PUSH {r0,lr}; POP {r0,pc}; A32 SVC.
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0xbd01_b501]);
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[0] = 0x1234_5678;
        let initial_sp = cpu.registers[13];
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 3,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(0), Some(0x1234_5678));
        assert_eq!(cpu.register(13), Some(initial_sp));
        assert_eq!(cpu.register(15), Some(0x1004));
        assert!(!cpu.thumb);
    }

    #[test]
    fn thumb_literal_load_uses_word_aligned_pc() {
        // BLX reaches NOP at 0x1008, then LDR r0,[PC,#0] at 0x100a.
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0x4800_bf00, 0x1234_5678]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 3),
            RunReport {
                executed: 3,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(0), Some(0x1234_5678));
    }

    #[test]
    fn thumb_high_register_add_keeps_flags_and_state() {
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0xbf00_4480]);
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[0] = 3;
        cpu.registers[8] = 5;
        assert_eq!(
            cpu.run(&mut image, 2),
            RunReport {
                executed: 2,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(8), Some(8));
        assert!(cpu.thumb);
        assert_eq!(cpu.register(15), Some(0x100a));
    }

    #[test]
    fn thumb_short_branch_uses_pc_plus_four() {
        // BLX to B at 0x1008, branch over one halfword to NOP and BX LR.
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0xffff_e000, 0x4770_bf00]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 5),
            RunReport {
                executed: 4,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(15), Some(0x1004));
    }

    #[test]
    fn thumb_register_compare_sets_condition_flags() {
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0xbf00_4288]);
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[0] = 5;
        cpu.registers[1] = 7;
        assert_eq!(
            cpu.run(&mut image, 2),
            RunReport {
                executed: 2,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(
            cpu.flags.map(|f| (f.n, f.z, f.c, f.v)),
            Some((true, false, false, false))
        );
    }

    #[test]
    fn thumb_conditional_branch_takes_or_skips_based_on_flags() {
        // BLX; CMP r0,r1; BNE over NOP; BX LR; A32 SVC on return.
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0xd100_4288, 0x4770_bf00]);
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[0] = 5;
        cpu.registers[1] = 7;
        assert_eq!(
            cpu.run(&mut image, 6),
            RunReport {
                executed: 4,
                stop: StopReason::SupervisorCall
            }
        );
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[0] = 7;
        cpu.registers[1] = 7;
        assert_eq!(
            cpu.run(&mut image, 6),
            RunReport {
                executed: 5,
                stop: StopReason::SupervisorCall
            }
        );
    }

    #[test]
    fn thumb_immediate_word_store_and_load_use_private_memory() {
        // BLX; STR r0,[r1,#0]; LDR r2,[r1,#0].
        let mut image = fixture(&[0xfa00_0000, 0xef00_0000, 0x680a_6008]);
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[0] = 0x1234_5678;
        cpu.registers[1] = cpu.registers[13] - 4;
        assert_eq!(
            cpu.run(&mut image, 3),
            RunReport {
                executed: 3,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(2), Some(0x1234_5678));
        assert_eq!(cpu.memory_writes(), 1);
    }

    #[test]
    fn stack_block_transfer_round_trip() {
        // MOV r0,#1; MOV r1,#2; STMDB sp!,{r0,r1}; clear; LDMIA sp!,{r0,r1}.
        let mut image = fixture(&[
            0xe3a0_0001,
            0xe3a0_1002,
            0xe92d_0003,
            0xe3a0_0000,
            0xe3a0_1000,
            0xe8bd_0003,
        ]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 6),
            RunReport {
                executed: 6,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(0), Some(1));
        assert_eq!(cpu.register(1), Some(2));
        assert_eq!(cpu.register(13), Some(0x1000_0000));
    }

    #[test]
    fn block_load_of_pc_branches_or_stops_at_thumb() {
        let mut image = fixture(&[0xe8bd_8000, 0, 0, 0, 0xef00_0000]);
        image.write(0x0fff_fffc, &0x1010u32.to_le_bytes()).unwrap();
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[13] = 0x0fff_fffc;
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 1,
                stop: StopReason::SupervisorCall
            }
        );
        assert_eq!(cpu.register(13), Some(0x1000_0000));
        image.write(0x0fff_fffc, &0x1011u32.to_le_bytes()).unwrap();
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[13] = 0x0fff_fffc;
        assert_eq!(
            cpu.run(&mut image, 4),
            RunReport {
                executed: 0,
                stop: StopReason::ThumbStateRequired
            }
        );
    }

    #[test]
    fn preindexed_writeback_and_postindexed_load_restore_stack() {
        // MOV r0,#7; STR r0,[sp,#-4]!; LDR r1,[sp],#4.
        let mut image = fixture(&[0xe3a0_0007, 0xe52d_0004, 0xe49d_1004]);
        let mut cpu = ArmProbe::new(&image);
        assert_eq!(
            cpu.run(&mut image, 3),
            RunReport {
                executed: 3,
                stop: StopReason::StepLimit
            }
        );
        assert_eq!(cpu.register(1), Some(7));
        assert_eq!(cpu.register(13), Some(0x1000_0000));
    }

    #[test]
    fn kernel_probe_creates_only_allowed_in_memory_arbiters() {
        let mut image = fixture(&[0xef00_0021, 0xef00_0021, 0xef00_0022]);
        let mut allowed = [false; 192];
        allowed[0x21] = true;
        allowed[0x22] = true;
        image.svc_mask = Some(allowed);
        let mut cpu = ArmProbe::new(&image);
        let mut kernel = KernelProbe::new();
        let report = cpu.run_with_kernel(&mut image, &mut kernel, 10);
        assert_eq!(
            report,
            KernelRunReport {
                executed: 2,
                serviced: 2,
                stop: KernelStop::ServiceUnavailable
            }
        );
        assert_eq!(kernel.address_arbiter_count(), 2);
        assert_eq!(cpu.register(0), Some(0));
        assert_eq!(cpu.register(1), Some(0x101));
        let mut denied = fixture(&[0xef00_0021]);
        denied.svc_mask = Some([false; 192]);
        let mut cpu = ArmProbe::new(&denied);
        let mut kernel = KernelProbe::new();
        assert_eq!(
            cpu.run_with_kernel(&mut denied, &mut kernel, 10).stop,
            KernelStop::ServiceNotAllowed
        );
        assert_eq!(kernel.address_arbiter_count(), 0);
    }

    #[test]
    fn kernel_probe_returns_invalid_address_for_null_free_only() {
        let mut image = fixture(&[0xe3a0_0001, 0xef00_0001, 0xef00_0022]);
        let mut allowed = [false; 192];
        allowed[1] = true;
        allowed[0x22] = true;
        image.svc_mask = Some(allowed);
        let mut cpu = ArmProbe::new(&image);
        let mut kernel = KernelProbe::new();
        assert_eq!(
            cpu.run_with_kernel(&mut image, &mut kernel, 8),
            KernelRunReport {
                executed: 2,
                serviced: 1,
                stop: KernelStop::ServiceUnavailable
            }
        );
        assert_eq!(cpu.register(0), Some(0xe0e0_1bf5));
        assert_eq!(cpu.pending_supervisor_call(&image), Some(0x22));

        let mut unsupported = fixture(&[0xe3a0_0003, 0xef00_0001]);
        unsupported.svc_mask = Some(allowed);
        let mut cpu = ArmProbe::new(&unsupported);
        assert_eq!(
            cpu.run_with_kernel(&mut unsupported, &mut kernel, 8),
            KernelRunReport {
                executed: 1,
                serviced: 0,
                stop: KernelStop::ServiceUnavailable
            }
        );
        assert_eq!(cpu.register(0), Some(3));
    }

    #[test]
    fn kernel_probe_accepts_zero_size_heap_free_without_changing_memory() {
        let mut image = fixture(&[0xe3a0_0001, 0xef00_0001, 0xef00_0022]);
        let mut allowed = [false; 192];
        allowed[1] = true;
        allowed[0x22] = true;
        image.svc_mask = Some(allowed);
        let mut cpu = ArmProbe::new(&image);
        cpu.registers[1] = 0x0800_0000;
        let mut kernel = KernelProbe::new();
        assert_eq!(
            cpu.run_with_kernel(&mut image, &mut kernel, 8),
            KernelRunReport {
                executed: 2,
                serviced: 1,
                stop: KernelStop::ServiceUnavailable
            }
        );
        assert_eq!(cpu.register(0), Some(0));
        assert_eq!(cpu.register(1), Some(0x0800_0000));
        assert_eq!(cpu.memory_writes(), 0);

        let mut other_range = fixture(&[0xe3a0_0001, 0xef00_0001]);
        other_range.svc_mask = Some(allowed);
        let mut cpu = ArmProbe::new(&other_range);
        cpu.registers[1] = 0x1400_0000;
        assert_eq!(
            cpu.run_with_kernel(&mut other_range, &mut kernel, 8),
            KernelRunReport {
                executed: 1,
                serviced: 0,
                stop: KernelStop::ServiceUnavailable
            }
        );
    }
}
