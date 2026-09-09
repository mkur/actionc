use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

fn options(target: TargetId) -> SemanticOptions {
    let mut options = SemanticOptions::modern().with_target(target);
    options.algebraic_types.unions = true;
    options
}

#[test]
fn immutable_union_snapshots_reject_subobject_writes_and_address_escapes() {
    for statement in [
        "saved.word=1",
        "saved.word==+1",
        "saved.bytes(1)=1",
        "ptr=@saved",
        "ptr=saved",
        "address=@saved.bytes(0)",
        "BEGIN\nBYTE alias=saved.bytes(0)\nEND",
        "[ $AD saved ]",
    ] {
        let source = format!(
            "TYPE View=UNION [CARD word BYTE ARRAY bytes(3)] View original View POINTER ptr CARD address PROC Main()\nLET saved=original\n{statement}\nRETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let errors =
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains("immutable LET")),
            "{statement}: {errors:?}"
        );
    }
}

#[test]
fn union_value_boundaries_use_full_extent_and_existing_native_activation_homes() {
    let source = "TYPE View=UNION [CARD word BYTE ARRAY bytes(3)] View original \
        View FUNC POINTER callback(View input BYTE n) \
        View FUNC Copy(View input BYTE n) input.word==+n RETURN(input) \
        View FUNC Forward(View input BYTE n) RETURN(callback(input,n)) \
        PROC Main() callback=Copy LET saved=Forward(original,2) original=callback(saved,3) RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, options(target)).unwrap();
        let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&raw).unwrap();
        let size = if target == TargetId::Atari6502 { 3 } else { 4 };
        let copy = raw.routines.iter().find(|r| r.name == "Copy").unwrap();
        assert_eq!(
            copy.signature.result.as_ref().unwrap().width.unwrap().get(),
            size
        );
        assert_eq!(
            copy.params[0].duration,
            if target == TargetId::Atari6502 {
                nir::NirStorageDuration::RoutineStatic
            } else {
                nir::NirStorageDuration::Automatic
            }
        );
        let captures: Vec<_> = raw
            .routines
            .iter()
            .flat_map(|r| &r.locals)
            .filter(|l| l.purpose == nir::NirLocalPurpose::AggregateCapture)
            .collect();
        assert!(!captures.is_empty());
        assert!(captures.iter().all(|l| l.layout.size.get() == size));
        for program in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            nir::verify_program(&program).unwrap();
            match target {
                TargetId::Atari6502 => {
                    actionc::mir6502::lower_program(&program).unwrap();
                }
                TargetId::Motorola68000 => {
                    let mir = actionc::mir68k::lower_program(&program).unwrap();
                    let copy = mir.routines.iter().find(|r| r.name == "Copy").unwrap();
                    assert_eq!(copy.frame.parameters.len(), 3);
                    assert!(
                        copy.frame
                            .objects
                            .iter()
                            .any(|o| o.size.get() == size && o.addressable)
                    );
                }
                _ => {
                    let mir = actionc::mir65816::lower_program(&program).unwrap();
                    let copy = mir.routines.iter().find(|r| r.name == "Copy").unwrap();
                    assert_eq!(copy.frame.parameters.len(), 3);
                    assert!(copy.frame.objects.iter().any(|o| o.size.get() == size));
                }
            }
        }
    }
}

#[test]
fn union_callables_require_exact_nominal_signatures() {
    for statement in [
        "callback=Wrong",
        "callback=@Wrong",
        "callback=$9000",
        "Take(other)",
        "first=Wrong(first)",
    ] {
        let source = format!(
            "TYPE One=UNION [CARD word] TYPE Two=UNION [CARD word] One first Two other One FUNC POINTER callback(One input) Two FUNC Wrong(Two input) RETURN(input) PROC Take(One input) RETURN PROC Main() {statement} RETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).is_err(),
            "{statement}"
        );
    }
}
