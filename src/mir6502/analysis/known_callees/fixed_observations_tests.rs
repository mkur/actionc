use super::*;
use crate::asm6502::InlineAsmRelocationKind as Kind;
use crate::mir6502::ir::MirInlineAsmTarget as Target;
use crate::nir::SymbolId;

fn global(id: u32, backing: MirGlobalBacking) -> MirGlobal {
    MirGlobal {
        id: SymbolId(id), name: format!("g{id}"), kind: "word".into(),
        width: Some(MirWidth::Word), storage_size: 8, backing, init: None,
    }
}

fn relocation(kind: Kind, target: Target, addend: i32, requires_zero_page: bool) -> MirMachineItem {
    MirMachineItem::Relocation {
        kind, target, addend, requires_zero_page, span: crate::source::Span::new(0, 0),
    }
}

#[test]
fn fixed_observation_pointer_fetches_preserve_identity_aliases_and_wraps() {
    for (opcode, address, high) in [(0xb1, 0xe6, 0xe7), (0x91, 0xe6, 0xe7),
        (0xb1, 0xff, 0x00), (0x6c, 0x12ff, 0x1200)] {
        let zp = opcode != 0x6c;
        let kind = if zp { Kind::Low8 } else { Kind::Absolute16 };
        for (target, addend, alias) in [
            (Target::Absolute(address), 0, false),
            (Target::Memory(MirMem::Global { id: SymbolId(0), offset: 0 }), 0, false),
            (Target::Memory(MirMem::Global { id: SymbolId(2), offset: 1 }), 1, true),
        ] {
            let expected_identity = match &target {
                Target::Memory(mem) => offset_memory(mem.clone(), addend).unwrap(),
                _ => absolute_mem(address),
            };
            let mut p = callee_facts_machine(vec![MirMachineItem::Byte(opcode),
                relocation(kind, target, addend, zp), MirMachineItem::Byte(0x60)]);
            p.globals = vec![
                global(0, MirGlobalBacking::Absolute(if alias { address - 4 } else { address })),
                global(1, MirGlobalBacking::Alias { target: SymbolId(0), offset: 1 }),
                global(2, MirGlobalBacking::Alias { target: SymbolId(1), offset: 1 }),
            ];
            let observations = machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
            assert_eq!(observations.reads[0].base, expected_identity);
            assert_eq!(observations.reads.iter().map(|r|
                (fixed_storage_address(&p, &r.base).unwrap(), r.bytes)).collect::<Vec<_>>(),
                [(address, 1), (high, 1)]);
            assert!(observations.writes.is_empty()); // No inferred pointee region.
            let facts = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
            let summary = facts.get(RoutineId(0)).unwrap();
            if opcode == 0xb1 { assert_eq!(summary.reads(), &MirKnownMemoryReads::Unknown); }
            if opcode == 0x91 { assert_eq!(summary.writes(), &MirKnownMemoryWrites::Unknown); }
        }
    }
}

#[test]
fn fixed_observation_unresolved_invalid_and_selected_operands_stay_conservative() {
    let mem = MirMem::Global { id: SymbolId(0), offset: 0 };
    let mut p = callee_facts_machine(vec![MirMachineItem::Byte(0xb1),
        relocation(Kind::Low8, Target::Memory(mem.clone()), 0, true), MirMachineItem::Byte(0x60)]);
    for backing in [
        MirGlobalBacking::Ordinary { offset: 0 },
        MirGlobalBacking::Alias { target: SymbolId(99), offset: 0 },
        MirGlobalBacking::Alias { target: SymbolId(1), offset: 0 },
        MirGlobalBacking::Alias { target: SymbolId(2), offset: 1 },
    ] {
        p.globals = vec![global(0, backing),
            global(1, MirGlobalBacking::Alias { target: SymbolId(0), offset: 0 }),
            global(2, MirGlobalBacking::Absolute(0xffff))];
        assert_eq!(fixed_storage_address(&p, &mem), None);
        assert!(machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    }
    p.globals = vec![global(0, MirGlobalBacking::Absolute(0xe6))];
    // A selected byte without a full-address/ZP contract stays unresolved.
    for kind in [Kind::Low8, Kind::High8] {
        p.machine_blocks[0].items[1] = relocation(kind, Target::Memory(mem.clone()), 0, false);
        assert!(machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    }
    p.machine_blocks[0].items[1] = relocation(Kind::Low8, Target::Memory(mem.clone()), 8, true);
    assert!(machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none()); // Out of object.
    p.machine_blocks[0].items[1] = relocation(Kind::Low8, Target::Memory(mem.clone()), 0, true);
    p.globals[0].backing = MirGlobalBacking::Absolute(0x100);
    for opcode in [0xa5, 0xb5, 0xb6, 0xa1, 0xb1] {
        p.machine_blocks[0].items[0] = MirMachineItem::Byte(opcode);
        assert!(machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    }
    for mem in [MirMem::Static { id: SymbolId(0), offset: 0 },
        MirMem::Param { id: crate::nir::ParamId(0), offset: 0 },
        MirMem::Local { id: crate::nir::LocalId(0), offset: 0 },
        MirMem::ZeroPage(MirZpSlot(0))] {
        assert_eq!(fixed_storage_address(&p, &mem), None);
    }
    p.globals[0].backing = MirGlobalBacking::Alias { target: SymbolId(1), offset: 0xffff };
    p.globals.push(global(1, MirGlobalBacking::Alias { target: SymbolId(2), offset: 1 }));
    p.globals.push(global(2, MirGlobalBacking::Absolute(0)));
    assert_eq!(fixed_storage_address(&p, &mem), None); // Alias-offset overflow.
}

#[test]
fn fixed_observation_indexed_indirect_keeps_the_full_zero_page() {
    let mut p = callee_facts_machine(vec![MirMachineItem::Byte(0xa1),
        relocation(Kind::Low8, Target::Memory(MirMem::Global { id: SymbolId(0), offset: 0 }), 0, true),
        MirMachineItem::Byte(0x60)]);
    p.globals = vec![global(0, MirGlobalBacking::Absolute(0xff))];
    let observations = machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
    assert_eq!(observations.reads.len(), 1);
    assert_eq!(fixed_storage_address(&p, &observations.reads[0].base), Some(0));
    assert_eq!(observations.reads[0].bytes, 256);
    assert!(observations.writes.is_empty());
}

#[test]
fn symbolic_observation_retains_signed_bases_without_loosening_callee_summaries() {
    use super::observations::{decode_storage_observations, ObservationExtent};
    for opcode in [0xbd, 0xb9, 0x99, 0xfe] {
        let mut p = callee_facts_machine(vec![MirMachineItem::Byte(opcode),
            relocation(Kind::Absolute16, Target::Memory(MirMem::Global { id: SymbolId(0), offset: 0 }), -2, false),
            MirMachineItem::Byte(0x60)]);
        p.globals = vec![global(0, MirGlobalBacking::Ordinary { offset: 0 })];
        let decoded = decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
        let access = decoded.reads.first().or(decoded.writes.first()).unwrap();
        assert_eq!(access.address.base, MirMem::Global { id: SymbolId(0), offset: 0 });
        assert_eq!(access.address.addend, -2);
        assert_eq!(access.address.fixed(&p), None);
        assert_eq!(access.extent, ObservationExtent::Bytes(256));
        assert_eq!(decoded.reads.len(), usize::from(opcode != 0x99));
        assert_eq!(decoded.writes.len(), usize::from(opcode == 0x99 || opcode == 0xfe));
        assert!(flatten_machine_items(&p.machine_blocks[0].items, &p.routines[0], &p).is_none());
        assert!(machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    }
}

#[test]
fn symbolic_observation_pending_pointers_and_invalid_operands_are_distinct() {
    use super::observations::{decode_storage_observations, ObservationExtent};
    let mut p = callee_facts_machine(vec![MirMachineItem::Byte(0x6c),
        relocation(Kind::Absolute16, Target::Memory(MirMem::Global { id: SymbolId(0), offset: 0 }), 0, false)]);
    p.globals = vec![global(0, MirGlobalBacking::Ordinary { offset: 0 })];
    let decoded = decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
    assert_eq!(decoded.reads[0].extent, ObservationExtent::PointerFetch);
    assert_eq!(decoded.reads[0].address.fixed(&p), None);
    for backing in [MirGlobalBacking::Alias { target: SymbolId(99), offset: 0 },
        MirGlobalBacking::Alias { target: SymbolId(0), offset: 0 }] {
        p.globals[0].backing = backing;
        assert!(decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    }
    p.globals[0].backing = MirGlobalBacking::Absolute(1);
    p.machine_blocks[0].items[1] = relocation(Kind::Absolute16,
        Target::Memory(MirMem::Global { id: SymbolId(0), offset: 0 }), -2, false);
    assert!(decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    p.machine_blocks[0].items = vec![MirMachineItem::Byte(0xad), MirMachineItem::Byte(1)];
    assert!(decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
    p.machine_blocks[0].items = vec![MirMachineItem::Byte(0x02)];
    assert!(decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).is_none());
}

#[test]
fn symbolic_observation_fixed_alias_addends_apply_once_and_match_emission() {
    use super::observations::decode_storage_observations;
    let mut p = callee_facts_machine(vec![MirMachineItem::Byte(0xbd),
        relocation(Kind::Absolute16, Target::Memory(MirMem::Global { id: SymbolId(1), offset: 1 }), -2, false),
        MirMachineItem::Byte(0x60)]);
    p.globals = vec![global(0, MirGlobalBacking::Absolute(0x7000)),
        global(1, MirGlobalBacking::Alias { target: SymbolId(0), offset: 3 })];
    let decoded = decode_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
    let address = decoded.reads[0].address.fixed(&p).unwrap();
    assert_eq!(address, 0x7002);
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    crate::mir6502::emit::emit_program(&p, 0x3000, &mut emitter).unwrap();
    let bytes = emitter.finish_with_relocations().unwrap().bytes;
    assert!(bytes.windows(3).any(|b| b == [0xbd, address as u8, (address >> 8) as u8]));
}
