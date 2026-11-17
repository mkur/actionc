use super::*;
use crate::mir6502::ir::{MirAddressConsumer, MirBinaryOp, MirPointerPair, MirRuntimeHelper, MirTempId};

fn pointer(lo: u8) -> MirAddressConsumer {
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo: MirFixedZpSlot(lo) })
}
fn store(consumer: MirAddressConsumer, src: MirValue) -> MirOp {
    MirOp::StoreIndirect { consumer, src, offset: 0 }
}
fn read_facts(op: MirOp) -> (MirKnownMemoryReads, MirKnownMemoryWrites) {
    let p = program(vec![routine(0, vec![op], MirTerminator::Return)], vec![]);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let summary = known.get(RoutineId(0)).unwrap();
    (summary.reads().clone(), summary.writes().clone())
}
fn exact(reads: &MirKnownMemoryReads, expected: Vec<MirMem>) {
    let MirKnownMemoryReads::Exact(actual) = reads else { panic!("expected {expected:?}, got {reads:?}") };
    assert_eq!(actual.len(), expected.len(), "{reads:?}");
    for byte in expected { assert!(actual.contains(&byte), "missing {byte:?}: {reads:?}"); }
}

fn emitted_cases() -> Vec<(MirOp, Vec<MirMem>)> {
    let local = |offset| MirMem::Local { id: crate::nir::LocalId(0), offset };
    vec![
        (store(pointer(0xac), MirValue::Def(MirDef::Reg(MirReg::A))), vec![absolute_mem(0xac), absolute_mem(0xad)]),
        (MirOp::CopyDirectWordToIndirect { source: local(0),
            destination: pointer(0xac), destination_offset: 3 }, vec![local(0), local(1), absolute_mem(0xac), absolute_mem(0xad)]),
        (MirOp::AbsoluteWordSubToIndirect { source: MirMem::Absolute(0x600), rhs: local(0),
            destination: pointer(0xac), destination_offset: 3 }, vec![MirMem::Absolute(0x600), MirMem::Absolute(0x601), local(0), local(1), absolute_mem(0xac), absolute_mem(0xad)]),
    ]
}

#[test]
fn directional_store_reads_cover_emitted_operands_without_reading_the_pointee() {
    for (op, expected) in emitted_cases() {
        let mut p = program(vec![routine(0, vec![op.clone()], MirTerminator::Return)], vec![]);
        p.routines[0].frame.fixed_zero_page = vec![MirFixedZpSlot(0xac), MirFixedZpSlot(0xad)];
        p.routines[0].frame.locals.push(crate::mir6502::ir::MirStorageSlot {
            purpose: Default::default(), id: crate::mir6502::ir::MirStorageId(0), name: Some("word".into()),
            storage: crate::mir6502::ir::MirStorageClass::Scalar, storage_size: 2, scalar_width: Some(MirWidth::Word),
            base: MirStorageBase::Local(crate::nir::LocalId(0)), offset: 0, mutable: true, init: None,
        });
        for facts in [MirKnownCalleeSummaries::analyze(&p), MirKnownCalleeSummaries::analyze_for_rewriting(&p)] {
            let summary = facts.get(RoutineId(0)).unwrap();
            exact(summary.reads(), expected.clone());
            assert_eq!(summary.writes(), &MirKnownMemoryWrites::Unknown);
            assert!(!summary.writes().preserves_fixed_byte(MirFixedZpSlot(0xe4), facts.fixed_storage()));
        }
        let origin = 0x3000;
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
        let emitted = crate::mir6502::emit::emit_program_with_physical_memory(&p, origin, &mut emitter, true).unwrap();
        let bytes = emitter.finish_with_relocations().unwrap().bytes;
        let mut offset = usize::from(emitted.routine_addresses[0].address - origin);
        let mut reads = BTreeSet::new();
        let mut stack = 0i32;
        while offset < bytes.len() {
            let (name, mode, len) = crate::codegen::decode_6502_opcode(bytes[offset]).unwrap();
            if name == "RTS" { break; }
            if name == "JMP" && mode == AddressingMode::Absolute {
                offset = usize::from(u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]) - origin);
                continue; // Entry jump over ordinary inline storage.
            }
            assert!(!matches!(name, "JSR" | "BRK"), "unexpected call in {op:?}");
            match name { "PHA" | "PHP" => stack += 1, "PLA" | "PLP" => stack -= 1, _ => {} }
            assert!(stack >= 0, "incoming stack read in {op:?}");
            if name == "STA" && mode == AddressingMode::IndirectIndexedY {
                let lo = bytes[offset + 1];
                reads.extend([u16::from(lo), u16::from(lo.wrapping_add(1))]);
            } else if matches!(name, "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "CPX" | "CPY" | "BIT") {
                match mode {
                    AddressingMode::ZeroPage => { reads.insert(u16::from(bytes[offset + 1])); },
                    AddressingMode::Absolute => { reads.insert(u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]])); },
                    AddressingMode::Immediate => {},
                    _ => panic!("unbounded emitted read {name} {mode:?} in {op:?}"),
                }
            }
            offset += len;
        }
        assert_eq!(stack, 0);
        assert_eq!(reads, expected.iter().map(|mem| emitted.physical_memory.address(RoutineId(0), mem).unwrap()).collect(), "{op:?}");
    }
}

#[test]
fn directional_store_projection_retains_typed_sources_and_plain_store_address_reads() {
    let source = MirMem::Global { id: crate::nir::SymbolId(0), offset: 3 };
    let (reads, writes) = read_facts(store(pointer(0xac), MirValue::PointerCell(source.clone())));
    exact(&reads, vec![source.clone(), absolute_mem(0xac), absolute_mem(0xad)]);
    assert_eq!(writes, MirKnownMemoryWrites::Unknown);
    let (reads, writes) = read_facts(MirOp::Store {
        dst: MirAddr::PointerCell { ptr: source.clone(), offset: 5 },
        src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte });
    exact(&reads, vec![source.clone(), offset_mem(&source, 1)]);
    assert_eq!(writes, MirKnownMemoryWrites::Unknown);
    let (reads, writes) = read_facts(MirOp::Store {
        dst: MirAddr::AbsoluteIndexedY { base: source },
        src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte });
    exact(&reads, vec![]); // Indexed destination address does not read its contents.
    assert_eq!(writes, MirKnownMemoryWrites::Unknown);
}

#[test]
fn directional_read_projection_keeps_real_unknowns_and_unexpanded_inputs() {
    let source = pointer(0xac);
    for op in [
        MirOp::LoadIndirect { consumer: source, dst: MirDef::Reg(MirReg::A), offset: 0 },
        MirOp::CopyIndirectWord { source, destination: pointer(0xae), source_offset: 0, destination_offset: 0 },
        MirOp::IndirectByteCompound { op: MirBinaryOp::Add, target: pointer(0xae), source, offset: 0 },
        MirOp::Barrier { effects: MirEffects::default() },
        MirOp::MachineBlock { id: MirMachineBlockId(99), effects: MirEffects::default() },
        MirOp::RuntimeHelper { helper: MirRuntimeHelper::SArgs, args: vec![], result: None, additional_results: vec![],
            effects: MirEffects { opaque: true, ..Default::default() } },
        store(source, MirValue::Def(MirDef::VTemp(MirTempId(0)))),
        store(source, MirValue::PointerCell(MirMem::ZeroPage(MirZpSlot(99)))),
        MirOp::Store { dst: MirAddr::PointerCell { ptr: MirMem::Absolute(0x700), offset: 0 },
            src: MirValue::PointerCell(MirMem::Absolute(0x600)), width: MirWidth::Word },
        store(pointer(0xff), MirValue::ConstU8(1)),
    ] {
        assert_eq!(read_facts(op.clone()).0, MirKnownMemoryReads::Unknown, "{op:?}");
    }
    for source in [MirMem::Absolute(0xffff), absolute_mem(0xff),
        MirMem::Global { id: crate::nir::SymbolId(0), offset: 0xffff }] {
        for op in [MirOp::CopyDirectWordToIndirect { source: source.clone(), destination: pointer(0xac), destination_offset: 0 },
            MirOp::AbsoluteWordSubToIndirect { source, rhs: MirMem::Absolute(0x700), destination: pointer(0xac), destination_offset: 0 }] {
            assert_eq!(read_facts(op.clone()).0, MirKnownMemoryReads::Unknown, "{op:?}");
        }
    }
    exact(&read_facts(store(pointer(0xfe), MirValue::ConstU8(1))).0, vec![absolute_mem(0xfe), absolute_mem(0xff)]);
}

#[test]
fn directional_read_projection_resolves_every_current_virtual_carrier_lane() {
    let slot = MirZpSlot(0);
    let op = store(MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(slot)), MirValue::ConstU8(1));
    for (start, size, valid) in [(0xe4, 0, false), (0xe4, 1, false), (0xe4, 2, true), (0xfe, 2, true), (0xff, 2, false)] {
        let mut r = routine(0, vec![op.clone()], MirTerminator::Return);
        r.frame.virtual_zero_page.push(slot);
        if size != 0 { r.frame.zero_page_allocations.push(MirZpAllocation { slot, start: MirFixedZpSlot(start), size }); }
        let p = program(vec![r], vec![]);
        let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        let reads = known.get(RoutineId(0)).unwrap().reads();
        if valid { exact(reads, vec![absolute_mem(u16::from(start)), absolute_mem(u16::from(start) + 1)]); }
        else { assert_eq!(reads, &MirKnownMemoryReads::Unknown, "{start:02x} size={size}"); }
    }
}

fn call(target: u32, effects: MirEffects) -> MirOp {
    MirOp::Call { target: MirCallTarget::Routine(RoutineId(target)), args: vec![], result: None, additional_results: vec![],
        abi: MirCallAbi { library: None, params: vec![], result: None, additional_results: vec![], clobbers: Default::default(), preserves: Default::default() }, effects }
}

#[test]
fn directional_read_facts_compose_and_keep_structured_uncertainty_separate() {
    let p = program(vec![routine(0, vec![store(pointer(0xac), MirValue::ConstU8(1))], MirTerminator::Return),
        routine(1, vec![call(0, MirEffects { memory_reads: MirMemoryEffect::Unknown, memory_writes: MirMemoryEffect::Unknown, ..Default::default() })], MirTerminator::Return)], vec![]);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    exact(known.get(RoutineId(1)).unwrap().reads(), vec![absolute_mem(0xac), absolute_mem(0xad)]);
    assert_eq!(known.get(RoutineId(1)).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
    let finite = MirMemoryEffect::Regions(vec![crate::mir6502::ir::MirMemoryRegion {
        kind: MirMemoryRegionKind::ZeroPage, offset: 0xe4, size: 2 }]);
    for (read_effect, write_effect, opaque, finite_reads, finite_writes) in [
        (finite.clone(), MirMemoryEffect::Unknown, false, true, false),
        (MirMemoryEffect::Unknown, finite.clone(), false, false, true),
        (finite.clone(), MirMemoryEffect::All, false, true, false),
        (MirMemoryEffect::All, finite.clone(), false, false, true),
        (finite.clone(), finite.clone(), true, false, false),
    ] {
        let effects = MirEffects { memory_reads: read_effect, memory_writes: write_effect, opaque, ..Default::default() };
        for op in [call(99, effects.clone()), MirOp::RuntimeHelper { helper: MirRuntimeHelper::SArgs, args: vec![], result: None, additional_results: vec![], effects }] {
            let (reads, writes) = read_facts(op);
            if finite_reads { exact(&reads, vec![absolute_mem(0xe4), absolute_mem(0xe5)]); }
            else { assert_eq!(reads, MirKnownMemoryReads::Unknown); }
            assert_eq!(matches!(writes, MirKnownMemoryWrites::Exact(_)), finite_writes);
        }
    }
}

#[test]
fn directional_reads_do_not_publish_finite_unexpanded_branch_operands() {
    use crate::mir6502::ir::{MirCond, MirEdge};
    let p = program(vec![routine(0, vec![store(pointer(0xac), MirValue::ConstU8(1))],
        MirTerminator::Branch { cond: MirCond::BoolValue(MirValue::PointerCell(MirMem::Absolute(0x700))),
            then_edge: MirEdge::plain(MirBlockId(0)), else_edge: MirEdge::plain(MirBlockId(0)) })], vec![]);
    assert_eq!(summarize_routine_reads(&p.routines[0], &MirKnownCalleeSummaries::default()), MirKnownMemoryReads::Unknown);
}
