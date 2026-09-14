use actionc::{
    lexer,
    mir68k::{self, hunk, materialize, object},
    nir, parser,
    semantic::{self, SemanticOptions},
    target::TargetId,
};
use actionc_vm68k_tests::{Machine, hunk::File};

fn compile(source: &str, optimized: bool) -> object::Object {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            multidimensional_arrays: true,
            ..SemanticOptions::modern().with_target(TargetId::Motorola68000)
        },
    )
    .unwrap_or_else(|e| panic!("{source}\n{e:#?}"));
    let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&raw).unwrap();
    let program = if optimized {
        nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    let mir = mir68k::lower_program(&program).unwrap();
    let options = if optimized {
        materialize::Options::default()
    } else {
        materialize::Options::conservative()
    };
    object::emit(
        &mir,
        &materialize::materialize_with_options(&mir, options).unwrap(),
    )
    .unwrap()
}

#[test]
fn multidimensional_bases_order_shape_and_hunk_relocations() {
    let source = "CARD ARRAY grid(2,3)=[1 2 3 4 5 6],other(3,2)=[10 20 30 40 50 60] \
        CARD observed,updated,tail CARD POINTER saved=[@grid(1,2)] \
        BYTE FUNC Row() grid=other RETURN(1) \
        PROC Main() observed=grid(Row(),2) grid(1,2)=99 updated=other(2,1) tail=saved^ RETURN";
    for optimized in [false, true] {
        let object = compile(source, optimized);
        for origin in [0x10000, 0x30000] {
            let image = object.link(origin).unwrap();
            let mut vm = Machine::from_image(&image).unwrap();
            vm.run(100_000).assert_completed();
            for (name, value) in [("observed", 6), ("updated", 99), ("tail", 6)] {
                assert_eq!(
                    vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                    value,
                    "{name}/{optimized}"
                );
            }
            assert_eq!(
                vm.read_array(image.symbol("grid").unwrap()).unwrap(),
                [10, 20, 30, 40, 50, 99]
            );
        }
        let executable = hunk::emit(&hunk::entry_thunk(&object).unwrap()).unwrap();
        let file = File::parse(&executable.bytes).unwrap();
        for bases in [[0x10000, 0x30000, 0x50000], [0x60000, 0x20000, 0x40000]] {
            let mut vm = file.load(&bases[..file.segments().len()]).unwrap();
            vm.run(100_000).assert_completed();
            for (name, value) in [("observed", 6u16), ("updated", 99), ("tail", 6)] {
                let symbol = executable
                    .object
                    .symbols
                    .iter()
                    .find(|s| s.name == name)
                    .unwrap();
                assert_eq!(
                    vm.cpu
                        .mem
                        .bytes(symbol.location.resolve(&bases).unwrap(), 2)
                        .unwrap(),
                    value.to_be_bytes()
                );
            }
        }
    }
}

#[test]
fn multidimensional_native_large_offsets_and_recursive_local_backing() {
    let source = "BYTE ARRAY big(2,65537) BYTE row LONGCARD column LONGINT result CARD localResult \
        CARD FUNC Recur(BYTE depth) CARD ARRAY scratch(2,3)=[7] CARD child \
        scratch(1,2)=depth IF depth>0 THEN child=Recur(depth-1) ELSE child=0 FI RETURN(scratch(1,2)+child) \
        PROC Main() row=1 column=65536 big(row,column)=91 result=big(1,65536) localResult=Recur(4) RETURN";
    for optimized in [false, true] {
        let image = compile(source, optimized).link(0x10000).unwrap();
        let mut vm = Machine::from_image(&image).unwrap();
        vm.run(200_000).assert_completed();
        assert_eq!(vm.read_scalar(image.symbol("result").unwrap()).unwrap(), 91);
        assert_eq!(
            vm.read_scalar(image.symbol("localResult").unwrap())
                .unwrap(),
            10
        );
        let data = vm.read_array(image.symbol("big").unwrap()).unwrap();
        assert_eq!(data.len(), 131074);
        assert_eq!(data[131073], 91);
        assert!(data[..131073].iter().all(|v| *v == 0));
    }
}

#[test]
fn multidimensional_native_inline_records_and_fixed_address_descriptors() {
    let source = "TYPE Tile=[BYTE tag LONGINT ARRAY pixels(2,3)] Tile first=$5001,second=$5801 Tile POINTER p \
        BYTE calls LONGINT result,guard CARD localResult \
        BYTE FUNC Row() calls==+1 p=second RETURN(1) \
        BYTE FUNC Column() calls==+10 RETURN(2) \
        LONGINT FUNC Value() calls==+100 first.pixels(1,2)=40 RETURN(2) \
        PROC Main() CARD ARRAY fixed(2,3)=$6001,other(3,2)=[9] \
        p=first first.pixels(1,2)=3 second.pixels(1,2)=10 \
        p.pixels(Row(),Column())==+Value() result=first.pixels(1,2) guard=second.pixels(1,2) \
        fixed(1,2)=123 localResult=fixed(1,2) fixed=other fixed(1,2)=77 localResult==+other(2,1) RETURN";
    for optimized in [false, true] {
        let image = compile(source, optimized).link(0x10000).unwrap();
        let mut vm = Machine::from_image(&image).unwrap();
        vm.cpu
            .mem
            .map(0x5000, &vec![0xA5; 0x1100], true, false)
            .unwrap();
        vm.run(200_000).assert_completed();
        for (name, value) in [
            ("calls", 111),
            ("result", 42),
            ("guard", 10),
            ("localResult", 200),
        ] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                value,
                "{name}/{optimized}"
            );
        }
        assert_eq!(vm.cpu.mem.bytes(0x5001, 1).unwrap(), [0xA5]);
        assert_eq!(vm.cpu.mem.bytes(0x600B, 2).unwrap(), 123u16.to_be_bytes());
    }
}

#[test]
fn multidimensional_native_mutable_byte_wide_and_record_arrays() {
    let source = "TYPE Item=[BYTE tag CARD number] Item ARRAY items(2,3),replacement(3,2) \
        BYTE ARRAY bytes(2,3)=[7],newBytes(3,2)=[9] LONGINT ARRAY wide(2,3)=[-7],newWide(3,2)=[-9] \
        CARD result BYTE zero LONGINT value \
        PROC Main() BYTE ARRAY scratch(2,3)=[11],localOther(3,2)=[13] \
        zero=bytes(1,2)+scratch(1,2) bytes=newBytes bytes(1,2)=21 \
        scratch=localOther scratch(1,2)=22 items=replacement items(1,2).number=1234 \
        result=replacement(2,1).number+newBytes(2,1)+localOther(2,1) \
        wide=newWide wide(1,2)=-123456 value=newWide(2,1) RETURN";
    for optimized in [false, true] {
        let image = compile(source, optimized).link(0x10000).unwrap();
        let mut vm = Machine::from_image(&image).unwrap();
        vm.run(200_000).assert_completed();
        for (name, value) in [("result", 1277), ("zero", 0), ("value", -123456i32 as u32)] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                value,
                "{name}/{optimized}"
            );
        }
    }
}

#[test]
fn multidimensional_native_volatile_coordinates_are_read_once_before_rhs() {
    let source = "VOLATILE BYTE row=$6100,column=$6101 CARD ARRAY grid(2,3) CARD result \
        CARD FUNC Value() row=0 column=0 grid(1,2)=40 RETURN(2) \
        PROC Main() grid(row,column)==+Value() result=grid(1,2) RETURN";
    for optimized in [false, true] {
        let image = compile(source, optimized).link(0x10000).unwrap();
        let mut vm = Machine::from_image(&image).unwrap();
        vm.cpu.mem.map(0x6100, &[1, 2], true, false).unwrap();
        vm.cpu.mem.trace_range(0x6100..0x6102);
        vm.run(100_000).assert_completed();
        assert_eq!(vm.read_scalar(image.symbol("result").unwrap()).unwrap(), 42);
        assert_eq!(
            vm.cpu.mem.take_trace(),
            [
                (0x6100, false),
                (0x6101, false),
                (0x6100, true),
                (0x6101, true)
            ]
        );
    }
}
