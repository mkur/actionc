//! Read-only, bounded bindings to authoritative three-byte pointer homes.
use super::*;

#[cfg(test)]
#[path = "pointer_forwarding_tests.rs"]
mod tests;

/// An incoming home is never registered as a writable temporary allocation.
#[derive(Clone, Copy, Debug)]
pub(super) struct Source {
    parameter: ParamId,
    home: Slot,
}

impl Source {
    pub(super) fn memory(self) -> Memory {
        Memory::Stack(self.home.offset.into())
    }
}

#[derive(Debug)]
struct Binding {
    temp: TempId,
    source: Source,
    definition: (BlockId, usize),
    uses: BTreeSet<usize>,
}

#[derive(Default, Debug)]
pub(super) struct Plan {
    bindings: Vec<Binding>,
}

fn canonical(address: &Mir65816Address) -> bool {
    address.index.is_none() && address.displacement.get() == 0
}

fn incoming(
    routine: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
) -> Result<Option<Source>, String> {
    let Mir65816AddressBase::Parameter(id) = address.base else {
        return Ok(None);
    };
    if !canonical(address) {
        return Ok(None);
    }
    let p = routine
        .frame
        .parameters
        .iter()
        .find(|p| p.param == id)
        .ok_or("unknown parameter")?;
    if p.frame_object.is_some()
        || !matches!(p.incoming, Mir65816AbiHome::StackArgument {size, ..} if size.get() == 3)
        || routine
            .frame
            .objects
            .iter()
            .any(|o| o.owner == Mir65816FrameObjectOwner::Param(id))
    {
        return Ok(None);
    }
    // Check operations as well as frame metadata: a forged immutable home may
    // not conceal writes, partial access, volatility, escape or aggregate Copy.
    let refers = |a: &Mir65816Address| a.base == Mir65816AddressBase::Parameter(id);
    if routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .any(|op| match op {
            Mir65816Op::Load {
                address,
                width,
                volatile,
                ..
            } => refers(address) && (*volatile || width.get() != 3 || !canonical(address)),
            Mir65816Op::Store { address, .. } | Mir65816Op::AddressOf { address, .. } => {
                refers(address)
            }
            Mir65816Op::Copy {
                source,
                destination,
                ..
            } => refers(source) || refers(destination),
            _ => false,
        })
    {
        return Ok(None);
    }
    let offset = frame.incoming_home(routine, id)?;
    abi::stack::access_displacement(ByteOffset::new(offset), ByteSize::new(3), ByteSize::ZERO)
        .map_err(|e| e.to_string())?;
    Ok(Some(Source {
        parameter: id,
        home: Slot {
            offset: offset as u16,
            width: 3,
        },
    }))
}

fn barrier(op: &Mir65816Op) -> bool {
    match op {
        Mir65816Op::Call { .. } | Mir65816Op::Store { .. } | Mir65816Op::Copy { .. } => true,
        Mir65816Op::Load { volatile, .. } => *volatile,
        Mir65816Op::AddressOf { .. }
        | Mir65816Op::Unary { .. }
        | Mir65816Op::Cast { .. }
        | Mir65816Op::PointerOffset { .. }
        | Mir65816Op::Binary { .. }
        | Mir65816Op::Compare { .. } => false,
    }
}

/// Only read paths that resolve complete native pointer homes participate.
/// In particular, identity casts and edge-copy schedules retain their captures.
fn supported(op: &Mir65816Op, temp: TempId) -> bool {
    let is_pointer = |value: &Mir65816Value| matches!(value, Mir65816Value::Temp(id,w) if *id==temp && w.get()==3);
    let contains = |value: &Mir65816Value| matches!(value, Mir65816Value::Temp(id,_) if *id==temp);
    match op {
        Mir65816Op::Load {
            address,
            volatile: false,
            ..
        }
        | Mir65816Op::AddressOf { address, .. } => {
            matches!(&address.base, Mir65816AddressBase::Indirect(v) if is_pointer(v))
                && address.displacement.get() <= u16::MAX.into()
                && address
                    .index
                    .as_ref()
                    .is_none_or(|index| !contains(&index.value))
        }
        Mir65816Op::Compare {
            width, left, right, ..
        } => {
            width.get() == 3
                && (!contains(left) || is_pointer(left))
                && (!contains(right) || is_pointer(right))
        }
        Mir65816Op::PointerOffset {
            width,
            base,
            offset,
            ..
        } => width.get() == 3 && is_pointer(base) && !contains(offset),
        Mir65816Op::Binary {
            width,
            left,
            right,
            operation: NirBinaryOp::Add | NirBinaryOp::Sub,
            ..
        } => {
            width.get() == 3
                && (!contains(left) || is_pointer(left))
                && (!contains(right) || is_pointer(right))
        }
        _ => false,
    }
}

impl Plan {
    pub(super) fn new(routine: &Mir65816Routine, frame: &AllocatedFrame) -> Result<Self, String> {
        let mut plan = Self::default();
        let counts = liveness::input_counts(routine);
        for block in &routine.blocks {
            for (index, op) in block.ops.iter().enumerate() {
                let Mir65816Op::Load {
                    dest,
                    width,
                    address,
                    volatile: false,
                } = op
                else {
                    continue;
                };
                if width.get() != 3 || counts.get(dest).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let Some(Location::Stack(capture)) = frame.temps.get(dest) else {
                    continue;
                };
                if capture.width != 3 {
                    return Err("pointer capture width mismatch".into());
                }
                let Some(source) = incoming(routine, frame, address)? else {
                    continue;
                };
                abi::stack::access_displacement(
                    ByteOffset::new(capture.offset.into()),
                    ByteSize::new(3),
                    ByteSize::ZERO,
                )
                .map_err(|e| e.to_string())?;
                if Location::Stack(*capture).overlaps(Location::Stack(source.home)) {
                    return Err("pointer capture overlaps authoritative source".into());
                }
                let mut uses = BTreeSet::new();
                let mut covered = 0;
                for (at, consumer) in block.ops.iter().enumerate().skip(index + 1) {
                    // Even a final call/store use is excluded. Unknown writes
                    // cannot extend a private source's proven stable window.
                    if barrier(consumer) {
                        break;
                    }
                    let occurrences = liveness::operation_inputs(consumer)
                        .iter()
                        .filter(|id| **id == *dest)
                        .count();
                    if occurrences != 0 {
                        if !supported(consumer, *dest) {
                            break;
                        }
                        uses.insert(at);
                        covered += occurrences;
                    }
                    if covered == counts[dest] {
                        break;
                    }
                }
                if covered != counts[dest]
                    && !block.ops[index + 1..].iter().any(barrier)
                    && matches!(
                        routine.result_home,
                        Some(Mir65816AbiHome::NativeResult(
                            abi::ResultLocation::A16X8ZeroExtended
                        ))
                    )
                    && matches!(&block.terminator, Mir65816Terminator::Return {value:Some(Mir65816Value::Temp(id,w)),..} if id==dest && w.get()==3)
                {
                    uses.insert(block.ops.len());
                    covered += 1;
                }
                // The exhaustive routine-wide count includes hidden address and
                // index uses, all terminators/edges and other blocks. Any use
                // not explicitly admitted retains the entire original capture.
                if covered != counts[dest] {
                    continue;
                }
                plan.bindings.push(Binding {
                    temp: *dest,
                    source,
                    definition: (block.id, index),
                    uses,
                });
            }
        }
        Ok(plan)
    }

    /// Install only the preflighted read bindings for this exact source site.
    /// Writes continue to use AllocatedFrame::temps, even for a bound TempId.
    pub(super) fn enter(&self, b: &mut Builder<'_>, block: BlockId, index: usize) -> bool {
        b.borrowed.clear();
        for binding in &self.bindings {
            if binding.definition.0 == block && binding.uses.contains(&index) {
                debug_assert_eq!(
                    b.frame
                        .incoming_home(b.routine, binding.source.parameter)
                        .unwrap(),
                    u32::from(binding.source.home.offset)
                );
                b.borrowed.insert(binding.temp, binding.source);
            }
        }
        if self
            .bindings
            .iter()
            .any(|binding| binding.definition == (block, index))
        {
            // Retain the operation's conservative fact boundary, but publish no
            // fabricated store/definition for the reserved, unwritten home.
            b.code.barrier();
            true
        } else {
            false
        }
    }
}
