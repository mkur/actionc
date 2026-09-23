use super::*;
use crate::mir65816::emit::{AllocatedFrame, Location, Slot, copies::WordHome, layout};
use crate::nir::{ParamId, RoutineId, TempId};
use std::collections::BTreeMap;

fn frame() -> AllocatedFrame {
    AllocatedFrame {
        extent: 8,
        spill_bytes: 8,
        peak_below_entry: 8,
        temps: BTreeMap::new(),
        edge_copies: vec![],
    }
}
fn finish(e: TrackedEmitter65816) -> Code {
    e.finish_selected(RoutineId(7), &frame(), None).unwrap()
}
fn captures(trace: bool) -> Code {
    let mut e = TrackedEmitter65816::default();
    #[cfg(feature = "native65816-state-proof")]
    if trace {
        e.trace();
    }
    let _ = trace;
    e.op(Implied::Tsc);
    e.op(Implied::Sec);
    e.word(WordOp::SbcImm, 8);
    e.op(Implied::Tcs);
    e.establish_body();
    let a = Slot {
        offset: 2,
        width: 2,
    };
    let b = Slot {
        offset: 4,
        width: 2,
    };
    let c = Slot {
        offset: 6,
        width: 2,
    };
    let incoming = Slot {
        offset: 12,
        width: 2,
    };
    for s in [a, b, c] {
        e.register_home(s);
    }
    e.a16();
    e.a16();
    e.word(WordOp::LdaImm, 7);
    e.byte(ByteOp::StaStack, 2);
    e.remember_word(TempId(0), a);
    assert!(e.consume_word(Some(TempId(0)), Some(a.into()), Some(WordHome::Stack(2))));
    assert!(!e.consume_word(Some(TempId(0)), Some(a.into()), Some(WordHome::Stack(2))));
    e.byte(ByteOp::StaStack, 2);
    e.remember_frame_word(crate::mir65816::Mir65816FrameObjectId(0), 0, a);
    assert!(e.consume_frame_word(crate::mir65816::Mir65816FrameObjectId(0), 0, a, 2));
    e.capture_incoming_word(ParamId(0), incoming, TempId(1), b.into());
    assert!(e.store_incoming_capture(TempId(1), b.into(), c));
    e.capture_incoming_word(ParamId(0), incoming, TempId(0), a.into());
    e.barrier();
    e.a8();
    e.a16();
    e.op(Implied::Tsc);
    e.op(Implied::Clc);
    e.word(WordOp::AdcImm, 8);
    e.op(Implied::Tcs);
    e.native_return(None).unwrap();
    finish(e)
}
#[test]
fn fresh_replay_recomputes_modes_generations_and_consumed_witnesses() {
    for trace in [false, true] {
        let reference = captures(trace);
        let replayed = emit(reference.selected.as_ref().unwrap(), trace).unwrap();
        equivalent(&reference, &replayed).unwrap();
        let again = emit(replayed.selected.as_ref().unwrap(), trace).unwrap();
        equivalent(&replayed, &again).unwrap();
        assert_eq!(
            reference.selected.as_ref().unwrap().records(),
            replayed.selected.as_ref().unwrap().records()
        );
        for (index, _) in reference
            .selected
            .as_ref()
            .unwrap()
            .records()
            .iter()
            .enumerate()
        {
            let site = reference
                .selected
                .as_ref()
                .unwrap()
                .site(Node(index))
                .unwrap();
            assert_eq!(
                replayed.selected.as_ref().unwrap().validate(site),
                Ok(Node(index))
            );
        }
    }
}
#[test]
fn replay_rebuilds_short_long_dispatch_spans_and_fixups_from_symbolic_sites() {
    for count in [0, 200] {
        let mut e = TrackedEmitter65816::default();
        let dest = e.label();
        e.begin_source(crate::nir::BlockId(0), 0);
        let start = e.position();
        e.dispatch(Branch::Equal, dest);
        for _ in 0..count {
            e.op(Implied::Nop);
        }
        e.mark(dest);
        e.native_return(None).unwrap();
        e.fused_span(crate::nir::BlockId(0), 0, start, 1);
        let raw = finish(e);
        let finalized = layout::finalize(raw, true).unwrap();
        assert_eq!(finalized.conditional_branches[0].short, count == 0);
        let replayed = layout::finalize(
            emit(finalized.selected.as_ref().unwrap(), false).unwrap(),
            true,
        )
        .unwrap();
        equivalent(&finalized, &replayed).unwrap();
    }
}
#[test]
fn indirect_transfer_rebuilds_the_exact_per_continuation() {
    let mut e = TrackedEmitter65816::default();
    let resume = e.label();
    e.op(Implied::Phk);
    e.push_return(resume);
    e.a8();
    e.byte(ByteOp::LdaImm, 2);
    e.op(Implied::Pha);
    e.a16();
    e.word(WordOp::LdaImm, 0x1234);
    e.op(Implied::Pha);
    e.indirect_transfer();
    e.mark(resume);
    e.native_return(None).unwrap();
    let reference = finish(e);
    let replayed = emit(reference.selected.as_ref().unwrap(), false).unwrap();
    equivalent(&reference, &replayed).unwrap();
    assert_eq!(replayed.return_fixups.len(), 1);
}
#[test]
fn stored_success_and_stale_capture_identity_cannot_authorize_replay() {
    for decision_only in [false, true] {
        let reference = captures(false);
        let s = reference.selected.as_ref().unwrap();
        let mut records = s.records()[..s.records().len() - 2].to_vec();
        if decision_only {
            let r = records
                .iter_mut()
                .find(|r| r.decision == Some(true))
                .unwrap();
            r.decision = Some(false);
        } else {
            let r = records
                .iter_mut()
                .find(|r| matches!(r.action, Action::Request(Request::RememberWord(..))))
                .unwrap();
            r.action = Action::Request(Request::RememberWord(
                TempId(999),
                Location::Stack(Slot {
                    offset: 2,
                    width: 2,
                }),
            ));
        }
        let changed = SelectedRoutine::new(
            RoutineId(7),
            &frame(),
            None,
            Recording {
                records,
                parents: vec![],
                source: None,
            },
            &reference,
        )
        .unwrap();
        assert!(
            emit(&changed, false)
                .unwrap_err()
                .contains("consume decisions differ")
        );
        equivalent(&reference, &captures(false)).unwrap();
    }
}
#[test]
fn decision_observations_have_a_checked_structural_location() {
    let reference = captures(false);
    let s = reference.selected.as_ref().unwrap();
    for wrong_location in [false, true] {
        let mut records = s.records().to_vec();
        if wrong_location {
            records[0].decision = Some(true);
        } else {
            records
                .iter_mut()
                .find(|r| r.decision.is_some())
                .unwrap()
                .decision = None;
        }
        assert!(s.edited(records).is_err());
    }
}

#[test]
fn replay_uses_verified_input_cfg_but_validates_its_new_output() {
    let code = captures(false);
    let selected = code.selected.as_ref().unwrap();
    let stop = Node(
        selected
            .records()
            .iter()
            .position(|r| matches!(r.action, Action::Request(Request::ConsumeWord(..))))
            .unwrap(),
    );
    let (_, prefix_work) = super::super::work::measure(|| prefix(selected, stop).unwrap());
    assert_eq!(prefix_work.get("prefix_replay"), Some(&1));
    assert_eq!(prefix_work.get("cfg"), None);
    let (output, full_work) = super::super::work::measure(|| emit(selected, false).unwrap());
    assert_eq!(full_work.get("full_replay"), Some(&1));
    assert_eq!(full_work.get("cfg"), Some(&1));
    equivalent(&code, &output).unwrap();
}
