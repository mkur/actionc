#![allow(dead_code)] // Later analysis slices consume the full typed summary.

use std::collections::BTreeSet;

use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirArgHome, MirBinaryOp, MirCallTarget, MirCarryIn, MirCond,
    MirCondDest, MirDef, MirEffects, MirFixedZpSlot, MirFlag, MirFlagTest, MirMem, MirMemoryEffect,
    MirOp, MirPointerPair, MirReg, MirRegisterSet, MirResultHome, MirSpillId, MirTempId,
    MirTerminator, MirValue, MirWidth, MirZpSlot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) enum MirTempAccess {
    Full(MirTempId),
    Exact { temp: MirTempId, byte: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) enum MirTempUseKind {
    Operand,
    Address,
    CallTarget,
    CallArgument,
    BranchCondition,
    EdgeArgument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) struct MirTempUse {
    pub access: MirTempAccess,
    pub kind: MirTempUseKind,
}

impl MirTempAccess {
    pub(in crate::mir6502) fn temp(self) -> MirTempId {
        match self {
            Self::Full(temp) | Self::Exact { temp, .. } => temp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) enum MirHomeByte {
    Spill { id: MirSpillId, offset: u16 },
    VirtualZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirFlagSet {
    pub c: bool,
    pub z: bool,
    pub n: bool,
    pub v: bool,
}

impl MirFlagSet {
    pub(in crate::mir6502) fn all() -> Self {
        Self {
            c: true,
            z: true,
            n: true,
            v: true,
        }
    }

    pub(in crate::mir6502) fn contains(self, flag: MirFlag) -> bool {
        match flag {
            MirFlag::C => self.c,
            MirFlag::Z => self.z,
            MirFlag::N => self.n,
            MirFlag::V => self.v,
        }
    }

    fn insert(&mut self, flag: MirFlag) {
        match flag {
            MirFlag::C => self.c = true,
            MirFlag::Z => self.z = true,
            MirFlag::N => self.n = true,
            MirFlag::V => self.v = true,
        }
    }

    pub(in crate::mir6502) fn any(self) -> bool {
        self.c || self.z || self.n || self.v
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum MirOpKind {
    LoadImm,
    Load,
    Store,
    Move,
    LeaAddr,
    Extend,
    Truncate,
    Unary,
    Binary,
    UpdateMem,
    UpdateIndexedMem,
    AddByteToWordMem,
    SubByteFromWordMem,
    Compare,
    CompareIndirectBytes,
    Call,
    RuntimeHelper,
    MaterializeAddress,
    MaterializeIndexedAddress,
    AdvanceAddress,
    LoadIndirect,
    StoreIndirect,
    IndirectByteCompound,
    Barrier,
    MachineBlock,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirLogicalEffects {
    pub temp_uses: Vec<MirTempAccess>,
    pub classified_temp_uses: Vec<MirTempUse>,
    pub temp_defs: Vec<MirTempAccess>,
    /// Legacy whole-temp definition shape used by the compatibility liveness
    /// domain. Exact lane definitions above remain the reaching-def truth.
    pub full_temp_defs_compat: BTreeSet<MirTempId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirHomeEffects {
    /// Direct home reads used by the current spill/home compatibility helpers.
    /// Address-consumer pair reads are recorded separately below.
    pub reads: BTreeSet<MirHomeByte>,
    pub writes: BTreeSet<MirHomeByte>,
    pub unknown_reads: bool,
    pub unknown_writes: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMemoryEffects {
    pub direct_reads: Vec<MirMemoryRange>,
    pub direct_writes: Vec<MirMemoryRange>,
    pub structured_reads: MirMemoryEffect,
    pub structured_writes: MirMemoryEffect,
    pub indirect_reads: bool,
    pub indirect_writes: bool,
    pub opaque: bool,
    /// Point identities retained by the block-local compatibility queries.
    pub reads: Vec<MirMem>,
    pub definite_writes: Vec<MirMem>,
    pub may_write_any: bool,
    pub has_unknown_effects: bool,
    /// Compatibility view for the existing materialization helpers. Calls did
    /// not historically treat `opaque` alone as a memory effect, while
    /// barriers and machine blocks did.
    pub may_write_any_compat: bool,
    pub has_unknown_effects_compat: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMemoryRange {
    pub base: MirMem,
    pub bytes: u16,
}

impl MirMemoryEffects {
    pub(in crate::mir6502) fn reads(&self, mem: &MirMem) -> bool {
        self.reads.iter().any(|read| read == mem)
    }

    pub(in crate::mir6502) fn definitely_writes(&self, mem: &MirMem) -> bool {
        self.definite_writes.iter().any(|write| write == mem)
    }

    pub(in crate::mir6502) fn may_write(&self, mem: &MirMem) -> bool {
        self.may_write_any || self.definitely_writes(mem)
    }

    pub(in crate::mir6502) fn may_write_compat(&self, mem: &MirMem) -> bool {
        self.may_write_any_compat || self.definitely_writes(mem)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineEffects {
    pub register_reads: MirRegisterSet,
    pub register_writes: MirRegisterSet,
    pub register_clobbers: MirRegisterSet,
    pub conservative_register_clobbers: MirRegisterSet,
    pub flag_reads: MirFlagSet,
    pub flag_writes: MirFlagSet,
    pub flag_clobbers: MirFlagSet,
    pub uses_previous_carry: bool,
    pub definitely_overwrites_carry: bool,
    pub definitely_overwrites_overflow: bool,
    pub writes_any_flags_compat: bool,
    pub unknown_flag_or_a_effects: bool,
    pub opaque_flag_or_a_effects: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirAddressConsumerEffects {
    pub pair_reads: BTreeSet<MirHomeByte>,
    pub pair_writes: BTreeSet<MirHomeByte>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::mir6502) struct MirSpillByte {
    pub id: MirSpillId,
    pub offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirOpEffectSummary {
    pub kind: MirOpKind,
    pub logical: MirLogicalEffects,
    pub homes: MirHomeEffects,
    pub memory: MirMemoryEffects,
    pub machine: MirMachineEffects,
    pub addresses: MirAddressConsumerEffects,
    pub projected_spill_reads: BTreeSet<MirSpillId>,
    pub projected_spill_writes: BTreeSet<MirSpillId>,
    pub projected_spill_byte_reads: BTreeSet<MirSpillByte>,
    pub projected_spill_byte_writes: BTreeSet<MirSpillByte>,
    pub removable_when_results_dead: bool,
}

impl MirOpEffectSummary {
    fn new(kind: MirOpKind) -> Self {
        Self {
            kind,
            logical: MirLogicalEffects::default(),
            homes: MirHomeEffects::default(),
            memory: MirMemoryEffects::default(),
            machine: MirMachineEffects::default(),
            addresses: MirAddressConsumerEffects::default(),
            projected_spill_reads: BTreeSet::new(),
            projected_spill_writes: BTreeSet::new(),
            projected_spill_byte_reads: BTreeSet::new(),
            projected_spill_byte_writes: BTreeSet::new(),
            removable_when_results_dead: false,
        }
    }

    pub(in crate::mir6502) fn uses_temp(&self, temp: MirTempId) -> bool {
        self.logical
            .temp_uses
            .iter()
            .any(|access| access.temp() == temp)
    }

    pub(in crate::mir6502) fn temp_use_count(&self, temp: MirTempId) -> usize {
        self.logical
            .temp_uses
            .iter()
            .filter(|access| access.temp() == temp)
            .count()
    }

    pub(in crate::mir6502) fn reads_reg(&self, reg: MirReg) -> bool {
        register_set_contains(self.machine.register_reads, reg)
    }

    pub(in crate::mir6502) fn writes_reg(&self, reg: MirReg) -> bool {
        register_set_contains(self.machine.register_writes, reg)
    }

    pub(in crate::mir6502) fn may_clobber_reg_compat(&self, reg: MirReg) -> bool {
        self.writes_reg(reg)
            || register_set_contains(self.machine.register_clobbers, reg)
            || register_set_contains(self.machine.conservative_register_clobbers, reg)
    }

    pub(in crate::mir6502) fn reads_spill_byte_compat(&self, id: MirSpillId, offset: u16) -> bool {
        self.projected_spill_byte_reads
            .contains(&MirSpillByte { id, offset })
    }

    pub(in crate::mir6502) fn writes_spill_byte_compat(&self, id: MirSpillId, offset: u16) -> bool {
        !matches!(
            self.kind,
            MirOpKind::Compare | MirOpKind::CompareIndirectBytes
        ) && self
            .projected_spill_byte_writes
            .contains(&MirSpillByte { id, offset })
    }

    pub(in crate::mir6502) fn may_read_unknown_spill_byte_compat(&self) -> bool {
        matches!(
            self.kind,
            MirOpKind::RuntimeHelper | MirOpKind::Barrier | MirOpKind::MachineBlock
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirTerminatorEffectSummary {
    pub logical: MirLogicalEffects,
    pub homes: MirHomeEffects,
    pub memory: MirMemoryEffects,
    pub machine: MirMachineEffects,
    pub projected_spill_reads: BTreeSet<MirSpillId>,
    pub projected_spill_byte_reads: BTreeSet<MirSpillByte>,
    pub consumes_flags_compat: bool,
}

pub(in crate::mir6502) fn classify_op(op: &MirOp) -> MirOpEffectSummary {
    let mut summary = MirOpEffectSummary::new(match op {
        MirOp::LoadImm { .. } => MirOpKind::LoadImm,
        MirOp::Load { .. } => MirOpKind::Load,
        MirOp::Store { .. } => MirOpKind::Store,
        MirOp::Move { .. } => MirOpKind::Move,
        MirOp::LeaAddr { .. } => MirOpKind::LeaAddr,
        MirOp::Extend { .. } => MirOpKind::Extend,
        MirOp::Truncate { .. } => MirOpKind::Truncate,
        MirOp::Unary { .. } => MirOpKind::Unary,
        MirOp::Binary { .. } => MirOpKind::Binary,
        MirOp::UpdateMem { .. } => MirOpKind::UpdateMem,
        MirOp::UpdateIndexedMem { .. } => MirOpKind::UpdateIndexedMem,
        MirOp::AddByteToWordMem { .. } => MirOpKind::AddByteToWordMem,
        MirOp::SubByteFromWordMem { .. } => MirOpKind::SubByteFromWordMem,
        MirOp::Compare { .. } => MirOpKind::Compare,
        MirOp::CompareIndirectBytes { .. } => MirOpKind::CompareIndirectBytes,
        MirOp::Call { .. } => MirOpKind::Call,
        MirOp::RuntimeHelper { .. } => MirOpKind::RuntimeHelper,
        MirOp::MaterializeAddress { .. } => MirOpKind::MaterializeAddress,
        MirOp::MaterializeIndexedAddress { .. } => MirOpKind::MaterializeIndexedAddress,
        MirOp::AdvanceAddress { .. } => MirOpKind::AdvanceAddress,
        MirOp::LoadIndirect { .. } => MirOpKind::LoadIndirect,
        MirOp::StoreIndirect { .. } => MirOpKind::StoreIndirect,
        MirOp::IndirectByteCompound { .. } => MirOpKind::IndirectByteCompound,
        MirOp::Barrier { .. } => MirOpKind::Barrier,
        MirOp::MachineBlock { .. } => MirOpKind::MachineBlock,
    });

    match op {
        MirOp::LoadImm { dst, width, .. } => {
            record_def(dst, *width, &mut summary);
            mark_register_result_flags(dst, &mut summary);
            summary.removable_when_results_dead = true;
        }
        MirOp::Load { dst, src, width } => {
            record_load_addr(src, *width, &mut summary);
            record_def(dst, *width, &mut summary);
            mark_register_result_flags(dst, &mut summary);
            summary.removable_when_results_dead = true;
        }
        MirOp::Store {
            dst, src, width, ..
        } => {
            record_store_addr(dst, *width, &mut summary);
            record_value(src, &mut summary);
        }
        MirOp::Move { dst, src, width }
        | MirOp::Unary {
            dst, src, width, ..
        } => {
            record_value(src, &mut summary);
            record_def(dst, *width, &mut summary);
            mark_register_result_flags(dst, &mut summary);
            summary.removable_when_results_dead = true;
        }
        MirOp::LeaAddr { dst, width, .. } => {
            record_def(dst, *width, &mut summary);
            summary.removable_when_results_dead = true;
        }
        MirOp::Extend {
            dst, src, to_width, ..
        }
        | MirOp::Truncate {
            dst, src, to_width, ..
        } => {
            record_value(src, &mut summary);
            record_def(dst, *to_width, &mut summary);
            mark_register_result_flags(dst, &mut summary);
            summary.removable_when_results_dead = true;
        }
        MirOp::Binary {
            op,
            dst,
            left,
            right,
            width,
            carry_in,
            ..
        } => {
            record_value(left, &mut summary);
            record_value(right, &mut summary);
            record_def(dst, *width, &mut summary);
            summary.machine.uses_previous_carry =
                matches!(carry_in, Some(MirCarryIn::FromPrevious));
            let initializes_carry =
                matches!(carry_in, None | Some(MirCarryIn::Clear | MirCarryIn::Set));
            if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) && initializes_carry {
                summary.machine.definitely_overwrites_carry = true;
                summary.machine.definitely_overwrites_overflow = true;
            }
            record_binary_flags(*op, &mut summary.machine.flag_writes);
            mark_register_result_flags(dst, &mut summary);
            summary.removable_when_results_dead = true;
        }
        MirOp::UpdateMem { mem, width, .. } => {
            record_memory_range_read(mem, *width, &mut summary);
            record_definite_memory_range_write(mem, *width, &mut summary);
            write_zn(&mut summary.machine.flag_writes);
            summary.machine.writes_any_flags_compat = true;
        }
        MirOp::UpdateIndexedMem { base, .. } => {
            let addr = MirAddr::AbsoluteIndexedX { base: base.clone() };
            record_load_addr(&addr, MirWidth::Byte, &mut summary);
            record_store_addr(&addr, MirWidth::Byte, &mut summary);
            set_register(&mut summary.machine.register_reads, MirReg::X);
            write_zn(&mut summary.machine.flag_writes);
            summary.machine.writes_any_flags_compat = true;
        }
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            let high = offset_mem(mem, 1);
            record_memory_read(mem, &mut summary);
            record_memory_read(&high, &mut summary);
            record_value(value, &mut summary);
            record_definite_memory_write(mem, &mut summary);
            record_definite_memory_write(&high, &mut summary);
            summary.machine.conservative_register_clobbers.a = true;
            summary.machine.flag_clobbers = MirFlagSet::all();
            summary.machine.writes_any_flags_compat = true;
        }
        MirOp::Compare {
            dst, left, right, ..
        } => {
            record_value(left, &mut summary);
            record_value(right, &mut summary);
            match dst {
                MirCondDest::Temp(temp) => {
                    summary.logical.temp_defs.push(MirTempAccess::Exact {
                        temp: *temp,
                        byte: 0,
                    });
                    summary
                        .projected_spill_writes
                        .insert(projected_temp_spill(*temp, 0));
                    summary.projected_spill_byte_writes.insert(MirSpillByte {
                        id: projected_temp_spill(*temp, 0),
                        offset: 0,
                    });
                }
                MirCondDest::Flags => {
                    summary.machine.definitely_overwrites_carry = true;
                }
            }
            summary.machine.flag_writes.c = true;
            write_zn(&mut summary.machine.flag_writes);
            summary.machine.writes_any_flags_compat = true;
            summary.removable_when_results_dead = matches!(dst, MirCondDest::Temp(_));
        }
        MirOp::CompareIndirectBytes {
            dst,
            left,
            right,
            offset,
            ..
        } => {
            record_consumer_read(*left, &mut summary);
            record_consumer_read(*right, &mut summary);
            record_indirect_y_access(*left, *offset, &mut summary);
            summary.memory.indirect_reads = true;
            summary.memory.has_unknown_effects = true;
            set_register(&mut summary.machine.register_writes, MirReg::A);
            match dst {
                MirCondDest::Temp(temp) => {
                    summary.logical.temp_defs.push(MirTempAccess::Exact {
                        temp: *temp,
                        byte: 0,
                    });
                    summary
                        .projected_spill_writes
                        .insert(projected_temp_spill(*temp, 0));
                    summary.projected_spill_byte_writes.insert(MirSpillByte {
                        id: projected_temp_spill(*temp, 0),
                        offset: 0,
                    });
                }
                MirCondDest::Flags => {
                    summary.machine.definitely_overwrites_carry = true;
                }
            }
            summary.machine.flag_writes.c = true;
            write_zn(&mut summary.machine.flag_writes);
            summary.machine.writes_any_flags_compat = true;
            summary.removable_when_results_dead = matches!(dst, MirCondDest::Temp(_));
        }
        MirOp::Call {
            target,
            abi,
            args,
            result,
            effects,
            ..
        } => {
            record_call_target(target, &mut summary);
            for arg in args {
                record_value_as(&arg.value, MirTempUseKind::CallArgument, &mut summary);
            }
            if let Some(result) = result {
                record_def(&result.dst, result.width, &mut summary);
            }
            apply_structured_effects(effects, false, &mut summary);
            let mut clobbers = effects.clobbers;
            merge_register_sets(&mut clobbers, abi.clobbers);
            merge_register_sets(&mut summary.machine.register_clobbers, clobbers);
            let mut preserves = effects.preserves;
            merge_register_sets(&mut preserves, abi.preserves);
            summary.machine.conservative_register_clobbers =
                incomplete_call_clobbers(clobbers, preserves);
            if clobbers.flags {
                summary.machine.flag_clobbers = MirFlagSet::all();
            }
            summary.machine.writes_any_flags_compat = true;
            summary.machine.unknown_flag_or_a_effects = true;
        }
        MirOp::RuntimeHelper {
            args,
            result,
            effects,
            ..
        } => {
            for arg in args {
                record_arg_home_read(arg, &mut summary);
            }
            if let Some(result) = result {
                record_result_home_write(result, &mut summary);
            }
            apply_structured_effects(effects, false, &mut summary);
            summary.machine.conservative_register_clobbers =
                incomplete_call_clobbers(effects.clobbers, effects.preserves);
            summary.machine.writes_any_flags_compat = true;
            summary.machine.unknown_flag_or_a_effects = true;
        }
        MirOp::MaterializeAddress { consumer, value } => {
            record_value_as(value, MirTempUseKind::Address, &mut summary);
            record_consumer_write(*consumer, &mut summary);
        }
        MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            ..
        } => {
            record_consumer_write(*consumer, &mut summary);
            record_value_as(base, MirTempUseKind::Address, &mut summary);
            record_value_as(index, MirTempUseKind::Address, &mut summary);
            summary.machine.conservative_register_clobbers.a = true;
            if consumer.uses_scaled_y() {
                set_register(&mut summary.machine.register_writes, MirReg::Y);
                summary.machine.conservative_register_clobbers.y = true;
            }
            summary.machine.flag_clobbers = MirFlagSet::all();
            summary.machine.writes_any_flags_compat = true;
        }
        MirOp::AdvanceAddress {
            consumer, index, ..
        } => {
            record_consumer_read(*consumer, &mut summary);
            record_consumer_write(*consumer, &mut summary);
            record_value_as(index, MirTempUseKind::Address, &mut summary);
        }
        MirOp::LoadIndirect {
            consumer,
            dst,
            offset,
        } => {
            record_consumer_read(*consumer, &mut summary);
            record_indirect_y_access(*consumer, *offset, &mut summary);
            record_def(dst, MirWidth::Byte, &mut summary);
            mark_register_result_flags(dst, &mut summary);
            summary.memory.indirect_reads = true;
            summary.memory.has_unknown_effects = true;
        }
        MirOp::StoreIndirect {
            consumer,
            src,
            offset,
        } => {
            record_consumer_read(*consumer, &mut summary);
            record_indirect_y_access(*consumer, *offset, &mut summary);
            record_value(src, &mut summary);
            summary.memory.indirect_writes = true;
            summary.memory.may_write_any = true;
            summary.memory.has_unknown_effects = true;
            summary.memory.may_write_any_compat = true;
            summary.memory.has_unknown_effects_compat = true;
        }
        MirOp::IndirectByteCompound { target, source, .. } => {
            record_consumer_read(*target, &mut summary);
            record_consumer_read(*source, &mut summary);
            record_consumer_write(*target, &mut summary);
            summary.memory.indirect_reads = true;
            summary.memory.indirect_writes = true;
            summary.memory.may_write_any = true;
            summary.memory.has_unknown_effects = true;
            summary.memory.may_write_any_compat = true;
            summary.memory.has_unknown_effects_compat = true;
            summary.machine.conservative_register_clobbers.a = true;
            summary.machine.flag_clobbers = MirFlagSet::all();
            summary.machine.writes_any_flags_compat = true;
        }
        MirOp::Barrier { effects } | MirOp::MachineBlock { effects, .. } => {
            apply_structured_effects(effects, true, &mut summary);
            summary.machine.conservative_register_clobbers = all_registers();
            summary.machine.opaque_flag_or_a_effects = true;
        }
    }

    summary
}

pub(in crate::mir6502) fn classify_terminator(
    terminator: &MirTerminator,
) -> MirTerminatorEffectSummary {
    let mut summary = MirTerminatorEffectSummary::default();
    match terminator {
        MirTerminator::Jump(edge) => {
            for arg in &edge.args {
                record_terminator_value(
                    &arg.value,
                    Some(arg.width),
                    MirTempUseKind::EdgeArgument,
                    &mut summary,
                );
            }
        }
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => {
            match cond {
                MirCond::Deferred => {}
                MirCond::BoolValue(value) => record_terminator_value(
                    value,
                    None,
                    MirTempUseKind::BranchCondition,
                    &mut summary,
                ),
                MirCond::FlagTest(test) => record_flag_test(test, &mut summary.machine.flag_reads),
                MirCond::AnyFlagTest(tests) => {
                    for test in tests {
                        record_flag_test(test, &mut summary.machine.flag_reads);
                    }
                }
                MirCond::FusedCompare { flag_test, .. } => {
                    record_flag_test(flag_test, &mut summary.machine.flag_reads)
                }
            }
            summary.consumes_flags_compat =
                matches!(cond, MirCond::FlagTest(_) | MirCond::FusedCompare { .. });
            for edge in [then_edge, else_edge] {
                for arg in &edge.args {
                    record_terminator_value(
                        &arg.value,
                        Some(arg.width),
                        MirTempUseKind::EdgeArgument,
                        &mut summary,
                    );
                }
            }
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
    summary
}

pub(in crate::mir6502) fn classify_value(value: &MirValue) -> MirOpEffectSummary {
    let mut summary = MirOpEffectSummary::new(MirOpKind::Move);
    record_value(value, &mut summary);
    summary
}

pub(in crate::mir6502) fn count_call_target_temp_uses(
    target: &MirCallTarget,
    temp: MirTempId,
) -> usize {
    let mut summary = MirOpEffectSummary::new(MirOpKind::Call);
    record_call_target(target, &mut summary);
    summary.temp_use_count(temp)
}

fn record_terminator_value(
    value: &MirValue,
    width: Option<MirWidth>,
    kind: MirTempUseKind,
    summary: &mut MirTerminatorEffectSummary,
) {
    let value_summary = classify_typed_value(value, width);
    summary.logical.classified_temp_uses.extend(
        value_summary
            .logical
            .temp_uses
            .iter()
            .copied()
            .map(|access| MirTempUse { access, kind }),
    );
    summary
        .logical
        .temp_uses
        .extend(value_summary.logical.temp_uses);
    summary.homes.reads.extend(value_summary.homes.reads);
    summary.memory.reads.extend(value_summary.memory.reads);
    merge_register_sets(
        &mut summary.machine.register_reads,
        value_summary.machine.register_reads,
    );
    summary
        .projected_spill_reads
        .extend(value_summary.projected_spill_reads);
    summary
        .projected_spill_byte_reads
        .extend(value_summary.projected_spill_byte_reads);
}

fn classify_typed_value(value: &MirValue, width: Option<MirWidth>) -> MirOpEffectSummary {
    if matches!(width, Some(MirWidth::Byte))
        && let MirValue::Def(MirDef::VTemp(temp)) = value
    {
        let mut summary = MirOpEffectSummary::new(MirOpKind::Move);
        record_temp_use(
            MirTempAccess::Exact {
                temp: *temp,
                byte: 0,
            },
            MirTempUseKind::Operand,
            &mut summary,
        );
        summary
            .projected_spill_reads
            .insert(projected_temp_spill(*temp, 0));
        summary.projected_spill_byte_reads.insert(MirSpillByte {
            id: projected_temp_spill(*temp, 0),
            offset: 0,
        });
        summary
    } else {
        classify_value(value)
    }
}

fn record_value(value: &MirValue, summary: &mut MirOpEffectSummary) {
    record_value_as(value, MirTempUseKind::Operand, summary);
}

fn record_value_as(value: &MirValue, kind: MirTempUseKind, summary: &mut MirOpEffectSummary) {
    match value {
        MirValue::Def(MirDef::VTemp(temp)) => {
            record_temp_use(MirTempAccess::Full(*temp), kind, summary);
            summary
                .projected_spill_reads
                .insert(projected_temp_spill(*temp, 0));
            summary.projected_spill_byte_reads.insert(MirSpillByte {
                id: projected_temp_spill(*temp, 0),
                offset: 0,
            });
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) => {
            record_temp_use(
                MirTempAccess::Exact {
                    temp: *id,
                    byte: *byte,
                },
                kind,
                summary,
            );
            if *byte <= 1 {
                summary
                    .projected_spill_reads
                    .insert(projected_temp_spill(*id, *byte));
                summary.projected_spill_byte_reads.insert(MirSpillByte {
                    id: projected_temp_spill(*id, *byte),
                    offset: 0,
                });
            }
        }
        MirValue::Def(MirDef::Reg(reg)) => set_register(&mut summary.machine.register_reads, *reg),
        MirValue::Word { lo, hi } => {
            record_value_as(lo, kind, summary);
            record_value_as(hi, kind, summary);
        }
        MirValue::PointerCell(mem) => record_memory_read(mem, summary),
        MirValue::StorageAddrByte { mem, .. } => record_home_reference(mem, summary),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn record_temp_use(access: MirTempAccess, kind: MirTempUseKind, summary: &mut MirOpEffectSummary) {
    summary.logical.temp_uses.push(access);
    summary
        .logical
        .classified_temp_uses
        .push(MirTempUse { access, kind });
}

fn record_def(def: &MirDef, width: MirWidth, summary: &mut MirOpEffectSummary) {
    match def {
        MirDef::VTemp(temp) => {
            summary.logical.full_temp_defs_compat.insert(*temp);
            summary.logical.temp_defs.push(MirTempAccess::Exact {
                temp: *temp,
                byte: 0,
            });
            if width == MirWidth::Word {
                summary.logical.temp_defs.push(MirTempAccess::Exact {
                    temp: *temp,
                    byte: 1,
                });
            }
            summary
                .projected_spill_writes
                .insert(projected_temp_spill(*temp, 0));
            summary.projected_spill_byte_writes.insert(MirSpillByte {
                id: projected_temp_spill(*temp, 0),
                offset: 0,
            });
        }
        MirDef::VTempByte { id, byte } => {
            summary.logical.temp_defs.push(MirTempAccess::Exact {
                temp: *id,
                byte: *byte,
            });
            if *byte <= 1 {
                summary
                    .projected_spill_writes
                    .insert(projected_temp_spill(*id, *byte));
                summary.projected_spill_byte_writes.insert(MirSpillByte {
                    id: projected_temp_spill(*id, *byte),
                    offset: 0,
                });
            }
        }
        MirDef::Reg(reg) => set_register(&mut summary.machine.register_writes, *reg),
    }
}

fn record_load_addr(addr: &MirAddr, width: MirWidth, summary: &mut MirOpEffectSummary) {
    match addr {
        MirAddr::Direct(mem) => record_memory_range_read(mem, width, summary),
        MirAddr::AbsoluteIndexedX { base: mem } => {
            set_register(&mut summary.machine.register_reads, MirReg::X);
            record_memory_read(mem, summary);
            summary.memory.indirect_reads = true;
        }
        MirAddr::AbsoluteIndexedY { base: mem } => {
            set_register(&mut summary.machine.register_reads, MirReg::Y);
            record_memory_read(mem, summary);
            summary.memory.indirect_reads = true;
        }
        MirAddr::PointerCell { ptr: mem, .. } => {
            record_memory_range_read(mem, MirWidth::Word, summary);
            summary.memory.indirect_reads = true;
        }
        MirAddr::ComputedIndex { base, index, .. } => {
            record_value_as(base, MirTempUseKind::Address, summary);
            record_value_as(index, MirTempUseKind::Address, summary);
            summary.memory.indirect_reads = true;
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            record_memory_range_read(ptr, MirWidth::Word, summary);
            record_value_as(index, MirTempUseKind::Address, summary);
            summary.memory.indirect_reads = true;
        }
        MirAddr::Deref { ptr, .. } => {
            record_value_as(ptr, MirTempUseKind::Address, summary);
            summary.memory.indirect_reads = true;
        }
        MirAddr::IndirectIndexedY { zp } => {
            set_register(&mut summary.machine.register_reads, MirReg::Y);
            record_consumer_read(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(*zp)),
                summary,
            );
            summary.memory.indirect_reads = true;
        }
        MirAddr::FixedIndirectIndexedY { zp } => {
            set_register(&mut summary.machine.register_reads, MirReg::Y);
            record_consumer_read(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo: *zp }),
                summary,
            );
            summary.memory.indirect_reads = true;
        }
        MirAddr::ZeroPageIndexedX { .. } => {
            set_register(&mut summary.machine.register_reads, MirReg::X);
            summary.memory.indirect_reads = true;
        }
        MirAddr::Label(_) => {
            summary.memory.indirect_reads = true;
        }
    }
}

fn record_store_addr(addr: &MirAddr, width: MirWidth, summary: &mut MirOpEffectSummary) {
    match addr {
        MirAddr::Direct(mem) => record_definite_memory_range_write(mem, width, summary),
        MirAddr::PointerCell { ptr, .. } => {
            record_memory_range_read(ptr, MirWidth::Word, summary);
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::ComputedIndex { base, index, .. } => {
            record_value_as(base, MirTempUseKind::Address, summary);
            record_value_as(index, MirTempUseKind::Address, summary);
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            record_memory_range_read(ptr, MirWidth::Word, summary);
            record_value_as(index, MirTempUseKind::Address, summary);
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::Deref { ptr, .. } => {
            record_value_as(ptr, MirTempUseKind::Address, summary);
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::IndirectIndexedY { zp } => {
            set_register(&mut summary.machine.register_reads, MirReg::Y);
            record_consumer_read(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(*zp)),
                summary,
            );
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::FixedIndirectIndexedY { zp } => {
            set_register(&mut summary.machine.register_reads, MirReg::Y);
            record_consumer_read(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo: *zp }),
                summary,
            );
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::ZeroPageIndexedX { .. } | MirAddr::AbsoluteIndexedX { .. } => {
            set_register(&mut summary.machine.register_reads, MirReg::X);
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::AbsoluteIndexedY { .. } => {
            set_register(&mut summary.machine.register_reads, MirReg::Y);
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
        MirAddr::Label(_) => {
            summary.memory.indirect_writes = true;
            mark_may_write_any(summary);
        }
    }
}

fn record_call_target(target: &MirCallTarget, summary: &mut MirOpEffectSummary) {
    if let MirCallTarget::Indirect { target, .. } = target {
        record_value_as(target, MirTempUseKind::CallTarget, summary);
    }
}

fn record_memory_read(mem: &MirMem, summary: &mut MirOpEffectSummary) {
    push_unique_range(&mut summary.memory.direct_reads, mem, 1);
    push_unique_mem(&mut summary.memory.reads, mem);
    record_home_reference(mem, summary);
}

fn record_memory_range_read(mem: &MirMem, width: MirWidth, summary: &mut MirOpEffectSummary) {
    let bytes = width_bytes(width);
    push_unique_range(&mut summary.memory.direct_reads, mem, bytes);
    // Preserve the old point query while the routine-wide range query is being
    // introduced. Explicit two-byte pseudos record each lane separately.
    push_unique_mem(&mut summary.memory.reads, mem);
    for offset in 0..bytes {
        record_home_reference(&offset_mem(mem, offset), summary);
    }
}

fn record_home_reference(mem: &MirMem, summary: &mut MirOpEffectSummary) {
    if let Some(home) = home_byte(mem) {
        summary.homes.reads.insert(home);
    }
    if let MirMem::Spill { id, offset } = mem {
        summary.projected_spill_reads.insert(*id);
        summary.projected_spill_byte_reads.insert(MirSpillByte {
            id: *id,
            offset: *offset,
        });
    }
}

fn record_definite_memory_write(mem: &MirMem, summary: &mut MirOpEffectSummary) {
    push_unique_range(&mut summary.memory.direct_writes, mem, 1);
    push_unique_mem(&mut summary.memory.definite_writes, mem);
    if let Some(home) = home_byte(mem) {
        summary.homes.writes.insert(home);
    }
    if let MirMem::Spill { id, offset } = mem {
        summary.projected_spill_writes.insert(*id);
        summary.projected_spill_byte_writes.insert(MirSpillByte {
            id: *id,
            offset: *offset,
        });
    }
}

fn record_definite_memory_range_write(
    mem: &MirMem,
    width: MirWidth,
    summary: &mut MirOpEffectSummary,
) {
    let bytes = width_bytes(width);
    push_unique_range(&mut summary.memory.direct_writes, mem, bytes);
    push_unique_mem(&mut summary.memory.definite_writes, mem);
    for offset in 0..bytes {
        let lane = offset_mem(mem, offset);
        if let Some(home) = home_byte(&lane) {
            summary.homes.writes.insert(home);
        }
        if let MirMem::Spill { id, offset } = lane {
            summary.projected_spill_writes.insert(id);
            summary
                .projected_spill_byte_writes
                .insert(MirSpillByte { id, offset });
        }
    }
}

fn record_consumer_read(consumer: MirAddressConsumer, summary: &mut MirOpEffectSummary) {
    for mem in consumer_mems(consumer) {
        push_unique_range(&mut summary.memory.direct_reads, &mem, 1);
        push_unique_mem(&mut summary.memory.reads, &mem);
        if let Some(home) = home_byte(&mem) {
            summary.addresses.pair_reads.insert(home);
        }
    }
}

fn record_consumer_write(consumer: MirAddressConsumer, summary: &mut MirOpEffectSummary) {
    for mem in consumer_mems(consumer) {
        push_unique_range(&mut summary.memory.direct_writes, &mem, 1);
        if let Some(home) = home_byte(&mem) {
            summary.addresses.pair_writes.insert(home);
        }
    }
}

fn record_indirect_y_access(
    consumer: MirAddressConsumer,
    offset: u16,
    summary: &mut MirOpEffectSummary,
) {
    if !consumer.uses_scaled_y() {
        set_register(&mut summary.machine.register_writes, MirReg::Y);
        write_zn(&mut summary.machine.flag_writes);
        summary.machine.writes_any_flags_compat = true;
        return;
    }
    set_register(&mut summary.machine.register_reads, MirReg::Y);
    if offset == 1 {
        set_register(&mut summary.machine.register_writes, MirReg::Y);
        write_zn(&mut summary.machine.flag_writes);
        summary.machine.writes_any_flags_compat = true;
    }
}

fn record_arg_home_read(home: &MirArgHome, summary: &mut MirOpEffectSummary) {
    match home {
        MirArgHome::Reg(reg) => set_register(&mut summary.machine.register_reads, *reg),
        MirArgHome::RegisterPair { lo, hi } => {
            set_register(&mut summary.machine.register_reads, *lo);
            set_register(&mut summary.machine.register_reads, *hi);
        }
        MirArgHome::BytePair { lo, hi } => {
            record_arg_home_read(lo, summary);
            record_arg_home_read(hi, summary);
        }
        MirArgHome::ZeroPage(slot) => record_memory_read(&MirMem::ZeroPage(*slot), summary),
        MirArgHome::FixedZeroPage(slot) => {
            record_memory_read(&MirMem::FixedZeroPage(*slot), summary)
        }
        MirArgHome::Absolute(address) => record_memory_read(&MirMem::Absolute(*address), summary),
        MirArgHome::StackFrame { .. } => {
            // The helper stack base is resolved during emission, so this is not
            // an absolute address at MIR analysis time.
            summary.homes.unknown_reads = true;
            summary.memory.indirect_reads = true;
            summary.memory.has_unknown_effects = true;
        }
    }
}

fn record_result_home_write(home: &MirResultHome, summary: &mut MirOpEffectSummary) {
    match home {
        MirResultHome::Reg(reg) => set_register(&mut summary.machine.register_writes, *reg),
        MirResultHome::RegisterPair { lo, hi } => {
            set_register(&mut summary.machine.register_writes, *lo);
            set_register(&mut summary.machine.register_writes, *hi);
        }
        MirResultHome::ZeroPage(slot) => {
            record_definite_memory_write(&MirMem::ZeroPage(*slot), summary)
        }
        MirResultHome::FixedZeroPage(slot) => {
            record_definite_memory_write(&MirMem::FixedZeroPage(*slot), summary)
        }
        MirResultHome::Absolute(address) => {
            record_definite_memory_write(&MirMem::Absolute(*address), summary)
        }
        MirResultHome::ReturnSlot { .. } => {
            // The public return-slot base is assigned outside this operation.
            summary.homes.unknown_writes = true;
            summary.memory.indirect_writes = true;
            summary.memory.has_unknown_effects = true;
        }
    }
}

fn apply_structured_effects(
    effects: &MirEffects,
    opaque_is_memory_effect: bool,
    summary: &mut MirOpEffectSummary,
) {
    summary.memory.structured_reads = effects.memory_reads.clone();
    summary.memory.structured_writes = effects.memory_writes.clone();
    summary.memory.opaque = effects.opaque;
    summary.memory.indirect_reads |= effects.opaque
        || matches!(
            effects.memory_reads,
            MirMemoryEffect::Unknown | MirMemoryEffect::All
        );
    summary.memory.indirect_writes |= effects.opaque
        || matches!(
            effects.memory_writes,
            MirMemoryEffect::Unknown | MirMemoryEffect::All
        );
    summary.memory.has_unknown_effects = effects.opaque
        || !matches!(effects.memory_reads, MirMemoryEffect::None)
        || !matches!(effects.memory_writes, MirMemoryEffect::None);
    summary.memory.may_write_any =
        effects.opaque || !matches!(effects.memory_writes, MirMemoryEffect::None);
    summary.memory.has_unknown_effects_compat = (opaque_is_memory_effect && effects.opaque)
        || !matches!(effects.memory_reads, MirMemoryEffect::None)
        || !matches!(effects.memory_writes, MirMemoryEffect::None);
    summary.memory.may_write_any_compat = (opaque_is_memory_effect && effects.opaque)
        || !matches!(effects.memory_writes, MirMemoryEffect::None);
    summary.homes.unknown_reads =
        effects.opaque || !matches!(effects.memory_reads, MirMemoryEffect::None);
    summary.homes.unknown_writes =
        effects.opaque || !matches!(effects.memory_writes, MirMemoryEffect::None);
    summary.machine.register_clobbers = effects.clobbers;
    if effects.clobbers.flags || effects.opaque {
        summary.machine.flag_clobbers = MirFlagSet::all();
    }
}

fn mark_may_write_any(summary: &mut MirOpEffectSummary) {
    summary.memory.may_write_any = true;
    summary.memory.may_write_any_compat = true;
}

fn mark_register_result_flags(def: &MirDef, summary: &mut MirOpEffectSummary) {
    if matches!(def, MirDef::Reg(_)) {
        write_zn(&mut summary.machine.flag_writes);
        summary.machine.writes_any_flags_compat = true;
    }
}

fn record_binary_flags(op: MirBinaryOp, flags: &mut MirFlagSet) {
    write_zn(flags);
    if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) {
        flags.c = true;
        flags.v = true;
    } else if matches!(op, MirBinaryOp::Lsh | MirBinaryOp::Rsh) {
        flags.c = true;
    }
}

fn record_flag_test(test: &MirFlagTest, flags: &mut MirFlagSet) {
    flags.insert(match test {
        MirFlagTest::CSet | MirFlagTest::CClear => MirFlag::C,
        MirFlagTest::ZSet | MirFlagTest::ZClear => MirFlag::Z,
        MirFlagTest::NSet | MirFlagTest::NClear => MirFlag::N,
        MirFlagTest::VSet | MirFlagTest::VClear => MirFlag::V,
    });
}

fn write_zn(flags: &mut MirFlagSet) {
    flags.z = true;
    flags.n = true;
}

fn home_byte(mem: &MirMem) -> Option<MirHomeByte> {
    match mem {
        MirMem::Spill { id, offset } => Some(MirHomeByte::Spill {
            id: *id,
            offset: *offset,
        }),
        MirMem::ZeroPage(slot) => Some(MirHomeByte::VirtualZeroPage(*slot)),
        MirMem::FixedZeroPage(slot) => Some(MirHomeByte::FixedZeroPage(*slot)),
        MirMem::Absolute(_)
        | MirMem::Static { .. }
        | MirMem::Global { .. }
        | MirMem::Local { .. }
        | MirMem::Param { .. } => None,
    }
}

fn consumer_mems(consumer: MirAddressConsumer) -> [MirMem; 2] {
    match consumer.pointer_pair() {
        MirPointerPair::Virtual(slot) => [MirMem::ZeroPage(slot), MirMem::ZeroPage(slot)],
        MirPointerPair::Fixed { lo } => [
            MirMem::FixedZeroPage(lo),
            MirMem::FixedZeroPage(MirFixedZpSlot(lo.0.saturating_add(1))),
        ],
    }
}

fn projected_temp_spill(temp: MirTempId, byte: u8) -> MirSpillId {
    MirSpillId(temp.0.saturating_mul(2).saturating_add(u32::from(byte)))
}

fn offset_mem(mem: &MirMem, delta: u16) -> MirMem {
    match mem {
        MirMem::Absolute(address) => MirMem::Absolute(address.saturating_add(delta)),
        MirMem::Static { id, offset } => MirMem::Static {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Global { id, offset } => MirMem::Global {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Local { id, offset } => MirMem::Local {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Param { id, offset } => MirMem::Param {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Spill { id, offset } => MirMem::Spill {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::ZeroPage(slot) => MirMem::ZeroPage(*slot),
        MirMem::FixedZeroPage(slot) => {
            MirMem::FixedZeroPage(MirFixedZpSlot(slot.0.saturating_add(delta as u8)))
        }
    }
}

fn push_unique_mem(mems: &mut Vec<MirMem>, mem: &MirMem) {
    if !mems.iter().any(|existing| existing == mem) {
        mems.push(mem.clone());
    }
}

fn push_unique_range(ranges: &mut Vec<MirMemoryRange>, base: &MirMem, bytes: u16) {
    let range = MirMemoryRange {
        base: base.clone(),
        bytes,
    };
    if !ranges.iter().any(|existing| existing == &range) {
        ranges.push(range);
    }
}

fn width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn set_register(registers: &mut MirRegisterSet, reg: MirReg) {
    match reg {
        MirReg::A => registers.a = true,
        MirReg::X => registers.x = true,
        MirReg::Y => registers.y = true,
    }
}

fn register_set_contains(registers: MirRegisterSet, reg: MirReg) -> bool {
    match reg {
        MirReg::A => registers.a,
        MirReg::X => registers.x,
        MirReg::Y => registers.y,
    }
}

fn merge_register_sets(into: &mut MirRegisterSet, other: MirRegisterSet) {
    into.a |= other.a;
    into.x |= other.x;
    into.y |= other.y;
    into.flags |= other.flags;
    into.sp |= other.sp;
}

fn all_registers() -> MirRegisterSet {
    MirRegisterSet {
        a: true,
        x: true,
        y: true,
        flags: true,
        sp: true,
    }
}

fn incomplete_call_clobbers(clobbers: MirRegisterSet, preserves: MirRegisterSet) -> MirRegisterSet {
    MirRegisterSet {
        a: !clobbers.a && !preserves.a,
        x: !clobbers.x && !preserves.x,
        y: !clobbers.y && !preserves.y,
        flags: !clobbers.flags && !preserves.flags,
        // A call's transient stack use is balanced at its MIR boundary unless
        // the structured effects explicitly say that SP is clobbered.
        sp: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlockId, MirCallAbi, MirCallArg, MirCallResult, MirCarryOut, MirCompareOp, MirEdge,
        MirEdgeArg, MirMachineBlockId, MirOpRef, MirRuntimeHelper, MirUnaryOp, MirUpdateOp,
        RoutineId,
    };

    fn temp(id: u32) -> MirDef {
        MirDef::VTemp(MirTempId(id))
    }

    fn temp_value(id: u32) -> MirValue {
        MirValue::Def(temp(id))
    }

    fn spill(id: u32, offset: u16) -> MirMem {
        MirMem::Spill {
            id: MirSpillId(id),
            offset,
        }
    }

    fn consumer() -> MirAddressConsumer {
        MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0x90),
        })
    }

    fn opaque_effects() -> MirEffects {
        MirEffects {
            memory_reads: MirMemoryEffect::None,
            memory_writes: MirMemoryEffect::None,
            opaque: true,
            ..MirEffects::default()
        }
    }

    fn call_abi() -> MirCallAbi {
        MirCallAbi {
            params: Vec::new(),
            result: None,
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        }
    }

    #[test]
    fn every_operation_family_has_an_explicit_effect_classification() {
        let operations = vec![
            (
                MirOpKind::LoadImm,
                MirOp::LoadImm {
                    dst: temp(1),
                    value: 1,
                    width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::Load,
                MirOp::Load {
                    dst: temp(1),
                    src: MirAddr::Direct(spill(1, 0)),
                    width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::Store,
                MirOp::Store {
                    dst: MirAddr::Direct(spill(1, 0)),
                    src: temp_value(2),
                    width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::Move,
                MirOp::Move {
                    dst: temp(1),
                    src: temp_value(2),
                    width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::LeaAddr,
                MirOp::LeaAddr {
                    dst: temp(1),
                    target: MirMem::Absolute(0x4000),
                    width: MirWidth::Word,
                },
            ),
            (
                MirOpKind::Extend,
                MirOp::Extend {
                    dst: temp(1),
                    src: temp_value(2),
                    from_width: MirWidth::Byte,
                    to_width: MirWidth::Word,
                    signed: false,
                },
            ),
            (
                MirOpKind::Truncate,
                MirOp::Truncate {
                    dst: temp(1),
                    src: temp_value(2),
                    from_width: MirWidth::Word,
                    to_width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::Unary,
                MirOp::Unary {
                    op: MirUnaryOp::Neg,
                    dst: temp(1),
                    src: temp_value(2),
                    width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::Binary,
                MirOp::Binary {
                    op: MirBinaryOp::Add,
                    dst: temp(1),
                    left: temp_value(2),
                    right: MirValue::ConstU8(1),
                    width: MirWidth::Byte,
                    carry_in: Some(MirCarryIn::Clear),
                    carry_out: MirCarryOut::Ignore,
                },
            ),
            (
                MirOpKind::UpdateMem,
                MirOp::UpdateMem {
                    op: MirUpdateOp::Inc,
                    mem: spill(1, 0),
                    width: MirWidth::Byte,
                },
            ),
            (
                MirOpKind::UpdateIndexedMem,
                MirOp::UpdateIndexedMem {
                    op: MirUpdateOp::Inc,
                    base: MirMem::Absolute(0x4000),
                },
            ),
            (
                MirOpKind::AddByteToWordMem,
                MirOp::AddByteToWordMem {
                    mem: spill(1, 0),
                    value: temp_value(2),
                },
            ),
            (
                MirOpKind::SubByteFromWordMem,
                MirOp::SubByteFromWordMem {
                    mem: spill(1, 0),
                    value: temp_value(2),
                },
            ),
            (
                MirOpKind::Compare,
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: temp_value(1),
                    right: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ),
            (
                MirOpKind::CompareIndirectBytes,
                MirOp::CompareIndirectBytes {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: consumer(),
                    right: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                        lo: MirFixedZpSlot(0x92),
                    }),
                    offset: 0,
                    signed: false,
                },
            ),
            (
                MirOpKind::Call,
                MirOp::Call {
                    target: MirCallTarget::Indirect {
                        target: temp_value(1),
                        width: MirWidth::Word,
                    },
                    abi: call_abi(),
                    args: vec![MirCallArg {
                        value: temp_value(2),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(MirReg::A),
                    }],
                    result: Some(MirCallResult {
                        dst: MirDef::Reg(MirReg::A),
                        width: MirWidth::Byte,
                        home: MirResultHome::Reg(MirReg::A),
                    }),
                    effects: MirEffects::default(),
                },
            ),
            (
                MirOpKind::RuntimeHelper,
                MirOp::RuntimeHelper {
                    helper: MirRuntimeHelper::Mul,
                    args: vec![MirArgHome::FixedZeroPage(MirFixedZpSlot(0x80))],
                    result: Some(MirResultHome::FixedZeroPage(MirFixedZpSlot(0x82))),
                    effects: MirEffects::default(),
                },
            ),
            (
                MirOpKind::MaterializeAddress,
                MirOp::MaterializeAddress {
                    consumer: consumer(),
                    value: temp_value(1),
                },
            ),
            (
                MirOpKind::MaterializeIndexedAddress,
                MirOp::MaterializeIndexedAddress {
                    consumer: consumer(),
                    base: temp_value(1),
                    index: temp_value(2),
                    scale: 2,
                },
            ),
            (
                MirOpKind::AdvanceAddress,
                MirOp::AdvanceAddress {
                    consumer: consumer(),
                    index: temp_value(1),
                    scale: 2,
                },
            ),
            (
                MirOpKind::LoadIndirect,
                MirOp::LoadIndirect {
                    consumer: consumer(),
                    dst: temp(1),
                    offset: 0,
                },
            ),
            (
                MirOpKind::StoreIndirect,
                MirOp::StoreIndirect {
                    consumer: consumer(),
                    src: temp_value(1),
                    offset: 0,
                },
            ),
            (
                MirOpKind::IndirectByteCompound,
                MirOp::IndirectByteCompound {
                    op: MirBinaryOp::Add,
                    target: consumer(),
                    source: consumer(),
                    offset: 0,
                },
            ),
            (
                MirOpKind::Barrier,
                MirOp::Barrier {
                    effects: opaque_effects(),
                },
            ),
            (
                MirOpKind::MachineBlock,
                MirOp::MachineBlock {
                    id: MirMachineBlockId(1),
                    effects: opaque_effects(),
                },
            ),
        ];

        assert_eq!(operations.len(), 25);
        for (expected, operation) in operations {
            assert_eq!(classify_op(&operation).kind, expected, "{operation:?}");
        }
    }

    #[test]
    fn dual_indirect_compare_records_both_pointer_reads_and_machine_clobbers() {
        let right = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0x92),
        });
        let effects = classify_op(&MirOp::CompareIndirectBytes {
            dst: MirCondDest::Temp(MirTempId(7)),
            op: MirCompareOp::Ne,
            left: consumer(),
            right,
            offset: 3,
            signed: false,
        });

        for slot in 0x90..=0x93 {
            assert!(
                effects
                    .addresses
                    .pair_reads
                    .contains(&MirHomeByte::FixedZeroPage(MirFixedZpSlot(slot)))
            );
        }
        assert!(effects.memory.indirect_reads);
        assert!(effects.memory.has_unknown_effects);
        assert!(effects.writes_reg(MirReg::A));
        assert!(effects.writes_reg(MirReg::Y));
        assert!(effects.machine.flag_writes.c);
        assert!(effects.machine.flag_writes.z);
        assert!(effects.machine.flag_writes.n);
        assert!(
            effects
                .logical
                .temp_defs
                .iter()
                .any(|access| access.temp() == MirTempId(7))
        );
    }

    #[test]
    fn effects_distinguish_logical_homes_memory_and_machine_state() {
        let store = classify_op(&MirOp::Store {
            dst: MirAddr::Direct(spill(7, 3)),
            src: MirValue::Word {
                lo: Box::new(temp_value(4)),
                hi: Box::new(MirValue::Def(MirDef::Reg(MirReg::X))),
            },
            width: MirWidth::Word,
        });
        assert!(store.uses_temp(MirTempId(4)));
        assert!(store.reads_reg(MirReg::X));
        assert!(store.memory.definitely_writes(&spill(7, 3)));
        assert_eq!(
            store.memory.direct_writes,
            vec![MirMemoryRange {
                base: spill(7, 3),
                bytes: 2,
            }]
        );
        assert!(store.homes.writes.contains(&MirHomeByte::Spill {
            id: MirSpillId(7),
            offset: 3,
        }));
        assert!(store.homes.writes.contains(&MirHomeByte::Spill {
            id: MirSpillId(7),
            offset: 4,
        }));

        let indirect = classify_op(&MirOp::LoadIndirect {
            consumer: consumer(),
            dst: MirDef::Reg(MirReg::A),
            offset: 0,
        });
        assert_eq!(indirect.addresses.pair_reads.len(), 2);
        assert!(indirect.memory.indirect_reads);
        assert!(indirect.memory.has_unknown_effects);
        assert!(!indirect.memory.has_unknown_effects_compat);
        assert!(indirect.writes_reg(MirReg::A));
        assert!(indirect.writes_reg(MirReg::Y));
        assert!(indirect.machine.flag_writes.z);
        assert!(indirect.machine.flag_writes.n);
    }

    #[test]
    fn calls_and_machine_blocks_remain_conservative() {
        let call = classify_op(&MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: call_abi(),
            args: Vec::new(),
            result: None,
            effects: opaque_effects(),
        });
        assert!(call.memory.has_unknown_effects);
        assert!(!call.memory.has_unknown_effects_compat);
        assert!(call.may_clobber_reg_compat(MirReg::A));
        assert!(call.machine.flag_clobbers.any());

        let machine = classify_op(&MirOp::MachineBlock {
            id: MirMachineBlockId(1),
            effects: opaque_effects(),
        });
        assert!(machine.memory.has_unknown_effects_compat);
        assert!(machine.memory.may_write_any_compat);
        assert!(machine.may_clobber_reg_compat(MirReg::X));
        assert!(machine.machine.opaque_flag_or_a_effects);
    }

    #[test]
    fn projects_exact_temp_spill_bytes_for_cfg_cleanup() {
        let temp = MirTempId(3);
        let spill = projected_temp_spill(temp, 1);
        let value = MirValue::Def(MirDef::VTempByte { id: temp, byte: 1 });
        let read = classify_op(&MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(99),
                offset: 0,
            }),
            src: value.clone(),
            width: MirWidth::Byte,
        });
        assert!(read.reads_spill_byte_compat(spill, 0));
        assert!(!read.reads_spill_byte_compat(spill, 1));

        let write = classify_op(&MirOp::LoadImm {
            dst: MirDef::VTempByte { id: temp, byte: 1 },
            value: 1,
            width: MirWidth::Byte,
        });
        assert!(write.writes_spill_byte_compat(spill, 0));

        let terminator = classify_terminator(&MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        });
        assert!(
            terminator
                .projected_spill_byte_reads
                .contains(&MirSpillByte {
                    id: spill,
                    offset: 0,
                })
        );
    }

    #[test]
    fn terminator_effects_cover_conditions_and_edge_arguments() {
        let edge = |target, value| MirEdge {
            target: MirBlockId(target),
            args: vec![MirEdgeArg {
                value,
                width: MirWidth::Byte,
            }],
        };
        let terminators = vec![
            MirTerminator::Jump(edge(1, temp_value(1))),
            MirTerminator::Branch {
                cond: MirCond::Deferred,
                then_edge: edge(1, temp_value(2)),
                else_edge: edge(2, MirValue::ConstU8(0)),
            },
            MirTerminator::Branch {
                cond: MirCond::BoolValue(temp_value(3)),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::ZSet),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
            MirTerminator::Branch {
                cond: MirCond::AnyFlagTest([MirFlagTest::CSet, MirFlagTest::VSet]),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
            MirTerminator::Branch {
                cond: MirCond::FusedCompare {
                    producer: MirOpRef {
                        block: MirBlockId(0),
                        op_index: 0,
                    },
                    flag_test: MirFlagTest::NClear,
                },
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
            MirTerminator::Return,
            MirTerminator::Exit,
            MirTerminator::Unreachable,
        ];

        for terminator in &terminators {
            let _ = classify_terminator(terminator);
        }
        assert!(
            classify_terminator(&terminators[0])
                .logical
                .temp_uses
                .iter()
                .any(|access| access.temp() == MirTempId(1))
        );
        assert!(classify_terminator(&terminators[3]).consumes_flags_compat);
        let any_flags = classify_terminator(&terminators[4]);
        assert!(any_flags.machine.flag_reads.c);
        assert!(any_flags.machine.flag_reads.v);
        assert!(!any_flags.consumes_flags_compat);
        assert!(classify_terminator(&terminators[5]).consumes_flags_compat);
    }

    #[test]
    fn scaled_y_addressing_exposes_y_and_increment_effects() {
        let consumer = MirAddressConsumer::ScaledIndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xAC),
        });
        let materialize = classify_op(&MirOp::MaterializeIndexedAddress {
            consumer,
            base: MirValue::ConstU16(0x4000),
            index: MirValue::ConstU8(3),
            scale: 2,
        });
        assert!(materialize.writes_reg(MirReg::Y));
        assert!(materialize.may_clobber_reg_compat(MirReg::Y));
        assert!(materialize.addresses.pair_reads.is_empty());
        assert_eq!(materialize.addresses.pair_writes.len(), 2);

        let low = classify_op(&MirOp::LoadIndirect {
            consumer,
            dst: MirDef::Reg(MirReg::A),
            offset: 0,
        });
        assert!(low.reads_reg(MirReg::Y));
        assert!(!low.writes_reg(MirReg::Y));

        let high = classify_op(&MirOp::StoreIndirect {
            consumer,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 1,
        });
        assert!(high.reads_reg(MirReg::Y));
        assert!(high.writes_reg(MirReg::Y));
        assert!(high.machine.flag_writes.z);
        assert!(high.machine.flag_writes.n);
    }

    #[test]
    fn absolute_indexed_accesses_expose_their_index_register_inputs() {
        let load = classify_op(&MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::AbsoluteIndexedX {
                base: MirMem::Absolute(0x4000),
            },
            width: MirWidth::Byte,
        });
        assert!(load.reads_reg(MirReg::X));

        let update = classify_op(&MirOp::UpdateIndexedMem {
            op: MirUpdateOp::Inc,
            base: MirMem::Absolute(0x4000),
        });
        assert!(update.reads_reg(MirReg::X));
        assert!(update.machine.flag_writes.z);
        assert!(update.machine.flag_writes.n);
    }
}
