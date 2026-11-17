use super::*;
use crate::nir::SymbolId;

fn global(backing: MirGlobalBacking) -> MirGlobal {
    MirGlobal { id: SymbolId(0), name: "state".into(), kind: "word".into(),
        width: Some(MirWidth::Word), storage_size: 2, backing, init: None }
}

#[test]
fn fixed_overlap_byte_and_word_writes_keep_logical_identity_and_checked_extents() {
    let state = MirMem::Global { id: SymbolId(0), offset: 0 };
    for address in [0x600, 0xe4, 0xff, 0xffff] {
        for width in [MirWidth::Byte, MirWidth::Word] {
            let mut p = program(vec![routine(0, vec![MirOp::Store {
                dst: MirAddr::Direct(state.clone()), src: MirValue::Def(MirDef::Reg(MirReg::A)), width,
            }], MirTerminator::Return)], vec![]);
            p.globals.push(global(MirGlobalBacking::Absolute(address)));
            let facts = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
            let writes = facts.get(RoutineId(0)).unwrap().writes();
            let expected = if width == MirWidth::Word { vec![state.clone(), offset_mem(&state, 1)] }
                else { vec![state.clone()] };
            assert_eq!(writes, &MirKnownMemoryWrites::Exact(expected));
            for slot in [0xe3, 0xe4, 0xe5, 0xe6, 0xff] {
                let overlap = u16::from(slot) == address
                    || (width == MirWidth::Word && address.checked_add(1).is_none_or(|a| a == u16::from(slot)));
                assert_eq!(writes.preserves_fixed_byte(MirFixedZpSlot(slot), facts.fixed_storage()), !overlap,
                    "{address:04x} {width:?} slot={slot:02x}");
            }
            assert!(!writes.preserves_fixed_pair(MirFixedZpSlot(0xff), facts.fixed_storage()));
            assert!(writes.may_write_caller_private_byte(address, facts.fixed_storage()));
        }
    }
}

#[test]
fn fixed_overlap_private_byte_observation_matches_named_aliases_and_unknowns() {
    let mut p = program(vec![], vec![]);
    p.globals = vec![global(MirGlobalBacking::Absolute(0x7000)), MirGlobal {
        id: SymbolId(1), backing: MirGlobalBacking::Alias { target: SymbolId(0), offset: 1 },
        ..global(MirGlobalBacking::Absolute(0))
    }];
    let fixed = MirFixedStorage::new(&p);
    let alias = MirMem::Global { id: SymbolId(1), offset: 1 };
    for byte in [alias, MirMem::Absolute(0x7002)] {
        let writes = MirKnownMemoryWrites::Exact(vec![byte.clone()]);
        let reads = MirKnownMemoryReads::Exact(vec![byte]);
        for address in [0xe4, 0x7001, 0x7002, 0x7003] {
            assert_eq!(writes.may_write_caller_private_byte(address, &fixed), address == 0x7002);
            assert_eq!(reads.may_read_caller_private_byte(address, &fixed), address == 0x7002);
        }
    }
    let fixed_zp = MirKnownMemoryReads::Exact(vec![MirMem::FixedZeroPage(MirFixedZpSlot(0xe4))]);
    assert!(fixed_zp.may_read_caller_private_byte(0xe4, &fixed));
    for byte in [MirMem::Global { id: SymbolId(99), offset: 0 }, MirMem::ZeroPage(MirZpSlot(99))] {
        assert!(MirKnownMemoryReads::Exact(vec![byte.clone()]).may_read_caller_private_byte(0xe4, &fixed));
        assert!(MirKnownMemoryWrites::Exact(vec![byte]).may_write_caller_private_byte(0xe4, &fixed));
    }
    assert!(MirKnownMemoryReads::Unknown.may_read_caller_private_byte(0xe4, &fixed));
    assert!(!MirKnownMemoryWrites::Unknown.preserves_fixed_byte(MirFixedZpSlot(0xe4), &fixed));
    assert!(MirKnownMemoryWrites::Unknown.may_write_mem(&MirMem::Absolute(0x600), &fixed));
    assert!(MirKnownMemoryWrites::exact().preserves_fixed_pair(MirFixedZpSlot(0xfe), &fixed));
    assert!(!MirKnownMemoryWrites::exact().preserves_fixed_pair(MirFixedZpSlot(0xff), &fixed));
}
