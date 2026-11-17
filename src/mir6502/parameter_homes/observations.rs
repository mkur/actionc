//! Read-only certificates tied to one expanded program. The borrowed source
//! fixes body, binding, exposure and callee dependencies for their whole use.
use super::*;
use crate::mir6502::analysis::{
    effects::{MirHomeByte, classify_op},
    home_liveness::action_return_home_uses,
    record_bases::{SourceBinding, SourceObject, SourceStorage},
    sites::MirSite,
};
use std::collections::BTreeMap;

pub(in crate::mir6502) struct InvocationHomes<'a> {
    source: &'a MirProgram,
    abi: BTreeSet<MirFixedZpSlot>,
    objects: BTreeMap<RoutineId, Vec<(MirStorageBase, Vec<MirFixedZpSlot>)>>,
}

impl<'a> InvocationHomes<'a> {
    pub fn is_current(&self, source: &MirProgram) -> bool {
        std::ptr::eq(self.source, source)
    }

    #[cfg(test)]
    pub fn new(source: &'a MirProgram) -> Self { Self::with_call_scope(source, false) }

    fn with_call_scope(source: &'a MirProgram, acyclic: bool) -> Self {
        let addresses = ObjectAddressControl::new(source);
        let mut objects = BTreeMap::new();
        let mut calls = if acyclic { ResidenceCalls::with_acyclic(source, MirCalleeEntryStage::ActualEntry) }
            else { ResidenceCalls::new(source, MirCalleeEntryStage::ActualEntry) };
        let mut abi: BTreeSet<_> = action_return_home_uses().iter().filter_map(|h| {
            if let MirHomeByte::FixedZeroPage(s) = h { Some(s) } else { None }
        }).collect();
        for r in &source.routines {
            if let Some(widths) = &r.frame.direct_parameter_abi {
                let mut offset = 0;
                for parameter in widths {
                    for &width in parameter.width().transfers() {
                        for lane in crate::mir6502::abi::physical_lanes(
                            &crate::mir6502::abi::action_arg_home(offset, width), width,
                        ).unwrap_or_default() {
                            if let MirParamLane::ZeroPage(s) = lane { abi.insert(s); }
                        }
                        offset += crate::mir6502::abi::action_arg_width_bytes(width);
                    }
                }
            }
            for op in r.blocks.iter().flat_map(|b| &b.ops) {
                if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. } | MirOp::SArgsCapture { .. }) {
                    abi.extend(classify_op(op).homes.reads.iter().filter_map(|h| {
                        if let MirHomeByte::FixedZeroPage(s) = h { Some(*s) } else { None }
                    }));
                }
            }
        }
        for r in &source.routines {
            if r.frame.resident_bytes().is_empty() || r.abi != MirRoutineAbi::Action
                || r.frame.parameter_handoff.is_some()
                || !verify_bindings(r).is_empty()
                || !verify_storage_with_address_control(source, r, MirCalleeEntryStage::ActualEntry, &addresses).is_empty()
                || r.blocks.iter().flat_map(|b| &b.ops).any(|op| match op {
                    MirOp::RuntimeHelper { .. } | MirOp::SArgsCapture { .. } | MirOp::MachineBlock { .. } | MirOp::Barrier { .. } => true,
                    _ => false,
                })
                || !verify_residence(r, Some(&mut calls), Some(&addresses)).is_empty()
            { continue; }
            let Some(entry) = lifetimes::entry_dependencies(r) else { continue; };
            let mut certified = Vec::new();
            for slot in r.frame.params.iter().chain(&r.frame.locals) {
                let start = match slot.base {
                    MirStorageBase::Param(id) => match r.frame.parameter_home(id) {
                        Some(MirParamHome::ZeroPage(s)) => Some(*s), _ => None,
                    },
                    MirStorageBase::Local(id) => match r.frame.local_home(id) {
                        Some(MirLocalHome::ZeroPage(s)) => Some(*s), _ => None,
                    },
                    _ => None,
                };
                let Some(start) = start else { continue; };
                let bytes: Vec<_> = (0..slot.storage_size).map(|i| MirFixedZpSlot(start.0 + i as u8)).collect();
                if bytes.iter().any(|s| abi.contains(s) || entry.contains(&MirHomeByte::FixedZeroPage(*s))) { continue; }
                certified.push((slot.base.clone(), bytes));
            }
            objects.insert(r.id, certified);
        }
        Self { source, objects, abi }
    }

    #[cfg(test)]
    pub fn with_references(source: &'a MirProgram, references: &[&MirProgram]) -> Self {
        Self::with_references_and_calls(source, references, false)
    }

    pub fn with_references_and_calls(source: &'a MirProgram, references: &[&MirProgram], acyclic: bool) -> Self {
        let mut facts = Self::with_call_scope(source, acyclic);
        let references: Vec<_> = references.iter().map(|p| ObjectAddressControl::new(p)).collect();
        for (id, objects) in &mut facts.objects {
            objects.retain(|(base, _)| {
                let object = match *base {
                    MirStorageBase::Param(id) => MirMem::Param { id, offset: 0 },
                    MirStorageBase::Local(id) => MirMem::Local { id, offset: 0 },
                    _ => return false,
                };
                references.iter().all(|p| p.compiler_controls_address(*id, &object))
            });
        }
        facts
    }

    pub fn storage_rejection(&self, record: &SourceStorage) -> Option<&'static str> {
        if !matches!(&record.binding, Some(SourceBinding::Parameter(MirParamHome::ZeroPage(_)))
            | Some(SourceBinding::Local(MirLocalHome::ZeroPage(_)))) {
            return Some("source-declaration");
        }
        if self.abi.iter().any(|s| record.contains(u16::from(s.0))) { return Some("abi-boundary"); }
        (!self.certifies_storage(record)).then_some("uncertified-invocation-home")
    }

    /// There is no API accepting another body or generation: every site below
    /// is looked up in this immutable source, never in an edited candidate.
    pub fn certifies_storage(&self, record: &SourceStorage) -> bool {
        let SourceObject::Routine { owner, slot, base } = &record.object else { return false; };
        let Some(r) = self.source.routines.iter().find(|r| r.id == *owner) else { return false; };
        let Some(decl) = r.frame.params.iter().chain(&r.frame.locals).find(|s| s.id == *slot && s.base == *base) else { return false; };
        let binding = match *base {
            MirStorageBase::Param(id) => r.frame.parameter_home(id).cloned().map(SourceBinding::Parameter),
            MirStorageBase::Local(id) => r.frame.local_home(id).cloned().map(SourceBinding::Local),
            _ => None,
        };
        record.offset == 0 && record.bytes == decl.storage_size && record.binding == binding
            && self.objects.get(owner).is_some_and(|objects| objects.iter().any(|(b, lanes)|
                b == base && lanes.first().is_some_and(|s| record.address == u16::from(s.0))))
    }

    pub fn read_object(&self, owner: RoutineId, site: MirSite, mem: &MirMem) -> Option<MirStorageBase> {
        let MirSite::Op { block, op_index } = site else { return None; };
        let r = self.source.routines.iter().find(|r| r.id == owner)?;
        let op = r.blocks.iter().find(|b| b.id == block)?.ops.get(op_index)?;
        if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. } | MirOp::SArgsCapture { .. } | MirOp::MachineBlock { .. } | MirOp::Barrier { .. }) { return None; }
        let e = classify_op(op);
        let observed = e.memory.direct_reads.iter().any(|range| (0..range.bytes).any(|i| {
            let byte = match range.base {
                MirMem::Param { id, offset } => offset.checked_add(i).map(|offset| MirMem::Param { id, offset }),
                MirMem::Local { id, offset } => offset.checked_add(i).map(|offset| MirMem::Local { id, offset }),
                MirMem::FixedZeroPage(s) => u16::from(s.0).checked_add(i).filter(|a| *a < 256)
                    .map(|a| MirMem::FixedZeroPage(MirFixedZpSlot(a as u8))),
                _ => None,
            };
            byte.as_ref() == Some(mem)
        })) || matches!(mem, MirMem::FixedZeroPage(s) if e.addresses.pair_reads.contains(&MirHomeByte::FixedZeroPage(*s)));
        if !observed { return None; }
        let (base, physical) = match mem {
            MirMem::Param { id, offset } => (Some((MirStorageBase::Param(*id), *offset)), None),
            MirMem::Local { id, offset } => (Some((MirStorageBase::Local(*id), *offset)), None),
            MirMem::FixedZeroPage(s) => (None, Some(*s)),
            _ => return None, // raw numeric and assembly observations stay physical
        };
        self.objects.get(&owner)?.iter().find_map(|(object, bytes)| {
            let matches = base.as_ref().is_some_and(|(b, offset)| b == object && usize::from(*offset) < bytes.len())
                || physical.is_some_and(|s| bytes.contains(&s));
            matches.then(|| object.clone())
        })
    }
}
