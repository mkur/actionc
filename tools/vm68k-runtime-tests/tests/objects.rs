mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, prepare_file},
    mir68k::{materialize, object::*, *},
    nir,
    target::{ByteOffset, ByteSize},
};
use actionc_vm68k_tests::Machine;

fn object(optimize: bool) -> Object {
    let source = common::Source::new(
        r#"
LONGCARD result,zero
LONGCARD ARRAY table(3)=[10 20]
LONGCARD POINTER cell=[@zero]
LONGCARD FUNC Plus(LONGCARD value)
RETURN(value+1)
LONGCARD FUNC POINTER callback(LONGCARD value)=[@Plus]
PROC Main()
  cell^=callback(table(1))
  result=zero+table(0)+table(2)
RETURN
"#,
    );
    let mut mir = prepare_file(
        &source.0,
        &NativeCompileOptions {
            optimize,
            ..Default::default()
        },
    )
    .unwrap()
    .mir;
    let mut alias = mir.data.iter().find(|d| d.name == "zero").unwrap().clone();
    alias.placement = Mir68kDataPlacement::Alias {
        target: alias.id,
        offset: ByteOffset::ZERO,
    };
    alias.id = Mir68kDataId::Global(nir::SymbolId(900));
    alias.name = "zeroAlias".into();
    alias.zero_fill = ByteSize::ZERO;
    mir.data.push(alias);
    let machine = materialize::materialize(&mir).unwrap();
    emit(&mir, &machine).unwrap()
}

#[test]
fn one_object_executes_with_independent_code_data_and_bss_bases() {
    for optimize in [false, true] {
        let object = object(optimize);
        assert!(object.relocations.iter().any(|r| r.section == SectionId(0)));
        assert!(object.relocations.iter().any(|r| r.section != SectionId(0)
            && matches!(
                r.target,
                RelocationTarget::Location(Location::Section {
                    id: SectionId(0),
                    ..
                })
            )));
        for (start, step, reverse) in [
            (0x10000, 0x4000, false),
            (0x30000, 0x8000, true),
            (0x70000, 0x2000, false),
        ] {
            let mut bases: Vec<_> = (0..object.sections.len())
                .map(|i| start + i as u32 * step)
                .collect();
            if reverse {
                bases.reverse();
            }
            let image = object.link_sections(&bases, None).unwrap();
            assert_eq!(
                image.symbol("zero").unwrap().address(),
                image.symbol("zeroAlias").unwrap().address()
            );
            let mut vm = Machine::from_image(&image).unwrap();
            assert_eq!(vm.read_scalar(image.symbol("zero").unwrap()).unwrap(), 0);
            assert_eq!(
                vm.read_array(image.symbol("table").unwrap()).unwrap(),
                [10, 20, 0]
            );
            vm.run(10000).assert_completed();
            assert_eq!(vm.read_scalar(image.symbol("result").unwrap()).unwrap(), 31);
            assert_eq!(
                vm.read_scalar(image.symbol("zeroAlias").unwrap()).unwrap(),
                21
            );
        }
    }
}

#[test]
fn object_rejects_invalid_fixups_and_checks_addends_before_patching() {
    let original = object(true);
    let bases: Vec<_> = (0..original.sections.len())
        .map(|i| 0x10000 + i as u32 * 0x2000)
        .collect();
    for bad in 0..8 {
        let mut object = original.clone();
        match bad {
            0 => object.relocations.push(object.relocations[0].clone()),
            1 => object.relocations[0].section = SectionId(999),
            2 => object.relocations[0].offset = u32::MAX,
            3 => {
                object.relocations[0].target = RelocationTarget::Location(Location::Section {
                    id: SectionId(999),
                    offset: 0,
                })
            }
            4 => object.relocations[0].width = 3,
            5 => object.relocations[0].byte_index = Some(0),
            6 => object.relocations[0].addend = i64::MAX,
            _ => object.relocations[0].addend = -0x1000000,
        }
        assert!(object.link_sections(&bases, None).is_err(), "case {bad}");
    }
    let mut object = original;
    object.relocations[0].addend = -2;
    let relocation = &object.relocations[0];
    let RelocationTarget::Location(location) = relocation.target else {
        panic!()
    };
    let expected = location.resolve(&bases).unwrap() - 2;
    let image = object.link_sections(&bases, None).unwrap();
    let bytes =
        &image.segments[0].bytes[relocation.offset as usize..relocation.offset as usize + 4];
    assert_eq!(bytes, expected.to_be_bytes());
    object.relocations[0].target = RelocationTarget::ImageEnd;
    assert!(object.link_sections(&bases, None).is_err());
    assert!(object.link(0x10000).is_ok());
    let mut overlap = bases.clone();
    overlap[1] = overlap[0];
    assert!(object.link_sections(&overlap, Some(0x90000)).is_err());
}
