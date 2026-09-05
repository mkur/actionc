//! Whole-record operations must preserve every embedded byte and place effect.
use super::array_execution_tests::{execute_with_limit, outputs};

fn execute(
    output: &crate::codegen::CodegenOutput,
    initialize: impl FnOnce(&mut [u8; 65536]),
) -> [u8; 65536] {
    execute_with_limit(output, 1_000_000, initialize)
}

#[test]
fn embedded_record_copies_preserve_full_extents_and_guards() {
    for (element, width) in [
        ("BYTE", 1usize),
        ("CHAR", 1),
        ("INT", 2),
        ("CARD", 2),
        ("REAL", 6),
        ("Cell", 3),
    ] {
        for count in [1usize, 2, 100, 127, 128, 129, 255, 256, 257] {
            let size = 2 + count * width;
            let source = format!(
                "TYPE Cell=[BYTE tag CARD value] \
                TYPE Buffer=[BYTE lead {element} ARRAY values({count}) BYTE tail] \
                Buffer source=$7001,destination=$7801 PROC Main() destination=source RETURN"
            );
            let expected = (0..size).map(|i| (i * 37 + 13) as u8).collect::<Vec<_>>();
            for (backend, output) in outputs(&source) {
                let ram = execute(&output, |ram| {
                    ram[0x7000..0x8000].fill(0xCC);
                    ram[0x7001..0x7001 + size].copy_from_slice(&expected);
                });
                assert_eq!(
                    &ram[0x7801..0x7801 + size],
                    &expected,
                    "{backend} {element}({count})"
                );
                assert_eq!(
                    &ram[0x7001..0x7001 + size],
                    &expected,
                    "source changed: {backend} {element}({count})"
                );
                for address in [0x7000, 0x7001 + size, 0x7800, 0x7801 + size] {
                    assert_eq!(
                        ram[address], 0xCC,
                        "guard: {backend} {element}({count}) at {address:x}"
                    );
                }
            }
        }
    }
}

#[test]
fn embedded_record_copies_handle_self_and_overlap_in_both_directions() {
    for count in [127usize, 128, 129, 255, 256, 257] {
        let size = 2 + count * 2;
        for destination in [0x7100usize, 0x7101, 0x7102] {
            let source = format!(
                "TYPE Buffer=[BYTE lead INT ARRAY values({count}) BYTE tail] \
                Buffer source=$7101,destination=${destination:X} PROC Main() destination=source RETURN"
            );
            let initial = (0..0x400usize)
                .map(|i| (i * 19 + 7) as u8)
                .collect::<Vec<_>>();
            let mut expected = initial.clone();
            expected.copy_within(0x101..0x101 + size, destination - 0x7000);
            for (backend, output) in outputs(&source) {
                let ram = execute(&output, |ram| ram[0x7000..0x7400].copy_from_slice(&initial));
                assert_eq!(
                    &ram[0x7000..0x7400],
                    &expected,
                    "{backend} count={count} destination={destination:x}"
                );
            }
        }
    }
}

#[test]
fn embedded_record_copies_compose_local_nested_pointer_and_indexed_places() {
    let source = "TYPE Buffer=[BYTE lead BYTE ARRAY bytes(257) CARD ARRAY words(129) BYTE tail] \
        TYPE Holder=[BYTE lead Buffer ARRAY items(2) BYTE tail] \
        Buffer source=$7001,destination=$7801 Buffer globalData Holder holderData \
        Buffer ARRAY origins(2),targets(2) Buffer POINTER p \
        BYTE order=$0600,targetCalls=$0601,sourceCalls=$0602 \
        BYTE FUNC PickTarget() targetCalls==+1 order=order*10+1 RETURN(0) \
        BYTE FUNC PickSource() sourceCalls==+1 order=order*10+2 RETURN(1) \
        PROC Main() Buffer localData \
          order=0 targetCalls=0 sourceCalls=0 \
          globalData=source localData=globalData holderData.items(1)=localData \
          origins(1)=holderData.items(1) targets(PickTarget())=origins(PickSource()) \
          p=destination p^=targets(0) RETURN";
    let size = 517;
    let expected = (0..size).map(|i| (i * 37 + 13) as u8).collect::<Vec<_>>();
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |ram| {
            ram[0x7000..0x8000].fill(0xCC);
            ram[0x7001..0x7001 + size].copy_from_slice(&expected);
        });
        assert_eq!(&ram[0x7801..0x7801 + size], &expected, "{backend}");
        assert_eq!(&ram[0x7001..0x7001 + size], &expected, "{backend}");
        assert_eq!(&ram[0x600..0x603], &[12, 1, 1], "{backend}");
        assert_eq!([ram[0x7800], ram[0x7801 + size]], [0xCC; 2], "{backend}");
    }
}

#[test]
fn embedded_record_copy_destination_survives_copying_calls_in_source_indexes() {
    let source = "TYPE Buffer=[BYTE ARRAY values(257)] \
        Buffer source=$7001,destination=$7801,extraSource=$7401,extraDestination=$7601 \
        Buffer ARRAY rows(1) \
        BYTE FUNC Pick() extraDestination=extraSource RETURN(0) \
        PROC Main() rows(0)=source destination=rows(Pick()) RETURN";
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |ram| {
            ram[0x7000..0x8000].fill(0xCC);
            ram[0x7001..0x7102].fill(0xA5);
            ram[0x7401..0x7502].fill(0x5A);
        });
        assert_eq!(
            &ram[0x7801..0x7902],
            &[0xA5; 257],
            "{backend}: captured destination"
        );
        assert_eq!(&ram[0x7601..0x7702], &[0x5A; 257], "{backend}: nested copy");
    }
}

#[test]
fn embedded_small_record_copies_handle_large_parent_offsets_and_strides() {
    let source = "TYPE Cell=[BYTE tag CARD value] TYPE Holder=[Cell ARRAY values(257)] \
        Holder source=$7001 Holder POINTER p Cell destination=$7801 CARD index=$0600 \
        PROC Main() p=source destination=p.values(index) RETURN";
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |ram| {
            ram[0x600..0x602].copy_from_slice(&256u16.to_le_bytes());
            ram[0x7000..0x8000].fill(0xCC);
            ram[0x7301..0x7304].copy_from_slice(&[0xA5, 0x34, 0x12]);
        });
        assert_eq!(&ram[0x7801..0x7804], &[0xA5, 0x34, 0x12], "{backend}");
    }
    for padding in [254usize, 255, 256, 257] {
        let source = format!(
            "TYPE Cell=[BYTE tag CARD value] \
            TYPE Holder=[BYTE ARRAY padding({padding}) Cell item] \
            Holder source=$7001 Holder POINTER p Cell destination=$7801 \
            PROC Main() p=source destination=p.item RETURN"
        );
        for (backend, output) in outputs(&source) {
            let ram = execute(&output, |ram| {
                ram[0x7000..0x8000].fill(0xCC);
                ram[0x7001 + padding..0x7004 + padding].copy_from_slice(&[0xA5, 0x34, 0x12]);
            });
            assert_eq!(
                &ram[0x7801..0x7804],
                &[0xA5, 0x34, 0x12],
                "{backend}: offset={padding}"
            );
        }
    }
    let source = "TYPE Cell=[BYTE tag CARD value] TYPE Holder=[BYTE ARRAY padding(257) Cell item] \
        Holder ARRAY rows(2)=$7001 Cell destination=$7801 CARD index=$0600 \
        PROC Main() destination=rows(index).item RETURN";
    for (backend, output) in outputs(source) {
        let ram = execute(&output, |ram| {
            ram[0x600..0x602].copy_from_slice(&1u16.to_le_bytes());
            ram[0x7000..0x8000].fill(0xCC);
            ram[0x7206..0x7209].copy_from_slice(&[0xA5, 0x34, 0x12]);
        });
        assert_eq!(
            &ram[0x7801..0x7804],
            &[0xA5, 0x34, 0x12],
            "{backend}: parent stride=260"
        );
    }
}
