mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    mir68k::{machine::Instruction, materialize::Options},
};
use actionc_vm68k_tests::Machine;

#[test]
fn direct_branches_match_all_integer_relations_and_keep_numeric_booleans() {
    for (ty, bits, signed) in [
        ("BYTE", 8, false),
        ("CARD", 16, false),
        ("INT", 16, true),
        ("LONGCARD", 32, false),
        ("LONGINT", 32, true),
    ] {
        let mut source = format!(
            "{ty} a,b\nBYTE truth,result\nBYTE ARRAY branches(6),numbers(6)\nPROC Main()\n"
        );
        for (index, relation) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
            source.push_str(&format!(
                "IF a{relation}b THEN branches({index})=1 ELSE branches({index})=0 FI\n"
            ));
        }
        for (index, relation) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
            source.push_str(&format!("numbers({index})=BYTE(a{relation}b)\n"));
        }
        source.push_str("IF truth THEN result=7 ELSE result=9 FI\nRETURN\n");
        let source = common::Source::new(&source);
        let mask = ((1u64 << bits) - 1) as u32;
        let half = 1u32 << (bits - 1);
        let values = [0, 1, half - 1, half, half + 1, mask - 1, mask];
        let number = |n: u32| {
            if signed && n & half != 0 {
                i64::from(n) - (1i64 << bits)
            } else {
                i64::from(n)
            }
        };
        for optimize in [false, true] {
            let mut counts = Vec::new();
            let mut steps = Vec::new();
            for control_flow in [false, true] {
                let program = compile_file(
                    &source.0,
                    &NativeCompileOptions {
                        optimize,
                        codegen: Options {
                            control_flow,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .unwrap();
                counts.push(
                    program
                        .machine
                        .blocks
                        .iter()
                        .flat_map(|b| &b.instructions)
                        .filter(|op| matches!(op, Instruction::SetCondition { .. }))
                        .count(),
                );
                let mut total = 0;
                for a in values {
                    for b in values {
                        let expected = [
                            number(a) == number(b),
                            number(a) != number(b),
                            number(a) < number(b),
                            number(a) <= number(b),
                            number(a) > number(b),
                            number(a) >= number(b),
                        ]
                        .map(u32::from);
                        let mut vm = Machine::from_image(&program.image).unwrap();
                        for (name, value) in [("a", a), ("b", b), ("truth", (a ^ b) & 255)] {
                            vm.write_scalar(program.image.symbol(name).unwrap(), value)
                                .unwrap();
                        }
                        let run = vm.run(20_000);
                        run.assert_completed();
                        for name in ["branches", "numbers"] {
                            assert_eq!(
                                vm.read_array(program.image.symbol(name).unwrap()).unwrap(),
                                expected,
                                "{ty}/{optimize}/{control_flow}/{a:x}/{b:x}/{name}"
                            );
                        }
                        assert_eq!(
                            vm.read_scalar(program.image.symbol("result").unwrap())
                                .unwrap(),
                            if (a ^ b) & 255 != 0 { 7 } else { 9 }
                        );
                        total += run.steps;
                    }
                }
                steps.push(total);
            }
            assert!(
                counts[1] > 0 && counts[1] < counts[0],
                "numeric booleans remain, branch booleans disappear: {ty}/{optimize}/{counts:?}"
            );
            assert!(steps[1] < steps[0]);
        }
    }
}
