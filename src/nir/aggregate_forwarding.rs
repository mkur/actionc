//! Same-block snapshot read forwarding. This does not redirect producers or
//! relax the whole-capture call/return ABI. Unknown proofs retain the snapshot.

use super::analysis::storage_references::{op_references, terminator_references};
use super::facts::root_storage_id;
use super::*;

struct Forwarding {
    routine: usize,
    block: usize,
    copy: usize,
    capture: NirStorageId,
    source: NirPlace,
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
                find_forwarding(index, routine, analysis.routine(routine.id).unwrap())
            });
        let Some(next) = next else { break };
        let routine = &mut forwarded.routines[next.routine];
        for (index, op) in routine.blocks[next.block].ops.iter_mut().enumerate() {
            if index == next.copy {
                continue;
            }
            match op {
                NirOp::Load { place, .. } | NirOp::AddrOf { place, .. } => {
                    redirect_root(place, next.capture, &next.source);
                }
                NirOp::CopyBytes { source, .. } => {
                    redirect_root(source, next.capture, &next.source);
                }
                _ => {}
            }
        }
        routine.blocks[next.block].ops.remove(next.copy);
        routine.temps = super::optimizer::collect_temps(&routine.blocks);
    }
    verify_program(&forwarded)?;
    Ok(forwarded)
}

fn find_forwarding(
    routine_index: usize,
    routine: &NirRoutine,
    facts: &NirRoutineAggregateRegions<'_>,
) -> Option<Forwarding> {
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
            let Some(source_id) = direct_storage_id(source) else {
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
            if source_facts.width != Some(*size)
                || source_facts.machine_visible
                || facts.relation(destination, *size, source, *size, at)
                    != Ok(NirRegionRelation::Disjoint)
                || !snapshot_uses_are_forwardable(routine, facts, destination, source, *size, at)
            {
                continue;
            }
            return Some(Forwarding {
                routine: routine_index,
                block: block_index,
                copy,
                capture,
                source: source.clone(),
            });
        }
    }
    None
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
    routine: &NirRoutine,
    facts: &NirRoutineAggregateRegions<'_>,
    destination: &NirPlace,
    source: &NirPlace,
    size: ByteSize,
    initialized: NirAggregatePoint,
) -> bool {
    let capture = direct_storage_id(destination).unwrap();
    let mut last_read = initialized;
    for block in &routine.blocks {
        // Aggregate values at call/return boundaries must remain complete
        // caller-owned captures, even when byte equality would be provable.
        if terminator_references(&block.terminator).contains(&capture) {
            return false;
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
                _ => (false, false),
            };
            // address_use already proved that indirect users are bounded
            // internal memory consumers. Inspect their resolved regions above
            // as well as every direct syntactic/ABI/effect reference here.
            if explicit && !read && !address {
                return false;
            }
            if read || address {
                if at.block != initialized.block || at.op_index <= initialized.op_index {
                    return false;
                }
                if read {
                    last_read = at;
                }
            }
        }
    }
    if last_read == initialized {
        return true;
    }
    let after_copy = NirAggregatePoint {
        op_index: initialized.op_index + 1,
        ..initialized
    };
    // The single complete initializing write precedes every consumer each time
    // this block executes (including repeated loop entries). Neither image may
    // change until the last redirected read. Calls after that read are irrelevant.
    facts
        .unchanged_between(source, size, after_copy, last_read)
        .is_ok()
        && facts
            .unchanged_between(destination, size, after_copy, last_read)
            .is_ok()
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
