use super::*;

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::stack(routine).unwrap();
    let code = TrackedEmitter65816::for_test(&frame);
    Builder {
        routine,
        frame,
        code,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
    }
}

#[test]
fn constant_stores_use_exact_pairs_tails_and_local_immediate_reuse() {
    let p = word_tests::program();
    for (value, bytes, expected) in [
        (Mir65816Value::U8(255), 2, vec![0xa9, 255, 0, 0x83, 16]),
        (
            Mir65816Value::Address(
                crate::target::AddressValue::data(0xabcdef),
                ByteSize::new(3),
            ),
            4,
            vec![0xa9, 0xef, 0xcd, 0x83, 16, 0xa9, 0xab, 0, 0x83, 18],
        ),
        (
            Mir65816Value::U16(0x1234),
            4,
            vec![0xa9, 0x34, 0x12, 0x83, 16, 0xa9, 0, 0, 0x83, 18],
        ),
        (
            Mir65816Value::U24(0xabcdef),
            4,
            vec![0xa9, 0xef, 0xcd, 0x83, 16, 0xa9, 0xab, 0, 0x83, 18],
        ),
        (
            Mir65816Value::U32(0x12341234),
            4,
            vec![0xa9, 0x34, 0x12, 0x83, 16, 0x83, 18],
        ),
        (
            Mir65816Value::U32(0x89abcdef),
            2,
            vec![0xa9, 0xef, 0xcd, 0x83, 16],
        ),
        (
            Mir65816Value::U32(0x89abcdef),
            3,
            vec![0xa9, 0xef, 0xcd, 0x83, 16, 0xe2, 0x20, 0xa9, 0xab, 0x83, 18],
        ),
        (
            Mir65816Value::Null(ByteSize::new(3)),
            3,
            vec![0xa9, 0, 0, 0x83, 16, 0xe2, 0x20, 0x83, 18],
        ),
        (
            Mir65816Value::U32(0),
            4,
            vec![0xa9, 0, 0, 0x83, 16, 0x83, 18],
        ),
        (
            Mir65816Value::U24(0x123412),
            3,
            vec![0xa9, 0x12, 0x34, 0x83, 16, 0xe2, 0x20, 0x83, 18],
        ),
    ] {
        let mut b = builder(&p.routines[0]);
        b.code.a16();
        let start = b.code.code().bytes.len();
        assert_eq!(
            b.constant_store(Memory::Stack(16), &value, bytes, false),
            Ok(true)
        );
        assert_eq!(&b.code.code().bytes[start..], expected, "{value:?}/{bytes}");
    }
}

#[test]
fn constant_store_fallbacks_leave_code_and_mode_untouched() {
    let p = word_tests::program();
    for (value, bytes, volatile, byte_mode) in [
        (Mir65816Value::U32(0), 4, true, false),
        (Mir65816Value::U8(0), 1, false, false),
        (
            Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
            3,
            false,
            false,
        ),
        (
            Mir65816Value::Temp(TempId(999), ByteSize::new(4)),
            4,
            false,
            false,
        ),
        (Mir65816Value::U24(0xabcdef), 3, false, true),
    ] {
        let mut b = builder(&p.routines[0]);
        if byte_mode {
            b.code.a8();
        } else {
            b.code.a16();
        }
        let before = format!("{:?}", b.code);
        assert_eq!(
            b.constant_store(Memory::Stack(16), &value, bytes, volatile),
            Ok(false)
        );
        assert_eq!(format!("{:?}", b.code), before);
    }
    for memory in [
        Memory::Absolute(0x12ffff),
        Memory::Pointer {
            slot: 0,
            offset: 65532,
        },
    ] {
        let mut b = builder(&p.routines[0]);
        b.code.a8();
        assert_eq!(
            b.constant_store(memory, &Mir65816Value::U24(0xabcdef), 3, false),
            Ok(true)
        );
    }
}

#[test]
fn constant_stores_preflight_full_extent_and_transient_stack_delta() {
    let p = word_tests::program();
    for bytes in 2..=4 {
        for (memory, delta, valid) in [
            (Memory::Stack(256 - u32::from(bytes)), 0, true),
            (Memory::Stack(257 - u32::from(bytes)), 0, false),
            (Memory::Stack(255 - u32::from(bytes)), 1, true),
            (Memory::Stack(256 - u32::from(bytes)), 1, false),
            (Memory::Stack(0), 0, false),
            (Memory::Stack(1), u32::MAX, false),
            (Memory::Absolute(0x1000000 - u32::from(bytes)), 0, true),
            (Memory::Absolute(0x1000001 - u32::from(bytes)), 0, false),
            (Memory::DirectPage(256 - u16::from(bytes)), 0, true),
            (Memory::DirectPage(257 - u16::from(bytes)), 0, false),
            (
                Memory::Pointer {
                    slot: 0,
                    offset: 65535 - u16::from(bytes) + 1,
                },
                0,
                true,
            ),
            (
                Memory::Pointer {
                    slot: 0,
                    offset: 65535 - u16::from(bytes) + 2,
                },
                0,
                false,
            ),
            (
                Memory::Symbol(
                    Target::Routine(p.routines[0].id),
                    u32::MAX - u32::from(bytes) + 1,
                ),
                0,
                true,
            ),
            (
                Memory::Symbol(
                    Target::Routine(p.routines[0].id),
                    u32::MAX - u32::from(bytes) + 2,
                ),
                0,
                false,
            ),
        ] {
            let mut b = builder(&p.routines[0]);
            b.code.test_delta(delta);
            b.code.a8();
            let before = format!("{:?}", b.code);
            let result = b.constant_store(memory, &Mir65816Value::U32(0), bytes, false);
            if valid {
                assert_eq!(result, Ok(true), "{bytes}/{delta}");
            } else {
                assert!(result.is_err(), "{bytes}/{delta}");
                assert_eq!(format!("{:?}", b.code), before);
            }
        }
    }
}
