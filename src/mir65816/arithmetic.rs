//! Native arithmetic legalization. Helpers are typed routines, never name-selected.
use super::*;
use crate::nir::{
    NirCallableKind, NirCallableSignature, NirIntegerRole, NirIntegerType, NirType, NirTypeKind,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Multiply,
    Divide,
    Modulo,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Helper {
    pub operation: Operation,
    pub bytes: u8,
    pub signed: bool,
}
impl Helper {
    pub fn verify(self) -> Result<(), String> {
        let valid = match self.operation {
            Operation::Multiply => matches!(self.bytes, 2 | 4) && !self.signed,
            _ => matches!((self.bytes, self.signed), (1..=4, false) | (2 | 4, true)),
        };
        if valid {
            Ok(())
        } else {
            Err("unsupported native arithmetic helper contract".into())
        }
    }
    pub fn name(self) -> String {
        let op = match self.operation {
            Operation::Multiply => "mul",
            Operation::Divide => "div",
            Operation::Modulo => "mod",
        };
        format!(
            "__a816_{op}_{}{}_v1",
            if self.signed { "i" } else { "u" },
            self.bytes * 8
        )
    }
    fn signature(self, id: SignatureId) -> NirCallableSignature {
        let ty = NirType {
            kind: NirTypeKind::Integer(NirIntegerType {
                bits: self.bytes * 8,
                signed: self.signed,
                role: if self.bytes == 3 {
                    NirIntegerRole::Size
                } else {
                    NirIntegerRole::Ordinary
                },
            }),
            summary: self.name(),
            width: Some(ByteSize::new(self.bytes.into())),
            pointer: false,
        };
        NirCallableSignature {
            id,
            params: vec![ty.clone(), ty.clone()],
            variadic: None,
            result: Some(ty),
            kind: NirCallableKind::Func,
            convention: NirCallConvention::TargetPublic,
        }
    }
    pub(super) fn plan(self, signature: SignatureId) -> Result<Mir65816CallPlan, String> {
        self.verify()?;
        lower::call_plan(
            &self.signature(signature),
            2,
            None,
            Mir65816CallConvention::Native,
            ByteSize::new(3),
            false,
        )
    }
    pub(super) fn routine(
        self,
        id: RoutineId,
        signature: SignatureId,
    ) -> Result<Mir65816Routine, String> {
        let plan = self.plan(signature)?;
        let parameters = plan
            .arguments
            .iter()
            .enumerate()
            .map(|(i, &incoming)| {
                let Mir65816AbiHome::StackArgument { offset, size, .. } = incoming else {
                    unreachable!()
                };
                Ok(Mir65816ParameterPlan {
                    param: ParamId(i as u32),
                    incoming,
                    frame_object: None,
                    body_stack_offset: Some(
                        abi::stack::incoming_displacement(ByteSize::ZERO, offset, size)
                            .map_err(|e| e.to_string())?,
                    ),
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Mir65816Routine {
            helper: Some(self),
            id,
            signature,
            entry: Default::default(),
            temps: vec![],
            name: self.name(),
            convention: NirCallConvention::TargetPublic,
            result_home: plan.result,
            frame: Mir65816FramePlan {
                strategy: Mir65816FrameStrategy::HardwareStackRelative,
                bank: 0,
                objects: vec![],
                parameters,
                automatic_bytes: ByteSize::ZERO,
                saved_state_bytes: ByteSize::ZERO,
                spill_bytes: ByteSize::ZERO,
                outgoing: Mir65816OutgoingArea::PerCallBelowFrame,
                outgoing_bytes: ByteSize::ZERO,
                incoming_bytes: plan.native.unwrap().argument_bytes,
                incoming_extent: plan.outgoing_bytes,
                extent: ByteSize::ZERO,
                minimum_stack_peak: ByteSize::ZERO,
                allocation_complete: false,
            },
            prologue: Mir65816ProloguePlan {
                required_mode: plan.mode_before,
                reserve_bytes: ByteSize::ZERO,
                parameter_copies: vec![],
            },
            epilogue: Mir65816EpiloguePlan {
                restored_mode: plan.mode_after,
                release_bytes: ByteSize::ZERO,
                return_form: Mir65816ReturnForm::FarRtl,
            },
            blocks: vec![],
        })
    }
}
fn constant(v: &Mir65816Value, known: &BTreeMap<TempId, u32>) -> Option<u32> {
    match v {
        Mir65816Value::U8(n) => Some((*n).into()),
        Mir65816Value::U16(n) => Some((*n).into()),
        Mir65816Value::U24(n) | Mir65816Value::U32(n) => Some(*n),
        Mir65816Value::Null(_) => Some(0),
        Mir65816Value::Temp(id, _) => known.get(id).copied(),
        _ => None,
    }
}
fn mask(bytes: ByteSize) -> u32 {
    u32::MAX >> (32 - bytes.get() * 8)
}
fn literal(n: u32, bytes: ByteSize) -> Mir65816Value {
    match bytes.get() {
        1 => Mir65816Value::U8(n as u8),
        2 => Mir65816Value::U16(n as u16),
        3 => Mir65816Value::U24(n & 0xffffff),
        _ => Mir65816Value::U32(n),
    }
}
/// Idempotent preparation, shared by machine emission and all artifact writers.
/// Generated entries carry no source span and use the same routine/link identity space.
pub fn prepare(input: &Mir65816Program) -> Result<Mir65816Program, String> {
    verify_program(input)
        .map_err(|e| format!("invalid MIR65816 before arithmetic preparation: {e:?}"))?;
    if input.call_convention != Mir65816CallConvention::Native {
        return Ok(input.clone());
    }
    let mut program = input.clone();
    let mut helpers: BTreeMap<_, _> = program
        .routines
        .iter()
        .filter_map(|r| r.helper.map(|h| (h, (r.id, r.signature))))
        .collect();
    let mut next_routine = program.routines.iter().map(|r| r.id.0).max().unwrap_or(0);
    let mut next_signature = program
        .routines
        .iter()
        .map(|r| r.signature.0)
        .chain(
            program
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .filter_map(|op| match op {
                    Mir65816Op::Call { signature, .. } => signature.map(|s| s.0),
                    _ => None,
                }),
        )
        .max()
        .unwrap_or(0);
    let mut added = vec![];
    for routine in &mut program.routines {
        // NIR permits narrow binary operands, with zero extension; signed
        // widening is already an explicit Cast. Make the helper ABI exact.
        let mut next_temp = routine.temps.iter().map(|(id, _)| id.0).max().unwrap_or(0);
        let original = routine.clone();
        for block in &mut routine.blocks {
            let mut ops = Vec::new();
            for mut op in std::mem::take(&mut block.ops) {
                if let Mir65816Op::Binary {
                    dest,
                    width,
                    operation: operation @ (NirBinaryOp::Mul | NirBinaryOp::Div | NirBinaryOp::Mod),
                    left,
                    right,
                    ..
                } = &mut op
                {
                    if !(1..=4).contains(&width.get()) {
                        return Err("invalid arithmetic computation width".into());
                    }
                    for value in [left, right] {
                        let from = value_width(&original, value)
                            .ok_or("arithmetic operand has no typed width")?;
                        if from != *width {
                            if from.get() > width.get() && *operation != NirBinaryOp::Mul {
                                return Err(
                                    "arithmetic operand requires an explicit narrowing cast".into(),
                                );
                            }
                            if let Some(n) = constant(value, &BTreeMap::new()) {
                                *value = literal(n, *width);
                            } else {
                                next_temp = next_temp
                                    .checked_add(1)
                                    .ok_or("arithmetic temporary identity overflow")?;
                                let id = TempId(next_temp);
                                let ty = original
                                    .temps
                                    .iter()
                                    .find(|(id, _)| id == dest)
                                    .ok_or("missing computation type")?
                                    .1
                                    .clone();
                                routine.temps.push((id, ty));
                                ops.push(Mir65816Op::Cast {
                                    dest: id,
                                    from,
                                    from_signed: false,
                                    to: *width,
                                    kind: NirCastKind::Integer,
                                    value: value.clone(),
                                });
                                *value = Mir65816Value::Temp(id, *width);
                            }
                        }
                    }
                }
                ops.push(op);
            }
            block.ops = ops;
        }
        for block in &mut routine.blocks {
            let mut known = BTreeMap::new();
            for op in &mut block.ops {
                if let Mir65816Op::Cast {
                    dest,
                    from,
                    from_signed,
                    to,
                    value,
                    ..
                } = op
                {
                    if let Some(mut n) = constant(value, &known) {
                        n &= mask(*from);
                        if *from_signed && n & (1 << (from.get() * 8 - 1)) != 0 {
                            n |= !mask(*from);
                        }
                        known.insert(*dest, n & mask(*to));
                    }
                }
                let Mir65816Op::Binary {
                    dest,
                    width,
                    signed,
                    operation,
                    left,
                    right,
                } = op
                else {
                    continue;
                };
                let family = match operation {
                    NirBinaryOp::Mul => Operation::Multiply,
                    NirBinaryOp::Div => Operation::Divide,
                    NirBinaryOp::Mod => Operation::Modulo,
                    _ => continue,
                };
                if family == Operation::Multiply
                    && constant(left, &known).is_some()
                    && constant(right, &known).is_none()
                {
                    std::mem::swap(left, right);
                }
                if let Some(n) = constant(right, &known).map(|n| n & mask(*width)) {
                    let reduction = match family {
                        Operation::Multiply if n == 0 => Some((NirBinaryOp::And, 0)),
                        Operation::Multiply if n.is_power_of_two() => {
                            Some((NirBinaryOp::Lsh, n.trailing_zeros()))
                        }
                        Operation::Divide if !*signed && n.is_power_of_two() => {
                            Some((NirBinaryOp::Rsh, n.trailing_zeros()))
                        }
                        Operation::Modulo if !*signed && n.is_power_of_two() => {
                            Some((NirBinaryOp::And, n - 1))
                        }
                        _ => None,
                    };
                    if let Some((operation_new, n)) = reduction {
                        *operation = operation_new;
                        *right = literal(n, *width);
                        continue;
                    }
                }
                let helper = Helper {
                    operation: family,
                    bytes: width.get() as u8,
                    signed: family != Operation::Multiply && *signed,
                };
                helper.verify()?;
                let (id, signature) = if let Some(&identity) = helpers.get(&helper) {
                    identity
                } else {
                    next_routine = next_routine
                        .checked_add(1)
                        .ok_or("helper routine identity overflow")?;
                    next_signature = next_signature
                        .checked_add(1)
                        .ok_or("helper signature identity overflow")?;
                    let identity = (RoutineId(next_routine), SignatureId(next_signature));
                    added.push(helper.routine(identity.0, identity.1)?);
                    helpers.insert(helper, identity);
                    identity
                };
                let plan = helper.plan(signature)?;
                routine.frame.outgoing_bytes =
                    routine.frame.outgoing_bytes.max(plan.outgoing_bytes);
                routine.frame.minimum_stack_peak = routine.frame.minimum_stack_peak.max(
                    ByteSize::new(routine.frame.extent.get() + plan.outgoing_bytes.get() + 3),
                );
                *op = Mir65816Op::Call {
                    target: Mir65816CallTarget::Helper(id),
                    signature: Some(signature),
                    args: vec![left.clone(), right.clone()],
                    result: Some((*dest, *width)),
                    convention: Mir65816CallConvention::Native,
                    plan,
                };
            }
        }
    }
    program.routines.extend(added);
    verify_program(&program).map_err(|e| format!("invalid prepared arithmetic: {e:?}"))?;
    Ok(program)
}
