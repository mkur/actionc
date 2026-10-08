//! Native result declarations and routes, independent of physical allocation.
use super::*;
use crate::mir65816::analysis::{Definition, ProgramPoint, RoutineAnalysis};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Declaration {
    pub temp: TempId,
    pub bytes: u8,
    pub lanes: abi::ResultLocation,
}
impl Declaration {
    pub fn checked(
        result: Option<(TempId, ByteSize)>,
        plan: &Mir65816CallPlan,
    ) -> Result<Option<Self>, String> {
        // Even discarded outputs retain, and must validate, the callee contract.
        let lanes = effects::CallContract::result(plan.result)?;
        let Some((temp, size)) = result else {
            return Ok(None);
        };
        let bytes = allocation::width(size)?;
        let lanes = lanes.ok_or("call result requires its declared native lanes")?;
        if home_demand::result_home(bytes.into()) != Some(Mir65816AbiHome::NativeResult(lanes)) {
            return Err("call result width mismatch".into());
        }
        Ok(Some(Self { temp, bytes, lanes }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Discard,
    Capture,
    Return(ProgramPoint),
    ZeroTest(ProgramPoint),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Call {
    pub target: Mir65816CallTarget,
    pub signature: Option<SignatureId>,
    pub convention: Mir65816CallConvention,
    pub abi: Mir65816CallPlan,
    pub result: Option<Declaration>,
    pub route: Route,
}
pub(super) type Plan = BTreeMap<ProgramPoint, Call>;

impl Call {
    pub fn check_transfer(&self, form: &selected::Instruction) -> Result<bool, String> {
        let (transfer, actual, target) = match form {
            selected::Instruction::NativeCall(target, contract) => {
                (abi::FarTransfer::Jsl, contract, Some(*target))
            }
            selected::Instruction::IndirectTransfer(Some(contract)) => {
                (abi::FarTransfer::StackRtl, contract, None)
            }
            _ => return Ok(false),
        };
        let expected_target = match self.target {
            Mir65816CallTarget::Direct(id) => Some(Target::Routine(RoutineId(id))),
            Mir65816CallTarget::Helper(id) => Some(Target::Routine(id)),
            Mir65816CallTarget::Runtime(id) => Some(Target::Runtime(id)),
            Mir65816CallTarget::Indirect(..) => None,
            Mir65816CallTarget::Builtin(_) => {
                return Err("unresolved call target in placement".into());
            }
        };
        if target != expected_target
            || *actual != effects::CallContract::from_plan(&self.abi, transfer)?
        {
            return Err("selected call differs from checked native contract".into());
        }
        Ok(true)
    }
}

pub(super) fn returns(
    r: &Mir65816Routine,
    op: &Mir65816Op,
    term: &Mir65816Terminator,
    count: usize,
) -> bool {
    let (
        Mir65816Op::Call {
            target,
            result: Some((temp, bytes)),
            plan,
            ..
        },
        Mir65816Terminator::Return {
            value: Some(Mir65816Value::Temp(value, width)),
            ..
        },
    ) = (op, term)
    else {
        return false;
    };
    temp == value
        && bytes == width
        && count == 1
        && matches!(
            target,
            Mir65816CallTarget::Direct(_)
                | Mir65816CallTarget::Helper(_)
                | Mir65816CallTarget::Runtime(_)
        )
        && plan.result == r.result_home
        && plan.result == home_demand::result_home(bytes.get())
}

pub(super) fn plan(
    r: &Mir65816Routine,
    logical: &RoutineAnalysis<'_>,
    demand: &home_demand::Plan,
) -> Result<Plan, String> {
    let mut calls = Plan::new();
    for block in &r.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let Mir65816Op::Call {
                target,
                signature,
                convention,
                result,
                plan,
                ..
            } = op
            else {
                continue;
            };
            let point = ProgramPoint {
                block: block.id,
                index,
            };
            let declaration = Declaration::checked(*result, plan)?;
            let route = if let Some(d) = declaration {
                let facts = logical
                    .value(logical.temp(d.temp).map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| format!("{e:?}"))?;
                if facts.width.get() != u32::from(d.bytes)
                    || facts.definition != Definition::Operation(point)
                {
                    return Err("call result differs from logical definition".into());
                }
                if index + 1 == block.ops.len()
                    && returns(r, op, &block.terminator, facts.uses.len())
                {
                    Route::Return(ProgramPoint {
                        block: block.id,
                        index: index + 1,
                    })
                } else if demand.native_output(d.temp).is_some()
                    && block
                        .ops
                        .get(index + 1)
                        .is_some_and(|op| zero_test(d.temp, d.bytes, op))
                {
                    Route::ZeroTest(ProgramPoint {
                        block: block.id,
                        index: index + 1,
                    })
                } else {
                    Route::Capture
                }
            } else {
                Route::Discard
            };
            calls.insert(
                point,
                Call {
                    target: target.clone(),
                    signature: *signature,
                    convention: *convention,
                    abi: plan.clone(),
                    result: declaration,
                    route,
                },
            );
        }
    }
    Ok(calls)
}

pub(super) fn zero_test(temp: TempId, bytes: u8, op: &Mir65816Op) -> bool {
    let Mir65816Op::Compare {
        operation,
        left,
        right,
        width,
        ..
    } = op
    else {
        return false;
    };
    let input = |v: &Mir65816Value| matches!(v, Mir65816Value::Temp(id,w) if *id==temp && w.get()==u32::from(bytes));
    let zero = |v: &Mir65816Value| matches!(v, Mir65816Value::U8(0) | Mir65816Value::U16(0));
    matches!(bytes, 1 | 2)
        && width.get() == u32::from(bytes)
        && matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne)
        && (input(left) && zero(right) || zero(left) && input(right))
}

/// A home can disappear only after the entire replacement allocation fits.
/// Removing interference nodes can change coloring, so compare actual extents.
pub(super) fn admit_returns(
    r: &Mir65816Routine,
    demand: &mut home_demand::Plan,
    counts: &BTreeMap<TempId, usize>,
    definitions: &BTreeMap<TempId, usize>,
) {
    // Preserve already qualified Return demand when adding a new family.
    // Each family compares its full allocation against the preceding plan.
    for phase in 0..2 {
        let candidates: Vec<_> = r
            .blocks
            .iter()
            .flat_map(|b| {
                b.ops.iter().enumerate().filter_map(move |(index, op)| {
                    let Mir65816Op::Call {
                        target,
                        result: Some((temp, bytes)),
                        plan,
                        ..
                    } = op
                    else {
                        return None;
                    };
                    if definitions.get(temp) != Some(&1)
                        || counts.get(temp) != Some(&1)
                        || !matches!(
                            target,
                            Mir65816CallTarget::Direct(_)
                                | Mir65816CallTarget::Helper(_)
                                | Mir65816CallTarget::Runtime(_)
                        )
                        || Declaration::checked(Some((*temp, *bytes)), plan).is_err()
                    {
                        return None;
                    }
                    let consumer = index + 1;
                    let admissible = if phase == 0 && consumer == b.ops.len() {
                        returns(r, op, &b.terminator, 1)
                    } else if phase == 1 && consumer < b.ops.len() {
                        zero_test(*temp, bytes.get() as u8, &b.ops[consumer])
                    } else {
                        false
                    };
                    admissible.then_some((
                        *temp,
                        home_demand::Accumulator {
                            block: b.id,
                            producer: index,
                            consumer,
                            bytes: bytes.get() as u8,
                        },
                    ))
                })
            })
            .collect();
        trial_outputs(r, demand, candidates);
    }
}

fn trial_outputs(
    r: &Mir65816Routine,
    demand: &mut home_demand::Plan,
    candidates: Vec<(TempId, home_demand::Accumulator)>,
) {
    if candidates.is_empty() {
        return;
    }
    let Ok(before) = AllocatedFrame::with_demand(r, demand) else {
        return;
    };
    let mut previous = Vec::new();
    for (temp, interval) in &candidates {
        previous.push((
            *temp,
            demand
                .decisions
                .insert(*temp, home_demand::Decision::NativeOutput(*interval)),
        ));
    }
    let old_mixed = demand.mixed.clone();
    demand.mixed = mixed::Plan::new(r, demand);
    let accepted = AllocatedFrame::with_demand(r, demand).is_ok_and(|after| {
        after.extent <= before.extent
            && after.spill_bytes <= before.spill_bytes
            && after.peak_below_entry <= before.peak_below_entry
    });
    if accepted {
        for (temp, interval) in candidates {
            demand
                .native_producers
                .insert((interval.block, interval.producer), temp);
            demand
                .native_consumers
                .insert((interval.block, interval.consumer), temp);
        }
    } else {
        demand.mixed = old_mixed;
        for (temp, old) in previous {
            if let Some(old) = old {
                demand.decisions.insert(temp, old);
            } else {
                demand.decisions.remove(&temp);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declaration_checks_lanes_without_requiring_a_home() {
        for (bytes, lanes) in [
            (1, abi::ResultLocation::A8ZeroExtended),
            (2, abi::ResultLocation::A16),
            (3, abi::ResultLocation::A16X8ZeroExtended),
            (4, abi::ResultLocation::A16X16),
        ] {
            let ast = crate::parser::parse(
                &crate::lexer::tokenize(
                    "CARD FUNC Echo(CARD x) RETURN(x) PROC Main() CARD x x=Echo(1) RETURN",
                )
                .unwrap(),
            )
            .unwrap();
            let model = crate::semantic::analyze_with_options(
                &ast,
                crate::semantic::SemanticOptions::modern()
                    .with_target(crate::target::TargetId::Wdc65816Native),
            )
            .unwrap();
            let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
            let mir = crate::mir65816::lower_program(&nir).unwrap();
            let mut plan = mir
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .find_map(|op| {
                    if let Mir65816Op::Call { plan, .. } = op {
                        Some(plan.clone())
                    } else {
                        None
                    }
                })
                .unwrap();
            plan.result = Some(Mir65816AbiHome::NativeResult(lanes));
            assert_eq!(
                Declaration::checked(Some((TempId(77), ByteSize::new(bytes))), &plan)
                    .unwrap()
                    .unwrap()
                    .lanes,
                lanes
            );
            assert!(
                Declaration::checked(Some((TempId(77), ByteSize::new(5 - bytes))), &plan).is_err()
            );
            assert!(Declaration::checked(None, &plan).unwrap().is_none());
        }
    }
    #[test]
    fn native_output_permissions_require_a_fresh_origin_and_preserved_lanes() {
        for lanes in [
            abi::ResultLocation::A8ZeroExtended,
            abi::ResultLocation::A16,
            abi::ResultLocation::A16X8ZeroExtended,
            abi::ResultLocation::A16X16,
        ] {
            for failure in 0..6 {
                let frame = AllocatedFrame {
                    extent: 0,
                    spill_bytes: 0,
                    peak_below_entry: 3,
                    temps: BTreeMap::new(),
                    edge_copies: Vec::new(),
                };
                let mut e = tracked::TrackedEmitter65816::for_test(&frame);
                e.begin_source(BlockId(0), 0);
                if failure != 1 {
                    let contract = effects::CallContract {
                        outgoing: 0,
                        arguments: Vec::new(),
                        result: Some(lanes),
                    };
                    e.instruction(selected::Instruction::NativeCall(
                        Target::Routine(RoutineId(99)),
                        contract,
                    ))
                    .unwrap();
                }
                if failure == 2 {
                    e.word(selected::WordOp::LdaImm, 7);
                }
                if failure == 3 {
                    e.op(selected::Implied::Tax);
                }
                let accepted = e.publish_native(TempId(7), lanes);
                let wide = matches!(
                    lanes,
                    abi::ResultLocation::A16X8ZeroExtended | abi::ResultLocation::A16X16
                );
                assert_eq!(
                    accepted,
                    failure != 1 && failure != 2 && !(failure == 3 && wide)
                );
                e.span(BlockId(0), 0, 0);
                e.begin_source(BlockId(0), if failure == 4 { 2 } else { 1 });
                let consumed = e.consume_native(TempId(if failure == 5 { 8 } else { 7 }), lanes);
                assert_eq!(consumed, accepted && failure != 4 && failure != 5);
            }
        }
    }
}
