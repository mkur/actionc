//! Conservative natural-loop transforms for captured index arithmetic.
use super::*;

#[derive(Clone)]
struct IndexLoop {
    header: BlockId,
    preheader: BlockId,
    members: BTreeSet<BlockId>,
    tails: BTreeSet<BlockId>,
}

fn natural_loops(routine: &NirRoutine) -> Vec<IndexLoop> {
    let cfg = NirCfg::from_routine(routine);
    let dominance = NirDominance::from_cfg(&cfg);
    let mut loops = BTreeMap::<BlockId, (BTreeSet<BlockId>, BTreeSet<BlockId>)>::new();
    for &tail in cfg.reachable() {
        for &header in cfg.successors(tail) {
            if !dominance.is_backedge(tail, header) {
                continue;
            }
            let (members, tails) = loops.entry(header).or_default();
            members.insert(header);
            tails.insert(tail);
            let mut pending = vec![tail];
            while let Some(block) = pending.pop() {
                if members.insert(block) {
                    pending.extend(cfg.predecessors(block).iter().copied());
                }
            }
        }
    }
    let mut result = Vec::new();
    for (header, (members, tails)) in loops {
        if members.iter().any(|b| !dominance.dominates(header, *b)) {
            continue;
        }
        let outside: Vec<_> = cfg
            .predecessors(header)
            .difference(&members)
            .copied()
            .collect();
        let [preheader] = outside.as_slice() else {
            continue;
        };
        // Reuse an existing dedicated entry edge. Do not change CFG topology
        // or speculate arithmetic into an unrelated successor path.
        if !routine.blocks.iter().any(|b| {
            b.id == *preheader
                && matches!(&b.terminator, NirTerminator::Goto(edge) if edge.target == header)
        }) {
            continue;
        }
        result.push(IndexLoop {
            header,
            preheader: *preheader,
            members,
            tails,
        });
    }
    // Inner loops first: an invariant can subsequently move to its outermost
    // legal preheader. Every membership/dominance fact still describes this CFG.
    result.sort_by_key(|l| (l.members.len(), l.header));
    result
}

pub(in crate::nir::optimizer) fn hoist_invariant_arithmetic(routine: &mut NirRoutine) {
    for region in natural_loops(routine) {
        debug_assert!(!region.tails.is_empty());
        // This first motion pass does not cross call, machine or volatile
        // barriers. Ordinary memory operations stay exactly where they were.
        if routine
            .blocks
            .iter()
            .filter(|b| region.members.contains(&b.id))
            .flat_map(|b| &b.ops)
            .any(ordering_barrier)
        {
            continue;
        }
        let indexes = index_temps(routine);
        let cfg = NirCfg::from_routine(routine);
        let dominance = NirDominance::from_cfg(&cfg);
        let definitions = NirUseDef::from_routine(routine);
        let mut moved = BTreeSet::new();
        let mut hoisted = Vec::new();
        loop {
            let before = moved.len();
            for block in &mut routine.blocks {
                if !region.members.contains(&block.id) {
                    continue;
                }
                block.ops.retain(|op| {
                    let Some((dest, _)) = op_def(op) else {
                        return true;
                    };
                    let Some(inputs) = arithmetic_inputs(op) else {
                        return true;
                    };
                    if !indexes.contains(&dest) || moved.len() >= 64 {
                        return true;
                    }
                    let invariant = inputs.iter().all(|value| match value {
                        NirValue::IntegerConst { .. } => true,
                        NirValue::Temp { id, .. } => {
                            moved.contains(id)
                                || definitions.unique_definition(*id).is_some_and(|def| {
                                    !region.members.contains(&def.block)
                                        && dominance.dominates(def.block, region.preheader)
                                })
                        }
                        _ => false,
                    });
                    if !invariant {
                        return true;
                    }
                    moved.insert(dest);
                    hoisted.push(op.clone());
                    false
                });
            }
            if before == moved.len() {
                break;
            }
        }
        routine
            .blocks
            .iter_mut()
            .find(|b| b.id == region.preheader)
            .unwrap()
            .ops
            .extend(hoisted);
    }
    routine.temps = collect_temps(&routine.blocks);
}
#[cfg(test)]
mod tests {
    use super::*;

    fn promoted(source: &str) -> NirProgram {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let model = crate::semantic::analyze_with_options(
            &ast,
            crate::semantic::SemanticOptions::modern()
                .with_target(crate::target::TargetId::Motorola68000),
        )
        .unwrap();
        let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let p = crate::nir::storage_optimizer::propagate_program(&raw).unwrap();
        crate::nir::promotion::promote_program(&p, crate::nir::NirPromotionPolicy::NativeLoops)
            .unwrap()
    }

    fn source(body: &str) -> String {
        format!(
            "CARD ARRAY a(4,10)\nCARD limit\nVOLATILE BYTE port\nPROC Side() RETURN\nPROC Main() CARD r,c FOR r=0 TO 3 DO FOR c=1 TO limit DO {body} OD OD RETURN"
        )
    }

    #[test]
    fn index_hoisting_moves_row_products_but_keeps_descriptor_loads_in_loop() {
        let mut p = promoted(&source("a(r,c)=r+c"));
        let r = p.routines.last_mut().unwrap();
        let inner = natural_loops(r)
            .into_iter()
            .min_by_key(|l| l.members.len())
            .unwrap();
        let in_loop = |r: &NirRoutine| {
            r.blocks
                .iter()
                .filter(|b| inner.members.contains(&b.id))
                .flat_map(|b| &b.ops)
                .cloned()
                .collect::<Vec<_>>()
        };
        let before = in_loop(r);
        assert!(before.iter().any(|op| matches!(
            op,
            NirOp::Binary {
                op: NirBinaryOp::Mul,
                ..
            }
        )));
        hoist_invariant_arithmetic(r);
        let after = in_loop(r);
        assert!(!after.iter().any(|op| matches!(
            op,
            NirOp::Binary {
                op: NirBinaryOp::Mul,
                ..
            }
        )));
        let memory = |ops: Vec<NirOp>| {
            ops.into_iter()
                .filter(|op| matches!(op, NirOp::Load { .. } | NirOp::Store { .. }))
                .collect::<Vec<_>>()
        };
        assert_eq!(memory(before), memory(after));
        verify_program(&p).unwrap();
        let once = p.clone();
        hoist_invariant_arithmetic(p.routines.last_mut().unwrap());
        assert_eq!(p, once);
    }

    #[test]
    fn index_hoisting_rejects_effect_barriers_and_faulting_coordinate_inputs() {
        for body in ["Side() a(r,c)=0", "port=1 a(r,c)=0", "a(r/limit,c)=0"] {
            let mut p = promoted(&source(body));
            let before = p.clone();
            hoist_invariant_arithmetic(p.routines.last_mut().unwrap());
            verify_program(&p).unwrap();
            // No descriptor/port/division load or operation is speculated on
            // the zero-trip inner-loop path. Independent total casts may move.
            for (old, new) in before
                .routines
                .last()
                .unwrap()
                .blocks
                .iter()
                .zip(&p.routines.last().unwrap().blocks)
            {
                let sensitive = |ops: &[NirOp]| {
                    ops.iter()
                        .filter(|op| arithmetic_inputs(op).is_none())
                        .cloned()
                        .collect::<Vec<_>>()
                };
                assert_eq!(sensitive(&old.ops), sensitive(&new.ops));
            }
            if body.contains("Side") || body.contains("port") {
                assert_eq!(p, before);
            }
        }
    }
}
