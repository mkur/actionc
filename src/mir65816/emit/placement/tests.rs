use super::*;

fn program(source: &str) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::mir65816::lower_program(&nir).unwrap()
}

fn record() -> Mir65816Program {
    program(
        "TYPE Cell=[BYTE tag CARD amount Cell POINTER next] PROC Work(Cell POINTER p CARD n) p.amount=n p.tag=7 p=p.next n=p.amount+1 RETURN PROC Main() RETURN",
    )
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
fn unqualified_forms_are_explicit_resource_barriers() {
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
                    resources::Form::Barrier
                );
            }
        }
    }
    materialize(&p).unwrap();
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
