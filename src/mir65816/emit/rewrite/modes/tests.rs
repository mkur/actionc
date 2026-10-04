use super::*;
use crate::mir65816::emit::{AllocatedFrame, replay};
use crate::nir::{BlockId, RoutineId};

fn finish(mut e: TrackedEmitter65816) -> Code {
    e.a16();
    e.native_return(None).unwrap();
    layout::finalize(
        e.finish_selected(
            RoutineId(0),
            &AllocatedFrame {
                extent: 0,
                spill_bytes: 0,
                peak_below_entry: 0,
                temps: Default::default(),
                edge_copies: vec![],
            },
            None,
        )
        .unwrap(),
        true,
    )
    .unwrap()
}

#[test]
fn overwritten_modes_reemit_spans_and_invalidate_old_sites() {
    for byte in [false, true] {
        let mut e = TrackedEmitter65816::default();
        #[cfg(feature = "native65816-state-proof")]
        e.trace();
        e.begin_source(BlockId(0), 0);
        e.a8();
        e.span(BlockId(0), 0, 0);
        let start = e.position();
        e.begin_source(BlockId(0), 1);
        e.barrier();
        e.a16();
        if byte {
            e.a8();
        }
        e.byte(ByteOp::StaStack, 2);
        e.span(BlockId(0), 1, start);
        let original = finish(e);
        let old_site = original.selected.as_ref().unwrap().site(Node(0)).unwrap();
        let result = apply(original.clone(), true).unwrap();
        assert!(result.bytes.len() < original.bytes.len());
        assert_eq!(result.mir_spans[&(BlockId(0), 0)], 0..0);
        assert!(
            result
                .selected
                .as_ref()
                .unwrap()
                .validate(old_site)
                .is_err()
        );
        let replayed = layout::finalize(
            replay::emit(result.selected.as_ref().unwrap(), true).unwrap(),
            true,
        )
        .unwrap();
        replay::equivalent(&result, &replayed).unwrap();
        let again = apply(result.clone(), true).unwrap();
        replay::equivalent(&result, &again).unwrap();
    }
}

#[test]
fn labels_status_masks_and_real_instructions_protect_mode_requests() {
    for barrier in 0..5 {
        let mut e = TrackedEmitter65816::default();
        e.a8();
        match barrier {
            0 => {
                let label = e.label();
                e.mark(label);
            }
            1 => e.byte(ByteOp::Rep, 0x01),
            2 => e.byte(ByteOp::Sep, 0x30),
            3 => e.byte(ByteOp::StaStack, 2),
            4 => e.op(Implied::Nop),
            _ => unreachable!(),
        }
        e.a16();
        // Restore X separately; a mixed mask is never treated as a Mode request.
        if barrier == 2 {
            e.byte(ByteOp::Rep, 0x10);
        }
        e.byte(ByteOp::StaStack, 4);
        let original = finish(e);
        assert!(
            overwritten(
                original.selected.as_ref().unwrap().records(),
                &roots(original.selected.as_ref().unwrap().records()).unwrap()
            )
            .is_empty()
        );
        replay::equivalent(&original, &apply(original.clone(), false).unwrap()).unwrap();
    }
}

#[test]
fn empty_successor_request_is_materialized_after_removal() {
    let mut e = TrackedEmitter65816::default();
    e.a8();
    e.a8();
    e.a16();
    e.byte(ByteOp::StaStack, 2);
    let original = finish(e);
    let result = apply(original.clone(), false).unwrap();
    // The redundant A8 request still needs a SEP when its predecessor is removed;
    // this pass cannot claim savings by dropping the demanded width too.
    replay::equivalent(&original, &result).unwrap();
}
