
use super::*;
use crate::mir6502::ir::{MirDef, MirValue};

fn materialized(source: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::mir6502::materialize_program(
        crate::mir6502::lower_program(&nir).unwrap(),
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn pointer_writes_keep_general_memory_unknown_and_physical_stores_visible() {
    let mut p = materialized("BYTE v PROC Draw() v=1 RETURN PROC Main() Draw() RETURN");
    let pair = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(0xac),
    });
    let cell = |slot| MirMem::FixedZeroPage(MirFixedZpSlot(slot));
    let store = |slot, n| MirOp::Store {
        dst: MirAddr::Direct(cell(slot)),
        src: MirValue::ConstU8(n),
        width: MirWidth::Byte,
    };
    for op in [
        MirOp::StoreIndirect {
            consumer: pair,
            offset: 1,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
        },
        MirOp::CopyDirectWordToIndirect {
            source: cell(0xa0),
            destination: pair,
            destination_offset: 1,
        },
    ] {
        let bytes = if matches!(op, MirOp::StoreIndirect { .. }) {
            1
        } else {
            2
        };
        p.routines[0].blocks[0].ops = vec![
            store(0xac, 0xff),
            store(0xad, 0xff),
            store(0xe4, 9),
            op,
            store(0xe4, 0),
        ];
        let known = MirKnownCalleeSummaries::analyze_exit_facts(&p, false, Some(&p), true);
        let writes = known.get(p.routines[0].id).unwrap().writes();
        assert!(
            matches!(writes, MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0, bytes)]),
            "{writes:?}"
        );
        assert!(writes.may_write_mem(&cell(0xed), &known.fixed_storage));
        assert!(!writes.preserves_private_fixed_byte(MirFixedZpSlot(0xe4), &known.fixed_storage));
        assert!(writes.preserves_private_fixed_byte(MirFixedZpSlot(0xed), &known.fixed_storage));
    }
}

#[test]
fn forwarded_pointer_is_instantiated_separately_at_each_call() {
    let mut p = materialized(
        "PROC Draw(BYTE POINTER p BYTE n) p^=n RETURN
            PROC Forward(BYTE POINTER p BYTE n) Draw(p,n) RETURN
            PROC Near() Forward($A2,7) RETURN PROC Far() Forward($7100,8) RETURN
            PROC Wrap() Forward($FFFF,9) RETURN
            PROC Branch(BYTE which) IF which THEN Near() ELSE Far() FI RETURN
            PROC Main() Branch(0) Wrap() RETURN",
    );
    for width in [1, 2] {
        if width == 2 {
            let op = p.routines[0].blocks[0]
                .ops
                .iter_mut()
                .find(|op| matches!(op, MirOp::StoreIndirect { .. }))
                .unwrap();
            let MirOp::StoreIndirect { consumer, .. } = op else {
                unreachable!()
            };
            *op = MirOp::CopyDirectWordToIndirect {
                source: MirMem::Absolute(0x680),
                destination: *consumer,
                destination_offset: 0,
            };
        }
        let known = MirKnownCalleeSummaries::analyze_exit_facts(&p, false, Some(&p), true);
        for (name, ranges) in [
            ("Near", vec![(0xA2, width)]),
            ("Far", vec![(0x7100, width)]),
            ("Wrap", vec![(0xFFFF, width)]),
            ("Branch", vec![(0xA2, width), (0x7100, width)]),
        ] {
            let r = p.routines.iter().find(|r| r.name == name).unwrap();
            let writes = known.get(r.id).unwrap().writes();
            assert!(
                matches!(writes, MirKnownMemoryWrites::Pointee { numeric, formals, .. }
                    if formals.is_empty() && numeric.len() == ranges.len() && ranges.iter().all(|r|numeric.contains(r))),
                "{name}: {writes:?}"
            );
            for lane in [0, 0xA2, 0xA3] {
                assert_eq!(
                    writes.preserves_private_fixed_byte(MirFixedZpSlot(lane), &known.fixed_storage),
                    ranges
                        .iter()
                        .all(|&(a, n)| u16::from(lane).wrapping_sub(a) >= n)
                );
            }
        }
        assert!(
            known
                .get(p.routines[1].id)
                .unwrap()
                .writes()
                .has_formal_writes()
        );
    }
}

#[test]
fn tn_screen_store_uses_program_pointer_and_keeps_reviewed_scratch() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
    let ast = crate::includes::load_program_with_includes(path).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let mut p = crate::mir6502::materialize_program(
        crate::mir6502::lower_program(&nir).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let putchar = p.routines.iter().find(|r| r.name == "Putchar").unwrap();
    let known = MirKnownCalleeSummaries::analyze_exit_facts(&p, false, Some(&p), true);
    let writes = known.get(putchar.id).unwrap().writes();
    assert!(
        matches!(writes, MirKnownMemoryWrites::Pointee { ordinary: true, .. }),
        "{writes:?}"
    );
    for slot in [0xa0, 0xe6, 0xe7, 0xff] {
        assert!(!writes.preserves_private_fixed_byte(MirFixedZpSlot(slot), &known.fixed_storage));
    }
    assert!(writes.preserves_private_fixed_byte(MirFixedZpSlot(0xa8), &known.fixed_storage));
    assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0xa8), &known.fixed_storage));
    let id = putchar.id;
    let producer = p.routines.iter_mut().find(|r| r.name == "CalcAdr").unwrap();
    producer.frame.callee_review.as_mut().unwrap().fingerprint ^= 1;
    let stale = MirKnownCalleeSummaries::analyze_exit_facts(&p, false, Some(&p), true);
    assert_eq!(
        stale.get(id).unwrap().writes(),
        &MirKnownMemoryWrites::Unknown
    );
}
