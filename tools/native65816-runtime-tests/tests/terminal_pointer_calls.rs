mod support;
use actionc::mir65816::{Mir65816AbiHome, Mir65816AddressBase, Mir65816Op, Mir65816Value, o65};
use actionc_vm::native65816::Inputs;
use support::*;

fn prepared(
    source: &str,
    optimize: bool,
    fallback: bool,
) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    if fallback {
        // A legal narrow numeric argument uses the existing zero-extension
        // fallback, while the exact-width pointer arguments remain borrowed.
        let r = p
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Forward")
            .unwrap();
        for op in r.blocks.iter_mut().flat_map(|b| &mut b.ops) {
            if let Mir65816Op::Call { args, .. } = op {
                args[2] = Mir65816Value::U8(0x55);
            }
        }
        actionc::mir65816::verify_program(&p.mir).unwrap();
    }
    p
}

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                200_000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == pc
            )
            .unwrap()
    );
}

#[test]
fn final_pointer_arguments_keep_complete_areas_results_and_reserved_homes_unread() {
    let source = "BYTE POINTER base=$7100\nADDRESS result=$7200\nADDRESS FUNC Echo(BYTE a BYTE POINTER p LONGCARD n BYTE POINTER q CARD b) RETURN(ADDRESS(p))\nADDRESS FUNC Forward(BYTE POINTER p) RETURN(Echo(3,p,LONGCARD($89abcdef),p,$4321))\nPROC Main() result=Forward(base) RETURN\n";
    for optimize in [false, true] {
        for fallback in [false, true] {
            let p = prepared(source, optimize, fallback);
            let c = p.compile(&layout()).unwrap();
            assert_eq!(
                c.image.to_json().unwrap(),
                prepared(&source.replace('\n', "\r\n"), optimize, fallback)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let r = c
                .machine
                .prepared
                .routines
                .iter()
                .find(|r| r.name == "Forward")
                .unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let mut omitted = vec![];
            let mut call = None;
            for b in &r.blocks {
                for (i, op) in b.ops.iter().enumerate() {
                    if let Mir65816Op::Load {
                        dest,
                        width,
                        address,
                        volatile: false,
                    } = op
                    {
                        if width.get() == 3
                            && matches!(address.base, Mir65816AddressBase::Parameter(_))
                            && m.code.mir_spans[&(b.id, i)].is_empty()
                        {
                            omitted.push(m.frame.temps[dest].stack().unwrap().offset);
                        }
                    }
                    if let Mir65816Op::Call { plan, .. } = op {
                        call = Some((m.code.mir_spans[&(b.id, i)].start, plan));
                    }
                }
            }
            assert!(!omitted.is_empty());
            assert_eq!(omitted.len(),r.blocks.iter().flat_map(|b|&b.ops).filter(|op|matches!(op,Mir65816Op::Load{width,address,..} if width.get()==3 && matches!(address.base,Mir65816AddressBase::Parameter(_)))).count());
            let (offset, plan) = call.unwrap();
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    o65::relocate(
                        &bytes,
                        &support::o65::placement(
                            &bytes,
                            variant - 1,
                            vec![support::o65::fault(variant - 1)],
                        ),
                    )
                    .unwrap()
                });
                let address = |name: &str| {
                    if let Some(l) = &loaded {
                        l.routine_address(
                            l.profile()
                                .routines
                                .iter()
                                .find(|r| r.name == name)
                                .unwrap(),
                        )
                    } else {
                        c.image
                            .routines
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    }
                };
                for value in [0u32, 0x12fffe, 0xffffff] {
                    for mask in [0, 4] {
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), mask)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), mask)
                        };
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                        reach(&mut h, address("Forward") + offset as u32);
                        let body = h.cpu.registers().s as usize;
                        for &slot in &omitted {
                            h.bus.ram[body + slot as usize..body + slot as usize + 3].fill(0xa7);
                        }
                        let before = h.bus.reads.len();
                        reach(&mut h, address("Echo"));
                        for &slot in &omitted {
                            assert!(!h.bus.reads[before..].iter().any(|&a| {
                                (body + slot as usize..body + slot as usize + 3)
                                    .contains(&(a as usize))
                            }));
                        }
                        let mut expected = vec![0; plan.outgoing_bytes.get() as usize];
                        for (home, bits) in plan.arguments.iter().zip([
                            3,
                            value,
                            if fallback { 0x55 } else { 0x89abcdef },
                            value,
                            0x4321,
                        ]) {
                            let Mir65816AbiHome::StackArgument { offset, size, .. } = home else {
                                unreachable!()
                            };
                            expected[offset.get() as usize..(offset.get() + size.get()) as usize]
                                .copy_from_slice(&u32::to_le_bytes(bits)[..size.get() as usize]);
                        }
                        let start = h.cpu.registers().s as usize + 4;
                        assert_eq!(&h.bus.ram[start..start + expected.len()], expected);
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.value(0x7200, 3), value);
                    }
                }
            }
        }
    }
}
