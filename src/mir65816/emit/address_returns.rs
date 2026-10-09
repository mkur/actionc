//! A pure address expression and its sole Return share one deferred owner.
//! Logical reads keep their original sites; the physical read belongs to Return.
use super::*;
use crate::mir65816::analysis::ProgramPoint;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "address_return_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Expression {
    pub block: BlockId,
    pub source: TempId,
    pub read: ProgramPoint,
    pub consumer: ProgramPoint,
    pub offset: u16,
    pub nodes: Vec<(TempId, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Contract {
    pub expression: Expression,
    pub source: Location,
}

fn pointer(r: &Mir65816Routine, id: TempId) -> bool {
    r.temps
        .iter()
        .any(|(t, ty)| *t == id && allocation::pointer_type(ty))
}

fn candidate(
    r: &Mir65816Routine,
    b: &Mir65816Block,
    counts: &BTreeMap<TempId, usize>,
    definitions: &BTreeMap<TempId, usize>,
) -> Option<Expression> {
    let Mir65816Terminator::Return {
        value: Some(Mir65816Value::Temp(mut wanted, bytes)),
        ..
    } = b.terminator
    else {
        return None;
    };
    if bytes.get() != 3 || !pointer(r, wanted) || r.result_home != home_demand::result_home(3) {
        return None;
    }
    let mut nodes = Vec::new();
    let mut offset = 0u16;
    let mut address_seen = false;
    for (index, op) in b.ops.iter().enumerate().rev().take(16) {
        if liveness::operation_output(op) != Some(wanted)
            || counts.get(&wanted) != Some(&1)
            || definitions.get(&wanted) != Some(&1)
            || !pointer(r, wanted)
        {
            return None;
        }
        nodes.push((wanted, index));
        match op {
            Mir65816Op::Cast {
                kind: NirCastKind::Pointer,
                ..
            } => {
                let (_, source) = allocation::pointer_identity(r, op)?;
                wanted = source;
            }
            Mir65816Op::AddressOf { address, width, .. } if width.get() == 3 => {
                let Mir65816AddressBase::Indirect(Mir65816Value::Temp(source, bytes)) =
                    address.base
                else {
                    return None;
                };
                if bytes.get() != 3 || !pointer(r, source) {
                    return None;
                }
                offset = offset.checked_add(address_offsets::constant(address)?)?;
                address_seen = true;
                wanted = source;
            }
            _ => return None,
        }
        // Compose the complete consecutive family. An unsupported address or
        // cast in this family rejects the whole trial, never a partial suffix.
        let continues = index
            .checked_sub(1)
            .and_then(|i| b.ops.get(i))
            .is_some_and(|op| {
                liveness::operation_output(op) == Some(wanted)
                    && matches!(op, Mir65816Op::AddressOf { .. } | Mir65816Op::Cast { .. })
            });
        if !continues {
            if !address_seen {
                return None;
            }
            nodes.reverse();
            return Some(Expression {
                block: b.id,
                source: wanted,
                read: ProgramPoint { block: b.id, index },
                consumer: ProgramPoint {
                    block: b.id,
                    index: b.ops.len(),
                },
                offset,
                nodes,
            });
        }
    }
    None
}

impl Expression {
    pub(super) fn resolve(
        &self,
        r: &Mir65816Routine,
        demand: &home_demand::Plan,
        frame: &AllocatedFrame,
    ) -> Result<Contract, String> {
        // Borrow permission is checked at the ORIGINAL logical read, never
        // obtained by looking up the omitted temp at a new consumer site.
        let pointers = demand.pointers.resolve(r, frame)?;
        let source = if let Some(slot) =
            pointers.read_home((self.read.block, self.read.index), self.source)
        {
            Location::Stack(slot)
        } else {
            if demand.omits(self.source) {
                return Err("deferred address lost its source owner".into());
            }
            *frame
                .temps
                .get(&self.source)
                .ok_or("missing deferred address source capture")?
        };
        let slot = source.slot();
        if slot.width != 3 {
            return Err("partial deferred address source".into());
        }
        match source {
            Location::Stack(slot) => {
                abi::stack::access_displacement(
                    ByteOffset::new(slot.offset.into()),
                    ByteSize::new(3),
                    ByteSize::ZERO,
                )
                .map_err(|e| e.to_string())?;
            }
            Location::DirectPage(slot) => {
                if resources::Scratch::range(slot.offset, 3).is_none() {
                    return Err("unowned deferred address source".into());
                }
            }
        }
        Ok(Contract {
            expression: self.clone(),
            source,
        })
    }
}

pub(super) fn admit(
    r: &Mir65816Routine,
    demand: &mut home_demand::Plan,
    counts: &BTreeMap<TempId, usize>,
    definitions: &BTreeMap<TempId, usize>,
) {
    for b in &r.blocks {
        let Some(expression) = candidate(r, b, counts, definitions) else {
            continue;
        };
        // Existing closed register/native/component owners are never displaced.
        if expression.nodes.iter().any(|(t, _)| demand.omits(*t)) {
            continue;
        }
        let Ok(before) = AllocatedFrame::with_demand(r, demand) else {
            continue;
        };
        if expression.resolve(r, demand, &before).is_err() {
            continue;
        }
        let old: Vec<_> = expression
            .nodes
            .iter()
            .map(|&(temp, producer)| {
                (
                    temp,
                    demand.decisions.insert(
                        temp,
                        home_demand::Decision::DeferredAddress(home_demand::Accumulator {
                            block: b.id,
                            producer,
                            consumer: producer + 1,
                            bytes: 3,
                        }),
                    ),
                )
            })
            .collect();
        let previous_mixed = demand.mixed.clone();
        demand.mixed = mixed::Plan::new(r, demand);
        let accepted = AllocatedFrame::with_demand(r, demand).is_ok_and(|after| {
            after.extent <= before.extent
                && after.spill_bytes <= before.spill_bytes
                && after.peak_below_entry <= before.peak_below_entry
                && expression.resolve(r, demand, &after).is_ok()
        });
        if accepted {
            demand.address_returns.insert(b.id, expression);
        } else {
            demand.mixed = previous_mixed;
            for (temp, decision) in old {
                if let Some(decision) = decision {
                    demand.decisions.insert(temp, decision);
                } else {
                    demand.decisions.remove(&temp);
                }
            }
        }
    }
}
