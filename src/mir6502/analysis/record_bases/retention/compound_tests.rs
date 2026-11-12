use super::*;
use crate::mir6502::analysis::{
    address_ranges::MirAddressRanges, known_callees::MirKnownCalleeSummaries,
    posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration,
};
use crate::mir6502::{self, Mir6502Config};

fn prepare() -> (
    crate::nir::NirProgram,
    MirProgram,
    MirProgram,
    MirPhysicalMemory,
) {
    let text = include_str!("../../../../../fixtures/runtime/checked_pointer_ranges.act");
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let n = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let l = mir6502::lower_program(&n).unwrap();
    let m = mir6502::materialize_program_with_origin_and_runtime(
        l.clone(),
        &Mir6502Config::optimized(),
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    let s = crate::mir6502::emit::emit_program_with_physical_references(
        &m,
        0x3000,
        &mut emitter,
        true,
        &m,
    )
    .unwrap();
    emitter.finish_with_relocations().unwrap();
    let mut map = s.physical_memory;
    map.protect_execution_ranges(s.source_ranges.iter().map(|r| (r.start, r.end)).collect());
    (n, l, m, map)
}
fn query(
    r: &MirRoutine,
    memory: &MirPhysicalMemory,
    calls: &CallWrites,
    known: &MirKnownCalleeSummaries,
) -> Option<CompoundRegionProof> {
    let snapshot =
        PostHomeAnalysisSnapshot::new_with_known_callees(r, MirRoutineGeneration::initial(), known)
            .unwrap();
    let c = PostHomeRewriteContext::new(&snapshot);
    r.blocks.iter().find_map(|b| {
        prove_compound_region(
            r,
            &c,
            MirSite::Op {
                block: b.id,
                op_index: b.ops.len().checked_sub(9)?,
            },
            memory,
            calls,
        )
    })
}
#[test]
fn regional_proof_replays_exact_body_layout_roles_and_generation() {
    let (n, l, m, map) = prepare();
    let ranges = MirAddressRanges::project(&n, &l, &m).unwrap();
    let calls = CallWrites::analyze(&m, &map)
        .with_bounded_machine_calls(&m)
        .with_range_inputs(&m, &ranges);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&m);
    let r = m.routines.iter().find(|r| r.name == "Guarded").unwrap();
    let proof = query(r, &map, &calls, &known).unwrap();
    assert_ne!(proof.original.last(), proof.replacement.last());
    let plan = crate::mir6502::rewrite::plan::MirCompoundRegionPlan {
        proof: proof.clone(),
    };
    let apply = |r: &mut MirRoutine, generation, memory| {
        crate::mir6502::rewrite::driver::apply_compound_region(r, generation, &known, memory, &plan)
    };
    assert!(!apply(
        &mut r.clone(),
        MirRoutineGeneration::initial().next(),
        &map
    ));
    let mut changed = r.clone();
    changed.blocks.last_mut().unwrap().ops.pop();
    assert!(!apply(&mut changed, MirRoutineGeneration::initial(), &map));
    let mut shifted = map.clone();
    shifted.protect_execution_ranges(vec![(0x2104, 0x290B)]);
    assert!(!apply(
        &mut r.clone(),
        MirRoutineGeneration::initial(),
        &shifted
    ));
    let mut forged = proof.clone();
    forged.replacement = vec![proof.original.last().unwrap().clone()];
    let snapshot = PostHomeAnalysisSnapshot::new_with_known_callees(
        r,
        MirRoutineGeneration::initial(),
        &known,
    )
    .unwrap();
    assert!(!forged.validate(r, &PostHomeRewriteContext::new(&snapshot), &map));
    let MirSite::Op { block, op_index } = proof.point().site else {
        unreachable!()
    };
    let generic = crate::mir6502::rewrite::plan::MirPostHomeRewritePlan {
        witness: None,
        generation: MirRoutineGeneration::initial(),
        block,
        range: op_index..op_index + 9,
        replacement: proof.replacement().to_vec(),
        removed_homes: vec![],
        exit_state_change: MirExitStateChange::default(),
        change_set: crate::mir6502::rewrite::plan::MirChangeSet::posthome_operation_change(),
        stat: "test-unproved-compound",
        observations: vec![],
        family_priority: 0,
        estimated_byte_saving: 20,
        estimated_cycle_saving: 28,
    };
    let error = crate::mir6502::rewrite::driver::MirPostHomeRewriteDriver::with_max_rounds(1)
        .run_fixed_point_with_physical_memory(&mut r.clone(), &known, &map, |_, _| {
            vec![generic.clone()]
        })
        .unwrap_err();
    assert!(format!("{error:?}").contains("removed home definition"));
    let mut selected = r.clone();
    assert!(apply(&mut selected, MirRoutineGeneration::initial(), &map));
    assert_eq!(
        r.blocks.iter().map(|b| b.ops.len()).sum::<usize>()
            - selected.blocks.iter().map(|b| b.ops.len()).sum::<usize>(),
        8
    );
    assert!(query(&selected, &map, &calls, &known).is_none());
}
#[test]
fn either_source_home_lane_and_execution_overlap_keep_the_call_barrier() {
    let (n, l, m, map) = prepare();
    let ranges = MirAddressRanges::project(&n, &l, &m).unwrap();
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&m);
    let r = m.routines.iter().find(|r| r.name == "Guarded").unwrap();
    for (lane, address) in [(0, 0x2200), (1, 0x2200), (0, 0x00AC)] {
        let mut alias = map.clone();
        let entry=alias.entries.iter_mut().find(|(id,mem,_,_)|*id==r.id && matches!(mem,MirMem::Param {id:crate::nir::ParamId(0),offset} if *offset==lane)).unwrap();
        entry.2 = address;
        let calls = CallWrites::analyze(&m, &alias)
            .with_bounded_machine_calls(&m)
            .with_range_inputs(&m, &ranges);
        assert!(
            query(r, &alias, &calls, &known).is_none(),
            "lane {lane}, {address:04X}"
        );
    }
    let mut code = map.clone();
    code.protect_execution_ranges(vec![(0x290A, 0x290B)]);
    let calls = CallWrites::analyze(&m, &code)
        .with_bounded_machine_calls(&m)
        .with_range_inputs(&m, &ranges);
    assert!(query(r, &code, &calls, &known).is_none());
}
#[test]
fn explicit_scratch_reads_live_tail_state_and_changed_targets_are_rejected() {
    let (n, l, m, map) = prepare();
    let ranges = MirAddressRanges::project(&n, &l, &m).unwrap();
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&m);
    let calls = CallWrites::analyze(&m, &map)
        .with_bounded_machine_calls(&m)
        .with_range_inputs(&m, &ranges);
    let r = m.routines.iter().find(|r| r.name == "Guarded").unwrap();
    let mut observed = r.clone();
    observed.blocks.last_mut().unwrap().ops.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC))),
        width: MirWidth::Byte,
    });
    assert!(query(&observed, &map, &calls, &known).is_none());
    let mut changed = r.clone();
    let b = changed.blocks.last_mut().unwrap();
    let i = b.ops.len() - 10;
    if let MirOp::Call { target, .. } = &mut b.ops[i] {
        *target = MirCallTarget::Routine(RoutineId(999));
    }
    assert!(query(&changed, &map, &calls, &known).is_none());
    let mut published = r.clone();
    published.abi = MirRoutineAbi::ActionObservable;
    assert!(query(&published, &map, &calls, &known).is_none());
    let mut stale = r.clone();
    stale.blocks[0].ops.insert(
        0,
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0,
            width: MirWidth::Byte,
        },
    );
    assert!(query(&stale, &map, &calls, &known).is_none());
    let generic = m
        .routines
        .iter()
        .find(|r| r.name == "Unrestricted")
        .unwrap();
    assert!(query(generic, &map, &calls, &known).is_none());
}
