mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    nir::NirPromotionPolicy,
};
use actionc_vm68k_tests::Machine;

#[test]
fn promoted_loops_preserve_calls_conditional_definitions_and_zero_trip_paths() {
    let source = common::Source::new(
        "\
LONGINT ARRAY data(16)\n\
LONGINT result\n\
CARD count\n\
BYTE flag\n\
LONGINT FUNC Delta(LONGINT value) RETURN(value*3)\n\
LONGINT FUNC Sum(CARD n)\n\
  LONGINT POINTER p\n\
  CARD i\n\
  LONGINT total\n\
  IF flag THEN p=data ELSE p=data p==+4 FI\n\
  total=0 i=0\n\
  WHILE i<n DO\n\
    IF flag THEN total==+Delta(p^) ELSE total==-Delta(p^) FI\n\
    p==+4 i==+1\n\
  OD\n\
RETURN(total)\n\
PROC Main() result=Sum(count) RETURN\n",
    );
    let values = [
        0i32,
        1,
        -1,
        i32::MIN,
        i32::MAX,
        10000,
        -12345678,
        65535,
        12345,
        -98765,
        42,
        1,
        2,
        3,
        4,
        5,
    ];
    for promotion in [
        NirPromotionPolicy::Conservative,
        NirPromotionPolicy::NativeLoops,
    ] {
        let program = compile_file(
            &source.0,
            &NativeCompileOptions {
                promotion,
                ..Default::default()
            },
        )
        .unwrap();
        for count in [0, 1, 3, 7, 15] {
            for flag in [0, 1] {
                let mut vm = Machine::from_image(&program.image).unwrap();
                vm.write_array(
                    program.image.symbol("data").unwrap(),
                    &values.map(|v| v as u32),
                )
                .unwrap();
                vm.write_scalar(program.image.symbol("count").unwrap(), count)
                    .unwrap();
                vm.write_scalar(program.image.symbol("flag").unwrap(), flag)
                    .unwrap();
                vm.run(100_000).assert_completed();
                let start = usize::from(flag == 0);
                let expected =
                    values[start..start + count as usize]
                        .iter()
                        .fold(0i32, |sum, value| {
                            if flag == 1 {
                                sum.wrapping_add(value.wrapping_mul(3))
                            } else {
                                sum.wrapping_sub(value.wrapping_mul(3))
                            }
                        });
                assert_eq!(
                    vm.read_scalar(program.image.symbol("result").unwrap())
                        .unwrap(),
                    expected as u32,
                    "{promotion:?}/{count}/{flag}"
                );
                assert_eq!(
                    vm.read_array(program.image.symbol("data").unwrap())
                        .unwrap(),
                    values.map(|v| v as u32)
                );
            }
        }
    }
}
