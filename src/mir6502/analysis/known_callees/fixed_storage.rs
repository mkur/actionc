//! Source-fixed address evidence for one analysis of one program.
//! Logical footprints remain unchanged; no caller frame resolves callee homes.
use super::{MirMem, MirProgram, fixed_storage_address};
use crate::nir::SymbolId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirFixedStorage {
    globals: BTreeMap<SymbolId, u16>,
}

impl MirFixedStorage {
    pub(in crate::mir6502) fn new(program: &MirProgram) -> Self {
        Self {
            globals: program.globals.iter().filter_map(|global| {
                fixed_storage_address(program, &MirMem::Global { id: global.id, offset: 0 })
                    .map(|address| (global.id, address))
            }).collect(),
        }
    }

    pub(in crate::mir6502) fn address(&self, memory: &MirMem) -> Option<u16> {
        match *memory {
            MirMem::Absolute(address) => Some(address),
            MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
            MirMem::Global { id, offset } => self.globals.get(&id)?.checked_add(offset),
            _ => None,
        }
    }

    pub(in crate::mir6502) fn may_overlap(&self, left: &MirMem, right: &MirMem) -> bool {
        match (self.address(left), self.address(right)) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{MirFixedZpSlot, MirGlobal, MirGlobalBacking, MirSpillId, MirWidth};
    use crate::nir::{LocalId, ParamId};

    fn global(id: u32, backing: MirGlobalBacking) -> MirGlobal {
        MirGlobal { id: SymbolId(id), name: format!("g{id}"), kind: "word".into(),
            width: Some(MirWidth::Word), storage_size: 2, backing, init: None }
    }
    fn mem(id: u32, offset: u16) -> MirMem { MirMem::Global { id: SymbolId(id), offset } }
    fn program(globals: Vec<MirGlobal>) -> MirProgram {
        MirProgram { globals, statics: vec![], routines: vec![], machine_blocks: vec![], runtime_helpers: vec![] }
    }

    #[test]
    fn fixed_storage_overlap_uses_backing_and_all_alias_offsets() {
        let p = program(vec![
            global(0, MirGlobalBacking::Absolute(0x600)),
            global(1, MirGlobalBacking::Absolute(0xe4)),
            global(2, MirGlobalBacking::Alias { target: SymbolId(1), offset: 1 }),
            global(3, MirGlobalBacking::Alias { target: SymbolId(2), offset: 1 }),
            global(4, MirGlobalBacking::Absolute(0xe7)),
        ]);
        let fixed = MirFixedStorage::new(&p);
        assert!(!fixed.may_overlap(&mem(0, 0), &MirMem::FixedZeroPage(MirFixedZpSlot(0xe4))));
        assert!(fixed.may_overlap(&mem(1, 0), &MirMem::FixedZeroPage(MirFixedZpSlot(0xe4))));
        assert!(!fixed.may_overlap(&mem(1, 0), &mem(1, 1)));
        assert_eq!(fixed.address(&mem(3, 1)), Some(0xe7));
        for alias in [mem(4, 0), MirMem::Absolute(0xe7), MirMem::FixedZeroPage(MirFixedZpSlot(0xe7))] {
            assert!(fixed.may_overlap(&mem(3, 1), &alias));
            assert!(fixed.may_overlap(&alias, &mem(3, 1)));
        }
    }

    #[test]
    fn fixed_storage_missing_invalid_and_private_addresses_remain_unknown() {
        let p = program(vec![
            global(0, MirGlobalBacking::Ordinary { offset: 0 }),
            global(1, MirGlobalBacking::Alias { target: SymbolId(99), offset: 0 }),
            global(2, MirGlobalBacking::Alias { target: SymbolId(3), offset: 0 }),
            global(3, MirGlobalBacking::Alias { target: SymbolId(2), offset: 0 }),
            global(4, MirGlobalBacking::Absolute(0xffff)),
            global(5, MirGlobalBacking::Alias { target: SymbolId(4), offset: 1 }),
            global(6, MirGlobalBacking::Alias { target: SymbolId(5), offset: 0xffff }),
        ]);
        let fixed = MirFixedStorage::new(&p);
        for unresolved in [mem(0, 0), mem(1, 0), mem(2, 0), mem(3, 0), mem(4, 1),
            mem(5, 0), mem(6, 1), mem(99, 0),
            MirMem::Local { id: LocalId(0), offset: 0 },
            MirMem::Param { id: ParamId(0), offset: 0 },
            MirMem::Spill { id: MirSpillId(0), offset: 0 },
            MirMem::Static { id: SymbolId(0), offset: 0 },
            MirMem::ZeroPage(crate::mir6502::ir::MirZpSlot(0)),
        ] {
            assert_eq!(fixed.address(&unresolved), None, "{unresolved:?}");
            assert!(fixed.may_overlap(&unresolved, &MirMem::Absolute(0xe4)), "{unresolved:?}");
            assert!(fixed.may_overlap(&MirMem::Absolute(0xe4), &unresolved), "{unresolved:?}");
        }
        assert_eq!(fixed.address(&mem(4, 0)), Some(0xffff));
    }

    #[test]
    fn fixed_storage_rebuild_observes_backing_changes_without_body_changes() {
        let mut p = program(vec![global(0, MirGlobalBacking::Absolute(0x600)),
            global(1, MirGlobalBacking::Alias { target: SymbolId(0), offset: 1 })]);
        let old = MirFixedStorage::new(&p);
        p.globals[0].backing = MirGlobalBacking::Absolute(0xe4);
        let candidate = MirFixedStorage::new(&p);
        let byte = MirMem::Absolute(0xe5);
        assert!(!old.may_overlap(&mem(1, 0), &byte));
        assert!(candidate.may_overlap(&mem(1, 0), &byte));
    }
}
