//! Preallocation resource windows, checked against authoritative typed effects.
//! A window includes address setup and the final access, not just its last opcode.
use super::{effects::*, selected::*, state::Environment, *};
use crate::mir65816::analysis::ProgramPoint;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Scratch(u64);
impl Scratch {
    pub fn range(offset: u16, bytes: u16) -> Option<Self> {
        let start = offset.checked_sub(abi::generated::DP_SCRATCH_OFFSET as u16)?;
        if bytes == 0 || u32::from(start) + u32::from(bytes) > 64 {
            return None;
        }
        Some(Self(if bytes == 64 {
            u64::MAX
        } else {
            ((1u64 << bytes) - 1) << start
        }))
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn contains(self, other: Self) -> bool {
        other.0 & !self.0 == 0
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

// One definition of the selector workspaces. Residence uses the same ABI block;
// it does not acquire extra bank-zero bytes.
pub(super) const PTR: u8 = abi::generated::DP_POINTER0_OFFSET as u8;
pub(super) const RESULT: u8 = abi::generated::DP_SCRATCH_OFFSET as u8 + 8;
pub(super) const RIGHT: u8 = abi::generated::DP_SCRATCH_OFFSET as u8 + 16;
pub(super) const INDEX: u8 = abi::generated::DP_SCRATCH_OFFSET as u8 + 20;
pub(super) const COPY_SOURCE: u8 = abi::generated::DP_POINTER1_OFFSET as u8;
pub(super) const COPY_DEST: u8 = abi::generated::DP_SCRATCH_OFFSET as u8 + 24;
pub(super) const COPY_COUNT: u8 = abi::generated::DP_SCRATCH_OFFSET as u8 + 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Form {
    RecordMemory,
    Scalar,
    Address,
    /// No residence may be extended through an unqualified operation.
    Barrier,
    Boundary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Requirements {
    pub form: Form,
    pub reads: Registers,
    pub writes: Registers,
    pub clobbers: Registers,
    pub flag_reads: u8,
    pub flag_writes: u8,
    pub flag_clobbers: u8,
    pub environment_writes: u16,
    /// Complete conservative workspace extent, never a promise of defined data.
    pub scratch: Scratch,
    pub external_reads: Option<u32>,
    pub external_writes: Option<u32>,
    pub extra_stack: u32,
}

fn external(a: &Mir65816Address) -> bool {
    !matches!(
        a.base,
        Mir65816AddressBase::AutomaticFrame(_) | Mir65816AddressBase::Parameter(_)
    )
}

impl Requirements {
    fn new(form: Form) -> Self {
        let barrier = matches!(form, Form::Barrier | Form::Boundary);
        Self {
            form,
            // These are closed-operation upper bounds. In particular X may be
            // used for a complete pointer reload or a wide ABI result.
            reads: Registers::ALL,
            writes: Registers::ALL,
            clobbers: if barrier {
                Registers::ALL
            } else {
                Registers::default()
            },
            flag_reads: NZCV,
            flag_writes: NZCV,
            flag_clobbers: if barrier { NZCV } else { 0 },
            environment_writes: if barrier {
                env::ALL
            } else {
                env::M | env::X | env::PC | env::PBR
            },
            scratch: if barrier {
                Scratch::range(
                    abi::generated::DP_SCRATCH_OFFSET as u16,
                    abi::generated::DP_SCRATCH_SIZE as u16,
                )
                .unwrap()
            } else {
                // Ordinary selector workspaces end at COPY_COUNT+3. The
                // separate scalar residence pool starts at scratch+32.
                Scratch::range(abi::generated::DP_SCRATCH_OFFSET as u16, 31).unwrap()
            },
            external_reads: (!barrier).then_some(0),
            external_writes: (!barrier).then_some(0),
            extra_stack: 0,
        }
    }
    fn operation(op: &Mir65816Op) -> Self {
        let form = match op {
            Mir65816Op::Load {
                width,
                address,
                volatile: false,
                ..
            }
            | Mir65816Op::Store {
                width,
                address,
                volatile: false,
                ..
            } if (1..=4).contains(&width.get()) && address.index.is_none() => Form::RecordMemory,
            Mir65816Op::AddressOf { address, .. } if address.index.is_none() => Form::Address,
            Mir65816Op::PointerOffset {
                offset_signed: false,
                ..
            } => Form::Address,
            Mir65816Op::Binary { operation, .. }
                if matches!(
                    operation,
                    NirBinaryOp::Add
                        | NirBinaryOp::Sub
                        | NirBinaryOp::And
                        | NirBinaryOp::Or
                        | NirBinaryOp::Xor
                ) =>
            {
                Form::Scalar
            }
            Mir65816Op::Compare { .. } | Mir65816Op::Cast { .. } => Form::Scalar,
            _ => Form::Barrier,
        };
        let mut this = Self::new(form);
        match op {
            Mir65816Op::Load { width, address, .. }
                if external(address) && form != Form::Barrier =>
            {
                this.external_reads = Some(width.get())
            }
            Mir65816Op::Store { width, address, .. }
                if external(address) && form != Form::Barrier =>
            {
                this.external_writes = Some(width.get())
            }
            Mir65816Op::Call { plan, .. } => {
                this.extra_stack = plan.outgoing_bytes.get()
                    + plan.native.map_or(6, |n| n.transfer.peak_bytes().get());
            }
            _ => (),
        }
        this
    }
}

/// Immutable descriptions are chosen from MIR before any home allocation.
pub(super) struct Plan {
    pub windows: BTreeMap<ProgramPoint, Requirements>,
}
impl Plan {
    pub fn new(r: &Mir65816Routine) -> Self {
        Self {
            windows: r
                .blocks
                .iter()
                .flat_map(|b| {
                    b.ops
                        .iter()
                        .enumerate()
                        .map(|(index, op)| {
                            (
                                ProgramPoint { block: b.id, index },
                                Requirements::operation(op),
                            )
                        })
                        .chain([(
                            ProgramPoint {
                                block: b.id,
                                index: b.ops.len(),
                            },
                            Requirements::new(Form::Boundary),
                        )])
                })
                .collect(),
        }
    }
}

/// The same typed instruction definition supplies replay effects and resource
/// observations. Stored selector claims cannot expand these requirements.
pub(super) fn check_instruction(
    requirements: &Requirements,
    form: &Instruction,
    state: Environment,
    stored: &InstructionEffects,
    dp_homes: Scratch,
) -> Result<(u32, u32), String> {
    let e = form.effects(state);
    if &e != stored {
        return Err("placement resource effects differ from typed instruction".into());
    }
    let within = |actual: Registers, allowed: Registers| {
        actual.a & !allowed.a == 0 && actual.x & !allowed.x == 0 && actual.y & !allowed.y == 0
    };
    if !within(e.reads, requirements.reads)
        || !within(e.writes, requirements.writes)
        || !within(e.clobbers, requirements.clobbers)
        || e.flag_reads & !requirements.flag_reads != 0
        || e.flag_writes & !requirements.flag_writes != 0
        || e.flag_clobbers & !requirements.flag_clobbers != 0
        || e.environment_writes & !requirements.environment_writes != 0
    {
        return Err("selected instruction exceeds operation register/state resources".into());
    }
    let mut reads = 0;
    let mut writes = 0;
    for effect in &e.memory {
        let bytes = match effect.memory {
            Memory::DirectPage { offset, bytes } => {
                let end = offset
                    .checked_add(bytes)
                    .ok_or("DP resource range overflow")?;
                let scratch = Scratch::range(offset, bytes)
                    .is_some_and(|range| requirements.scratch.union(dp_homes).contains(range));
                let metadata = effect.access == Access::Read
                    && matches!(requirements.form, Form::Barrier | Form::Boundary)
                    && offset >= abi::generated::DP_STACK_FLOOR_OFFSET as u16
                    && end <= abi::generated::DP_STACK_CEILING_OFFSET as u16 + 2;
                if bytes == 0 || !(scratch || metadata) {
                    return Err("selected instruction exceeds operation scratch resources".into());
                }
                continue;
            }
            Memory::Stack { .. } | Memory::Unknown => continue,
            Memory::Long { bytes, .. }
            | Memory::Symbol { bytes, .. }
            | Memory::SymbolIndexedX { bytes, .. }
            | Memory::IndirectLong { bytes, .. } => u32::from(bytes),
        };
        if effect.access == Access::Read {
            reads += bytes;
        } else {
            writes += bytes;
        }
    }
    Ok((reads, writes))
}
