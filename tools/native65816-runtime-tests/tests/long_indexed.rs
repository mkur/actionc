mod support;
use actionc::mir65816::{Mir65816Op, emit::proof, o65 as format};
use actionc_vm::native65816::Access;
use support::*;

const SOURCE: &str = "BYTE ARRAY bytes(65536)
 CARD index=$7100 BYTE result=$7102
 BYTE FUNC Work(CARD i) RETURN(bytes(i))
 PROC Main() result=Work(index) RETURN";

#[test]
fn symbolic_card_indexes_load_exact_bytes_across_banks_and_o65_rebasing() {
    let code = assemble("sep #$20\nlda f:$12fff9,x\nstp\nnop", 0x40000);
    assert_eq!(&code[2..6], &[0xbf, 0xf9, 0xff, 0x12]);
    for optimize in [false, true] {
        let p = prepare(SOURCE, optimize);
        let mut options = layout();
        options.data_origin = 0x12fff9;
        let c = p.compile(&options).unwrap();
        assert_eq!(
            c.image.to_json().unwrap(),
            prepare(&SOURCE.replace('\n', "\r\n"), optimize)
                .compile(&options)
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        let work = c
            .machine
            .prepared
            .routines
            .iter()
            .find(|r| r.name == "Work")
            .unwrap();
        let m = c.machine.routines.iter().find(|m| m.id == work.id).unwrap();
        let (replayed, _) = proof::replay_code(&m.code, true).unwrap();
        assert_eq!(replayed.bytes, m.code.bytes);
        let effects = proof::instruction_effects(&replayed);
        assert_eq!(
            effects
                .iter()
                .filter(|r| r.effects.memory.iter().any(|m| matches!(
                    m.memory,
                    proof::EffectMemory::SymbolIndexedX { bytes: 1, .. }
                )))
                .count(),
            1
        );
        let object = p.compile_o65(&Default::default()).unwrap().bytes;
        for variant in 0..3 {
            let relocated = (variant > 0).then(|| {
                format::relocate(
                    &object,
                    &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                )
                .unwrap()
            });
            let base = relocated.as_ref().map_or_else(
                || context::symbol(&c.image, "bytes"),
                |l| o65::object(l, "bytes"),
            );
            for index in [0u16, 1, 255, 256, 65535] {
                for irq in [0, 4] {
                    let mut h = if let Some(l) = &relocated {
                        Harness::new_o65(l, &caller(l.entry()), irq)
                    } else {
                        Harness::new(&c.image, &caller(c.image.entry), irq)
                    };
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&index.to_le_bytes());
                    let at = base + u32::from(index);
                    h.bus.ram[at as usize - 1..at as usize + 2]
                        .copy_from_slice(&[0xa5, 0x89, 0xa5]);
                    h.bus.watched.extend([at - 1, at, at + 1]);
                    h.run();
                    h.guards(irq);
                    assert_eq!(h.bus.ram[0x7102], 0x89);
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, a, k)| (a, k))
                            .collect::<Vec<_>>(),
                        [(at, Access::Read)]
                    );
                    assert_eq!(
                        &h.bus.ram[at as usize - 1..at as usize + 2],
                        &[0xa5, 0x89, 0xa5]
                    );
                }
            }
        }
    }
}

#[test]
fn indexed_loads_reject_volatile_and_displaced_forms_and_survive_reentry() {
    for variant in 0..3 {
        let mut p = prepare(SOURCE, true);
        for op in p
            .mir
            .routines
            .iter_mut()
            .flat_map(|r| &mut r.blocks)
            .flat_map(|b| &mut b.ops)
        {
            if let Mir65816Op::Load {
                address, volatile, ..
            } = op
                && address.index.is_some()
            {
                match variant {
                    0 => *volatile = true,
                    1 => address.displacement = actionc::target::ByteOffset::new(1),
                    _ => address.index.as_mut().unwrap().stride = actionc::target::ByteSize::new(2),
                }
            }
        }
        let c = p.compile(&layout()).unwrap();
        assert!(
            c.machine
                .routines
                .iter()
                .flat_map(|m| proof::instruction_effects(
                    &proof::replay_code(&m.code, true).unwrap().0
                )
                .to_vec())
                .all(|e| e
                    .effects
                    .memory
                    .iter()
                    .all(|m| !matches!(m.memory, proof::EffectMemory::SymbolIndexedX { .. })))
        );
    }
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 BYTE ARRAY values(512) BYTE scratch
        BYTE FUNC Work(CARD i) RETURN(values(i))
        CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(257) irqAck=1 RETURN(saved)
        PROC Task(CARD POINTER argument) argument^=CARD(Work(argument^)) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts(source);
}

#[test]
fn symbolic_byte_stores_preserve_payload_order_and_neighbors_after_rebasing() {
    assert_eq!(
        &assemble("sep #$20\nsta f:$12fff9,x\nstp\nnop", 0x40000)[2..6],
        &[0x9f, 0xf9, 0xff, 0x12]
    );
    for constant in [false, true] {
        let source = SOURCE.replace(
            "BYTE FUNC Work(CARD i) RETURN(bytes(i))",
            &format!(
                "BYTE FUNC Work(CARD i) BYTE old old=bytes(i) bytes(i)={} RETURN(old)",
                if constant { "73" } else { "old XOR $FF" }
            ),
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let mut options = layout();
            options.data_origin = 0x12fff9;
            let c = p.compile(&options).unwrap();
            let traced = proof::materialize_with_trace(&p.mir).unwrap().0;
            assert!(
                traced
                    .routines
                    .iter()
                    .flat_map(|m| proof::instruction_effects(&m.code))
                    .any(|e| e.effects.memory.iter().any(|m| m.access
                        == proof::EffectAccess::MayWrite
                        && matches!(
                            m.memory,
                            proof::EffectMemory::SymbolIndexedX { bytes: 1, .. }
                        )))
            );
            let object = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let relocated = (variant > 0).then(|| {
                    format::relocate(
                        &object,
                        &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                let base = relocated.as_ref().map_or_else(
                    || context::symbol(&c.image, "bytes"),
                    |l| o65::object(l, "bytes"),
                );
                for index in [0u16, 255, 256, 65535] {
                    let mut h = if let Some(l) = &relocated {
                        Harness::new_o65(l, &caller(l.entry()), 0)
                    } else {
                        Harness::new(&c.image, &caller(c.image.entry), 0)
                    };
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&index.to_le_bytes());
                    let at = base + u32::from(index);
                    h.bus.ram[at as usize - 1..at as usize + 2]
                        .copy_from_slice(&[0xa5, 0x89, 0xa5]);
                    h.bus.watched.extend([at - 1, at, at + 1]);
                    h.run();
                    h.guards(0);
                    let value = if constant { 73 } else { 0x76 };
                    assert_eq!(h.bus.ram[0x7102], 0x89);
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, a, k)| (a, k))
                            .collect::<Vec<_>>(),
                        [(at, Access::Read), (at, Access::Write(value))]
                    );
                    assert_eq!(
                        &h.bus.ram[at as usize - 1..at as usize + 2],
                        &[0xa5, value, 0xa5]
                    );
                }
            }
        }
    }
    let source="MODULE TEST VOLATILE BYTE irqAck=$7800 BYTE ARRAY values(512) BYTE scratch
        BYTE FUNC Work(CARD i) BYTE old old=values(i) values(i)=old XOR $FF RETURN(old)
        CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(257) irqAck=1 RETURN(saved)
        PROC Task(CARD POINTER argument) argument^=CARD(Work(argument^)) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts(source);
}
