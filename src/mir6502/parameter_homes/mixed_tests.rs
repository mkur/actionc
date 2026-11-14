use super::*;
#[allow(dead_code)]
#[path = "../../codegen/test_cpu.rs"]
mod cpu;

fn source(text: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    super::super::lower_program(&nir).unwrap()
}
fn bind(program: &mut MirProgram, lanes: Vec<MirParamLane>) -> (RoutineId, crate::nir::ParamId) {
    let routine = program
        .routines
        .iter_mut()
        .find(|r| r.name == "Echo")
        .unwrap();
    let MirStorageBase::Param(id) = routine.frame.params[0].base else {
        panic!()
    };
    routine.frame.parameter_homes = Some(vec![MirParamBinding {
        param: id,
        home: MirParamHome::Lanes(lanes),
    }]);
    routine.frame.fixed_zero_page.push(MirFixedZpSlot(0xa0));
    (routine.id, id)
}
const SOURCE: &str = "CARD input=$600,observed=$602\nCARD FUNC Echo(CARD value) RETURN(value)\nPROC Main() observed=Echo(input) RETURN\n";

#[test]
fn mixed_lanes_execute_and_publish_only_the_real_memory_fragment() {
    for origin in [0x3000u16, 0x41c7] {
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            for text in [SOURCE.to_string(), SOURCE.replace('\n', "\r\n")] {
                let mut program = source(&text);
                let (owner, id) = bind(
                    &mut program,
                    vec![
                        MirParamLane::ZeroPage(MirFixedZpSlot(0xa0)),
                        MirParamLane::Reg(MirReg::X),
                    ],
                );
                super::super::verify_program(&program, MirPhase::PreMaterialization).unwrap();
                let program = super::super::materialize_program_with_origin_and_runtime(
                    program,
                    &super::super::Mir6502Config {
                        enable_small_leaf_inlining: false,
                        ..super::super::Mir6502Config::optimized()
                    },
                    origin,
                    runtime,
                )
                .unwrap();
                let layout = super::super::materialize::MaterializeLayout::new(&program, origin);
                let mut emitter =
                    crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
                let summary = super::super::emit::emit_program_with_physical_memory(
                    &program,
                    origin,
                    &mut emitter,
                    true,
                )
                .unwrap();
                let bytes = emitter.finish_with_relocations().unwrap().bytes;
                for (offset, expected) in [(0, Some(0xa0)), (1, None)] {
                    let mem = MirMem::Param { id, offset };
                    assert_eq!(layout.mem_address(owner, &mem), expected);
                    assert_eq!(summary.physical_memory.address(owner, &mem), expected);
                }
                let symbols: Vec<_> = summary
                    .storage_symbols
                    .iter()
                    .filter(|s| {
                        s.scope == crate::codegen::CodegenSymbolScope::Routine("Echo".into())
                    })
                    .collect();
                assert_eq!(symbols.len(), 1);
                assert_eq!(
                    (&*symbols[0].name, symbols[0].address, symbols[0].size),
                    ("value+0", 0xa0, 1)
                );
                let entry = summary
                    .routine_addresses
                    .iter()
                    .find(|r| r.name == "Main")
                    .unwrap()
                    .address;
                for low in 0..=255u16 {
                    let value = low | ((low ^ 0xa5) << 8);
                    let mut memory = [0x5au8; 65536];
                    memory[origin as usize..origin as usize + bytes.len()].copy_from_slice(&bytes);
                    memory[0x600..0x602].copy_from_slice(&value.to_le_bytes());
                    cpu::run_memory(&mut memory, entry as usize);
                    assert_eq!(&memory[0x602..0x604], &value.to_le_bytes());
                }
            }
        }
    }
}

#[test]
fn malformed_mixed_bindings_and_source_overlap_are_rejected() {
    for lanes in [
        vec![],
        vec![MirParamLane::ZeroPage(MirFixedZpSlot(0xa0))],
        vec![
            MirParamLane::Reg(MirReg::X),
            MirParamLane::ZeroPage(MirFixedZpSlot(0xa0)),
        ],
        vec![MirParamLane::ZeroPage(MirFixedZpSlot(0xa0)); 2],
        vec![
            MirParamLane::ZeroPage(MirFixedZpSlot(0xe0)),
            MirParamLane::Reg(MirReg::X),
        ],
    ] {
        let mut p = source(SOURCE);
        bind(&mut p, lanes);
        assert!(super::super::verify_program(&p, MirPhase::PreMaterialization).is_err());
    }
    let mut p = source(&format!("BYTE reserved=$A0\n{SOURCE}"));
    bind(
        &mut p,
        vec![
            MirParamLane::ZeroPage(MirFixedZpSlot(0xa0)),
            MirParamLane::Reg(MirReg::X),
        ],
    );
    assert!(
        super::super::verify_program(&p, MirPhase::PreMaterialization)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("source-owned"))
    );
}

#[test]
fn mixed_register_clobber_cannot_fall_back_to_a_fictitious_address() {
    let mut p = source(SOURCE);
    let (owner, _) = bind(
        &mut p,
        vec![
            MirParamLane::ZeroPage(MirFixedZpSlot(0xa0)),
            MirParamLane::Reg(MirReg::X),
        ],
    );
    p.routines
        .iter_mut()
        .find(|r| r.id == owner)
        .unwrap()
        .blocks[0]
        .ops
        .insert(
            0,
            // An observable read cannot disappear as a dead constant write.
            MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(MirMem::Absolute(0xd20a)),
                width: MirWidth::Byte,
            },
        );
    assert!(
        super::super::materialize_program(p, &super::super::Mir6502Config::optimized()).is_err()
    );
}
