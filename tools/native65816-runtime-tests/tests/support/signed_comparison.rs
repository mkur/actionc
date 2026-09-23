//! Check whole signed selection windows against verified MIR and linked bytes.
//! This deliberately does not broaden the independent CMP window decoder.
use super::*;
use actionc::mir65816::{Mir65816Op, Mir65816Value, emit::Location};
use actionc::nir::NirCompareOp;

pub fn check(p: &native65816::Prepared, c: &native65816::Compiled) -> usize {
    let dispatches = control_flow::dispatches(&p.mir, &c.machine, |id| {
        c.image
            .routines
            .iter()
            .find(|r| r.id == id.0)
            .unwrap()
            .address
    });
    let mut count = 0;
    for m in &c.machine.routines {
        let r = p.mir.routines.iter().find(|r| r.id == m.id).unwrap();
        let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
        let segment = c
            .image
            .segments
            .iter()
            .find(|s| s.address == linked.address)
            .unwrap();
        let bytes = &segment.bytes;
        let long = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], 0]);
        for block in &r.blocks {
            for (i, op) in block.ops.iter().enumerate() {
                let Mir65816Op::Compare {
                    dest,
                    width,
                    signed: true,
                    operation,
                    left,
                    right,
                } = op
                else {
                    continue;
                };
                if width.get() != 2 || matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne) {
                    continue;
                }
                let span = &m.code.mir_spans[&(block.id, i)];
                let mut at = span.start;
                if bytes[at..].starts_with(&[0xc2, 0x20]) {
                    at += 2;
                }
                let (left, right) = if matches!(operation, NirCompareOp::Gt | NirCompareOp::Le) {
                    (right, left)
                } else {
                    (left, right)
                };
                let operand = |v: &Mir65816Value, load: bool| -> Vec<u8> {
                    match v {
                        Mir65816Value::Temp(id, _) => match m.frame.temps[id] {
                            Location::Stack(s) => {
                                vec![if load { 0xa3 } else { 0xe3 }, s.offset as u8]
                            }
                            Location::DirectPage(s) => {
                                vec![if load { 0xa5 } else { 0xe5 }, s.offset as u8]
                            }
                        },
                        Mir65816Value::U8(v) => vec![if load { 0xa9 } else { 0xe9 }, *v, 0],
                        Mir65816Value::U16(v) => {
                            vec![if load { 0xa9 } else { 0xe9 }, *v as u8, (*v >> 8) as u8]
                        }
                        _ => panic!("unhandled signed probe operand {v:?}"),
                    }
                };
                let a = operand(left, true);
                assert_eq!(&bytes[at..at + a.len()], a);
                at += a.len();
                assert_eq!(bytes[at], 0x38);
                at += 1;
                let b = operand(right, false);
                assert_eq!(&bytes[at..at + b.len()], b);
                at += b.len();
                assert_eq!(&bytes[at..at + 3], &[0x70, 4, 0x5c]);
                assert_eq!(long(at + 3), linked.address + at as u32 + 9);
                assert_eq!(&bytes[at + 6..at + 9], &[0x49, 0, 0x80]);
                at += 9;
                let predicate = if matches!(operation, NirCompareOp::Lt | NirCompareOp::Gt) {
                    0x30
                } else {
                    0x10
                };
                let sites: Vec<_> = dispatches
                    .iter()
                    .filter(|d| {
                        d.routine == r.id
                            && (span.start..span.end).contains(&((d.at - linked.address) as usize))
                    })
                    .collect();
                if let [d] = sites[..] {
                    assert_eq!(d.at, linked.address + at as u32);
                    assert_eq!(d.predicate, predicate);
                    assert!(!m.code.mir_spans.contains_key(&(block.id, block.ops.len())));
                    if d.short {
                        assert_eq!(bytes[at], predicate);
                    } else {
                        assert_eq!(&bytes[at..at + 3], &[predicate ^ 0x20, 4, 0x5c]);
                        assert_eq!(long(at + 3), d.target);
                    }
                } else {
                    assert!(sites.is_empty());
                    assert_eq!(&bytes[at..at + 3], &[predicate ^ 0x20, 4, 0x5c]);
                    assert_eq!(long(at + 3), linked.address + at as u32 + 14);
                    assert_eq!(&bytes[at + 6..at + 11], &[0xe2, 0x20, 0xa9, 0, 0x5c]);
                    assert_eq!(long(at + 11), linked.address + at as u32 + 18);
                    let destination = m.frame.temps[dest].stack().unwrap().offset as u8;
                    assert_eq!(
                        &bytes[at + 14..span.end],
                        &[0xe2, 0x20, 0xa9, 1, 0xe2, 0x20, 0x83, destination]
                    );
                }
                count += 1;
            }
        }
    }
    count
}
