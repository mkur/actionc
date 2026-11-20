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
fn indexed_array_keeps_private_words_except_at_physical_destinations() {
    let mut program = materialized("BYTE ARRAY table(32) BYTE i PROC Paint() table(i)=7 RETURN PROC Main() Paint() RETURN");
    let owner = program.routines.iter().find(|r| r.name == "Paint").unwrap().id;
    let global = program.globals.iter().position(|g| g.name == "table").unwrap();
    for address in [None, Some(0xfff0)] {
        if let Some(a) = address { program.globals[global].backing = crate::mir6502::ir::MirGlobalBacking::Absolute(a); }
        let known = MirKnownCalleeSummaries::analyze_for_source_homes(&program, 0x3000);
        let writes = known.get(owner).unwrap().writes();
        for slot in [0xe4, 0xe5] {
            assert_eq!(writes.preserves_private_fixed_byte(MirFixedZpSlot(slot), known.fixed_storage()), address.is_none(), "{writes:?}");
        }
        assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0xe4), known.fixed_storage()));
    }
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
        let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0);
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
        let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0);
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

fn tn_program() -> MirProgram {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
    let ast = crate::includes::load_program_with_includes(path).unwrap();
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
fn tn_screen_store_uses_program_pointer_and_keeps_reviewed_scratch() {
    let mut p = tn_program();
    let putchar = p.routines.iter().find(|r| r.name == "Putchar").unwrap();
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0);
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
    let stale = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0);
    assert_eq!(
        stale.get(id).unwrap().writes(),
        &MirKnownMemoryWrites::Unknown
    );
}

#[test]
fn a_captured_pointer_uses_the_staged_zp_arguments() {
    let mut p = materialized(
        "PROC Paint(BYTE a,b,c BYTE POINTER dst) dst^=c RETURN
        PROC Apply() Paint(1,2,3,$7100) RETURN PROC Main() Apply() RETURN",
    );
    let call = p.routines[1].blocks[0]
        .ops
        .iter_mut()
        .find(|op| matches!(op, MirOp::Call { .. }))
        .unwrap();
    let MirOp::Call { args, .. } = call else {
        unreachable!()
    };
    // A materialized call retains diagnostic values beside the physical lanes.
    // Deliberately disagree with the bytes actually staged at $A3/$A4.
    args[3].value = MirValue::ConstU8(0xa8);
    args[4].value = MirValue::ConstU8(0);
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0);
    let writes = known.get(p.routines[1].id).unwrap().writes();
    assert!(
        matches!(writes,MirKnownMemoryWrites::Pointee{numeric,formals,..}
        if numeric == &[(0x7100,1)] && formals.is_empty()),
        "{writes:?}"
    );
    assert!(writes.preserves_private_fixed_byte(MirFixedZpSlot(0xa8), &known.fixed_storage));
    assert!(native_numeric_observations(&p).contains(&(0x7100, 1)));
    let capture = p.routines[0].blocks[0]
        .ops
        .iter_mut()
        .find(|op| matches!(op, MirOp::SArgsCapture { .. }))
        .unwrap();
    let MirOp::SArgsCapture { byte_count, .. } = capture else {
        unreachable!()
    };
    *byte_count -= 1;
    let stale = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0);
    assert_eq!(
        stale.get(p.routines[1].id).unwrap().writes(),
        &MirKnownMemoryWrites::Unknown
    );
}

#[test]
fn reviewed_and_native_row_writes_keep_numeric_destinations_and_scratch() {
    let source = concat!(
        include_str!("../../../../../fixtures/runtime/reviewed_fixed_store.act"),
        "
        CARD ARRAY rows(4)
        PROC Forward(CARD p BYTE value) Store(p,value) RETURN
        PROC Row(BYTE i) Forward(rows(i),7) RETURN
        PROC First() Forward(rows(0),7) RETURN
        PROC Near() Forward($A2,7) RETURN
        PROC Mixed(BYTE i) IF i THEN Near() ELSE Row(i) FI RETURN
        PROC Native(BYTE i) BYTE POINTER rowptr rowptr=rows(i) rowptr^=i RETURN
        PROC Main() Mixed(1) Native(1) RETURN"
    );
    let mut p = materialized(source);
    let id = |name: &str| p.routines.iter().find(|r| r.name == name).unwrap().id;
    let (row, near, mixed, native, forward) = (
        id("Row"),
        id("Near"),
        id("Mixed"),
        id("Native"),
        id("Forward"),
    );
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    for (id, numeric) in [
        (row, false),
        (near, true),
        (mixed, true),
        (native, false),
        (id("First"), false),
    ] {
        let writes = known.get(id).unwrap().writes();
        for (byte, preserved) in [
            (0xa0, id == native),
            (0xa1, id == native),
            (0xa2, !numeric),
            (0xef, true),
        ] {
            assert_eq!(
                writes.preserves_private_fixed_byte(MirFixedZpSlot(byte), known.fixed_storage()),
                preserved,
                "{id:?} ${byte:02x} {writes:?}"
            );
        }
        if id != near {
            assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0xef), known.fixed_storage()));
        }
    }
    assert!(known.get(forward).unwrap().writes().has_formal_writes());
    assert!(native_numeric_observations(&p).contains(&(0xa2, 1)));

    // Both register lanes must come from one word. Reusing the other offset
    // is still a valid byte load, but does not reconstruct that word.
    let mut broken = p.clone();
    let row_block = &mut broken
        .routines
        .iter_mut()
        .find(|r| r.id == row)
        .unwrap()
        .blocks[0];
    let lane = row_block
        .ops
        .iter_mut()
        .find_map(|op| match op {
            MirOp::LoadIndirect { offset, .. } if *offset == 1 => Some(offset),
            _ => None,
        })
        .unwrap();
    *lane = 0;
    // The displayed A/X values cannot certify a physical destination when
    // the actual definition of one register comes from the wrong word lane.
    let args = row_block.ops.iter_mut().find_map(|op| match op {
        MirOp::Call { args, .. } => Some(args),
        _ => None,
    }).unwrap();
    args[0].value = MirValue::ConstU8(0x00);
    args[1].value = MirValue::ConstU8(0x71);
    assert_eq!(
        MirKnownCalleeSummaries::analyze_for_source_homes(&broken, 0)
            .get(row)
            .unwrap()
            .writes(),
        &MirKnownMemoryWrites::Unknown
    );

    p.routines[0]
        .frame
        .callee_review
        .as_mut()
        .unwrap()
        .fingerprint ^= 1;
    assert_eq!(
        MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0)
            .get(row)
            .unwrap()
            .writes(),
        &MirKnownMemoryWrites::Unknown
    );
}

#[test]
fn write_through_join_requires_both_entry_pointer_bytes() {
    let mut p = materialized("BYTE out=$680
        PROC Draw(BYTE a,b,c BYTE POINTER dst) BYTE which
          IF a THEN which=b ELSE which=c FI
          IF which THEN dst^==+1 FI RETURN
        PROC Main() Draw(1,2,3,$7100) RETURN");
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    let draw = &p.routines[0];
    let summary = known.get(draw.id).unwrap();
    assert!(matches!(summary.writes(), MirKnownMemoryWrites::Pointee { formals, .. }
        if formals.iter().any(|w| (w.argument, w.offset, w.bytes) == (3, 0, 1))), "{summary:?}");
    assert!(summary.reads().may_read_caller_private_byte(0x7100, known.fixed_storage()));
    let target = draw.frame.params.last().unwrap().base.clone();
    let MirStorageBase::Param(id) = target else { unreachable!() };
    let branch = p.routines[0].blocks.iter_mut().find(|b| b.ops.iter().any(|op|
        matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { .. }), .. }))).unwrap();
    branch.ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Param { id, offset: 1 }),
        src: MirValue::ConstU8(0x82), width: MirWidth::Byte,
    });
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    assert_eq!(known.get(p.routines[0].id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
}

/// Model the contiguous capture frame that the allocator gives a wide ABI.
fn bind_capture(routine: &mut MirRoutine, mut slot: u8) {
    use crate::mir6502::ir::{MirParamBinding, MirParamHome};
    routine.frame.parameter_homes = Some(routine.frame.params.iter().map(|p| {
        let MirStorageBase::Param(id) = p.base else { unreachable!() };
        let home = MirParamHome::ZeroPage(MirFixedZpSlot(slot));
        slot += p.storage_size as u8;
        MirParamBinding { param: id, home }
    }).collect());
}

#[test]
fn loop_write_bound_requires_the_entry_pointer_to_survive_every_iteration() {
    let mut p = materialized("PROC Paint(BYTE a,b,count BYTE POINTER dst)
        WHILE count DO dst^==+1 count==-1 OD RETURN
        PROC Near() Paint(1,2,3,$EF) RETURN
        PROC Alias() Paint(1,2,3,$EE) RETURN
        PROC Main() Near() Alias() RETURN");
    bind_capture(&mut p.routines[0], 0xea);
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    assert!(matches!(known.get(p.routines[1].id).unwrap().writes(),
        MirKnownMemoryWrites::Pointee { numeric, formals, .. }
        if numeric == &[(0xef, 1)] && formals.is_empty()));
    assert_eq!(known.get(p.routines[2].id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
    // A new value in either pointer byte can redirect the next iteration.
    let body = p.routines[0].blocks.iter_mut().find(|b| b.ops.iter().any(|op|
        matches!(op, MirOp::StoreIndirect { .. }))).unwrap();
    body.ops.push(MirOp::Store { dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xed))),
        src: MirValue::ConstU8(0x20), width: MirWidth::Byte });
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    assert_eq!(known.get(p.routines[1].id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
}

#[test]
fn range_loop_uses_the_opaque_keyboard_preservation_for_its_pointer() {
    let mut p = tn_program();
    let index = p.routines.iter().position(|r| r.name == "Range").unwrap();
    let id = p.routines[index].id;
    bind_capture(&mut p.routines[index], 0xea);
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    let writes = known.get(id).unwrap().writes();
    assert!(matches!(writes, MirKnownMemoryWrites::Pointee { formals, opaque_preserves: Some(kept), .. }
        if formals.len() == 1 && formals.iter().all(|w| (w.argument,w.offset,w.bytes) == (3,0,1))
        && kept.contains(&0xef)), "{writes:?}");
    // Move the high byte outside the keyboard's reviewed $E0-$EF range.
    bind_capture(&mut p.routines[index], 0xec);
    let known = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    assert_eq!(known.get(id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
}

#[test]
fn each_call_discharges_forwarded_and_aliased_write_arguments() {
    let mut p = materialized("PROC Paint(BYTE a,b,count BYTE POINTER dst)
          WHILE count DO dst^==+1 count==-1 OD RETURN
        PROC Forward(BYTE POINTER p) Paint(1,2,2,p) RETURN
        PROC Good() BYTE c Forward(@c) RETURN
        PROC Bad() Forward($EE) RETURN
        PROC Either(BYTE POINTER first, second BYTE which)
          IF which THEN first^=1 ELSE second^=2 FI RETURN
        PROC Aliased() Either($A2,$A2,1) RETURN
        PROC Main() Good() Bad() Aliased() RETURN");
    bind_capture(&mut p.routines[0], 0xea);
    let facts = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    let writes = |name| facts.get(p.routines.iter().find(|r| r.name == name).unwrap().id).unwrap().writes();
    assert!(writes("Forward").has_formal_writes());
    assert!(writes("Good").preserves_private_fixed_byte(MirFixedZpSlot(0xef), facts.fixed_storage()), "{:?}", writes("Good"));
    assert_eq!(writes("Bad"), &MirKnownMemoryWrites::Unknown);
    assert!(matches!(writes("Aliased"), MirKnownMemoryWrites::Pointee { numeric, .. }
        if numeric == &[(0xa2, 1)]), "{:?}", writes("Aliased"));
}
