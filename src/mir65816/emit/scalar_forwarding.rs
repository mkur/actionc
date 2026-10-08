//! Single-use scalar reads from authoritative private homes and bounded calls.
use super::*;

#[cfg(test)]
#[path = "scalar_forwarding_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceKind {
    Parameter(ParamId),
    FrameObject(Mir65816FrameObjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Source {
    kind: SourceKind,
    pub(super) home: Slot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    temp: TempId,
    source: Source,
    definition: (BlockId, usize),
    consumer: usize,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(in crate::mir65816::emit) struct Plan {
    bindings: Vec<Binding>,
}

fn canonical(address: &Mir65816Address) -> bool {
    address.index.is_none() && address.displacement.get() == 0
}

fn incoming(
    r: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    bytes: u8,
) -> Result<Option<Source>, String> {
    let Mir65816AddressBase::Parameter(id) = address.base else {
        return Ok(None);
    };
    let p = r
        .frame
        .parameters
        .iter()
        .find(|p| p.param == id)
        .ok_or("unknown scalar parameter")?;
    if !canonical(address)
        || p.frame_object.is_some()
        || !matches!(p.incoming, Mir65816AbiHome::StackArgument {size, ..} if size.get() == u32::from(bytes))
        || r.frame
            .objects
            .iter()
            .any(|o| o.owner == Mir65816FrameObjectOwner::Param(id))
    {
        return Ok(None);
    }
    let refers = |a: &Mir65816Address| a.base == address.base;
    if r.blocks.iter().flat_map(|b| &b.ops).any(|op| match op {
        Mir65816Op::Load {
            address,
            width,
            volatile,
            ..
        } => {
            refers(address) && (*volatile || width.get() != u32::from(bytes) || !canonical(address))
        }
        Mir65816Op::Store { address, .. } | Mir65816Op::AddressOf { address, .. } => {
            refers(address)
        }
        Mir65816Op::Copy {
            source,
            destination,
            ..
        } => refers(source) || refers(destination),
        _ => false,
    }) {
        return Ok(None);
    }
    let offset = frame.incoming_home(r, id)?;
    abi::stack::access_displacement(
        ByteOffset::new(offset),
        ByteSize::new(bytes.into()),
        ByteSize::ZERO,
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(Source {
        kind: SourceKind::Parameter(id),
        home: Slot {
            offset: offset as u16,
            width: bytes,
        },
    }))
}

fn operand(
    r: &Mir65816Routine,
    frame: &AllocatedFrame,
    value: &Mir65816Value,
    arithmetic: bool,
) -> bool {
    match value {
        Mir65816Value::U32(_) => true,
        Mir65816Value::U8(_) | Mir65816Value::U16(_) | Mir65816Value::U24(_) => arithmetic,
        Mir65816Value::Temp(id, w) => {
            w.get() == 4 && matches!(frame.temps.get(id), Some(Location::Stack(s)) if s.width == 4)
        }
        Mir65816Value::Param(id) => matches!(frame.parameter_home(r, *id), Ok((_, 4))),
        _ => false,
    }
}

fn local(
    r: &Mir65816Routine,
    address: &Mir65816Address,
    bytes: u8,
) -> Result<Option<Source>, String> {
    let Mir65816AddressBase::AutomaticFrame(id) = address.base else {
        return Ok(None);
    };
    if !canonical(address) {
        return Ok(None);
    }
    let object = r
        .frame
        .objects
        .iter()
        .find(|o| o.id == id)
        .ok_or("unknown scalar source object")?;
    if object.addressable
        || object.size.get() != u32::from(bytes)
        || !matches!(object.owner, Mir65816FrameObjectOwner::Local(_))
        || r.frame
            .parameters
            .iter()
            .any(|p| p.frame_object == Some(id))
    {
        return Ok(None);
    }
    let refers = |a: &Mir65816Address| a.base == address.base;
    if r.blocks.iter().flat_map(|b| &b.ops).any(|op| match op {
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
        } => {
            refers(address) && (*volatile || width.get() != u32::from(bytes) || !canonical(address))
        }
        Mir65816Op::AddressOf { address, .. } => refers(address),
        Mir65816Op::Copy {
            source,
            destination,
            ..
        } => refers(source) || refers(destination),
        _ => false,
    }) {
        return Ok(None);
    }
    let offset = object.stack_offset;
    let end = u64::from(offset.get()) + u64::from(bytes);
    if offset.get() == 0 || end > u64::from(r.frame.extent.get()) + 1 {
        return Err("scalar source exceeds automatic frame extent".into());
    }
    if r.frame.objects.iter().any(|o| {
        o.id != id
            && u64::from(o.stack_offset.get()) < end
            && u64::from(offset.get()) < u64::from(o.stack_offset.get()) + u64::from(o.size.get())
    }) {
        return Err("scalar source overlaps another frame object".into());
    }
    abi::stack::access_displacement(offset, ByteSize::new(bytes.into()), ByteSize::ZERO)
        .map_err(|e| e.to_string())?;
    Ok(Some(Source {
        kind: SourceKind::FrameObject(id),
        home: Slot {
            offset: offset.get() as u16,
            width: bytes,
        },
    }))
}

fn supported(r: &Mir65816Routine, frame: &AllocatedFrame, op: &Mir65816Op) -> bool {
    match op {
        Mir65816Op::Compare {
            width, left, right, ..
        } => width.get() == 4 && operand(r, frame, left, false) && operand(r, frame, right, false),
        Mir65816Op::Binary {
            width,
            operation,
            left,
            right,
            ..
        } => {
            width.get() == 4
                && matches!(
                    operation,
                    NirBinaryOp::Add
                        | NirBinaryOp::Sub
                        | NirBinaryOp::And
                        | NirBinaryOp::Or
                        | NirBinaryOp::Xor
                )
                && operand(r, frame, left, true)
                && operand(r, frame, right, true)
        }
        Mir65816Op::Call {
            target: Mir65816CallTarget::Direct(_),
            ..
        } => true,
        Mir65816Op::Store {
            width,
            volatile: false,
            ..
        } => width.get() == 4,
        _ => false,
    }
}

fn terminal(
    r: &Mir65816Routine,
    frame: &AllocatedFrame,
    op: &Mir65816Op,
    temp: TempId,
    source: Source,
) -> bool {
    let bytes = u32::from(source.home.width);
    if let Mir65816Op::Call {
        target: Mir65816CallTarget::Direct(_),
        args,
        plan,
        ..
    } = op
    {
        // Full reservation is the worst displacement, including when the
        // selected schedule builds the area incrementally with pushes.
        return args.len() == plan.arguments.len()
            && effects::CallContract::from_plan(plan, abi::FarTransfer::Jsl).is_ok()
            && outgoing_padding(&plan.arguments, plan.outgoing_bytes).is_ok()
            && abi::stack::access_displacement(
                ByteOffset::new(source.home.offset.into()),
                ByteSize::new(bytes), plan.outgoing_bytes,
            ).is_ok()
            && args.iter().zip(&plan.arguments).all(|(value, home)| {
                !matches!(value, Mir65816Value::Temp(id, _) if *id == temp)
                    || matches!((value, home), (Mir65816Value::Temp(_, w), Mir65816AbiHome::StackArgument { size, .. }) if w.get() == bytes && size.get() == bytes)
            });
    }
    let Mir65816Op::Store {
        address,
        value,
        width,
        volatile: false,
    } = op
    else {
        return bytes == 4 && supported(r, frame, op);
    };
    if bytes != 4
        || width.get() != bytes
        || !matches!(value, Mir65816Value::Temp(id, w) if *id == temp && w.get() == bytes)
        || address.displacement.get() > u16::MAX.into()
    {
        return false;
    }
    // Private, nonescaping invocation storage cannot alias linked memory or an
    // indirect source-language object. Keep normal exact-width store selection.
    let (start, extent) = match address.base {
        Mir65816AddressBase::AutomaticFrame(id) => {
            let Some(object) = r.frame.objects.iter().find(|o| o.id == id) else {
                return false;
            };
            if !object.mutable
                || object.stack_offset.get() == 0
                || u64::from(object.stack_offset.get()) + u64::from(object.size.get())
                    > u64::from(r.frame.extent.get()) + 1
            {
                return false;
            }
            (object.stack_offset.get(), object.size.get())
        }
        Mir65816AddressBase::Parameter(id) => {
            if !r
                .frame
                .parameters
                .iter()
                .any(|p| p.param == id && p.frame_object.is_some())
            {
                return false;
            }
            let Ok((offset, width)) = frame.parameter_home(r, id) else {
                return false;
            };
            (offset, u32::from(width))
        }
        Mir65816AddressBase::Static(NirStorageId::Global(_))
        | Mir65816AddressBase::External(_)
        | Mir65816AddressBase::Indirect(_) => return true,
        _ => return false,
    };
    let Some(offset) = start.checked_add(address.displacement.get()) else {
        return false;
    };
    address.index.is_none()
        && u64::from(address.displacement.get()) + u64::from(bytes) <= u64::from(extent)
        && offset.abs_diff(u32::from(source.home.offset)) >= bytes
        && abi::stack::access_displacement(
            ByteOffset::new(offset),
            ByteSize::new(bytes),
            ByteSize::ZERO,
        )
        .is_ok()
}

fn owned_source(
    r: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    bytes: u8,
) -> Result<Option<Source>, String> {
    match address.base {
        Mir65816AddressBase::Parameter(_) => incoming(r, frame, address, bytes),
        Mir65816AddressBase::AutomaticFrame(_) => local(r, address, bytes),
        _ => Ok(None),
    }
}

fn private_interval(r: &Mir65816Routine, frame: &AllocatedFrame, op: &Mir65816Op) -> bool {
    match op {
        Mir65816Op::Load {
            address,
            width,
            volatile: false,
            ..
        } => {
            (1..=4).contains(&width.get())
                && owned_source(r, frame, address, width.get() as u8)
                    .ok()
                    .flatten()
                    .is_some()
        }
        Mir65816Op::Cast { .. }
        | Mir65816Op::Unary { .. }
        | Mir65816Op::Compare { .. }
        | Mir65816Op::AddressOf { .. }
        | Mir65816Op::PointerOffset { .. } => true,
        Mir65816Op::Binary { operation, .. } => matches!(
            operation,
            NirBinaryOp::Add
                | NirBinaryOp::Sub
                | NirBinaryOp::And
                | NirBinaryOp::Or
                | NirBinaryOp::Xor
        ),
        _ => false,
    }
}

impl Plan {
    pub(in crate::mir65816::emit) fn temps(&self) -> impl Iterator<Item = TempId> + '_ {
        self.bindings.iter().map(|b| b.temp)
    }

    /// Logical admission and final source geometry use the same canonical owner
    /// checks. No storage read is deferred through a call, write or external read.
    pub(in crate::mir65816::emit) fn call_inputs(
        r: &Mir65816Routine,
        frame: &AllocatedFrame,
        eligible: &BTreeSet<TempId>,
        pointers: &pointer_forwarding::Plan,
    ) -> Result<Self, String> {
        if eligible.is_empty() {
            return Ok(Self::default());
        }
        let counts = liveness::input_counts(r);
        let definitions = liveness::definition_counts(r);
        let mut plan = Self::default();
        for block in &r.blocks {
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
                let bytes = width.get() as u8;
                if !eligible.contains(dest)
                    || !matches!(width.get(), 1 | 2)
                    || counts.get(dest) != Some(&1)
                    || definitions.get(dest) != Some(&1)
                    || !r.temps.iter().any(|(id, ty)| {
                        id == dest
                            && !ty.pointer
                            && ty
                                .kind
                                .integer()
                                .is_some_and(|i| u32::from(i.bits) == width.get() * 8)
                    })
                {
                    continue;
                }
                let Some((consumer, call)) = block
                    .ops
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, op)| liveness::operation_inputs(op).contains(dest))
                else {
                    continue;
                };
                if consumer - index > 16
                    || !matches!(
                        call,
                        Mir65816Op::Call {
                            target: Mir65816CallTarget::Direct(_),
                            ..
                        }
                    )
                    || block.ops[index + 1..consumer]
                        .iter()
                        .any(|op| !private_interval(r, frame, op))
                {
                    continue;
                }
                let Some(source) = owned_source(r, frame, address, bytes)? else {
                    continue;
                };
                if !terminal(r, frame, call, *dest, source) {
                    continue;
                }
                plan.bindings.push(Binding {
                    temp: *dest,
                    source,
                    definition: (block.id, index),
                    consumer,
                });
            }
        }
        plan.check_calls(r, frame, pointers)?;
        Ok(plan)
    }

    fn check_calls(
        &self,
        r: &Mir65816Routine,
        frame: &AllocatedFrame,
        pointers: &pointer_forwarding::Plan,
    ) -> Result<(), String> {
        let calls: BTreeSet<_> = self
            .bindings
            .iter()
            .map(|b| (b.definition.0, b.consumer))
            .collect();
        // Include existing captured wide read bindings in the same preflight.
        let ordinary = Self::new(r, frame)?;
        for (block, index) in calls {
            let op = &r
                .blocks
                .iter()
                .find(|b| b.id == block)
                .ok_or("missing scalar call block")?
                .ops[index];
            let Mir65816Op::Call {
                target, args, plan, ..
            } = op
            else {
                return Err("missing scalar terminal call".into());
            };
            let mut b = Builder {
                stack_checks: true,
                routine: r,
                frame: frame.clone(),
                code: TrackedEmitter65816::for_entry(r.prologue.required_mode),
                blocks: BTreeMap::new(),
                next_block: None,
                loop_x: None,
                borrowed: BTreeMap::new(),
                scalar_borrowed: BTreeMap::new(),
                resident: BTreeMap::new(),
            };
            pointers.enter(&mut b, block, index);
            for binding in ordinary.bindings.iter().chain(&self.bindings) {
                if binding.definition.0 == block && binding.consumer == index {
                    b.scalar_borrowed.insert(binding.temp, binding.source);
                }
            }
            let padding = outgoing_padding(&plan.arguments, plan.outgoing_bytes)?;
            let arguments = b.call_arguments_with_a(
                args,
                plan,
                target,
                (!padding.is_empty()).then_some(false),
                None,
            )?;
            // Full reservation preflight bounds every intermediate push depth,
            // including an exact-width tail. The same complete typed schedule
            // selector and store fallback are used during actual emission.
            let outgoing =
                u16::try_from(plan.outgoing_bytes.get()).map_err(|_| "scalar outgoing extent")?;
            let _ = call_copies::pushes::Plan::new(&arguments, args, &padding, outgoing);
        }
        Ok(())
    }

    pub(in crate::mir65816::emit) fn resolve_inputs(
        &self,
        r: &Mir65816Routine,
        frame: &AllocatedFrame,
        pointers: &pointer_forwarding::Plan,
    ) -> Result<Self, String> {
        let eligible = self.temps().collect();
        let resolved = Self::call_inputs(r, frame, &eligible, pointers)?;
        if self.bindings.len() != resolved.bindings.len()
            || self.bindings.iter().any(|b| {
                !resolved.bindings.iter().any(|a| {
                    a.temp == b.temp
                        && a.definition == b.definition
                        && a.consumer == b.consumer
                        && a.source.kind == b.source.kind
                })
            })
        {
            return Err("borrowed scalar demand no longer valid in final frame".into());
        }
        Ok(resolved)
    }

    pub(in crate::mir65816::emit) fn with_inputs(
        r: &Mir65816Routine,
        frame: &AllocatedFrame,
        inputs: &Self,
        pointers: &pointer_forwarding::Plan,
    ) -> Result<Self, String> {
        let mut plan = Self::new(r, frame)?;
        let inputs = inputs.resolve_inputs(r, frame, pointers)?;
        plan.bindings.extend(inputs.bindings);
        Ok(plan)
    }
    pub(in crate::mir65816::emit) fn placement_bindings(
        &self,
    ) -> Vec<super::super::placement::BorrowedInput> {
        self.bindings
            .iter()
            .map(|b| super::super::placement::BorrowedInput {
                temp: b.temp,
                definition: crate::mir65816::analysis::ProgramPoint {
                    block: b.definition.0,
                    index: b.definition.1,
                },
                uses: vec![crate::mir65816::analysis::ProgramPoint {
                    block: b.definition.0,
                    index: b.consumer,
                }],
                source: match b.source.kind {
                    SourceKind::Parameter(id) => super::super::placement::InputOwner::Parameter(id),
                    SourceKind::FrameObject(id) => super::super::placement::InputOwner::Frame(id),
                },
                home: b.source.home,
            })
            .collect()
    }

    pub(in crate::mir65816::emit) fn new(
        r: &Mir65816Routine,
        frame: &AllocatedFrame,
    ) -> Result<Self, String> {
        let counts = liveness::input_counts(r);
        let definitions = liveness::definition_counts(r);
        let mut plan = Self::default();
        for block in &r.blocks {
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
                if !matches!(width.get(), 1 | 2 | 4)
                    || counts.get(dest) != Some(&1)
                    || definitions.get(dest) != Some(&1)
                    || !r.temps.iter().any(|(id, ty)| {
                        id == dest
                            && !ty.pointer
                            && ty
                                .kind
                                .integer()
                                .is_some_and(|i| u32::from(i.bits) == width.get() * 8)
                    })
                {
                    continue;
                }
                // Omitted accumulator homes and closed DP/X allocations retain
                // their existing producers/consumers and fact witnesses.
                let Some(Location::Stack(capture)) = frame.temps.get(dest) else {
                    continue;
                };
                if u32::from(capture.width) != width.get() {
                    return Err("scalar capture width mismatch".into());
                }
                let Some(consumer) = block.ops.get(index + 1) else {
                    continue;
                };
                if liveness::operation_inputs(consumer)
                    .iter()
                    .filter(|id| *id == dest)
                    .count()
                    != 1
                    || !supported(r, frame, consumer)
                    // This selector reads the original capture at the later
                    // comparison after omitting AND. Keep its proof ownership.
                    || top_bits::owns_mask(block, index + 1, &counts)
                {
                    continue;
                }
                let Some(source) = (match address.base {
                    Mir65816AddressBase::Parameter(_) => {
                        incoming(r, frame, address, capture.width)?
                    }
                    Mir65816AddressBase::AutomaticFrame(_) => local(r, address, capture.width)?,
                    _ => None,
                }) else {
                    continue;
                };
                if !terminal(r, frame, consumer, *dest, source) {
                    continue;
                }
                if frame
                    .temps
                    .values()
                    .any(|home| home.overlaps(Location::Stack(source.home)))
                {
                    return Err("temporary overlaps authoritative scalar source".into());
                }
                plan.bindings.push(Binding {
                    temp: *dest,
                    source,
                    definition: (block.id, index),
                    consumer: index + 1,
                });
            }
        }
        Ok(plan)
    }

    #[cfg(feature = "native65816-state-proof")]
    pub(in crate::mir65816::emit) fn observations(&self) -> Vec<super::super::proof::ScalarRead> {
        self.bindings
            .iter()
            .map(|b| super::super::proof::ScalarRead {
                temp: b.temp,
                block: b.definition.0,
                producer: b.definition.1,
                consumer: b.consumer,
                source: match b.source.kind {
                    SourceKind::Parameter(id) => super::super::proof::HomeOwner::Incoming(id),
                    SourceKind::FrameObject(id) => super::super::proof::HomeOwner::FrameObject(id),
                },
                offset: b.source.home.offset,
                bytes: b.source.home.width,
            })
            .collect()
    }

    pub(super) fn enter(&self, b: &mut Builder<'_>, block: BlockId, index: usize) -> bool {
        b.scalar_borrowed.clear();
        for binding in &self.bindings {
            if binding.definition.0 == block && binding.consumer == index {
                match binding.source.kind {
                    SourceKind::Parameter(id) => debug_assert_eq!(
                        b.frame.incoming_home(b.routine, id).unwrap(),
                        u32::from(binding.source.home.offset)
                    ),
                    SourceKind::FrameObject(id) => debug_assert_eq!(
                        b.object(id).unwrap(),
                        u32::from(binding.source.home.offset)
                    ),
                }
                b.scalar_borrowed.insert(binding.temp, binding.source);
            }
        }
        let omitted = self.bindings.iter().any(|b| b.definition == (block, index));
        if omitted {
            b.code.barrier();
        }
        omitted
    }
}

/// Commit an entire family's demand only when final allocation and every call
/// remain valid. A refused preview leaves all preceding owners untouched.
pub(in crate::mir65816::emit) fn admit_inputs(r: &Mir65816Routine, demand: &mut home_demand::Plan) {
    let terminal_temps: BTreeSet<_> = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .flat_map(|op| {
            if let Mir65816Op::Call {
                target: Mir65816CallTarget::Direct(_),
                args,
                ..
            } = op
            {
                args.iter()
                    .filter_map(|v| {
                        if let Mir65816Value::Temp(id, w) = v {
                            matches!(w.get(), 1 | 2).then_some(*id)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        })
        .collect();
    let eligible: BTreeSet<_> = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter_map(|op| {
            if let Mir65816Op::Load {
                dest,
                width,
                volatile: false,
                address,
            } = op
                && matches!(width.get(), 1 | 2)
                && !demand.omits(*dest)
                && terminal_temps.contains(dest)
                && canonical(address)
                && matches!(
                    address.base,
                    Mir65816AddressBase::Parameter(_) | Mir65816AddressBase::AutomaticFrame(_)
                )
            {
                Some(*dest)
            } else {
                None
            }
        })
        .collect();
    if eligible.is_empty()
        || !r.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            matches!(
                op,
                Mir65816Op::Call {
                    target: Mir65816CallTarget::Direct(_),
                    ..
                }
            )
        })
    {
        return;
    }
    let Ok(before) = AllocatedFrame::with_demand(r, demand) else {
        return;
    };
    let Ok(pointers) = demand.pointers.resolve(r, &before) else {
        return;
    };
    let Ok(inputs) = Plan::call_inputs(r, &before, &eligible, &pointers) else {
        return;
    };
    if inputs.bindings.is_empty() {
        return;
    }
    let previous: Vec<_> = inputs
        .temps()
        .map(|id| {
            (
                id,
                demand.decisions.insert(id, home_demand::Decision::Borrowed),
            )
        })
        .collect();
    let old_mixed = demand.mixed.clone();
    demand.mixed = mixed::Plan::new(r, demand);
    let accepted = AllocatedFrame::with_demand(r, demand).and_then(|after| {
        if after.extent > before.extent
            || after.spill_bytes > before.spill_bytes
            || after.peak_below_entry > before.peak_below_entry
        {
            return Err("scalar input resource growth".into());
        }
        let pointers = demand.pointers.resolve(r, &after)?;
        inputs.resolve_inputs(r, &after, &pointers)
    });
    match accepted {
        Ok(inputs) => demand.scalar_inputs = inputs,
        Err(_) => {
            demand.mixed = old_mixed;
            for (id, decision) in previous {
                if let Some(decision) = decision {
                    demand.decisions.insert(id, decision);
                } else {
                    demand.decisions.remove(&id);
                }
            }
        }
    }
}
