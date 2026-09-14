mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    nir::NirPromotionPolicy,
};
use actionc_vm68k_tests::Machine;

#[test]
fn retained_values_survive_pressure_division_recursion_and_early_returns() {
    let source = common::Source::new(
        "\
LONGINT seed,result
CARD count
LONGINT FUNC Rec(CARD n)
  LONGINT saved
  IF n=0 THEN RETURN(11) FI
  saved=LONGINT(n)*13
RETURN(saved+Rec(n-1))
LONGINT FUNC Mix(LONGINT x CARD n) RETURN(x/7+Rec(n))
LONGINT FUNC Work(CARD n)
  LONGINT a,b,c,d,e,f,g
  INT short
  BYTE small
  CARD i
  a=seed b=seed+1 c=seed+2 d=seed+3 e=seed+4 f=seed+5 g=seed+6
  short=32760 small=250
  IF n=0 THEN RETURN(a) FI
  i=0
  WHILE i<n DO
    a==+Mix(b,i AND 3) b==+c c==+d d==+e e==+f f==+g g==+LONGINT(i)
    small==+37 short==-1234 i==+1
  OD
RETURN(a XOR b XOR c XOR d XOR e XOR f XOR g XOR LONGINT(small) XOR LONGINT(short))
PROC Main() result=Work(count) RETURN
",
    );
    for allocation in [false, true] {
        let mut options = NativeCompileOptions {
            promotion: NirPromotionPolicy::NativeLoops,
            ..Default::default()
        };
        options.codegen.register_allocation = allocation;
        let program = compile_file(&source.0, &options).unwrap();
        if allocation {
            assert!(
                program
                    .machine
                    .routines
                    .iter()
                    .any(|r| r.frame.saved_register_bytes.get() == 16),
                "pressure case must exercise the entire preserved-register pool"
            );
        }
        for seed in [0i32, 1, -1, i32::MIN, i32::MAX, -123456789] {
            for count in [0u32, 1, 3, 9, 30] {
                let mut values = std::array::from_fn::<_, 7, _>(|i| seed.wrapping_add(i as i32));
                let mut small = 250u8;
                let mut short = 32760i16;
                for i in 0..count {
                    let recursion = 11 + (1..=(i & 3)).sum::<u32>() as i32 * 13;
                    values[0] = values[0].wrapping_add((values[1] / 7).wrapping_add(recursion));
                    for j in 1..6 {
                        values[j] = values[j].wrapping_add(values[j + 1]);
                    }
                    values[6] = values[6].wrapping_add(i as i32);
                    small = small.wrapping_add(37);
                    short = short.wrapping_sub(1234);
                }
                let expected = if count == 0 {
                    seed
                } else {
                    values.into_iter().fold(0, |a, b| a ^ b) ^ i32::from(small) ^ i32::from(short)
                };
                let mut vm = Machine::from_image(&program.image).unwrap();
                vm.write_scalar(program.image.symbol("seed").unwrap(), seed as u32)
                    .unwrap();
                vm.write_scalar(program.image.symbol("count").unwrap(), count)
                    .unwrap();
                vm.run(1_000_000).assert_completed();
                assert_eq!(
                    vm.read_scalar(program.image.symbol("result").unwrap())
                        .unwrap(),
                    expected as u32,
                    "{allocation}/{seed}/{count}"
                );
            }
        }
    }
}
