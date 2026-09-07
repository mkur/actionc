#![allow(dead_code)]

use std::path::Path;

use actionc::nir::{
    self, BlockId, NirDataFragment, NirGlobalInit, NirLocalBacking, NirMemoryAccess, NirOp,
    NirPlaceKind, NirStorageDuration, NirTerminator, NirValue, SymbolId, TempId,
};
use actionc::target::{AddressValue, ByteOffset, TargetId};

mod nir_fixture_support;

use nir_fixture_support::{NIR_FIXTURE_CASES, lower_case, structural_coverage_programs};

fn fixture(name: &str) -> nir::NirProgram {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let case = NIR_FIXTURE_CASES
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing NIR fixture case `{name}`"));
    lower_case(repo_root, *case)
}

fn assert_rejected(case: &str, program: &nir::NirProgram, expected: &str) {
    let diagnostics = match nir::verify_program(program) {
        Ok(()) => panic!("invalid NIR matrix case `{case}` unexpectedly verified"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(expected)),
        "invalid NIR matrix case `{case}` did not report `{expected}`: {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_one_fault_from_each_contract_family() {
    let mut cases = Vec::new();

    let mut missing_global = fixture("scalar_assignments");
    let NirOp::Store { place, .. } = &mut missing_global.routines[0].blocks[0].ops[0] else {
        panic!("expected scalar store")
    };
    let NirPlaceKind::Global { id, .. } = &mut place.kind else {
        panic!("expected global store")
    };
    *id = SymbolId(999);
    cases.push(("stable storage ID", missing_global, "unknown global id 999"));

    let mut missing_block = fixture("scalar_assignments");
    missing_block.routines[0].blocks[0].terminator = NirTerminator::Goto(nir::NirEdge {
        target: BlockId(999),
        args: Vec::new(),
    });
    cases.push((
        "CFG edge",
        missing_block,
        "target block id `999` does not exist",
    ));

    let mut undefined_temp = fixture("scalar_assignments");
    let NirOp::Store { src, .. } = &mut undefined_temp.routines[0].blocks[0].ops[2] else {
        panic!("expected temp store")
    };
    let NirValue::Temp { id, .. } = src else {
        panic!("expected temp source")
    };
    *id = TempId(999);
    cases.push(("use-def", undefined_temp, "uses undefined temp `%t999`"));

    let card_type = fixture("integer_operations").globals[2]
        .ty
        .clone()
        .expect("CARD result type");
    let mut type_mismatch = fixture("scalar_assignments");
    let NirOp::Load { ty, .. } = &mut type_mismatch.routines[0].blocks[0].ops[1] else {
        panic!("expected scalar load")
    };
    *ty = card_type;
    cases.push((
        "operation type",
        type_mismatch,
        "type does not match temp table",
    ));

    let mut missing_signature = fixture("calls_returns");
    let call = missing_signature
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Main")
        .and_then(|routine| {
            routine.blocks[0]
                .ops
                .iter_mut()
                .find(|op| matches!(op, NirOp::Call { .. }))
        })
        .expect("direct call");
    let NirOp::Call { signature, .. } = call else {
        unreachable!()
    };
    *signature = None;
    cases.push((
        "call signature",
        missing_signature,
        "call has no callable signature or convention",
    ));

    let mut storage_mismatch = fixture("activation_storage.motorola-68000");
    storage_mismatch.routines[0].params[0].duration = NirStorageDuration::RoutineStatic;
    cases.push((
        "activation storage",
        storage_mismatch,
        "duration does not match routine activation",
    ));

    let mut alias_cycle = fixture("activation_storage.motorola-68000");
    let worker = &mut alias_cycle.routines[0];
    let alias = worker
        .locals
        .iter_mut()
        .find(|local| local.name == "alias")
        .expect("alias local");
    alias.backing = NirLocalBacking::Alias {
        target: alias.id,
        target_name: alias.name.clone(),
        offset: ByteOffset::ZERO,
    };
    cases.push(("alias graph", alias_cycle, "participates in an alias cycle"));

    let mut bad_fragment = fixture("data_relocations");
    let Some(NirGlobalInit::Bytes { image, .. }) = &mut bad_fragment.globals[1].init else {
        panic!("relocation byte image")
    };
    let NirDataFragment::Address { offset, .. } = &mut image.fragments[0] else {
        panic!("address fragment")
    };
    *offset = ByteOffset::new(4);
    cases.push((
        "data relocation",
        bad_fragment,
        "exceeds 4 initialized bytes",
    ));

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad_effect = structural_coverage_programs(repo_root).remove(1);
    let builtin = bad_effect.routines[0].blocks[0]
        .ops
        .iter_mut()
        .find(|op| {
            matches!(
                op,
                NirOp::Call {
                    callee: nir::NirCallee::Builtin(_),
                    ..
                }
            )
        })
        .expect("builtin effect call");
    let NirOp::Call { effects, .. } = builtin else {
        unreachable!()
    };
    let NirMemoryAccess::Regions(regions) = &mut effects.memory.writes else {
        panic!("global effect region")
    };
    regions[0].offset = ByteOffset::new(1);
    cases.push(("effect region", bad_effect, "exceeds storage size 1"));

    let mut wrong_address_space = fixture("pointer_operations");
    let address = wrong_address_space.routines[0].blocks[0]
        .ops
        .iter_mut()
        .find_map(|op| match op {
            NirOp::Store {
                src: NirValue::AddressConst { address, .. },
                ..
            } => Some(address),
            _ => None,
        })
        .expect("typed address constant");
    *address = AddressValue::code(address.value);
    cases.push((
        "address space",
        wrong_address_space,
        "address constant does not fit its typed address space",
    ));

    let mut wrong_foreign_target = fixture("inline_asm_fixed_array");
    let code = wrong_foreign_target.routines[0].blocks[0]
        .ops
        .iter_mut()
        .find_map(|op| match op {
            NirOp::ForeignCode { code, .. } => Some(code),
            _ => None,
        })
        .expect("inline assembly payload");
    code.target = TargetId::Wdc65816Native;
    cases.push((
        "foreign target",
        wrong_foreign_target,
        "cannot be used with selected target",
    ));

    for (name, program, expected) in cases {
        assert_rejected(name, &program, expected);
    }
}
