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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Call {
    pub target: Mir65816CallTarget,
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

pub(super) fn plan(r: &Mir65816Routine, logical: &RoutineAnalysis<'_>) -> Result<Plan, String> {
    let mut calls = Plan::new();
    for block in &r.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let Mir65816Op::Call {
                target,
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
                    abi: plan.clone(),
                    result: declaration,
                    route,
                },
            );
        }
    }
    Ok(calls)
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
}
