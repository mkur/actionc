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

/// Recognize a header-tested, increasing SSA counter with one backedge. The
/// bound proves that its *source-width* update cannot wrap, including the last
/// update. This proof is required before widening an incremental offset.
fn induction_step(
    routine: &NirRoutine,
    region: &IndexLoop,
    param_index: usize,
) -> Option<(NirValue, u64)> {
    let header = routine.blocks.iter().find(|b| b.id == region.header)?;
    let param = header.params.get(param_index)?;
    let integer = param.ty.kind.integer()?;
    let maximum = (1u64 << (integer.bits - u8::from(integer.signed))) - 1;
    let [tail] = region.tails.iter().copied().collect::<Vec<_>>()[..] else {
        return None;
    };
    let pre = routine.blocks.iter().find(|b| b.id == region.preheader)?;
    let NirTerminator::Goto(entry) = &pre.terminator else {
        return None;
    };
    let latch = routine.blocks.iter().find(|b| b.id == tail)?;
    let NirTerminator::Goto(back) = &latch.terminator else {
        return None;
    };
    if back.target != region.header {
        return None;
    }
    let NirValue::Temp { id: updated, .. } = back.args.get(param_index)? else {
        return None;
    };
    let update = routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find(|op| op_def(op).is_some_and(|(id, _)| id == *updated))?;
    let NirOp::Binary {
        ty,
        op: NirBinaryOp::Add,
        left: NirValue::Temp { id, .. },
        right,
        ..
    } = update
    else {
        return None;
    };
    if *id != param.dest || *ty != param.ty {
        return None;
    }
    let step = const_integer_bits(right)?;
    if step == 0 || step > maximum {
        return None;
    }
    let NirTerminator::Branch {
        condition: NirValue::Temp { id: condition, .. },
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    if !region.members.contains(&then_edge.target) || region.members.contains(&else_edge.target) {
        return None;
    }
    let compare = header
        .ops
        .iter()
        .find(|op| op_def(op).is_some_and(|(id, _)| id == *condition))?;
    let NirOp::Compare {
        op,
        operand_ty,
        left: NirValue::Temp { id, .. },
        right,
        ..
    } = compare
    else {
        return None;
    };
    if *id != param.dest || *operand_ty != param.ty {
        return None;
    }
    let bound = const_integer_bits(right)?;
    let last = match op {
        NirCompareOp::Le => bound,
        NirCompareOp::Lt => bound.checked_sub(1)?,
        _ => return None,
    };
    if last > maximum.checked_sub(step)? {
        return None;
    }
    Some((entry.args.get(param_index)?.clone(), step))
}

pub(in crate::nir::optimizer) fn reduce_induction_arithmetic(routine: &mut NirRoutine) {
    let mut next = routine.temps.iter().map(|t| t.id.0).max().unwrap_or(0) + 1;
    for region in natural_loops(routine) {
        let indexes = index_temps(routine);
        let cfg = NirCfg::from_routine(routine);
        let dominance = NirDominance::from_cfg(&cfg);
        let header_params = routine
            .blocks
            .iter()
            .find(|b| b.id == region.header)
            .unwrap()
            .params
            .clone();
        let mut count = 0;
        for (param_index, counter) in header_params.iter().enumerate() {
            let Some((initial, step)) = induction_step(routine, &region, param_index) else {
                continue;
            };
            let tail = *region.tails.first().unwrap();
            let products: Vec<_> = routine.blocks.iter()
                .filter(|b| region.members.contains(&b.id) && b.id != region.header && dominance.dominates(b.id, tail))
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(op, NirOp::Binary { dest, op: NirBinaryOp::Mul, .. } if indexes.contains(dest)))
                .cloned().collect();
            for product in products {
                if count >= 4 {
                    break;
                } // Bound additional loop-carried state.
                let NirOp::Binary {
                    dest,
                    ty,
                    left,
                    right,
                    ..
                } = product
                else {
                    unreachable!()
                };
                let (variable, factor) = if let Some(c) = const_integer_bits(&right) {
                    (left, c)
                } else if let Some(c) = const_integer_bits(&left) {
                    (right, c)
                } else {
                    continue;
                };
                let Some(result_integer) = ty.kind.integer() else {
                    continue;
                };
                // Preserve the cheap shift case and avoid carrying another
                // scalar for ordinary source multiplication. This pass targets
                // generated address-stride products whose recomputation is
                // expensive enough to justify loop-carried state.
                if result_integer.role != crate::nir::NirIntegerRole::Address
                    || !matches!(result_integer.bits, 16 | 24 | 32)
                    || factor <= 1
                    || factor.is_power_of_two()
                {
                    continue;
                }
                let NirValue::Temp { id, .. } = &variable else {
                    continue;
                };
                let cast = if *id == counter.dest {
                    if counter.ty.width != ty.width {
                        continue;
                    }
                    None
                } else {
                    let Some(NirOp::Cast {
                        src: NirValue::Temp { id: src, .. },
                        from,
                        to,
                        kind,
                        ..
                    }) = routine
                        .blocks
                        .iter()
                        .flat_map(|b| &b.ops)
                        .find(|op| op_def(op).is_some_and(|(dest, _)| dest == *id))
                    else {
                        continue;
                    };
                    if *src != counter.dest
                        || *from != counter.ty
                        || *to != ty
                        || to.width < from.width
                    {
                        continue;
                    }
                    Some((from.clone(), to.clone(), *kind))
                };
                // Constants and the proven counter are the only inputs. Calls
                // cannot modify SSA values; no descriptor, load or source
                // expression is moved, cached or evaluated an extra time.
                let mut entry_ops = Vec::new();
                let start = if let Some((from, to, kind)) = cast {
                    let dest = TempId(next);
                    next += 1;
                    entry_ops.push(NirOp::Cast {
                        dest,
                        src: initial.clone(),
                        from,
                        to: to.clone(),
                        kind,
                    });
                    NirValue::Temp { id: dest, ty: to }
                } else {
                    initial.clone()
                };
                let initial_product = TempId(next);
                next += 1;
                entry_ops.push(NirOp::Binary {
                    dest: initial_product,
                    ty: ty.clone(),
                    op: NirBinaryOp::Mul,
                    left: start,
                    right: value_for_type(factor, &ty).unwrap(),
                });
                let accumulator = TempId(next);
                next += 1;
                let advanced = TempId(next);
                next += 1;
                let accumulated = NirValue::Temp {
                    id: accumulator,
                    ty: ty.clone(),
                };
                let mask = (1u64 << result_integer.bits) - 1;
                let advance = NirOp::Binary {
                    dest: advanced,
                    ty: ty.clone(),
                    op: NirBinaryOp::Add,
                    left: accumulated.clone(),
                    right: value_for_type(step.wrapping_mul(factor) & mask, &ty).unwrap(),
                };
                let replacement = BTreeMap::from([(dest, accumulated)]);
                for block in &mut routine.blocks {
                    block
                        .ops
                        .retain(|op| !op_def(op).is_some_and(|(id, _)| id == dest));
                    for op in &mut block.ops {
                        rewrite_op_values(op, &replacement);
                    }
                    rewrite_terminator_values(&mut block.terminator, &replacement);
                    if block.id == region.header {
                        block.params.push(NirBlockParam {
                            dest: accumulator,
                            ty: ty.clone(),
                        });
                    }
                    if block.id == region.preheader {
                        block.ops.extend(entry_ops.clone());
                        let NirTerminator::Goto(edge) = &mut block.terminator else {
                            unreachable!()
                        };
                        edge.args.push(NirValue::Temp {
                            id: initial_product,
                            ty: ty.clone(),
                        });
                    }
                    if block.id == tail {
                        block.ops.push(advance.clone());
                        let NirTerminator::Goto(edge) = &mut block.terminator else {
                            unreachable!()
                        };
                        edge.args.push(NirValue::Temp {
                            id: advanced,
                            ty: ty.clone(),
                        });
                    }
                }
                count += 1;
            }
        }
    }
    routine.temps = collect_temps(&routine.blocks);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn promoted(source: &str) -> NirProgram {
        promoted_for_target(source, crate::target::TargetId::Motorola68000)
    }

    fn promoted_for_target(source: &str, target: crate::target::TargetId) -> NirProgram {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let model = crate::semantic::analyze_with_options(
            &ast,
            crate::semantic::SemanticOptions::modern().with_target(target),
        )
        .unwrap();
        let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let p = crate::nir::storage_optimizer::propagate_program(&raw).unwrap();
        let mut p =
            crate::nir::promotion::promote_program(&p, crate::nir::NirPromotionPolicy::NativeLoops)
                .unwrap();
        for routine in &mut p.routines {
            optimize_values_in_routine(routine);
            routine.temps = collect_temps(&routine.blocks);
        }
        verify_program(&p).unwrap();
        p
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

    #[test]
    fn index_induction_uses_a_typed_loop_parameter_and_constant_advance() {
        for (kind, last) in [
            ("BYTE", 254),
            ("CARD", 254),
            ("INT", 254),
            ("LONGCARD", 254),
        ] {
            let mut p = promoted(&format!(
                "CARD ARRAY a(256,10)\nPROC Main() {kind} row FOR row=0 TO {last} DO a(row,0)=CARD(row) OD RETURN"
            ));
            let r = p.routines.last_mut().unwrap();
            let before = r.blocks.iter().map(|b| b.params.len()).sum::<usize>();
            reduce_induction_arithmetic(r);
            assert!(
                r.blocks.iter().map(|b| b.params.len()).sum::<usize>() > before,
                "{kind}\n{}",
                crate::nir::format_program(&p)
            );
            verify_program(&p).unwrap();
            let once = p.clone();
            reduce_induction_arithmetic(p.routines.last_mut().unwrap());
            assert_eq!(p, once);
        }
    }

    #[test]
    fn index_induction_rejects_source_counter_wrap_and_non_affine_casts() {
        for (kind, bound, coordinate) in [
            ("BYTE", 255, "row"),
            ("CARD", 65535, "row"),
            ("INT", 32767, "row"),
            ("CARD", 254, "BYTE(row+250)"),
        ] {
            let mut p = promoted(&format!(
                "CARD ARRAY a(256,10)\nPROC Main() {kind} row FOR row=0 TO {bound} DO a({coordinate},0)=0 IF row=2 THEN EXIT FI OD RETURN"
            ));
            let before = p.clone();
            reduce_induction_arithmetic(p.routines.last_mut().unwrap());
            assert_eq!(p, before, "{kind}/{coordinate}");
            verify_program(&p).unwrap();
        }
    }

    #[test]
    fn index_induction_keeps_target_address_widths_verified() {
        use crate::target::TargetId;
        for target in [
            TargetId::Atari6502,
            TargetId::Wdc65816Native,
            TargetId::Wdc65816Small,
            TargetId::Motorola68000,
        ] {
            let mut p = promoted_for_target(
                "CARD ARRAY a(256,10),flat(256) CARD result PROC Main() CARD row FOR row=0 TO 254 DO result==+flat(row)+a(row,0)+a(row,1) OD RETURN",
                target,
            );
            let before = p.routines[0]
                .blocks
                .iter()
                .map(|b| b.params.len())
                .sum::<usize>();
            reduce_induction_arithmetic(&mut p.routines[0]);
            verify_program(&p).unwrap();
            assert!(
                p.routines[0]
                    .blocks
                    .iter()
                    .map(|b| b.params.len())
                    .sum::<usize>()
                    > before,
                "{target:?}\n{}",
                crate::nir::format_program(&p)
            );
        }
    }
}
