use super::*;

pub(in crate::mir6502) fn program() -> MirProgram {
    lower(include_str!(
        "../../../../../fixtures/runtime/bounded_call_pointer_retention.act"
    ))
}
fn lower(text: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&raw).unwrap();
    crate::mir6502::lower_program(&raw).unwrap()
}
fn machine(program: &MirProgram) -> BoundedMachine {
    BoundedMachine::analyze(
        program
            .routines
            .iter()
            .find(|r| r.name == "CopyBytes")
            .unwrap(),
        program,
    )
    .unwrap()
}
fn memory() -> MirPhysicalMemory {
    let mut map = MirPhysicalMemory::default();
    map.own_data(0x7000, 512);
    map
}
fn cells(count: Option<u8>) -> BTreeMap<u16, u8> {
    let mut cells = BTreeMap::from([(0xA3, 0x72)]);
    if let Some(count) = count {
        cells.insert(0xA4, count);
    }
    cells
}

#[test]
fn bounded_machine_copy_records_all_destinations_and_scratch_without_name_assumptions() {
    let p = program();
    let code = machine(&p);
    for count in [None, Some(0), Some(1), Some(2), Some(255)] {
        let writes = code
            .writes([Some(0), Some(0x70), Some(0)], cells(count), &memory())
            .unwrap();
        let end = if count == Some(1) { 0x7001 } else { 0x7100 };
        let expected = (0x7000..end).chain(0xA0..=0xA2).collect::<BTreeSet<_>>();
        assert_eq!(writes, expected);
        assert!(!writes.contains(&0xAC));
        assert!(!writes.contains(&0xAE));
    }
    let mut renamed = p.clone();
    renamed
        .routines
        .iter_mut()
        .find(|r| r.id == code.owner)
        .unwrap()
        .name = "AnyLeaf".into();
    assert_eq!(
        BoundedMachine::analyze(
            renamed
                .routines
                .iter()
                .find(|r| r.id == code.owner)
                .unwrap(),
            &renamed
        ),
        Some(code.clone())
    );
    let text = include_str!("../../../../../fixtures/runtime/bounded_call_pointer_retention.act")
        .replace("\r\n", "\n");
    assert_eq!(machine(&lower(&text.replace('\n', "\r\n"))), code);
}

#[test]
fn bounded_machine_copy_rejects_unknown_or_unowned_destinations_and_unsafe_reads() {
    let code = machine(&program());
    assert!(
        code.writes([None, Some(0x70), Some(0)], cells(None), &memory())
            .is_none()
    );
    assert!(
        code.writes([Some(0), None, Some(0)], cells(None), &memory())
            .is_none()
    );
    assert!(
        code.writes([Some(0), Some(0x70), None], cells(None), &memory())
            .is_none()
    );
    let mut missing = cells(None);
    missing.remove(&0xA3);
    assert!(
        code.writes([Some(0), Some(0x70), Some(0)], missing, &memory())
            .is_none()
    );
    for start in [0x00ACu16, 0x0100, 0x0300, 0x8000, 0xD000, 0xFFFF] {
        let mut map = MirPhysicalMemory::default();
        map.own_data(start, 512);
        let [lo, hi] = start.to_le_bytes();
        assert!(
            code.writes([Some(lo), Some(hi), Some(0)], cells(None), &map)
                .is_none()
        );
    }
    let mut short = MirPhysicalMemory::default();
    short.own_data(0x7000, 255);
    assert!(
        code.writes([Some(0), Some(0x70), Some(0)], cells(None), &short)
            .is_none()
    );
    assert!(
        code.writes([Some(0), Some(0x70), Some(0)], cells(Some(1)), &short)
            .is_some()
    );
    for high in [0x80, 0xD0, 0xFF] {
        let mut c = cells(None);
        c.insert(0xA3, high);
        assert!(
            code.writes([Some(0), Some(0x70), Some(0xFF)], c, &memory())
                .is_none()
        );
    }
}

#[test]
fn bounded_machine_copy_declines_unsupported_and_malformed_control_flow() {
    let code = machine(&program());
    for (index, byte) in [(0, 0x48), (8, 0xF8), (10, 0xFF), (17, 0xF8)] {
        let mut changed = code.clone();
        changed.bytes[index].value = Some(byte);
        assert!(
            changed
                .writes([Some(0), Some(0x70), Some(0)], cells(None), &memory())
                .is_none(),
            "{index}"
        );
    }
    let mut changed = code.clone();
    changed.bytes[10].value = Some(0xFF);
    assert!(
        changed
            .writes([Some(0), Some(0x70), Some(0)], cells(None), &memory())
            .is_none()
    );
    let p = program();
    let mut r = p
        .routines
        .iter()
        .find(|r| r.id == code.owner)
        .unwrap()
        .clone();
    r.effects.opaque = true;
    assert!(BoundedMachine::analyze(&r, &p).is_none());
}

#[test]
fn bounded_machine_copy_rejects_incoming_flag_observation_before_any_definition() {
    let source = include_str!("../../../../../fixtures/runtime/bounded_call_pointer_retention.act")
        .replace("\r\n", "\n");
    for branch in ["bcc", "bcs", "beq", "bne", "bmi", "bpl", "bvc", "bvs"] {
        let text = source
            .replace("  sta $A0", &format!("  {branch} done\n  sta $A0"))
            .replace("  rts", "done:\n  rts");
        let p = lower(&text);
        assert!(
            machine(&p)
                .writes([Some(0), Some(0x70), Some(0)], cells(None), &memory())
                .is_none(),
            "{branch}"
        );
    }
}

#[test]
fn bounded_machine_copy_rejects_unresolved_relocation_addends_and_byte_selectors() {
    let original = program();
    for high in [false, true] {
        let mut p = original.clone();
        let relocation = p.machine_blocks[0]
            .items
            .iter_mut()
            .find(|item| matches!(item, MirMachineItem::Relocation { .. }))
            .unwrap();
        let MirMachineItem::Relocation { addend, kind, .. } = relocation else {
            unreachable!()
        };
        if high {
            *kind = crate::asm6502::InlineAsmRelocationKind::High8;
        } else {
            *addend = 1;
        }
        let r = p.routines.iter().find(|r| r.name == "CopyBytes").unwrap();
        assert!(BoundedMachine::analyze(r, &p).is_none());
    }
    let mut code = machine(&original);
    code.bytes[1].memory = Some(MirMem::Absolute(0x70AC));
    code.bytes[1].value = Some(0xAC);
    assert!(
        code.writes([Some(0), Some(0x70), Some(0)], cells(None), &memory())
            .is_none()
    );
}

fn ranged_words(
    dst: (u16, u16),
    src: (u16, u16),
) -> Vec<crate::mir6502::analysis::address_ranges::CoherentWordRange> {
    use crate::mir6502::analysis::address_ranges::{AbiByte::*, CoherentWordRange};
    vec![
        CoherentWordRange {
            lo: Register(MirReg::A),
            hi: Register(MirReg::X),
            min: dst.0,
            max: dst.1,
        },
        CoherentWordRange {
            lo: Register(MirReg::Y),
            hi: Cell(0xA3),
            min: src.0,
            max: src.1,
        },
    ]
}
#[test]
fn whole_word_intervals_include_the_tail_and_require_a_complete_code_map() {
    let code = machine(&program());
    let words = ranged_words((0x2104, 0x280B), (0x2104, 0x270C));
    let mut map = MirPhysicalMemory::default();
    assert!(
        code.writes_with_ranges([None; 3], BTreeMap::new(), &map, &words)
            .is_none()
    );
    map.protect_execution_ranges(vec![(0x3000, 0x4000)]);
    for count in [None, Some(0), Some(1), Some(255)] {
        let cells = count
            .map(|n| BTreeMap::from([(0xA4, n)]))
            .unwrap_or_default();
        let writes = code
            .writes_with_ranges([None; 3], cells, &map, &words)
            .unwrap();
        let last = if count == Some(1) { 0x280B } else { 0x290A };
        assert_eq!(writes, (0x2104..=last).chain(0xA0..=0xA2).collect());
        assert!(writes.contains(&0x280B)); // end of arena is not an ownership limit
    }
    assert!(!map.contains_write(0x2104, 1)); // the old object proof is untouched
    map.protect_execution_ranges(vec![(0x290A, 0x290B)]);
    assert!(
        code.writes_with_ranges([None; 3], BTreeMap::new(), &map, &words)
            .is_none()
    );
    map.protect_execution_ranges(vec![(0x270C, 0x270D)]);
    assert!(
        code.writes_with_ranges([None; 3], BTreeMap::new(), &map, &words)
            .is_none()
    );
}
#[test]
fn interval_copy_rejects_unsafe_ranges_mixed_lanes_and_excessive_work() {
    let code = machine(&program());
    let mut map = MirPhysicalMemory::default();
    map.protect_execution_ranges(vec![]);
    for bad in [
        (0x00AC, 0x00AC),
        (0x100, 0x200),
        (0x300, 0x3FF),
        (0x7F01, 0x7FFF),
        (0xD000, 0xD100),
        (0xFFF0, 0xFFFF),
        (0x2800, 0x2100),
    ] {
        for words in [
            ranged_words(bad, (0x2200, 0x2300)),
            ranged_words((0x2200, 0x2300), bad),
        ] {
            assert!(
                code.writes_with_ranges([None; 3], BTreeMap::new(), &map, &words)
                    .is_none(),
                "{bad:?}"
            );
        }
    }
    assert!(
        code.writes_with_ranges(
            [None; 3],
            BTreeMap::new(),
            &map,
            &ranged_words((0x400, 0x7F00), (0x400, 0x7F00))
        )
        .is_none()
    );
    let words = ranged_words((0x2104, 0x280B), (0x2104, 0x270C));
    let mut mixed = code.clone();
    mixed.bytes[3].value = Some(0xA2);
    mixed.bytes[3].memory = Some(MirMem::Absolute(0xA2)); // STX overwrites the low lane of the destination
    assert!(
        mixed
            .writes_with_ranges([None; 3], BTreeMap::new(), &map, &words)
            .is_none()
    );
    assert!(
        code.writes_with_ranges([Some(0), Some(0x10), None], BTreeMap::new(), &map, &words)
            .is_none()
    );
}
