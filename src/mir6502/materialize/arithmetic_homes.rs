//! Reconstruct arithmetic call boundaries while operands still refer to source
//! objects. Only the retained routine, with its proposed bindings, is emitted.
use super::*;
use crate::mir6502::ir::MirRoutine;

pub(in crate::mir6502) struct ArithmeticHomes {
    pub routine: MirRoutine,
    pub setup_writes: BTreeSet<u8>,
}

impl ArithmeticHomes {
    /// Rewrites can fuse, specialize or remove a call. Require the same helpers
    /// in the same order in each block before relying on their return contracts.
    fn agrees_with(&self, selected: &MirRoutine) -> bool {
        fn sites(
            routine: &MirRoutine,
        ) -> impl Iterator<Item = (MirBlockId, MirRuntimeHelper)> + '_ {
            routine.blocks.iter().flat_map(|block| {
                block.ops.iter().filter_map(move |op| {
                    if let MirOp::RuntimeHelper { helper, .. } = op {
                        Some((block.id, *helper))
                    } else {
                        None
                    }
                })
            })
        }
        sites(&self.routine).eq(sites(selected))
    }
}

/// Use emission's power-of-two reductions before testing their primitive ops.
/// Leave pointer-cell reads and carry-dependent inputs to the original path.
pub(in crate::mir6502) fn reduced_constants(
    retained: &MirRoutine,
    layout: &MaterializeLayout,
) -> Option<MirRoutine> {
    let mut candidate = false;
    for op in retained.blocks.iter().flat_map(|b| &b.ops) {
        if let MirOp::Binary {
            op: MirBinaryOp::Mul | MirBinaryOp::UDiv | MirBinaryOp::UMod,
            left,
            right,
            carry_in,
            carry_out,
            ..
        } = op
        {
            if carry_in.is_some() || *carry_out != MirCarryOut::Ignore {
                return None;
            }
            candidate |= constant_value_u16(right).is_some() || constant_value_u16(left).is_some();
        }
    }
    if !candidate {
        return None;
    }
    let mut routine = retained.clone();
    let mut stats = MirPeepholeStats::default();
    strength_reduce_constant_multiplications(&mut routine, layout, &BTreeSet::new(), &mut stats);
    strength_reduce_constant_unsigned_divmod(&mut routine, &mut stats);
    Some(routine)
}

pub(in crate::mir6502) fn plan(
    retained: &MirRoutine,
    selected: &MirRoutine,
    program: &MirProgram,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Option<ArithmeticHomes> {
    if !config.select_runtime_helpers
        || !retained.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            matches!(
                op,
                MirOp::Binary {
                    op: MirBinaryOp::Mul
                        | MirBinaryOp::Div
                        | MirBinaryOp::UDiv
                        | MirBinaryOp::Mod
                        | MirBinaryOp::UMod
                        | MirBinaryOp::Lsh
                        | MirBinaryOp::Rsh,
                    ..
                }
            )
        })
    {
        return None;
    }
    let mut linked = Vec::new();
    for op in selected.blocks.iter().flat_map(|b| &b.ops) {
        if let MirOp::RuntimeHelper { helper, .. } = op {
            crate::mir6502::runtime_contracts::helper_invocation(program, op)?;
            linked.push(*helper);
        }
    }
    let mut routine = match reduced_constants(retained, layout) {
        Some(reduced) => reduced,
        None if linked.is_empty() => return None,
        None => retained.clone(),
    };
    runtime::expose_zero_extended_binary_inputs(&mut routine);
    runtime::fuse_adjacent_divmod(&mut routine, config, layout, &mut linked);
    let widths = collect_routine_temp_widths(&routine);
    for block in &mut routine.blocks {
        let mut ops = Vec::with_capacity(block.ops.len());
        for op in std::mem::take(&mut block.ops) {
            if matches!(&op, MirOp::Binary { dst: MirDef::VTemp(_), left, right,
                carry_in: None, carry_out: MirCarryOut::Ignore, .. }
                if !value_contains_pointer_cell(left) && !value_contains_pointer_cell(right))
                && native_word_shift(&op).is_none()
                && runtime::expand_binary_helper(&op, config, layout, &widths, &linked, &mut ops)
                    .is_some()
            {
                continue;
            }
            ops.push(op);
        }
        block.ops = ops;
    }
    // A producer can fold into argument setup after a fixed input has already
    // been written. Reserve these lanes even if the object dies at the call.
    let mut setup_writes = BTreeSet::new();
    for op in routine.blocks.iter().flat_map(|b| &b.ops) {
        if let MirOp::RuntimeHelper { args, .. } = op {
            setup_writes.extend(args.iter().filter_map(|home| match home {
                MirArgHome::FixedZeroPage(slot) => Some(slot.0),
                _ => None,
            }));
        }
    }
    let plan = ArithmeticHomes {
        routine,
        setup_writes,
    };
    plan.agrees_with(selected).then_some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(source: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }

    #[test]
    fn planning_uses_the_emitted_word_shift_helper_and_staging() {
        let config = Mir6502Config::optimized();
        for (direction, helper) in [
            ("RSH", MirRuntimeHelper::Rsh),
            ("LSH", MirRuntimeHelper::Lsh),
        ] {
            for count in [4, 5, 6, 7, 8] {
                let p = program(&format!(
                    "BYTE b CARD w PROC Probe(CARD x) b=x {direction} {count} w=x {direction} {count} RETURN"
                ));
                let r = &p.routines[0];
                let layout = MaterializeLayout::new(&p, 0x3000);
                let selected = crate::mir6502::materialize_program(p.clone(), &config).unwrap();
                let planned = plan(r, &selected.routines[0], &selected, &config, &layout);
                if matches!(count, 4 | 8) {
                    assert!(planned.is_none());
                    continue;
                }
                let planned = planned.unwrap();
                assert_eq!(planned.setup_writes, [0x84].into());
                let view = planned.routine;
                let calls = |r: &MirRoutine| {
                    r.blocks.iter().flat_map(|b| &b.ops)
                    .filter(|op| matches!(op, MirOp::RuntimeHelper { helper: h, .. } if *h == helper)).cloned().collect::<Vec<_>>()
                };
                assert_eq!(calls(&view).len(), 2);
                assert_eq!(calls(&selected.routines[0]), calls(&view));
                for block in &view.blocks {
                    for (i, _) in block
                        .ops
                        .iter()
                        .enumerate()
                        .filter(|(_, op)| matches!(op, MirOp::RuntimeHelper { .. }))
                    {
                        assert!(matches!(&block.ops[i-3..i], [
                            MirOp::Store { dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x84))), src: MirValue::ConstU8(n), .. },
                            MirOp::Move { dst: MirDef::Reg(MirReg::A), .. },
                            MirOp::Move { dst: MirDef::Reg(MirReg::X), .. },
                        ] if *n == count));
                        let capture = |op: &MirOp, reg| {
                            matches!(op,
                            MirOp::Move { src: MirValue::Def(MirDef::Reg(r)), .. }
                            | MirOp::Store { src: MirValue::Def(MirDef::Reg(r)), .. } if *r == reg)
                        };
                        assert!(capture(&block.ops[i + 1], MirReg::A));
                        assert!(capture(&block.ops[i + 2], MirReg::X));
                    }
                }
                assert!(
                    plan(
                        r,
                        &selected.routines[0],
                        &selected,
                        &Mir6502Config {
                            select_runtime_helpers: false,
                            ..config.clone()
                        },
                        &layout
                    )
                    .is_none()
                );
            }
        }
        // An explicitly widened byte can also use the selected word helper.
        let p = program("CARD w PROC Probe(BYTE x) w=CARD(x) LSH 5 RETURN");
        let selected = crate::mir6502::materialize_program(p.clone(), &config).unwrap();
        assert!(
            plan(
                &p.routines[0],
                &selected.routines[0],
                &selected,
                &config,
                &MaterializeLayout::new(&p, 0x3000)
            )
            .is_some()
        );
    }
}
