use super::{Slot, TempId, state::*, tracked::*};

fn mir_loop() -> (TrackedEmitter65816, super::Label) {
    let mut e = TrackedEmitter65816::default();
    e.test_frame(8);
    let l = e.label();
    e.declare_blocks([l].into_iter());
    e.prove_entries([(l, [(None, 1), (Some(l), 1)].into())].into(), [l].into());
    (e, l)
}

fn pending_fallthrough() -> (TrackedEmitter65816, super::Label) {
    let mut e = TrackedEmitter65816::default();
    e.test_frame(0);
    let a = e.label();
    let b = e.label();
    e.declare_blocks([a, b].into_iter());
    e.prove_entries(
        [(a, [(None, 1)].into()), (b, [(Some(a), 1)].into())].into(),
        [a, b].into(),
    );
    e.mark(a);
    e.a16();
    e.fallthrough(b);
    (e, b)
}

#[test]
fn fallthrough_retains_logical_edges_without_instruction_or_mode_changes() {
    let (mut e, b) = pending_fallthrough();
    e.mark(b);
    e.a16();
    assert!(e.code().bytes.is_empty());
    e.op(Implied::Rtl);
    let c = e.finish();
    assert_eq!(c.bytes, [0x6b]);
    assert_eq!(c.mir_transfers.len(), 1);
    assert!(c.mir_transfers[0].fallthrough);
    assert_eq!(c.labels[&b], c.mir_transfers[0].offset);
}

#[test]
#[should_panic(expected = "fallthrough must bind the next MIR block")]
fn fallthrough_cannot_skip_another_binding() {
    let (mut e, _) = pending_fallthrough();
    let other = e.label();
    e.mark(other);
}

#[test]
#[should_panic(expected = "instruction without an execution contract")]
fn fallthrough_cannot_skip_an_instruction() {
    let (mut e, _) = pending_fallthrough();
    e.op(Implied::Nop);
}

#[test]
#[should_panic(expected = "unbound fallthrough target")]
fn fallthrough_must_be_bound_before_finalization() {
    let (e, _) = pending_fallthrough();
    e.finish();
}

#[test]
fn proved_mir_entry_omits_rep_but_clears_values_and_keeps_byte_transition() {
    let (mut e, l) = mir_loop();
    let slot = Slot {
        offset: 2,
        width: 2,
    };
    e.register_home(slot);
    e.a16();
    e.word(WordOp::LdaImm, 0xab80);
    e.byte(ByteOp::StaStack, 2);
    e.remember_word(TempId(0), slot);
    e.mark(l);
    let at = e.position();
    e.a16();
    assert_eq!(e.position(), at);
    assert!(!e.consume_word(
        Some(TempId(0)),
        Some(slot.into()),
        Some(super::copies::WordHome::Stack(2))
    ));
    e.a8();
    assert_eq!(&e.code().bytes[at..], &[0xe2, 0x20]);
    e.a16();
    e.jump(l);
    e.finish();
}

#[test]
fn dead_or_unproved_mir_binding_keeps_explicit_rep() {
    let mut e = TrackedEmitter65816::default();
    e.test_frame(0);
    let live = e.label();
    let dead = e.label();
    e.declare_blocks([live, dead].into_iter());
    e.prove_entries(
        [(live, [(None, 1)].into()), (dead, Default::default())].into(),
        [live].into(),
    );
    e.mark(live);
    e.op(Implied::Rtl);
    e.mark(dead);
    let at = e.position();
    e.a16();
    assert_eq!(&e.code().bytes[at..], &[0xc2, 0x20]);
    e.op(Implied::Rtl);
    e.finish();
}

#[test]
#[should_panic(expected = "unchecked MIR predecessors")]
fn missing_backedge_obligation_rejects_finalization() {
    let (mut e, l) = mir_loop();
    e.mark(l);
    e.finish();
}

#[test]
#[should_panic(expected = "MIR exit width")]
fn proved_entry_cannot_hide_an_incompatible_late_backedge() {
    let (mut e, l) = mir_loop();
    e.mark(l);
    e.a8();
    e.jump(l);
}

#[test]
#[should_panic(expected = "duplicate MIR predecessor")]
fn duplicate_predecessor_cannot_replace_a_missing_one() {
    let (mut e, l) = mir_loop();
    e.mark(l);
    e.branch(Branch::Equal, l);
    e.jump(l);
}

#[test]
fn immutable_values_unknowns_and_overlapping_home_generations() {
    let mut s = State65816::default();
    assert!(!Value::Unknown.matches(Value::Unknown));
    let a = s.fresh(Width::Word);
    let b = s.fresh(Width::Word);
    assert!(!a.matches(b));
    let slot = Slot {
        offset: 2,
        width: 2,
    };
    s.register_home(slot);
    s.load_a(a);
    s.write_stack(2, Width::Word);
    s.x = s.a;
    s.load_a(b);
    s.write_stack(3, Width::Byte);
    assert!(s.homes.is_empty());
    assert_eq!(s.x, a); // Captured token never follows a mutable home.
    s.load_a(a);
    s.write_stack(2, Width::Word);
    s.publish_word(TempId(0), slot, (1, 0));
    s.unknown_write();
    assert!(!s.consume_word(
        Some(TempId(0)),
        Some(slot.into()),
        Some(super::copies::WordHome::Stack(2)),
        Some((1, 0))
    ));
}
#[test]
fn flags_partial_lanes_calls_and_status_masks() {
    let mut s = State65816::default();
    s.load_a(Value::Constant(0x7fff, Width::Word));
    s.carry = Some(false);
    s.arithmetic(Value::Constant(1, Width::Word), false);
    assert_eq!(
        (s.a, s.carry, s.overflow),
        (
            Value::Constant(0x8000, Width::Word),
            Some(false),
            Some(true)
        )
    );
    s.compare(Value::Constant(1, Width::Word));
    assert!(!s.nz.matches(s.a));
    s.status(0x20, true);
    s.status(0x20, false);
    assert_eq!(s.a, Value::Unknown);
    s.x = Value::Constant(0x1234, Width::Word);
    s.status(0x10, true);
    assert_eq!(s.x, Value::Constant(0x34, Width::Byte));
    s.status(0xdf, true);
    assert_eq!(s.carry, Some(true));
    assert_eq!(s.overflow, Some(true));
    assert_eq!(s.env.decimal, Some(true));
    assert!(!s.env.irq_preserved);
    assert_eq!(s.nz, Value::Unknown);
    s.status(0xff, false);
    s.call_return();
    assert!(matches!(s.a, Value::Opaque(_, Width::Word)));
    assert_eq!(s.carry, None);
    assert!(s.homes.is_empty());
}
#[test]
fn labels_preserve_execution_contract_but_revoke_omission_and_values() {
    let mut e = TrackedEmitter65816::default();
    e.a8();
    e.byte(ByteOp::LdaImm, 1);
    let l = e.label();
    e.branch(Branch::Equal, l);
    e.mark(l);
    let start = e.position();
    e.byte(ByteOp::LdaImm, 2);
    assert_eq!(e.position() - start, 2);
    e.a8();
    assert_eq!(&e.code().bytes[e.position() - 2..], &[0xe2, 0x20]);
}
#[test]
#[should_panic(expected = "incompatible execution contracts")]
fn conflicting_backedge_is_rejected() {
    let mut e = TrackedEmitter65816::default();
    let l = e.label();
    e.mark(l);
    e.a8();
    e.branch(Branch::NotEqual, l);
}
#[test]
#[should_panic(expected = "immediate width")]
fn immediate_encoding_is_not_a_mode_assumption() {
    let mut e = TrackedEmitter65816::default();
    e.a8();
    e.word(WordOp::LdaImm, 1);
}
#[test]
fn observed_facts_never_extend_adjacency_permission() {
    let mut e = TrackedEmitter65816::default();
    let slot = Slot {
        offset: 254,
        width: 2,
    };
    e.register_home(slot);
    e.a16();
    e.word(WordOp::LdaImm, 0x8000);
    e.byte(ByteOp::StaStack, 254);
    e.remember_word(TempId(0), slot);
    e.op(Implied::Clc);
    assert!(!e.consume_word(
        Some(TempId(0)),
        Some(slot.into()),
        Some(super::copies::WordHome::Stack(254))
    ));
    e.remember_word(TempId(0), slot);
    e.barrier();
    assert!(!e.consume_word(
        Some(TempId(0)),
        Some(slot.into()),
        Some(super::copies::WordHome::Stack(254))
    ));
}
#[test]
fn stack_equations_and_transfer_peak_are_separate_from_body_addresses() {
    let mut e = TrackedEmitter65816::default();
    e.op(Implied::Tsc);
    e.op(Implied::Sec);
    e.word(WordOp::SbcImm, 8);
    e.op(Implied::Tcs);
    e.establish_body();
    assert_eq!(e.delta(), 0);
    e.op(Implied::Tsc);
    e.op(Implied::Sec);
    e.word(WordOp::SbcImm, 5);
    e.op(Implied::Tcs);
    assert_eq!(e.delta(), 5);
    let resume = e.label();
    e.op(Implied::Phk);
    e.push_return(resume);
    e.a8();
    e.op(Implied::Pha);
    e.a16();
    e.op(Implied::Pha);
    e.indirect_transfer();
    e.mark(resume);
    assert_eq!(e.delta(), 5);
    assert_eq!(e.peak(), 19);
}

#[test]
fn a_byte_constant_cannot_invent_the_hidden_high_lane_on_transfer() {
    let mut s = State65816::default();
    let v = s.narrow(Value::Constant(0x81, Width::Byte), Width::Word);
    assert!(matches!(v, Value::Opaque(_, Width::Word)));
    let v = s.narrow(Value::Constant(0xab81, Width::Word), Width::Byte);
    assert_eq!(v, Value::Constant(0x81, Width::Byte));
}
#[test]
fn call_and_join_invalidate_value_and_flag_relations_without_new_mode_omissions() {
    use super::Target;
    let mut e = TrackedEmitter65816::default();
    let slot = Slot {
        offset: 2,
        width: 2,
    };
    e.register_home(slot);
    e.a16();
    e.word(WordOp::LdaImm, 1);
    e.byte(ByteOp::StaStack, 2);
    e.remember_word(TempId(0), slot);
    e.reference(
        ReferenceOp::Jsl,
        Target::Routine(super::RoutineId(0)),
        0,
        None,
    );
    assert!(!e.consume_word(
        Some(TempId(0)),
        Some(slot.into()),
        Some(super::copies::WordHome::Stack(2))
    ));
    let before = e.position();
    e.a16();
    assert_eq!(e.position(), before);
    let label = e.label();
    e.mark(label);
    e.a16();
    assert_eq!(e.position(), before + 2);
}

#[test]
fn frame_witness_requires_object_byte_generation_full_nz_and_exact_cursor() {
    let slot = Slot {
        offset: 254,
        width: 2,
    };
    let identity = WordIdentity::Frame(super::Mir65816FrameObjectId(3), 4);
    for case in 0..12 {
        let mut s = State65816::default();
        s.register_home(slot);
        s.load_a(Value::Constant(0x8000, Width::Word));
        s.write_stack(254, Width::Word);
        s.publish_adjacent(identity, slot, (12, 1));
        let mut requested = identity;
        let mut home = slot;
        let mut cursor = (12, 1);
        match case {
            1 => requested = WordIdentity::Temp(TempId(3)),
            2 => requested = WordIdentity::Frame(super::Mir65816FrameObjectId(4), 4),
            3 => requested = WordIdentity::Frame(super::Mir65816FrameObjectId(3), 5),
            4 => home.offset = 253,
            5 => s.write_stack(255, Width::Byte),
            6 => s.write_stack(254, Width::Word), // Same bits, different generation.
            7 => s.unknown_write(),
            8 => s.compare(Value::Constant(0x8000, Width::Word)),
            9 => cursor.0 += 1,
            10 => cursor.1 += 1,
            11 => {
                s.status(0x20, true);
                s.status(0x20, false);
            }
            _ => {}
        }
        assert_eq!(
            s.consume_adjacent(
                Some(requested),
                Some(home.into()),
                Some(super::copies::WordHome::Stack(254)),
                Some(cursor)
            ),
            case == 0
        );
        assert!(!s.consume_adjacent(
            Some(identity),
            Some(slot.into()),
            Some(super::copies::WordHome::Stack(254)),
            Some((12, 1))
        ));
    }
}

#[test]
fn incoming_read_facts_require_identity_generation_and_never_admit_writes() {
    let source = Slot {
        offset: 254,
        width: 2,
    };
    let capture = Slot {
        offset: 2,
        width: 2,
    };
    for case in 0..8 {
        let mut e = TrackedEmitter65816::default();
        e.test_frame(8);
        e.register_home(capture);
        e.capture_incoming_word(super::ParamId(0), source, TempId(1), capture.into());
        let mut s = e.state_for_incoming_test();
        let mut param = super::ParamId(0);
        let mut home = source;
        match case {
            1 => param = super::ParamId(1),
            2 => home.offset = 252,
            3 => s.write_stack(255, Width::Byte),
            4 => s.write_stack(254, Width::Word),
            5 => s.write_stack(2, Width::Word),
            6 => s.unknown_write(),
            7 => s.compare(Value::Constant(0, Width::Word)),
            _ => {}
        }
        assert_eq!(s.consume_incoming(param, home, e.word_cursor()), case == 0);
        if case == 4 {
            assert!(!s.homes.contains_key(&super::Location::Stack(Slot {
                offset: 254,
                width: 2
            })));
        }
        assert!(!s.consume_incoming(super::ParamId(0), source, e.word_cursor()));
    }
}

#[test]
fn dp_generations_are_separate_and_partial_writes_invalidate_the_word() {
    use super::{Location, copies::WordHome};
    let mut s = State65816::default();
    let slot = Slot {
        offset: 32,
        width: 2,
    };
    let dp = Location::DirectPage(slot);
    s.register_home(slot);
    s.register_home(dp);
    s.load_a(Value::Constant(17, Width::Word));
    s.write_stack(32, Width::Word);
    s.load_a(Value::Constant(23, Width::Word));
    s.write_dp(32, Width::Word);
    assert_eq!(
        s.read_stack(32, Width::Word),
        Value::Constant(17, Width::Word)
    );
    assert_eq!(s.read_dp(32, Width::Word), Value::Constant(23, Width::Word));
    s.publish_word(TempId(0), dp, (1, 0));
    assert!(!s.consume_word(
        Some(TempId(0)),
        Some(slot.into()),
        Some(WordHome::Stack(32)),
        Some((1, 0))
    ));
    s.publish_word(TempId(0), dp, (1, 0));
    assert!(s.consume_word(
        Some(TempId(0)),
        Some(dp),
        Some(WordHome::DirectPage(32)),
        Some((1, 0))
    ));
    s.write_dp(33, Width::Byte);
    assert!(!s.homes.contains_key(&dp));
    assert_eq!(
        s.read_stack(32, Width::Word),
        Value::Constant(17, Width::Word)
    );
    s.write_dp(8, Width::Word);
    assert!(s.homes.is_empty());
}

fn x_loop() -> (TrackedEmitter65816, [super::Label; 4]) {
    let mut e = TrackedEmitter65816::default();
    e.test_frame(0);
    let labels = [e.label(), e.label(), e.label(), e.label()];
    let [pre, head, body, end] = labels;
    e.register_home(super::Location::DirectPage(Slot {
        offset: 32,
        width: 2,
    }));
    e.register_home(super::Location::DirectPage(Slot {
        offset: 34,
        width: 2,
    }));
    e.declare_blocks(labels.into_iter());
    e.prove_entries(
        [
            (pre, [(None, 1)].into()),
            (head, [(Some(pre), 1), (Some(body), 1)].into()),
            (body, [(Some(head), 1)].into()),
            (end, [(Some(head), 1)].into()),
        ]
        .into(),
        labels.into(),
    );
    e.prove_x(XContract {
        param: TempId(0),
        increment: Some((
            TempId(1),
            super::Location::DirectPage(Slot {
                offset: 34,
                width: 2,
            }),
        )),
        home: super::Location::DirectPage(Slot {
            offset: 32,
            width: 2,
        }),
        header: head,
        body,
        predecessors: [pre, body].into(),
    });
    e.mark(pre);
    e.word(WordOp::LdaImm, 0);
    e.byte(ByteOp::StaDp, 32);
    (e, labels)
}

#[test]
fn x_relation_crosses_only_checked_entries_and_refreshes_after_store() {
    let (mut e, [_, head, body, end]) = x_loop();
    e.refresh_x();
    e.jump(head);
    e.mark(head);
    let home = super::Location::DirectPage(Slot {
        offset: 32,
        width: 2,
    });
    assert!(
        e.state_for_incoming_test()
            .x
            .matches(e.state_for_incoming_test().homes[&home].value)
    );
    assert_eq!(e.state_for_incoming_test().a, Value::Unknown);
    e.compare_x_word(TempId(0), home, 8);
    let yes = e.label();
    e.dispatch(Branch::CarryClear, yes);
    e.jump(end);
    e.mark(yes);
    e.fallthrough(body);
    e.mark(body);
    assert!(e.load_x_word(Some(TempId(0)), Some(home)));
    e.op(Implied::Clc);
    e.word(WordOp::AdcImm, 1);
    e.byte(ByteOp::StaDp, 34);
    e.byte(ByteOp::LdaDp, 34);
    e.byte(ByteOp::StaDp, 32);
    let s = e.state_for_incoming_test();
    assert!(!s.x.matches(s.homes[&home].value));
    e.refresh_x();
    e.jump(head);
    e.mark(end);
    assert!(!e.load_x_word(Some(TempId(0)), Some(home)));
    e.op(Implied::Rtl);
    e.finish();
}

#[test]
fn x_missing_refresh_clobber_and_partial_store_are_rejected() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let (mut e, [_, head, _, _]) = x_loop();
    assert!(catch_unwind(AssertUnwindSafe(|| e.jump(head))).is_err());
    for kind in 0..6 {
        let (mut e, [_, head, _, _]) = x_loop();
        e.refresh_x();
        e.jump(head);
        e.mark(head);
        assert!(
            catch_unwind(AssertUnwindSafe(|| match kind {
                0 => e.op(Implied::Dex),
                1 => e.byte(ByteOp::Sep, 0x10),
                2 => e.reference(
                    ReferenceOp::Jsl,
                    super::Target::Routine(super::RoutineId(0)),
                    0,
                    None
                ),
                3 => {
                    e.a8();
                    e.byte(ByteOp::StaDp, 33);
                    e.a16();
                    e.compare_x_word(
                        TempId(0),
                        super::Location::DirectPage(Slot {
                            offset: 32,
                            width: 2,
                        }),
                        8,
                    );
                }
                4 => {
                    e.byte(ByteOp::StaDp, 32);
                    e.load_x_word(
                        Some(TempId(0)),
                        Some(super::Location::DirectPage(Slot {
                            offset: 32,
                            width: 2,
                        })),
                    );
                }
                _ => {
                    e.finish();
                }
            }))
            .is_err()
        );
    }
}

#[test]
fn cpx_uses_index_width_preserves_a_x_and_v() {
    for a8 in [false, true] {
        let mut e = TrackedEmitter65816::default();
        e.a16();
        e.word(WordOp::LdaImm, 0x8000);
        e.op(Implied::Tax);
        if a8 {
            e.a8();
            e.byte(ByteOp::LdaImm, 0x55);
        }
        let before = e.state_for_incoming_test();
        e.word(WordOp::CpxImm, 0x7fff);
        let after = e.state_for_incoming_test();
        assert_eq!(
            (after.a, after.x, after.overflow),
            (before.a, before.x, before.overflow)
        );
        assert_eq!(after.carry, Some(true));
        assert_eq!(after.nz, Value::Constant(1, Width::Word));
        assert!(e.code().bytes.ends_with(&[0xe0, 0xff, 0x7f]));
    }
}

fn x_body() -> (TrackedEmitter65816, [super::Label; 4]) {
    let (mut e, labels) = x_loop();
    let [_, head, body, end] = labels;
    e.refresh_x();
    e.jump(head);
    e.mark(head);
    let yes = e.label();
    e.dispatch(Branch::CarryClear, yes);
    e.jump(end);
    e.mark(yes);
    e.fallthrough(body);
    e.mark(body);
    (e, labels)
}
#[test]
fn increment_keeps_distinct_home_values_and_requires_final_refresh() {
    let (mut e, [_, head, _, end]) = x_body();
    let p = super::Location::DirectPage(Slot {
        offset: 32,
        width: 2,
    });
    let q = super::Location::DirectPage(Slot {
        offset: 34,
        width: 2,
    });
    let old = e.state_for_incoming_test().x;
    let at = e.position();
    e.increment_x_word(TempId(0), p, TempId(1), q);
    assert_eq!(&e.code().bytes[at..], &[0xe8, 0x8a]);
    let s = e.state_for_incoming_test();
    assert_eq!(s.homes[&p].value, old);
    assert_ne!(s.x, old);
    assert_eq!(s.a, s.x);
    assert_eq!(s.nz, s.x);
    e.byte(ByteOp::StaDp, 34);
    e.byte(ByteOp::LdaDp, 34);
    e.byte(ByteOp::StaDp, 32);
    e.refresh_x();
    e.jump(head);
    e.mark(end);
    e.op(Implied::Rtl);
    e.finish();
}
#[test]
fn increment_pending_relation_rejects_consumers_even_after_home_store() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let p = super::Location::DirectPage(Slot {
        offset: 32,
        width: 2,
    });
    let q = super::Location::DirectPage(Slot {
        offset: 34,
        width: 2,
    });
    for kind in 0..8 {
        let (mut e, [_, head, _, _]) = x_body();
        if kind < 6 {
            e.increment_x_word(TempId(0), p, TempId(1), q);
        }
        assert!(
            catch_unwind(AssertUnwindSafe(|| match kind {
                0 => e.compare_x_word(TempId(0), p, 8),
                1 => {
                    e.load_x_word(Some(TempId(0)), Some(p));
                }
                2 => e.increment_x_word(TempId(0), p, TempId(1), q),
                3 => {
                    let internal = e.label();
                    e.jump(internal);
                }
                4 => e.jump(head),
                5 => {
                    e.byte(ByteOp::StaDp, 32);
                    e.compare_x_word(TempId(0), p, 8);
                }
                6 => e.increment_x_word(TempId(0), p, TempId(2), q),
                _ => e.op(Implied::Inx),
            }))
            .is_err(),
            "kind {kind}"
        );
    }
}

#[test]
fn word_eor_preserves_carry_and_overflow_but_replaces_a_and_nz() {
    for known in [false, true] {
        let mut e = TrackedEmitter65816::default();
        e.a16();
        if known {
            e.word(WordOp::LdaImm, 0x7fff);
        }
        e.op(Implied::Sec);
        e.word(WordOp::SbcImm, 0xffff);
        let before = e.state_for_incoming_test();
        e.word(WordOp::EorImm, 0x8000);
        let after = e.state_for_incoming_test();
        assert_eq!(
            (after.carry, after.overflow),
            (before.carry, before.overflow)
        );
        assert_eq!(after.a, after.nz);
        assert!(!after.a.matches(before.a));
        if known {
            assert_eq!(after.a, Value::Constant(0, Width::Word));
            assert_eq!(after.overflow, Some(true));
        } else {
            assert!(matches!(after.a, Value::Opaque(_, Width::Word)));
        }
        let join = e.label();
        e.branch(Branch::OverflowClear, join);
        e.word(WordOp::EorImm, 0x8000);
        e.mark(join);
        assert_eq!(e.state_for_incoming_test().a, Value::Unknown);
        assert!(e.resident().is_none());
    }
}
#[test]
#[should_panic]
fn word_eor_requires_a16() {
    let mut e = TrackedEmitter65816::default();
    e.a8();
    e.word(WordOp::EorImm, 0x8000);
}
