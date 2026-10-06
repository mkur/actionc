mod support;
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

#[test]
fn empty_frames_return_without_stack_writes_or_domain_limit_reads() {
    for (source, name, expected) in [
        ("PROC Main() RETURN", "Main", 0xd5aa),
        (
            "CARD FUNC Echo(CARD value) RETURN(value) PROC Main() RETURN",
            "Echo",
            0xbeef,
        ),
    ] {
        for optimize in [false, true] {
            let prepared = prepare(source, optimize);
            let checked = prepared.compile(&layout()).unwrap();
            let mut options = layout();
            options.stack_checks = false;
            let unchecked = prepared.compile(&options).unwrap();
            assert_eq!(
                serde_json::to_value(&checked.image.segments).unwrap(),
                serde_json::to_value(&unchecked.image.segments).unwrap()
            );
            let routine = checked
                .image
                .routines
                .iter()
                .find(|r| r.name == name)
                .unwrap();
            assert_eq!(routine.fixed_frame, 0);
            for mask in [0, 4] {
                for initial_s in [0x4019usize, 0x5fe8] {
                    let image = &checked.image;
                    let mut h = Harness::new(image, &[0xdb, 0xea], mask);
                    let mut r = h.cpu.registers();
                    r.s = initial_s as u16;
                    r.pc = routine.address as u16;
                    r.pbr = (routine.address >> 16) as u8;
                    r.a = 0xd5aa;
                    r.x = 0xbeef;
                    r.y = 0x1234;
                    h.cpu = Machine::start_at(r);
                    // RTL returns to the harness's stop instruction at $040000.
                    h.bus.ram[initial_s + 1..initial_s + 4].copy_from_slice(&[0xff, 0xff, 4]);
                    h.bus.ram[initial_s + 4..initial_s + 6]
                        .copy_from_slice(&0xbeefu16.to_le_bytes());
                    h.run();
                    let out = h.cpu.registers();
                    assert_eq!(
                        (out.a, out.x, out.y, out.s, out.d, out.dbr, out.p & 0x3c),
                        (
                            expected,
                            0xbeef,
                            0x1234,
                            initial_s as u16 + 3,
                            0x2000,
                            0,
                            mask
                        )
                    );
                    assert!(
                        h.bus
                            .writes
                            .iter()
                            .all(|(a, _)| (0x2080..0x20c0).contains(a))
                    );
                    assert!(!h.bus.reads.iter().any(|a| (0x20c4..0x20c8).contains(a)));
                    assert_eq!(&h.bus.ram[0x2000..0x2080], h.caller_workspace);
                    assert_eq!(&h.bus.ram[0x20c0..0x2100], h.domain_tail);
                }
            }
        }
    }
}

#[test]
fn entry_reservations_fault_before_stack_writes_on_floor_wrap_and_ceiling_errors() {
    let image = compile("PROC Main() CARD value value=42 RETURN", false);
    let frame = image
        .routines
        .iter()
        .find(|r| r.address == image.entry)
        .unwrap()
        .fixed_frame;
    assert!(frame >= 4);
    for mask in [0, 4] {
        for initial_s in [0x401c, 2, 0x6000] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            let mut registers = h.cpu.registers();
            registers.s = initial_s;
            registers.pc = image.entry as u16;
            registers.pbr = (image.entry >> 16) as u8;
            h.cpu = Machine::start_at(registers);
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        1000,
                        |_| Inputs::default(),
                        |cpu| cpu.pc() == image.stack_overflow && cpu.is_instruction_boundary()
                    )
                    .unwrap()
            );
            let r = h.cpu.registers();
            assert_eq!((r.a, r.x, r.s), (frame, initial_s, initial_s));
            assert_eq!((r.d, r.dbr, r.p & 0x3c), (0x2000, 0, mask));
            assert!(
                h.bus.writes.is_empty(),
                "fault must precede every write: {:?}",
                h.bus.writes
            );
        }
    }
}

#[test]
fn call_check_includes_far_return_bytes_before_filling_outgoing_space() {
    let image = compile(
        "BYTE phase PROC Empty() RETURN PROC Main() phase=1 Empty() RETURN",
        false,
    );
    let main = image
        .routines
        .iter()
        .find(|r| r.address == image.entry)
        .unwrap();
    assert_eq!(main.fixed_frame, 0);
    assert_eq!(main.local_stack_peak, 4); // O=1 plus JSL=3
    let mut h = Harness::new(&image, &caller(image.entry), 0);
    let mut registers = h.cpu.registers();
    registers.s = 0x401c;
    registers.pc = image.entry as u16;
    registers.pbr = (image.entry >> 16) as u8;
    h.cpu = Machine::start_at(registers);
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                1000,
                |_| Inputs::default(),
                |cpu| cpu.pc() == image.stack_overflow && cpu.is_instruction_boundary()
            )
            .unwrap()
    );
    let r = h.cpu.registers();
    assert_eq!((r.a, r.x, r.s), (4, 0x401c, 0x401c));
    assert_eq!(h.global(&image, "phase", 1), 1);
    assert!(
        h.bus
            .writes
            .iter()
            .all(|&(address, _)| !(0x4000..0x6000).contains(&address))
    );
}

#[test]
fn compact_edge_frames_check_exact_floor_before_any_write_in_both_modes() {
    for (source, extents) in [
        (include_str!("fixtures/code_quality/sum_loop.act"), [10, 6]),
        (
            include_str!("fixtures/code_quality/loop_rotation.act"),
            [14, 8],
        ),
        (include_str!("fixtures/code_quality/byte_sum.act"), [16, 18]),
    ] {
        for optimize in [false, true] {
            // Same compiler path for host text in either newline convention.
            let image = compile(&source.replace("\r\n", "\n"), optimize);
            let work = image
                .routines
                .iter()
                .find(|r| r.name.contains("WORK"))
                .unwrap();
            let extent = extents[usize::from(optimize)];
            assert_eq!((work.fixed_frame, work.local_stack_peak), (extent, extent));
            for mask in [0, 4] {
                for below_floor in [false, true] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    let mut r = h.cpu.registers();
                    let initial_s = 0x4019 + extent - u16::from(below_floor);
                    r.s = initial_s;
                    r.pc = work.address as u16;
                    r.pbr = (work.address >> 16) as u8;
                    h.cpu = Machine::start_at(r);
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                1000,
                                |_| Inputs::default(),
                                |c| c.is_instruction_boundary()
                                    && (c.pc() == image.stack_overflow
                                        || c.registers().s != initial_s)
                            )
                            .unwrap()
                    );
                    let r = h.cpu.registers();
                    if below_floor {
                        assert_eq!(h.cpu.pc(), image.stack_overflow);
                        assert_eq!((r.a, r.x, r.s), (extent, initial_s, initial_s));
                    } else {
                        assert_ne!(h.cpu.pc(), image.stack_overflow);
                        assert_eq!(r.s, 0x4019);
                    }
                    assert_eq!((r.d, r.dbr, r.p & 0x3c), (0x2000, 0, mask));
                    assert!(h.bus.writes.is_empty());
                }
            }
        }
    }
}
