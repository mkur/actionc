use std::collections::BTreeMap;
use std::fmt::Write;

use super::analysis::use_def::NirUseDef;
use super::{
    NirOp, NirPlace, NirPlaceKind, NirProgram, NirPromotionBlocker, NirStorageId,
    analyze_program_storage,
};

const OP_KINDS: [&str; 16] = [
    "define",
    "set",
    "declare",
    "assign",
    "compound_assign",
    "load",
    "addr_of",
    "store",
    "unary",
    "cast",
    "binary",
    "compare",
    "call",
    "machine_block",
    "unsupported",
    "note",
];

const PLACE_KINDS: [&str; 9] = [
    "param",
    "local",
    "global",
    "absolute",
    "deref",
    "index",
    "field",
    "legacy_symbol",
    "unresolved",
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NirProgramStats {
    pub routines: usize,
    pub blocks: usize,
    pub operations: usize,
    pub temp_definitions: usize,
    pub cross_block_temp_uses: usize,
    pub block_parameters: usize,
    pub edge_arguments: usize,
    pub operation_kinds: BTreeMap<&'static str, usize>,
    pub loads: NirPlaceStats,
    pub stores: NirPlaceStats,
    pub storage: NirStorageStats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NirPlaceStats {
    pub total: usize,
    pub kinds: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NirStorageStats {
    pub homes: usize,
    pub promotable: usize,
    pub locals: NirStorageKindStats,
    pub params: NirStorageKindStats,
    pub globals: NirStorageKindStats,
    pub blocker_counts: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NirStorageKindStats {
    pub homes: usize,
    pub promotable: usize,
}

pub fn collect_program_stats(program: &NirProgram) -> NirProgramStats {
    let mut stats = NirProgramStats {
        routines: program.routines.len(),
        operation_kinds: zeroed_counts(&OP_KINDS),
        loads: NirPlaceStats {
            kinds: zeroed_counts(&PLACE_KINDS),
            ..NirPlaceStats::default()
        },
        stores: NirPlaceStats {
            kinds: zeroed_counts(&PLACE_KINDS),
            ..NirPlaceStats::default()
        },
        ..NirProgramStats::default()
    };

    for routine in &program.routines {
        stats.blocks += routine.blocks.len();
        stats.temp_definitions += routine.temps.len();

        let use_def = NirUseDef::from_routine(routine);
        for (temp, uses) in use_def.all_uses() {
            let Some(definition) = use_def.unique_definition(*temp) else {
                continue;
            };
            stats.cross_block_temp_uses += uses
                .iter()
                .filter(|use_site| use_site.block() != definition.block)
                .count();
        }

        for block in &routine.blocks {
            stats.operations += block.ops.len();
            for op in &block.ops {
                increment(&mut stats.operation_kinds, op_kind(op));
                match op {
                    NirOp::Load { place, .. } => stats.loads.record(place),
                    NirOp::Store { place, .. } => stats.stores.record(place),
                    NirOp::Define { .. }
                    | NirOp::Set { .. }
                    | NirOp::Declare { .. }
                    | NirOp::Assign { .. }
                    | NirOp::CompoundAssign { .. }
                    | NirOp::AddrOf { .. }
                    | NirOp::Unary { .. }
                    | NirOp::Cast { .. }
                    | NirOp::Binary { .. }
                    | NirOp::Compare { .. }
                    | NirOp::Call { .. }
                    | NirOp::MachineBlock { .. }
                    | NirOp::Unsupported { .. }
                    | NirOp::Note { .. } => {}
                }
            }
        }
    }

    // The current NIR contract has no block-parameter or edge-argument fields.
    // Keep the counters visible now so Phase 4 can populate them without
    // changing the census format.
    stats.block_parameters = 0;
    stats.edge_arguments = 0;
    stats.storage.blocker_counts = NirPromotionBlocker::ALL
        .iter()
        .map(|blocker| (blocker.code(), 0))
        .collect();
    for routine in analyze_program_storage(program).routines {
        for facts in routine.homes.values() {
            stats.storage.homes += 1;
            let kind = match facts.id {
                NirStorageId::Local(_) => &mut stats.storage.locals,
                NirStorageId::Param(_) => &mut stats.storage.params,
                NirStorageId::Global(_) => &mut stats.storage.globals,
            };
            kind.homes += 1;
            if facts.is_promotable() {
                stats.storage.promotable += 1;
                kind.promotable += 1;
            }
            for blocker in &facts.blockers {
                *stats
                    .storage
                    .blocker_counts
                    .entry(blocker.code())
                    .or_default() += 1;
            }
        }
    }
    stats
}

pub fn format_stats_comparison(lowered: &NirProgram, optimized: &NirProgram) -> String {
    let lowered = collect_program_stats(lowered);
    let optimized = collect_program_stats(optimized);
    let mut output = String::new();
    writeln!(output, "nir statistics").expect("write NIR statistics header");
    write_stage(&mut output, "lowered", &lowered);
    write_stage(&mut output, "optimized", &optimized);
    writeln!(output, "optimizer_total").expect("write NIR optimizer total header");
    write_change(
        &mut output,
        "operations",
        lowered.operations,
        optimized.operations,
    );
    write_change(
        &mut output,
        "temp_definitions",
        lowered.temp_definitions,
        optimized.temp_definitions,
    );
    write_change(
        &mut output,
        "loads",
        lowered.loads.total,
        optimized.loads.total,
    );
    write_change(
        &mut output,
        "stores",
        lowered.stores.total,
        optimized.stores.total,
    );
    output
}

impl NirPlaceStats {
    fn record(&mut self, place: &NirPlace) {
        self.total += 1;
        increment(&mut self.kinds, place_kind(place));
    }
}

fn write_stage(output: &mut String, name: &str, stats: &NirProgramStats) {
    writeln!(output, "stage {name}").expect("write NIR statistics stage");
    writeln!(output, "routines={}", stats.routines).expect("write NIR routine count");
    writeln!(output, "blocks={}", stats.blocks).expect("write NIR block count");
    writeln!(output, "operations={}", stats.operations).expect("write NIR operation count");
    writeln!(output, "temp_definitions={}", stats.temp_definitions).expect("write NIR temp count");
    writeln!(
        output,
        "cross_block_temp_uses={}",
        stats.cross_block_temp_uses
    )
    .expect("write NIR cross-block temp-use count");
    writeln!(output, "block_parameters={}", stats.block_parameters)
        .expect("write NIR block-parameter count");
    writeln!(output, "edge_arguments={}", stats.edge_arguments)
        .expect("write NIR edge-argument count");
    for kind in OP_KINDS {
        writeln!(
            output,
            "op.{kind}={}",
            stats.operation_kinds.get(kind).copied().unwrap_or(0)
        )
        .expect("write NIR operation-kind count");
    }
    write_place_stats(output, "load", &stats.loads);
    write_place_stats(output, "store", &stats.stores);
    writeln!(output, "storage.homes={}", stats.storage.homes)
        .expect("write NIR storage-home count");
    writeln!(output, "storage.promotable={}", stats.storage.promotable)
        .expect("write NIR promotable-storage count");
    write_storage_kind_stats(output, "local", &stats.storage.locals);
    write_storage_kind_stats(output, "param", &stats.storage.params);
    write_storage_kind_stats(output, "global", &stats.storage.globals);
    for blocker in NirPromotionBlocker::ALL {
        writeln!(
            output,
            "storage.blocker.{}={}",
            blocker.code(),
            stats
                .storage
                .blocker_counts
                .get(blocker.code())
                .copied()
                .unwrap_or(0)
        )
        .expect("write NIR storage-blocker count");
    }
}

fn write_storage_kind_stats(output: &mut String, name: &str, stats: &NirStorageKindStats) {
    writeln!(output, "storage.{name}.homes={}", stats.homes)
        .expect("write NIR storage-kind home count");
    writeln!(output, "storage.{name}.promotable={}", stats.promotable)
        .expect("write NIR storage-kind promotable count");
}

fn write_place_stats(output: &mut String, prefix: &str, stats: &NirPlaceStats) {
    writeln!(output, "{prefix}.total={}", stats.total).expect("write NIR place total");
    for kind in PLACE_KINDS {
        writeln!(
            output,
            "{prefix}.{kind}={}",
            stats.kinds.get(kind).copied().unwrap_or(0)
        )
        .expect("write NIR place-kind count");
    }
}

fn write_change(output: &mut String, name: &str, lowered: usize, optimized: usize) {
    let removed = lowered.saturating_sub(optimized);
    let added = optimized.saturating_sub(lowered);
    writeln!(output, "{name}.removed={removed}").expect("write NIR removed count");
    writeln!(output, "{name}.added={added}").expect("write NIR added count");
}

fn zeroed_counts(keys: &[&'static str]) -> BTreeMap<&'static str, usize> {
    keys.iter().copied().map(|key| (key, 0)).collect()
}

fn increment(counts: &mut BTreeMap<&'static str, usize>, key: &'static str) {
    *counts.entry(key).or_default() += 1;
}

fn op_kind(op: &NirOp) -> &'static str {
    match op {
        NirOp::Define { .. } => "define",
        NirOp::Set { .. } => "set",
        NirOp::Declare { .. } => "declare",
        NirOp::Assign { .. } => "assign",
        NirOp::CompoundAssign { .. } => "compound_assign",
        NirOp::Load { .. } => "load",
        NirOp::AddrOf { .. } => "addr_of",
        NirOp::Store { .. } => "store",
        NirOp::Unary { .. } => "unary",
        NirOp::Cast { .. } => "cast",
        NirOp::Binary { .. } => "binary",
        NirOp::Compare { .. } => "compare",
        NirOp::Call { .. } => "call",
        NirOp::MachineBlock { .. } => "machine_block",
        NirOp::Unsupported { .. } => "unsupported",
        NirOp::Note { .. } => "note",
    }
}

fn place_kind(place: &NirPlace) -> &'static str {
    match &place.kind {
        NirPlaceKind::Param { .. } => "param",
        NirPlaceKind::Local { .. } => "local",
        NirPlaceKind::Global { .. } => "global",
        NirPlaceKind::Absolute(_) => "absolute",
        NirPlaceKind::Deref { .. } => "deref",
        NirPlaceKind::Index { .. } => "index",
        NirPlaceKind::Field { .. } => "field",
        NirPlaceKind::Symbol(_) => "legacy_symbol",
        NirPlaceKind::UnresolvedName(_) => "unresolved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{
        BlockId, NirBlock, NirLocal, NirLocalBacking, NirRoutine, NirTerminator, NirType,
        NirTypeKind, NirValue, TempId,
    };

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(1),
            pointer: false,
        }
    }

    #[test]
    fn census_counts_cross_block_temp_uses_and_place_shapes() {
        let ty = byte_type();
        let local = NirLocal {
            id: crate::nir::LocalId(0),
            name: "value".to_string(),
            kind: "scalar".to_string(),
            storage: crate::nir::NirStorageClass::Scalar,
            ty: ty.clone(),
            backing: NirLocalBacking::Ordinary,
            init: None,
        };
        let place = NirPlace {
            kind: NirPlaceKind::Local {
                id: local.id,
                name: local.name.clone(),
            },
            ty: Some(ty.clone()),
        };
        let temp = TempId(0);
        let program = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: vec![local],
                temps: vec![crate::nir::NirTemp {
                    id: temp,
                    ty: ty.clone(),
                    def: crate::nir::NirTempDef {
                        block: BlockId(0),
                        op_index: 0,
                    },
                }],
                notes: Vec::new(),
                blocks: vec![
                    NirBlock {
                        id: BlockId(0),
                        label: "entry".to_string(),
                        ops: vec![NirOp::Load {
                            dest: temp,
                            ty: ty.clone(),
                            place: place.clone(),
                        }],
                        terminator: NirTerminator::Goto("use".to_string()),
                    },
                    NirBlock {
                        id: BlockId(1),
                        label: "use".to_string(),
                        ops: vec![NirOp::Store {
                            place,
                            src: NirValue::Temp { id: temp, ty },
                            ty: byte_type(),
                        }],
                        terminator: NirTerminator::Return(None),
                    },
                ],
            }],
        };

        let stats = collect_program_stats(&program);
        assert_eq!(stats.routines, 1);
        assert_eq!(stats.blocks, 2);
        assert_eq!(stats.operations, 2);
        assert_eq!(stats.temp_definitions, 1);
        assert_eq!(stats.cross_block_temp_uses, 1);
        assert_eq!(stats.loads.total, 1);
        assert_eq!(stats.loads.kinds["local"], 1);
        assert_eq!(stats.stores.total, 1);
        assert_eq!(stats.stores.kinds["local"], 1);
        assert_eq!(stats.storage.homes, 1);
        assert_eq!(stats.storage.promotable, 0);
        assert_eq!(stats.storage.locals.homes, 1);
        assert_eq!(stats.storage.blocker_counts["read_before_definition"], 1);
    }
}
