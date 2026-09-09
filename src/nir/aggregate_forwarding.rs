//! Path-proven snapshot forwarding, including complete private ABI captures.
//! Unknown proofs retain staging; the whole-capture ABI is never relaxed.

use super::analysis::aggregate_lifetime::AggregateLifetime;
use super::analysis::storage_references::{op_references, terminator_references};
use super::facts::root_storage_id;
use super::*;

struct Forwarding {
    routine: usize,
    block: usize,
    copy: usize,
    capture: NirStorageId,
    source: NirPlace,
    producer: Option<(usize, usize)>,
}

pub(super) fn forward_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let mut forwarded = program.clone();
    loop {
        // Each rewrite removes a CopyBytes. Refresh immutable facts before
        // considering another link in a chain; never reuse stale alias facts.
        let analysis = analyze_aggregate_regions(&forwarded)?;
        let next = forwarded
            .routines
            .iter()
            .enumerate()
            .find_map(|(index, routine)| {
                find_forwarding(
                    &forwarded,
                    index,
                    routine,
                    analysis.routine(routine.id).unwrap(),
                )
            });
        let Some(next) = next else { break };
        let routine = &mut forwarded.routines[next.routine];
        if let Some((block, op)) = next.producer {
            // A single-use automatic result buffer can be produced directly
            // into the fresh final home. No observable write is advanced.
            let NirOp::CopyBytes { destination, .. } = &routine.blocks[next.block].ops[next.copy]
            else {
                unreachable!()
            };
            let destination = destination.clone();
            let NirOp::Call {
                aggregate_result, ..
            } = &mut routine.blocks[block].ops[op]
            else {
                unreachable!()
            };
            *aggregate_result = Some(destination);
            routine.blocks[next.block].ops.remove(next.copy);
            routine.temps = super::optimizer::collect_temps(&routine.blocks);
            continue;
        }
        for (block_index, block) in routine.blocks.iter_mut().enumerate() {
            for (index, op) in block.ops.iter_mut().enumerate() {
                if block_index == next.block && index == next.copy {
                    continue;
                }
                match op {
                    NirOp::Load { place, .. } | NirOp::AddrOf { place, .. } => {
                        redirect_root(place, next.capture, &next.source);
                    }
                    NirOp::CopyBytes { source, .. } => {
                        redirect_root(source, next.capture, &next.source);
                    }
                    NirOp::Call { args, .. } => {
                        for arg in args {
                            if let NirValue::Aggregate { place } = arg {
                                redirect_root(place, next.capture, &next.source);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let NirTerminator::Return(Some(NirValue::Aggregate { place })) =
                &mut block.terminator
            {
                redirect_root(place, next.capture, &next.source);
            }
        }
        routine.blocks[next.block].ops.remove(next.copy);
        routine.temps = super::optimizer::collect_temps(&routine.blocks);
    }
    verify_program(&forwarded)?;
    Ok(forwarded)
}

fn find_forwarding(
    program: &NirProgram,
    routine_index: usize,
    routine: &NirRoutine,
    facts: &NirRoutineAggregateRegions<'_>,
) -> Option<Forwarding> {
    let mut fallback = None;
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for (copy, op) in block.ops.iter().enumerate() {
            let NirOp::CopyBytes {
                destination,
                source,
                size,
                destination_volatile: false,
                source_volatile: false,
            } = op
            else {
                continue;
            };
            let Some(NirStorageId::Local(id)) = direct_storage_id(destination) else {
                continue;
            };
            let Some(local) = routine.locals.iter().find(|local| local.id == id) else {
                continue;
            };
            let capture = NirStorageId::Local(id);
            // Fixed direct roots/fields can be reused without reevaluating an
            // address expression. Loaded pointers and dynamic subobjects need
            // an additional address-lifetime proof and remain staged.
            let Some(source_id) = root_storage_id(source) else {
                continue;
            };
            if local.purpose != NirLocalPurpose::AggregateCapture
                || local.backing != NirLocalBacking::Ordinary
                || local.init.is_some()
                || local.layout.size != *size
                || !same_complete_type(source.ty.as_ref(), Some(&local.ty), *size)
                || facts.address_use(capture) == NirAggregateAddressUse::ExposedOrUnknown
            {
                continue;
            }
            let at = NirAggregatePoint {
                block: block.id,
                op_index: copy,
            };
            // Exact, complete, ordinary homes only. In particular, equal byte
            // sizes or different unresolved pointers do not prove eligibility.
            let Some(source_facts) = facts.storage().homes.get(&source_id) else {
                continue;
            };
            if source_facts.machine_visible
                || facts.relation(destination, *size, source, *size, at)
                    != Ok(NirRegionRelation::Disjoint)
                || !snapshot_uses_are_forwardable(
                    program,
                    routine,
                    facts,
                    destination,
                    source,
                    *size,
                    at,
                )
            {
                continue;
            }
            let forwarding = Forwarding {
                routine: routine_index,
                block: block_index,
                copy,
                capture,
                source: source.clone(),
                producer: fresh_result_producer(program, routine, facts, source, *size, at),
            };
            // First reuse backing that can itself cross the whole-value ABI.
            // Removing it into a global/field first can strand several argument
            // copies that could all share this one private capture.
            if whole_capture(routine, source, *size) {
                return Some(forwarding);
            }
            if fallback.is_none() {
                fallback = Some(forwarding);
            }
        }
    }
    fallback
}

fn same_complete_type(left: Option<&NirType>, right: Option<&NirType>, size: ByteSize) -> bool {
    matches!(
        (left.map(|ty| &ty.kind), right.map(|ty| &ty.kind)),
        (Some(NirTypeKind::Record { definition: Some(a), size: Some(x), .. }),
         Some(NirTypeKind::Record { definition: Some(b), size: Some(y), .. }))
            if a == b && *x == size && *y == size
    )
}

fn snapshot_uses_are_forwardable(
    program: &NirProgram,
    routine: &NirRoutine,
    facts: &NirRoutineAggregateRegions<'_>,
    destination: &NirPlace,
    source: &NirPlace,
    size: ByteSize,
    initialized: NirAggregatePoint,
) -> bool {
    let capture = direct_storage_id(destination).unwrap();
    let lifetime =
        AggregateLifetime::analyze(routine, facts, destination, source, size, initialized);
    for block in &routine.blocks {
        if terminator_references(&block.terminator).contains(&capture) {
            if !matches!(&block.terminator,
                NirTerminator::Return(Some(NirValue::Aggregate { place }))
                if direct_storage_id(place) == Some(capture))
                || !whole_capture(routine, source, size)
                || !lifetime.at(NirAggregatePoint {
                    block: block.id,
                    op_index: block.ops.len(),
                })
            {
                return false;
            }
        }
        for (op_index, op) in block.ops.iter().enumerate() {
            let at = NirAggregatePoint {
                block: block.id,
                op_index,
            };
            if at == initialized {
                continue;
            }
            let explicit = op_references(op).contains(&capture);
            let is_capture = |place: &NirPlace, width: ByteSize| {
                facts
                    .region(place, width, at)
                    .is_ok_and(|region| region.memory.kind == NirMemoryRegionKind::Storage(capture))
            };
            let (read, address) = match op {
                NirOp::Load { place, ty, .. } => (
                    ty.width.is_some_and(|width| is_capture(place, width)),
                    false,
                ),
                NirOp::CopyBytes {
                    destination,
                    source: read_source,
                    size: width,
                    destination_volatile: false,
                    source_volatile: false,
                } => {
                    if is_capture(destination, *width) {
                        return false;
                    }
                    let read = is_capture(read_source, *width);
                    // Keep uncertain/partial-overlap consumers staged. Copies
                    // to a known whole source itself also remain conservative.
                    if read
                        && facts.relation(destination, *width, source, size, at)
                            != Ok(NirRegionRelation::Disjoint)
                    {
                        return false;
                    }
                    (read, false)
                }
                NirOp::AddrOf { place, .. } => (false, is_capture(place, ByteSize::ONE)),
                NirOp::Store { place, ty, .. } => {
                    if ty.width.is_some_and(|width| is_capture(place, width)) {
                        return false;
                    }
                    (false, false)
                }
                NirOp::Call {
                    callee,
                    args,
                    aggregate_result,
                    ..
                } if explicit => {
                    // Defined callees copy all incoming aggregate images into
                    // distinct mutable parameter homes before executing their
                    // body. Unknown/indirect entry contracts remain staged.
                    if !defined_callee(program, callee)
                        || !whole_capture(routine, source, size)
                        || aggregate_result.as_ref().is_some_and(|result| {
                            facts.relation(
                                result,
                                result.ty.as_ref().unwrap().width.unwrap(),
                                source,
                                size,
                                at,
                            ) != Ok(NirRegionRelation::Disjoint)
                        })
                    {
                        return false;
                    }
                    let reads_capture = |arg: &NirValue| {
                        matches!(arg,
                        NirValue::Aggregate { place } if direct_storage_id(place) == Some(capture))
                    };
                    let read = args.iter().any(reads_capture);
                    // Do not hide an effect, result or other reference behind
                    // an otherwise eligible argument use.
                    let mut remaining = op.clone();
                    if let NirOp::Call { args, .. } = &mut remaining {
                        args.retain(|arg| !reads_capture(arg));
                    }
                    if op_references(&remaining).contains(&capture) {
                        return false;
                    }
                    (read, false)
                }
                _ => (false, false),
            };
            // address_use already proved that indirect users are bounded
            // internal memory consumers. Inspect their resolved regions above
            // as well as every direct syntactic/ABI/effect reference here.
            if explicit && !read && !address {
                return false;
            }
            if read || address {
                if !lifetime.at(at) {
                    return false;
                }
            }
        }
    }
    true
}

fn whole_capture(routine: &NirRoutine, place: &NirPlace, size: ByteSize) -> bool {
    let Some(NirStorageId::Local(id)) = direct_storage_id(place) else {
        return false;
    };
    routine.locals.iter().any(|local| {
        local.id == id
            && local.purpose == NirLocalPurpose::AggregateCapture
            && local.backing == NirLocalBacking::Ordinary
            && local.init.is_none()
            && local.layout.size == size
            && same_complete_type(place.ty.as_ref(), Some(&local.ty), size)
    })
}

fn defined_callee(program: &NirProgram, callee: &NirCallee) -> bool {
    matches!(callee, NirCallee::User { id, .. } if program.routines.iter()
        .any(|routine| routine.id == *id && !routine.entry.external))
}

// Distinct producer/publication proof, intentionally narrower than read reuse:
// a whole automatic buffer has one producer and just this copy as its consumer.
// Static reentry and validation/address consumers need stronger ownership proofs.
fn fresh_result_producer(
    program: &NirProgram,
    routine: &NirRoutine,
    facts: &NirRoutineAggregateRegions<'_>,
    source: &NirPlace,
    size: ByteSize,
    copied: NirAggregatePoint,
) -> Option<(usize, usize)> {
    if routine.activation != NirActivationModel::NativeReentrant
        || !whole_capture(routine, source, size)
    {
        return None;
    }
    let id = direct_storage_id(source)?;
    if facts.address_use(id) == NirAggregateAddressUse::ExposedOrUnknown {
        return None;
    }
    let mut producer = None;
    for (block_index, block) in routine.blocks.iter().enumerate() {
        if terminator_references(&block.terminator).contains(&id) {
            return None;
        }
        for (op_index, op) in block.ops.iter().enumerate() {
            let at = NirAggregatePoint {
                block: block.id,
                op_index,
            };
            if at == copied || !op_references(op).contains(&id) {
                continue;
            }
            let NirOp::Call {
                callee,
                aggregate_result: Some(result),
                ..
            } = op
            else {
                return None;
            };
            if direct_storage_id(result) != Some(id)
                || !defined_callee(program, callee)
                || producer.is_some()
            {
                return None;
            }
            let mut remaining = op.clone();
            if let NirOp::Call {
                aggregate_result, ..
            } = &mut remaining
            {
                *aggregate_result = None;
            }
            if op_references(&remaining).contains(&id)
                || !AggregateLifetime::analyze(routine, facts, source, source, size, at).at(copied)
            {
                return None;
            }
            producer = Some((block_index, op_index));
        }
    }
    producer
}

fn redirect_root(place: &mut NirPlace, capture: NirStorageId, source: &NirPlace) {
    if direct_storage_id(place) == Some(capture) {
        *place = source.clone();
    } else if root_storage_id(place) == Some(capture)
        && let NirPlaceKind::Field { base, .. } = &mut place.kind
    {
        redirect_root(base, capture, source);
    }
}
