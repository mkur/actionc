use actionc::target::TargetId;
use actionc::{
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn options(target: TargetId) -> SemanticOptions {
    let mut options = SemanticOptions::modern().with_target(target);
    options.algebraic_types.variants = true;
    options
}

#[test]
fn variant_alternatives_have_canonical_overlapping_target_layouts() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let source = "TYPE Event=VARIANT [NONE KEY [BYTE code] MOVE [INT x,y]] Event current PROC Main() RETURN";
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, options(target)).unwrap();
        let variant = model.variants.types.values().next().unwrap();
        assert_eq!(
            variant
                .constructors
                .iter()
                .map(|c| c.id.tag)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let packed = target == TargetId::Atari6502;
        assert_eq!(variant.size, if packed { 5 } else { 6 });
        assert_eq!(variant.payload_offset, if packed { 1 } else { 2 });
        let layout = model
            .layout
            .record_for_owner(variant.identity.symbol.unwrap())
            .unwrap();
        assert_eq!(layout.size, variant.size);
        assert_eq!(layout.alignment, variant.alignment);
        let semir = semantic::ir::lower_program(&ast, &model);
        let nir = actionc::nir::lower_program(&semir);
        actionc::nir::verify_program(&nir).unwrap();
    }
}

#[test]
fn variants_are_modern_only_and_can_be_independently_gated() {
    let ast = parse(&tokenize("TYPE State=VARIANT [IDLE ACTIVE]").unwrap()).unwrap();
    let mut gated = SemanticOptions::modern();
    gated.algebraic_types.variants = false;
    for options in [SemanticOptions::default(), gated] {
        assert!(
            semantic::analyze_with_options(&ast, options)
                .unwrap_err()
                .iter()
                .any(|e| e.message.contains("not enabled"))
        );
    }
}

#[test]
fn variant_definition_errors_are_reported_before_lowering() {
    let too_many = (0..256)
        .map(|i| format!("C{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    for (definition, needle) in [
        ("".to_string(), "1..255"),
        (too_many, "1..255"),
        ("ONE one".to_string(), "duplicate VARIANT"),
        ("ONE [BYTE x,x]".to_string(), "duplicate"),
        ("ONE [BYTE ARRAY bytes(3)]".to_string(), "direct ARRAY"),
        ("ONE [Broken value]".to_string(), "cyclic"),
    ] {
        let ast =
            parse(&tokenize(&format!("TYPE Broken=VARIANT [{definition}]")).unwrap()).unwrap();
        let errors =
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(needle)),
            "{definition}: {errors:?}"
        );
    }
}

#[test]
fn constructors_snapshots_and_flat_matches_lower_end_to_end() {
    let source = r#"
TYPE Event=VARIANT [NONE KEY [BYTE code] MOVE [INT x,y]]
Event current
BYTE result
INT FUNC Read()
CASE current OF
WHEN Event.NONE THEN
  RETURN(0)
WHEN Event.KEY(code) THEN
  RETURN(code)
WHEN Event.MOVE(x,y) THEN
  RETURN(x+y)
ESAC
PROC Main()
  current=Event.MOVE(12,-3)
  LET saved=current
  current=Event.NONE
CASE saved OF
WHEN Event.MOVE(x,_) THEN
  result=BYTE(x)
ELSE
  result=255
ESAC
RETURN
"#;
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap();
    actionc::codegen::generate_semir_profile_with_origin(
        &semir,
        0x3000,
        actionc::codegen::CodegenProfile::Modern,
    )
    .unwrap();
    for optimized in [false, true] {
        let nir = if optimized {
            actionc::nir::optimize_program(&nir).unwrap()
        } else {
            nir.clone()
        };
        for runtime in [
            actionc::runtime::Runtime::ActionCart,
            actionc::runtime::Runtime::Standalone,
        ] {
            actionc::mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x3000,
                &actionc::mir6502::Mir6502Config::default(),
                runtime,
            )
            .unwrap();
        }
    }
}

#[test]
fn variant_value_and_pattern_rejections_are_semantic() {
    let prefix = "TYPE Event=VARIANT [NONE KEY [BYTE code]] TYPE Other=VARIANT [NONE]\nEvent current Other other BYTE result CARD address\n";
    for (body, needle) in [
        ("current=Event.KEY", "payload"),
        ("current=Event.NONE()", "payload"),
        ("current=Event.KEY(1,2)", "payload"),
        ("current=Other.NONE", "nominal"),
        ("current.code=1", "variant"),
        ("current.__variant_tag=1", "variant"),
        ("result=current", "variant"),
        ("result=BYTE(current)", "variant"),
        ("result=current+1", "variant"),
        ("IF current THEN result=1 FI", "variant"),
        (
            "CASE current OF\nWHEN Other.NONE THEN\nresult=1\nELSE\nresult=2\nESAC",
            "exact selector",
        ),
        (
            "CASE current OF\nWHEN Event.NONE THEN\nresult=1\nESAC",
            "non-exhaustive",
        ),
        (
            "CASE current OF\nWHEN Event.KEY(code) THEN\ncode=1\nELSE\nresult=2\nESAC",
            "immutable",
        ),
        (
            "CASE current OF\nWHEN Event.KEY(code) THEN\naddress=@code\nELSE\nresult=2\nESAC",
            "immutable",
        ),
        (
            "CASE current OF\nWHEN Event.KEY(1) THEN\nresult=1\nELSE\nresult=2\nESAC",
            "flat",
        ),
        (
            "CASE current OF\nWHEN Event.KEY(code) THEN\nresult=code\nWHEN Event.KEY(_) THEN\nresult=2\nELSE\nresult=3\nESAC",
            "duplicate",
        ),
        (
            "CASE current OF\nWHEN Event.KEY(code) THEN\nresult=code\nELSE\nresult=2\nESAC\nresult=code",
            "undefined",
        ),
        (
            "CASE current OF\nWHEN Event.KEY(code) THEN\nLET x=code\nELSE\nresult=2\nESAC",
            "BEGIN/END",
        ),
    ] {
        let source = format!("{prefix}PROC Main()\n{body}\nRETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern())
            .err()
            .unwrap_or_else(|| panic!("accepted {body}"));
        assert!(
            errors.iter().any(|e| e
                .message
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())),
            "{body}: {errors:?}"
        );
    }
    for declaration in [
        "Event current=$700",
        "Event current=[0]",
        "VOLATILE Event current",
        "TYPE Holder=[Event inner] Holder current=$700",
        "Event ARRAY entries(2)=$700",
    ] {
        let source =
            format!("TYPE Event=VARIANT [NONE KEY [BYTE code]] {declaration} PROC Main() RETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern())
            .err()
            .expect(declaration);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("variant-containing storage")),
            "{declaration}: {errors:?}"
        );
    }
}

#[test]
fn native_construction_and_automatic_zero_storage_use_target_layout() {
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let source = "TYPE Event=VARIANT [NONE KEY [BYTE code]]\nPROC Main()\nEvent local\nlocal=Event.KEY(7)\nRETURN";
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&nir).unwrap();
        let main = nir.routines.iter().find(|r| r.name == "Main").unwrap();
        assert!(
            main.locals
                .iter()
                .all(|local| local.duration == actionc::nir::NirStorageDuration::Automatic)
        );
        let first = main.blocks.first().unwrap().ops.first().unwrap();
        assert!(
            matches!(first, actionc::nir::NirOp::CopyBytes { .. }),
            "{first:?}"
        );
        assert!(
            nir.statics
                .iter()
                .any(|data| data.image.bytes.iter().all(|byte| *byte == 0))
        );
        let optimized = actionc::nir::optimize_program(&nir).unwrap();
        match target {
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&optimized).unwrap();
            }
            _ => {
                actionc::mir65816::lower_program(&optimized).unwrap();
            }
        }
    }
}

#[test]
fn fault_call_contract_cannot_be_weakened_or_resumed() {
    let source = "TYPE Event=VARIANT [NONE]\nEvent original,copy\nPROC Main() copy=original RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    actionc::nir::verify_program(&nir).unwrap();
    for mutation in 0..5 {
        let mut malformed = nir.clone();
        let block = malformed
            .routines
            .iter_mut()
            .flat_map(|r| &mut r.blocks)
            .find(|block| {
                block.ops.iter().any(|op| {
                    matches!(
                        op,
                        actionc::nir::NirOp::Call {
                            callee: actionc::nir::NirCallee::Fault(_),
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let actionc::nir::NirOp::Call { args, effects, .. } = block.ops.last_mut().unwrap() else {
            unreachable!()
        };
        match mutation {
            0 => args.push(actionc::nir::NirValue::ConstU8(1)),
            1 => effects.opaque = false,
            2 => effects.memory.writes = actionc::nir::NirMemoryAccess::None,
            3 => effects.may_call_external = false,
            _ => block.terminator = actionc::nir::NirTerminator::Return(None),
        }
        assert!(
            actionc::nir::verify_program(&malformed).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn qualified_constructors_and_patterns_share_imported_nominal_identity() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider=InMemorySourceProvider::default()
        .with_source(root.clone(),b"MODULE App USE Lib AS A USE Lib AS B A.Event current BYTE result PROC Main()\ncurrent=A.Event.KEY(9)\nCASE current OF\nWHEN B.Event.KEY(code) THEN\nresult=code\nELSE\nresult=0\nESAC\nRETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib.act"),b"MODULE Lib PUBLIC TYPE Event=VARIANT [NONE KEY [BYTE code]] ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model =
        semantic::analyze_compilation_with_options(&loaded, SemanticOptions::modern()).unwrap();
    assert_eq!(model.variants.types.len(), 1);
    let semir = semantic::ir::lower_compilation(&loaded, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap();
    actionc::nir::optimize_program(&nir).unwrap();
    actionc::codegen::generate_semir_profile_with_origin(
        &semir,
        0x3000,
        actionc::codegen::CodegenProfile::Modern,
    )
    .unwrap();
}
