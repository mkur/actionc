//! Definite overwrites at entry, independent of the callee's destination-write
//! footprint. In particular, an indirect copy loop need not have bounded data
//! addresses to prove which incoming ABI/scratch bytes it observes.
use super::super::entry_inputs::{MirEntryRequirements, program_state_access};
use super::*;

#[derive(Clone, Default, PartialEq, Eq)]
struct Defined {
    registers: u8,
    flags: u8, // C, N, Z, V; a load defines only N/Z.
    bytes: BTreeSet<u8>,
}
impl Defined {
    fn meet(&mut self, other: &Self) -> bool {
        let old = self.clone();
        self.registers &= other.registers;
        self.flags &= other.flags;
        self.bytes.retain(|b| other.bytes.contains(b));
        *self != old
    }
}

impl BoundedMachine {
    pub(in crate::mir6502) fn entry_inputs(&self, program: &MirProgram) -> Option<MirEntryRequirements> {
        let mut entries = BTreeMap::from([(0usize, Defined::default())]);
        let mut pending = VecDeque::from([0usize]);
        let mut inputs = MirEntryRequirements::default();
        let mut register_inputs = 0u8;
        let mut returned = false;
        let mut budget = MAX_STEPS;
        while let Some(pc) = pending.pop_front() {
            budget = budget.checked_sub(1)?;
            let mut state = entries.get(&pc)?.clone();
            let opcode = self.bytes.get(pc)?.value?;
            let (name, mode, len) = crate::codegen::decode_6502_opcode(opcode)?;
            let next = pc.checked_add(len)?;
            let operands = self.bytes.get(pc + 1..next)?;
            let access = |bytes: u16| {
                // MachineByte retains typed relocation identity independently
                // of its numeric value. Classify that identity before resolving
                // the address; literal addresses never inherit a global's role.
                let mem = machine_operand_mem(operands)?;
                let state = program_state_access(program, &mem, bytes)?;
                let address = fixed_storage_address(program, &mem);
                let zp = matches!(mode, AddressingMode::ZeroPage | AddressingMode::IndirectIndexedY);
                if !state || zp {
                    address?;
                }
                if let Some(address) = address {
                    let last = address.checked_add(bytes.checked_sub(1)?)?;
                    if zp && last >= 256 { return None; }
                }
                Some((state, address))
            };
            let mut reads = 0u8;
            let mut writes = 0u8;
            let mut flag_reads = 0u8;
            let mut flag_writes = 0u8;
            let mut memory_reads = Vec::new();
            let mut memory_writes = Vec::new();
            let mut successors = vec![next];
            match (name, mode) {
                ("RTS", _) if next == self.bytes.len() => {
                    returned = true;
                    continue;
                }
                (
                    "LDA" | "LDX" | "LDY",
                    AddressingMode::Immediate | AddressingMode::ZeroPage | AddressingMode::Absolute,
                ) => {
                    writes = match name {
                        "LDX" => 2,
                        "LDY" => 4,
                        _ => 1,
                    };
                    flag_writes = 6;
                    if mode != AddressingMode::Immediate {
                        let (state, address) = access(1)?;
                        if !state && let Some(a) = address && a < 256 {
                            memory_reads.push(a as u8);
                        }
                    }
                }
                ("STA" | "STX" | "STY", AddressingMode::ZeroPage | AddressingMode::Absolute) => {
                    reads = match name {
                        "STX" => 2,
                        "STY" => 4,
                        _ => 1,
                    };
                    let (_, address) = access(1)?;
                    if let Some(a) = address && a < 256 {
                        memory_writes.push(a as u8);
                    }
                }
                ("LDA" | "STA", AddressingMode::IndirectIndexedY) => {
                    let (state, address) = access(2)?;
                    let low = u8::try_from(address?).ok()?;
                    if !state {
                        memory_reads.extend([low, low.checked_add(1)?]);
                    }
                    reads = 4;
                    if name == "LDA" {
                        writes = 1;
                        flag_writes = 6;
                    } else {
                        reads |= 1;
                    }
                    // A possible indirect write is not a definite definition of
                    // any fixed byte. It also cannot resurrect an overwritten
                    // entry definition; ordinary pointers exclude private ZP.
                }
                ("INX" | "DEX" | "INY" | "DEY", _) => {
                    reads = if name.ends_with('X') { 2 } else { 4 };
                    writes = reads;
                    flag_writes = 6;
                }
                ("TAX" | "TAY" | "TXA" | "TYA", _) => {
                    (reads, writes) = match name {
                        "TAX" => (1, 2),
                        "TAY" => (1, 4),
                        "TXA" => (2, 1),
                        _ => (4, 1),
                    };
                    flag_writes = 6;
                }
                ("CMP" | "CPX" | "CPY", AddressingMode::Immediate) => {
                    reads = match name {
                        "CPX" => 2,
                        "CPY" => 4,
                        _ => 1,
                    };
                    flag_writes = 7;
                }
                ("CLC" | "SEC", _) => flag_writes = 1,
                ("CLV", _) => flag_writes = 8,
                (
                    "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ",
                    AddressingMode::Relative,
                ) => {
                    flag_reads = match name {
                        "BCC" | "BCS" => 1,
                        "BPL" | "BMI" => 2,
                        "BNE" | "BEQ" => 4,
                        _ => 8,
                    };
                    let delta = i8::from_ne_bytes([operands.first()?.value?]);
                    let target =
                        usize::try_from(isize::try_from(next).ok()? + isize::from(delta)).ok()?;
                    successors.push(target);
                }
                ("NOP", _) => {}
                // Calls, stack manipulation, unknown instructions and open
                // control boundaries do not have an entry certificate here.
                _ => return None,
            }
            register_inputs |= reads & !state.registers;
            let observed = flag_reads & !state.flags;
            inputs.flags.c |= observed & 1 != 0;
            inputs.flags.n |= observed & 2 != 0;
            inputs.flags.z |= observed & 4 != 0;
            inputs.flags.v |= observed & 8 != 0;
            inputs.fixed.extend(
                memory_reads
                    .into_iter()
                    .filter(|b| !state.bytes.contains(b)),
            );
            state.registers |= writes;
            state.flags |= flag_writes;
            state.bytes.extend(memory_writes);
            for successor in successors {
                if !self.starts.contains(&successor) {
                    return None;
                }
                if let Some(old) = entries.get_mut(&successor) {
                    if old.meet(&state) {
                        pending.push_back(successor);
                    }
                } else {
                    entries.insert(successor, state.clone());
                    pending.push_back(successor);
                }
            }
        }
        inputs.registers = MirRegisterSet {
            a: register_inputs & 1 != 0,
            x: register_inputs & 2 != 0,
            y: register_inputs & 4 != 0,
            ..Default::default()
        };
        returned.then_some(inputs)
    }
}
