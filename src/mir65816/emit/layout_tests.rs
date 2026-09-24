use super::*;
use crate::mir65816::emit::{ConditionalBranch, Fixup, Label, MirTransfer};
use crate::nir::BlockId;

fn code(at: usize, target: usize) -> Code {
    let mut c = Code::default();
    c.bytes = vec![0xea; (at + 6).max(target + 1)];
    c.bytes[at..at + 6].copy_from_slice(&[0xf0, 4, 0x5c, 0, 0, 0]);
    c.labels.insert(Label(0), target);
    c.boundaries = (0..=c.bytes.len())
        .filter(|&p| !(at < p && p < at + 6))
        .collect();
    c.fixups.push(Fixup {
        offset: at + 3,
        target: Target::Label(Label(0)),
        addend: 0,
        byte: None,
    });
    c.conditional_branches.push(ConditionalBranch {
        offset: at,
        predicate: 0xd0,
        target: Label(0),
        short: false,
        dispatch: false,
    });
    c
}

#[test]
fn disabled_finalizer_is_byte_and_metadata_identical() {
    let mut c = code(0, 12);
    c.mir_spans.insert((BlockId(0), 0), 0..13);
    let before = format!("{c:?}");
    assert_eq!(format!("{:?}", finalize(c, false).unwrap()), before);
}

#[test]
fn signed_displacement_boundaries_include_own_forward_shrink() {
    for d in [-129i64, -128, -127, 0, 126, 127, 128] {
        let at = 200;
        let mut target = (at as i64 + 2 + d) as usize;
        if target >= at + 2 {
            target += 4;
        }
        let old = code(at, target);
        let original = old.bytes.clone();
        let c = finalize(old, true).unwrap();
        let fits = (-128..=127).contains(&d);
        assert_eq!(c.conditional_branches[0].short, fits, "{d}");
        if fits {
            assert_eq!(&c.bytes[at..at + 2], &[0xd0, d as i8 as u8]);
            assert!(c.fixups.is_empty());
            assert_eq!(c.bytes.len(), original.len() - 4);
        } else {
            assert_eq!(c.bytes, original);
            assert_eq!(c.fixups.len(), 1);
        }
    }
}

#[test]
fn cascading_shortening_reaches_a_deterministic_fixed_point() {
    let mut c = code(0, 136);
    c.bytes[20..26].copy_from_slice(&[0x90, 4, 0x5c, 0, 0, 0]);
    c.labels.insert(Label(1), 30);
    c.boundaries.retain(|&p| !(20 < p && p < 26));
    c.fixups.push(Fixup {
        offset: 23,
        target: Target::Label(Label(1)),
        addend: 0,
        byte: None,
    });
    c.conditional_branches.push(ConditionalBranch {
        offset: 20,
        predicate: 0xb0,
        target: Label(1),
        short: false,
        dispatch: false,
    });
    let a = finalize(c.clone(), true).unwrap();
    let b = finalize(c, true).unwrap();
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    assert!(a.conditional_branches.iter().all(|s| s.short));
    assert_eq!(&a.bytes[..2], &[0xd0, 126]);
    assert_eq!(&a.bytes[16..18], &[0xb0, 4]);
}

#[test]
fn one_mapping_preserves_fixups_per_spans_coincident_labels_and_transfers() {
    let mut c = code(0, 16);
    c.bytes[6..10].copy_from_slice(&[0x22, 0, 0, 0]);
    c.fixups.push(Fixup {
        offset: 7,
        target: Target::Routine(crate::nir::RoutineId(7)),
        addend: 0,
        byte: None,
    });
    c.bytes[10..13].copy_from_slice(&[0x62, 0, 0]);
    c.return_fixups.push((11, Label(0)));
    c.labels.insert(Label(1), 16);
    c.mir_spans.insert((BlockId(0), 0), 0..16);
    c.mir_spans.insert((BlockId(1), 0), 16..16);
    c.mir_transfers.push(MirTransfer {
        source: Label(2),
        target: Label(0),
        offset: 16,
        fallthrough: true,
    });
    let c = finalize(c, true).unwrap();
    assert_eq!(c.labels[&Label(0)], 12);
    assert_eq!(c.labels[&Label(1)], 12);
    assert_eq!(c.fixups[0].offset, 3);
    assert_eq!(c.return_fixups, [(7, Label(0))]);
    assert_eq!(c.mir_spans[&(BlockId(0), 0)], 0..12);
    assert_eq!(c.mir_spans[&(BlockId(1), 0)], 12..12);
    assert_eq!(c.mir_transfers[0].offset, 12);
}

#[test]
fn malformed_metadata_cannot_be_erased_by_relaxation() {
    let mut c = code(0, 12);
    c.labels.clear();
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.labels.insert(Label(1), 2);
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.boundaries.remove(&12);
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.fixups[0].addend = 1;
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.fixups.push(Fixup {
        offset: 3,
        target: Target::StackOverflow,
        addend: 0,
        byte: None,
    });
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.return_fixups.push((3, Label(0)));
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.mir_spans.insert((BlockId(0), 0), 0..3);
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.conditional_branches.push(c.conditional_branches[0]);
    assert!(finalize(c, true).is_err());
    let mut c = code(0, 12);
    c.conditional_branches[0].offset = usize::MAX;
    assert!(finalize(c, true).is_err());
}

#[test]
fn placed_branches_require_same_bank_and_24_bit_addresses() {
    let c = finalize(code(0, 12), true).unwrap();
    validate_branches(&c, Some(0x01fff6)).unwrap();
    validate_branches(&c, Some(0xfffff6)).unwrap();
    for base in [0x01fffe, 0x01fffa, 0xfffffa, 0x1000000] {
        assert!(validate_branches(&c, Some(base)).is_err(), "{base:x}");
    }
}

#[cfg(feature = "native65816-state-proof")]
#[test]
fn trace_events_keep_their_order_and_claims_at_remapped_boundaries() {
    use crate::mir65816::emit::tracked::{Branch, Implied, TrackedEmitter65816, WordOp};
    let mut e = TrackedEmitter65816::default();
    e.trace();
    e.a16();
    e.word(WordOp::LdaImm, 0x8000);
    let target = e.label();
    e.dispatch(Branch::NotEqual, target);
    e.op(Implied::Nop);
    e.mark(target);
    e.op(Implied::Nop);
    let code = e.finish();
    let at = code.conditional_branches[0].offset;
    let mut expected = code.state_trace.clone();
    for s in &mut expected {
        if s.pc >= at + 6 {
            s.pc -= 4;
        }
    }
    let mut bad = code.clone();
    bad.state_trace[0].pc = at + 3;
    assert!(finalize(bad, true).is_err());
    assert_eq!(finalize(code, true).unwrap().state_trace, expected);
}

fn jump(at: usize, target: usize) -> Code {
    use crate::mir65816::emit::LocalJump;
    let mut c = Code::default();
    c.bytes = vec![0xea; (at + 4).max(target + 1)];
    c.bytes[at..at + 4].copy_from_slice(&[0x5c, 0, 0, 0]);
    c.labels.insert(Label(0), target);
    c.boundaries = (0..=c.bytes.len())
        .filter(|&p| !(at < p && p < at + 4))
        .collect();
    c.fixups.push(Fixup {
        offset: at + 1,
        target: Target::Label(Label(0)),
        addend: 0,
        byte: None,
    });
    c.local_jumps.push(LocalJump {
        offset: at,
        target: Label(0),
        encoding: JumpEncoding::Long,
    });
    c
}

#[test]
fn local_jump_signed_limits_and_same_bank_placement() {
    for d in [
        -32769i64, -32768, -32767, -129, -128, -127, 0, 126, 127, 128, 32766, 32767, 32768,
    ] {
        // Forward distances are measured after shrinking the candidate itself.
        let at = if d < 0 { 33000 } else { 0 };
        let target = (at as i64 + if d < 0 { 3 } else { 4 } + d) as usize;
        let raw = jump(at, target);
        let c = finalize(raw.clone(), true).unwrap();
        let bra_delta = if d < 0 { d + 1 } else { d };
        let expected = if i8::try_from(bra_delta).is_ok() {
            JumpEncoding::Relative8
        } else if i16::try_from(d).is_ok() {
            JumpEncoding::Relative16
        } else {
            JumpEncoding::Long
        };
        assert_eq!(c.local_jumps[0].encoding, expected, "{d}");
        assert_eq!(c.bytes.len(), raw.bytes.len() - 4 + expected.size());
        assert_eq!(c.fixups.len(), usize::from(expected == JumpEncoding::Long));
        validate_branches(&c, Some(0x010000)).unwrap();
        validate_branches(&c, Some(0xff0000)).unwrap();
        if expected != JumpEncoding::Long {
            let delta = c.labels[&Label(0)] as i64 - (at + expected.size()) as i64;
            let encoded = if expected == JumpEncoding::Relative8 {
                c.bytes[at + 1] as i8 as i64
            } else {
                i16::from_le_bytes(c.bytes[at + 1..at + 3].try_into().unwrap()) as i64
            };
            assert_eq!(delta, encoded);
        }
    }
    let c = finalize(jump(0, 12), true).unwrap();
    for base in [0x01fffe, 0x01fffa, 0xfffffa, 0x1000000] {
        assert!(validate_branches(&c, Some(base)).is_err());
    }
}

#[test]
fn mixed_conditional_and_jump_relaxation_cascades_and_keeps_all_metadata() {
    let mut c = code(0, 136);
    c.bytes[20..24].copy_from_slice(&[0x5c, 0, 0, 0]);
    c.labels.insert(Label(1), 30);
    c.boundaries.retain(|&p| !(20 < p && p < 24));
    c.fixups.push(Fixup {
        offset: 21,
        target: Target::Label(Label(1)),
        addend: 0,
        byte: None,
    });
    c.local_jumps.push(crate::mir65816::emit::LocalJump {
        offset: 20,
        target: Label(1),
        encoding: JumpEncoding::Long,
    });
    // Two bytes from the jump leave the first branch just outside range.
    // Another internal conditional supplies the remaining reduction.
    c.bytes[40..46].copy_from_slice(&[0x70, 4, 0x5c, 0, 0, 0]);
    c.labels.insert(Label(2), 50);
    c.boundaries.retain(|&p| !(40 < p && p < 46));
    c.fixups.push(Fixup {
        offset: 43,
        target: Target::Label(Label(2)),
        addend: 0,
        byte: None,
    });
    c.conditional_branches.push(ConditionalBranch {
        offset: 40,
        predicate: 0x50,
        target: Label(2),
        short: false,
        dispatch: false,
    });
    c.mir_spans.insert((BlockId(0), 0), 0..136);
    let after = finalize(c.clone(), true).unwrap();
    assert!(after.conditional_branches.iter().all(|s| s.short));
    assert_eq!(after.local_jumps[0].encoding, JumpEncoding::Relative8);
    assert_eq!(after.bytes.len(), c.bytes.len() - 10);
    assert_eq!(after.mir_spans[&(BlockId(0), 0)], 0..126);
    assert_eq!(
        format!("{after:?}"),
        format!("{:?}", finalize(c, true).unwrap())
    );
}

#[test]
fn jump_validation_rejects_corruption_overlaps_and_erased_metadata() {
    for problem in 0..10 {
        let mut c = jump(0, 16);
        match problem {
            0 => {
                c.local_jumps[0].offset = usize::MAX;
            }
            1 => {
                c.labels.insert(Label(0), c.bytes.len());
            }
            2 => {
                c.labels.insert(Label(1), 2);
            }
            3 => {
                c.boundaries.remove(&16);
            }
            4 => {
                c.fixups[0].addend = 1;
            }
            5 => {
                c.fixups.push(c.fixups[0].clone());
            }
            6 => {
                c.return_fixups.push((1, Label(0)));
            }
            7 => {
                c.local_jumps.push(c.local_jumps[0]);
            }
            8 => {
                c.mir_spans.insert((BlockId(0), 0), 0..2);
            }
            9 => {
                c.bytes[0] = 0x22;
            }
            _ => unreachable!(),
        }
        assert!(finalize(c, true).is_err(), "{problem}");
    }
    for target in [16, 300] {
        let c = finalize(jump(0, target), true).unwrap();
        let mut corrupt = c.clone();
        corrupt.bytes[1] ^= 1;
        assert!(validate_branches(&corrupt, None).is_err());
        let mut overlap = c.clone();
        overlap.fixups.push(Fixup {
            offset: 1,
            target: Target::StackOverflow,
            addend: 0,
            byte: Some(0),
        });
        assert!(validate_branches(&overlap, None).is_err());
        assert!(finalize(c, true).is_err());
    }
}

#[test]
fn a_provisional_brl_becomes_bra_after_an_intervening_conditional_shrinks() {
    let mut c = jump(0, 134);
    c.bytes[20..26].copy_from_slice(&[0x70, 4, 0x5c, 0, 0, 0]);
    c.labels.insert(Label(1), 30);
    c.boundaries.retain(|&p| !(20 < p && p < 26));
    c.fixups.push(Fixup {
        offset: 23,
        target: Target::Label(Label(1)),
        addend: 0,
        byte: None,
    });
    c.conditional_branches.push(ConditionalBranch {
        offset: 20,
        predicate: 0x50,
        target: Label(1),
        short: false,
        dispatch: false,
    });
    let final_code = finalize(c, true).unwrap();
    assert_eq!(final_code.local_jumps[0].encoding, JumpEncoding::Relative8);
    assert_eq!(&final_code.bytes[..2], &[0x80, 126]);
    assert!(final_code.conditional_branches[0].short);
}
