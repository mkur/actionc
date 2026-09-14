mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn initialized_array_reads_writes_and_decay_use_the_element_storage() {
    let source = common::Source::new(
        "\
        INT ARRAY narrow(3)=[-32768 -1 123]\n\
        CARD ARRAY unsigned(2)=[32768 65535]\n\
        LONGINT ARRAY wide(3)=[-136 7 -2147483648]\n\
        BYTE ARRAY small(3)=[1 128 255]\n\
        LONGINT first,last,passed,product\n\
        CARD index=[1],u\n\
        BYTE b\n\
        LONGINT FUNC Read(LONGINT ARRAY a CARD i) RETURN(a(i))\n\
        PROC Main()\n\
          first=narrow(0) last=wide(2) passed=Read(wide,index)\n\
          product=LONGINT(32)*wide(0)\n\
          narrow(index)=-23 wide(index)=-45\n\
          u=unsigned(index) b=small(2)\n\
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
        let mut vm = Machine::from_image(&image).unwrap();
        vm.run(10_000).assert_completed();
        for (name, value) in [
            ("first", (-32768i32) as u32),
            ("last", 0x80000000),
            ("passed", 7),
            ("product", (-4352i32) as u32),
            ("u", 65535),
            ("b", 255),
        ] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                value,
                "{optimize}/{name}"
            );
        }
        assert_eq!(
            vm.read_array(image.symbol("narrow").unwrap()).unwrap(),
            [32768, (-23i16) as u16 as u32, 123]
        );
        assert_eq!(
            vm.read_array(image.symbol("wide").unwrap()).unwrap(),
            [(-136i32) as u32, (-45i32) as u32, 0x80000000]
        );
    }
}

#[test]
fn initialized_automatic_arrays_receive_fresh_backing_on_each_call() {
    let source = common::Source::new(
        "\
        LONGINT first,second\n\
        LONGINT FUNC Compute(LONGINT delta)\n\
          LONGINT ARRAY a(2)=[-136 7]\n\
          a(0)==+delta\n\
        RETURN(a(0)*a(1))\n\
        PROC Main() first=Compute(1) second=Compute(2) RETURN\n",
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
        let mut vm = Machine::from_image(&image).unwrap();
        vm.run(10_000).assert_completed();
        for (name, value) in [("first", -945i32), ("second", -938)] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                value as u32,
                "{optimize}/{name}"
            );
        }
    }
}

#[test]
fn named_record_pointers_access_pointees_and_preserve_other_arguments() {
    let source = common::Source::new(
        "\
        TYPE Pair=[LONGINT left,right]\n\
        Pair data\n\
        Pair POINTER ptr\n\
        LONGINT first,second\n\
        LONGINT FUNC Update(Pair POINTER p LONGINT extra)\n\
          p.left==+extra p.right==+1\n\
        RETURN(p.left+p.right+extra)\n\
        PROC Main()\n\
          ptr=@data ptr.left=-100 ptr.right=7\n\
          first=Update(ptr,30) second=Update(@data,40)\n\
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
        let mut vm = Machine::from_image(&image).unwrap();
        vm.run(10_000).assert_completed();
        for (name, value) in [("first", -32i32), ("second", 19)] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                value as u32,
                "{optimize}/{name}"
            );
        }
        assert_eq!(
            vm.cpu
                .mem
                .bytes(image.symbol("data").unwrap().address().unwrap(), 8)
                .unwrap(),
            [0xff, 0xff, 0xff, 0xe2, 0, 0, 0, 9]
        );
    }
}
