//! Shared aggregate snapshots, not aliases to their source storage.
use actionc::target::TargetId;
use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
};

#[test]
fn record_bindings_have_typed_capture_storage_on_all_targets() {
    let source = "TYPE Row=[INT number BYTE ARRAY bytes(17)] Row original,copy \
        PROC Main() LET saved=original copy=saved RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&program).unwrap();
        let capture = program
            .routines
            .iter()
            .flat_map(|r| &r.locals)
            .find(|local| local.purpose == nir::NirLocalPurpose::AggregateCapture)
            .unwrap();
        assert!(capture.layout.size.get() >= 19);
        assert!(matches!(capture.backing, nir::NirLocalBacking::Ordinary));
        let optimized = nir::optimize_program(&program).unwrap();
        match target {
            TargetId::Atari6502 => {
                actionc::mir6502::lower_program(&optimized).unwrap();
            }
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&optimized).unwrap();
            }
            _ => {
                actionc::mir65816::lower_program(&optimized).unwrap();
            }
        }
        let mut malformed = program;
        let capture = malformed
            .routines
            .iter_mut()
            .flat_map(|r| &mut r.locals)
            .find(|local| local.purpose == nir::NirLocalPurpose::AggregateCapture)
            .unwrap();
        capture.ty.kind = nir::NirTypeKind::Real;
        assert!(nir::verify_program(&malformed).is_err());
    }
}

#[test]
fn immutable_aggregate_subobjects_cannot_be_written_or_exposed() {
    for statement in [
        "saved.value=1",
        "saved.value==+1",
        "saved.bytes(1)=1",
        "saved.inner.value=1",
        "address=@saved",
        "address=@saved.value",
        "address=@saved.bytes(0)",
        "ptr=saved",
        "Use(saved)",
        "bytes=saved.bytes",
        "UseBytes(saved.bytes)",
        "address=CARD(saved)",
        "address=saved",
        "address=saved+1",
        "address=-saved",
        "BEGIN\nCARD alias=@saved.value\nEND",
        "BEGIN\nBYTE alias=saved.bytes(0)\nEND",
        "[ $AD saved ]",
    ] {
        let source = format!(
            "TYPE Inner=[BYTE value] TYPE Row=[BYTE value BYTE ARRAY bytes(3) Inner inner]\n\
            Row original Row POINTER ptr BYTE POINTER bytes CARD address\n\
            PROC Use(Row POINTER arg) RETURN PROC UseBytes(BYTE POINTER arg) RETURN\n\
            PROC Main()\nLET saved=original\n{statement}\nRETURN"
        );
        let ast =
            parse(&tokenize(&source).unwrap()).unwrap_or_else(|e| panic!("{statement}: {e:?}"));
        let errors =
            semantic::analyze_with_options(&ast, SemanticOptions::modern()).expect_err(statement);
        assert!(
            errors.iter().any(|e| e.message.contains("immutable LET")),
            "{statement}: {errors:?}"
        );
    }
}

#[test]
fn snapshot_pointees_remain_mutable_and_field_values_remain_copyable() {
    let source = "TYPE Node=[INT value Node POINTER next INT POINTER numbers] Node original \
        PROC Main() LET saved=original saved.next.value=7 saved.numbers(1)=-3 \
        LET number=CARD(saved.value) LET link=saved.next link.value=8 RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    nir::optimize_program(&program).unwrap();
}

#[test]
fn snapshots_do_not_enable_the_unimplemented_aggregate_call_abi() {
    let source = "TYPE Row=[BYTE value] Row original \
        PROC Take(Row arg) arg.value=7 RETURN \
        PROC Main() LET saved=original Take(saved) RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("by-value aggregate parameters"))
    );
}
