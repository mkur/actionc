//! Typed value consumers shared by target selection and allocation.
use super::*;
use std::collections::BTreeMap;

fn address_values<'a>(address: &'a Mir68kAddress, values: &mut Vec<&'a Mir68kValue>) {
    if let Mir68kAddressBase::Indirect(value) = &address.base {
        values.push(value);
    }
    if let Some(index) = &address.index {
        values.push(&index.value);
    }
}

pub(super) fn op_values(op: &Mir68kOp) -> Vec<&Mir68kValue> {
    let mut values = Vec::new();
    match op {
        Mir68kOp::Load { address, .. } | Mir68kOp::AddressOf { address, .. } => {
            address_values(address, &mut values)
        }
        Mir68kOp::Store { address, value, .. } => {
            address_values(address, &mut values);
            values.push(value);
        }
        Mir68kOp::Copy {
            destination,
            source,
            ..
        } => {
            address_values(destination, &mut values);
            address_values(source, &mut values);
        }
        Mir68kOp::Unary { value, .. } | Mir68kOp::Cast { value, .. } => values.push(value),
        Mir68kOp::PointerOffset { base, offset, .. } => values.extend([base, offset]),
        Mir68kOp::Binary { left, right, .. } | Mir68kOp::Compare { left, right, .. } => {
            values.extend([left, right])
        }
        Mir68kOp::Call { target, args, .. } => {
            values.extend(args);
            if let Mir68kCallTarget::Indirect(value, _) = target {
                values.push(value);
            }
        }
        Mir68kOp::Fault(_) => {}
    }
    values
}

pub(super) fn edges(term: &Mir68kTerminator) -> Vec<&Mir68kEdge> {
    match term {
        Mir68kTerminator::Goto(edge) => vec![edge],
        Mir68kTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        _ => Vec::new(),
    }
}

pub(super) fn term_values(term: &Mir68kTerminator) -> Vec<&Mir68kValue> {
    let mut values: Vec<_> = edges(term)
        .into_iter()
        .flat_map(|edge| &edge.args)
        .collect();
    match term {
        Mir68kTerminator::Branch { condition, .. } => values.push(condition),
        Mir68kTerminator::Return {
            value: Some(value), ..
        } => values.push(value),
        _ => {}
    }
    values
}

pub(super) fn use_counts(routine: &Mir68kRoutine) -> BTreeMap<TempId, usize> {
    let mut uses = BTreeMap::new();
    for block in &routine.blocks {
        for value in block
            .ops
            .iter()
            .flat_map(op_values)
            .chain(term_values(&block.terminator))
        {
            if let Mir68kValue::Temp(id, _) = value {
                *uses.entry(*id).or_default() += 1;
            }
        }
    }
    uses
}

pub(super) fn branch_compare<'a>(
    block: &'a Mir68kBlock,
    uses: &BTreeMap<TempId, usize>,
) -> Option<&'a Mir68kOp> {
    let compare @ Mir68kOp::Compare { dest, .. } = block.ops.last()? else {
        return None;
    };
    let Mir68kTerminator::Branch {
        condition: Mir68kValue::Temp(id, _),
        ..
    } = &block.terminator
    else {
        return None;
    };
    (id == dest && uses.get(id) == Some(&1)).then_some(compare)
}
