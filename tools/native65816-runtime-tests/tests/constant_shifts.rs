mod support;
use actionc::mir65816::{Mir65816Op, image::Image};
use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
use support::*;

#[test]
fn constants_cover_all_residuals_widths_zero_fill_and_large_counts() {
    for (ty, bytes) in [
        ("BYTE", 1),
        ("CARD", 2),
        ("INT", 2),
        ("SIZE", 3),
        ("LONGCARD", 4),
        ("LONGINT", 4),
    ] {
        let bits = bytes * 8;
        let mask = ((1u64 << bits) - 1) as u32;
        let counts: Vec<u32> = (0..=bits as u32 + 1).chain([255, 256, 0xffffff]).collect();
        let mut source = format!("{ty} input=$7100\n");
        for i in 0..counts.len() {
            source.push_str(&format!(
                "{ty} left{i}={},right{i}={}\n",
                0x8000 + i * 16,
                0x8008 + i * 16
            ));
        }
        for (i, count) in counts.iter().enumerate() {
            // Mutable incoming home, a nested call, and runtime input prevent
            // folding the value. All constants remain numeric shift operands.
            source.push_str(&format!("PROC Shift{i}({ty} value) value==+1 left{i}=value LSH {count} right{i}=value RSH {count} RETURN\n"));
        }
        source.push_str("PROC Main()\n");
        for i in 0..counts.len() {
            source.push_str(&format!("Shift{i}(input)\n"));
        }
        source.push_str("RETURN\n");
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            for value in [0, 0xfe, 0xffff, 0x89abcdee, mask - 1, mask] {
                for irq in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), irq);
                    h.bus.ram[0x7100..0x7100 + bytes]
                        .copy_from_slice(&(value & mask).to_le_bytes()[..bytes]);
                    h.bus.map(0x8000, &vec![0xa5; counts.len() * 16], true);
                    h.run();
                    h.guards(irq);
                    let shifted = value.wrapping_add(1) & mask;
                    let mut expected = vec![0xa5; counts.len() * 16];
                    for (i, &count) in counts.iter().enumerate() {
                        let left = if count >= bits as u32 {
                            0
                        } else {
                            shifted.wrapping_shl(count) & mask
                        };
                        let right = if count >= bits as u32 {
                            0
                        } else {
                            shifted >> count
                        };
                        expected[i * 16..i * 16 + bytes]
                            .copy_from_slice(&left.to_le_bytes()[..bytes]);
                        expected[i * 16 + 8..i * 16 + 8 + bytes]
                            .copy_from_slice(&right.to_le_bytes()[..bytes]);
                    }
                    assert_eq!(
                        &h.bus.ram[0x8000..0x8000 + expected.len()],
                        expected,
                        "{ty}/{optimize}/{value:x}/{irq}"
                    );
                }
            }
        }
    }
}

#[test]
fn native_residual_word_chains_match_ca65_and_preserve_scratch_neighbors() {
    for left in [false, true] {
        let body = if left {
            "asl $08\nrol $0a"
        } else {
            "lsr $0a\nror $08"
        };
        let code = assemble(&format!("rep #$20\n{body}\nstp\nnop"), 0x040000);
        assert_eq!(
            &code[2..6],
            if left {
                &[0x06, 8, 0x26, 10]
            } else {
                &[0x46, 10, 0x66, 8]
            }
        );
        for value in [0u32, 1, 0xffff, 0x80000000, u32::MAX] {
            for p in [0, 1, 0x24, 0x25] {
                let mut bus = Bus::new();
                bus.map(0x040000, &code, false);
                bus.map(0x2007, &[0xa5; 6], true);
                bus.ram[0x2008..0x200c].copy_from_slice(&value.to_le_bytes());
                let mut cpu = Machine::start_at(Registers {
                    a: 0xabcd,
                    x: 0x1234,
                    y: 0x5678,
                    s: 0x5fe0,
                    d: 0x2000,
                    dbr: 0,
                    pbr: 4,
                    pc: 0,
                    p,
                    emulation_mode: false,
                });
                assert!(
                    cpu.run_until(&mut bus, 100, |_| Inputs::default(), |cpu| cpu.is_stopped())
                        .unwrap()
                );
                let expected = if left {
                    value.wrapping_shl(1)
                } else {
                    value >> 1
                };
                assert_eq!(&bus.ram[0x2008..0x200c], expected.to_le_bytes());
                assert_eq!((bus.ram[0x2007], bus.ram[0x200c]), (0xa5, 0xa5));
                assert!(
                    bus.writes
                        .iter()
                        .all(|(at, _)| (0x2008..0x200c).contains(at))
                );
                let r = cpu.registers();
                assert_eq!(
                    (r.a, r.x, r.y, r.s, r.d, r.p & 4),
                    (0xabcd, 0x1234, 0x5678, 0x5fe0, 0x2000, p & 4)
                );
            }
        }
    }
}

#[test]
fn index_scaling_preserves_full_24_bit_carries_and_exact_volatile_accesses() {
    let source = "BYTE ARRAY data(8)=$FFFF SIZE index BYTE result PROC Main() result=data(index) data(index)=37 RETURN";
    for optimize in [false, true] {
        for stride in [1u32, 2, 3, 4, 5, 257, 65535, 65536, 0x800000, 0xffffff] {
            let mut p = prepare(source, optimize);
            let mut changed = 0;
            for op in p
                .mir
                .routines
                .iter_mut()
                .flat_map(|r| &mut r.blocks)
                .flat_map(|b| &mut b.ops)
            {
                if let Mir65816Op::Load {
                    address, volatile, ..
                }
                | Mir65816Op::Store {
                    address, volatile, ..
                } = op
                {
                    if let Some(index) = &mut address.index {
                        index.stride = actionc::target::ByteSize::new(stride);
                        address.base = actionc::mir65816::Mir65816AddressBase::External(
                            actionc::mir65816::Mir65816ExternalAddress::Absolute(
                                actionc::target::AddressValue::data(0x52ffff),
                            ),
                        );
                        *volatile = true;
                        changed += 1;
                    }
                }
            }
            assert_eq!(changed, 2);
            actionc::mir65816::verify_program(&p.mir).unwrap();
            let image =
                Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap()).unwrap();
            for index in [0u32, 1, 0x101, 0xffff, 0x10000, 0xffffff] {
                let target = 0x52ffffu32.wrapping_add(index.wrapping_mul(stride)) & 0xffffff;
                // Avoid overlapping code, bank-zero domain/stack, or globals.
                assert!(target > 0x060000);
                for irq in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), irq);
                    let at = support::context::symbol(&image, "index") as usize;
                    h.bus.ram[at..at + 3].copy_from_slice(&index.to_le_bytes()[..3]);
                    h.bus.map(target - 1, &[0xa5, 0x7b, 0xa5], true);
                    h.bus.watched.insert(target);
                    h.run();
                    h.guards(irq);
                    assert_eq!(h.global(&image, "result", 1), 0x7b);
                    assert_eq!(
                        &h.bus.ram[target as usize - 1..target as usize + 2],
                        &[0xa5, 37, 0xa5]
                    );
                    let accesses: Vec<_> = h.bus.trace.iter().map(|(_, a, op)| (*a, *op)).collect();
                    assert_eq!(
                        accesses,
                        [(target, Access::Read), (target, Access::Write(37))]
                    );
                }
            }
        }
    }
}

#[test]
fn accumulator_word_shifts_have_independent_encodings_and_survive_reentry() {
    for (name, opcode) in [("asl", 0x0a), ("lsr", 0x4a)] {
        let code = assemble(
            &format!("rep #$20\nlda $40,s\n{name} a\n{name} a\n{name} a\nsta $41,s\nstp\nnop"),
            0x40000,
        );
        assert_eq!(&code[2..9], &[0xa3, 64, opcode, opcode, opcode, 0x83, 65]);
    }
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 CARD scratch
        CARD FUNC Work(CARD value) CARD a,b a=value RSH 3 b=value LSH 7 RETURN(a XOR b)
        CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work($8001) irqAck=1 RETURN(saved)
        PROC Task(CARD POINTER argument) argument^=Work(argument^) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts(source);
}
