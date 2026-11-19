use super::*;

/// Collect explicit destinations independently of complete exit summaries.
/// Even a routine containing another, opaque operation can contain an explicit
/// address that a relocation must preserve. Each formal is visited once per
/// routine; concrete addresses stay local to the invocation that supplied them.
pub(in crate::mir6502) fn native_numeric_observations(
    program: &MirProgram,
) -> BTreeSet<(u16, u16)> {
    let known = MirKnownCalleeSummaries {
        fixed_storage: Arc::new(MirFixedStorage::new(program)),
        ..Default::default()
    };
    let mut ranges = BTreeSet::new();
    let mut pending = Vec::new();
    let mut seen = BTreeSet::new();
    let mut calls = BTreeMap::<_, Vec<_>>::new();
    let mut trackers = BTreeMap::new();
    for routine in &program.routines {
        if routine
            .frame
            .callee_review
            .as_ref()
            .is_some_and(|r| !r.facts.buffers.is_empty())
            && let Some(facts) = super::super::reviewed_contract(program, routine)
            && let Some(buffers) = super::super::reviewed_buffers::buffer_accesses(routine, facts)
        {
            pending.extend(
                buffers
                    .iter()
                    .filter_map(|b| FormalWrite::from_reviewed(b, routine.id))
                    .map(|w| (routine.id, w)),
            );
        }
        for b in &routine.blocks {
            for (op_index, op) in b.ops.iter().enumerate() {
                let site = MirSite::Op {
                    block: b.id,
                    op_index,
                };
                if let MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    ..
                } = op
                {
                    calls.entry(*id).or_default().push((routine, site, op));
                }
                let destination = if let MirOp::Store {
                    dst: MirAddr::Deref { ptr, offset },
                    width,
                    ..
                } = op
                {
                    let write = FormalWrite {
                        argument: 0,
                        offset: *offset,
                        bytes: crate::mir6502::abi::action_arg_width_bytes(*width),
                        carrier: None,
                        count: None, reviewed_copy: None,
                    };
                    retained_base(program, routine, site, ptr, write).map(|base| (base, write))
                } else if destination(op).is_some() {
                    trackers
                        .entry(routine.id)
                        .or_insert_with(|| {
                            crate::mir6502::runtime_contracts::buffers::argument_values(
                                program, routine,
                            )
                        })
                        .as_ref()
                        .and_then(|values| {
                            store_base(
                                TracePurpose::NumericObservation,
                                program,
                                routine,
                                &known,
                                values,
                                site,
                                op,
                            )
                        })
                } else {
                    None
                };
                if let Some((base, write)) = destination {
                    observe(base, write, routine.id, &mut pending, &mut ranges);
                }
            }
        }
    }
    while let Some((id, write)) = pending.pop() {
        if !seen.insert((id, write)) {
            continue;
        }
        for &(routine, site, call) in calls.get(&id).into_iter().flatten() {
            let Some(values) = trackers
                .entry(routine.id)
                .or_insert_with(|| {
                    crate::mir6502::runtime_contracts::buffers::argument_values(program, routine)
                })
                .as_ref()
            else {
                continue;
            };
            let write = write.at_call(program, routine, call, &known, values, site);
            if write.bytes == 0 { continue; }
            if let Some(base) = known.call_base(
                TracePurpose::NumericObservation,
                program,
                routine,
                call,
                values,
                site,
                write,
            ) {
                observe(base, write, routine.id, &mut pending, &mut ranges);
            }
        }
    }
    ranges
}

fn observe(
    base: Base,
    write: FormalWrite,
    owner: RoutineId,
    pending: &mut Vec<(RoutineId, FormalWrite)>,
    ranges: &mut BTreeSet<(u16, u16)>,
) {
    match base {
        Base::Numeric(n) => {
            ranges.insert((n.wrapping_add(write.offset), write.bytes));
        }
        Base::Formal(argument) => pending.push((owner, FormalWrite { argument, ..write })),
        Base::Object => {}
    }
}
