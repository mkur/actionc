mod support;
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

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
        (include_str!("fixtures/code_quality/sum_loop.act"), [14, 12]),
        (
            include_str!("fixtures/code_quality/loop_rotation.act"),
            [18, 20],
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
