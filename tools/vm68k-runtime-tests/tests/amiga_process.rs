mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, prepare_file},
    mir68k::{amiga, hunk, materialize, object, *},
    runtime_fault::RuntimeFault,
};
use actionc_vm68k_tests::{
    Outcome, STACK_BOTTOM, STACK_TOP,
    amiga::{Call, Os},
    hunk::File,
};

fn build(source: &str, optimize: bool, codegen: materialize::Options) -> hunk::Executable {
    let source = common::Source::new(source);
    let mir = prepare_file(
        &source.0,
        &NativeCompileOptions {
            optimize,
            ..Default::default()
        },
    )
    .unwrap()
    .mir;
    let machine = amiga::materialize(&mir, codegen).unwrap();
    assert!(
        !machine
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .any(|op| matches!(op, machine::Instruction::Trap(_)))
    );
    hunk::emit(&object::emit(&mir, &machine).unwrap()).unwrap()
}

const SOURCE: &str = r#"
LONGCARD divisor,before,after,result
LONGCARD FUNC Inner(LONGCARD n)
  LONGCARD ARRAY locals(64)
  locals(0)=n
RETURN(locals(0)/divisor)
LONGCARD FUNC Outer(LONGCARD n)
  LONGCARD saved
  saved=n+1
RETURN(Inner(saved)+saved)
PROC Main()
  before=7
  result=Outer(41)
  after=9
RETURN
"#;

#[test]
fn shell_startup_cleanup_and_nested_faults_restore_the_entry_state() {
    for (optimize, codegen) in [
        (false, Default::default()),
        (true, Default::default()),
        (true, materialize::Options::conservative()),
    ] {
        let executable = build(SOURCE, optimize, codegen);
        let file = File::parse(&executable.bytes).unwrap();
        for bases in [
            [0x10000, 0x40000, 0x60000],
            [0x30000, 0x70000, 0x50000],
            [0x70000, 0x50000, 0x20000],
        ] {
            let address = |name: &str| {
                executable
                    .object
                    .symbols
                    .iter()
                    .find(|s| s.name == name)
                    .unwrap()
                    .location
                    .resolve(&bases)
                    .unwrap()
            };
            // Every load has fresh, guarded state, including after failed launches.
            for case in 0..7 {
                let mut vm = file.load(&bases).unwrap();
                let mut os = Os {
                    fail_open: case == 1,
                    missing_output: case == 2,
                    ..Default::default()
                };
                if case == 4 {
                    os.write_results.extend([3, -1]);
                }
                if case == 5 {
                    os.write_results.push_back(0);
                }
                if case == 6 {
                    os.write_results.extend([1, 2, 3]);
                }
                os.install(&mut vm).unwrap();
                vm.cpu
                    .mem
                    .write(
                        address("divisor"),
                        &if case < 3 { 2u32 } else { 0 }.to_be_bytes(),
                    )
                    .unwrap();
                vm.cpu
                    .mem
                    .write(address("result"), &0x12345678u32.to_be_bytes())
                    .unwrap();
                vm.cpu.mem.trace_range(STACK_BOTTOM..STACK_TOP);
                let run = os.run(&mut vm, 100000);
                run.assert_completed();
                assert_eq!(run.registers[0], if case == 0 { 0 } else { 20 });
                assert_eq!(os.open_count, if case == 1 { 0 } else { 1 });
                assert_eq!(os.close_count, os.open_count);
                assert_eq!(
                    os.events
                        .iter()
                        .filter(|e| e.call == Call::OpenLibrary)
                        .count(),
                    1
                );
                for (name, expected) in [
                    ("before", if case == 1 || case == 2 { 0u32 } else { 7 }),
                    ("after", if case == 0 { 9 } else { 0 }),
                    ("result", if case == 0 { 63 } else { 0x12345678 }),
                ] {
                    assert_eq!(
                        vm.cpu.mem.bytes(address(name), 4).unwrap(),
                        expected.to_be_bytes(),
                        "case {case}/{name}"
                    );
                }
                let expected: &[u8] = match case {
                    3 | 6 => b"Action! DivisionByZero\n",
                    4 => b"Act",
                    _ => b"",
                };
                assert_eq!(os.output, expected, "case {case}");
                let trace = vm.cpu.mem.take_trace();
                let lowest = trace
                    .iter()
                    .filter(|(_, write)| *write)
                    .map(|(address, _)| *address)
                    .min()
                    .unwrap();
                assert!(
                    STACK_TOP - lowest < 65536,
                    "inherited 64 KiB stack exceeded"
                );
            }
        }
    }
}

#[test]
fn every_typed_fault_has_a_terminal_platform_adapter_and_cpu_errors_stay_distinct() {
    let source = common::Source::new("PROC Main() RETURN");
    let original = prepare_file(&source.0, &Default::default()).unwrap().mir;
    for reason in RuntimeFault::ALL {
        let mut mir = original.clone();
        mir.routines[0].blocks[0].ops = vec![Mir68kOp::Fault(reason)];
        mir.routines[0].blocks[0].terminator = Mir68kTerminator::Exit;
        let machine = amiga::materialize(&mir, Default::default()).unwrap();
        let executable = hunk::emit(&object::emit(&mir, &machine).unwrap()).unwrap();
        let mut vm = File::parse(&executable.bytes)
            .unwrap()
            .load(&[0x10000, 0x40000, 0x60000])
            .unwrap();
        let mut os = Os::default();
        os.install(&mut vm).unwrap();
        let run = os.run(&mut vm, 10000);
        run.assert_completed();
        assert_eq!(run.registers[0], 20);
        assert_eq!(os.output, format!("Action! {}\n", reason.name()).as_bytes());
        assert_eq!((os.open_count, os.close_count), (1, 1));
        let mut bad = machine.clone();
        bad.platform
            .routines
            .remove(&amiga::PlatformRoutineId::Fault(reason));
        assert!(object::emit(&mir, &bad).is_err());
    }
    let executable = build("PROC Main() DO OD RETURN", true, Default::default());
    let mut vm = File::parse(&executable.bytes)
        .unwrap()
        .load(&[0x10000, 0x40000, 0x60000])
        .unwrap();
    let mut os = Os::default();
    os.install(&mut vm).unwrap();
    assert!(matches!(
        os.run(&mut vm, 1000).outcome,
        Outcome::BudgetExhausted
    ));
    assert_eq!((os.open_count, os.close_count), (1, 0));
    let mut machine = amiga::materialize(&original, Default::default()).unwrap();
    let entry = machine.routines[0].entry;
    machine
        .blocks
        .iter_mut()
        .find(|b| b.id == entry)
        .unwrap()
        .instructions = vec![machine::Instruction::Trap(0)];
    let executable = hunk::emit(&object::emit(&original, &machine).unwrap()).unwrap();
    let mut vm = File::parse(&executable.bytes)
        .unwrap()
        .load(&[0x10000, 0x40000, 0x60000])
        .unwrap();
    let mut os = Os::default();
    os.install(&mut vm).unwrap();
    assert!(matches!(
        os.run(&mut vm, 10000).outcome,
        Outcome::Exception(_)
    ));
    assert_eq!((os.open_count, os.close_count), (1, 0));
}
