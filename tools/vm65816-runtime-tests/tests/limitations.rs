//! Expected architectural behavior which the pinned upstream core does not satisfy.
//! Run explicitly with `cargo test --test limitations -- --ignored --nocapture`.
//! These remain failing reproducers, not assertions accepting incorrect CPU behavior.
use actionc_vm65816_tests::Machine;

#[test]
#[ignore = "upstream samples NMI at selected instruction cycles; a short raw pulse can be lost"]
fn nmi_pulse_inside_an_instruction_is_remembered() {
    let mut m = Machine::native(0x018000);
    m.bus.map(
        0x018000,
        &[
            0xAF, 0x00, 0x00, 0x12, // six-cycle 16-bit LDA long
            0xEA, 0x80, 0xFD, // NOP / BRA back to NOP
        ],
        false,
    );
    m.bus.map(0x120000, &[0x34, 0x12], false);
    m.bus.map(0xFFEA, &[0x00, 0x90], false);
    m.bus.map(0x9000, &[0x40, 0xEA], false);
    m.tick(); // fetch LDA opcode
    m.bus.nmi = true;
    m.tick();
    m.tick(); // two full cycles asserted, deassert before LDA completes
    m.bus.nmi = false;
    assert!(
        m.run_until(100, |m| m.bus.nmi_acknowledgements == 1),
        "NMI pulse lost: PC=${:06X}, A=${:04X}",
        m.pc(),
        m.cpu.registers().a
    );
}

#[test]
#[ignore = "upstream reset() leaves D and DBR unchanged"]
fn reset_clears_direct_page_and_data_bank_registers() {
    let mut m = Machine::native(0x018000);
    m.bus.map(0xFFFC, &[0x00, 0x80], false);
    let mut r = m.cpu.registers().clone();
    r.d = 0x2345;
    r.dbr = 0x67;
    m.cpu.set_registers(r);
    m.cpu.reset(&mut m.bus);
    assert_eq!((m.cpu.registers().d, m.cpu.registers().dbr), (0, 0));
}
