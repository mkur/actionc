//! Read-only, bounded bindings to authoritative three-byte pointer homes.
use super::*;

#[cfg(test)]
#[path = "pointer_forwarding_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceKind {
    Parameter(ParamId),
    FrameObject(Mir65816FrameObjectId),
}

/// A source home is never registered as a writable temporary allocation.
#[derive(Clone, Copy, Debug)]
pub(super) struct Source {
    kind: SourceKind,
    home: Slot,
}

impl Source {
    pub(super) fn memory(self) -> Memory {
        Memory::Stack(self.home.offset.into())
    }
}

#[derive(Clone, Debug)]
struct Binding {
    temp: TempId,
    source: Source,
    definition: (BlockId, usize),
    uses: BTreeSet<usize>,
}

#[derive(Clone, Default, Debug)]
pub(in crate::mir65816::emit) struct Plan {
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
        kind: SourceKind::Parameter(id),
        home: Slot {
            offset: offset as u16,
            width: 3,
        },
    }))
}

fn local(routine: &Mir65816Routine, address: &Mir65816Address) -> Result<Option<Source>, String> {
    let Mir65816AddressBase::AutomaticFrame(id) = address.base else {
        return Ok(None);
    };
    if !canonical(address) {
        return Ok(None);
    }
    let object = routine
        .frame
        .objects
        .iter()
        .find(|o| o.id == id)
        .ok_or("unknown pointer source object")?;
    if object.addressable
        || object.size.get() != 3
        || !matches!(object.owner, Mir65816FrameObjectOwner::Local(_))
        || routine
            .frame
            .parameters
            .iter()
            .any(|p| p.frame_object == Some(id))
    {
        return Ok(None);
    }
    let refers = |a: &Mir65816Address| a.base == Mir65816AddressBase::AutomaticFrame(id);
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
            }
            | Mir65816Op::Store {
                address,
                width,
                volatile,
                ..
            } => refers(address) && (*volatile || width.get() != 3 || !canonical(address)),
            Mir65816Op::AddressOf { address, .. } => refers(address),
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
    let offset = object.stack_offset;
    let end = offset
        .get()
        .checked_add(2)
        .ok_or("pointer source extent overflow")?;
    if offset.get() == 0 || end > routine.frame.extent.get() {
        return Err("pointer source exceeds automatic frame extent".into());
    }
    if routine.frame.objects.iter().any(|other| {
        other.id != id
            && u64::from(offset.get())
                < u64::from(other.stack_offset.get()) + u64::from(other.size.get())
            && u64::from(other.stack_offset.get()) <= u64::from(end)
    }) {
        return Err("pointer source overlaps another frame object".into());
    }
    abi::stack::access_displacement(offset, ByteSize::new(3), ByteSize::ZERO)
        .map_err(|e| e.to_string())?;
    Ok(Some(Source {
        kind: SourceKind::FrameObject(id),
        home: Slot {
            offset: offset.get() as u16,
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

/// Direct private destinations must contain all three bytes and be disjoint
/// from the authoritative source. Indexed private destinations retain captures:
/// their dynamic geometry is outside this local proof.
fn stored_value_destination(
    routine: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    source: Source,
) -> bool {
    let (start, size) = match address.base {
        Mir65816AddressBase::AutomaticFrame(id) => {
            let Some(object) = routine.frame.objects.iter().find(|o| o.id == id) else {
                return false;
            };
            if object.stack_offset.get() == 0
                || u64::from(object.stack_offset.get()) + u64::from(object.size.get())
                    > u64::from(routine.frame.extent.get()) + 1
            {
                return false;
            }
            (object.stack_offset.get(), object.size.get())
        }
        Mir65816AddressBase::Parameter(id) => {
            let Ok((offset, size)) = frame.parameter_home(routine, id) else {
                return false;
            };
            (offset, u32::from(size))
        }
        Mir65816AddressBase::Static(NirStorageId::Global(_))
        | Mir65816AddressBase::External(_)
        | Mir65816AddressBase::Indirect(_) => return true,
        Mir65816AddressBase::Static(_) => return false,
    };
    let displacement = address.displacement.get();
    if address.index.is_some() || u64::from(displacement) + 3 > u64::from(size) {
        return false;
    }
    let Some(offset) = start.checked_add(displacement) else {
        return false;
    };
    offset != u32::from(source.home.offset)
        && abi::stack::access_displacement(
            ByteOffset::new(offset),
            ByteSize::new(3),
            ByteSize::ZERO,
        )
        .is_ok()
        && Builder::private_pointer_geometry(source.memory(), Memory::Stack(offset))
}

/// Consume a private source at a barrier, never through it. Address and payload
/// selectors resolve borrowed reads through value_memory, without requiring the
/// omitted temporary's reserved home to have been initialized.
fn terminal(
    routine: &Mir65816Routine,
    frame: &AllocatedFrame,
    op: &Mir65816Op,
    temp: TempId,
    source: Source,
) -> bool {
    if let Mir65816Op::Call {
        target: Mir65816CallTarget::Direct(_),
        args,
        plan,
        ..
    } = op
    {
        // The original incoming slot can be farther from S than its capture.
        // Reject the binding (not the valid call) if full outgoing reservation
        // would put any source byte outside d,S. Pushes check each actual delta.
        return args.len() == plan.arguments.len()
            && effects::CallContract::from_plan(plan, abi::FarTransfer::Jsl).is_ok()
            && abi::stack::access_displacement(
                ByteOffset::new(source.home.offset.into()),
                ByteSize::new(3),
                plan.outgoing_bytes,
            )
            .is_ok()
            && args.iter().zip(&plan.arguments).all(|(value, home)| {
                !matches!(value, Mir65816Value::Temp(id,_) if *id==temp)
                    || matches!((value,home), (Mir65816Value::Temp(_,w),Mir65816AbiHome::StackArgument{size,..}) if w.get()==3 && size.get()==3)
            });
    }
    let Mir65816Op::Store {
        address,
        value,
        width,
        volatile: false,
    } = op
    else {
        return false;
    };
    if address.displacement.get() > u16::MAX.into()
        || address
            .index
            .as_ref()
            .is_some_and(|index| matches!(&index.value, Mir65816Value::Temp(id,_) if *id==temp))
    {
        return false;
    }
    if matches!(value, Mir65816Value::Temp(id,_) if *id==temp) {
        width.get() == 3
            && matches!(value, Mir65816Value::Temp(_,w) if w.get()==3)
            && !matches!(&address.base, Mir65816AddressBase::Indirect(Mir65816Value::Temp(id,_)) if *id==temp)
            && stored_value_destination(routine, frame, address, source)
    } else {
        (1..=4).contains(&width.get())
            && matches!(&address.base, Mir65816AddressBase::Indirect(Mir65816Value::Temp(id,w)) if *id==temp && w.get()==3)
    }
}

pub(in crate::mir65816::emit) fn pointer_alias(r: &Mir65816Routine, op: &Mir65816Op) -> Option<(TempId, TempId)> {
    let (dest, value) = match op {
        Mir65816Op::Cast { dest, from, to, value, kind, .. }
            if from.get() == 3 && to == from && matches!(kind, NirCastKind::Pointer | NirCastKind::Integer | NirCastKind::IntegerToPointer | NirCastKind::PointerToInteger) => (*dest, value),
        Mir65816Op::AddressOf { dest, width, address }
            if width.get() == 3 && canonical(address) => {
                let Mir65816AddressBase::Indirect(value) = &address.base else { return None; };
                (*dest, value)
            }
        _ => return None,
    };
    let Mir65816Value::Temp(input, width) = value else { return None; };
    let scalar24 = |id| r.temps.iter().any(|(t, ty)| *t == id && ty.width == Some(ByteSize::new(3))
        && (ty.pointer || ty.kind.integer().is_some_and(|i| i.bits == 24)));
    (width.get() == 3 && scalar24(dest) && scalar24(*input)).then_some((dest, *input))
}

impl Plan {
    pub(in crate::mir65816::emit) fn new(routine: &Mir65816Routine, frame: &AllocatedFrame) -> Result<Self, String> {
        let mut plan = Self::default();
        let counts = liveness::input_counts(routine);
        let definitions = liveness::definition_counts(routine);
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
                if let Some(home) = frame.temps.get(dest) {
                    let Location::Stack(capture) = home else { continue; };
                    if capture.width != 3 { return Err("pointer capture width mismatch".into()); }
                }
                let Some(source) = (match address.base {
                    Mir65816AddressBase::Parameter(_) => incoming(routine, frame, address)?,
                    Mir65816AddressBase::AutomaticFrame(_) => local(routine, address)?,
                    _ => None,
                }) else {
                    continue;
                };
                if frame
                    .temps
                    .values()
                    .any(|home| home.overlaps(Location::Stack(source.home)))
                {
                    return Err("temporary overlaps authoritative pointer source".into());
                }
                // Aliases share the same authoritative source, but retain
                // separate definitions/use sets. Admission is atomic over the
                // complete group, including otherwise hidden address uses.
                if definitions.get(dest) != Some(&1) { continue; }
                let mut group = BTreeMap::from([(*dest, (index, BTreeSet::new()))]);
                let mut covered = 0;
                for (at, consumer) in block.ops.iter().enumerate().skip(index + 1) {
                    let inputs = liveness::operation_inputs(consumer);
                    let used: Vec<_> = group.keys().copied().filter(|id| inputs.contains(id)).collect();
                    let occurrences = inputs.iter().filter(|id| group.contains_key(id)).count();
                    if barrier(consumer) {
                        if occurrences != 0
                            && covered + occurrences == group.keys().map(|id| counts[id]).sum::<usize>()
                            && used.iter().all(|id| terminal(routine, frame, consumer, *id, source))
                        {
                            for id in used { group.get_mut(&id).unwrap().1.insert(at); }
                            covered += occurrences;
                        }
                        break;
                    }
                    let alias = pointer_alias(routine, consumer).filter(|(_, input)| group.contains_key(input));
                    if occurrences != 0 {
                        if alias.is_none() && !used.iter().all(|id| supported(consumer, *id)) { break; }
                        for id in used { group.get_mut(&id).unwrap().1.insert(at); }
                        covered += occurrences;
                    }
                    if let Some((alias, _)) = alias {
                        if definitions.get(&alias) != Some(&1) || counts.get(&alias).copied().unwrap_or(0) == 0 { break; }
                        group.insert(alias, (at, BTreeSet::new()));
                    }
                    if covered == group.keys().map(|id| counts[id]).sum::<usize>() { break; }
                }
                if covered != group.keys().map(|id| counts[id]).sum::<usize>()
                    && !block.ops[index + 1..].iter().any(barrier)
                    && routine.result_home == Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended))
                    && let Mir65816Terminator::Return { value: Some(Mir65816Value::Temp(id, w)), .. } = &block.terminator
                    && w.get() == 3
                    && let Some((_, uses)) = group.get_mut(id)
                {
                    uses.insert(block.ops.len());
                    covered += 1;
                }
                if covered != group.keys().map(|id| counts[id]).sum::<usize>() { continue; }
                for (temp, (at, uses)) in group {
                    plan.bindings.push(Binding { temp, source, definition: (block.id, at), uses });
                }
            }
        }
        Ok(plan)
    }

    pub(in crate::mir65816::emit) fn temps(&self) -> impl Iterator<Item = TempId> + '_ {
        self.bindings.iter().map(|b| b.temp)
    }

    pub(super) fn resolve(&self, routine: &Mir65816Routine, frame: &AllocatedFrame) -> Result<Self, String> {
        let checked = Self::new(routine, frame)?;
        let mut resolved = Self::default();
        for binding in &self.bindings {
            let actual = checked.bindings.iter().find(|b| b.temp == binding.temp
                && b.definition == binding.definition && b.uses == binding.uses
                && b.source.kind == binding.source.kind)
                .ok_or("borrowed pointer demand no longer valid in final frame")?;
            resolved.bindings.push(actual.clone());
        }
        Ok(resolved)
    }

    /// Install only the preflighted read bindings for this exact source site.
    /// Writes continue to use AllocatedFrame::temps, even for a bound TempId.
    pub(super) fn enter(&self, b: &mut Builder<'_>, block: BlockId, index: usize) -> bool {
        b.borrowed.clear();
        for binding in &self.bindings {
            if binding.definition.0 == block && binding.uses.contains(&index) {
                debug_assert_eq!(
                    match binding.source.kind {
                        SourceKind::Parameter(id) => b.frame.incoming_home(b.routine, id).unwrap(),
                        SourceKind::FrameObject(id) => b.object(id).unwrap(),
                    },
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
            // fabricated store/definition for the omitted home.
            b.code.barrier();
            true
        } else {
            false
        }
    }
}
