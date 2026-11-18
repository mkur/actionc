//! Expand unambiguous retained word shifts for source-home planning. Use the same
//! binary selector and ABI staging as emission. The returned operations belong
//! only to the planning routine; the placement trial replays the retained input.
use super::*;
use crate::mir6502::analysis::use_def::MirTempUseDefIndex;
use crate::mir6502::ir::MirRoutine;

pub(in crate::mir6502) struct PlannedShifts {
    pub routine: MirRoutine,
    pub setup_writes: std::collections::BTreeSet<u8>,
}

impl PlannedShifts {
    /// A declaration may have been selected for a different caller. Check the
    /// actual routine as well; rewrites can remove or introduce helper sites.
    pub fn agrees_with(&self, selected: &MirRoutine) -> bool {
        fn shift_calls(r: &MirRoutine) -> [usize; 2] {
            let mut counts = [0; 2];
            for op in r.blocks.iter().flat_map(|b| &b.ops) {
                match op {
                    MirOp::RuntimeHelper {
                        helper: MirRuntimeHelper::Lsh,
                        ..
                    } => counts[0] += 1,
                    MirOp::RuntimeHelper {
                        helper: MirRuntimeHelper::Rsh,
                        ..
                    } => counts[1] += 1,
                    _ => {}
                }
            }
            counts
        }
        shift_calls(&self.routine) == shift_calls(selected)
    }
}

pub(in crate::mir6502) fn planned_word_shifts(
    routine: &MirRoutine,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Option<PlannedShifts> {
    if !config.select_runtime_helpers {
        return None;
    }
    let mut result = None;
    let facts = std::cell::OnceCell::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        // Apply replacements backwards so their source positions stay valid.
        for (index, shift) in block.ops.iter().enumerate().rev() {
            let MirOp::Binary {
                op: MirBinaryOp::Lsh | MirBinaryOp::Rsh,
                width: MirWidth::Word,
                dst: dst @ MirDef::VTemp(id),
                left: MirValue::Def(MirDef::VTemp(source)),
                right,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            } = shift
            else {
                continue;
            };
            if constant_value_u16(right).is_none() || native_word_shift(shift).is_some() {
                continue;
            }

            // An adjacent full-word load excludes widened byte producers and
            // supplies the operand before the shift. Limit this path to stores;
            // expression trees folded into call arguments have their own staging.
            if !matches!(index.checked_sub(1).and_then(|i| block.ops.get(i)),
                Some(MirOp::Load { dst: MirDef::VTemp(t), src: MirAddr::Direct(_), width: MirWidth::Word })
                if t == source)
            {
                continue;
            }
            let tail = &block.ops[index + 1..];
            let word_store = match tail {
                [
                    MirOp::Store {
                        dst: MirAddr::Direct(_),
                        src: MirValue::Def(d),
                        width: MirWidth::Word,
                    },
                    ..,
                ] if d == dst => true,
                [
                    MirOp::Truncate {
                        dst: byte,
                        src: MirValue::Def(d),
                        from_width: MirWidth::Word,
                        to_width: MirWidth::Byte,
                    },
                    MirOp::Store {
                        src: MirValue::Def(value),
                        width: MirWidth::Byte,
                        ..
                    },
                    ..,
                ] if d == dst && byte == value => false,
                _ => continue,
            };
            let (widths, uses) = facts.get_or_init(|| {
                (
                    collect_routine_temp_widths(routine),
                    MirTempUseDefIndex::from_routine(routine),
                )
            });
            // The fused word store consumes the result directly in A/X.
            if word_store
                && uses.uses_of_temp(*id).iter().any(|u| {
                    u.site
                        != (crate::mir6502::analysis::sites::MirSite::Op {
                            block: block.id,
                            op_index: index + 1,
                        })
                })
            {
                continue;
            }
            let mut ops = Vec::new();
            let consumed = if word_store {
                let consumed = select_word_helper_store_consumer(
                    &block.ops,
                    index,
                    config,
                    layout,
                    widths,
                    true,
                    &mut Vec::new(),
                    &mut ops,
                );
                if consumed == 0 {
                    continue;
                }
                consumed
            } else {
                runtime::expand_binary_helper(shift, config, layout, widths, &[], &mut ops)
                    .expect("constant word shift has a helper");
                1
            };
            let plan = result.get_or_insert_with(|| PlannedShifts {
                routine: routine.clone(),
                setup_writes: Default::default(),
            });
            // Loads of the source can fold into A/X setup after the count store.
            // Reserve its destination even when the source dies at the call.
            for op in ops
                .iter()
                .take_while(|op| !matches!(op, MirOp::RuntimeHelper { .. }))
            {
                for write in crate::mir6502::analysis::effects::classify_op(op)
                    .memory
                    .direct_writes
                {
                    let MirMem::FixedZeroPage(slot) = write.base else {
                        return None;
                    };
                    for offset in 0..write.bytes {
                        plan.setup_writes
                            .insert(slot.0.checked_add(u8::try_from(offset).ok()?)?);
                    }
                }
            }
            plan.routine.blocks[block_index]
                .ops
                .splice(index..index + consumed, ops);
        }
    }
    result
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
                let planned = planned_word_shifts(r, &config, &layout);
                if matches!(count, 4 | 8) {
                    assert!(planned.is_none());
                    continue;
                }
                let plan = planned.unwrap();
                assert_eq!(plan.setup_writes, [0x84].into());
                let view = plan.routine;
                let selected = crate::mir6502::materialize_program(p.clone(), &config).unwrap();
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
                    planned_word_shifts(
                        r,
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
        let widened = program("CARD w PROC Probe(BYTE x) w=CARD(x) LSH 5 RETURN");
        assert!(
            planned_word_shifts(
                &widened.routines[0],
                &config,
                &MaterializeLayout::new(&widened, 0x3000)
            )
            .is_none()
        );
    }
}
