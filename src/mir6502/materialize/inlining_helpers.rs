//! A bounded symbolic witness for retained-helper input correspondence.
//! Byte expressions are interned in a shared DAG, so equality does not depend
//! on fresh temp IDs and repeated expressions cannot grow exponentially.
use std::collections::BTreeMap;

use crate::mir6502::analysis::leaf_routines::LeafRoutine;
use crate::mir6502::ir::*;

type Value = Vec<usize>;
#[derive(PartialEq, Eq)]
enum Node {
    Input(MirTempId, u8),
    Constant(u8),
    Compute(MirOp, Value, u8),
    Result(usize, u8),
}
#[derive(Default)]
struct Dag(Vec<Node>);
impl Dag {
    fn intern(&mut self, node: Node) -> Option<usize> {
        if let Some(i) = self.0.iter().position(|old| *old == node) {
            return Some(i);
        }
        if self.0.len() >= 2048 {
            return None;
        }
        self.0.push(node);
        Some(self.0.len() - 1)
    }
}
#[derive(Default)]
struct State {
    temps: BTreeMap<(MirTempId, u8), usize>,
    memory: BTreeMap<u8, usize>,
    carry: Option<usize>,
    events: Vec<(MirOp, Value)>,
}
fn bytes(width: MirWidth) -> u8 {
    if width == MirWidth::Byte { 1 } else { 2 }
}
impl State {
    fn value(
        &mut self,
        value: &MirValue,
        width: MirWidth,
        dag: &mut Dag,
        seed: bool,
    ) -> Option<Value> {
        match value {
            MirValue::ConstU8(n) => {
                self.value(&MirValue::ConstU16(u16::from(*n)), width, dag, seed)
            }
            MirValue::ConstU16(n) => (0..bytes(width))
                .map(|i| dag.intern(Node::Constant((n >> (8 * i)) as u8)))
                .collect(),
            MirValue::Word { lo, hi } if width == MirWidth::Word => {
                let mut result = self.value(lo, MirWidth::Byte, dag, seed)?;
                result.extend(self.value(hi, MirWidth::Byte, dag, seed)?);
                Some(result)
            }
            MirValue::Def(def) => {
                let (id, start) = match def {
                    MirDef::VTemp(id) => (*id, 0),
                    MirDef::VTempByte { id, byte } if width == MirWidth::Byte => (*id, *byte),
                    _ => return None,
                };
                (start..start + bytes(width))
                    .map(|byte| {
                        if seed && !self.temps.contains_key(&(id, byte)) {
                            self.temps
                                .insert((id, byte), dag.intern(Node::Input(id, byte))?);
                        }
                        self.temps.get(&(id, byte)).copied()
                    })
                    .collect()
            }
            _ => None,
        }
    }
    fn define(&mut self, def: &MirDef, value: Value) -> Option<()> {
        let (id, start) = match def {
            MirDef::VTemp(id) => (*id, 0),
            MirDef::VTempByte { id, byte } if value.len() == 1 => (*id, *byte),
            _ => return None,
        };
        for (i, node) in value.into_iter().enumerate() {
            self.temps.insert((id, start + i as u8), node);
        }
        Some(())
    }
    fn run(
        &mut self,
        ops: &[MirOp],
        params: &BTreeMap<(crate::nir::ParamId, u16), Value>,
        dag: &mut Dag,
    ) -> Option<()> {
        for op in ops {
            let zero = MirValue::ConstU8(0);
            let dest = MirDef::VTemp(MirTempId(0));
            let mut inputs = Vec::new();
            let mut shape = op.clone();
            let (dst, width, produces_carry) = match op {
                MirOp::Load { dst, src, width } => {
                    let value = match src {
                        MirAddr::Direct(MirMem::Param { id, offset }) => {
                            params.get(&(*id, *offset))?.clone()
                        }
                        MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))) => {
                            (0..bytes(*width))
                                .map(|i| self.memory.get(&slot.checked_add(i)?).copied())
                                .collect::<Option<Value>>()?
                        }
                        _ => return None,
                    };
                    self.define(dst, value)?;
                    continue;
                }
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
                    src,
                    width,
                } => {
                    for (i, node) in self.value(src, *width, dag, false)?.into_iter().enumerate() {
                        self.memory.insert(slot.checked_add(i as u8)?, node);
                    }
                    continue;
                }
                MirOp::Move { dst, src, width } => {
                    let value = self.value(src, *width, dag, false)?;
                    self.define(dst, value)?;
                    continue;
                }
                MirOp::LoadImm { dst, value, width } => {
                    let value = self.value(&MirValue::ConstU16(*value), *width, dag, false)?;
                    self.define(dst, value)?;
                    continue;
                }
                MirOp::Unary {
                    dst, src, width, ..
                } => {
                    inputs.extend(self.value(src, *width, dag, false)?);
                    if let MirOp::Unary { dst, src, .. } = &mut shape {
                        *dst = dest;
                        *src = zero;
                    }
                    (dst.clone(), *width, false)
                }
                MirOp::Extend {
                    dst,
                    src,
                    from_width,
                    to_width,
                    ..
                }
                | MirOp::Truncate {
                    dst,
                    src,
                    from_width,
                    to_width,
                } => {
                    inputs.extend(self.value(src, *from_width, dag, false)?);
                    match &mut shape {
                        MirOp::Extend { dst, src, .. } | MirOp::Truncate { dst, src, .. } => {
                            *dst = dest;
                            *src = zero;
                        }
                        _ => unreachable!(),
                    }
                    (dst.clone(), *to_width, false)
                }
                MirOp::Binary {
                    dst,
                    left,
                    right,
                    width,
                    carry_in,
                    carry_out,
                    ..
                } => {
                    inputs.extend(self.value(left, *width, dag, false)?);
                    inputs.extend(self.value(right, *width, dag, false)?);
                    if *carry_in == Some(MirCarryIn::FromPrevious) {
                        inputs.push(self.carry?);
                    }
                    if let MirOp::Binary {
                        dst, left, right, ..
                    } = &mut shape
                    {
                        *dst = dest;
                        *left = zero.clone();
                        *right = zero;
                    }
                    (dst.clone(), *width, *carry_out == MirCarryOut::Produce)
                }
                MirOp::Compare {
                    dst: MirCondDest::Temp(id),
                    left,
                    right,
                    width,
                    ..
                } => {
                    inputs.extend(self.value(left, *width, dag, false)?);
                    inputs.extend(self.value(right, *width, dag, false)?);
                    if let MirOp::Compare {
                        dst, left, right, ..
                    } = &mut shape
                    {
                        *dst = MirCondDest::Temp(MirTempId(0));
                        *left = zero.clone();
                        *right = zero;
                    }
                    (MirDef::VTemp(*id), MirWidth::Byte, false)
                }
                MirOp::RuntimeHelper { helper, .. } if helper.is_wide() => {
                    let inputs = (0x82..=0x85)
                        .chain(0xC0..=0xC3)
                        .map(|byte| self.memory.get(&byte).copied())
                        .collect::<Option<Value>>()?;
                    let index = self.events.len();
                    self.events.push((op.clone(), inputs));
                    self.memory.clear();
                    self.carry = None;
                    for byte in 0..4 {
                        self.memory
                            .insert(0xC4 + byte, dag.intern(Node::Result(index, byte))?);
                    }
                    continue;
                }
                _ => return None,
            };
            let value = (0..bytes(width))
                .map(|byte| dag.intern(Node::Compute(shape.clone(), inputs.clone(), byte)))
                .collect::<Option<Value>>()?;
            self.define(&dst, value)?;
            // Preserve only the explicitly declared arithmetic carry chain.
            if produces_carry {
                self.carry = Some(dag.intern(Node::Compute(shape, inputs, 2))?);
            }
        }
        Some(())
    }
}

pub(super) fn inputs_correspond(
    leaf: &LeafRoutine,
    args: &[MirCallArg],
    expanded: &[MirOp],
) -> bool {
    if !leaf
        .routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .any(|op| matches!(op, MirOp::RuntimeHelper { .. }))
    {
        return true;
    }
    let check = || -> Option<bool> {
        if leaf.routine.blocks.len() != 1 {
            return None;
        }
        let mut dag = Dag::default();
        let mut old = State::default();
        let mut new = State::default();
        let params = args
            .iter()
            .zip(&leaf.params)
            .map(|(arg, param)| {
                Some((
                    (param.id, param.offset),
                    new.value(&arg.value, arg.width, &mut dag, true)?,
                ))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        old.run(&leaf.routine.blocks[0].ops, &params, &mut dag)?;
        new.run(expanded, &BTreeMap::new(), &mut dag)?;
        Some(old.events == new.events)
    };
    check() == Some(true)
}
