mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    mir68k::{machine::Instruction, materialize::Options},
};
use actionc_vm68k_tests::Machine;

#[test]
fn relaxed_branches_execute_both_arms_and_backedges_at_multiple_origins() {
    let source = common::Source::new(
        "\
        CARD input,result\n\
        CARD FUNC Count(CARD n)\n\
          CARD i,total\n\
          i=0 total=0\n\
          WHILE i<n DO\n\
            IF i<5 OR (i AND 1)=0 THEN total==+i ELSE total==-i FI\n\
            i==+1\n\
          OD\n\
        RETURN(total)\n\
        PROC Main() result=Count(input) RETURN\n",
    );
    for optimize in [false, true] {
        for origin in [0x10000, 0x23000] {
            let build = |relax_branches| {
                compile_file(
                    &source.0,
                    &NativeCompileOptions {
                        origin,
                        optimize,
                        codegen: Options {
                            relax_branches,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .unwrap()
            };
            let plain = build(false);
            let compact = build(true);
            assert!(compact.image.segments[0].bytes.len() < plain.image.segments[0].bytes.len());
            assert!(
                compact
                    .machine
                    .blocks
                    .iter()
                    .flat_map(|b| &b.instructions)
                    .any(|i| matches!(
                        i,
                        Instruction::BranchRelative {
                            condition: Some(_),
                            ..
                        }
                    ))
            );
            for n in [0u16, 1, 5, 6, 17, 127] {
                let expected = (0..n).fold(0u16, |sum, i| {
                    if i < 5 || i % 2 == 0 {
                        sum.wrapping_add(i)
                    } else {
                        sum.wrapping_sub(i)
                    }
                });
                let mut steps = Vec::new();
                for program in [&plain, &compact] {
                    let mut vm = Machine::from_image(&program.image).unwrap();
                    vm.write_scalar(program.image.symbol("input").unwrap(), u32::from(n))
                        .unwrap();
                    let run = vm.run(100_000);
                    run.assert_completed();
                    assert_eq!(
                        vm.read_scalar(program.image.symbol("result").unwrap())
                            .unwrap(),
                        u32::from(expected),
                        "{optimize}/{origin:x}/{n}"
                    );
                    steps.push(run.steps);
                }
                assert!(steps[1] <= steps[0]);
            }
        }
    }
}
