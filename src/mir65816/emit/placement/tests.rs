use super::*;

fn program(source: &str) -> Mir65816Program {
    program_with_optimization(source, false)
}

fn program_with_optimization(source: &str, optimize: bool) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let nir = if optimize {
        crate::nir::optimize_program(&nir).unwrap()
    } else {
        nir
    };
    crate::mir65816::lower_program(&nir).unwrap()
}

fn record() -> Mir65816Program {
    program(
        "TYPE Cell=[BYTE tag CARD amount Cell POINTER next] PROC Work(Cell POINTER p CARD n) p.amount=n p.tag=7 p=p.next n=p.amount+1 RETURN PROC Main() RETURN",
    )
}

#[test]
fn deferred_record_address_components_keep_their_complete_stack_inputs() {
    let mut r = super::super::mixed::tests::routine();
    let pointer_type = r.temps[0].1.clone();
    r.temps.push((TempId(2), pointer_type));
    let mut address = match &r.blocks[0].ops[1] {
        Mir65816Op::Load { address, .. } => address.clone(),
        _ => unreachable!(),
    };
    address.displacement = ByteOffset::new(4);
    let mut destination = address.clone();
    destination.base =
        Mir65816AddressBase::Indirect(Mir65816Value::Param(r.frame.parameters[0].param));
    r.blocks[0].ops.splice(
        1..1,
        [
            Mir65816Op::AddressOf {
                dest: TempId(2),
                address,
                width: ByteSize::new(3),
            },
            Mir65816Op::Store {
                address: destination,
                value: Mir65816Value::Temp(TempId(2), ByteSize::new(3)),
                width: ByteSize::new(3),
                volatile: false,
            },
        ],
    );
    let plan = Plan::new(&r, &[]).unwrap();
    assert!(
        plan.demand
            .producer(r.blocks[0].id, 1)
            .is_some_and(|interval| interval.bytes == 3)
    );
    assert!(matches!(plan.frame.temps[&TempId(0)], Location::Stack(_)));
    plan.verify().unwrap();
    super::super::select::routine(&r, false).unwrap();
}

#[test]
fn current_word_plan_is_complete() {
    let p = super::super::select::word_tests::program();
    for r in &p.routines {
        let plan = Plan::new(r, &p.data).unwrap();
        assert_eq!(plan.contract.values.len(), r.temps.len());
        plan.verify().unwrap();
    }
}

#[test]
fn forged_missing_partial_and_out_of_domain_homes_are_rejected() {
    let p = super::super::select::word_tests::program();
    let r = &p.routines[0];
    for problem in 0..6 {
        let mut plan = Plan::new(r, &p.data).unwrap();
        let id = r.temps[0].0;
        match problem {
            0 => {
                plan.frame.temps.remove(&id);
            }
            1 => {
                plan.frame.temps.insert(
                    id,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 2,
                    }),
                );
            }
            2 => {
                plan.contract.values.get_mut(&id).unwrap().bytes = 1;
            }
            3 => {
                plan.contract.values.get_mut(&id).unwrap().materialized = None;
            }
            4 => {
                plan.contract.values.get_mut(&id).unwrap().reads.clear();
            }
            _ => {
                plan.contract.frame.peak_below_entry += 1;
            }
        }
        assert!(plan.verify().is_err(), "forged home {problem}");
    }
}

#[test]
fn omitted_values_have_one_complete_owner_and_every_use_is_covered() {
    for source in [
        "LONGCARD FUNC Work(CARD a) RETURN(LONGCARD(a+1)) PROC Main() RETURN",
        "TYPE Cell=[BYTE tag CARD n Cell POINTER next] CARD FUNC Work(Cell POINTER p) RETURN(p.n+1) PROC Main() RETURN",
        "PROC Work(BYTE POINTER p) BYTE POINTER local local=p RETURN PROC Main() RETURN",
        "TYPE Cell=[Cell POINTER next] PROC Work(Cell POINTER p,q) p.next=q+1 RETURN PROC Main() RETURN",
    ] {
        let p = program(source);
        for r in &p.routines {
            let mut plan = Plan::new(r, &p.data).unwrap();
            for (&id, value) in &plan.contract.values {
                assert_eq!(
                    value.reads.len(),
                    plan.logical
                        .value(plan.logical.temp(id).unwrap())
                        .unwrap()
                        .uses
                        .len()
                );
                if value.materialized.is_none() {
                    assert!(!matches!(value.capture, Capture::Home(_)));
                }
            }
            if let Some(id) = plan
                .contract
                .values
                .iter()
                .find_map(|(&id, v)| v.materialized.is_none().then_some(id))
            {
                let old = plan.contract.values[&id].clone();
                plan.contract.values.get_mut(&id).unwrap().capture =
                    Capture::Home(Location::Stack(Slot {
                        offset: 2,
                        width: old.bytes,
                    }));
                assert!(plan.verify().is_err());
                plan.contract.values.insert(id, old);
                plan.verify().unwrap();
            }
        }
        materialize(&p).unwrap();
    }
}

#[test]
fn independently_forged_intervals_resources_and_edge_arguments_are_rejected() {
    let p = program("LONGCARD FUNC Work(CARD a) RETURN(LONGCARD(a+1)) PROC Main() RETURN");
    let r = &p.routines[0];
    for problem in 0..5 {
        let mut plan = Plan::new(r, &p.data).unwrap();
        let (&id, _) = plan
            .contract
            .values
            .iter()
            .find(|(_, v)| matches!(v.capture, Capture::Registers { .. }))
            .unwrap();
        match problem {
            0 => {
                if let Capture::Registers { interval, .. } =
                    &mut plan.contract.values.get_mut(&id).unwrap().capture
                {
                    interval.consumer += 1;
                }
            }
            1 => {
                if let Capture::Registers { lanes, .. } =
                    &mut plan.contract.values.get_mut(&id).unwrap().capture
                {
                    *lanes = Lanes::A8;
                }
            }
            2 => {
                let point = *plan.contract.windows.keys().next().unwrap();
                plan.contract.requirements_mut(point).scratch = resources::Scratch::default();
            }
            3 => {
                plan.contract.windows.remove(&ProgramPoint {
                    block: r.blocks[0].id,
                    index: 0,
                });
            }
            _ => {
                plan.demand.decisions.insert(
                    id,
                    home_demand::Decision::Memory(home_demand::MemoryReason::UnsupportedProducer),
                );
            }
        }
        assert!(plan.verify().is_err(), "forged plan {problem}");
    }
    let p = program(
        "CARD FUNC Work(CARD a) IF a THEN a=a+1 ELSE a=a+2 FI RETURN(a) PROC Main() RETURN",
    );
    let mut plan = Plan::new(&p.routines[0], &p.data).unwrap();
    plan.contract.edges.values_mut().next().unwrap().target = BlockId(u32::MAX);
    assert!(plan.verify().is_err());
}

#[test]
fn selected_resource_and_boundary_forgery_cannot_publish() {
    let p = record();
    let machine = materialize(&p).unwrap();
    let selected = machine.routines[0].code.selected.as_ref().unwrap();
    let original = selected.placement().unwrap();
    original.verify_selected(selected).unwrap();
    let other = materialize(&p).unwrap();
    let other_selected = other.routines[0].code.selected.as_ref().unwrap();
    assert_eq!(selected.allocation, other_selected.allocation);
    assert!(
        original
            .verify_selected(other_selected)
            .unwrap_err()
            .contains("foreign or stale")
    );
    for problem in 0..5 {
        let mut contract = original.clone();
        match problem {
            0 => contract.routine = RoutineId(u32::MAX),
            1 => contract.frame.extent += 2,
            2 => {
                contract.windows.clear();
            }
            3 => {
                contract.reachable.clear();
            }
            _ => {
                contract.edges.clear();
            }
        }
        // The record routine is a single block, so forging its empty edge set
        // is a no-op; separately forge its entry label in that case.
        if problem == 4 {
            contract.entry = BlockId(u32::MAX);
        }
        assert!(
            contract.verify_selected(selected).is_err(),
            "contract {problem}"
        );
    }
    let point = original
        .windows
        .keys()
        .copied()
        .find(|&p| {
            original
                .requirements(p)
                .unwrap()
                .external_writes
                .is_some_and(|n| n > 0)
        })
        .unwrap();
    for resource in 0..4 {
        let mut contract = original.clone();
        let req = contract.requirements_mut(point);
        match resource {
            0 => req.writes.a = 0,
            1 => req.flag_writes = 0,
            2 => {
                req.scratch = resources::Scratch::default();
                let index = contract.windows[&point].0;
                contract.window_rows[index].dp_operands = resources::Scratch::default();
            }
            _ => req.external_writes = Some(1),
        }
        assert!(
            contract.verify_selected(selected).is_err(),
            "resource {resource}"
        );
    }
}

#[test]
fn external_accesses_have_exact_width_and_adjacent_assignment_ownership() {
    let p = program("LONGCARD src,dst PROC Work() dst=src RETURN PROC Main() RETURN");
    let r = &p.routines[0];
    let plan = Plan::new(r, &p.data).unwrap();
    assert!(
        plan.contract
            .values
            .values()
            .any(|v| matches!(v.capture, Capture::Assignment { .. }))
    );
    let machine = materialize(&p).unwrap();
    let selected = machine.routines[0].code.selected.as_ref().unwrap();
    let mut contract = selected.placement().unwrap().clone();
    let point = contract
        .windows
        .keys()
        .copied()
        .find(|&p| contract.requirements(p).unwrap().external_reads == Some(4))
        .unwrap();
    contract.requirements_mut(point).external_reads = Some(3);
    assert!(contract.verify_selected(selected).is_err());
}

#[test]
fn scratch_is_physical_and_a_write_cannot_clobber_a_live_value() {
    let p = super::super::select::word_tests::program();
    let machine = materialize(&p).unwrap();
    let selected = machine.routines[0].code.selected.as_ref().unwrap();
    let mut contract = selected.placement().unwrap().clone();
    // Independently make a definitely emitted result store overlap the live
    // resource set. Width-sensitive protection must reject the full word store.
    let (point, offset, bytes) = selected
        .records()
        .iter()
        .find_map(|r| {
            let s = r.source?;
            if let Action::Instruction { effects, .. } = &r.action {
                effects.memory.iter().find_map(|e| match e.memory {
                    super::super::effects::Memory::DirectPage { offset, bytes }
                        if e.access == super::super::effects::Access::Write =>
                    {
                        Some((
                            ProgramPoint {
                                block: s.block,
                                index: s.index,
                            },
                            offset,
                            bytes,
                        ))
                    }
                    _ => None,
                })
            } else {
                None
            }
        })
        .unwrap();
    let id = contract.windows[&point];
    contract.window_rows[id.0].protected_dp = resources::Scratch::range(offset, bytes).unwrap();
    assert!(
        contract
            .verify_selected(selected)
            .unwrap_err()
            .contains("scratch conflicts")
    );
    assert!(resources::Scratch::range(0, 2).is_none());
    assert!(resources::Scratch::range(191, 2).is_none());
    assert!(
        resources::Scratch::range(128, 64)
            .unwrap()
            .contains(resources::Scratch::range(189, 3).unwrap())
    );
}

#[test]
fn indexed_forms_are_qualified_while_calls_remain_resource_barriers() {
    let p = program(
        "TYPE Cell=[BYTE ARRAY data(5)] PROC Touch() RETURN PROC Work(Cell POINTER p CARD n) p.data(n)=BYTE(n) Touch() RETURN PROC Main() RETURN",
    );
    let plan = Plan::new(&p.routines[1], &p.data).unwrap();
    for b in &p.routines[1].blocks {
        for (index, op) in b.ops.iter().enumerate() {
            if matches!(
                op,
                Mir65816Op::Call { .. }
                    | Mir65816Op::Store {
                        address: Mir65816Address { index: Some(_), .. },
                        ..
                    }
            ) {
                assert_eq!(
                    plan.contract
                        .requirements(ProgramPoint { block: b.id, index })
                        .unwrap()
                        .form,
                    if matches!(op, Mir65816Op::Call { .. }) {
                        resources::Form::Barrier
                    } else {
                        resources::Form::IndexedMemory
                    }
                );
            }
        }
    }
    materialize(&p).unwrap();
}

fn aggregate_program() -> Mir65816Program {
    let mut p = program(
        "TYPE Cell=[CARD n Cell POINTER next] PROC Work(Cell POINTER root,other) LET p=root.next LET n=p.n other.n=n p.n=7 other.n=p.n p.n=n RETURN PROC Main() RETURN",
    );
    let r = &mut p.routines[0];
    let (at, mut source) = r.blocks[0]
        .ops
        .iter()
        .enumerate()
        .find_map(|(i, op)| {
            if let Mir65816Op::Load { width, address, .. } = op {
                (width.get() == 2
                    && matches!(
                        address.base,
                        Mir65816AddressBase::Indirect(Mir65816Value::Temp(_, _))
                    ))
                .then_some((i, address.clone()))
            } else {
                None
            }
        })
        .unwrap();
    source.displacement = ByteOffset::ZERO;
    let mut destination = source.clone();
    destination.base =
        Mir65816AddressBase::Indirect(Mir65816Value::Param(r.frame.parameters[1].param));
    r.blocks[0].ops.insert(
        at + 1,
        Mir65816Op::Copy {
            source,
            destination,
            bytes: ByteSize::new(5),
            overlap_safe: true,
            source_volatile: false,
            destination_volatile: false,
        },
    );
    crate::mir65816::verify_program(&p).unwrap();
    p
}

#[test]
fn aggregate_windows_preserve_live_captures_and_require_exact_replayed_protocols() {
    let p = aggregate_program();
    let r = &p.routines[0];
    let plan = Plan::new(r, &p.data).unwrap();
    let point = *plan.contract.aggregates.keys().next().unwrap();
    assert_eq!(
        plan.contract.requirements(point).unwrap().form,
        resources::Form::Aggregate
    );
    assert!(
        plan.frame
            .temps
            .values()
            .any(|h| matches!(h, Location::DirectPage(_)))
    );
    plan.verify().unwrap();
    let machine = materialize(&p).unwrap();
    let selected = machine.routines[0].code.selected.as_ref().unwrap();
    let contract = selected.placement().unwrap();
    contract.verify_selected(selected).unwrap();
    for problem in 0..5 {
        let mut forged = plan.contract.clone();
        match problem {
            0 => forged.aggregates.clear(),
            1 => forged.aggregates.get_mut(&point).unwrap().0 += 1,
            2 => forged.aggregates.get_mut(&point).unwrap().1 = false,
            3 => forged.requirements_mut(point).scratch = resources::Scratch::default(),
            _ => forged.requirements_mut(point).external_reads = Some(1),
        }
        let mut altered = Plan::new(r, &p.data).unwrap();
        altered.contract = forged;
        assert!(altered.verify().is_err(), "aggregate plan {problem}");
    }
    for problem in 0..4 {
        let mut records = selected.records().to_vec();
        let at = records
            .iter()
            .position(|r| matches!(r.action, Action::Request(Request::AggregateCopy { .. })))
            .unwrap();
        if let Action::Request(Request::AggregateCopy {
            bytes,
            overlap_safe,
        }) = &mut records[at].action
        {
            match problem {
                0 => *bytes = 0,
                1 => *bytes += 1,
                2 => *overlap_safe = false,
                _ => *bytes = 1 << 24,
            }
        }
        assert!(
            selected
                .edited(records)
                .and_then(|s| contract.verify_selected(&s))
                .is_err(),
            "aggregate request {problem}"
        );
    }
    // Equal static access counts cannot authorize a different runtime extent.
    // Fresh replay reconstructs the canonical loop count from the MIR request.
    let mut records = selected.records().to_vec();
    let at = records
        .iter()
        .position(|r| matches!(r.action, Action::Request(Request::AggregateCopy { .. })))
        .unwrap();
    let child = records
        .iter_mut()
        .skip(at + 1)
        .find(|r| {
            matches!(
                r.action,
                Action::Instruction {
                    form: Instruction::Byte(ByteOp::LdaImm, 5),
                    ..
                }
            )
        })
        .unwrap();
    if let Action::Instruction { form, effects, .. } = &mut child.action {
        *form = Instruction::Byte(ByteOp::LdaImm, 6);
        *effects = form.effects(child.before.env);
    }
    let edited = selected.edited(records).unwrap();
    assert!(super::super::replay::emit(&edited, false).is_err());
}

#[test]
fn indexed_record_windows_use_common_homes_and_reject_scratch_and_extent_forgery() {
    let p = program(
        "TYPE Cell=[CARD n Cell POINTER next BYTE ARRAY data(8)] PROC Work(Cell POINTER root BYTE i) LET p=root.next p.data(i)=7 p.n=9 p.data(i)=p.data(i) RETURN PROC Main() RETURN",
    );
    let r = &p.routines[0];
    let plan = Plan::new(r, &p.data).unwrap();
    let (&point, _) = plan
        .contract
        .windows
        .iter()
        .find(|(p, _)| {
            plan.contract.requirements(**p).unwrap().form == resources::Form::IndexedMemory
        })
        .unwrap();
    assert!(
        plan.frame
            .temps
            .values()
            .any(|h| matches!(h, Location::DirectPage(s) if s.width == 3))
    );
    materialize(&p).unwrap();
    for problem in 0..3 {
        let mut altered = Plan::new(r, &p.data).unwrap();
        let requirement = altered.contract.requirements_mut(point);
        match problem {
            0 => requirement.scratch = resources::Scratch::default(),
            1 => requirement.external_writes = Some(2),
            _ => requirement.form = resources::Form::Barrier,
        }
        assert!(altered.verify().is_err(), "indexed resource {problem}");
    }
}

#[test]
fn stale_emitter_owners_and_forged_logical_boundaries_are_rejected() {
    let p = program(
        "TYPE Cell=[CARD amount] PROC Work(Cell POINTER p CARD n) p.amount=n RETURN PROC Main() RETURN",
    );
    let r = &p.routines[0];
    for problem in 0..4 {
        let mut plan = Plan::new(r, &p.data).unwrap();
        match problem {
            0 => {
                assert!(!plan.pointers.placement_bindings().is_empty());
                plan.pointers = Default::default();
            }
            1 => plan.contract.routine = RoutineId(u32::MAX),
            2 => plan.contract.entry = BlockId(u32::MAX),
            _ => plan.contract.reachable.clear(),
        }
        assert!(plan.verify().is_err(), "stale owner {problem}");
    }
}

#[test]
fn dense_resource_rows_are_complete_and_cannot_forge_indices() {
    let p = super::super::select::word_tests::program();
    let machine = materialize(&p).unwrap();
    let selected = machine.routines[0].code.selected.as_ref().unwrap();
    let original = selected.placement().unwrap();
    assert_eq!(original.window_rows.len(), original.windows.len());
    assert!(original.descriptions.len() < original.window_rows.len());
    for problem in 0..5 {
        let corrupt = |contract: &mut Contract| match problem {
            0 => *contract.windows.values_mut().next().unwrap() = WindowId(usize::MAX),
            1 => {
                contract.window_rows.pop();
            }
            2 => {
                let first = *contract.windows.values().next().unwrap();
                *contract.windows.values_mut().nth(1).unwrap() = first;
            }
            3 => contract.window_rows[0].description = DescriptionId(usize::MAX),
            _ => contract.window_rows[0].access_owner = WindowId(usize::MAX),
        };
        let mut contract = original.clone();
        corrupt(&mut contract);
        assert!(
            contract.verify_selected(selected).is_err(),
            "dense row {problem}"
        );
        let mut plan = Plan::new(&p.routines[0], &p.data).unwrap();
        corrupt(&mut plan.contract);
        assert!(plan.verify().is_err(), "dense plan {problem}");
    }
    let mut plan = Plan::new(&p.routines[0], &p.data).unwrap();
    plan.contract.window_rows[0].dp_operands = resources::Scratch::range(128, 64).unwrap();
    assert!(plan.verify().is_err());
}

#[test]
fn branch_entry_and_simultaneous_transfer_forgery_cannot_publish() {
    let r = mixed::tests::diamond();
    for bad in 0..3 {
        let mut plan = Plan::new(&r, &[]).unwrap();
        match bad {
            0 => {
                plan.contract.entries.clear();
            }
            1 => {
                plan.contract.regions.clear();
            }
            _ => {
                plan.contract
                    .edges
                    .values_mut()
                    .find(|e| e.mixed.is_some())
                    .unwrap()
                    .mixed
                    .as_mut()
                    .unwrap()
                    .edge
                    .args
                    .reverse();
            }
        }
        assert!(plan.verify().is_err());
    }
    let code = select::routine(&r, false).unwrap().code;
    let selected = code.selected.as_ref().unwrap();
    for bad in 0..3 {
        let mut records = selected.records().to_vec();
        let action = &mut records
            .iter_mut()
            .find(|r| matches!(r.action, Action::Request(Request::MixedEdge(..))))
            .unwrap()
            .action;
        if let Action::Request(Request::MixedEdge(plan, staging)) = action {
            match bad {
                0 => {
                    plan.edge.args.reverse();
                }
                1 => {
                    staging.push(Slot {
                        offset: 2,
                        width: 3,
                    });
                }
                _ => {
                    *action = Action::Request(Request::Barrier);
                }
            }
        }
        let edited = selected.edited(records).unwrap();
        assert!(
            selected
                .placement()
                .unwrap()
                .verify_selected(&edited)
                .is_err()
        );
    }
}

#[test]
fn invocation_segment_contract_and_reload_forgery_cannot_publish() {
    // A forged loop census must not weaken the fixed-point entry contract.
    let loop_routine = mixed::tests::looping();
    let mut plan = Plan::new(&loop_routine, &[]).unwrap();
    plan.contract.loops.clear();
    assert!(plan.verify().is_err());
    let r = mixed::tests::calling();
    for bad in 0..3 {
        let mut plan = Plan::new(&r, &[]).unwrap();
        match bad {
            0 => plan.contract.segments.clear(),
            1 => plan.contract.segments.get_mut(&TempId(0)).unwrap()[0].first -= 1,
            _ => {
                plan.contract.segments.get_mut(&TempId(0)).unwrap()[0]
                    .slot
                    .width = 2
            }
        }
        assert!(plan.verify().is_err());
    }
    let code = select::routine(&r, false).unwrap().code;
    let selected = code.selected.as_ref().unwrap();
    for bad in 0..4 {
        let mut records = selected.records().to_vec();
        let index = records
            .iter()
            .position(|r| matches!(r.action, Action::Request(Request::ReloadResident(..))))
            .unwrap();
        if bad == 3 {
            let end = records[index + 1..]
                .iter()
                .position(|r| matches!(r.action, Action::SourceEnd { .. }))
                .unwrap()
                + index
                + 1;
            let reload = records.remove(index);
            records.insert(end - 1, reload);
        } else if let Action::Request(Request::ReloadResident(_, source, dest)) =
            &mut records[index].action
        {
            match bad {
                0 => source.offset += 2,
                1 => dest.offset += 2,
                _ => records[index].action = Action::Request(Request::Barrier),
            }
        }
        let result = selected
            .edited(records)
            .and_then(|edited| selected.placement().unwrap().verify_selected(&edited));
        assert!(result.is_err(), "reload forgery {bad}");
    }
}

#[test]
fn native_result_routes_are_recomputed_from_the_complete_logical_census() {
    let p = program(
        "CARD FUNC Echo(CARD x) RETURN(x) CARD FUNC Forward(CARD x) CARD y y=1 RETURN(Echo(x)) PROC Main() RETURN",
    );
    let r = &p.routines[1];
    let mut plan = Plan::new(r, &p.data).unwrap();
    let (&point, call) = plan.calls.iter().next().unwrap();
    assert!(matches!(call.route, call_flow::Route::Return(_)));
    let temp = call.result.unwrap().temp;
    assert!(!plan.frame.temps.contains_key(&temp));
    plan.calls.get_mut(&point).unwrap().route = call_flow::Route::Capture;
    assert!(plan.verify().unwrap_err().contains("recomputed"));
    let mut plan = Plan::new(r, &p.data).unwrap();
    plan.contract
        .calls
        .get_mut(&point)
        .unwrap()
        .abi
        .outgoing_bytes = ByteSize::ZERO;
    assert!(plan.verify().unwrap_err().contains("recomputed"));
}

#[test]
fn native_zero_consumers_own_lanes_and_refuse_hidden_or_different_uses() {
    for ty in ["BYTE", "CARD", "INT"] {
        let p = program_with_optimization(
            &format!(
                "{ty} FUNC Echo({ty} x) RETURN(x) BYTE FUNC Work({ty} value) RETURN(Echo(value)=0) PROC Main() RETURN"
            ),
            true,
        );
        let original = &p.routines[1];
        let call_index = original.blocks[0]
            .ops
            .iter()
            .position(|op| matches!(op, Mir65816Op::Call { .. }))
            .unwrap();
        let Mir65816Op::Call {
            result: Some((temp, bytes)),
            ..
        } = original.blocks[0].ops[call_index]
        else {
            panic!()
        };
        for refusal in 0..4 {
            let mut r = original.clone();
            let b = &mut r.blocks[0];
            if refusal == 3 {
                let ty = r
                    .temps
                    .iter()
                    .find(|(id, _)| *id == temp)
                    .unwrap()
                    .1
                    .clone();
                r.temps.push((TempId(999), ty));
                b.ops.push(Mir65816Op::Cast {
                    dest: TempId(999),
                    kind: NirCastKind::Integer,
                    from: bytes,
                    to: bytes,
                    from_signed: false,
                    value: Mir65816Value::Temp(temp, bytes),
                });
            } else if let Mir65816Op::Compare {
                operation, right, ..
            } = &mut b.ops[call_index + 1]
            {
                if refusal == 1 {
                    *operation = NirCompareOp::Lt;
                }
                if refusal == 2 {
                    *right = if bytes.get() == 1 {
                        Mir65816Value::U8(1)
                    } else {
                        Mir65816Value::U16(1)
                    };
                }
            } else {
                panic!("{ty}: {:?}", b.ops)
            }
            let plan = Plan::new(&r, &p.data).unwrap();
            assert_eq!(
                plan.demand.native_output(temp).is_some(),
                refusal == 0,
                "{ty}/{refusal}"
            );
            assert_eq!(plan.frame.temps.contains_key(&temp), refusal != 0);
            let m = super::super::select::routine(&r, false).unwrap();
            if refusal == 0 {
                let span = &m.code.mir_spans[&(r.blocks[0].id, call_index + 1)];
                assert!(
                    m.code.bytes[span.clone()]
                        .windows(2)
                        .any(|bytes| bytes == [0xc9, 0]),
                    "fresh comparison missing"
                );
            }
        }
    }
}

#[test]
fn native_local_destinations_omit_only_the_intermediate_and_keep_store_spans() {
    for ty in ["BYTE", "CARD", "ADDRESS", "LONGCARD"] {
        let p = program_with_optimization(
            &format!(
                "{ty} FUNC Echo({ty} x) RETURN(x) {ty} FUNC Work({ty} value) {ty} local local=Echo(value) Echo(value) RETURN(local) PROC Main() RETURN"
            ),
            true,
        );
        let original = p
            .routines
            .iter()
            .find(|r| r.name.ends_with("Work"))
            .unwrap();
        let index = original.blocks[0]
            .ops
            .iter()
            .position(|op| matches!(op, Mir65816Op::Call { .. }))
            .unwrap();
        let Mir65816Op::Call {
            result: Some((temp, bytes)),
            ..
        } = original.blocks[0].ops[index]
        else {
            panic!()
        };
        let Mir65816Op::Store { address, .. } = &original.blocks[0].ops[index + 1] else {
            panic!("{:?}", original.blocks[0].ops)
        };
        let Mir65816AddressBase::AutomaticFrame(object) = address.base else {
            panic!()
        };
        for shape in 0..7 {
            let mut rejected = original.blocks[0].ops[index + 1].clone();
            let mut routine = original.clone();
            if let Mir65816Op::Store { address, width, .. } = &mut rejected {
                match shape {
                    0 => {
                        address.index = Some(Mir65816Index {
                            value: Mir65816Value::U8(0),
                            stride: ByteSize::new(1),
                        })
                    }
                    1 => address.base = Mir65816AddressBase::Indirect(Mir65816Value::U24(0x7100)),
                    2 => {
                        address.base =
                            Mir65816AddressBase::Static(NirStorageId::Global(SymbolId(0)))
                    }
                    3 => address.displacement = ByteOffset::new(255),
                    4 => *width = ByteSize::new(5),
                    5 => {
                        routine
                            .frame
                            .objects
                            .iter_mut()
                            .find(|o| o.id == object)
                            .unwrap()
                            .owner = Mir65816FrameObjectOwner::Param(ParamId(0))
                    }
                    6 => {
                        address.base =
                            Mir65816AddressBase::AutomaticFrame(Mir65816FrameObjectId(u32::MAX))
                    }
                    _ => unreachable!(),
                }
            }
            assert!(!call_flow::private_store(
                &routine,
                temp,
                bytes.get() as u8,
                &rejected
            ));
        }
        for refusal in 0..4 {
            let mut r = original.clone();
            match refusal {
                1 => {
                    r.frame
                        .objects
                        .iter_mut()
                        .find(|o| o.id == object)
                        .unwrap()
                        .addressable = true
                }
                2 => {
                    r.frame
                        .objects
                        .iter_mut()
                        .find(|o| o.id == object)
                        .unwrap()
                        .mutable = false
                }
                3 => {
                    if let Mir65816Op::Store { volatile, .. } = &mut r.blocks[0].ops[index + 1] {
                        *volatile = true
                    }
                }
                _ => (),
            }
            let plan = Plan::new(&r, &p.data).unwrap();
            assert_eq!(
                plan.demand.native_output(temp).is_some(),
                refusal == 0,
                "{ty}/{refusal}"
            );
            assert_eq!(plan.frame.temps.contains_key(&temp), refusal != 0);
            if refusal == 0 {
                assert!(matches!(
                    plan.calls[&ProgramPoint {
                        block: r.blocks[0].id,
                        index
                    }]
                        .route,
                    call_flow::Route::Store(_)
                ));
                let m = super::super::select::routine(&r, false).unwrap();
                let span = &m.code.mir_spans[&(r.blocks[0].id, index + 1)];
                let writes: u8 = m.code.bytes[span.clone()]
                    .iter()
                    .filter(|&&b| b == 0x83)
                    .count() as u8;
                assert_eq!(writes, if bytes.get() > 2 { 2 } else { 1 });
                assert!(!m.frame.temps.contains_key(&temp));
            }
        }
    }
}
