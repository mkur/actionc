//! Byte lifetimes belong to one materialized routine. Reads and writes occupy
//! their instruction, including stores whose result is dead.
use super::super::logical::{self, ObjectByte};
use super::*;
use crate::mir6502::analysis::sites::MirSite;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Byte {
    Source(ObjectByte),
    Home(MirHomeByte),
}

#[derive(Default)]
pub(super) struct Lifetime {
    pub occupied: BTreeSet<MirSite>,
    pub across_calls: BTreeSet<MirSite>,
}

pub(super) struct FrameLives {
    pub temps: MirHomeLiveness,
    bytes: BTreeMap<Byte, Lifetime>,
}

impl FrameLives {
    pub fn new(r: &MirRoutine, cfg: &MirCfg) -> Option<Self> {
        Self::with_temps(r, cfg, MirHomeLiveness::analyze(r, cfg))
    }

    pub fn with_temps(r: &MirRoutine, cfg: &MirCfg, temps: MirHomeLiveness) -> Option<Self> {
        let mut bytes = BTreeMap::new();
        let calls: BTreeSet<_> = r
            .blocks
            .iter()
            .flat_map(|b| {
                b.ops
                    .iter()
                    .enumerate()
                    .filter(|(_, op)| {
                        matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. })
                    })
                    .map(|(op_index, _)| MirSite::Op {
                        block: b.id,
                        op_index,
                    })
            })
            .collect();
        let mut collect =
            |live: &MirHomeLiveness, identify: &dyn Fn(MirHomeByte) -> Option<Byte>| {
                live.visit_sites(|site, transfer, after| {
                    for key in after
                        .iter()
                        .chain(transfer.reads.iter().chain(&transfer.writes).copied())
                    {
                        if let Some(byte) = identify(key) {
                            let range: &mut Lifetime = bytes.entry(byte).or_default();
                            range.occupied.insert(site);
                            if calls.contains(&site) && after.contains(key) {
                                range.across_calls.insert(site);
                            }
                        }
                    }
                });
            };
        collect(&temps, &|key| Some(Byte::Home(key)));
        let mut objects = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (mem, _) in r.frame.resident_bytes() {
            let object = ObjectByte::from_mem(&mem, 0)?;
            let key = MirHomeByte::Spill {
                id: MirSpillId(u32::try_from(names.len()).ok()?),
                offset: 0,
            };
            objects.insert(object, key);
            names.insert(key, object);
        }
        if !names.is_empty() {
            let named = logical::analyze(r, cfg, |mem, delta| {
                objects.get(&ObjectByte::from_mem(mem, delta)?).copied()
            });
            collect(&named, &|key| Some(Byte::Source(*names.get(&key)?)));
        }
        // Possible writes interfere even when the effect cannot kill the
        // reaching value. The liveness transfer includes only definite kills.
        for b in &r.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                let site = MirSite::Op {
                    block: b.id,
                    op_index: i,
                };
                for key in classify_op(op).homes.writes {
                    bytes
                        .entry(Byte::Home(key))
                        .or_default()
                        .occupied
                        .insert(site);
                }
            }
        }
        Some(Self { temps, bytes })
    }

    pub fn disjoint(&self, left: Byte, right: Byte) -> bool {
        match (self.get(left), self.get(right)) {
            (Some(a), Some(b)) => a.occupied.is_disjoint(&b.occupied),
            // An unused named byte has no accesses in the body. Parameter
            // capture precedes any eligible spill's first definition.
            (None, _) => matches!(left, Byte::Source(_)),
            (_, None) => matches!(right, Byte::Source(_)),
        }
    }

    pub fn get(&self, byte: Byte) -> Option<&Lifetime> {
        self.bytes.get(&byte)
    }
}
