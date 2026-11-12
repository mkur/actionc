//! Byte equalities over independently allocated frame objects. No estimated
//! addresses, source names or distinct uncanonicalized IDs certify disjointness.
use super::{SsaLiteValueEnv, SsaLiteValueKey};
use crate::mir6502::ir::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_FACTS: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Object {
    Param(crate::nir::ParamId),
    Local(crate::nir::LocalId),
    Spill(MirSpillId),
}
#[derive(Debug, Clone, Copy)]
struct View {
    target: Object,
    offset: u16,
    size: u16,
}
#[derive(Debug, Clone)]
pub(super) struct HomeStorage {
    owner: RoutineId,
    slots: BTreeMap<Object, View>,
    duplicates: BTreeSet<Object>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Overlap {
    Disjoint,
    Overlap,
    Unknown,
}
impl HomeStorage {
    pub fn new(r: &MirRoutine) -> Self {
        let mut result = Self {
            owner: r.id,
            slots: BTreeMap::new(),
            duplicates: BTreeSet::new(),
        };
        for s in r.frame.params.iter().chain(&r.frame.locals) {
            // Descriptors and initialized address-like machine declarations do
            // not establish an ordinary scalar object for this narrow proof.
            if s.storage != MirStorageClass::Scalar
                || s.scalar_width.is_none()
                || s.init.is_some()
                || s.storage_size == 0
            {
                continue;
            }
            let (id, target, offset) = match s.base {
                MirStorageBase::Param(id) => (Object::Param(id), Object::Param(id), 0),
                MirStorageBase::Local(id) => (Object::Local(id), Object::Local(id), 0),
                MirStorageBase::LocalAlias { id, target } => {
                    (Object::Local(id), Object::Local(target), s.offset)
                }
                _ => continue,
            };
            result.insert(
                id,
                View {
                    target,
                    offset,
                    size: s.storage_size,
                },
            );
        }
        // Coloring rewrites uses to the surviving ID; emission allocates one
        // byte per declared spill. A missing or duplicated ID is never guessed.
        for &id in &r.frame.spills {
            result.insert(
                Object::Spill(id),
                View {
                    target: Object::Spill(id),
                    offset: 0,
                    size: 1,
                },
            );
        }
        result
    }
    fn insert(&mut self, id: Object, view: View) {
        if self.slots.insert(id, view).is_some() {
            self.duplicates.insert(id);
        }
    }
    fn region(&self, owner: RoutineId, mem: &MirMem, bytes: u16) -> Option<(Object, u16, u16)> {
        if owner != self.owner || bytes == 0 {
            return None;
        }
        let (mut id, mut offset) = match *mem {
            MirMem::Param { id, offset } => (Object::Param(id), offset),
            MirMem::Local { id, offset } => (Object::Local(id), offset),
            MirMem::Spill { id, offset } => (Object::Spill(id), offset),
            _ => return None,
        };
        for _ in 0..16 {
            if self.duplicates.contains(&id) {
                return None;
            }
            let view = self.slots.get(&id)?;
            if u32::from(offset) + u32::from(bytes) > u32::from(view.size) {
                return None;
            }
            if view.target == id {
                return Some((id, offset, bytes));
            }
            offset = offset.checked_add(view.offset)?;
            id = view.target;
        }
        None
    }
    pub fn overlap(&self, owner: RoutineId, a: &MirMem, bytes: u16, b: &MirMem) -> Overlap {
        match (self.region(owner, a, bytes), self.region(owner, b, 1)) {
            (Some((x, _, _)), Some((y, _, _))) if x != y => Overlap::Disjoint,
            (Some((_, start, len)), Some((_, byte, _))) => {
                if byte >= start && u32::from(byte) < u32::from(start) + u32::from(len) {
                    Overlap::Overlap
                } else {
                    Overlap::Disjoint
                }
            }
            _ => Overlap::Unknown,
        }
    }
    pub fn ordinary_byte(&self, mem: &MirMem) -> bool {
        self.region(self.owner, mem, 1).is_some()
    }
}

#[derive(Debug, Clone)]
pub(super) struct Precision {
    homes: Arc<HomeStorage>,
    exhausted: bool,
    pub peak: usize,
}
impl Precision {
    pub fn new(r: &MirRoutine) -> Self {
        Self {
            homes: Arc::new(HomeStorage::new(r)),
            exhausted: false,
            peak: 0,
        }
    }
    fn disjoint(&self, write: &MirMem, bytes: u16, home: &MirMem) -> bool {
        self.homes.overlap(self.homes.owner, write, bytes, home) == Overlap::Disjoint
    }
    pub fn ordinary_byte(&self, mem: &MirMem) -> bool {
        self.homes.ordinary_byte(mem)
    }
    pub fn invalidate(&self, env: &mut SsaLiteValueEnv, write: &MirMem, bytes: u16) {
        let before = env.mem.len();
        env.mem.retain(|(destination, value)| {
            self.disjoint(write, bytes, destination)
                && match value {
                    SsaLiteValueKey::DirectMem(source) => self.disjoint(write, bytes, source),
                    _ => true,
                }
        });
        env.stats.killed += before - env.mem.len();
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if matches!(env.reg_fact(reg),Some(SsaLiteValueKey::DirectMem(source)) if !self.disjoint(write,bytes,source))
            {
                env.kill_reg(reg);
            }
        }
    }
    /// `value` describes the pre-write machine value, not a new load of source.
    pub fn store(
        &mut self,
        env: &mut SsaLiteValueEnv,
        dst: &MirMem,
        value: Option<SsaLiteValueKey>,
    ) {
        self.invalidate(env, dst, 1);
        if self.exhausted || !self.ordinary_byte(dst) {
            return;
        }
        let Some(value) = value else {
            return;
        };
        if let SsaLiteValueKey::DirectMem(source) = &value {
            // Unknown overlap and exact self-copies contribute no new fact.
            if !self.disjoint(dst, 1, source) {
                return;
            }
        }
        if env.mem.len() >= MAX_FACTS {
            env.kill_memory_dependencies();
            self.exhausted = true;
            return;
        }
        env.learn_mem(dst.clone(), value);
        self.peak = self.peak.max(env.mem.len());
    }
}

#[cfg(test)]
mod tests;
