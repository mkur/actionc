use super::*;
use crate::{
    mir6502::{MirBlock, MirBlockId},
    nir::{LocalId, ParamId},
};
fn slot(id: u32, size: u16) -> MirStorageSlot {
    MirStorageSlot {
        purpose: MirStoragePurpose::Storage,
        id: MirStorageId(id),
        name: None,
        storage: MirStorageClass::Scalar,
        storage_size: size,
        scalar_width: Some(if size == 1 {
            MirWidth::Byte
        } else {
            MirWidth::Word
        }),
        base: MirStorageBase::Local(LocalId(id)),
        offset: 0,
        mutable: true,
        init: None,
    }
}
fn routine() -> MirRoutine {
    MirRoutine {
        inline: Default::default(),
        scalar_signature: None,
        id: RoutineId(7),
        name: "Independent".into(),
        abi: MirRoutineAbi::Action,
        frame: MirFrame {
            locals: vec![slot(0, 2), slot(1, 2)],
            spills: (0..300).map(MirSpillId).collect(),
            ..Default::default()
        },
        temps: vec![],
        blocks: vec![MirBlock {
            id: MirBlockId(0),
            label: "entry".into(),
            params: vec![],
            ops: vec![],
            terminator: MirTerminator::Return,
        }],
        effects: Default::default(),
    }
}
fn local(id: u32, offset: u16) -> MirMem {
    MirMem::Local {
        id: LocalId(id),
        offset,
    }
}
fn spill(id: u32) -> MirMem {
    MirMem::Spill {
        id: MirSpillId(id),
        offset: 0,
    }
}
fn key(mem: MirMem) -> Option<SsaLiteValueKey> {
    Some(SsaLiteValueKey::DirectMem(mem))
}
#[test]
fn home_value_invalidation_preserves_multiple_copies_and_disjoint_unknown_stores() {
    let r = routine();
    let mut p = Precision::new(&r);
    let mut env = SsaLiteValueEnv::default();
    p.store(&mut env, &spill(0), key(local(0, 0)));
    p.store(&mut env, &spill(1), key(local(0, 1)));
    p.store(&mut env, &spill(2), None);
    assert_eq!(env.mem_fact(&spill(0)), key(local(0, 0)).as_ref());
    assert_eq!(env.mem_fact(&spill(1)), key(local(0, 1)).as_ref());
    assert_eq!(p.peak, 2);
    p.invalidate(&mut env, &local(0, 1), 1);
    assert!(env.mem_fact(&spill(0)).is_some());
    assert!(env.mem_fact(&spill(1)).is_none());
    p.invalidate(&mut env, &local(0, 0), 2);
    assert!(env.mem.is_empty());
}
#[test]
fn home_value_invalidation_kills_both_ends_register_dependencies_and_physical_alias_views() {
    let mut r = routine();
    let mut alias = slot(2, 1);
    alias.base = MirStorageBase::LocalAlias {
        id: LocalId(2),
        target: LocalId(0),
    };
    alias.offset = 1;
    r.frame.locals.push(alias);
    let mut p = Precision::new(&r);
    let mut env = SsaLiteValueEnv::default();
    p.store(&mut env, &spill(0), key(local(0, 1)));
    p.store(&mut env, &spill(1), key(local(1, 0)));
    env.a = key(local(0, 1));
    env.x = key(local(1, 0));
    env.y = Some(SsaLiteValueKey::ConstU8(9));
    p.invalidate(&mut env, &local(2, 0), 1);
    assert!(env.mem_fact(&spill(0)).is_none());
    assert!(env.a.is_none());
    assert!(env.x.is_some());
    assert!(env.y.is_some());
    p.invalidate(&mut env, &spill(1), 1);
    assert!(env.mem.is_empty());
    assert_eq!(
        p.homes.overlap(r.id, &local(2, 0), 1, &local(0, 1)),
        Overlap::Overlap
    );
}
#[test]
fn home_value_invalidation_rejects_unknown_owner_missing_and_overlaid_homes() {
    let mut r = routine();
    r.frame.spills.push(MirSpillId(0));
    let h = HomeStorage::new(&r);
    assert_eq!(
        h.overlap(RoutineId(8), &local(0, 0), 1, &local(1, 0)),
        Overlap::Unknown
    );
    for a in [
        spill(0),
        local(99, 0),
        local(0, 2),
        MirMem::Absolute(0x3000),
        MirMem::FixedZeroPage(MirFixedZpSlot(0xA0)),
        MirMem::ZeroPage(MirZpSlot(0)),
        MirMem::Param {
            id: ParamId(0),
            offset: 0,
        },
    ] {
        assert_eq!(
            h.overlap(r.id, &a, 1, &local(0, 0)),
            Overlap::Unknown,
            "{a:?}"
        );
    }
    let mut p = Precision::new(&r);
    let mut env = SsaLiteValueEnv::default();
    p.store(&mut env, &spill(1), key(local(0, 0)));
    p.invalidate(&mut env, &MirMem::Absolute(0x3000), 1);
    assert!(env.mem.is_empty());
    // A colored slot's surviving ID denotes the same actual byte.
    assert_eq!(h.overlap(r.id, &spill(1), 1, &spill(1)), Overlap::Overlap);
}
#[test]
fn home_value_invalidation_does_not_relearn_a_stale_source_or_self_copy() {
    let r = routine();
    let mut p = Precision::new(&r);
    let mut env = SsaLiteValueEnv::default();
    p.store(&mut env, &spill(0), key(local(0, 0)));
    p.store(&mut env, &local(0, 0), Some(SsaLiteValueKey::ConstU8(7)));
    assert!(env.mem_fact(&spill(0)).is_none());
    assert_eq!(
        env.mem_fact(&local(0, 0)),
        Some(&SsaLiteValueKey::ConstU8(7))
    );
    p.store(&mut env, &local(0, 0), key(local(0, 0)));
    assert!(env.mem_fact(&local(0, 0)).is_none());
    p.store(&mut env, &spill(1), None);
    assert!(env.mem_fact(&spill(1)).is_none());
}
#[test]
fn home_value_invalidation_rejects_alias_cycles_out_of_bounds_and_descriptors() {
    let mut r = routine();
    let mut a = slot(2, 2);
    a.base = MirStorageBase::LocalAlias {
        id: LocalId(2),
        target: LocalId(0),
    };
    a.offset = 1;
    r.frame.locals.push(a);
    let mut b = slot(3, 1);
    b.base = MirStorageBase::LocalAlias {
        id: LocalId(3),
        target: LocalId(4),
    };
    r.frame.locals.push(b);
    let mut c = slot(4, 1);
    c.base = MirStorageBase::LocalAlias {
        id: LocalId(4),
        target: LocalId(3),
    };
    r.frame.locals.push(c);
    let h = HomeStorage::new(&r);
    assert!(h.region(r.id, &local(2, 0), 1).is_some());
    assert!(h.region(r.id, &local(2, 0), 2).is_none());
    assert!(h.region(r.id, &local(3, 0), 1).is_none());
    assert!(h.region(r.id, &local(0, 0), 0).is_none());
    assert!(h.region(r.id, &local(0, 0), 65535).is_none());
}
#[test]
fn home_value_invalidation_budget_discards_facts_and_keeps_invalidating() {
    let r = routine();
    let mut p = Precision::new(&r);
    let mut env = SsaLiteValueEnv::default();
    for id in 0..256 {
        p.store(
            &mut env,
            &spill(id),
            Some(SsaLiteValueKey::ConstU8(id as u8)),
        );
    }
    assert_eq!(env.mem.len(), 256);
    p.store(&mut env, &spill(256), Some(SsaLiteValueKey::ConstU8(0)));
    assert!(env.mem.is_empty());
    assert!(p.exhausted);
    assert_eq!(p.peak, 256);
    p.store(&mut env, &spill(257), key(local(0, 0)));
    assert!(env.mem.is_empty());
    env.a = key(local(0, 0));
    p.invalidate(&mut env, &local(0, 0), 1);
    assert!(env.a.is_none());
}

#[test]
fn home_value_invalidation_rejects_unallocated_zp_overlays_and_initialized_views() {
    let mut r = routine();
    r.frame.zero_page_allocations = vec![
        MirZpAllocation {
            slot: MirZpSlot(1),
            start: MirFixedZpSlot(0x90),
            size: 1,
        },
        MirZpAllocation {
            slot: MirZpSlot(2),
            start: MirFixedZpSlot(0x90),
            size: 1,
        },
    ];
    let mut initialized = slot(5, 2);
    initialized.init = Some(MirStorageInit::ZeroFill {
        bytes: 2,
        mutable: true,
        section: "data".into(),
    });
    r.frame.locals.push(initialized);
    let h = HomeStorage::new(&r);
    assert_eq!(
        h.overlap(
            r.id,
            &MirMem::ZeroPage(MirZpSlot(1)),
            1,
            &MirMem::ZeroPage(MirZpSlot(2))
        ),
        Overlap::Unknown
    );
    assert!(!h.ordinary_byte(&local(5, 0)));
    let mut env = SsaLiteValueEnv::default();
    let mut p = Precision::new(&r);
    p.store(&mut env, &spill(0), key(local(0, 0)));
    p.invalidate(&mut env, &MirMem::ZeroPage(MirZpSlot(1)), 1);
    assert!(env.mem.is_empty());
}
