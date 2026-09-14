mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
#[test]
fn narrow_signed_operands_are_extended_before_wide_computation() {
    let source = common::Source::new(
        "LONGINT total,normal,product,band INT narrow PROC Entry() normal=total+narrow total==+narrow product=LONGINT(3)*narrow band=LONGINT(-1) AND narrow RETURN",
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
        for value in [0u16, 1, 0x7fff, 0x8000, 0xffff] {
            let mut vm = Machine::from_image(&image).unwrap();
            vm.write_scalar(image.symbol("total").unwrap(), 0x12340000)
                .unwrap();
            vm.write_scalar(image.symbol("narrow").unwrap(), u32::from(value))
                .unwrap();
            vm.run(1000).assert_completed();
            let extended = i32::from(value as i16) as u32;
            for (name, expected) in [
                ("total", 0x12340000u32.wrapping_add(extended)),
                ("normal", 0x12340000u32.wrapping_add(extended)),
                ("product", extended.wrapping_mul(3)),
                ("band", extended),
            ] {
                assert_eq!(
                    vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                    expected,
                    "{optimize}/{value:x}/{name}"
                );
            }
        }
    }
}
