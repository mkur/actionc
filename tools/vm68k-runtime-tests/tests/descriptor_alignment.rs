mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn descriptor_proofs_preserve_pre_entry_replacement_and_call_rebinding() {
    let source = common::Source::new(
        "LONGCARD ARRAY backing(8), grid(2,2)
LONGCARD initial,aligned,odd,localValue
PROC Rebind() grid=@backing(0)+1 RETURN
PROC Automatic()
  LONGCARD ARRAY localGrid(2,2)
  localValue=localGrid(0,0)
RETURN
PROC Main()
  initial=grid(0,0)
  backing(0)=$12345678 backing(1)=$ABCDEF00
  grid=backing aligned=grid(0,0)
  Rebind() odd=grid(0,0)
  Automatic()
RETURN
",
    );
    for optimize in [false, true] {
        let mut baseline = None;
        for pointer_alignment in [false, true] {
            let program = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    codegen: actionc::mir68k::materialize::Options {
                        pointer_alignment,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            let mut vm = Machine::from_image(&program.image).unwrap();
            vm.cpu
                .mem
                .map(0x4001, &0x87654321u32.to_be_bytes(), true, false)
                .unwrap();
            vm.cpu
                .mem
                .write(
                    program.image.symbol("grid").unwrap().address().unwrap(),
                    &0x4001u32.to_be_bytes(),
                )
                .unwrap();
            let run = vm.run(10_000);
            run.assert_completed();
            let actual = ["initial", "aligned", "odd", "localValue"]
                .map(|name| vm.read_scalar(program.image.symbol(name).unwrap()).unwrap());
            assert_eq!(actual, [0x87654321, 0x12345678, 0x345678AB, 0]);
            if let Some(steps) = baseline {
                assert!(run.steps < steps, "proven descriptor accesses must improve");
            } else {
                baseline = Some(run.steps);
            }
        }
    }
}
