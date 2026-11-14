use super::*;

const OWNER: RoutineId = RoutineId(0);
const SPILL: MirSpillId = MirSpillId(3);
fn home() -> MirMem {
    MirMem::Spill {
        id: SPILL,
        offset: 0,
    }
}
fn capture(state: &mut State, memory: &MirPhysicalMemory) {
    state.observe(
        &MirOp::Store {
            dst: MirAddr::Direct(home()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        0,
        OWNER,
        memory,
        RecordBaseOptions::default(),
    );
}
fn value(state: &State, memory: &MirPhysicalMemory) -> Option<ByteValue> {
    state.value(
        &MirValue::PointerCell(home()),
        OWNER,
        memory,
        RecordBaseOptions::default(),
    )
}

#[test]
fn retired_capture_requires_certificate_and_never_has_a_physical_address() {
    let mut memory = MirPhysicalMemory::default();
    let mut state = State::default();
    state.registers[reg(MirReg::A)] = Some(ByteValue::Memory(0x500));
    capture(&mut state, &memory);
    assert_eq!(value(&state, &memory), None);
    assert!(memory.retire_private_spill(OWNER, SPILL));
    state.registers[reg(MirReg::A)] = Some(ByteValue::Memory(0x500));
    capture(&mut state, &memory);
    assert_eq!(value(&state, &memory), Some(ByteValue::Memory(0x500)));
    assert_eq!(memory.address(OWNER, &home()), None);
    assert!(!memory.ordinary(OWNER, &home()));
    assert!(memory.retired_home(RoutineId(1), &home()).is_none());
    assert!(
        memory
            .retired_home(
                OWNER,
                &MirMem::Spill {
                    id: SPILL,
                    offset: 1
                }
            )
            .is_none()
    );
    assert_eq!(
        state.value(
            &MirValue::StorageAddrByte {
                mem: home(),
                byte: 0
            },
            OWNER,
            &memory,
            RecordBaseOptions::default()
        ),
        None
    );
    let mut allocated = MirPhysicalMemory::default();
    allocated.insert(OWNER, home(), 0x500, true);
    assert!(!allocated.retire_private_spill(OWNER, SPILL));
}

#[test]
fn retired_capture_loses_stale_unknown_and_barrier_values() {
    let mut memory = MirPhysicalMemory::default();
    assert!(memory.retire_private_spill(OWNER, SPILL));
    let mut state = State::default();
    state.registers[reg(MirReg::A)] = Some(ByteValue::Memory(0x500));
    capture(&mut state, &memory);
    state.write(0x600, 1);
    assert_eq!(value(&state, &memory), Some(ByteValue::Memory(0x500)));
    state.write(0x500, 1);
    assert_eq!(value(&state, &memory), None);
    state.registers[reg(MirReg::A)] = Some(ByteValue::Constant(42));
    capture(&mut state, &memory);
    state.registers[reg(MirReg::A)] = None;
    capture(&mut state, &memory);
    assert_eq!(value(&state, &memory), None);
    state.registers[reg(MirReg::A)] = Some(ByteValue::Constant(42));
    capture(&mut state, &memory);
    state.observe(
        &MirOp::Barrier {
            effects: MirEffects::default(),
        },
        1,
        OWNER,
        &memory,
        RecordBaseOptions::default(),
    );
    assert_eq!(value(&state, &memory), None);
}

#[test]
fn retired_capture_cfg_join_requires_matching_origins_on_every_path() {
    let routine = MirRoutine {
        id: OWNER,
        name: "Join".into(),
        inline: Default::default(),
        scalar_signature: None,
        abi: MirRoutineAbi::Action,
        frame: MirFrame::default(),
        temps: vec![],
        effects: MirEffects::default(),
        blocks: vec![],
    };
    let memory = MirPhysicalMemory::default();
    let calls = CallWrites::default();
    let problem = ResidentProblem {
        r: &routine,
        memory: &memory,
        calls: &calls,
        entry: None,
    };
    let mut a = State::default();
    a.private_cells
        .insert((OWNER, SPILL), ByteValue::Memory(0x500));
    let mut same = Some(a.clone());
    problem.join(&mut same, &Some(a.clone()));
    assert_eq!(same.unwrap().private_cells, a.private_cells);
    for other in [
        None,
        Some(ByteValue::Memory(0x501)),
        Some(ByteValue::Constant(42)),
    ] {
        let mut b = State::default();
        if let Some(other) = other {
            b.private_cells.insert((OWNER, SPILL), other);
        }
        let mut joined = Some(a.clone());
        problem.join(&mut joined, &Some(b));
        assert!(joined.unwrap().private_cells.is_empty());
    }
}
