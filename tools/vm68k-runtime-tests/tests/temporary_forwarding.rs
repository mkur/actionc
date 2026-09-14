mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    mir68k::materialize::Options,
};
use actionc_vm68k_tests::Machine;

#[test]
fn forwarding_preserves_calls_branches_widths_and_volatile_accesses() {
    let source = common::Source::new(
        "\
        LONGCARD seed,result,calls\n\
        CARD narrow\n\
        VOLATILE BYTE port=$2001\n\
        LONGCARD FUNC Bump(LONGCARD value)\n\
          calls==+1 port=BYTE(value)\n\
        RETURN(value*LONGCARD(65537)+calls)\n\
        PROC Main()\n\
          BYTE i\n\
          LONGCARD x,y\n\
          x=seed calls=0\n\
          FOR i=0 TO 12 DO\n\
            y=(x XOR LONGCARD(305419896))*(x+LONGCARD(3))\n\
            IF (y AND LONGCARD(128))#LONGCARD(0) THEN\n\
              x=Bump(y)+Bump(x)\n\
            ELSE x=(y RSH i)+LONGCARD(narrow) FI\n\
            narrow=CARD(x) port=BYTE(narrow)\n\
          OD\n\
          result=x+LONGCARD(port)\n\
        RETURN\n",
    );
    for optimize in [false, true] {
        let build = |forward_temporaries| {
            compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    codegen: Options {
                        forward_temporaries,
                        select_instructions: false,
                        relax_branches: false,
                        ..Options::conservative()
                    },
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let plain = build(false);
        let forwarded = build(true);
        let mut saved_steps = 0;
        for seed in [0, 1, 127, 128, 65535, 65536, 0x80000000, u32::MAX] {
            let execute = |program: &actionc::compiler::native::NativeCompiledProgram| {
                let mut vm = Machine::from_image(&program.image).unwrap();
                vm.cpu.mem.map(0x2000, &[0xa5; 4], true, false).unwrap();
                vm.cpu.mem.trace_range(0x2000..0x2004);
                vm.write_scalar(program.image.symbol("seed").unwrap(), seed)
                    .unwrap();
                vm.write_scalar(program.image.symbol("narrow").unwrap(), 0x8765)
                    .unwrap();
                let run = vm.run(100_000);
                run.assert_completed();
                let values: Vec<_> = ["seed", "result", "calls", "narrow"]
                    .iter()
                    .map(|name| vm.read_scalar(program.image.symbol(name).unwrap()).unwrap())
                    .collect();
                let trace = vm.cpu.mem.take_trace();
                (
                    values,
                    vm.cpu.mem.bytes(0x2000, 4).unwrap().to_vec(),
                    trace,
                    run.steps,
                )
            };
            let expected = execute(&plain);
            let actual = execute(&forwarded);
            assert_eq!(
                (&actual.0, &actual.1, &actual.2),
                (&expected.0, &expected.1, &expected.2),
                "{optimize}/{seed:x}"
            );
            assert!(actual.3 <= expected.3);
            saved_steps += expected.3 - actual.3;
        }
        assert!(saved_steps > 0);
        assert_eq!(
            plain
                .machine
                .routines
                .iter()
                .map(|r| &r.frame)
                .collect::<Vec<_>>(),
            forwarded
                .machine
                .routines
                .iter()
                .map(|r| &r.frame)
                .collect::<Vec<_>>()
        );
    }
}
