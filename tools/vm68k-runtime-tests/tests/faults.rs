mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    mir68k::{self, *},
    runtime_fault::RuntimeFault,
};
use actionc_vm68k_tests::{Machine, Outcome};
fn empty() -> Mir68kProgram {
    let ast =
        actionc::parser::parse(&actionc::lexer::tokenize("PROC Entry() RETURN").unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern()
            .with_target(actionc::target::TargetId::Motorola68000),
    )
    .unwrap();
    mir68k::lower_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
}
#[test]
fn typed_faults_are_distinct_terminal_outcomes_with_a_return_guard() {
    for reason in RuntimeFault::ALL {
        let mut mir = empty();
        mir.routines[0].blocks[0].ops = vec![Mir68kOp::Fault(reason)];
        mir.routines[0].blocks[0].terminator = Mir68kTerminator::Exit;
        let machine = materialize::materialize(&mir).unwrap();
        let image = image::link(&mir, &machine, 0x10000).unwrap();
        let mut vm = Machine::from_image(&image).unwrap();
        let run = vm.run(100);
        assert!(
            matches!(run.outcome,Outcome::RuntimeFault(actual) if actual==reason),
            "{run:#?}"
        );
        let guard = vm.cpu.pc;
        // Model an adapter mistakenly returning: execute the emitted guard
        // directly, bypassing the wrapper's independently latched fault.
        vm.cpu.execute(1000);
        assert_eq!(vm.cpu.pc, guard);
        assert!(matches!(vm.run(100).outcome,Outcome::RuntimeFault(actual) if actual==reason));
    }
}
#[test]
fn native_faults_cannot_have_continuations_or_replace_unexplained_exits() {
    let mut mir = empty();
    mir.routines[0].blocks[0].ops = vec![Mir68kOp::Fault(RuntimeFault::InvalidArgument)];
    assert!(verify::verify_contract(&mir).is_err());
    mir.routines[0].blocks[0].terminator = Mir68kTerminator::Exit;
    mir.routines[0].blocks[0]
        .ops
        .push(Mir68kOp::Fault(RuntimeFault::DivisionByZero));
    assert!(verify::verify_contract(&mir).is_err());
    mir.routines[0].blocks[0].ops.clear();
    assert!(
        materialize::materialize(&mir)
            .unwrap_err()
            .contains("terminal exit")
    );
}
#[test]
fn source_variant_validation_reaches_the_typed_native_fault_adapter() {
    let source = common::Source::new(
        "TYPE Event=VARIANT [NONE SOME [BYTE value]] Event current BYTE before,after,result PROC Entry() before=7\nCASE current OF\nWHEN Event.NONE THEN\nresult=1\nWHEN Event.SOME(saved) THEN\nresult=saved\nESAC\nafter=9 RETURN",
    );
    for optimize in [false, true] {
        let image = compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap()
        .image;
        let mut vm = Machine::from_image(&image).unwrap();
        vm.cpu
            .mem
            .write(image.symbol("current").unwrap().address().unwrap(), &[255])
            .unwrap();
        let run = vm.run(5000);
        assert!(
            matches!(
                run.outcome,
                Outcome::RuntimeFault(RuntimeFault::InvalidVariantTag)
            ),
            "{optimize}: {run:#?}"
        );
        for (name, value) in [("before", 7), ("after", 0), ("result", 0)] {
            assert_eq!(vm.read_scalar(image.symbol(name).unwrap()).unwrap(), value);
        }
    }
}
