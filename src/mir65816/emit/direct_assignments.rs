//! Adjacent sole-use scalar assignments between disjoint allocated objects.
use super::*;

#[cfg(test)]
#[path = "direct_assignment_tests.rs"]
mod tests;

// Allocated symbols have link-checked, nonoverlapping extents. An absolute
// placement or alias does not establish that ownership, even without VOLATILE.
fn place(
    routine: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    data: &[Mir65816Data],
    bytes: u32,
    write: bool,
) -> Option<Memory> {
    if address.index.is_some() {
        return None;
    }
    let offset = address.displacement.get();
    let end = offset.checked_add(bytes)?;
    let id = match address.base {
        Mir65816AddressBase::AutomaticFrame(id) => {
            let object = routine.frame.objects.iter().find(|o| o.id == id)?;
            if end > object.size.get() || write && !object.mutable {
                return None;
            }
            return Some(Memory::Stack(
                object.stack_offset.get().checked_add(offset)?,
            ));
        }
        Mir65816AddressBase::Parameter(id) => {
            let parameter = routine.frame.parameters.iter().find(|p| p.param == id)?;
            // An incoming argument is read-only; mutable parameters have their
            // own frame object, which parameter_home resolves after allocation.
            if write && parameter.frame_object.is_none() {
                return None;
            }
            let (home, size) = frame.parameter_home(routine, id).ok()?;
            if end > u32::from(size) {
                return None;
            }
            return Some(Memory::Stack(home.checked_add(offset)?));
        }
        Mir65816AddressBase::Static(NirStorageId::Global(id))
        | Mir65816AddressBase::External(Mir65816ExternalAddress::Global(id)) => {
            Mir65816DataId::Global(id)
        }
        _ => return None,
    };
    let object = data.iter().find(|d| d.id == id)?;
    if object.placement != Mir65816DataPlacement::Allocate
        || write && !object.mutable
        || end > object.size.get()
    {
        return None;
    }
    Some(Memory::Symbol(Target::Data(id), offset))
}

struct Assignment {
    source: Memory,
    destination: Memory,
    bytes: u8,
}

pub(super) struct Plan(BTreeMap<usize, Assignment>);

impl Plan {
    pub(super) fn new(
        routine: &Mir65816Routine,
        frame: &AllocatedFrame,
        block: &Mir65816Block,
        counts: &BTreeMap<TempId, usize>,
        data: &[Mir65816Data],
    ) -> Self {
        let mut assignments = BTreeMap::new();
        for (index, ops) in block.ops.windows(2).enumerate() {
            let (
                Mir65816Op::Load {
                    dest,
                    width,
                    address: source,
                    volatile: false,
                },
                Mir65816Op::Store {
                    address: destination,
                    value: Mir65816Value::Temp(value, value_width),
                    width: stored_width,
                    volatile: false,
                },
            ) = (&ops[0], &ops[1])
            else {
                continue;
            };
            let bytes = width.get();
            if !(1..=4).contains(&bytes)
                || dest != value
                || width != value_width
                || width != stored_width
                || counts.get(dest) != Some(&1)
            {
                continue;
            }
            let (Some(source), Some(destination)) = (
                place(routine, frame, source, data, bytes, false),
                place(routine, frame, destination, data, bytes, true),
            ) else {
                continue;
            };
            let disjoint = match (source, destination) {
                (Memory::Stack(a), Memory::Stack(b)) => a.abs_diff(b) >= bytes,
                (Memory::Symbol(a, x), Memory::Symbol(b, y)) => a != b || x.abs_diff(y) >= bytes,
                // The ABI keeps invocation storage separate from linked data.
                (Memory::Stack(_), Memory::Symbol(..)) | (Memory::Symbol(..), Memory::Stack(_)) => {
                    true
                }
                _ => false,
            };
            if !disjoint {
                continue;
            }
            assignments.insert(
                index + 1,
                Assignment {
                    source,
                    destination,
                    bytes: bytes as u8,
                },
            );
        }
        Self(assignments)
    }

    pub(super) fn emit(&self, b: &mut Builder<'_>, index: usize) -> Result<bool, String> {
        if let Some(copy) = self.0.get(&index) {
            // No instruction intervenes between the original load and store.
            // Reuse the ordinary scalar transfer, including its private-frame
            // word selection; external bytes are never accessed twice.
            b.code.barrier();
            b.transfer(copy.source, copy.destination, copy.bytes, true)?;
            return Ok(true);
        }
        if self.0.contains_key(&(index + 1)) {
            b.code.barrier();
            return Ok(true);
        }
        Ok(false)
    }
}
