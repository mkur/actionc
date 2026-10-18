//! Context-specific footprints. Each call is audited with its actual bounded
//! arguments; these results must never be cached by callee ID alone.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{classify_op, classify_terminator},
    private_cells,
};
use std::collections::BTreeSet;

const MAX_OPS: usize = 512;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug)]
pub(in crate::mir6502) struct Access {
    pub pointer: Pointer,
    pub bytes: u16,
}

#[derive(Default, Debug)]
pub(in crate::mir6502) struct Summary {
    pub reads: Vec<Access>,
    pub writes: Vec<Access>,
    /// Objects whose addresses participate in the proof. Final layout must
    /// establish their physical extents and separation from deleted cells.
    pub borrowed: BTreeMap<Object, u16>,
    /// Retained pointer homes, including static parameter cells. Their
    /// logical privacy also needs physical alias exclusion after allocation.
    pub private_homes: BTreeSet<Object>,
}

pub(in crate::mir6502) fn summarize(program: &MirProgram, routine: &MirRoutine) -> Option<Summary> {
    let mut audit = Audit {
        program,
        summary: Summary::default(),
        active: BTreeSet::new(),
        protected: BTreeSet::new(),
        uncovered_reads: BTreeSet::new(),
        addressed: BTreeSet::new(),
        budget: MAX_OPS,
    };
    audit.routine(routine, &BTreeMap::new())?;
    // A pointer retained in a private home is safe only when every read is a
    // complete, explicitly tracked transfer after a defining store. Checking
    // after the whole graph also catches an earlier read or a sibling block.
    if !audit.protected.is_disjoint(&audit.uncovered_reads)
        || !audit.protected.is_disjoint(&audit.addressed)
    {
        return None;
    }
    audit.summary.private_homes = audit.protected;
    Some(audit.summary)
}

struct Audit<'a> {
    program: &'a MirProgram,
    summary: Summary,
    active: BTreeSet<RoutineId>,
    protected: BTreeSet<Object>,
    uncovered_reads: BTreeSet<Object>,
    addressed: BTreeSet<Object>,
    budget: usize,
}

fn bytes(width: MirWidth) -> u16 {
    if width == MirWidth::Byte { 1 } else { 2 }
}
fn scalar(v: Value) -> Option<()> {
    (!matches!(v, Value::Pointer(_))).then_some(())
}

impl Audit<'_> {
    fn tick(&mut self) -> Option<()> {
        self.budget = self.budget.checked_sub(1)?;
        Some(())
    }
    fn address(&mut self, pointer: Pointer) {
        self.addressed.insert(pointer.object);
        self.summary.borrowed.insert(pointer.object, pointer.extent);
    }
    fn access(&mut self, pointer: Pointer, bytes: u16, write: bool, covered: bool) -> Option<()> {
        if !pointer.covers(bytes) {
            return None;
        }
        let access = Access { pointer, bytes };
        if write {
            // Incoming parameter facts must remain immutable, including when
            // a nested call writes through an actual pointer into this frame.
            if matches!(pointer.object, Object::Param(..)) {
                return None;
            }
            self.summary.writes.push(access);
        } else {
            if !covered {
                self.uncovered_reads.insert(pointer.object);
            }
            self.summary.reads.push(access);
        }
        Some(())
    }
    fn routine(
        &mut self,
        r: &MirRoutine,
        params: &BTreeMap<ParamId, Value>,
    ) -> Option<BTreeSet<u8>> {
        if self.active.len() == MAX_DEPTH
            || !self.active.insert(r.id)
            || !matches!(r.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry)
            || r.frame
                .params
                .iter()
                .map(|s| u32::from(s.storage_size))
                .sum::<u32>()
                > u32::from(crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES)
            || r.blocks.is_empty()
            || private_cells::frame_exposed_in_data(self.program, r.id)
            || r.frame.locals.iter().chain(&r.frame.params).any(|s| {
                !matches!(s.base, MirStorageBase::Local(_) | MirStorageBase::Param(_))
                    || s.offset != 0
            })
        {
            return None;
        }
        MirCfg::from_routine(r).ok()?;
        for (id, value) in params {
            if matches!(value, Value::Pointer(_)) {
                self.protected.insert(Object::Param(r.id, *id));
            }
        }
        let defs = MirTempUseDefIndex::from_routine(r);
        let mut returned: Option<BTreeSet<u8>> = None;
        for b in &r.blocks {
            self.tick()?;
            // No value facts cross a CFG edge. Private pointer-home reads in
            // another block are caught by the complete uncovered-read census.
            let mut values = Values::new(self.program, r, b.id, &defs);
            let mut homes = BTreeMap::<Object, Pointer>::new();
            let mut result_bytes = BTreeSet::new();
            for (i, op) in b.ops.iter().enumerate() {
                self.tick()?;
                match op {
                    MirOp::LoadImm { dst, value, width } => {
                        let n = if *width == MirWidth::Byte {
                            *value & 255
                        } else {
                            *value
                        };
                        values.define(dst, *width, Value::Constant(n), i)?;
                    }
                    MirOp::Move { dst, src, width } => {
                        values.define(dst, *width, values.value(src)?, i)?
                    }
                    MirOp::LeaAddr {
                        dst,
                        target,
                        width: MirWidth::Word,
                    } => {
                        let pointer = Pointer::named(self.program, r, target)?;
                        self.address(pointer);
                        values.define(dst, MirWidth::Word, Value::Pointer(pointer), i)?;
                    }
                    MirOp::ComputeAddress {
                        dst,
                        base,
                        index,
                        elem_size,
                        offset,
                    } => {
                        let pointer = values.derived(base, index.as_ref(), *elem_size, *offset)?;
                        self.address(pointer);
                        values.define(dst, MirWidth::Word, Value::Pointer(pointer), i)?;
                    }
                    MirOp::Load { dst, src, width } => {
                        let pointer = values.address(src)?;
                        let n = bytes(*width);
                        let value = match src {
                            MirAddr::Direct(MirMem::Param { id, offset: 0 })
                                if *width == MirWidth::Word =>
                            {
                                params.get(id).copied().unwrap_or(Value::Scalar)
                            }
                            MirAddr::Direct(_)
                                if pointer.offset == 0 && *width == MirWidth::Word =>
                            {
                                homes
                                    .get(&pointer.object)
                                    .copied()
                                    .map_or(Value::Scalar, Value::Pointer)
                            }
                            _ => Value::Scalar,
                        };
                        self.access(pointer, n, false, matches!(value, Value::Pointer(_)))?;
                        if !matches!(src, MirAddr::Direct(_)) {
                            self.address(pointer);
                        }
                        values.define(dst, *width, value, i)?;
                    }
                    MirOp::Store { dst, src, width } => {
                        let value = values.value(src)?;
                        // The ordinary Action scalar result area is a known
                        // ABI write, not permission for arbitrary ZP accesses.
                        if let MirAddr::Direct(MirMem::FixedZeroPage(slot)) = dst {
                            scalar(value)?;
                            if !(0xA0..=0xA3).contains(&slot.0)
                                || u16::from(slot.0) + bytes(*width) > 0xA4
                            {
                                return None;
                            }
                            result_bytes.extend((0..bytes(*width)).map(|n| slot.0 + n as u8));
                            continue;
                        }
                        let pointer = values.address(dst)?;
                        self.access(pointer, bytes(*width), true, false)?;
                        homes.remove(&pointer.object);
                        if !matches!(dst, MirAddr::Direct(_)) {
                            self.address(pointer);
                        }
                        if let Value::Pointer(value) = value {
                            // Temporary pointer homes have no externally
                            // usable address, initialization, or partial reads.
                            let MirAddr::Direct(MirMem::Local { id, offset: 0 }) = dst else {
                                return None;
                            };
                            let slot = r
                                .frame
                                .locals
                                .iter()
                                .find(|s| s.base == MirStorageBase::Local(*id))?;
                            if *width != MirWidth::Word
                                || slot.storage_size != 2
                                || slot.init.is_some()
                            {
                                return None;
                            }
                            self.protected.insert(pointer.object);
                            homes.insert(pointer.object, value);
                        }
                    }
                    MirOp::CopyBytes {
                        destination,
                        source,
                        size,
                        source_volatile: false,
                        destination_volatile: false,
                    } => {
                        let src = values.address(source)?;
                        let dst = values.address(destination)?;
                        self.access(src, *size, false, false)?;
                        self.access(dst, *size, true, false)?;
                        homes.remove(&dst.object);
                        if !matches!(source, MirAddr::Direct(_)) {
                            self.address(src);
                        }
                        if !matches!(destination, MirAddr::Direct(_)) {
                            self.address(dst);
                        }
                    }
                    MirOp::Binary {
                        dst,
                        left,
                        right,
                        width,
                        op,
                        carry_in,
                        carry_out,
                    } => {
                        let left = values.value(left)?;
                        let right = values.value(right)?;
                        let value = match (left, right) {
                            (Value::Pointer(pointer), Value::Constant(n))
                                if *width == MirWidth::Word
                                    && matches!(carry_in, None | Some(MirCarryIn::Clear))
                                    && *carry_out == MirCarryOut::Ignore =>
                            {
                                let pointer = match op {
                                    MirBinaryOp::Add => pointer.add(n)?,
                                    // SBC with a clear input carry subtracts an
                                    // extra byte; only the logical Sub is used.
                                    MirBinaryOp::Sub if carry_in.is_none() => pointer.sub(n)?,
                                    _ => return None,
                                };
                                self.address(pointer);
                                Value::Pointer(pointer)
                            }
                            (Value::Pointer(_), _) | (_, Value::Pointer(_)) => return None,
                            _ => Value::Scalar,
                        };
                        values.define(dst, *width, value, i)?;
                    }
                    MirOp::Extend {
                        dst,
                        src,
                        to_width: width,
                        ..
                    }
                    | MirOp::Truncate {
                        dst,
                        src,
                        to_width: width,
                        ..
                    }
                    | MirOp::Unary {
                        dst, src, width, ..
                    } => {
                        scalar(values.value(src)?)?;
                        values.define(dst, *width, Value::Scalar, i)?;
                    }
                    MirOp::Compare { left, right, .. } => {
                        scalar(values.value(left)?)?;
                        scalar(values.value(right)?)?;
                        // Comparison result defs cannot retain an earlier
                        // constant or pointer fact in this block.
                        for access in classify_op(op).logical.temp_defs {
                            let id = match access {
                                super::super::effects::MirTempAccess::Full(id)
                                | super::super::effects::MirTempAccess::Exact {
                                    temp: id, ..
                                } => id,
                            };
                            values.temps.remove(&id);
                        }
                    }
                    MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        abi,
                        args,
                        result,
                        additional_results,
                        effects,
                    } => {
                        let callee = self.program.routines.iter().find(|c| c.id == *id)?;
                        if callee.abi != MirRoutineAbi::Action
                            || effects.may_call_os
                            || !matches!(
                                effects.memory_reads,
                                MirMemoryEffect::None | MirMemoryEffect::Unknown
                            )
                            || !matches!(
                                effects.memory_writes,
                                MirMemoryEffect::None | MirMemoryEffect::Unknown
                            )
                            || args.len() != callee.frame.params.len()
                            || args.len() != abi.params.len()
                            || !abi
                                .result
                                .iter()
                                .chain(abi.additional_results.iter().map(|v| &v.home))
                                .all(result_home)
                            || !result
                                .iter()
                                .chain(additional_results)
                                .all(|v| result_home(&v.home))
                        {
                            return None;
                        }
                        let mut inputs = BTreeMap::new();
                        let mut argument_offset = 0;
                        for ((arg, home), slot) in
                            args.iter().zip(&abi.params).zip(&callee.frame.params)
                        {
                            let MirStorageBase::Param(id) = slot.base else {
                                return None;
                            };
                            if arg.home != *home
                                || *home
                                    != crate::mir6502::abi::action_arg_home(
                                        argument_offset,
                                        arg.width,
                                    )
                                || slot.scalar_width != Some(arg.width)
                                || slot.storage_size != bytes(arg.width)
                            {
                                return None;
                            }
                            let value = values.value(&arg.value)?;
                            if matches!(value, Value::Pointer(_)) && arg.width != MirWidth::Word {
                                return None;
                            }
                            inputs.insert(id, value);
                            argument_offset += bytes(arg.width);
                        }
                        // Unknown/opaque coarse direct-call memory facts can
                        // be narrowed only by auditing the entire actual body,
                        // including its ABI and every transitive effect.
                        let first_write = self.summary.writes.len();
                        let returned = self.routine(callee, &inputs)?;
                        result_bytes.clear();
                        for value in result.iter().chain(additional_results) {
                            let MirResultHome::ReturnSlot { offset } = value.home else {
                                return None;
                            };
                            if !(0..bytes(value.width))
                                .all(|n| returned.contains(&(0xA0 + offset as u8 + n as u8)))
                            {
                                return None;
                            }
                        }
                        for write in &self.summary.writes[first_write..] {
                            homes.remove(&write.pointer.object);
                        }
                        for result in result.iter().chain(additional_results) {
                            values.define(&result.dst, result.width, Value::Scalar, i)?;
                        }
                    }
                    // Includes machine code, helpers, volatile copies,
                    // post-home operations, implicit pointer-cell reads, and
                    // direct accesses to absolute/hardware-backed objects.
                    _ => return None,
                }
            }
            let effects = classify_terminator(&b.terminator);
            if !effects.memory.direct_references.is_empty()
                || !effects.memory.address_references.is_empty()
                || effects.memory.indirect_reads
                || effects.memory.indirect_writes
            {
                return None;
            }
            match &b.terminator {
                MirTerminator::Return => {
                    returned = Some(match returned {
                        None => result_bytes,
                        Some(previous) => previous.intersection(&result_bytes).copied().collect(),
                    });
                }
                MirTerminator::Exit => {}
                MirTerminator::Jump(edge) if edge.args.is_empty() => {}
                MirTerminator::Branch {
                    cond,
                    then_edge,
                    else_edge,
                } if then_edge.args.is_empty() && else_edge.args.is_empty() => {
                    if let MirCond::BoolValue(value) = cond {
                        scalar(values.value(value)?)?;
                    }
                }
                _ => return None,
            }
        }
        self.active.remove(&r.id);
        Some(returned.unwrap_or_default())
    }
}

fn result_home(home: &MirResultHome) -> bool {
    matches!(home, MirResultHome::ReturnSlot { offset: 0 | 2 })
}
