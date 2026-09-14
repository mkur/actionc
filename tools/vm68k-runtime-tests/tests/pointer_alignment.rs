mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn pointer_alignment_preserves_odd_accesses_calls_joins_and_volatile_traces() {
    let source = common::Source::new(
        "\
LONGCARD ARRAY data(16)\n\
BYTE choose\n\
LONGCARD answer,unaligned,observed\n\
VOLATILE BYTE device=$4001\n\
PROC Bump(LONGCARD POINTER p) p^==+1 RETURN\n\
PROC Work()\n\
  LONGCARD POINTER p\n\
  CARD i\n\
  p=data\n\
  FOR i=0 TO 7 DO p^=LONGCARD(i)+100 p==+4 OD\n\
  p=data answer=0\n\
  FOR i=0 TO 7 DO answer==+p^ p==+4 OD\n\
  p=data p==+1 p^=$12345678 Bump(p) unaligned=p^\n\
  IF choose THEN p=data ELSE p=data p==+1 FI\n\
  observed=p^\n\
  device=$10 answer==+LONGCARD(device)\n\
RETURN\n\
PROC Main() Work() RETURN\n",
    );
    for optimize in [false, true] {
        let mut results: Vec<(([u32; 3], Vec<u32>, Vec<(u32, bool)>), u64)> = Vec::new();
        for enabled in [false, true] {
            let program = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    codegen: actionc::mir68k::materialize::Options {
                        pointer_alignment: enabled,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            for choose in [0, 1] {
                let mut vm = Machine::from_image(&program.image).unwrap();
                vm.cpu.mem.map(0x4000, &[0; 8], true, false).unwrap();
                vm.write_scalar(program.image.symbol("choose").unwrap(), choose)
                    .unwrap();
                vm.cpu.mem.trace_range(0x4001..0x4005);
                let run = vm.run(50_000);
                run.assert_completed();
                let actual = ["answer", "unaligned", "observed"]
                    .map(|name| vm.read_scalar(program.image.symbol(name).unwrap()).unwrap());
                assert_eq!(
                    actual,
                    [
                        828 + 0x10,
                        0x12345679,
                        if choose == 0 { 0x12345679 } else { 0x00123456 }
                    ]
                );
                let result = (
                    actual,
                    vm.read_array(program.image.symbol("data").unwrap())
                        .unwrap(),
                    vm.cpu.mem.take_trace(),
                );
                if enabled {
                    assert_eq!(result, results[choose as usize].0);
                    assert!(
                        run.steps < results[choose as usize].1,
                        "aligned loop must improve"
                    );
                } else {
                    results.push((result, run.steps));
                }
            }
        }
    }
}

#[test]
fn mutable_descriptor_and_unknown_parameter_can_point_to_odd_memory() {
    let source = common::Source::new(
        "\
LONGCARD ARRAY values(2)=[1 2]\n\
LONGCARD answer,called\n\
LONGCARD FUNC Read(LONGCARD POINTER p) RETURN(p^)\n\
PROC Main() answer=values(0) called=Read(values) RETURN\n",
    );
    for optimize in [false, true] {
        let program = compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap();
        let mut vm = Machine::from_image(&program.image).unwrap();
        vm.cpu
            .mem
            .map(
                0x4000,
                &[0, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 0],
                true,
                false,
            )
            .unwrap();
        // Change the actual descriptor before entry; its initializer is not a
        // permanent promise about the pointer loaded by Main or Read.
        vm.cpu
            .mem
            .write(
                program.image.symbol("values").unwrap().address().unwrap(),
                &0x4001u32.to_be_bytes(),
            )
            .unwrap();
        vm.run(10_000).assert_completed();
        for name in ["answer", "called"] {
            assert_eq!(
                vm.read_scalar(program.image.symbol(name).unwrap()).unwrap(),
                0x12345678
            );
        }
    }
}
