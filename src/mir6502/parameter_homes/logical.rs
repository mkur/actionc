//! Snapshot-local source-value liveness, independent of physical placement.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{MirHomeByte, MirMemoryEffects, classify_op, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
    sites::MirSite,
};
use crate::nir::{LocalId, ParamId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ObjectByte {
    Param(ParamId, u16),
    Local(LocalId, u16),
}

impl ObjectByte {
    pub fn from_mem(mem: &MirMem, delta: u16) -> Option<Self> {
        match mem {
            MirMem::Param { id, offset } => Some(Self::Param(*id, offset.checked_add(delta)?)),
            MirMem::Local { id, offset } => Some(Self::Local(*id, offset.checked_add(delta)?)),
            _ => None,
        }
    }
}

pub(super) struct LogicalHomes<'a> {
    // The borrow prevents mutations while these facts are consumed. IDs/sites
    // are meaningful only within this owning routine snapshot.
    routine: &'a MirRoutine,
    keys: BTreeMap<ObjectByte, MirHomeByte>,
    live: MirHomeLiveness,
}

impl<'a> LogicalHomes<'a> {
    pub fn new(routine: &'a MirRoutine) -> Option<Self> {
        let cfg = MirCfg::from_routine(routine).ok()?;
        cfg.entry()?;
        let mut keys = BTreeMap::new();
        for slot in routine.frame.params.iter().chain(&routine.frame.locals) {
            let mem = match slot.base {
                MirStorageBase::Param(id) => MirMem::Param { id, offset: 0 },
                MirStorageBase::Local(id) => MirMem::Local { id, offset: 0 },
                _ => continue,
            };
            for offset in 0..slot.storage_size {
                let object = ObjectByte::from_mem(&mem, offset)?;
                // Dense solver keys only: no source object becomes a spill,
                // and no key or result is reused for another routine snapshot.
                let key = MirHomeByte::Spill {
                    id: MirSpillId(u32::try_from(keys.len()).ok()?),
                    offset: 0,
                };
                keys.insert(object, key);
            }
        }
        let key = |mem: &MirMem, delta| keys.get(&ObjectByte::from_mem(mem, delta)?).copied();
        let live = analyze(routine, &cfg, key);
        Some(Self {
            routine,
            keys,
            live,
        })
    }

    pub fn entry_dependencies(&self) -> Option<BTreeSet<ObjectByte>> {
        let entry = MirCfg::from_routine(self.routine).ok()?.entry()?;
        let live = self.live.live_in(entry)?;
        Some(
            self.keys
                .iter()
                .filter_map(|(object, key)| live.contains(*key).then_some(*object))
                .collect(),
        )
    }

    pub fn live_after(&self, mem: &MirMem, site: MirSite) -> Option<bool> {
        self.live
            .live_after(*self.keys.get(&ObjectByte::from_mem(mem, 0)?)?, site)
            .ok()
    }
}

pub(super) fn analyze(
    routine: &MirRoutine,
    cfg: &MirCfg,
    key: impl Fn(&MirMem, u16) -> Option<MirHomeByte>,
) -> MirHomeLiveness {
    let transfers = routine
        .blocks
        .iter()
        .map(|block| {
            let ops = block
                .ops
                .iter()
                .map(|op| {
                    let e = classify_op(op);
                    project(
                        routine,
                        &e.memory,
                        &e.homes
                            .reads
                            .union(&e.addresses.pair_reads)
                            .copied()
                            .collect(),
                        !matches!(op, MirOp::Call { .. }),
                        &key,
                    )
                })
                .collect();
            let e = classify_terminator(&block.terminator);
            (
                block.id,
                MirHomeBlockTransfers {
                    ops,
                    terminator: project(routine, &e.memory, &BTreeSet::new(), true, &key),
                },
            )
        })
        .collect();
    MirHomeLiveness::analyze_transfers(routine, cfg, transfers)
}

/// Only logical writes define source values. Explicit physical reads can
/// observe bound values; scratch/ABI/callee writes cannot initialize them.
pub(super) fn project(
    routine: &MirRoutine,
    memory: &MirMemoryEffects,
    extra_reads: &BTreeSet<MirHomeByte>,
    defines: bool,
    key: &impl Fn(&MirMem, u16) -> Option<MirHomeByte>,
) -> MirHomeTransfer {
    let resident = routine.frame.resident_bytes();
    let mut t = MirHomeTransfer::default();
    for range in &memory.direct_reads {
        for delta in 0..range.bytes {
            t.reads.extend(key(&range.base, delta));
            if let Some(address) = physical_slot(&range.base, delta) {
                for (mem, slot) in &resident {
                    if *slot == address {
                        t.reads.extend(key(mem, 0));
                    }
                }
            }
        }
    }
    for (mem, slot) in &resident {
        if extra_reads.contains(&MirHomeByte::FixedZeroPage(*slot)) {
            t.reads.extend(key(mem, 0));
        }
    }
    if defines {
        for range in &memory.direct_writes {
            for delta in 0..range.bytes {
                t.writes.extend(key(&range.base, delta));
            }
        }
    }
    t
}

pub(super) fn physical_slot(mem: &MirMem, offset: u16) -> Option<MirFixedZpSlot> {
    match mem {
        MirMem::FixedZeroPage(slot) => Some(MirFixedZpSlot(slot.0.wrapping_add(offset as u8))),
        MirMem::Absolute(address) if address.wrapping_add(offset) < 256 => {
            Some(MirFixedZpSlot(address.wrapping_add(offset) as u8))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        crate::mir6502::lower_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap()
    }

    #[test]
    fn source_values_cross_only_live_calls_and_keep_entry_dependencies() {
        for (body, entry, live_at_calls) in [
            ("n=x Touch(n) n=9 Touch(1)", false, vec![false, true]),
            ("Touch(x) n=x", false, vec![false]),
            ("IF x THEN n=1 FI Touch(1)", true, vec![true]),
            ("n=x WHILE n DO Touch(1) n==-1 OD", false, vec![true]),
            ("Touch(1)", true, vec![true]),
        ] {
            let p = source(&format!(
                "PROC Touch(CARD x) RETURN CARD FUNC Hold(CARD x) CARD n {body} RETURN(n)"
            ));
            let r = &p.routines[1];
            let MirStorageBase::Local(id) = r.frame.locals[0].base else {
                panic!()
            };
            let facts = LogicalHomes::new(r).unwrap();
            let dependencies = facts.entry_dependencies().unwrap();
            for offset in 0..2 {
                assert_eq!(
                    dependencies.contains(&ObjectByte::Local(id, offset)),
                    entry,
                    "{body}"
                );
                let calls: Vec<_> = r
                    .blocks
                    .iter()
                    .flat_map(|b| {
                        b.ops.iter().enumerate().filter_map(|(op_index, op)| {
                            matches!(op, MirOp::Call { .. }).then(|| {
                                facts
                                    .live_after(
                                        &MirMem::Local { id, offset },
                                        MirSite::Op {
                                            block: b.id,
                                            op_index,
                                        },
                                    )
                                    .unwrap()
                            })
                        })
                    })
                    .collect();
                assert_eq!(calls, live_at_calls, "{body}");
            }
        }
        let p =
            source("CARD FUNC First() CARD n RETURN(n) CARD FUNC Second() CARD n n=1 RETURN(n)");
        assert!(
            !LogicalHomes::new(&p.routines[0])
                .unwrap()
                .entry_dependencies()
                .unwrap()
                .is_empty()
        );
        assert!(
            LogicalHomes::new(&p.routines[1])
                .unwrap()
                .entry_dependencies()
                .unwrap()
                .is_empty()
        );
    }
}
