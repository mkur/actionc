//! Literal opcodes and architectural expectations, independent of actionc.
//! Reference: WDC W65C816S datasheet, instruction tables and sections 2/3/7.
use actionc_vm65816_tests::{Access, Machine, Registers};

const ENTRY: u32 = 0x018000;
const IRQ: u32 = 0x009000;
const NMI: u32 = 0x009200;

fn code(bytes: &[u8]) -> Machine {
    let mut m = Machine::native(ENTRY);
    m.bus.map(ENTRY, bytes, false);
    m
}

fn edit(m: &mut Machine, f: impl FnOnce(&mut Registers)) {
    let mut r = m.cpu.registers().clone();
    f(&mut r);
    m.cpu.set_registers(r);
}

fn registers(r: &Registers) -> (u16, u16, u16, u16, u16, u8, u8, u16, u8, bool) {
    (
        r.a,
        r.x,
        r.y,
        r.s,
        r.d,
        r.dbr,
        r.pbr,
        r.pc,
        r.p.into(),
        r.emulation_mode,
    )
}

fn vectors(m: &mut Machine, handler: &[u8]) {
    m.bus.map(0xFFEE, &[0x00, 0x90], false);
    m.bus.map(0xFFEA, &[0x00, 0x92], false);
    m.bus.map(IRQ, handler, false);
    m.bus.map(NMI, &[0x40, 0xEA], false); // RTI
}

#[test]
fn reset_bootstrap_enters_native_mode_and_sets_stack_and_direct_page() {
    let mut m = Machine::default();
    m.bus.map(0xFFFC, &[0x00, 0x80], false);
    m.bus.map(
        0x8000,
        &[
            0x78, // SEI
            0x18, 0xFB, // CLC / XCE
            0xC2, 0x30, // REP #$30
            0xA9, 0xFF, 0x3F, 0x1B, // LDA #$3FFF / TCS
            0xA9, 0x00, 0x20, 0x5B, // LDA #$2000 / TCD
            0xEA,
        ],
        false,
    );
    m.cpu.reset(&mut m.bus);
    assert!(m.cpu.registers().emulation_mode);
    m.run_to(0x800D, 100);
    let r = m.cpu.registers();
    assert!(!r.emulation_mode);
    assert_eq!(u8::from(r.p) & 0x3C, 0x04);
    assert_eq!((r.s, r.d, r.pbr, r.dbr), (0x3FFF, 0x2000, 0, 0));
}

#[test]
fn width_changes_preserve_hidden_accumulator_and_truncate_indexes() {
    let mut m = code(&[
        0xA9, 0xCD, 0xAB, // LDA #$ABCD
        0xA2, 0x34, 0x12, // LDX #$1234
        0xA0, 0x78, 0x56, // LDY #$5678
        0xE2, 0x20, // SEP #$20: A becomes eight bits
        0xA9, 0xEF, // LDA #$EF: B must survive
        0xE2, 0x10, // SEP #$10: X/Y high bytes cleared
        0xC2, 0x30, // REP #$30
        0xEA,
    ]);
    m.run_to(ENTRY + 13, 100);
    assert_eq!((m.cpu.registers().a, m.cpu.registers().x), (0xABEF, 0x1234));
    m.run_to(ENTRY + 17, 100);
    let r = m.cpu.registers();
    assert_eq!((r.a, r.x, r.y), (0xABEF, 0x0034, 0x0078));
    assert_eq!(u8::from(r.p) & 0x30, 0);
}

#[test]
fn binary_add_subtract_and_flags_match_host_arithmetic() {
    for (a, b) in [
        (0u16, 0u16),
        (0, 1),
        (0xFFFF, 1),
        (0x7FFF, 1),
        (0x8000, 1),
        (0x8000, 0x8000),
        (0x1234, 0xABCD),
    ] {
        for subtract in [false, true] {
            for carry in [false, true] {
                let [lo, hi] = b.to_le_bytes();
                let mut m = code(&[if subtract { 0xE9 } else { 0x69 }, lo, hi, 0xEA]);
                edit(&mut m, |r| {
                    r.a = a;
                    r.p = (0x04 | u8::from(carry)).into();
                });
                m.step();
                let (value, c, v) = if subtract {
                    let wide = i32::from(a) - i32::from(b) - i32::from(!carry);
                    let value = wide as u16;
                    (value, wide >= 0, (a ^ b) & (a ^ value) & 0x8000 != 0)
                } else {
                    let wide = u32::from(a) + u32::from(b) + u32::from(carry);
                    let value = wide as u16;
                    (value, wide > 0xFFFF, !(a ^ b) & (a ^ value) & 0x8000 != 0)
                };
                let flags = u8::from(c)
                    | (u8::from(value == 0) << 1)
                    | (u8::from(v) << 6)
                    | (u8::from(value & 0x8000 != 0) << 7);
                assert_eq!(
                    (m.cpu.registers().a, u8::from(m.cpu.registers().p) & 0xC3),
                    (value, flags),
                    "a={a:04X} b={b:04X} subtract={subtract} carry={carry}"
                );
            }
        }
    }
}

#[test]
fn decimal_arithmetic_is_available_in_the_cpu_core() {
    let mut m = code(&[0xF8, 0xA9, 0x99, 0x99, 0x18, 0x69, 0x01, 0x00, 0xEA]);
    m.run_to(ENTRY + 8, 100);
    assert_eq!(m.cpu.registers().a, 0);
    assert_eq!(u8::from(m.cpu.registers().p) & 0x0B, 0x0B);
}

#[test]
fn counted_loop_uses_sixteen_bit_indexes() {
    let mut m = code(&[
        0xA2, 0x01, 0x01, // LDX #257
        0xA9, 0x00, 0x00, // LDA #0
        0x1A, 0xCA, // INC A / DEX
        0xD0, 0xFC, // BNE back to INC A
        0xEA,
    ]);
    m.run_to(ENTRY + 10, 3000);
    assert_eq!((m.cpu.registers().a, m.cpu.registers().x), (257, 0));
}

#[test]
fn long_word_access_crosses_bank_boundary_with_exact_byte_order() {
    let mut m = code(&[
        0xAF, 0xFF, 0xFF, 0x12, // LDA $12:FFFF
        0x8F, 0xFF, 0xFF, 0x34, // STA $34:FFFF
        0xEA,
    ]);
    edit(&mut m, |r| r.dbr = 0x7E);
    m.bus.map(0x12FFFF, &[0xCD, 0xAB], false);
    m.bus.map_ram(0x34FFFF, 2);
    m.run_to(ENTRY + 8, 100);
    assert_eq!(m.cpu.registers().a, 0xABCD);
    assert_eq!(m.bus.writes(), [(0x34FFFF, 0xCD), (0x350000, 0xAB)]);
}

#[test]
fn long_indexing_and_direct_page_indirect_pointer_keep_all_address_bits() {
    let mut m = code(&[
        0xBF, 0xFF, 0xFF, 0x12, // LDA $12:FFFF,X
        0xB7, 0x10, // LDA [$10],Y
        0xEA,
    ]);
    edit(&mut m, |r| {
        r.d = 0x2000;
        r.dbr = 0x7E;
        r.x = 1;
        r.y = 2;
    });
    m.bus.map(0x130000, &[0x78, 0x56, 0x34], false);
    m.bus.map(0x2010, &[0xFF, 0xFF, 0x12], false);
    m.step();
    assert_eq!(m.cpu.registers().a, 0x5678);
    m.step();
    assert_eq!(m.cpu.registers().a, 0x3456);
}

#[test]
fn stack_relative_word_round_trip_balances_native_stack() {
    let mut m = code(&[
        0xA9, 0x34, 0x12, 0x48, // LDA #$1234 / PHA
        0xA3, 0x01, // LDA 1,S
        0x1A, // INC A
        0x83, 0x01, // STA 1,S
        0x68, 0xEA, // PLA
    ]);
    m.run_to(ENTRY + 10, 100);
    assert_eq!((m.cpu.registers().a, m.cpu.registers().s), (0x1235, 0x3FFF));
    assert_eq!(m.bus.word(0x3FFE), 0x1235);
}

#[test]
fn nested_far_calls_save_program_banks_and_return_addresses() {
    let mut m = code(&[0x22, 0x00, 0x90, 0x02, 0xEA]); // JSL $02:9000
    m.bus.map(0x029000, &[0x22, 0x00, 0xA0, 0x03, 0x6B], false);
    m.bus.map(0x03A000, &[0x1A, 0x6B], false); // INC A / RTL
    m.run_to(0x03A000, 100);
    assert_eq!(m.cpu.registers().s, 0x3FF9);
    assert_eq!(
        m.bus.writes(),
        [
            (0x3FFF, 1),
            (0x3FFE, 0x80),
            (0x3FFD, 3),
            (0x3FFC, 2),
            (0x3FFB, 0x90),
            (0x3FFA, 3),
        ]
    );
    m.run_to(ENTRY + 4, 100);
    assert_eq!((m.cpu.registers().a, m.cpu.registers().s), (1, 0x3FFF));
}

#[test]
fn indirect_long_jump_loads_three_byte_code_pointer() {
    let mut m = code(&[0xDC, 0x00, 0x20]); // JML [$2000]
    m.bus.map(0x2000, &[0x34, 0x12, 0x56], false);
    m.bus.map(0x561234, &[0xEA], false);
    m.run_to(0x561234, 100);
    assert_eq!(m.cpu.registers().s, 0x3FFF);
}

#[test]
fn program_counter_wraps_within_its_program_bank() {
    let mut m = Machine::native(0x01FFFE);
    m.bus.map(0x01FFFE, &[0xA9, 0x34], false);
    m.bus.map(0x010000, &[0x12, 0xEA], false);
    m.step();
    assert_eq!((m.pc(), m.cpu.registers().a), (0x010001, 0x1234));
}

#[test]
fn masked_irq_waits_until_cli_and_nested_status_restore_keeps_outer_mask() {
    // The first PHP saves I=1; CLI is the first instruction allowed to unmask.
    let mut m = code(&[
        0x08, 0x78, 0x08, 0x78, 0x28, 0xEA, 0x28, 0xEA, 0x58, 0xEA, 0xEA,
    ]);
    vectors(&mut m, &[0x40, 0xEA]);
    m.bus.irq = true;
    m.run_to(ENTRY + 8, 100);
    assert_eq!(u8::from(m.cpu.registers().p) & 4, 4);
    assert_eq!(m.cpu.registers().s, 0x3FFF);
    assert!(
        !m.bus
            .trace
            .iter()
            .any(|t| matches!(t.access, Access::Read(0xFFEE, _)))
    );
    m.run_to(IRQ, 100);
    assert_eq!(m.cpu.registers().s, 0x3FFB);
    m.bus.irq = false;
    m.step(); // RTI
    assert_eq!(m.cpu.registers().s, 0x3FFF);
    assert_eq!(u8::from(m.cpu.registers().p) & 4, 0);
}

// Force full-width saves before touching A, including hidden B when interrupted M=1.
const SAVING_HANDLER: &[u8] = &[
    0xC2, 0x30, 0x48, 0xDA, 0x5A, 0x0B, 0x8B, // REP / PHA PHX PHY PHD PHB
    0xA9, 0x00, 0x22, 0x5B, // poison D
    0xA2, 0xAD, 0xDE, 0xA0, 0xEF, 0xBE, // poison X/Y
    0xE2, 0x20, 0xA9, 0x77, 0x48, 0xAB, // poison DBR via PHA/PLB
    0xC2, 0x30, 0xAB, 0x2B, 0x7A, 0xFA, 0x68, // restore DBR D Y X full A
    0x40, 0xEA, // RTI
];

#[test]
fn irq_frame_and_assembly_save_restore_preserve_every_register_in_all_widths() {
    for widths in [0, 0x10, 0x20, 0x30] {
        let mut m = code(&[0xEA, 0xEA, 0xEA]);
        vectors(&mut m, SAVING_HANDLER);
        edit(&mut m, |r| {
            r.a = 0xABCD;
            r.x = if widths & 0x10 == 0 { 0x1234 } else { 0x34 };
            r.y = if widths & 0x10 == 0 { 0x5678 } else { 0x78 };
            r.d = 0x2100;
            r.dbr = 0x42;
            r.p = (0xC9 | widths).into();
        });
        m.bus.irq = true;
        m.step(); // NOP polls the asserted IRQ.
        let before = m.cpu.registers().clone();
        m.run_to(IRQ, 32);
        assert_eq!(
            m.bus.writes(),
            [
                (0x3FFF, 1),
                (0x3FFE, 0x80),
                (0x3FFD, 1),
                (0x3FFC, 0xC9 | widths),
            ]
        );
        assert_eq!(u8::from(m.cpu.registers().p) & 0x0C, 4); // I set, D cleared
        m.bus.irq = false;
        m.run_to(ENTRY + 1, 200);
        assert_eq!(
            registers(m.cpu.registers()),
            registers(&before),
            "widths={widths:02X}"
        );
    }
}

#[test]
fn nmi_works_under_irq_masking_and_held_level_does_not_retrigger() {
    let mut m = code(&[0xEA, 0x80, 0xFD]); // NOP / BRA back
    vectors(&mut m, &[0x40, 0xEA]);
    m.bus.irq = true;
    m.bus.nmi = true;
    m.step();
    let before = m.cpu.registers().clone();
    m.run_to(NMI, 32);
    assert_eq!(m.bus.nmi_acknowledgements, 1);
    m.step(); // RTI
    assert_eq!(registers(m.cpu.registers()), registers(&before));
    for _ in 0..40 {
        m.tick();
    }
    assert_eq!(m.bus.nmi_acknowledgements, 1);
    m.bus.nmi = false;
    for _ in 0..10 {
        m.tick();
    }
    m.bus.nmi = true;
    m.run_to(NMI, 32);
    assert_eq!(m.bus.nmi_acknowledgements, 2);
}

#[test]
fn irq_asserted_at_each_cycle_of_long_store_waits_for_complete_word() {
    for cycle in 0..6 {
        let mut m = code(&[0x8F, 0xFF, 0xFF, 0x12, 0xEA, 0xEA]); // STA $12:FFFF
        edit(&mut m, |r| {
            r.a = 0xABCD;
            r.p = 0.into();
        });
        m.bus.map_ram(0x12FFFF, 2);
        vectors(&mut m, &[0x40, 0xEA]);
        for _ in 0..cycle {
            m.tick();
        }
        m.bus.irq = true;
        m.run_to(IRQ, 100);
        assert_eq!(m.bus.word(0x12FFFF), 0xABCD);
        let writes = m.bus.writes();
        assert_eq!(&writes[..2], [(0x12FFFF, 0xCD), (0x130000, 0xAB)]);
        assert_eq!(m.bus.word(0x3FFD), 0x8004);
    }
}

#[test]
fn wai_wakes_on_masked_irq_without_entering_handler() {
    let mut m = code(&[0xCB, 0xA9, 0x34, 0x12, 0xEA]);
    vectors(&mut m, &[0x40, 0xEA]);
    for _ in 0..20 {
        m.tick();
    }
    assert_eq!(m.pc(), ENTRY + 1);
    assert_eq!(m.cpu.registers().a, 0);
    m.bus.irq = true;
    m.run_to(ENTRY + 4, 100);
    assert_eq!((m.cpu.registers().a, m.cpu.registers().s), (0x1234, 0x3FFF));
    assert!(m.bus.writes().is_empty());
}

#[test]
fn cloned_mid_instruction_state_resumes_with_identical_bus_trace() {
    let mut m = code(&[0xAF, 0xFF, 0xFF, 0x12, 0xEA]);
    m.bus.map(0x12FFFF, &[0x34, 0x12], false);
    for _ in 0..3 {
        m.tick();
    }
    assert!(m.cpu.is_mid_instruction());
    let mut clone = m.clone();
    m.run_to(ENTRY + 4, 100);
    clone.run_to(ENTRY + 4, 100);
    assert_eq!(
        registers(m.cpu.registers()),
        registers(clone.cpu.registers())
    );
    assert_eq!(m.bus.trace, clone.bus.trace);
}

#[test]
fn block_moves_resume_after_irq_with_count_indexes_and_banks_intact() {
    // MVN/MVP wrap their 16-bit indexes within the selected source/destination banks.
    // Their literal operand order is destination bank, source bank.
    for backwards in [false, true] {
        for interrupt_cycle in 0..8 {
            let mut m = code(&[if backwards { 0x44 } else { 0x54 }, 0x34, 0x12, 0xEA]);
            edit(&mut m, |r| {
                r.a = 3;
                r.x = if backwards { 1 } else { 0xFFFE };
                r.y = if backwards { 0x0103 } else { 0x0100 };
                r.p = 0.into();
            });
            m.bus.map(0x12FFFE, &[0x11, 0x22], false);
            m.bus.map(0x120000, &[0x33, 0x44], false);
            m.bus.map_ram(0x340100, 4);
            vectors(&mut m, SAVING_HANDLER);
            for _ in 0..interrupt_cycle {
                m.tick();
            }
            m.bus.irq = true;
            m.run_to(IRQ, 100);
            assert_eq!(
                m.bus.word(0x3FFD),
                0x8000,
                "interrupt must restart block move"
            );
            let copied = m
                .bus
                .writes()
                .iter()
                .filter(|(a, _)| (0x340100..0x340104).contains(a))
                .count();
            assert!((1..=2).contains(&copied));
            m.bus.irq = false;
            m.run_to(ENTRY + 3, 300);
            assert_eq!(
                (0..4).map(|i| m.bus.peek(0x340100 + i)).collect::<Vec<_>>(),
                [0x11, 0x22, 0x33, 0x44],
                "backwards={backwards} cycle={interrupt_cycle}"
            );
            let r = m.cpu.registers();
            assert_eq!((r.a, r.s, r.dbr), (0xFFFF, 0x3FFF, 0x34));
            assert_eq!(
                (r.x, r.y),
                if backwards {
                    (0xFFFD, 0x00FF)
                } else {
                    (2, 0x0104)
                }
            );
            assert_eq!(
                m.bus
                    .writes()
                    .iter()
                    .filter(|(a, _)| (0x340100..0x340104).contains(a))
                    .count(),
                4
            );
        }
    }
}

#[test]
fn nmi_can_interrupt_irq_handler_and_return_to_the_interrupted_handler() {
    let mut m = code(&[0xEA, 0xEA, 0xEA]);
    vectors(&mut m, SAVING_HANDLER);
    edit(&mut m, |r| {
        r.p = 0x20.into();
        r.a = 0xABCD;
        r.x = 0x1234;
    });
    m.bus.irq = true;
    m.step();
    let before = m.cpu.registers().clone();
    m.run_to(IRQ, 32);
    m.bus.irq = false;
    m.step(); // REP #$30
    m.step(); // PHA full A
    m.bus.nmi = true;
    m.step(); // PHX polls NMI
    let handler_before = m.cpu.registers().clone();
    m.run_to(NMI, 32);
    m.bus.nmi = false;
    m.step(); // RTI to the IRQ handler
    assert_eq!(registers(m.cpu.registers()), registers(&handler_before));
    m.run_to(ENTRY + 1, 200);
    assert_eq!(registers(m.cpu.registers()), registers(&before));
}

#[test]
fn byte_store_does_not_touch_adjacent_device_register() {
    let mut m = code(&[0xE2, 0x20, 0xA9, 0x5A, 0x8F, 0x00, 0x00, 0x80, 0xEA]);
    m.bus.map_ram(0x800000, 1); // neighboring byte deliberately unmapped
    m.run_to(ENTRY + 8, 100);
    assert_eq!(m.bus.writes(), [(0x800000, 0x5A)]);
}

#[test]
#[should_panic(expected = "unmapped or read-only write at $002FFF")]
fn native_stack_guard_rejects_underflow() {
    let mut m = code(&[0x48, 0xEA]); // 16-bit PHA
    edit(&mut m, |r| r.s = 0x3000);
    m.step();
}

#[test]
fn cycle_budget_stops_an_endless_program() {
    let mut m = code(&[0x80, 0xFE]); // BRA to itself
    assert!(!m.run_until(100, |_| false));
    assert_eq!(m.bus.cycles, 100);
}

#[test]
#[should_panic(expected = "unmapped read at $124000")]
fn unmapped_access_is_rejected() {
    let mut m = code(&[0xAF, 0x00, 0x40, 0x12]);
    m.step();
}

#[test]
#[should_panic(expected = "unmapped or read-only write at $018000")]
fn code_write_is_rejected() {
    let mut m = code(&[0x8F, 0x00, 0x80, 0x01]);
    m.step();
}
