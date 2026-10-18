//! Small, fail-closed object provenance facts for pre-home observation proofs.
//! This is not a general alias analysis: offsets are singleton ranges, and
//! pointer-valued temps must be complete, preceding same-block definitions.
#![allow(dead_code)] // The following slice adds the observation-proof consumer.
use super::{
    sites::MirSite,
    use_def::{MirTempLane, MirTempRequirement, MirTempUseDefIndex},
};
use crate::{
    mir6502::ir::*,
    nir::{LocalId, ParamId, SymbolId},
};
use std::collections::BTreeMap;

mod audit;
#[allow(unused_imports)] // Consumed by scalar cleanup in the next slice.
pub(in crate::mir6502) use audit::{Summary, summarize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::mir6502) enum Object {
    Local(RoutineId, LocalId),
    Param(RoutineId, ParamId),
    Global(SymbolId),
    Static(SymbolId),
}

impl Object {
    pub fn memory(self, offset: u16) -> MirMem {
        match self {
            Self::Local(_, id) => MirMem::Local { id, offset },
            Self::Param(_, id) => MirMem::Param { id, offset },
            Self::Global(id) => MirMem::Global { id, offset },
            Self::Static(id) => MirMem::Static { id, offset },
        }
    }
    pub fn owner(self) -> RoutineId {
        match self {
            Self::Local(owner, _) | Self::Param(owner, _) => owner,
            Self::Global(_) | Self::Static(_) => RoutineId(0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct Pointer {
    pub object: Object,
    pub extent: u16,
    pub offset: u16,
}

impl Pointer {
    pub fn named(p: &MirProgram, r: &MirRoutine, mem: &MirMem) -> Option<Self> {
        let (object, offset, extent) = match mem {
            MirMem::Local { id, offset } => (
                Object::Local(r.id, *id),
                *offset,
                r.frame
                    .locals
                    .iter()
                    .find(|s| s.base == MirStorageBase::Local(*id) && s.offset == 0)?
                    .storage_size,
            ),
            MirMem::Param { id, offset } => (
                Object::Param(r.id, *id),
                *offset,
                r.frame
                    .params
                    .iter()
                    .find(|s| s.base == MirStorageBase::Param(*id) && s.offset == 0)?
                    .storage_size,
            ),
            MirMem::Global { id, offset } => (
                Object::Global(*id),
                *offset,
                p.globals
                    .iter()
                    .find(|g| {
                        g.id == *id && matches!(g.backing, MirGlobalBacking::Ordinary { .. })
                    })?
                    .storage_size,
            ),
            MirMem::Static { id, offset } => (
                Object::Static(*id),
                *offset,
                u16::try_from(p.statics.iter().find(|s| s.id == *id)?.image.bytes.len()).ok()?,
            ),
            _ => return None,
        };
        (extent > 0 && offset <= extent).then_some(Self {
            object,
            extent,
            offset,
        })
    }
    pub fn add(self, n: u16) -> Option<Self> {
        let offset = self.offset.checked_add(n)?;
        (offset <= self.extent).then_some(Self { offset, ..self })
    }
    pub fn sub(self, n: u16) -> Option<Self> {
        Some(Self {
            offset: self.offset.checked_sub(n)?,
            ..self
        })
    }
    pub fn covers(self, bytes: u16) -> bool {
        bytes > 0
            && self
                .offset
                .checked_add(bytes)
                .is_some_and(|end| end <= self.extent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) enum Value {
    Scalar,
    Constant(u16),
    Pointer(Pointer),
}

/// A fresh instance is required for each block. Pointer temps cannot carry
/// facts into successors or be reconstructed from partial lanes. Constants
/// and ordinary scalar values need no address/escape facts.
pub(in crate::mir6502) struct Values<'a> {
    program: &'a MirProgram,
    routine: &'a MirRoutine,
    block: MirBlockId,
    defs: &'a MirTempUseDefIndex,
    temps: BTreeMap<MirTempId, Value>,
}

impl<'a> Values<'a> {
    pub fn new(
        program: &'a MirProgram,
        routine: &'a MirRoutine,
        block: MirBlockId,
        defs: &'a MirTempUseDefIndex,
    ) -> Self {
        Self {
            program,
            routine,
            block,
            defs,
            temps: BTreeMap::new(),
        }
    }
    pub fn value(&self, value: &MirValue) -> Option<Value> {
        match value {
            MirValue::ConstU8(n) => Some(Value::Constant(u16::from(*n))),
            MirValue::ConstU16(n) => Some(Value::Constant(*n)),
            MirValue::Def(MirDef::VTemp(id)) => {
                Some(self.temps.get(id).copied().unwrap_or(Value::Scalar))
            }
            MirValue::Def(MirDef::VTempByte { id, .. })
                if !matches!(self.temps.get(id), Some(Value::Pointer(_))) =>
            {
                Some(Value::Scalar)
            }
            MirValue::GlobalAddr(id) => Pointer::named(
                self.program,
                self.routine,
                &MirMem::Global { id: *id, offset: 0 },
            )
            .map(Value::Pointer),
            MirValue::StaticAddr(id) => Pointer::named(
                self.program,
                self.routine,
                &MirMem::Static { id: *id, offset: 0 },
            )
            .map(Value::Pointer),
            MirValue::Word { lo, hi } => match (self.value(lo)?, self.value(hi)?) {
                (Value::Constant(lo), Value::Constant(hi)) if lo <= 255 && hi <= 255 => {
                    Some(Value::Constant(lo | (hi << 8)))
                }
                (Value::Pointer(_), _) | (_, Value::Pointer(_)) => None,
                _ => Some(Value::Scalar),
            },
            // Code, numeric pointer cells, physical registers, and split
            // relocation lanes do not establish a complete bounded object.
            _ => None,
        }
    }
    pub fn define(
        &mut self,
        dst: &MirDef,
        width: MirWidth,
        value: Value,
        index: usize,
    ) -> Option<()> {
        let MirDef::VTemp(id) = dst else {
            return None;
        };
        if matches!(value, Value::Pointer(_)) {
            let site = MirSite::Op {
                block: self.block,
                op_index: index,
            };
            if width != MirWidth::Word
                || !(0..2).all(|byte| self.defs.unique_definition(MirTempLane { temp: *id, byte }).is_some_and(|d| d.site == site))
                || self.defs.uses_of_temp(*id).iter().any(|u| {
                    u.requirement != MirTempRequirement::Full(*id)
                        || !matches!(u.site, MirSite::Op { block, op_index } if block == self.block && op_index > index)
                })
            { return None; }
        }
        let value = match value {
            Value::Constant(n) if width == MirWidth::Byte => Value::Constant(n & 255),
            other => other,
        };
        self.temps.insert(*id, value);
        Some(())
    }
    pub fn address(&self, addr: &MirAddr) -> Option<Pointer> {
        match addr {
            MirAddr::Direct(mem) => Pointer::named(self.program, self.routine, mem),
            MirAddr::Deref { ptr, offset } => self.derived(ptr, None, 1, *offset),
            MirAddr::ComputedIndex {
                base,
                index,
                elem_size,
                offset,
            } => self.derived(base, Some(index), *elem_size, *offset),
            _ => None,
        }
    }
    pub fn derived(
        &self,
        base: &MirValue,
        index: Option<&MirValue>,
        stride: u16,
        offset: u16,
    ) -> Option<Pointer> {
        let Value::Pointer(base) = self.value(base)? else {
            return None;
        };
        let scaled = match index {
            None => 0,
            Some(index) => {
                let Value::Constant(n) = self.value(index)? else {
                    return None;
                };
                n.checked_mul(stride)?
            }
        };
        base.add(scaled.checked_add(offset)?)
    }
}

#[cfg(test)]
mod tests;
