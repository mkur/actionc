mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, prepare_file},
    mir68k::{hunk, materialize, object::*, *},
    nir,
    target::{ByteOffset, ByteSize},
};
use actionc_vm68k_tests::hunk::File;

fn fixture() -> Object {
    Object {
        entry: Location::Section {
            id: SectionId(0),
            offset: 0,
        },
        sections: vec![
            Section {
                id: SectionId(0),
                bytes: vec![0x4e, 0xf9, 0, 0, 0, 0, 0x4e, 0x75, 0x4e, 0x71, 0x4e, 0x71],
                zero_fill: 0,
                size: 12,
                alignment: 2,
                writable: false,
                executable: true,
                fixed_address: None,
            },
            Section {
                id: SectionId(1),
                bytes: vec![0; 4],
                zero_fill: 0,
                size: 4,
                alignment: 2,
                writable: true,
                executable: false,
                fixed_address: None,
            },
            Section {
                id: SectionId(2),
                bytes: vec![],
                zero_fill: 8,
                size: 8,
                alignment: 2,
                writable: true,
                executable: false,
                fixed_address: None,
            },
        ],
        relocations: vec![
            Relocation {
                section: SectionId(0),
                offset: 2,
                width: 4,
                byte_index: None,
                target: RelocationTarget::Location(Location::Section {
                    id: SectionId(0),
                    offset: 6,
                }),
                addend: 0,
            },
            Relocation {
                section: SectionId(1),
                offset: 0,
                width: 4,
                byte_index: None,
                target: RelocationTarget::Location(Location::Section {
                    id: SectionId(2),
                    offset: 0,
                }),
                addend: 0,
            },
        ],
        symbols: vec![],
    }
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|n| n.to_be_bytes()).collect()
}
// Specified directly from RKM AmigaDOS (2024) §11.2, not computed by a writer.
const FIXTURE: &[u32] = &[
    1011, 0, 3, 0, 2, 3, 1, 2, // allocation table
    1001, 3, 0x4ef90000, 0x00064e75, 0x4e714e71, // JMP CODE+6; RTS; padding
    1004, 1, 0, 2, 0, 1010, 1002, 1, 0, 1004, 1, 2, 0, 0, 1010, 1003, 2, 1010,
];

#[test]
fn writer_and_independent_reader_match_specified_bytes() {
    let expected = bytes(FIXTURE);
    let actual = hunk::emit(&fixture()).unwrap();
    assert_eq!(actual.bytes, expected);
    let file = File::parse(&expected).unwrap();
    for bases in [
        [0x10000, 0x30000, 0x50000],
        [0x60000, 0x20000, 0x40000],
        [0x40000, 0x70000, 0x20000],
    ] {
        let mut vm = file.load(&bases).unwrap();
        vm.run(100).assert_completed();
        assert_eq!(
            vm.cpu.mem.bytes(bases[1], 4).unwrap(),
            bases[2].to_be_bytes()
        );
        assert_eq!(vm.cpu.mem.bytes(bases[2], 8).unwrap(), [0; 8]);
    }
}

#[test]
fn same_executable_runs_function_pointers_descriptors_aliases_and_bss_at_three_layouts() {
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
    for optimize in [false, true] {
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
        alias.name = "alias".into();
        alias.zero_fill = ByteSize::ZERO;
        mir.data.push(alias);
        let machine = materialize::materialize(&mir).unwrap();
        let object = emit(&mir, &machine).unwrap();
        let executable = hunk::emit(&hunk::entry_thunk(&object).unwrap()).unwrap();
        let file = File::parse(&executable.bytes).unwrap();
        assert_eq!(file.segments().len(), 3);
        for bases in [
            [0x10000, 0x30000, 0x50000],
            [0x60000, 0x20000, 0x40000],
            [0x40000, 0x70000, 0x20000],
        ] {
            let mut vm = file.load(&bases).unwrap();
            vm.run(10000).assert_completed();
            // Metadata is used only for assertions, never by File::parse/load.
            for (name, expected) in [("result", 31u32), ("zero", 21), ("alias", 21)] {
                let location = executable
                    .object
                    .symbols
                    .iter()
                    .find(|s| s.name == name)
                    .unwrap()
                    .location;
                assert_eq!(
                    vm.cpu
                        .mem
                        .bytes(location.resolve(&bases).unwrap(), 4)
                        .unwrap(),
                    expected.to_be_bytes()
                );
            }
        }
    }
}

#[test]
fn writer_rejects_unsupported_placements_and_relocations() {
    for case in 0..7 {
        let mut object = fixture();
        match case {
            0 => object.relocations[1].byte_index = Some(0),
            1 => object.relocations[1].width = 2,
            2 => object.relocations[0].offset = 3,
            3 => object.relocations[1].target = RelocationTarget::ImageEnd,
            4 => object.sections[1].fixed_address = Some(0x10000),
            5 => object.relocations[1].addend = i64::MAX,
            _ => object.relocations.push(object.relocations[0].clone()),
        }
        assert!(hunk::emit(&object).is_err(), "case {case}");
    }
    let mut object = fixture();
    object.relocations[0].addend = -2;
    assert!(hunk::emit(&object).is_ok());
    object.relocations[1].target = RelocationTarget::Location(Location::Absolute(0x2000));
    let executable = hunk::emit(&object).unwrap();
    let vm = File::parse(&executable.bytes)
        .unwrap()
        .load(&[0x10000, 0x30000, 0x50000])
        .unwrap();
    assert_eq!(vm.cpu.mem.bytes(0x30000, 4).unwrap(), [0, 0, 0x20, 0]);
}

#[test]
fn reader_rejects_truncation_bad_records_bounds_and_overlapping_relocations() {
    let valid = bytes(FIXTURE);
    for end in 0..valid.len() {
        assert!(File::parse(&valid[..end]).is_err(), "truncated at {end}");
    }
    for (index, value) in [
        (0, 0),
        (1, 1),
        (2, 999),
        (3, 1),
        (4, 3),
        (5, 0x40000003),
        (9, 4),
        (13, 999),
        (14, 65536),
        (15, 3),
        (16, 3),
        (16, 10),
        (18, 1000),
        (28, 1002),
    ] {
        let mut words = FIXTURE.to_vec();
        words[index] = value;
        assert!(File::parse(&bytes(&words)).is_err(), "{index}/{value}");
    }
    let mut words = FIXTURE.to_vec();
    words[14] = 2;
    words.insert(17, 4);
    assert!(File::parse(&bytes(&words)).is_err());
    let mut words = FIXTURE.to_vec();
    words.push(1010);
    assert!(File::parse(&bytes(&words)).is_err());
    let file = File::parse(&valid).unwrap();
    for bases in [
        [0x10001, 0x30000, 0x50000],
        [0x10000, 0x10000, 0x50000],
        [0x10000, 0x30000, 0xe0000],
        [0x10000, 0x30000, 0xfffffc],
        [0, 0x30000, 0x50000],
    ] {
        assert!(file.load(&bases).is_err());
    }
    let mut words = FIXTURE.to_vec();
    words[21] = u32::MAX;
    assert!(
        File::parse(&bytes(&words))
            .unwrap()
            .load(&[0x10000, 0x30000, 0x50000])
            .is_err()
    );
}

#[test]
fn large_relocation_groups_are_split_and_each_site_is_applied_once() {
    let mut object = fixture();
    let count = 65536u32;
    object.sections[1].bytes.resize(count as usize * 4, 0);
    object.sections[1].size = count * 4;
    let relocation = object.relocations[1].clone();
    for i in 1..count {
        object.relocations.push(Relocation {
            offset: i * 4,
            ..relocation.clone()
        });
    }
    let executable = hunk::emit(&object).unwrap();
    let file = File::parse(&executable.bytes).unwrap();
    let vm = file.load(&[0x10000, 0x20000, 0x70000]).unwrap();
    for cell in vm
        .cpu
        .mem
        .bytes(0x20000, count as usize * 4)
        .unwrap()
        .chunks_exact(4)
    {
        assert_eq!(cell, 0x70000u32.to_be_bytes());
    }
}
