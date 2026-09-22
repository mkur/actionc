use super::*;
use copies::{WordCopies, WordHome, WordStrategy};

#[test]
fn dp_arithmetic_and_return_preserve_adjacent_forwarding() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let mut frame = AllocatedFrame::new(r).unwrap();
    for (i, (id, _)) in r.temps.iter().enumerate() {
        frame.temps.insert(
            *id,
            Location::DirectPage(Slot {
                offset: 32 + 2 * i as u16,
                width: 2,
            }),
        );
    }
    let mut b = Builder {
        next_block: None,
        loop_x: None,
        routine: r,
        frame,
        code: TrackedEmitter65816::for_entry(r.prologue.required_mode),
        blocks: BTreeMap::new(),
    };
    for home in b.frame.temps.values() {
        b.code.register_home(*home);
    }
    let Mir65816Op::Binary {
        dest, left, right, ..
    } = r.blocks[0].ops.last().unwrap()
    else {
        panic!()
    };
    let Mir65816Value::Temp(a, _) = left else {
        panic!()
    };
    b.code.a16();
    b.code.word(WordOp::LdaImm, 0x8000);
    b.code.byte(ByteOp::StaDp, 32);
    b.remember_word(*a);
    let start = b.code.position();
    assert!(
        b.word_binary(*dest, 2, NirBinaryOp::Add, left, right)
            .unwrap()
    );
    assert_eq!(&b.code.code().bytes[start..], &[0x18, 0x65, 34, 0x85, 36]);
    let start = b.code.position();
    assert!(
        b.word_return(&Mir65816Value::Temp(*dest, ByteSize::new(2)))
            .unwrap()
    );
    assert_eq!(b.code.position(), start);
}

#[test]
fn mixed_space_scheduling_obeys_parallel_oracle_and_costs() {
    let homes = [
        WordHome::Stack(32),
        WordHome::DirectPage(32),
        WordHome::DirectPage(34),
    ];
    let index = |h| match h {
        WordHome::Stack(n) => usize::from(n),
        WordHome::DirectPage(n) => 64 + usize::from(n),
    };
    for a in 0..3 {
        for b in 0..3 {
            for c in 0..3 {
                let moves: Vec<_> = [a, b, c]
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| (homes[s].operand(), homes[i]))
                    .collect();
                let plan = WordCopies::plan_homes(moves.clone());
                let captures = plan.captures().unwrap();
                let initial: Vec<_> = (0..128u8).collect();
                let mut expected = initial.clone();
                let mut actual = initial.clone();
                let load = |mem: &[u8], s: WordOperand| {
                    let at = index(s.home().unwrap());
                    [mem[at], mem[at + 1]]
                };
                for &(s, d) in &moves {
                    let at = index(d);
                    expected[at..at + 2].copy_from_slice(&load(&initial, s));
                }
                let saved: Vec<_> = captures
                    .iter()
                    .map(|&i| load(&initial, moves[i].0))
                    .collect();
                let (order, repair) = plan.direct_emission().unwrap_or(((0..3).collect(), false));
                let mut last = None;
                for i in order {
                    let (s, d) = moves[i];
                    let v = captures
                        .iter()
                        .position(|&j| j == i)
                        .map_or_else(|| load(&actual, s), |k| saved[k]);
                    let at = index(d);
                    actual[at..at + 2].copy_from_slice(&v);
                    last = Some(v);
                }
                if repair {
                    last = Some(load(&actual, moves.last().unwrap().1.operand()));
                }
                assert_eq!(actual, expected);
                assert_eq!(last, Some(load(&initial, moves.last().unwrap().0)));
                assert!(!matches!(plan.strategy, WordStrategy::Complete));
            }
        }
    }
    let p = WordCopies::plan_homes(vec![(WordOperand::Stack(32), WordHome::DirectPage(32))]);
    assert_eq!(p.direct_emission(), Some((vec![0], false)));
    assert_eq!(p.cost(), (4, 9));
    let p = WordCopies::plan_homes(vec![(
        WordOperand::DirectPage(32),
        WordHome::DirectPage(32),
    )]);
    assert_eq!(p.direct_emission(), Some((vec![], true)));
    assert_eq!(p.cost(), (2, 4));
}

#[test]
fn dp_word_bounds_do_not_depend_on_stack_delta() {
    for (offset, ok) in [
        (30, false),
        (31, false),
        (32, true),
        (33, false),
        (62, true),
        (63, false),
        (64, false),
        (254, false),
    ] {
        for delta in [0, u32::MAX] {
            assert_eq!(
                word_home(Location::DirectPage(Slot { offset, width: 2 }), delta).is_ok(),
                ok
            );
        }
    }
}
