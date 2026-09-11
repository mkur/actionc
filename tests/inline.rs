use actionc::ast::{Item, Routine};
use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
use actionc::source::{InMemorySourceProvider, SourceOrigin};
use actionc::{lexer, mir6502, nir, parser, semantic};

fn routines(source: &str) -> Vec<Routine> {
    parser::parse(&lexer::tokenize(source).unwrap())
        .unwrap()
        .modules
        .into_iter()
        .flat_map(|m| m.items)
        .filter_map(|i| match i {
            Item::Routine(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[test]
fn inline_is_contextual_and_delimits_routines() {
    for source in [
        "INLINE BYTE FUNC First(BYTE value) RETURN(value) INLINE PROC Next() RETURN PROC Main() RETURN",
        "MODULE APP PUBLIC INLINE BYTE FUNC First(BYTE value) RETURN(value) INLINE PROC Next() RETURN PROC Main() RETURN ENDMODULE",
    ] {
        let rs = routines(source);
        assert_eq!(rs.len(), 3);
        assert!(rs[0].inline.requested());
        assert!(rs[1].inline.requested());
        assert!(!rs[2].inline.requested());
        assert!(rs[0].inline.span.is_some());
    }
    for source in [
        "BYTE INLINE PROC Main() INLINE=1 RETURN",
        "BYTE FUNC INLINE(BYTE value) RETURN(value) PROC Main() INLINE(1) RETURN",
        "MODULE APP BYTE INLINE PUBLIC BYTE FUNC Echo(BYTE INLINE) RETURN(INLINE) PROC Main() INLINE=Echo(1) RETURN ENDMODULE",
    ] {
        assert!(routines(source).iter().all(|r| !r.inline.requested()));
    }
    assert!(
        routines("inline longint func W(longint n) RETURN(n)")[0]
            .inline
            .requested()
    );
}

#[test]
fn inline_rejects_invalid_declarations() {
    for (source, expected) in [
        ("INLINE INLINE BYTE FUNC F() RETURN(1)", "duplicate INLINE"),
        (
            "INLINE BYTE value PROC Main() RETURN",
            "INLINE must precede",
        ),
        (
            "MODULE APP PUBLIC INLINE EXTERNAL PROC F() ENDMODULE",
            "INLINE cannot qualify an EXTERNAL",
        ),
        (
            "MODULE APP PUBLIC EXTERNAL INLINE PROC F() ENDMODULE",
            "INLINE cannot qualify an EXTERNAL",
        ),
        (
            "MODULE APP INLINE PUBLIC PROC F() RETURN ENDMODULE",
            "INLINE must precede",
        ),
        (
            "MODULE APP PROC Main() RETURN PUBLIC INLINE EXTERNAL PROC F() ENDMODULE",
            "INLINE cannot qualify an EXTERNAL",
        ),
        (
            "MODULE APP PROC Main() RETURN PUBLIC INLINE INLINE PROC F() RETURN ENDMODULE",
            "duplicate INLINE",
        ),
    ] {
        let errors = parser::parse(&lexer::tokenize(source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn imported_inline_hint_survives_verified_ir_and_optimization() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE APP USE LIB.MATH AS M BYTE input=$600,output=$601 PROC Main() output=M.INLINE(input) RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib/math.act"), b"MODULE LIB.MATH PUBLIC INLINE BYTE FUNC INLINE(BYTE value) RETURN(value XOR $55) ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = semantic::analyze_compilation(&loaded).unwrap();
    let sem = semantic::ir::lower_compilation(&loaded, &model);
    assert!(semantic::ir::format_program(&sem).contains("inline prefer"));
    let raw = nir::lower_program(&sem);
    nir::verify_program(&raw).unwrap();
    let optimized = nir::optimize_program(&raw).unwrap();
    for program in [&raw, &optimized] {
        assert_eq!(
            program
                .routines
                .iter()
                .filter(|r| r.inline.requested())
                .count(),
            1
        );
        assert!(nir::format_program(program).contains("inline prefer"));
        let mir = mir6502::lower_program(program).unwrap();
        assert_eq!(
            mir.routines.iter().filter(|r| r.inline.requested()).count(),
            1
        );
        assert!(mir6502::format_program(&mir).contains("inline prefer"));
    }
    let mut invalid = raw;
    invalid
        .routines
        .iter_mut()
        .find(|r| r.inline.requested())
        .unwrap()
        .entry
        .external = true;
    assert!(nir::verify_program(&invalid).unwrap_err().iter().any(|e| {
        e.message
            .contains("external routine cannot request inlining")
    }));
}

#[test]
fn annotated_routines_compile_across_modes_and_runtimes() {
    use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
    let dir = std::env::temp_dir().join(format!("actionc-inline-modes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.act");
    let source = "MODULE APP BYTE input=$600,output=$601 PUBLIC INLINE BYTE FUNC Map(BYTE value) RETURN(value XOR $55) PROC Main() output=Map(input) RETURN ENDMODULE";
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            std::fs::write(&path, source).unwrap();
            let hinted =
                compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
            if mode != CompileMode::Mir6502 {
                std::fs::write(&path, source.replace("INLINE ", "")).unwrap();
                let ordinary =
                    compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime))
                        .unwrap();
                assert_eq!(
                    hinted.object_bytes(),
                    ordinary.object_bytes(),
                    "{mode:?}/{runtime:?}"
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn optimized_inline_nir(source: &str) -> nir::NirProgram {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze(&ast).unwrap();
    let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&raw).unwrap();
    nir::optimize_program(&raw).unwrap()
}

#[test]
fn requested_private_scratch_uses_existing_definite_assignment_promotion() {
    for (ty, body) in [
        (
            "BYTE",
            "BYTE scratch IF value THEN scratch=1 ELSE scratch=2 FI RETURN(scratch)",
        ),
        (
            "INT",
            "INT scratch IF value THEN scratch=INT(value) ELSE scratch=-1 FI RETURN(scratch)",
        ),
        (
            "LONGINT",
            "LONGINT scratch IF value THEN scratch=LONGINT(value) ELSE scratch=LONGINT(-1) FI RETURN(scratch)",
        ),
        (
            "LONGINT",
            "LONGINT first,second first=LONGINT(value) second=first+1 RETURN(second)",
        ),
    ] {
        let source = format!(
            "BYTE input {ty} output INLINE {ty} FUNC Map(BYTE value) {body} PROC Main() output=Map(input) RETURN"
        );
        let program = optimized_inline_nir(&source);
        assert!(
            program.routines[0].locals.is_empty(),
            "{}",
            nir::format_program(&program)
        );
        mir6502::lower_program(&program).unwrap();
    }
}

#[test]
fn inline_hint_does_not_promote_persistent_initialized_addressed_or_volatile_homes() {
    for body in [
        "BYTE scratch scratch==+1 RETURN(scratch)",
        "BYTE scratch=[7] scratch==+1 RETURN(scratch)",
        "BYTE scratch IF value THEN scratch=value FI RETURN(scratch)",
        "VOLATILE BYTE scratch scratch=value RETURN(scratch)",
        "BYTE scratch=$700 scratch=value RETURN(scratch)",
        "BYTE scratch BYTE POINTER p p=@scratch p^=value RETURN(scratch)",
    ] {
        let source = format!(
            "BYTE input,output INLINE BYTE FUNC Map(BYTE value) {body} PROC Main() output=Map(input) RETURN"
        );
        let program = optimized_inline_nir(&source);
        assert!(
            !program.routines[0].locals.is_empty(),
            "{source}\n{}",
            nir::format_program(&program)
        );
    }
}
