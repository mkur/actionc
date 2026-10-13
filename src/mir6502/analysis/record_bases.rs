//! Complete resident base facts with final physical aliases, not record types.
#![allow(dead_code)] // Stage 1 supplies proofs; the following slice enables selection.
use super::{
    effects::classify_op,
    sites::{MirRoutineGeneration, MirSite},
};
use crate::mir6502::ir::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(in crate::mir6502) struct MirPhysicalMemory {
    entries: Vec<(RoutineId, MirMem, u16, bool)>,
}
impl MirPhysicalMemory {
    pub(in crate::mir6502) fn insert(
        &mut self,
        routine: RoutineId,
        mem: MirMem,
        address: u16,
        ordinary: bool,
    ) {
        if !self
            .entries
            .iter()
            .any(|(r, m, _, _)| *r == routine && *m == mem)
        {
            self.entries.push((routine, mem, address, ordinary));
        }
    }
    pub(in crate::mir6502) fn address(&self, routine: RoutineId, mem: &MirMem) -> Option<u16> {
        match mem {
            MirMem::FixedZeroPage(s) => Some(u16::from(s.0)),
            MirMem::Absolute(a) => Some(*a),
            _ => self
                .entries
                .iter()
                .find_map(|(r, m, a, _)| (*r == routine && m == mem).then_some(*a)),
        }
    }
    fn ordinary(&self, routine: RoutineId, mem: &MirMem) -> bool {
        self.entries.iter().any(|(r, m, a, normal)| {
            *r == routine
                && m == mem
                && *normal
                && ((0x400..0xC000).contains(a)
                    || matches!(m, MirMem::ZeroPage(_) | MirMem::Spill { .. }))
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct RecordBaseOptions {
    pub writes: bool,
    pub parameter_origins: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteValue {
    Constant(u8),
    Memory(u16),
}
impl ByteValue {
    fn affected(self, start: u16, bytes: u16) -> bool {
        matches!(self, Self::Memory(address) if address.wrapping_sub(start)<bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct RecordBaseProof {
    pub routine: RoutineId,
    pub generation: MirRoutineGeneration,
    pub site: MirSite,
    pub pair: u8,
    values: [ByteValue; 2],
    definitions: [usize; 2],
    pub options: RecordBaseOptions,
}

#[derive(Default)]
struct State {
    registers: [Option<ByteValue>; 3],
    pairs: BTreeMap<u8, (ByteValue, usize)>,
    cells: BTreeMap<u16, ByteValue>,
}
fn reg(reg: MirReg) -> usize {
    match reg {
        MirReg::A => 0,
        MirReg::X => 1,
        MirReg::Y => 2,
    }
}
fn pair_slot(slot: u8) -> bool {
    (0xAA..=0xAF).contains(&slot)
}

impl State {
    fn value(
        &self,
        value: &MirValue,
        routine: RoutineId,
        memory: &MirPhysicalMemory,
        options: RecordBaseOptions,
    ) -> Option<ByteValue> {
        match value {
            MirValue::ConstU8(v) => Some(ByteValue::Constant(*v)),
            MirValue::ConstU16(v) if *v <= 255 => Some(ByteValue::Constant(*v as u8)),
            MirValue::Def(MirDef::Reg(r)) => self.registers[reg(*r)],
            MirValue::StorageAddrByte { mem, byte } if *byte <= 1 => memory
                .address(routine, mem)
                .map(|a| ByteValue::Constant(a.to_le_bytes()[*byte as usize])),
            MirValue::PointerCell(mem) => {
                if !memory.ordinary(routine, mem) {
                    return None;
                }
                let address = memory.address(routine, mem)?;
                if !matches!(mem, MirMem::Param { .. }) || options.parameter_origins {
                    if let Some(v) = self.cells.get(&address) {
                        return Some(*v);
                    }
                }
                Some(ByteValue::Memory(address))
            }
            _ => None,
        }
    }
    fn write(&mut self, address: u16, bytes: u16) {
        self.pairs.retain(|slot, (value, _)| {
            u16::from(*slot).wrapping_sub(address) >= bytes && !value.affected(address, bytes)
        });
        self.cells.retain(|cell, value| {
            cell.wrapping_sub(address) >= bytes && !value.affected(address, bytes)
        });
        for value in &mut self.registers {
            if value.is_some_and(|v| v.affected(address, bytes)) {
                *value = None;
            }
        }
    }
    fn observe(
        &mut self,
        op: &MirOp,
        index: usize,
        routine: RoutineId,
        memory: &MirPhysicalMemory,
        options: RecordBaseOptions,
    ) {
        let effect = classify_op(op);
        let result = match op {
            MirOp::Load {
                dst: MirDef::Reg(r),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            } => {
                if !memory.ordinary(routine, mem) {
                    *self = Self::default();
                }
                Some((
                    *r,
                    self.value(
                        &MirValue::PointerCell(mem.clone()),
                        routine,
                        memory,
                        options,
                    ),
                ))
            }
            MirOp::Move {
                dst: MirDef::Reg(r),
                src,
                width: MirWidth::Byte,
            } => Some((*r, self.value(src, routine, memory, options))),
            MirOp::LoadImm {
                dst: MirDef::Reg(r),
                value,
                width: MirWidth::Byte,
            } => Some((*r, Some(ByteValue::Constant(*value as u8)))),
            _ => None,
        };
        let stored = match op {
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src,
                width: MirWidth::Byte,
            } => memory
                .address(routine, mem)
                .map(|address| (address, self.value(src, routine, memory, options), src)),
            _ => None,
        };
        if let Some((address, value, source)) = stored {
            if matches!(op, MirOp::Store { dst:MirAddr::Direct(mem), .. }
                if !matches!(mem,MirMem::FixedZeroPage(_)) && !memory.ordinary(routine,mem))
            {
                *self = Self::default();
            } else {
                self.write(address, 1);
            }
            if let Some(value) = value {
                if pair_slot(address as u8) && address <= 255 {
                    self.pairs.insert(address as u8, (value, index));
                }
                self.cells.insert(address, value);
            }
            // A completed direct store establishes equality with its source
            // carrier even when the incoming argument byte was unknown.
            if let MirValue::Def(MirDef::Reg(r)) = source {
                self.registers[reg(*r)] = Some(value.unwrap_or(ByteValue::Memory(address)));
            }
        } else if let MirOp::StoreIndirect {
            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo }),
            offset,
            ..
        } = op
        {
            let base = self
                .pairs
                .get(&lo.0)
                .zip(self.pairs.get(&lo.0.wrapping_add(1)))
                .and_then(|(low, high)| match (low.0, high.0) {
                    (ByteValue::Constant(l), ByteValue::Constant(h)) => {
                        Some(u16::from_le_bytes([l, h]))
                    }
                    _ => None,
                });
            if let Some(address) = base
                .filter(|_| options.writes)
                .map(|base| base.wrapping_add(*offset))
                .filter(|a| (0x400..0xC000).contains(a))
            {
                self.write(address, 1);
            } else {
                *self = Self::default();
            }
        } else if effect.memory.indirect_writes
            || effect.memory.opaque
            || effect.memory.may_write_any
            || matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            )
        {
            *self = Self::default();
        } else {
            for range in &effect.memory.direct_writes {
                if !matches!(range.base, MirMem::FixedZeroPage(_))
                    && !memory.ordinary(routine, &range.base)
                {
                    *self = Self::default();
                    break;
                }
                if let Some(address) = memory.address(routine, &range.base) {
                    self.write(address, range.bytes);
                } else {
                    *self = Self::default();
                }
            }
            for range in &effect.memory.direct_reads {
                if !memory.ordinary(routine, &range.base)
                    && matches!(range.base, MirMem::Absolute(_) | MirMem::Global { .. })
                {
                    *self = Self::default();
                }
            }
        }
        // Implicit constructors and compound operations can clobber a pair
        // without presenting a scalar direct store in the operation stream.
        for home in &effect.addresses.pair_writes {
            if let super::effects::MirHomeByte::FixedZeroPage(slot) = home {
                if stored.is_none_or(|(address, _, _)| address != u16::from(slot.0)) {
                    self.write(u16::from(slot.0), 1);
                }
            }
        }
        for r in [MirReg::A, MirReg::X, MirReg::Y] {
            if effect.may_clobber_reg_compat(r) {
                self.registers[reg(r)] = None;
            }
        }
        if let Some((r, value)) = result {
            self.registers[reg(r)] = value;
        }
    }
}

fn load_source(op: &MirOp) -> Option<MirValue> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some(MirValue::PointerCell(mem.clone())),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src,
            width: MirWidth::Byte,
        } if !matches!(src, MirValue::Def(_)) => Some(src.clone()),
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value,
            width: MirWidth::Byte,
        } => Some(MirValue::ConstU8(*value as u8)),
        _ => None,
    }
}
fn store_slot(op: &MirOp) -> Option<u8> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(slot)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => Some(slot.0),
        _ => None,
    }
}

pub(in crate::mir6502) fn prove(
    routine: &MirRoutine,
    generation: MirRoutineGeneration,
    site: MirSite,
    memory: &MirPhysicalMemory,
    options: RecordBaseOptions,
) -> Option<RecordBaseProof> {
    let MirSite::Op {
        block,
        op_index: index,
    } = site
    else {
        return None;
    };
    let block = routine.blocks.iter().find(|b| b.id == block)?;
    let window = block.ops.get(index..index.checked_add(4)?)?;
    let pair = store_slot(&window[1])?;
    if ![0xAA, 0xAC, 0xAE].contains(&pair) || store_slot(&window[3])? != pair + 1 {
        return None;
    }
    let inputs = [load_source(&window[0])?, load_source(&window[2])?];
    for input in &inputs {
        if let MirValue::PointerCell(mem) = input {
            if !memory.ordinary(routine.id, mem) {
                return None;
            }
            let address = memory.address(routine.id, mem)?;
            if address == u16::from(pair) || address == u16::from(pair + 1) {
                return None;
            }
        }
    }
    let mut state = State::default();
    for (i, op) in block.ops[..index].iter().enumerate() {
        state.observe(op, i, routine.id, memory, options);
    }
    let values = [
        state.value(&inputs[0], routine.id, memory, options)?,
        state.value(&inputs[1], routine.id, memory, options)?,
    ];
    let low = state.pairs.get(&pair)?;
    let high = state.pairs.get(&(pair + 1))?;
    if [low.0, high.0] != values {
        return None;
    }
    Some(RecordBaseProof {
        routine: routine.id,
        generation,
        site,
        pair,
        values,
        definitions: [low.1, high.1],
        options,
    })
}

pub(in crate::mir6502) fn validate(
    proof: &RecordBaseProof,
    routine: &MirRoutine,
    generation: MirRoutineGeneration,
    memory: &MirPhysicalMemory,
) -> bool {
    proof.routine == routine.id
        && proof.generation == generation
        && prove(routine, generation, proof.site, memory, proof.options).as_ref() == Some(proof)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(mem: MirMem) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }
    }
    fn store(mem: MirMem) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }
    fn sources() -> [MirMem; 2] {
        [0, 1].map(|offset| MirMem::Param {
            id: crate::nir::ParamId(0),
            offset,
        })
    }
    fn setup() -> Vec<MirOp> {
        let [low, high] = sources();
        vec![
            load(low),
            store(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC))),
            load(high),
            store(MirMem::FixedZeroPage(MirFixedZpSlot(0xAD))),
        ]
    }
    fn routine(ops: Vec<MirOp>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "Bases".into(),
            inline: Default::default(),
            scalar_signature: None,
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: vec![],
            effects: MirEffects::default(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".into(),
                params: vec![],
                ops,
                terminator: MirTerminator::Return,
            }],
        }
    }
    fn memory() -> MirPhysicalMemory {
        let mut memory = MirPhysicalMemory::default();
        for (i, mem) in sources().into_iter().enumerate() {
            memory.insert(RoutineId(0), mem, 0x3000 + i as u16, true);
        }
        memory
    }
    #[test]
    fn complete_pairs_survive_disjoint_stores_but_not_physical_aliases_or_partial_clobbers() {
        let local = MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        };
        let mut map = memory();
        map.insert(RoutineId(0), local.clone(), 0x4000, true);
        let mut ops = setup();
        ops.push(store(local.clone()));
        ops.extend(setup());
        let r = routine(ops);
        let generation = MirRoutineGeneration::initial();
        let site = MirSite::Op {
            block: MirBlockId(0),
            op_index: 5,
        };
        let proof = prove(&r, generation, site, &map, RecordBaseOptions::default()).unwrap();
        assert!(validate(&proof, &r, generation, &map));
        assert!(!validate(&proof, &r, generation.next(), &map));
        let mut changed = memory();
        changed.insert(RoutineId(0), local, 0x3001, true);
        assert!(!validate(&proof, &r, generation, &changed));
        let mut changed_r = r.clone();
        changed_r.blocks[0].ops[4] = MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem: MirMem::FixedZeroPage(MirFixedZpSlot(0xAD)),
            width: MirWidth::Byte,
        };
        assert!(
            prove(
                &changed_r,
                generation,
                site,
                &map,
                RecordBaseOptions::default()
            )
            .is_none()
        );
        let mut forged = proof.clone();
        forged.pair = 0xAA;
        assert!(!validate(&forged, &r, generation, &map));
    }
    #[test]
    fn only_proven_indirect_destinations_preserve_bases() {
        let write = MirOp::StoreIndirect {
            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                lo: MirFixedZpSlot(0xAC),
            }),
            src: MirValue::ConstU8(0),
            offset: 2,
        };
        let mut ops = setup();
        ops.push(write.clone());
        ops.extend(setup());
        let r = routine(ops);
        let site = MirSite::Op {
            block: MirBlockId(0),
            op_index: 5,
        };
        let generation = MirRoutineGeneration::initial();
        assert!(
            prove(
                &r,
                generation,
                site,
                &memory(),
                RecordBaseOptions {
                    writes: true,
                    parameter_origins: true
                }
            )
            .is_none()
        );
        let constant_setup = || {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0,
                    width: MirWidth::Byte,
                },
                store(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC))),
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0x60,
                    width: MirWidth::Byte,
                },
                store(MirMem::FixedZeroPage(MirFixedZpSlot(0xAD))),
            ]
        };
        let mut ops = constant_setup();
        ops.push(write);
        ops.extend(constant_setup());
        let r = routine(ops);
        assert!(
            prove(
                &r,
                generation,
                site,
                &memory(),
                RecordBaseOptions::default()
            )
            .is_none()
        );
        assert!(
            prove(
                &r,
                generation,
                site,
                &memory(),
                RecordBaseOptions {
                    writes: true,
                    parameter_origins: false
                }
            )
            .is_some()
        );
    }
    #[test]
    fn wrapped_write_ranges_include_each_byte_and_invalidate_both_dependencies() {
        let mut state = State::default();
        state.pairs.insert(0xAC, (ByteValue::Memory(0), 0));
        state.pairs.insert(0xAD, (ByteValue::Memory(0xFFFF), 1));
        state.write(0xFFFF, 2);
        assert!(state.pairs.is_empty());
        state.pairs.insert(0xAC, (ByteValue::Memory(0x3000), 0));
        state.pairs.insert(0xAD, (ByteValue::Memory(0x3001), 1));
        state.write(0x3002, 1);
        assert_eq!(state.pairs.len(), 2);
        state.write(0x3001, 1);
        assert_eq!(state.pairs.len(), 1);
    }
    #[test]
    fn physical_aliases_share_addresses_without_claiming_ordinary_absolute_memory() {
        let mut map = MirPhysicalMemory::default();
        let id = RoutineId(0);
        let cell = MirMem::Param {
            id: crate::nir::ParamId(0),
            offset: 0,
        };
        map.insert(id, cell.clone(), 0x3000, true);
        assert_eq!(
            map.address(id, &cell),
            map.address(id, &MirMem::Absolute(0x3000))
        );
        assert!(map.ordinary(id, &cell));
        assert!(!map.ordinary(id, &MirMem::Absolute(0x3000)));
    }
}
