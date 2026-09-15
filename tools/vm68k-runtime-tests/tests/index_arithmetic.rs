mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn incremental_indexes_preserve_rebinding_in_rhs_calls_and_zero_trip_loops() {
    let source = common::Source::new(
        "CARD ARRAY grid(256,10),first(256,10),second(256,10)\n\
         CARD start\nVOLATILE BYTE port=$2101\n\
         CARD FUNC Bump(CARD value)\n\
           IF value=252 THEN grid=second FI\n\
           port==+1\nRETURN(value+17)\n\
         PROC Main() CARD row\n\
           grid=first\n\
           FOR row=start TO 254 DO grid(row,0)=Bump(row) OD\n\
         RETURN\n",
    );
    let mut traces = Vec::new();
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
        for start in [250u32, 254, 255] {
            let mut vm = Machine::from_image(&image).unwrap();
            vm.cpu.mem.map(0x2100, &[0; 4], true, false).unwrap();
            vm.cpu.mem.trace_range(0x2100..0x2104);
            vm.write_scalar(image.symbol("start").unwrap(), start)
                .unwrap();
            vm.run(100_000).assert_completed();
            let mut first = vec![0; 2560];
            let mut second = first.clone();
            for row in start..=254 {
                let out = if start <= 252 && row > 252 {
                    &mut second
                } else {
                    &mut first
                };
                out[row as usize * 10] = row + 17;
            }
            assert_eq!(
                vm.read_array(image.symbol("first").unwrap()).unwrap(),
                first,
                "{optimize}/{start}"
            );
            assert_eq!(
                vm.read_array(image.symbol("second").unwrap()).unwrap(),
                second,
                "{optimize}/{start}"
            );
            assert_eq!(
                vm.cpu.mem.bytes(0x2101, 1).unwrap(),
                &[255u32.saturating_sub(start) as u8]
            );
            traces.push(vm.cpu.mem.take_trace());
        }
    }
    assert_eq!(&traces[..3], &traces[3..]);
}

#[test]
fn incremental_signed_indexes_keep_sign_extension_and_large_offsets() {
    // Rebase by two rows so negative signed coordinates stay in allocated RAM.
    let source = common::Source::new(
        "BYTE ARRAY storage(160003),grid(4,CARD(40000))\n\
         INT start\n\
         PROC Main() INT row\n\
           grid=@storage(80000)\n\
           FOR row=start TO 1 DO grid(row,0)=BYTE(row+5) OD\n\
         RETURN\n",
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
        for start in [-2i16, 0, 1, 2] {
            let mut vm = Machine::from_image(&image).unwrap();
            vm.write_scalar(image.symbol("start").unwrap(), start as u16 as u32)
                .unwrap();
            vm.run(100_000).assert_completed();
            let mut expected = vec![0; 160003];
            for row in start..=1 {
                expected[(80000 + i32::from(row) * 40000) as usize] = (row + 5) as u32;
            }
            assert_eq!(
                vm.read_array(image.symbol("storage").unwrap()).unwrap(),
                expected,
                "{optimize}/{start}"
            );
        }
    }
}
