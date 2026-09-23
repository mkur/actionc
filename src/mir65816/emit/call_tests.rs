use super::*;

fn home(offset: u32, size: u32) -> Mir65816AbiHome {
    Mir65816AbiHome::StackArgument {
        offset: ByteOffset::new(offset),
        size: ByteSize::new(size),
        alignment: ByteSize::ONE,
    }
}

#[test]
fn padding_preflight_keeps_holes_tail_and_the_last_encodable_byte() {
    assert_eq!(outgoing_padding(&[], ByteSize::ONE).unwrap(), [1]);
    assert!(
        outgoing_padding(&[home(0, 1)], ByteSize::ONE)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        outgoing_padding(
            &[home(0, 1), home(2, 2), home(4, 3), home(8, 4)],
            ByteSize::new(13)
        )
        .unwrap(),
        [2, 8, 13]
    );
    assert_eq!(
        outgoing_padding(&[home(0, 254)], ByteSize::new(255)).unwrap(),
        [255]
    );
}

#[test]
fn padding_preflight_rejects_invalid_ranges_without_truncation() {
    for (homes, outgoing) in [
        (vec![], 0),
        (vec![], 256),
        (vec![home(u32::MAX, 1)], 255),
        (vec![home(0, 0)], 1),
        (vec![home(0, 4)], 3),
        (vec![home(0, 3), home(2, 1)], 5),
        (vec![home(2, 1), home(0, 1)], 5),
        (
            vec![Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)],
            1,
        ),
    ] {
        assert!(outgoing_padding(&homes, ByteSize::new(outgoing)).is_err());
    }
}
