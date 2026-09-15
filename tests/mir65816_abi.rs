use actionc::mir65816::{self, abi, *};
use actionc::nir;
use actionc::target::{ByteOffset, ByteSize, TargetId};
use actionc::{lexer, parser, semantic};

fn source_nir(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    program
}

fn variants(source: &str, target: TargetId) -> [nir::NirProgram; 2] {
    let raw = source_nir(source, target);
    let optimized = nir::optimize_program(&raw).unwrap();
    [raw, optimized]
}

fn routine<'a>(program: &'a Mir65816Program, name: &str) -> &'a Mir65816Routine {
    program
        .routines
        .iter()
        .find(|routine| routine.name == name)
        .unwrap()
}

const CALLS: &str = r#"
BYTE b
CARD c
BYTE POINTER p
LONGINT value,result
LONGINT FUNC POINTER callback(BYTE first CARD second BYTE POINTER ptr LONGINT last)

PROC Empty()
RETURN

LONGINT FUNC Mixed(BYTE first CARD second BYTE POINTER ptr LONGINT last)
  first==+1
RETURN(last+LONGINT(first)+LONGINT(second)+LONGINT(ptr^))

PROC Main()
  p=@b
  result=Mixed(b,c,p,value)
  callback=@Mixed
  result=callback(b,c,p,value)
  Empty()
RETURN
"#;

#[test]
fn native_direct_and_indirect_calls_share_the_published_argument_and_result_layout() {
    for nir in variants(CALLS, TargetId::Wdc65816Native) {
        let mir = mir65816::lower_program(&nir).unwrap();
        let expected =
            [(0, 1, 1), (2, 2, 2), (4, 3, 1), (8, 4, 2)].map(|(offset, size, alignment)| {
                Mir65816AbiHome::StackArgument {
                    offset: ByteOffset::new(offset),
                    size: ByteSize::new(size),
                    alignment: ByteSize::new(alignment),
                }
            });
        let callee = routine(&mir, "Mixed");
        assert_eq!(
            callee
                .frame
                .parameters
                .iter()
                .map(|p| p.incoming)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(callee.frame.parameters[0].frame_object.is_some());
        assert_eq!(
            callee.result_home,
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16))
        );
        let mut transfers = Vec::new();
        for op in routine(&mir, "Main")
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
        {
            if let Mir65816Op::Call { plan, .. } = op {
                let native = plan.native.unwrap();
                assert_eq!(native.boundary, abi::BOUNDARY);
                assert_eq!(native.caller_cleanup_bytes, plan.outgoing_bytes);
                if plan.arguments.is_empty() {
                    assert_eq!(native.argument_bytes, ByteSize::ZERO);
                    assert_eq!(plan.outgoing_bytes, ByteSize::ONE);
                } else {
                    assert_eq!(plan.arguments, expected);
                    assert_eq!(plan.result, callee.result_home);
                    assert_eq!(native.argument_bytes.get(), 12);
                    assert_eq!(plan.outgoing_bytes.get(), 13);
                    transfers.push((
                        native.transfer,
                        plan.call_form,
                        native.transfer.peak_bytes().get(),
                    ));
                }
            }
        }
        assert_eq!(
            transfers,
            vec![
                (abi::FarTransfer::Jsl, Mir65816CallForm::FarJsl, 3),
                (abi::FarTransfer::StackRtl, Mir65816CallForm::FarStackRtl, 6)
            ]
        );
        let contract = mir.native_abi.unwrap();
        assert_eq!(contract.version, 1);
        assert!(contract.unsupported_signatures.is_empty());
        assert_eq!(contract.boundary.data_bank, 0);
        assert!(contract.boundary.decimal_clear);
        assert!(
            mir.task_switch_state
                .required
                .contains(&Mir65816SavedState::ProgramCounter)
        );
        assert_eq!(
            mir.task_switch_state.native_memory,
            Some(abi::SUSPENDED_MEMORY)
        );
        assert_eq!(abi::SUSPENDED_MEMORY.saved_frame_bytes.get(), 13);
        assert_eq!(abi::SUSPENDED_MEMORY.direct_page_bytes.get(), 256);
    }
}

#[test]
fn every_native_scalar_result_is_explicit_at_an_independently_callable_entry() {
    for (source_type, home) in [
        ("BYTE", abi::ResultLocation::A8ZeroExtended),
        ("CARD", abi::ResultLocation::A16),
        ("INT", abi::ResultLocation::A16),
        ("ADDRESS", abi::ResultLocation::A16X8ZeroExtended),
        ("SIZE", abi::ResultLocation::A16X8ZeroExtended),
        ("BYTE POINTER", abi::ResultLocation::A16X8ZeroExtended),
        ("LONGCARD", abi::ResultLocation::A16X16),
        ("LONGINT", abi::ResultLocation::A16X16),
    ] {
        let source = format!(
            "{source_type} FUNC Identity({source_type} value) RETURN(value) PROC Main() RETURN"
        );
        for nir in variants(&source, TargetId::Wdc65816Native) {
            let mir = mir65816::lower_program(&nir).unwrap();
            assert_eq!(
                routine(&mir, "Identity").result_home,
                Some(Mir65816AbiHome::NativeResult(home)),
                "{source_type}"
            );
        }
    }
}

#[test]
fn small_model_keeps_its_packed_homes_near_calls_and_abstract_results() {
    for nir in variants(CALLS, TargetId::Wdc65816Small) {
        let mir = mir65816::lower_program(&nir).unwrap();
        assert!(mir.native_abi.is_none());
        assert!(mir.task_switch_state.native_memory.is_none());
        let expected =
            [(0, 1), (1, 2), (3, 2), (5, 4)].map(|(offset, size)| Mir65816AbiHome::StackArgument {
                offset: ByteOffset::new(offset),
                size: ByteSize::new(size),
                alignment: ByteSize::ONE,
            });
        assert_eq!(
            routine(&mir, "Mixed")
                .frame
                .parameters
                .iter()
                .map(|p| p.incoming)
                .collect::<Vec<_>>(),
            expected
        );
        for op in routine(&mir, "Main")
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
        {
            if let Mir65816Op::Call { plan, .. } = op {
                assert!(plan.native.is_none());
                assert_eq!(plan.call_form, Mir65816CallForm::NearJsr);
                if plan.arguments.is_empty() {
                    assert_eq!(plan.outgoing_bytes, ByteSize::ZERO);
                } else {
                    assert_eq!(plan.arguments, expected);
                    assert_eq!(plan.outgoing_bytes.get(), 9);
                    assert_eq!(plan.result, Some(Mir65816AbiHome::AccumulatorAndX));
                }
            }
        }
    }
}

#[test]
fn aggregate_expansion_does_not_silently_qualify_a_public_aggregate_signature() {
    let source = "TYPE Pair=[CARD value] Pair original,result Pair FUNC Copy(Pair input) RETURN(input) PROC Main() result=Copy(original) RETURN";
    for nir in variants(source, TargetId::Wdc65816Native) {
        let signature = nir
            .routines
            .iter()
            .find(|r| r.name == "Copy")
            .unwrap()
            .signature
            .id;
        let mir = mir65816::lower_program(&nir).unwrap();
        let contract = mir.native_abi.unwrap();
        assert!(
            contract
                .unsupported_signatures
                .iter()
                .any(|entry| entry.signature == signature
                    && entry.reason == abi::AbiError::UnsupportedType)
        );
    }
}
