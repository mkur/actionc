//! Structured diagnostic projections of existing storage/effect facts.
use crate::mir6502::analysis::effects::{classify_op, classify_terminator};
use crate::mir6502::ir::*;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Home {
    pub kind: &'static str,
    pub routine: Option<u32>,
    pub id: u32,
    pub offset: u16,
}

pub(crate) fn memory(owner: RoutineId, mem: &MirMem) -> Home {
    let (kind, routine, id, offset) = match *mem {
        MirMem::Param { id, offset } => ("param", Some(owner.0), id.0, offset),
        MirMem::Local { id, offset } => ("local", Some(owner.0), id.0, offset),
        MirMem::Spill { id, offset } => ("spill", Some(owner.0), id.0, offset),
        MirMem::ZeroPage(id) => ("virtual-zp", Some(owner.0), id.0, 0),
        MirMem::Global { id, offset } => ("global", None, id.0, offset),
        MirMem::Static { id, offset } => ("static", None, id.0, offset),
        MirMem::Absolute(a) => ("absolute", None, u32::from(a), 0),
        MirMem::FixedZeroPage(a) => ("fixed-zp", None, u32::from(a.0), 0),
    };
    Home {
        kind,
        routine,
        id,
        offset,
    }
}

pub(crate) fn frame(r: &MirRoutine) -> Value {
    let slots = r
        .frame
        .params
        .iter()
        .chain(&r.frame.locals)
        .map(|s| {
            let (home, alias, abi_only) = match s.base {
                MirStorageBase::Param(id) => (
                    memory(
                        r.id,
                        &MirMem::Param {
                            id,
                            offset: s.offset,
                        },
                    ),
                    None,
                    false,
                ),
                MirStorageBase::ParamAbiOnly(id) => (
                    memory(
                        r.id,
                        &MirMem::Param {
                            id,
                            offset: s.offset,
                        },
                    ),
                    None,
                    true,
                ),
                MirStorageBase::Local(id) => (
                    memory(
                        r.id,
                        &MirMem::Local {
                            id,
                            offset: s.offset,
                        },
                    ),
                    None,
                    false,
                ),
                MirStorageBase::LocalAlias { id, target } => (
                    memory(
                        r.id,
                        &MirMem::Local {
                            id,
                            offset: s.offset,
                        },
                    ),
                    Some(target.0),
                    false,
                ),
                MirStorageBase::Spill(id) => (
                    memory(
                        r.id,
                        &MirMem::Spill {
                            id,
                            offset: s.offset,
                        },
                    ),
                    None,
                    false,
                ),
                MirStorageBase::Global(id) => (
                    memory(
                        r.id,
                        &MirMem::Global {
                            id,
                            offset: s.offset,
                        },
                    ),
                    None,
                    false,
                ),
                MirStorageBase::Static(id) => (
                    memory(
                        r.id,
                        &MirMem::Static {
                            id,
                            offset: s.offset,
                        },
                    ),
                    None,
                    false,
                ),
                MirStorageBase::Absolute(a) => (
                    memory(r.id, &MirMem::Absolute(a.wrapping_add(s.offset))),
                    None,
                    false,
                ),
            };
            json!({"slot":s.id.0,"home":home,"bytes":s.storage_size,"name":s.name,
            "purpose":format!("{:?}",s.purpose),"alias_local":alias,"abi_only":abi_only})
        })
        .collect::<Vec<_>>();
    json!({"routine":r.id.0,"slots":slots,
        "spills":r.frame.spills.iter().map(|id|memory(r.id,&MirMem::Spill{id:*id,offset:0})).collect::<Vec<_>>(),
        "virtual_zp":r.frame.virtual_zero_page.iter().map(|id|memory(r.id,&MirMem::ZeroPage(*id))).collect::<Vec<_>>(),
        "fixed_zp":r.frame.fixed_zero_page.iter().map(|id|memory(r.id,&MirMem::FixedZeroPage(*id))).collect::<Vec<_>>(),
        "zp_allocations":r.frame.zero_page_allocations.iter().map(|a|json!({"home":memory(r.id,&MirMem::ZeroPage(a.slot)),"start":a.start.0,"bytes":a.size,"layout":"provisional"})).collect::<Vec<_>>()})
}

#[derive(Default, Serialize)]
struct Accesses {
    reads: usize,
    writes: usize,
    address_references: usize,
    references: usize,
    terminator_reads: usize,
    read_points: Vec<Value>,
    write_points: Vec<Value>,
}

/// Counts describe explicit accesses, never a deadness/alias proof. Opaque and
/// indirect observations have separate points even when every explicit count is zero.
pub(crate) fn accesses(r: &MirRoutine) -> Value {
    let mut homes = BTreeMap::<Home, Accesses>::new();
    let mut unknown = Vec::new();
    for block in &r.blocks {
        let effects = block
            .ops
            .iter()
            .enumerate()
            .map(|(i, op)| {
                let e = classify_op(op);
                let compat = e.may_read_unknown_spill_byte_compat();
                (Some(i), e.memory, e.homes, compat)
            })
            .chain(std::iter::once_with(|| {
                let e = classify_terminator(&block.terminator);
                (None, e.memory, e.homes, false)
            }));
        for (op, memory_effects, home_effects, compat) in effects {
            let point = json!({"block":block.id.0,"op":op});
            for read in &memory_effects.direct_reads {
                for n in 0..read.bytes {
                    let mut home = memory(r.id, &read.base);
                    if home.kind == "absolute" {
                        home.id = u32::from((home.id as u16).saturating_add(n));
                    } else if home.kind == "fixed-zp" {
                        home.id = u32::from((home.id as u8).saturating_add(n as u8));
                    } else {
                        home.offset = home.offset.saturating_add(n);
                    }
                    let access = homes.entry(home).or_default();
                    access.reads += 1;
                    access.terminator_reads += usize::from(op.is_none());
                    access.read_points.push(point.clone());
                }
            }
            for write in &memory_effects.direct_writes {
                for n in 0..write.bytes {
                    let mut home = memory(r.id, &write.base);
                    if home.kind == "absolute" {
                        home.id = u32::from((home.id as u16).saturating_add(n));
                    } else if home.kind == "fixed-zp" {
                        home.id = u32::from((home.id as u8).saturating_add(n as u8));
                    } else {
                        home.offset = home.offset.saturating_add(n);
                    }
                    let access = homes.entry(home).or_default();
                    access.writes += 1;
                    access.write_points.push(point.clone());
                }
            }
            for mem in &memory_effects.direct_references {
                homes.entry(memory(r.id, mem)).or_default().references += 1;
            }
            for mem in &memory_effects.address_references {
                homes
                    .entry(memory(r.id, mem))
                    .or_default()
                    .address_references += 1;
            }
            if memory_effects.indirect_reads
                || memory_effects.indirect_writes
                || memory_effects.opaque
                || memory_effects.has_unknown_effects
                || home_effects.unknown_reads
                || home_effects.unknown_writes
            {
                unknown.push(json!({"point":point,"indirect_read":memory_effects.indirect_reads,"indirect_write":memory_effects.indirect_writes,
                    "opaque":memory_effects.opaque,"unknown_effects":memory_effects.has_unknown_effects,
                    "unknown_home_read":home_effects.unknown_reads,"unknown_home_write":home_effects.unknown_writes,
                    "compat_unknown_spill_reader":compat}));
            }
        }
    }
    // Keep declared but unreferenced homes visible as distinct facts.
    for id in &r.frame.spills {
        homes
            .entry(memory(r.id, &MirMem::Spill { id: *id, offset: 0 }))
            .or_default();
    }
    for id in &r.frame.virtual_zero_page {
        homes
            .entry(memory(r.id, &MirMem::ZeroPage(*id)))
            .or_default();
    }
    json!({"routine":r.id.0,"homes":homes.into_iter().map(|(home,counts)|json!({"home":home,"counts":counts})).collect::<Vec<_>>(),"non_explicit_observations":unknown,
        "interpretation":"zero explicit reads is not a deadness proof"})
}
