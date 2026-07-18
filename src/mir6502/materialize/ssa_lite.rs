use super::dead_spills::block_successor_indices;
use super::defs::{op_def, split_def_as_temp};
use super::flags::{op_writes_flags, terminator_consumes_flags};
use super::layout::MaterializeLayout;
use super::peepholes::mem_is_private_scratch;
use super::stats::MirPeepholeStats;
use super::temp_liveness::MirTempLiveSet;
use super::temp_uses::{op_uses_temp, terminator_uses_temp};
use super::values::offset_mem;
use crate::mir6502::MirRoutine;
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirBinaryOp, MirBlockId, MirCallTarget, MirCarryIn, MirCarryOut,
    MirDef, MirMem, MirOp, MirPointerPair, MirReg, MirTempId, MirTerminator, MirValue, MirWidth,
    RoutineId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SsaLiteValueKey {
    ConstU8(u8),
    DirectMem(MirMem),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SsaLiteScanStats {
    pub(super) learned: usize,
    pub(super) killed: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SsaLiteV2ObserveStats {
    pub(super) temp_aliases_learned: usize,
    pub(super) mem_facts_learned: usize,
    pub(super) reg_facts_learned: usize,
    pub(super) facts_killed_by_call: usize,
    pub(super) facts_killed_by_store: usize,
    pub(super) facts_killed_by_barrier: usize,
    pub(super) facts_killed_by_unknown: usize,
    pub(super) replaceable_temp_uses: usize,
    pub(super) replaceable_loads: usize,
    pub(super) copy_prop_candidates: usize,
    pub(super) memory_forward_candidates: usize,
    pub(super) address_facts_learned: usize,
    pub(super) address_reuse_candidates: usize,
}

impl SsaLiteV2ObserveStats {
    fn record_into(self, routine_id: RoutineId, peephole_stats: &mut MirPeepholeStats) {
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-temp-aliases-learned",
            self.temp_aliases_learned,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-mem-facts-learned",
            self.mem_facts_learned,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-reg-facts-learned",
            self.reg_facts_learned,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-facts-killed-call",
            self.facts_killed_by_call,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-facts-killed-store",
            self.facts_killed_by_store,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-facts-killed-barrier",
            self.facts_killed_by_barrier,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-facts-killed-unknown",
            self.facts_killed_by_unknown,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-replaceable-temp-uses",
            self.replaceable_temp_uses,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-replaceable-loads",
            self.replaceable_loads,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-copy-prop-candidates",
            self.copy_prop_candidates,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-memory-forward-candidates",
            self.memory_forward_candidates,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-address-facts-learned",
            self.address_facts_learned,
        );
        peephole_stats.record_many(
            routine_id,
            "ssa-lite-v2-address-reuse-candidates",
            self.address_reuse_candidates,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SsaLiteV2ValueKey {
    ConstU8(u8),
    DirectMem(MirMem),
    Temp(MirDef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MirCopyPropByteValue {
    ConstU8(u8),
    DirectMem(MirMem),
    Temp(MirDef),
}

impl From<MirCopyPropByteValue> for SsaLiteV2ValueKey {
    fn from(value: MirCopyPropByteValue) -> Self {
        match value {
            MirCopyPropByteValue::ConstU8(value) => SsaLiteV2ValueKey::ConstU8(value),
            MirCopyPropByteValue::DirectMem(mem) => SsaLiteV2ValueKey::DirectMem(mem),
            MirCopyPropByteValue::Temp(def) => SsaLiteV2ValueKey::Temp(def),
        }
    }
}

pub(super) fn classify_mir_copy_prop_byte_value(
    value: &MirValue,
    layout: &MaterializeLayout,
) -> Option<MirCopyPropByteValue> {
    match value {
        MirValue::ConstU8(value) => Some(MirCopyPropByteValue::ConstU8(*value)),
        MirValue::ConstU16(value) => u8::try_from(*value).ok().map(MirCopyPropByteValue::ConstU8),
        MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }) => {
            Some(MirCopyPropByteValue::Temp(match value {
                MirValue::Def(def) => def.clone(),
                _ => unreachable!(),
            }))
        }
        MirValue::Def(MirDef::Reg(_)) => None,
        MirValue::PointerCell(mem) if ssa_lite_mem_is_trackable(layout, mem) => {
            Some(MirCopyPropByteValue::DirectMem(mem.clone()))
        }
        MirValue::Word { .. }
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SsaLiteV2AddressKey {
    Address(MirValue),
    Indexed {
        base: MirValue,
        index: MirValue,
        scale: u8,
    },
    Advance {
        index: MirValue,
        scale: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SsaLiteV2AddressFact {
    consumer: MirAddressConsumer,
    key: SsaLiteV2AddressKey,
}

#[derive(Debug, Default, Clone)]
struct SsaLiteV2ObserveEnv {
    a: Option<SsaLiteV2ValueKey>,
    x: Option<SsaLiteV2ValueKey>,
    y: Option<SsaLiteV2ValueKey>,
    temps: Vec<(MirDef, SsaLiteV2ValueKey)>,
    mem: Vec<(MirMem, SsaLiteV2ValueKey)>,
    addresses: Vec<SsaLiteV2AddressFact>,
    stats: SsaLiteV2ObserveStats,
}

#[derive(Debug, Default, Clone)]
pub(super) struct SsaLiteValueEnv {
    pub(super) a: Option<SsaLiteValueKey>,
    pub(super) x: Option<SsaLiteValueKey>,
    pub(super) y: Option<SsaLiteValueKey>,
    pub(super) mem: Vec<(MirMem, SsaLiteValueKey)>,
    pub(super) stats: SsaLiteScanStats,
}

impl SsaLiteValueEnv {
    fn observe_op(&mut self, op: &MirOp, layout: &MaterializeLayout) {
        match op {
            MirOp::Load {
                dst: MirDef::Reg(reg),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            } => {
                self.kill_reg(*reg);
                if ssa_lite_mem_is_trackable(layout, mem) {
                    self.learn_reg(*reg, SsaLiteValueKey::DirectMem(mem.clone()));
                }
            }
            MirOp::LoadImm {
                dst: MirDef::Reg(reg),
                value,
                width: MirWidth::Byte,
            } => {
                self.kill_reg(*reg);
                if let Ok(value) = u8::try_from(*value) {
                    self.learn_reg(*reg, SsaLiteValueKey::ConstU8(value));
                }
            }
            MirOp::Move {
                dst: MirDef::Reg(reg),
                src,
                width: MirWidth::Byte,
            } => {
                let key = self.value_key(src, layout);
                self.kill_reg(*reg);
                if let Some(key) = key {
                    self.learn_reg(*reg, key);
                }
            }
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src,
                width: MirWidth::Byte,
            } => {
                let key = self.value_key(src, layout);
                self.kill_all_mem();
                self.kill_value(&SsaLiteValueKey::DirectMem(mem.clone()));
                if ssa_lite_mem_is_trackable(layout, mem)
                    && let Some(key) = key
                {
                    self.learn_mem(mem.clone(), key);
                }
            }
            MirOp::Store {
                dst: MirAddr::Direct(_),
                width: MirWidth::Word,
                ..
            } => {
                self.kill_memory_dependencies();
            }
            MirOp::Store { .. } => {
                self.kill_memory_dependencies();
            }
            MirOp::UpdateMem { mem, width, .. } => {
                self.kill_mem(mem);
                self.kill_value(&SsaLiteValueKey::DirectMem(mem.clone()));
                if *width == MirWidth::Word {
                    let hi = offset_mem(mem, 1);
                    self.kill_mem(&hi);
                    self.kill_value(&SsaLiteValueKey::DirectMem(hi));
                }
            }
            MirOp::AddByteToWordMem { mem, .. } | MirOp::SubByteFromWordMem { mem, .. } => {
                self.kill_reg(MirReg::A);
                self.kill_mem(mem);
                self.kill_value(&SsaLiteValueKey::DirectMem(mem.clone()));
                let hi = offset_mem(mem, 1);
                self.kill_mem(&hi);
                self.kill_value(&SsaLiteValueKey::DirectMem(hi));
            }
            MirOp::Load { dst, .. }
            | MirOp::LoadImm { dst, .. }
            | MirOp::Move { dst, .. }
            | MirOp::LeaAddr { dst, .. }
            | MirOp::Extend { dst, .. }
            | MirOp::Truncate { dst, .. }
            | MirOp::Unary { dst, .. }
            | MirOp::Binary { dst, .. }
            | MirOp::LoadIndirect { dst, .. } => {
                self.kill_def(dst);
            }
            MirOp::Call { .. } => {
                self.kill_all();
            }
            MirOp::RuntimeHelper { .. } | MirOp::Barrier { .. } | MirOp::MachineBlock { .. } => {
                self.kill_all();
            }
            MirOp::StoreIndirect { .. } | MirOp::IndirectByteCompound { .. } => {
                self.kill_memory_dependencies();
            }
            MirOp::MaterializeAddress { .. }
            | MirOp::MaterializeIndexedAddress { .. }
            | MirOp::AdvanceAddress { .. }
            | MirOp::Compare { .. } => {}
        }
    }

    fn value_key(&self, value: &MirValue, layout: &MaterializeLayout) -> Option<SsaLiteValueKey> {
        match value {
            MirValue::ConstU8(value) => Some(SsaLiteValueKey::ConstU8(*value)),
            MirValue::Def(MirDef::Reg(reg)) => self.reg_fact(*reg).cloned(),
            MirValue::PointerCell(mem) if ssa_lite_mem_is_trackable(layout, mem) => {
                Some(SsaLiteValueKey::DirectMem(mem.clone()))
            }
            _ => None,
        }
    }

    pub(super) fn reg_fact(&self, reg: MirReg) -> Option<&SsaLiteValueKey> {
        match reg {
            MirReg::A => self.a.as_ref(),
            MirReg::X => self.x.as_ref(),
            MirReg::Y => self.y.as_ref(),
        }
    }

    pub(super) fn mem_fact(&self, mem: &MirMem) -> Option<&SsaLiteValueKey> {
        self.mem
            .iter()
            .find_map(|(candidate, value)| (candidate == mem).then_some(value))
    }

    fn set_reg_fact(&mut self, reg: MirReg, value: Option<SsaLiteValueKey>) {
        match reg {
            MirReg::A => self.a = value,
            MirReg::X => self.x = value,
            MirReg::Y => self.y = value,
        }
    }

    fn learn_reg(&mut self, reg: MirReg, key: SsaLiteValueKey) {
        if self.reg_fact(reg) != Some(&key) {
            self.set_reg_fact(reg, Some(key));
            self.stats.learned += 1;
        }
    }

    fn kill_reg(&mut self, reg: MirReg) {
        if self.reg_fact(reg).is_some() {
            self.set_reg_fact(reg, None);
            self.stats.killed += 1;
        }
    }

    fn kill_def(&mut self, def: &MirDef) {
        if let MirDef::Reg(reg) = def {
            self.kill_reg(*reg);
        }
    }

    fn learn_mem(&mut self, mem: MirMem, key: SsaLiteValueKey) {
        if let Some((_mem, value)) = self.mem.iter_mut().find(|(candidate, _)| *candidate == mem) {
            if *value != key {
                *value = key;
                self.stats.learned += 1;
            }
            return;
        }
        self.mem.push((mem, key));
        self.stats.learned += 1;
    }

    fn kill_mem(&mut self, mem: &MirMem) {
        let before = self.mem.len();
        self.mem.retain(|(candidate, _)| candidate != mem);
        self.stats.killed += before.saturating_sub(self.mem.len());
    }

    fn kill_value(&mut self, key: &SsaLiteValueKey) {
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if self.reg_fact(reg) == Some(key) {
                self.kill_reg(reg);
            }
        }
        let before = self.mem.len();
        self.mem.retain(|(_, value)| value != key);
        self.stats.killed += before.saturating_sub(self.mem.len());
    }

    fn kill_all_mem(&mut self) {
        self.stats.killed += self.mem.len();
        self.mem.clear();
    }

    fn kill_memory_dependencies(&mut self) {
        self.kill_all_mem();
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if matches!(self.reg_fact(reg), Some(SsaLiteValueKey::DirectMem(_))) {
                self.kill_reg(reg);
            }
        }
    }

    fn kill_all(&mut self) {
        self.kill_reg(MirReg::A);
        self.kill_reg(MirReg::X);
        self.kill_reg(MirReg::Y);
        self.kill_all_mem();
    }
}

impl SsaLiteV2ObserveEnv {
    fn observe_op(&mut self, op: &MirOp, routine_id: RoutineId, layout: &MaterializeLayout) {
        self.count_rewrite_candidates(op, layout);
        match op {
            MirOp::Load {
                dst,
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            } => {
                self.kill_def(dst, SsaLiteV2KillReason::Unknown);
                if ssa_lite_mem_is_trackable(layout, mem) {
                    self.learn_def(dst.clone(), SsaLiteV2ValueKey::DirectMem(mem.clone()));
                }
            }
            MirOp::LoadImm {
                dst,
                value,
                width: MirWidth::Byte,
            } => {
                self.kill_def(dst, SsaLiteV2KillReason::Unknown);
                if let Ok(value) = u8::try_from(*value) {
                    self.learn_def(dst.clone(), SsaLiteV2ValueKey::ConstU8(value));
                }
            }
            MirOp::Move {
                dst,
                src,
                width: MirWidth::Byte,
            } => {
                let key = self.value_key(src, layout);
                self.kill_def(dst, SsaLiteV2KillReason::Unknown);
                if let Some(key) = key {
                    self.learn_def(dst.clone(), key);
                }
            }
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src,
                width: MirWidth::Byte,
            } => {
                let key = self.value_key(src, layout);
                self.kill_mem_and_dependents(mem, SsaLiteV2KillReason::Store);
                if ssa_lite_mem_is_trackable(layout, mem)
                    && let Some(key) = key
                {
                    self.learn_mem(mem.clone(), key);
                }
            }
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                width: MirWidth::Word,
                ..
            }
            | MirOp::UpdateMem {
                mem,
                width: MirWidth::Word,
                ..
            } => {
                self.kill_mem_and_dependents(mem, SsaLiteV2KillReason::Store);
                self.kill_mem_and_dependents(&offset_mem(mem, 1), SsaLiteV2KillReason::Store);
            }
            MirOp::UpdateMem { mem, .. } => {
                self.kill_mem_and_dependents(mem, SsaLiteV2KillReason::Store);
            }
            MirOp::Store { .. } => {
                self.kill_memory_dependencies(SsaLiteV2KillReason::Unknown);
            }
            MirOp::AddByteToWordMem { mem, .. } | MirOp::SubByteFromWordMem { mem, .. } => {
                self.kill_def(&MirDef::Reg(MirReg::A), SsaLiteV2KillReason::Unknown);
                self.kill_mem_and_dependents(mem, SsaLiteV2KillReason::Store);
                self.kill_mem_and_dependents(&offset_mem(mem, 1), SsaLiteV2KillReason::Store);
            }
            MirOp::Load { dst, .. }
            | MirOp::LoadImm { dst, .. }
            | MirOp::Move { dst, .. }
            | MirOp::LeaAddr { dst, .. }
            | MirOp::Extend { dst, .. }
            | MirOp::Truncate { dst, .. }
            | MirOp::Unary { dst, .. }
            | MirOp::Binary { dst, .. }
            | MirOp::LoadIndirect { dst, .. } => {
                self.kill_def(dst, SsaLiteV2KillReason::Unknown);
            }
            MirOp::Call { .. } | MirOp::RuntimeHelper { .. } => {
                self.kill_all(SsaLiteV2KillReason::Call);
            }
            MirOp::Barrier { .. } | MirOp::MachineBlock { .. } => {
                self.kill_all(SsaLiteV2KillReason::Barrier);
            }
            MirOp::StoreIndirect { .. } | MirOp::IndirectByteCompound { .. } => {
                self.kill_memory_dependencies(SsaLiteV2KillReason::Unknown);
            }
            MirOp::MaterializeAddress { consumer, value } => {
                self.kill_address_consumer_dependencies(
                    *consumer,
                    routine_id,
                    layout,
                    SsaLiteV2KillReason::Store,
                );
                self.learn_address(SsaLiteV2AddressFact {
                    consumer: *consumer,
                    key: SsaLiteV2AddressKey::Address(value.clone()),
                });
            }
            MirOp::MaterializeIndexedAddress {
                consumer,
                base,
                index,
                scale,
            } => {
                self.kill_address_consumer_dependencies(
                    *consumer,
                    routine_id,
                    layout,
                    SsaLiteV2KillReason::Store,
                );
                self.learn_address(SsaLiteV2AddressFact {
                    consumer: *consumer,
                    key: SsaLiteV2AddressKey::Indexed {
                        base: base.clone(),
                        index: index.clone(),
                        scale: *scale,
                    },
                });
            }
            MirOp::AdvanceAddress {
                consumer,
                index,
                scale,
            } => {
                self.kill_address_consumer_dependencies(
                    *consumer,
                    routine_id,
                    layout,
                    SsaLiteV2KillReason::Store,
                );
                self.learn_address(SsaLiteV2AddressFact {
                    consumer: *consumer,
                    key: SsaLiteV2AddressKey::Advance {
                        index: index.clone(),
                        scale: *scale,
                    },
                });
            }
            MirOp::Compare { .. } => {}
        }
    }

    fn count_rewrite_candidates(&mut self, op: &MirOp, layout: &MaterializeLayout) {
        let temp_uses = op_values(op)
            .into_iter()
            .map(|value| self.count_replaceable_temp_uses_in_value(value, layout))
            .sum::<usize>();
        self.stats.replaceable_temp_uses += temp_uses;
        if temp_uses > 0 {
            self.stats.copy_prop_candidates += 1;
        }

        if let MirOp::Load {
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
            ..
        } = op
            && let Some(key) = self.mem_fact(mem)
            && key != &SsaLiteV2ValueKey::DirectMem(mem.clone())
        {
            self.stats.replaceable_loads += 1;
            self.stats.memory_forward_candidates += 1;
        }
    }

    fn count_replaceable_temp_uses_in_value(
        &self,
        value: &MirValue,
        layout: &MaterializeLayout,
    ) -> usize {
        match value {
            MirValue::Def(def) => self
                .def_fact(def)
                .filter(|key| **key != SsaLiteV2ValueKey::Temp(def.clone()))
                .is_some() as usize,
            MirValue::Word { lo, hi } => {
                self.count_replaceable_temp_uses_in_value(lo, layout)
                    + self.count_replaceable_temp_uses_in_value(hi, layout)
            }
            MirValue::PointerCell(mem) => self
                .mem_fact(mem)
                .filter(|key| **key != SsaLiteV2ValueKey::DirectMem(mem.clone()))
                .filter(|_| mem_is_private_scratch(mem) || ssa_lite_mem_is_trackable(layout, mem))
                .is_some() as usize,
            MirValue::ConstU8(_)
            | MirValue::ConstU16(_)
            | MirValue::StaticAddr(_)
            | MirValue::GlobalAddr(_)
            | MirValue::RoutineAddr(_)
            | MirValue::RoutineAddrByte { .. }
            | MirValue::StorageAddrByte { .. } => 0,
        }
    }

    fn value_key(&self, value: &MirValue, layout: &MaterializeLayout) -> Option<SsaLiteV2ValueKey> {
        match value {
            MirValue::Def(MirDef::Reg(reg)) => self.reg_fact(*reg).cloned(),
            MirValue::Def(def) => self.def_fact(def).cloned().or_else(|| {
                classify_mir_copy_prop_byte_value(value, layout).map(SsaLiteV2ValueKey::from)
            }),
            MirValue::PointerCell(mem) => self.mem_fact(mem).cloned().or_else(|| {
                classify_mir_copy_prop_byte_value(value, layout).map(SsaLiteV2ValueKey::from)
            }),
            _ => classify_mir_copy_prop_byte_value(value, layout).map(SsaLiteV2ValueKey::from),
        }
    }

    fn def_fact(&self, def: &MirDef) -> Option<&SsaLiteV2ValueKey> {
        match def {
            MirDef::Reg(reg) => self.reg_fact(*reg),
            MirDef::VTemp(_) | MirDef::VTempByte { .. } => self
                .temps
                .iter()
                .find_map(|(candidate, key)| (candidate == def).then_some(key)),
        }
    }

    fn reg_fact(&self, reg: MirReg) -> Option<&SsaLiteV2ValueKey> {
        match reg {
            MirReg::A => self.a.as_ref(),
            MirReg::X => self.x.as_ref(),
            MirReg::Y => self.y.as_ref(),
        }
    }

    fn mem_fact(&self, mem: &MirMem) -> Option<&SsaLiteV2ValueKey> {
        self.mem
            .iter()
            .find_map(|(candidate, key)| (candidate == mem).then_some(key))
    }

    fn learn_def(&mut self, def: MirDef, key: SsaLiteV2ValueKey) {
        match def {
            MirDef::Reg(reg) => self.learn_reg(reg, key),
            MirDef::VTemp(_) | MirDef::VTempByte { .. } => self.learn_temp(def, key),
        }
    }

    fn learn_reg(&mut self, reg: MirReg, key: SsaLiteV2ValueKey) {
        let slot = match reg {
            MirReg::A => &mut self.a,
            MirReg::X => &mut self.x,
            MirReg::Y => &mut self.y,
        };
        if slot.as_ref() != Some(&key) {
            *slot = Some(key);
            self.stats.reg_facts_learned += 1;
        }
    }

    fn learn_temp(&mut self, def: MirDef, key: SsaLiteV2ValueKey) {
        if let Some((_def, value)) = self
            .temps
            .iter_mut()
            .find(|(candidate, _)| *candidate == def)
        {
            if *value != key {
                *value = key;
                self.stats.temp_aliases_learned += 1;
            }
            return;
        }
        self.temps.push((def, key));
        self.stats.temp_aliases_learned += 1;
    }

    fn learn_mem(&mut self, mem: MirMem, key: SsaLiteV2ValueKey) {
        if let Some((_mem, value)) = self.mem.iter_mut().find(|(candidate, _)| *candidate == mem) {
            if *value != key {
                *value = key;
                self.stats.mem_facts_learned += 1;
            }
            return;
        }
        self.mem.push((mem, key));
        self.stats.mem_facts_learned += 1;
    }

    fn learn_address(&mut self, fact: SsaLiteV2AddressFact) {
        if self.addresses.contains(&fact) {
            self.stats.address_reuse_candidates += 1;
            return;
        }
        self.addresses.push(fact);
        self.stats.address_facts_learned += 1;
    }

    fn kill_def(&mut self, def: &MirDef, reason: SsaLiteV2KillReason) {
        let mut killed = match def {
            MirDef::Reg(reg) => self.kill_reg(*reg),
            MirDef::VTemp(_) | MirDef::VTempByte { .. } => {
                let before = self.temps.len();
                self.temps.retain(|(candidate, _)| candidate != def);
                before.saturating_sub(self.temps.len())
            }
        };
        if matches!(def, MirDef::VTemp(_) | MirDef::VTempByte { .. }) {
            killed += self.kill_value_dependencies(&SsaLiteV2ValueKey::Temp(def.clone()));
        }
        self.record_kills(reason, killed);
    }

    fn kill_reg(&mut self, reg: MirReg) -> usize {
        let slot = match reg {
            MirReg::A => &mut self.a,
            MirReg::X => &mut self.x,
            MirReg::Y => &mut self.y,
        };
        if slot.take().is_some() { 1 } else { 0 }
    }

    fn kill_mem_and_dependents(&mut self, mem: &MirMem, reason: SsaLiteV2KillReason) {
        let key = SsaLiteV2ValueKey::DirectMem(mem.clone());
        let mut killed = self.kill_mem(mem);
        killed += self.kill_value_dependencies(&key);
        self.record_kills(reason, killed);
    }

    fn kill_mem(&mut self, mem: &MirMem) -> usize {
        let before = self.mem.len();
        self.mem.retain(|(candidate, _)| candidate != mem);
        before.saturating_sub(self.mem.len())
    }

    fn kill_value_dependencies(&mut self, key: &SsaLiteV2ValueKey) -> usize {
        let mut killed = 0;
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if self.reg_fact(reg) == Some(key) {
                killed += self.kill_reg(reg);
            }
        }
        let before_temps = self.temps.len();
        self.temps.retain(|(_, value)| value != key);
        killed += before_temps.saturating_sub(self.temps.len());
        let before_mem = self.mem.len();
        self.mem.retain(|(_, value)| value != key);
        killed += before_mem.saturating_sub(self.mem.len());
        killed
    }

    fn kill_memory_dependencies(&mut self, reason: SsaLiteV2KillReason) {
        let mut killed = self.mem.len();
        self.mem.clear();
        killed += self.addresses.len();
        self.addresses.clear();
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if matches!(self.reg_fact(reg), Some(SsaLiteV2ValueKey::DirectMem(_))) {
                killed += self.kill_reg(reg);
            }
        }
        let before_temps = self.temps.len();
        self.temps
            .retain(|(_, value)| !matches!(value, SsaLiteV2ValueKey::DirectMem(_)));
        killed += before_temps.saturating_sub(self.temps.len());
        self.record_kills(reason, killed);
    }

    fn kill_address_consumer_dependencies(
        &mut self,
        consumer: MirAddressConsumer,
        routine_id: RoutineId,
        layout: &MaterializeLayout,
        reason: SsaLiteV2KillReason,
    ) {
        let MirAddressConsumer::IndirectIndexedY(pair) = consumer;
        match pair {
            MirPointerPair::Fixed { lo } => {
                let lo = MirMem::FixedZeroPage(lo);
                self.kill_mem_and_dependents(&lo, reason);
                self.kill_mem_and_dependents(&offset_mem(&lo, 1), reason);
                self.kill_fixed_address_dependencies(lo_address(&lo), routine_id, layout, reason);
                self.kill_fixed_address_dependencies(
                    lo_address(&offset_mem(&lo, 1)),
                    routine_id,
                    layout,
                    reason,
                );
            }
            MirPointerPair::Virtual(slot) => {
                let lo = MirMem::ZeroPage(slot);
                self.kill_mem_and_dependents(&lo, reason);
                self.kill_mem_and_dependents(&offset_mem(&lo, 1), reason);
            }
        }
    }

    fn kill_fixed_address_dependencies(
        &mut self,
        address: u16,
        routine_id: RoutineId,
        layout: &MaterializeLayout,
        reason: SsaLiteV2KillReason,
    ) {
        let mut aliases = Vec::new();
        for (mem, key) in &self.mem {
            if ssa_lite_mem_resolves_to_address(layout, routine_id, mem, address) {
                aliases.push(mem.clone());
            }
            if let SsaLiteV2ValueKey::DirectMem(mem) = key
                && ssa_lite_mem_resolves_to_address(layout, routine_id, mem, address)
            {
                aliases.push(mem.clone());
            }
        }
        for (_def, key) in &self.temps {
            if let SsaLiteV2ValueKey::DirectMem(mem) = key
                && ssa_lite_mem_resolves_to_address(layout, routine_id, mem, address)
            {
                aliases.push(mem.clone());
            }
        }
        for key in [self.a.as_ref(), self.x.as_ref(), self.y.as_ref()]
            .into_iter()
            .flatten()
        {
            if let SsaLiteV2ValueKey::DirectMem(mem) = key
                && ssa_lite_mem_resolves_to_address(layout, routine_id, mem, address)
            {
                aliases.push(mem.clone());
            }
        }

        let mut killed = 0;
        for mem in aliases {
            killed += self.kill_mem(&mem);
            killed += self.kill_value_dependencies(&SsaLiteV2ValueKey::DirectMem(mem));
        }
        self.record_kills(reason, killed);
    }

    fn kill_all(&mut self, reason: SsaLiteV2KillReason) {
        let mut killed = self.kill_reg(MirReg::A);
        killed += self.kill_reg(MirReg::X);
        killed += self.kill_reg(MirReg::Y);
        killed += self.temps.len();
        self.temps.clear();
        killed += self.mem.len();
        self.mem.clear();
        killed += self.addresses.len();
        self.addresses.clear();
        self.record_kills(reason, killed);
    }

    fn record_kills(&mut self, reason: SsaLiteV2KillReason, count: usize) {
        match reason {
            SsaLiteV2KillReason::Call => self.stats.facts_killed_by_call += count,
            SsaLiteV2KillReason::Store => self.stats.facts_killed_by_store += count,
            SsaLiteV2KillReason::Barrier => self.stats.facts_killed_by_barrier += count,
            SsaLiteV2KillReason::Unknown => self.stats.facts_killed_by_unknown += count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SsaLiteV2KillReason {
    Call,
    Store,
    Barrier,
    Unknown,
}

fn op_values(op: &MirOp) -> Vec<&MirValue> {
    match op {
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::AddByteToWordMem { value: src, .. }
        | MirOp::SubByteFromWordMem { value: src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => vec![src],
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => vec![left, right],
        MirOp::MaterializeIndexedAddress { base, index, .. } => vec![base, index],
        MirOp::Call { target, args, .. } => {
            let mut values = Vec::new();
            if let crate::mir6502::ir::MirCallTarget::Indirect { target, .. } = target {
                values.push(target);
            }
            values.extend(args.iter().map(|arg| &arg.value));
            values
        }
        MirOp::Load {
            src: MirAddr::ComputedIndex { base, index, .. },
            ..
        } => vec![base, index],
        MirOp::Load {
            src: MirAddr::PointerIndex { index, .. },
            ..
        } => vec![index],
        MirOp::Load {
            src: MirAddr::Deref { ptr, .. },
            ..
        } => vec![ptr],
        MirOp::Store {
            dst: MirAddr::ComputedIndex { base, index, .. },
            src,
            ..
        } => vec![base, index, src],
        MirOp::Store {
            dst: MirAddr::PointerIndex { index, .. },
            src,
            ..
        } => vec![index, src],
        MirOp::Store {
            dst: MirAddr::Deref { ptr, .. },
            src,
            ..
        } => vec![ptr, src],
        MirOp::Store { src, .. } => vec![src],
        MirOp::LoadImm { .. }
        | MirOp::Load { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => Vec::new(),
    }
}

pub(super) fn record_ssa_lite_block_facts(
    ops: &[MirOp],
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) {
    let stats = scan_ssa_lite_block(ops, layout);
    peephole_stats.record_many(routine_id, "ssa-lite-facts-learned", stats.learned);
    peephole_stats.record_many(routine_id, "ssa-lite-facts-killed", stats.killed);
}

pub(super) fn record_ssa_lite_v2_observability(
    ops: &[MirOp],
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) {
    scan_ssa_lite_v2_observability(ops, routine_id, layout).record_into(routine_id, peephole_stats);
}

#[cfg(test)]
pub(super) fn fold_mir_copy_prop_const_uses(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    fold_mir_copy_prop_const_uses_inner(ops, None, None, None, routine_id, layout, peephole_stats)
}

#[cfg(test)]
pub(super) fn fold_mir_copy_prop_const_uses_with_terminator(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    fold_mir_copy_prop_const_uses_inner(
        ops,
        Some(terminator),
        None,
        None,
        routine_id,
        layout,
        peephole_stats,
    )
}

pub(super) fn fold_mir_copy_prop_const_uses_with_terminator_and_live_out(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    live_out: &MirTempLiveSet,
    block_id: MirBlockId,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    fold_mir_copy_prop_const_uses_inner(
        ops,
        Some(terminator),
        Some(live_out),
        Some(block_id),
        routine_id,
        layout,
        peephole_stats,
    )
}

fn fold_mir_copy_prop_const_uses_inner(
    ops: Vec<MirOp>,
    terminator: Option<&MirTerminator>,
    live_out: Option<&MirTempLiveSet>,
    block_id: Option<MirBlockId>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut env = SsaLiteV2ObserveEnv::default();
    let mut rewritten_uses = MirCopyPropRewriteCounts::default();
    let ops = ops
        .into_iter()
        .map(|op| {
            let (rewritten, count) = rewrite_mir_copy_prop_const_op(op, &env);
            rewritten_uses += count;
            env.observe_op(&rewritten, routine_id, layout);
            rewritten
        })
        .collect::<Vec<_>>();
    let dead_temp_byte_candidates = terminator
        .map(|terminator| collect_dead_copy_prop_temp_byte_def_candidates(&ops, terminator, layout))
        .unwrap_or_default();
    let dead_temp_byte_lane_safety = terminator
        .map(|terminator| {
            classify_dead_copy_prop_temp_byte_lane_safety(
                &ops,
                terminator,
                live_out,
                block_id,
                routine_id,
                layout,
                peephole_stats,
            )
        })
        .unwrap_or_default();
    let (ops, dead_temp_defs, dead_temp_successor_live_blocks) =
        if let Some(terminator) = terminator {
            remove_dead_copy_prop_temp_defs(
                ops,
                terminator,
                live_out,
                block_id,
                routine_id,
                layout,
                peephole_stats,
            )
        } else {
            (ops, 0, 0)
        };
    let (ops, dead_temp_byte_defs, successor_live_blocks) = if let Some(terminator) = terminator {
        remove_dead_copy_prop_temp_byte_defs(
            ops,
            terminator,
            live_out,
            block_id,
            routine_id,
            layout,
            peephole_stats,
        )
    } else {
        (ops, DeadTempByteDefCandidateStats::default(), 0)
    };
    peephole_stats.record_many(routine_id, "mir-copy-prop-const-uses", rewritten_uses.total);
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-direct-mem-uses",
        rewritten_uses.direct_mem,
    );
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-temp-alias-uses",
        rewritten_uses.temp_alias,
    );
    peephole_stats.record_many(routine_id, "mir-copy-prop-dead-temp-defs", dead_temp_defs);
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-dead-temp-def-blocked-successor-live",
        dead_temp_successor_live_blocks,
    );
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-dead-temp-byte-load-direct-defs",
        dead_temp_byte_defs.load_direct,
    );
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-dead-temp-byte-load-imm-defs",
        dead_temp_byte_defs.load_imm,
    );
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-dead-temp-byte-def-blocked-successor-live",
        successor_live_blocks,
    );
    peephole_stats.record_many(
        routine_id,
        "mir-copy-prop-dead-temp-byte-def-candidates",
        dead_temp_byte_candidates.total(),
    );
    dead_temp_byte_candidates.record_into(routine_id, peephole_stats);
    dead_temp_byte_lane_safety.record_into(routine_id, peephole_stats);
    ops
}

fn remove_dead_copy_prop_temp_defs(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
    block_id: Option<MirBlockId>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> (Vec<MirOp>, usize, usize) {
    let mut temp_defs = Vec::new();
    for op in &ops {
        if let Some(temp) = op_def(op).and_then(split_def_as_temp)
            && !temp_defs.contains(&temp)
        {
            temp_defs.push(temp);
        }
    }
    if temp_defs.is_empty() {
        return (ops, 0, 0);
    }

    let mut live = Vec::new();
    if let Some(live_out) = live_out {
        for id in live_out.full_temps() {
            if !live.contains(&id) {
                live.push(id);
            }
        }
        for (id, _byte) in live_out.exact_lanes() {
            if !live.contains(&id) {
                live.push(id);
            }
        }
    }
    for temp in &temp_defs {
        if terminator_uses_temp(terminator, *temp) {
            live.push(*temp);
        }
    }

    let mut removed = 0;
    let mut successor_live_blocks = 0;
    let mut kept = Vec::with_capacity(ops.len());
    for (op_index, op) in ops.into_iter().enumerate().rev() {
        let dst_temp = op_def(&op).and_then(split_def_as_temp);
        if let Some(temp) = dst_temp
            && op_is_dead_copy_prop_temp_def(&op, layout)
            && live_out.is_some_and(|live_out| temp_live_set_blocks_full_temp(live_out, temp))
        {
            successor_live_blocks += 1;
            peephole_stats.record_site(
                routine_id,
                "mir-copy-prop-dead-temp-def-blocked-successor-live",
                full_temp_site_detail(block_id, op_index, temp, &op),
            );
        }
        let dst_is_live = dst_temp.is_some_and(|temp| live.contains(&temp));
        if let Some(_temp) = dst_temp
            && !dst_is_live
            && op_is_dead_copy_prop_temp_def(&op, layout)
        {
            removed += 1;
            continue;
        }

        if let Some(temp) = dst_temp {
            live.retain(|candidate| *candidate != temp);
        }
        for temp in &temp_defs {
            if op_uses_temp(&op, *temp) && !live.contains(temp) {
                live.push(*temp);
            }
        }
        kept.push(op);
    }
    kept.reverse();
    (kept, removed, successor_live_blocks)
}

fn remove_dead_copy_prop_temp_byte_defs(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
    block_id: Option<MirBlockId>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> (Vec<MirOp>, DeadTempByteDefCandidateStats, usize) {
    let mut live = LiveTempByteLanes::default();
    if let Some(live_out) = live_out {
        live.observe_live_out(live_out);
    }
    live.observe_terminator(terminator);

    let mut removed = DeadTempByteDefCandidateStats::default();
    let mut successor_live_blocks = 0;
    let mut kept = Vec::with_capacity(ops.len());
    for (op_index, op) in ops.into_iter().enumerate().rev() {
        let lane = op_def_as_temp_byte_lane(&op);
        if let Some((id, byte)) = lane
            && op_is_removable_dead_copy_prop_temp_byte_def(&op, layout)
            && live_out.is_some_and(|live_out| temp_byte_live_set_blocks_lane(live_out, id, byte))
        {
            successor_live_blocks += 1;
            peephole_stats.record_site(
                routine_id,
                "mir-copy-prop-dead-temp-byte-def-blocked-successor-live",
                temp_byte_site_detail(block_id, op_index, id, byte, &op),
            );
        }
        let can_remove = lane.is_some_and(|(id, byte)| {
            op_is_removable_dead_copy_prop_temp_byte_def(&op, layout)
                && !live.exact_lane_live(id, byte)
                && !live.full_temp_live(id)
                && !live.sibling_lane_live(id, byte)
        });
        if can_remove {
            removed.record_op(&op, layout);
            continue;
        }

        if let Some((id, byte)) = lane {
            live.kill_exact_lane(id, byte);
        }
        live.observe_op_uses(&op);
        kept.push(op);
    }
    kept.reverse();
    (kept, removed, successor_live_blocks)
}

fn temp_byte_live_set_blocks_lane(live_out: &MirTempLiveSet, id: MirTempId, byte: u8) -> bool {
    live_out.exact_lane_live(id, byte)
        || live_out.full_temp_live(id)
        || live_out.exact_lane_live(id, byte ^ 1)
}

fn temp_live_set_blocks_full_temp(live_out: &MirTempLiveSet, id: MirTempId) -> bool {
    live_out.full_temp_live(id)
        || live_out.exact_lane_live(id, 0)
        || live_out.exact_lane_live(id, 1)
}

fn full_temp_site_detail(
    block_id: Option<MirBlockId>,
    op_index: usize,
    id: MirTempId,
    op: &MirOp,
) -> String {
    let block = block_id
        .map(|id| format!("b{}", id.0))
        .unwrap_or_else(|| "b?".to_string());
    format!(
        "block={block} op=#{op_index} temp=t{} kind={}",
        id.0,
        dead_temp_byte_op_kind(op)
    )
}

fn temp_byte_site_detail(
    block_id: Option<MirBlockId>,
    op_index: usize,
    id: MirTempId,
    byte: u8,
    op: &MirOp,
) -> String {
    let block = block_id
        .map(|id| format!("b{}", id.0))
        .unwrap_or_else(|| "b?".to_string());
    format!(
        "block={block} op=#{op_index} temp=t{}.{} kind={}",
        id.0,
        byte,
        dead_temp_byte_op_kind(op)
    )
}

fn record_temp_byte_candidate_site(
    peephole_stats: &mut MirPeepholeStats,
    routine_id: RoutineId,
    rule: &'static str,
    block_id: Option<MirBlockId>,
    op_index: usize,
    id: MirTempId,
    byte: u8,
    op: &MirOp,
    reason: &'static str,
) {
    peephole_stats.record_site(
        routine_id,
        rule,
        format!(
            "{} reason={reason}",
            temp_byte_site_detail(block_id, op_index, id, byte, op)
        ),
    );
}

fn dead_temp_byte_op_kind(op: &MirOp) -> &'static str {
    match op {
        MirOp::LoadImm { .. } => "load-imm",
        MirOp::Load {
            src: MirAddr::Direct(_),
            ..
        } => "load-direct",
        MirOp::Load { .. } => "load",
        MirOp::Move { .. } => "move",
        MirOp::LeaAddr { .. } => "lea-addr",
        MirOp::Extend { .. } => "extend",
        MirOp::Truncate { .. } => "truncate",
        MirOp::Unary { .. } => "unary",
        MirOp::Binary { .. } => "binary",
        _ => "other",
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DeadTempByteDefCandidateStats {
    load_imm: usize,
    load_direct: usize,
    move_: usize,
    lea_addr: usize,
    extend: usize,
    truncate: usize,
    unary: usize,
    binary: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DeadTempByteLaneSafetyStats {
    safe: DeadTempByteDefCandidateStats,
    safe_byte0: usize,
    safe_byte1: usize,
    blocked_exact_lane_live: usize,
    blocked_full_temp_live: usize,
    blocked_sibling_lane_live: usize,
    blocked_byte0: usize,
    blocked_byte1: usize,
    binary_reasons: DeadTempByteBinaryCandidateReasonStats,
}

impl DeadTempByteLaneSafetyStats {
    fn record_into(self, routine_id: RoutineId, peephole_stats: &mut MirPeepholeStats) {
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-candidates",
            self.safe.total(),
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-byte0-candidates",
            self.safe_byte0,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-byte1-candidates",
            self.safe_byte1,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-blocked-exact-lane-live",
            self.blocked_exact_lane_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-blocked-full-temp-live",
            self.blocked_full_temp_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-blocked-sibling-lane-live",
            self.blocked_sibling_lane_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-blocked-byte0",
            self.blocked_byte0,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-blocked-byte1",
            self.blocked_byte1,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-load-imm-candidates",
            self.safe.load_imm,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-load-direct-candidates",
            self.safe.load_direct,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-move-candidates",
            self.safe.move_,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lane-safe-binary-candidates",
            self.safe.binary,
        );
        self.binary_reasons.record_into(routine_id, peephole_stats);
    }

    fn record_safe_op(&mut self, op: &MirOp, byte: u8, layout: &MaterializeLayout) {
        self.safe.record_op(op, layout);
        match byte {
            0 => self.safe_byte0 += 1,
            1 => self.safe_byte1 += 1,
            _ => {}
        }
    }

    fn record_blocked_exact_lane_live(&mut self, byte: u8) {
        self.blocked_exact_lane_live += 1;
        self.record_blocked_byte(byte);
    }

    fn record_blocked_full_temp_live(&mut self, byte: u8) {
        self.blocked_full_temp_live += 1;
        self.record_blocked_byte(byte);
    }

    fn record_blocked_sibling_lane_live(&mut self, byte: u8) {
        self.blocked_sibling_lane_live += 1;
        self.record_blocked_byte(byte);
    }

    fn record_blocked_byte(&mut self, byte: u8) {
        match byte {
            0 => self.blocked_byte0 += 1,
            1 => self.blocked_byte1 += 1,
            _ => {}
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DeadTempByteBinaryCandidateReasonStats {
    blocked_successor_live: usize,
    blocked_exact_lane_live: usize,
    blocked_full_temp_live: usize,
    blocked_sibling_lane_live: usize,
    blocked_carry_out: usize,
    blocked_carry_from_previous: usize,
    lane_safe: usize,
}

impl DeadTempByteBinaryCandidateReasonStats {
    fn record_into(self, routine_id: RoutineId, peephole_stats: &mut MirPeepholeStats) {
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-blocked-successor-live",
            self.blocked_successor_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-blocked-exact-lane-live",
            self.blocked_exact_lane_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-blocked-full-temp-live",
            self.blocked_full_temp_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-blocked-sibling-lane-live",
            self.blocked_sibling_lane_live,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-blocked-carry-out",
            self.blocked_carry_out,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-blocked-carry-from-previous",
            self.blocked_carry_from_previous,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-lane-safe",
            self.lane_safe,
        );
    }

    fn record(&mut self, reason: &'static str) {
        match reason {
            "blocked-successor-live" => self.blocked_successor_live += 1,
            "blocked-exact-lane-live" => self.blocked_exact_lane_live += 1,
            "blocked-full-temp-live" => self.blocked_full_temp_live += 1,
            "blocked-sibling-lane-live" => self.blocked_sibling_lane_live += 1,
            "blocked-carry-out" => self.blocked_carry_out += 1,
            "blocked-carry-from-previous" => self.blocked_carry_from_previous += 1,
            "lane-safe" => self.lane_safe += 1,
            _ => {}
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LiveTempByteLanes {
    exact: Vec<(MirTempId, u8)>,
    full: Vec<MirTempId>,
}

impl LiveTempByteLanes {
    fn observe_live_out(&mut self, live_out: &MirTempLiveSet) {
        for (id, byte) in live_out.exact_lanes() {
            self.insert_exact(id, byte);
        }
        for id in live_out.full_temps() {
            self.insert_full(id);
        }
    }

    fn observe_terminator(&mut self, terminator: &MirTerminator) {
        if let MirTerminator::Branch {
            cond: crate::mir6502::ir::MirCond::BoolValue(value),
            ..
        } = terminator
        {
            self.observe_value(value);
        }
    }

    fn observe_op_uses(&mut self, op: &MirOp) {
        match op {
            MirOp::Load { src, .. } => self.observe_addr(src),
            MirOp::Store { dst, src, .. } => {
                self.observe_addr(dst);
                self.observe_value(src);
            }
            MirOp::Move { src, .. }
            | MirOp::Extend { src, .. }
            | MirOp::Truncate { src, .. }
            | MirOp::Unary { src, .. }
            | MirOp::AddByteToWordMem { value: src, .. }
            | MirOp::SubByteFromWordMem { value: src, .. }
            | MirOp::MaterializeAddress { value: src, .. }
            | MirOp::AdvanceAddress { index: src, .. }
            | MirOp::StoreIndirect { src, .. } => self.observe_value(src),
            MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
                self.observe_value(left);
                self.observe_value(right);
            }
            MirOp::MaterializeIndexedAddress { base, index, .. } => {
                self.observe_value(base);
                self.observe_value(index);
            }
            MirOp::Call { target, args, .. } => {
                if let MirCallTarget::Indirect { target, .. } = target {
                    self.observe_value(target);
                }
                for arg in args {
                    self.observe_value(&arg.value);
                }
            }
            MirOp::LoadImm { .. }
            | MirOp::RuntimeHelper { .. }
            | MirOp::LoadIndirect { .. }
            | MirOp::IndirectByteCompound { .. }
            | MirOp::Barrier { .. }
            | MirOp::LeaAddr { .. }
            | MirOp::UpdateMem { .. }
            | MirOp::MachineBlock { .. } => {}
        }
    }

    fn observe_addr(&mut self, addr: &MirAddr) {
        match addr {
            MirAddr::ComputedIndex { base, index, .. } => {
                self.observe_value(base);
                self.observe_value(index);
            }
            MirAddr::PointerIndex { index, .. } => self.observe_value(index),
            MirAddr::Deref { ptr, .. } => self.observe_value(ptr),
            MirAddr::Direct(_)
            | MirAddr::Label(_)
            | MirAddr::ZeroPageIndexedX { .. }
            | MirAddr::AbsoluteIndexedX { .. }
            | MirAddr::AbsoluteIndexedY { .. }
            | MirAddr::IndirectIndexedY { .. }
            | MirAddr::FixedIndirectIndexedY { .. }
            | MirAddr::PointerCell { .. } => {}
        }
    }

    fn observe_value(&mut self, value: &MirValue) {
        match value {
            MirValue::Def(MirDef::VTemp(id)) => self.insert_full(*id),
            MirValue::Def(MirDef::VTempByte { id, byte }) => self.insert_exact(*id, *byte),
            MirValue::Word { lo, hi } => {
                self.observe_value(lo);
                self.observe_value(hi);
            }
            MirValue::ConstU8(_)
            | MirValue::ConstU16(_)
            | MirValue::Def(MirDef::Reg(_))
            | MirValue::StaticAddr(_)
            | MirValue::GlobalAddr(_)
            | MirValue::RoutineAddr(_)
            | MirValue::RoutineAddrByte { .. }
            | MirValue::StorageAddrByte { .. }
            | MirValue::PointerCell(_) => {}
        }
    }

    fn insert_exact(&mut self, id: MirTempId, byte: u8) {
        if !self.exact.contains(&(id, byte)) {
            self.exact.push((id, byte));
        }
    }

    fn insert_full(&mut self, id: MirTempId) {
        if !self.full.contains(&id) {
            self.full.push(id);
        }
    }

    fn exact_lane_live(&self, id: MirTempId, byte: u8) -> bool {
        self.exact.contains(&(id, byte))
    }

    fn full_temp_live(&self, id: MirTempId) -> bool {
        self.full.contains(&id)
    }

    fn sibling_lane_live(&self, id: MirTempId, byte: u8) -> bool {
        let sibling = byte ^ 1;
        self.exact.contains(&(id, sibling))
    }

    fn kill_exact_lane(&mut self, id: MirTempId, byte: u8) {
        self.exact.retain(|(candidate_id, candidate_byte)| {
            *candidate_id != id || *candidate_byte != byte
        });
    }
}

impl DeadTempByteDefCandidateStats {
    fn total(self) -> usize {
        self.load_imm
            + self.load_direct
            + self.move_
            + self.lea_addr
            + self.extend
            + self.truncate
            + self.unary
            + self.binary
    }

    fn record_into(self, routine_id: RoutineId, peephole_stats: &mut MirPeepholeStats) {
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-load-imm-candidates",
            self.load_imm,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-load-direct-candidates",
            self.load_direct,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-move-candidates",
            self.move_,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-lea-addr-candidates",
            self.lea_addr,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-extend-candidates",
            self.extend,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-truncate-candidates",
            self.truncate,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-unary-candidates",
            self.unary,
        );
        peephole_stats.record_many(
            routine_id,
            "mir-copy-prop-dead-temp-byte-def-binary-candidates",
            self.binary,
        );
    }

    fn record_op(&mut self, op: &MirOp, layout: &MaterializeLayout) {
        match op {
            MirOp::LoadImm {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.load_imm += 1,
            MirOp::Load {
                dst: MirDef::VTempByte { .. },
                src: MirAddr::Direct(mem),
                ..
            } if ssa_lite_mem_is_trackable(layout, mem) => self.load_direct += 1,
            MirOp::Move {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.move_ += 1,
            MirOp::LeaAddr {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.lea_addr += 1,
            MirOp::Extend {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.extend += 1,
            MirOp::Truncate {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.truncate += 1,
            MirOp::Unary {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.unary += 1,
            MirOp::Binary {
                dst: MirDef::VTempByte { .. },
                ..
            } => self.binary += 1,
            _ => {}
        }
    }
}

fn collect_dead_copy_prop_temp_byte_def_candidates(
    ops: &[MirOp],
    terminator: &MirTerminator,
    layout: &MaterializeLayout,
) -> DeadTempByteDefCandidateStats {
    let mut temp_defs = Vec::new();
    for op in ops {
        if let Some(temp) = op_def_as_temp_byte_id(op)
            && !temp_defs.contains(&temp)
        {
            temp_defs.push(temp);
        }
    }
    if temp_defs.is_empty() {
        return DeadTempByteDefCandidateStats::default();
    }

    let mut live = Vec::new();
    for temp in &temp_defs {
        if terminator_uses_temp(terminator, *temp) {
            live.push(*temp);
        }
    }

    let mut candidates = DeadTempByteDefCandidateStats::default();
    for op in ops.iter().rev() {
        let dst_temp = op_def_as_temp_byte_id(op);
        let dst_is_live = dst_temp.is_some_and(|temp| live.contains(&temp));
        if dst_temp.is_some() && !dst_is_live && op_is_dead_copy_prop_temp_byte_def(op, layout) {
            candidates.record_op(op, layout);
        }

        if let Some(temp) = dst_temp {
            live.retain(|candidate| *candidate != temp);
        }
        for temp in &temp_defs {
            if op_uses_temp(op, *temp) && !live.contains(temp) {
                live.push(*temp);
            }
        }
    }
    candidates
}

fn classify_dead_copy_prop_temp_byte_lane_safety(
    ops: &[MirOp],
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
    block_id: Option<MirBlockId>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> DeadTempByteLaneSafetyStats {
    let mut live = LiveTempByteLanes::default();
    if let Some(live_out) = live_out {
        live.observe_live_out(live_out);
    }
    live.observe_terminator(terminator);

    let mut stats = DeadTempByteLaneSafetyStats::default();
    for (op_index, op) in ops.iter().enumerate().rev() {
        if let Some((id, byte)) = op_def_as_temp_byte_lane(op)
            && op_is_dead_copy_prop_temp_byte_def(op, layout)
        {
            let binary_site = matches!(
                op,
                MirOp::Binary {
                    dst: MirDef::VTempByte { .. },
                    ..
                }
            );
            let move_site = matches!(
                op,
                MirOp::Move {
                    dst: MirDef::VTempByte { .. },
                    ..
                }
            );
            if binary_site {
                let reason = temp_byte_binary_candidate_reason(op, live_out, &live, id, byte);
                stats.binary_reasons.record(reason);
                record_temp_byte_candidate_site(
                    peephole_stats,
                    routine_id,
                    "mir-copy-prop-dead-temp-byte-def-binary-candidate",
                    block_id,
                    op_index,
                    id,
                    byte,
                    op,
                    reason,
                );
            }
            if live.exact_lane_live(id, byte) {
                stats.record_blocked_exact_lane_live(byte);
                if move_site {
                    record_temp_byte_candidate_site(
                        peephole_stats,
                        routine_id,
                        "mir-copy-prop-dead-temp-byte-def-move-candidate",
                        block_id,
                        op_index,
                        id,
                        byte,
                        op,
                        "blocked-exact-lane-live",
                    );
                }
            } else if live.full_temp_live(id) {
                stats.record_blocked_full_temp_live(byte);
                if move_site {
                    record_temp_byte_candidate_site(
                        peephole_stats,
                        routine_id,
                        "mir-copy-prop-dead-temp-byte-def-move-candidate",
                        block_id,
                        op_index,
                        id,
                        byte,
                        op,
                        "blocked-full-temp-live",
                    );
                }
            } else if live.sibling_lane_live(id, byte) {
                stats.record_blocked_sibling_lane_live(byte);
                if move_site {
                    record_temp_byte_candidate_site(
                        peephole_stats,
                        routine_id,
                        "mir-copy-prop-dead-temp-byte-def-move-candidate",
                        block_id,
                        op_index,
                        id,
                        byte,
                        op,
                        "blocked-sibling-lane-live",
                    );
                }
            } else {
                stats.record_safe_op(op, byte, layout);
                peephole_stats.record_site(
                    routine_id,
                    "mir-copy-prop-dead-temp-byte-def-lane-safe-candidate",
                    temp_byte_site_detail(block_id, op_index, id, byte, op),
                );
                if move_site {
                    record_temp_byte_candidate_site(
                        peephole_stats,
                        routine_id,
                        "mir-copy-prop-dead-temp-byte-def-move-candidate",
                        block_id,
                        op_index,
                        id,
                        byte,
                        op,
                        "lane-safe",
                    );
                }
            }
        }

        if let Some((id, byte)) = op_def_as_temp_byte_lane(op) {
            live.kill_exact_lane(id, byte);
        }
        live.observe_op_uses(op);
    }
    stats
}

fn temp_byte_binary_candidate_reason(
    op: &MirOp,
    live_out: Option<&MirTempLiveSet>,
    live: &LiveTempByteLanes,
    id: MirTempId,
    byte: u8,
) -> &'static str {
    if live_out.is_some_and(|live_out| temp_byte_live_set_blocks_lane(live_out, id, byte)) {
        return "blocked-successor-live";
    }
    if live.exact_lane_live(id, byte) {
        return "blocked-exact-lane-live";
    }
    if live.full_temp_live(id) {
        return "blocked-full-temp-live";
    }
    if live.sibling_lane_live(id, byte) {
        return "blocked-sibling-lane-live";
    }
    match op {
        MirOp::Binary {
            carry_in: Some(MirCarryIn::FromPrevious),
            ..
        } => "blocked-carry-from-previous",
        MirOp::Binary { carry_out, .. } if !matches!(carry_out, MirCarryOut::Ignore) => {
            "blocked-carry-out"
        }
        _ => "lane-safe",
    }
}

#[cfg(test)]
pub(super) fn temp_byte_binary_candidate_reason_for_test(
    op: &MirOp,
    id: MirTempId,
    byte: u8,
    successor_live: bool,
    exact_lane_live: bool,
    full_temp_live: bool,
    sibling_lane_live: bool,
) -> &'static str {
    let live_out = successor_live.then(|| MirTempLiveSet::with_exact_lane(id, byte));
    let mut live = LiveTempByteLanes::default();
    if exact_lane_live {
        live.insert_exact(id, byte);
    }
    if full_temp_live {
        live.insert_full(id);
    }
    if sibling_lane_live {
        live.insert_exact(id, byte ^ 1);
    }
    temp_byte_binary_candidate_reason(op, live_out.as_ref(), &live, id, byte)
}

fn op_def_as_temp_byte_id(op: &MirOp) -> Option<crate::mir6502::ir::MirTempId> {
    match op_def(op) {
        Some(MirDef::VTempByte { id, .. }) => Some(*id),
        _ => None,
    }
}

fn op_def_as_temp_byte_lane(op: &MirOp) -> Option<(MirTempId, u8)> {
    match op_def(op) {
        Some(MirDef::VTempByte { id, byte }) => Some((*id, *byte)),
        _ => None,
    }
}

fn op_is_dead_copy_prop_temp_def(op: &MirOp, layout: &MaterializeLayout) -> bool {
    match op {
        MirOp::LoadImm {
            dst: MirDef::VTemp(_),
            ..
        }
        | MirOp::Move {
            dst: MirDef::VTemp(_),
            ..
        }
        | MirOp::LeaAddr {
            dst: MirDef::VTemp(_),
            ..
        }
        | MirOp::Extend {
            dst: MirDef::VTemp(_),
            ..
        }
        | MirOp::Truncate {
            dst: MirDef::VTemp(_),
            ..
        }
        | MirOp::Unary {
            dst: MirDef::VTemp(_),
            ..
        }
        | MirOp::Binary {
            dst: MirDef::VTemp(_),
            ..
        } => true,
        MirOp::Load {
            dst: MirDef::VTemp(_),
            src: MirAddr::Direct(mem),
            ..
        } => ssa_lite_mem_is_trackable(layout, mem),
        _ => false,
    }
}

fn op_is_removable_dead_copy_prop_temp_byte_def(op: &MirOp, layout: &MaterializeLayout) -> bool {
    match op {
        MirOp::LoadImm {
            dst: MirDef::VTempByte { .. },
            ..
        } => true,
        MirOp::Load {
            dst: MirDef::VTempByte { .. },
            src: MirAddr::Direct(mem),
            ..
        } => ssa_lite_mem_is_trackable(layout, mem),
        MirOp::Move {
            dst: MirDef::VTempByte { .. },
            ..
        } => true,
        MirOp::Binary {
            dst: MirDef::VTempByte { .. },
            carry_in,
            carry_out,
            ..
        } => byte_binary_def_is_removable(*carry_in, *carry_out),
        _ => false,
    }
}

fn byte_binary_def_is_removable(carry_in: Option<MirCarryIn>, carry_out: MirCarryOut) -> bool {
    !matches!(carry_in, Some(MirCarryIn::FromPrevious)) && matches!(carry_out, MirCarryOut::Ignore)
}

fn op_is_dead_copy_prop_temp_byte_def(op: &MirOp, layout: &MaterializeLayout) -> bool {
    match op {
        MirOp::LoadImm {
            dst: MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Move {
            dst: MirDef::VTempByte { .. },
            ..
        }
        | MirOp::LeaAddr {
            dst: MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Extend {
            dst: MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Truncate {
            dst: MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Unary {
            dst: MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Binary {
            dst: MirDef::VTempByte { .. },
            ..
        } => true,
        MirOp::Load {
            dst: MirDef::VTempByte { .. },
            src: MirAddr::Direct(mem),
            ..
        } => ssa_lite_mem_is_trackable(layout, mem),
        _ => false,
    }
}

pub(super) fn scan_ssa_lite_v2_observability(
    ops: &[MirOp],
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> SsaLiteV2ObserveStats {
    let mut env = SsaLiteV2ObserveEnv::default();
    for op in ops {
        env.observe_op(op, routine_id, layout);
    }
    env.stats
}

fn scan_ssa_lite_block(ops: &[MirOp], layout: &MaterializeLayout) -> SsaLiteScanStats {
    scan_ssa_lite_block_env(ops, layout).stats
}

pub(super) fn scan_ssa_lite_block_env(
    ops: &[MirOp],
    layout: &MaterializeLayout,
) -> SsaLiteValueEnv {
    scan_ssa_lite_block_env_from(ops, layout, SsaLiteValueEnv::default())
}

fn scan_ssa_lite_block_env_from(
    ops: &[MirOp],
    layout: &MaterializeLayout,
    initial: SsaLiteValueEnv,
) -> SsaLiteValueEnv {
    let mut env = initial;
    for op in ops {
        env.observe_op(op, layout);
    }
    env
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct MirCopyPropRewriteCounts {
    total: usize,
    direct_mem: usize,
    temp_alias: usize,
}

impl MirCopyPropRewriteCounts {
    fn one_const() -> Self {
        Self {
            total: 1,
            direct_mem: 0,
            temp_alias: 0,
        }
    }

    fn one_direct_mem() -> Self {
        Self {
            total: 1,
            direct_mem: 1,
            temp_alias: 0,
        }
    }

    fn one_temp_alias() -> Self {
        Self {
            total: 1,
            direct_mem: 0,
            temp_alias: 1,
        }
    }
}

impl std::ops::Add for MirCopyPropRewriteCounts {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            total: self.total + rhs.total,
            direct_mem: self.direct_mem + rhs.direct_mem,
            temp_alias: self.temp_alias + rhs.temp_alias,
        }
    }
}

impl std::ops::AddAssign for MirCopyPropRewriteCounts {
    fn add_assign(&mut self, rhs: Self) {
        self.total += rhs.total;
        self.direct_mem += rhs.direct_mem;
        self.temp_alias += rhs.temp_alias;
    }
}

fn rewrite_mir_copy_prop_const_op(
    op: MirOp,
    env: &SsaLiteV2ObserveEnv,
) -> (MirOp, MirCopyPropRewriteCounts) {
    match op {
        MirOp::Compare {
            dst,
            op,
            left,
            right,
            width: MirWidth::Byte,
            signed,
        } => {
            let (left, left_count) = rewrite_mir_copy_prop_byte_value(left, env);
            let (right, right_count) = rewrite_mir_copy_prop_byte_value(right, env);
            (
                MirOp::Compare {
                    dst,
                    op,
                    left,
                    right,
                    width: MirWidth::Byte,
                    signed,
                },
                left_count + right_count,
            )
        }
        MirOp::Store {
            dst:
                MirAddr::PointerIndex {
                    ptr,
                    index,
                    elem_size,
                    offset,
                },
            src,
            width,
        } => {
            let (index, count) = rewrite_mir_copy_prop_const_value(index, env);
            let (src, src_count) = match width {
                MirWidth::Byte => rewrite_mir_copy_prop_const_value(src, env),
                MirWidth::Word => (src, MirCopyPropRewriteCounts::default()),
            };
            (
                MirOp::Store {
                    dst: MirAddr::PointerIndex {
                        ptr,
                        index,
                        elem_size,
                        offset,
                    },
                    src,
                    width,
                },
                count + src_count,
            )
        }
        MirOp::Store {
            dst:
                MirAddr::ComputedIndex {
                    base,
                    index,
                    elem_size,
                    offset,
                },
            src,
            width,
        } => {
            let (base, base_count) = rewrite_mir_copy_prop_const_word_parts(base, env);
            let (index, count) = rewrite_mir_copy_prop_const_value(index, env);
            let (src, src_count) = match width {
                MirWidth::Byte => rewrite_mir_copy_prop_const_value(src, env),
                MirWidth::Word => (src, MirCopyPropRewriteCounts::default()),
            };
            (
                MirOp::Store {
                    dst: MirAddr::ComputedIndex {
                        base,
                        index,
                        elem_size,
                        offset,
                    },
                    src,
                    width,
                },
                base_count + count + src_count,
            )
        }
        MirOp::Store {
            dst: MirAddr::Deref { ptr, offset },
            src,
            width,
        } => {
            let (ptr, ptr_count) = rewrite_mir_copy_prop_const_word_parts(ptr, env);
            let (src, src_count) = match width {
                MirWidth::Byte => rewrite_mir_copy_prop_const_value(src, env),
                MirWidth::Word => (src, MirCopyPropRewriteCounts::default()),
            };
            (
                MirOp::Store {
                    dst: MirAddr::Deref { ptr, offset },
                    src,
                    width,
                },
                ptr_count + src_count,
            )
        }
        MirOp::Store {
            dst: MirAddr::Direct(dst),
            src,
            width: MirWidth::Byte,
        } => {
            let (src, count) = rewrite_mir_copy_prop_byte_value(src, env);
            (
                MirOp::Store {
                    dst: MirAddr::Direct(dst),
                    src,
                    width: MirWidth::Byte,
                },
                count,
            )
        }
        MirOp::Store {
            dst,
            src,
            width: MirWidth::Byte,
        } => {
            let (src, count) = rewrite_mir_copy_prop_const_value(src, env);
            (
                MirOp::Store {
                    dst,
                    src,
                    width: MirWidth::Byte,
                },
                count,
            )
        }
        MirOp::Move {
            dst,
            src,
            width: MirWidth::Byte,
        } => {
            let (src, count) = rewrite_mir_copy_prop_const_value(src, env);
            (
                MirOp::Move {
                    dst,
                    src,
                    width: MirWidth::Byte,
                },
                count,
            )
        }
        MirOp::Extend {
            dst,
            src,
            from_width,
            to_width,
            signed,
        } => {
            let (src, count) = rewrite_mir_copy_prop_const_value(src, env);
            (
                MirOp::Extend {
                    dst,
                    src,
                    from_width,
                    to_width,
                    signed,
                },
                count,
            )
        }
        MirOp::Truncate {
            dst,
            src,
            from_width,
            to_width,
        } => {
            let (src, count) = rewrite_mir_copy_prop_const_value(src, env);
            (
                MirOp::Truncate {
                    dst,
                    src,
                    from_width,
                    to_width,
                },
                count,
            )
        }
        MirOp::Binary {
            op,
            dst,
            left,
            right,
            width: MirWidth::Byte,
            carry_in,
            carry_out,
        } => {
            let (left, left_count) = match op {
                MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor => {
                    rewrite_mir_copy_prop_byte_value(left, env)
                }
                MirBinaryOp::Add
                | MirBinaryOp::Sub
                | MirBinaryOp::Mul
                | MirBinaryOp::Div
                | MirBinaryOp::Mod
                | MirBinaryOp::Lsh
                | MirBinaryOp::Rsh => rewrite_mir_copy_prop_const_value(left, env),
            };
            let (right, right_count) = match op {
                MirBinaryOp::Add | MirBinaryOp::Sub => rewrite_mir_copy_prop_byte_value(right, env),
                MirBinaryOp::And
                | MirBinaryOp::Or
                | MirBinaryOp::Xor
                | MirBinaryOp::Mul
                | MirBinaryOp::Div
                | MirBinaryOp::Mod
                | MirBinaryOp::Lsh
                | MirBinaryOp::Rsh => rewrite_mir_copy_prop_const_value(right, env),
            };
            (
                MirOp::Binary {
                    op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Byte,
                    carry_in,
                    carry_out,
                },
                left_count + right_count,
            )
        }
        MirOp::Unary {
            op,
            dst,
            src,
            width: MirWidth::Byte,
        } => {
            let (src, count) = rewrite_mir_copy_prop_byte_value(src, env);
            (
                MirOp::Unary {
                    op,
                    dst,
                    src,
                    width: MirWidth::Byte,
                },
                count,
            )
        }
        MirOp::AddByteToWordMem { mem, value } => {
            let (value, count) = rewrite_mir_copy_prop_const_value(value, env);
            (MirOp::AddByteToWordMem { mem, value }, count)
        }
        MirOp::SubByteFromWordMem { mem, value } => {
            let (value, count) = rewrite_mir_copy_prop_const_value(value, env);
            (MirOp::SubByteFromWordMem { mem, value }, count)
        }
        MirOp::StoreIndirect {
            consumer,
            src,
            offset,
        } => {
            let (src, count) = rewrite_mir_copy_prop_const_value(src, env);
            (
                MirOp::StoreIndirect {
                    consumer,
                    src,
                    offset,
                },
                count,
            )
        }
        MirOp::AdvanceAddress {
            consumer,
            index,
            scale,
        } => {
            let (index, count) = rewrite_mir_copy_prop_const_value(index, env);
            (
                MirOp::AdvanceAddress {
                    consumer,
                    index,
                    scale,
                },
                count,
            )
        }
        MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            scale,
        } => {
            let (base, base_count) = rewrite_mir_copy_prop_const_value(base, env);
            let (index, count) = rewrite_mir_copy_prop_const_value(index, env);
            (
                MirOp::MaterializeIndexedAddress {
                    consumer,
                    base,
                    index,
                    scale,
                },
                base_count + count,
            )
        }
        MirOp::Call {
            target,
            abi,
            args,
            result,
            effects,
        } => {
            let mut rewritten_count = MirCopyPropRewriteCounts::default();
            let args = args
                .into_iter()
                .map(|mut arg| {
                    if arg.width == MirWidth::Byte {
                        let (value, count) = rewrite_mir_copy_prop_const_value(arg.value, env);
                        arg.value = value;
                        rewritten_count += count;
                    }
                    arg
                })
                .collect();
            (
                MirOp::Call {
                    target,
                    abi,
                    args,
                    result,
                    effects,
                },
                rewritten_count,
            )
        }
        MirOp::MaterializeAddress { consumer, value } => {
            let (value, count) = rewrite_mir_copy_prop_const_value(value, env);
            (MirOp::MaterializeAddress { consumer, value }, count)
        }
        MirOp::Load {
            dst,
            src:
                MirAddr::PointerIndex {
                    ptr,
                    index,
                    elem_size,
                    offset,
                },
            width,
        } => {
            let (index, count) = rewrite_mir_copy_prop_const_value(index, env);
            (
                MirOp::Load {
                    dst,
                    src: MirAddr::PointerIndex {
                        ptr,
                        index,
                        elem_size,
                        offset,
                    },
                    width,
                },
                count,
            )
        }
        MirOp::Load {
            dst,
            src:
                MirAddr::ComputedIndex {
                    base,
                    index,
                    elem_size,
                    offset,
                },
            width,
        } => {
            let (base, base_count) = rewrite_mir_copy_prop_const_word_parts(base, env);
            let (index, count) = rewrite_mir_copy_prop_const_value(index, env);
            (
                MirOp::Load {
                    dst,
                    src: MirAddr::ComputedIndex {
                        base,
                        index,
                        elem_size,
                        offset,
                    },
                    width,
                },
                base_count + count,
            )
        }
        MirOp::Load {
            dst,
            src: MirAddr::Deref { ptr, offset },
            width,
        } => {
            let (ptr, count) = rewrite_mir_copy_prop_const_word_parts(ptr, env);
            (
                MirOp::Load {
                    dst,
                    src: MirAddr::Deref { ptr, offset },
                    width,
                },
                count,
            )
        }
        _ => (op, MirCopyPropRewriteCounts::default()),
    }
}

fn rewrite_mir_copy_prop_byte_value(
    value: MirValue,
    env: &SsaLiteV2ObserveEnv,
) -> (MirValue, MirCopyPropRewriteCounts) {
    match value {
        MirValue::Def(def @ (MirDef::VTemp(_) | MirDef::VTempByte { .. })) => {
            match env.def_fact(&def) {
                Some(SsaLiteV2ValueKey::ConstU8(value)) => (
                    MirValue::ConstU8(*value),
                    MirCopyPropRewriteCounts::one_const(),
                ),
                Some(SsaLiteV2ValueKey::DirectMem(mem)) => (
                    MirValue::PointerCell(mem.clone()),
                    MirCopyPropRewriteCounts::one_direct_mem(),
                ),
                Some(SsaLiteV2ValueKey::Temp(source)) if source != &def => (
                    MirValue::Def(source.clone()),
                    MirCopyPropRewriteCounts::one_temp_alias(),
                ),
                Some(SsaLiteV2ValueKey::Temp(_)) | None => {
                    (MirValue::Def(def), MirCopyPropRewriteCounts::default())
                }
            }
        }
        other => (other, MirCopyPropRewriteCounts::default()),
    }
}

fn rewrite_mir_copy_prop_const_value(
    value: MirValue,
    env: &SsaLiteV2ObserveEnv,
) -> (MirValue, MirCopyPropRewriteCounts) {
    match value {
        MirValue::Def(def @ (MirDef::VTemp(_) | MirDef::VTempByte { .. })) => {
            match env.def_fact(&def) {
                Some(SsaLiteV2ValueKey::ConstU8(value)) => (
                    MirValue::ConstU8(*value),
                    MirCopyPropRewriteCounts::one_const(),
                ),
                Some(SsaLiteV2ValueKey::Temp(source)) if source != &def => (
                    MirValue::Def(source.clone()),
                    MirCopyPropRewriteCounts::one_temp_alias(),
                ),
                Some(SsaLiteV2ValueKey::DirectMem(_) | SsaLiteV2ValueKey::Temp(_)) | None => {
                    (MirValue::Def(def), MirCopyPropRewriteCounts::default())
                }
            }
        }
        MirValue::Word { lo, hi } => {
            let (lo, lo_count) = rewrite_mir_copy_prop_const_value(*lo, env);
            let (hi, hi_count) = rewrite_mir_copy_prop_const_value(*hi, env);
            (
                MirValue::Word {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
                lo_count + hi_count,
            )
        }
        other => (other, MirCopyPropRewriteCounts::default()),
    }
}

fn rewrite_mir_copy_prop_const_word_parts(
    value: MirValue,
    env: &SsaLiteV2ObserveEnv,
) -> (MirValue, MirCopyPropRewriteCounts) {
    match value {
        MirValue::Word { lo, hi } => {
            let (lo, lo_count) = rewrite_mir_copy_prop_const_value(*lo, env);
            let (hi, hi_count) = rewrite_mir_copy_prop_const_value(*hi, env);
            (
                MirValue::Word {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
                lo_count + hi_count,
            )
        }
        other => (other, MirCopyPropRewriteCounts::default()),
    }
}

fn ssa_lite_mem_is_trackable(layout: &MaterializeLayout, mem: &MirMem) -> bool {
    match mem {
        MirMem::Global { id, .. } => layout.global_allows_idempotent_store_removal(*id),
        MirMem::Static { .. }
        | MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Spill { .. }
        | MirMem::ZeroPage(_)
        | MirMem::FixedZeroPage(_) => true,
        MirMem::Absolute(_) => false,
    }
}

fn lo_address(mem: &MirMem) -> u16 {
    match mem {
        MirMem::FixedZeroPage(slot) => u16::from(slot.0),
        _ => 0,
    }
}

fn ssa_lite_mem_resolves_to_address(
    layout: &MaterializeLayout,
    routine_id: RoutineId,
    mem: &MirMem,
    address: u16,
) -> bool {
    match mem {
        MirMem::FixedZeroPage(slot) => u16::from(slot.0) == address,
        MirMem::ZeroPage(_) => false,
        _ => layout
            .mem_address(routine_id, mem)
            .is_some_and(|mem_address| mem_address == address),
    }
}

fn ssa_lite_rewrite_byte_load(
    env: &SsaLiteValueEnv,
    op: &MirOp,
    layout: &MaterializeLayout,
) -> MirOp {
    let MirOp::Load {
        dst: MirDef::Reg(reg),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    } = op
    else {
        return op.clone();
    };
    if !mem_is_private_scratch(mem) {
        return op.clone();
    }
    match env.mem_fact(mem) {
        Some(SsaLiteValueKey::DirectMem(source))
            if source != mem && ssa_lite_mem_is_trackable(layout, source) =>
        {
            MirOp::Load {
                dst: MirDef::Reg(*reg),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            }
        }
        Some(SsaLiteValueKey::ConstU8(value)) => MirOp::LoadImm {
            dst: MirDef::Reg(*reg),
            value: u16::from(*value),
            width: MirWidth::Byte,
        },
        _ => op.clone(),
    }
}

fn ssa_lite_rewrite_value(
    env: &SsaLiteValueEnv,
    value: &MirValue,
    layout: &MaterializeLayout,
) -> MirValue {
    let MirValue::PointerCell(mem) = value else {
        return value.clone();
    };
    if !mem_is_private_scratch(mem) {
        return value.clone();
    }
    match env.mem_fact(mem) {
        Some(SsaLiteValueKey::DirectMem(source))
            if source != mem && ssa_lite_mem_is_trackable(layout, source) =>
        {
            MirValue::PointerCell(source.clone())
        }
        Some(SsaLiteValueKey::ConstU8(value)) => MirValue::ConstU8(*value),
        _ => value.clone(),
    }
}

fn ssa_lite_rewrite_byte_consumer(
    env: &SsaLiteValueEnv,
    op: &MirOp,
    layout: &MaterializeLayout,
) -> MirOp {
    match op {
        MirOp::Compare {
            dst,
            op,
            left,
            right,
            width: MirWidth::Byte,
            signed,
        } => MirOp::Compare {
            dst: dst.clone(),
            op: *op,
            left: ssa_lite_rewrite_value(env, left, layout),
            right: ssa_lite_rewrite_value(env, right, layout),
            width: MirWidth::Byte,
            signed: *signed,
        },
        MirOp::Binary {
            op,
            dst,
            left,
            right,
            width: MirWidth::Byte,
            carry_in,
            carry_out,
        } => MirOp::Binary {
            op: *op,
            dst: dst.clone(),
            left: ssa_lite_rewrite_value(env, left, layout),
            right: ssa_lite_rewrite_value(env, right, layout),
            width: MirWidth::Byte,
            carry_in: *carry_in,
            carry_out: *carry_out,
        },
        MirOp::Store {
            dst,
            src,
            width: MirWidth::Byte,
        } => MirOp::Store {
            dst: dst.clone(),
            src: ssa_lite_rewrite_value(env, src, layout),
            width: MirWidth::Byte,
        },
        _ => op.clone(),
    }
}

fn ssa_lite_rewrite_byte_move(
    env: &SsaLiteValueEnv,
    op: &MirOp,
    layout: &MaterializeLayout,
) -> MirOp {
    let MirOp::Move {
        dst: MirDef::Reg(reg),
        src: MirValue::PointerCell(mem),
        width: MirWidth::Byte,
    } = op
    else {
        return op.clone();
    };
    if !mem_is_private_scratch(mem) {
        return op.clone();
    }
    match env.mem_fact(mem) {
        Some(SsaLiteValueKey::DirectMem(source))
            if source != mem && ssa_lite_mem_is_trackable(layout, source) =>
        {
            MirOp::Load {
                dst: MirDef::Reg(*reg),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            }
        }
        Some(SsaLiteValueKey::ConstU8(value)) => MirOp::LoadImm {
            dst: MirDef::Reg(*reg),
            value: u16::from(*value),
            width: MirWidth::Byte,
        },
        _ => op.clone(),
    }
}

fn ssa_lite_rewrite_byte_op(
    env: &SsaLiteValueEnv,
    op: &MirOp,
    layout: &MaterializeLayout,
) -> MirOp {
    let rewritten = ssa_lite_rewrite_byte_load(env, op, layout);
    if rewritten != *op {
        return rewritten;
    }
    let rewritten = ssa_lite_rewrite_byte_move(env, op, layout);
    if rewritten != *op {
        return rewritten;
    }
    ssa_lite_rewrite_byte_consumer(env, op, layout)
}

fn ssa_lite_loaded_reg_key(
    op: &MirOp,
    layout: &MaterializeLayout,
) -> Option<(MirReg, SsaLiteValueKey)> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } if ssa_lite_mem_is_trackable(layout, mem) => {
            Some((*reg, SsaLiteValueKey::DirectMem(mem.clone())))
        }
        MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            value,
            width: MirWidth::Byte,
        } => u8::try_from(*value)
            .ok()
            .map(|value| (*reg, SsaLiteValueKey::ConstU8(value))),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SsaLiteReloadDecision {
    RemoveCallFree,
    RetainCallBarrier,
    RetainFlags,
}

fn ssa_lite_redundant_reload_decision_after(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> SsaLiteReloadDecision {
    for op in ops.iter().skip(index.saturating_add(1)) {
        if matches!(
            op,
            MirOp::Call { .. }
                | MirOp::RuntimeHelper { .. }
                | MirOp::Barrier { .. }
                | MirOp::MachineBlock { .. }
        ) {
            return SsaLiteReloadDecision::RetainCallBarrier;
        }
        if op_writes_flags(op) {
            return SsaLiteReloadDecision::RemoveCallFree;
        }
    }
    if terminator_consumes_flags(terminator) {
        SsaLiteReloadDecision::RetainFlags
    } else {
        SsaLiteReloadDecision::RemoveCallFree
    }
}
pub(super) fn fold_ssa_lite_byte_loads(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    fold_ssa_lite_byte_loads_with_env(
        ops,
        routine_id,
        layout,
        terminator,
        SsaLiteValueEnv::default(),
        "ssa-lite-consumer-forwards",
        peephole_stats,
    )
}

fn fold_ssa_lite_byte_loads_with_env(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    initial_env: SsaLiteValueEnv,
    stat_name: &'static str,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut env = initial_env;
    let mut out = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().cloned().enumerate() {
        let rewritten = ssa_lite_rewrite_byte_op(&env, &op, layout);
        if rewritten != op {
            peephole_stats.record(routine_id, stat_name);
        }
        if let Some((reg, key)) = ssa_lite_loaded_reg_key(&rewritten, layout)
            && env.reg_fact(reg) == Some(&key)
        {
            match ssa_lite_redundant_reload_decision_after(&ops, index, terminator) {
                SsaLiteReloadDecision::RemoveCallFree => {
                    peephole_stats.record(routine_id, "ssa-lite-redundant-reloads");
                    peephole_stats.record(routine_id, "ssa-lite-redundant-reloads-call-free");
                    continue;
                }
                SsaLiteReloadDecision::RetainCallBarrier => {
                    peephole_stats.record(routine_id, "ssa-lite-reload-retained-call-barrier");
                }
                SsaLiteReloadDecision::RetainFlags => {
                    peephole_stats.record(routine_id, "ssa-lite-reload-retained-flags");
                }
            }
        }
        env.observe_op(&rewritten, layout);
        out.push(rewritten);
    }
    out
}

pub(super) fn fold_ssa_lite_single_predecessor_loads(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) {
    let predecessor_counts = block_predecessor_index_counts(routine);
    let mut incoming: Vec<Option<SsaLiteValueEnv>> = vec![None; routine.blocks.len()];
    for block_index in 0..routine.blocks.len() {
        let initial_env = incoming[block_index].take().unwrap_or_default();
        let terminator = routine.blocks[block_index].terminator.clone();
        let successor_env = {
            let block = &mut routine.blocks[block_index];
            let ops = std::mem::take(&mut block.ops);
            block.ops = fold_ssa_lite_byte_loads_with_env(
                ops,
                routine.id,
                layout,
                &terminator,
                initial_env.clone(),
                "ssa-lite-cross-block-forwards",
                peephole_stats,
            );
            ssa_lite_successor_env(scan_ssa_lite_block_env_from(
                &block.ops,
                layout,
                initial_env,
            ))
        };

        for successor_index in block_successor_indices(routine, &terminator) {
            if !ssa_lite_env_has_transferable_facts(&successor_env) {
                continue;
            }
            if successor_index <= block_index {
                peephole_stats.record(routine.id, "ssa-lite-cross-block-backedge-skipped");
                peephole_stats.record(routine.id, "ssa-lite-cross-block-retained-backedge");
                continue;
            }
            if predecessor_counts[successor_index] != 1 {
                peephole_stats.record(routine.id, "ssa-lite-cross-block-join-skipped");
                peephole_stats.record(routine.id, "ssa-lite-cross-block-retained-join");
                continue;
            }
            peephole_stats.record(routine.id, "ssa-lite-cross-block-seeds");
            peephole_stats.record(routine.id, "ssa-lite-cross-block-seeds-call-free");
            incoming[successor_index] = Some(successor_env.clone());
        }
    }
}

fn block_predecessor_index_counts(routine: &MirRoutine) -> Vec<usize> {
    let mut counts = vec![0; routine.blocks.len()];
    for block in &routine.blocks {
        for successor_index in block_successor_indices(routine, &block.terminator) {
            counts[successor_index] += 1;
        }
    }
    counts
}

fn ssa_lite_successor_env(mut env: SsaLiteValueEnv) -> SsaLiteValueEnv {
    env.a = None;
    env.x = None;
    env.y = None;
    env.stats = SsaLiteScanStats::default();
    env
}

fn ssa_lite_env_has_transferable_facts(env: &SsaLiteValueEnv) -> bool {
    !env.mem.is_empty()
}
