use super::{Slot, TempId, state::*, tracked::*};

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
    assert!(!s.consume_word(Some(TempId(0)), Some(slot), Some(2), Some((1, 0))));
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
    assert!(!e.consume_word(Some(TempId(0)), Some(slot), Some(254)));
    e.remember_word(TempId(0), slot);
    e.barrier();
    assert!(!e.consume_word(Some(TempId(0)), Some(slot), Some(254)));
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
    assert!(!e.consume_word(Some(TempId(0)), Some(slot), Some(2)));
    let before = e.position();
    e.a16();
    assert_eq!(e.position(), before);
    let label = e.label();
    e.mark(label);
    e.a16();
    assert_eq!(e.position(), before + 2);
}
