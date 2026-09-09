//! Syntactic storage references, including non-memory/ABI/effect operands.
//! This is a reference census, not an alias or liveness analysis.

use std::collections::BTreeSet;

use crate::nir::*;

pub(in crate::nir) fn op_references(op: &NirOp) -> BTreeSet<NirStorageId> {
    let mut refs = BTreeSet::new();
    match op {
        NirOp::Load { place, .. }
        | NirOp::VolatileLoad { place, .. }
        | NirOp::AddrOf { place, .. } => place_references(place, &mut refs),
        NirOp::Store { place, src, .. } | NirOp::VolatileStore { place, src, .. } => {
            place_references(place, &mut refs);
            value_references(src, &mut refs);
        }
        NirOp::CopyBytes {
            destination,
            source,
            ..
        } => {
            place_references(destination, &mut refs);
            place_references(source, &mut refs);
        }
        NirOp::Unary { src, .. } | NirOp::Cast { src, .. } => value_references(src, &mut refs),
        NirOp::PointerOffset { base, offset, .. } => {
            value_references(base, &mut refs);
            value_references(offset, &mut refs);
        }
        NirOp::Binary { left, right, .. } | NirOp::Compare { left, right, .. } => {
            value_references(left, &mut refs);
            value_references(right, &mut refs);
        }
        NirOp::Real(real) => match real {
            NirRealOp::Copy {
                destination,
                source,
            } => {
                place_references(destination, &mut refs);
                real_source_references(source, &mut refs);
            }
            NirRealOp::Unary {
                destination,
                operand,
                ..
            } => {
                place_references(destination, &mut refs);
                real_source_references(operand, &mut refs);
            }
            NirRealOp::Binary {
                destination,
                left,
                right,
                ..
            } => {
                place_references(destination, &mut refs);
                real_source_references(left, &mut refs);
                real_source_references(right, &mut refs);
            }
            NirRealOp::Compare { left, right, .. } => {
                real_source_references(left, &mut refs);
                real_source_references(right, &mut refs);
            }
            NirRealOp::IntegerToReal {
                destination,
                source,
                ..
            } => {
                place_references(destination, &mut refs);
                value_references(source, &mut refs);
            }
            NirRealOp::RealToInteger { source, .. } => place_references(source, &mut refs),
        },
        NirOp::Call {
            callee,
            args,
            aggregate_result,
            effects,
            ..
        } => {
            if let NirCallee::Indirect { target, .. } = callee {
                value_references(target, &mut refs);
            }
            for arg in args {
                value_references(arg, &mut refs);
            }
            if let Some(place) = aggregate_result {
                place_references(place, &mut refs);
            }
            effect_references(&effects.memory, &mut refs);
        }
        NirOp::ForeignCode { code, effects } => {
            effect_references(&effects.memory, &mut refs);
            let mut target = |target: &NirForeignCodeTarget| {
                if let NirForeignCodeTarget::Storage(id) = target {
                    refs.insert(*id);
                }
            };
            match &code.payload {
                NirForeignCodePayload::Bytes { relocations, .. } => {
                    for relocation in relocations {
                        target(&relocation.target);
                    }
                }
                NirForeignCodePayload::Structured(items) => {
                    for item in items {
                        if let NirMachineItem::Relocation { target: id, .. } = item {
                            target(id);
                        }
                    }
                }
            }
        }
        NirOp::Unsupported { .. } => {}
    }
    refs
}

pub(in crate::nir) fn terminator_references(term: &NirTerminator) -> BTreeSet<NirStorageId> {
    let mut refs = BTreeSet::new();
    match term {
        NirTerminator::Return(Some(value)) => value_references(value, &mut refs),
        NirTerminator::Goto(edge) => {
            for arg in &edge.args {
                value_references(arg, &mut refs);
            }
        }
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            value_references(condition, &mut refs);
            for arg in then_edge.args.iter().chain(&else_edge.args) {
                value_references(arg, &mut refs);
            }
        }
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Exit
        | NirTerminator::Return(None) => {}
    }
    refs
}

pub(in crate::nir) fn routine_references(routine: &NirRoutine) -> BTreeSet<NirStorageId> {
    let mut refs = BTreeSet::new();
    for block in &routine.blocks {
        for op in &block.ops {
            refs.extend(op_references(op));
        }
        refs.extend(terminator_references(&block.terminator));
    }
    for local in &routine.locals {
        match local.backing {
            NirLocalBacking::Alias { target, .. } => {
                refs.insert(NirStorageId::Local(target));
            }
            NirLocalBacking::GlobalAlias { target, .. } => {
                refs.insert(NirStorageId::Global(target));
            }
            NirLocalBacking::Ordinary | NirLocalBacking::Absolute(_) => {}
        }
        if let NirLocalPurpose::AggregateBacking { owner } = local.purpose {
            refs.insert(NirStorageId::Local(owner));
        }
    }
    // Program-wide data relocations are covered separately by storage facts'
    // address_in_data flag (including global/static/local initializer images).
    refs
}

fn effect_references(effects: &NirMemoryEffects, refs: &mut BTreeSet<NirStorageId>) {
    for access in [&effects.reads, &effects.writes] {
        if let NirMemoryAccess::Regions(regions) = access {
            for region in regions {
                if let NirMemoryRegionKind::Storage(id) = region.kind {
                    refs.insert(id);
                }
            }
        }
    }
}

fn real_source_references(source: &NirRealSource, refs: &mut BTreeSet<NirStorageId>) {
    if let NirRealSource::Place(place) = source {
        place_references(place, refs);
    }
}

fn place_references(place: &NirPlace, refs: &mut BTreeSet<NirStorageId>) {
    match &place.kind {
        NirPlaceKind::Local { id, .. } => {
            refs.insert(NirStorageId::Local(*id));
        }
        NirPlaceKind::Param { id, .. } => {
            refs.insert(NirStorageId::Param(*id));
        }
        NirPlaceKind::Global { id, .. } => {
            refs.insert(NirStorageId::Global(*id));
        }
        NirPlaceKind::Field { base, .. } => place_references(base, refs),
        NirPlaceKind::Deref { addr } => value_references(addr, refs),
        NirPlaceKind::Index {
            base_addr, index, ..
        } => {
            value_references(base_addr, refs);
            value_references(index, refs);
        }
        NirPlaceKind::Absolute(_) => {}
    }
}

fn value_references(value: &NirValue, refs: &mut BTreeSet<NirStorageId>) {
    match value {
        NirValue::Aggregate { place } => place_references(place, refs),
        NirValue::GlobalAddr(id) => {
            refs.insert(NirStorageId::Global(*id));
        }
        NirValue::Param(id) => {
            refs.insert(NirStorageId::Param(*id));
        }
        NirValue::IntegerConst { .. }
        | NirValue::Null { .. }
        | NirValue::AddressConst { .. }
        | NirValue::StaticAddr { .. }
        | NirValue::Temp { .. }
        | NirValue::RoutineAddr { .. } => {}
    }
}
