use actionc::{
    compiler::{CompileMode, CompileOptions, Runtime, compile_file},
    includes::{ModuleLoadOptions, load_compilation_from_provider},
    nir, semantic::{self, SemanticOptions},
    source::{InMemorySourceProvider, SourceOrigin},
    target::TargetId,
};
use std::path::Path;

#[test]
fn null_values_compile_through_public_classic_and_mir6502_routes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/null_values.act");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        }
    }
}

#[test]
fn null_values_keep_module_pointer_identity_through_each_backend() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Links AS L\n\
            L.Link POINTER cursor\nBYTE empty\n\
            PROC Main() cursor=NULL cursor=L.Identity(NULL) empty=NULL<>cursor RETURN\n\
            ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/links.act"), b"MODULE Links\n\
            PUBLIC TYPE Link=[Link POINTER next]\n\
            PUBLIC Link POINTER FUNC Identity(Link POINTER value)\n\
            IF value=NULL THEN RETURN(NULL) FI\nRETURN(value)\nENDMODULE".to_vec());
    let loaded = load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    for target in [TargetId::Atari6502, TargetId::Motorola68000,
                   TargetId::Wdc65816Small, TargetId::Wdc65816Native] {
        let model = semantic::analyze_compilation_with_options(
            &loaded, SemanticOptions::modern().with_target(target)).unwrap();
        let raw = nir::lower_program(&semantic::ir::lower_compilation(&loaded, &model));
        nir::verify_program(&raw).unwrap();
        for program in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            nir::verify_program(&program).unwrap();
            match target {
                TargetId::Atari6502 => { actionc::mir6502::lower_program(&program).unwrap(); }
                TargetId::Motorola68000 => { actionc::mir68k::lower_program(&program).unwrap(); }
                _ => { actionc::mir65816::lower_program(&program).unwrap(); }
            }
        }
    }
}
