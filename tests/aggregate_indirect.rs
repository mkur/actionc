use actionc::{
    lexer::tokenize,
    nir::{self, *},
    parser::parse,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

#[test]
fn qualified_routine_values_lower_as_addresses_not_aggregate_global_loads() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Lib AS L L.Pair original L.Pair FUNC POINTER callback(L.Pair input) PROC Main() callback=L.Copy original=callback(original) RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE Pair=[CARD value] PUBLIC Pair FUNC Copy(Pair input) RETURN(input) ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap();
        let semir = semantic::ir::lower_compilation(&loaded, &model);
        let raw = nir::lower_program(&semir);
        nir::verify_program(&raw).unwrap();
        for nir in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            nir::verify_program(&nir).unwrap();
            match target {
                TargetId::Atari6502 => {
                    actionc::mir6502::lower_program(&nir).unwrap();
                }
                TargetId::Motorola68000 => {
                    actionc::mir68k::lower_program(&nir).unwrap();
                }
                _ => {
                    actionc::mir65816::lower_program(&nir).unwrap();
                }
            }
        }
    }
}

fn options(target: TargetId) -> SemanticOptions {
    let mut options = SemanticOptions::modern().with_target(target);
    options.algebraic_types.indirect_aggregate_calls = true;
    options
}

#[test]
fn aggregate_callbacks_reject_equal_layout_nominal_mismatches_and_raw_abis() {
    for body in [
        "callback=Wrong",
        "callback=@Wrong",
        "callback=$9000",
        "Take(Wrong)",
        "Take(@Wrong)",
        "Take($9000)",
    ] {
        let source = format!(
            "TYPE One=[BYTE x] TYPE Two=[BYTE x] One FUNC POINTER callback(One input) \
            Two FUNC Wrong(Two input) RETURN(input) \
            PROC Take(One FUNC POINTER action(One input)) RETURN \
            PROC Main() {body} RETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).is_err(),
            "{body}"
        );
    }
    for initializer in ["[$9000]", "[@Wrong]", "[0]"] {
        let source = format!(
            "TYPE One=[BYTE x] TYPE Two=[BYTE x] \
            Two FUNC Wrong(Two input) RETURN(input) \
            PROC Main() One FUNC POINTER callback(One input)={initializer} RETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).is_err(),
            "{initializer}"
        );
    }
}

#[test]
fn aggregate_indirect_nir_rejects_forged_callable_signature_identity() {
    let ast = parse(&tokenize(NATIVE).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap();
    let mut program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    let callee = program
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
        .find_map(|op| match op {
            NirOp::Call {
                callee: NirCallee::Indirect { ty, .. },
                ..
            } => Some(ty),
            _ => None,
        })
        .unwrap();
    if let NirTypeKind::Callable { signature, .. } = &mut callee.kind {
        signature.0 += 1;
    }
    assert!(nir::verify_program(&program).is_err());
}

#[test]
fn aggregate_native_variants_keep_the_missing_error_adapter_explicit() {
    let source = "TYPE Event=VARIANT [NONE SOME [BYTE x]] Event FUNC Make() RETURN(Event.SOME(3)) PROC Main() LET saved=Make() RETURN";
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, options(target)).unwrap();
        let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&program).unwrap();
        let errors = if target == TargetId::Motorola68000 {
            format!(
                "{:?}",
                actionc::mir68k::lower_program(&program).unwrap_err()
            )
        } else {
            format!(
                "{:?}",
                actionc::mir65816::lower_program(&program).unwrap_err()
            )
        };
        assert!(errors.contains("native target Error adapter"), "{errors}");
    }
}

const NATIVE: &str = r#"
TYPE Pair=[BYTE x CARD y]
Pair original
Pair FUNC POINTER callback(Pair input CARD n)
Pair FUNC Copy(Pair input CARD n)
 input.y==+n
RETURN(input)
Pair FUNC Forward(Pair input CARD n)
RETURN(callback(input,n))
PROC Main()
 callback=Copy original.x=3 original.y=1000
 LET first=Forward(original,2)
 original=callback(first,3)
RETURN
"#;

#[test]
fn aggregate_native_calls_plan_target_sized_pointers_and_private_activation_homes() {
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let ast = parse(&tokenize(NATIVE).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, options(target)).unwrap();
        let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&program).unwrap();
        let copy = program.routines.iter().find(|r| r.name == "Copy").unwrap();
        assert_eq!(
            copy.signature.result.as_ref().unwrap().width.unwrap().get(),
            4
        );
        assert_eq!(copy.params[0].duration, NirStorageDuration::Automatic);
        for program in [program.clone(), nir::optimize_program(&program).unwrap()] {
            if target == TargetId::Motorola68000 {
                use actionc::mir68k::*;
                let mir = lower_program(&program).unwrap();
                let copy = mir.routines.iter().find(|r| r.name == "Copy").unwrap();
                assert_eq!(copy.frame.parameters.len(), 3);
                assert!(copy.frame.objects.iter().any(|o| matches!(
                    o.owner,
                    Mir68kFrameObjectOwner::Local(_)
                ) && o.size.get() == 4
                    && o.addressable));
                assert!(mir.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).any(|op| matches!(op,
                    Mir68kOp::Call { target: Mir68kCallTarget::Indirect(_, _), plan, .. }
                    if plan.arguments.len() == 3 && plan.result.is_none() && plan.net_stack_delta == 0
                        && matches!(plan.arguments[0], Mir68kAbiHome::StackArgument { size, .. } if size.get() == 4)
                        && matches!(plan.arguments[1], Mir68kAbiHome::StackArgument { size, .. } if size.get() == 4))));
            } else {
                use actionc::mir65816::*;
                let mir = lower_program(&program).unwrap();
                assert_eq!(
                    mir.code_pointer_width.get(),
                    if target == TargetId::Wdc65816Native {
                        3
                    } else {
                        2
                    }
                );
                let copy = mir.routines.iter().find(|r| r.name == "Copy").unwrap();
                assert_eq!(copy.frame.parameters.len(), 3);
                assert!(
                    copy.frame
                        .objects
                        .iter()
                        .any(|o| matches!(o.owner, Mir65816FrameObjectOwner::Local(_))
                            && o.size.get() == 4)
                );
                assert!(mir.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).any(|op| matches!(op,
                    Mir65816Op::Call { target: Mir65816CallTarget::Indirect(_, _), plan, .. }
                    if plan.arguments.len() == 3 && plan.result.is_none() && plan.net_stack_delta == 0
                        && matches!(plan.arguments[0], Mir65816AbiHome::StackArgument { size, .. } if size == mir.code_pointer_width)
                        && matches!(plan.arguments[1], Mir65816AbiHome::StackArgument { size, .. } if size == mir.code_pointer_width))));
            }
        }
    }
}
