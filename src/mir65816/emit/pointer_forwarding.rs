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

impl Plan {
    pub(super) fn new(routine: &Mir65816Routine, frame: &AllocatedFrame) -> Result<Self, String> {
        let mut plan = Self::default();
        let counts = liveness::input_counts(routine);
        for block in &routine.blocks {
            for (index, pair) in block.ops.windows(2).enumerate() {
                let Mir65816Op::Load {
                    dest,
                    width,
                    address,
                    volatile: false,
                } = &pair[0]
                else {
                    continue;
                };
                if width.get() != 3 || counts.get(dest) != Some(&1) {
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
                let Mir65816Op::Load {
                    address,
                    volatile: false,
                    ..
                } = &pair[1]
                else {
                    continue;
                };
                if !matches!(address.base, Mir65816AddressBase::Indirect(Mir65816Value::Temp(id,w)) if id == *dest && w.get() == 3)
                    || address.index.is_some()
                    || address.displacement.get() > u16::MAX.into()
                {
                    continue;
                }
                // Both the original and substituted pointer setup use exactly
                // two native private reads; every byte fits d,S at body depth.
                abi::stack::access_displacement(
                    ByteOffset::new(capture.offset.into()),
                    ByteSize::new(3),
                    ByteSize::ZERO,
                )
                .map_err(|e| e.to_string())?;
                if Location::Stack(*capture).overlaps(Location::Stack(source.home)) {
                    return Err("pointer capture overlaps authoritative source".into());
                }
                plan.bindings.push(Binding {
                    temp: *dest,
                    source,
                    definition: (block.id, index),
                    uses: [index + 1].into(),
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
