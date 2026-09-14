//! Literal encodings checked against Motorola's M68000 Programmer's Reference
//! Manual, instruction entries MOVE, ADDI, Bcc, JSR, LINK, UNLK, RTS, TRAP, STOP:
//! https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf
//! These bytes are intentionally independent of actionc's encoder.
use actionc_vm68k_tests::{Machine, Memory, Outcome, STACK_TOP, TRAMPOLINE};
use r68k::cpu::{AccessType, Exception, ProcessingState};

fn machine(words: &[u16]) -> Machine {
    let mut memory = Memory::default();
    let mut code: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    code.extend_from_slice(&[0x4e, 0x71, 0x4e, 0x71]);
    memory.map(0x10000, &code, false, true).unwrap();
    memory.map(0x20000, &[0xcd; 16], true, false).unwrap();
    Machine::new(memory, 0x10000).unwrap()
}

#[test]
fn byte_word_long_transfers_and_big_endian() {
    let mut vm = machine(&[
        0x13fc, 0x0081, 0x0002, 0x0000, // MOVE.B #$81,$20000.L
        0x33fc, 0x8765, 0x0002, 0x0002, // MOVE.W #$8765,$20002.L
        0x23fc, 0x1234, 0x89ab, 0x0002, 0x0004, // MOVE.L #$123489ab,$20004.L
        0x1039, 0x0002, 0x0000, // MOVE.B $20000.L,D0
        0x3239, 0x0002, 0x0002, // MOVE.W $20002.L,D1
        0x2039, 0x0002, 0x0004, // MOVE.L $20004.L,D0
        0x4e75,
    ]);
    let result = vm.run(100);
    result.assert_completed();
    assert_eq!(result.registers[0], 0x123489ab);
    assert_eq!(result.registers[1] & 0xffff, 0x8765);
    assert_eq!(
        vm.cpu.mem.bytes(0x20000, 8).unwrap(),
        &[0x81, 0xcd, 0x87, 0x65, 0x12, 0x34, 0x89, 0xab]
    );
    assert!(result.cycles > result.steps);
}

#[test]
fn arithmetic_flags_and_conditional_branch() {
    let mut vm = machine(&[
        0x7000, // MOVEQ #0,D0
        0x103c, 0x007f, // MOVE.B #127,D0
        0x0600, 0x0001, // ADDI.B #1,D0: N=1,V=1,C=0,Z=0
        0x6902, 0x4afc, // BVS skip ILLEGAL
        0x6b02, 0x4afc, // BMI skip ILLEGAL
        0x6402, 0x4afc, // BCC skip ILLEGAL
        0x6602, 0x4afc, // BNE skip ILLEGAL
        0x0600, 0x0080, // ADDI.B #128,D0: Z=1,C=1
        0x6702, 0x4afc, // BEQ
        0x6502, 0x4afc, // BCS
        0x4e75,
    ]);
    vm.run(100).assert_completed();
    assert_eq!(vm.cpu.dar[0], 0);
}

#[test]
fn literal_division_primitives_preserve_shift_extend_and_quick_flags() {
    let mut vm = machine(&[
        0x203c, 0xffff, 0xffff, // MOVE.L #-1,D0
        0x7200, // MOVEQ #0,D1
        0xe388, // LSL.L #1,D0 (X=1)
        0xd381, // ADDX.L D1,D1 -> 1
        0x0c81, 0, 1, // CMPI.L #1,D1
        0x6702, 0x4afc, // BEQ, else ILLEGAL
        0x5280, // ADDQ.L #1,D0 -> $ffffffff
        0x5381, // SUBQ.L #1,D1 -> 0
        0x6702, 0x4afc, 0x4e75,
    ]);
    vm.run(100).assert_completed();
    assert_eq!(vm.cpu.dar[0], u32::MAX);
    assert_eq!(vm.cpu.dar[1], 0);
}

#[test]
fn immediate_word_logic_and_moveq_sign_extension_are_qualified() {
    let mut vm = machine(&[
        0x7280, // MOVEQ #-128,D1
        0x0c81, 0xffff, 0xff80, // CMPI.L #-128,D1
        0x6702, 0x4afc, 0x7000, // MOVEQ #0,D0
        0x0040, 0x8000, // ORI.W #$8000,D0
        0x6b02, 0x4afc, // BMI
        0x0440, 1, // SUBI.W #1,D0 -> $7fff, overflow
        0x6902, 0x4afc, // BVS
        0x0a40, 0xffff, // EORI.W #$ffff,D0 -> $8000
        0x0280, 0, 0x7fff, // ANDI.L #$7fff,D0 -> 0
        0x6702, 0x4afc, // BEQ
        0x4e75,
    ]);
    vm.run(100).assert_completed();
    assert_eq!(vm.cpu.dar[0], 0);
    assert_eq!(vm.cpu.dar[1], 0xffffff80);
}

#[test]
fn word_branches_use_the_opcode_pc_plus_two() {
    let mut vm = machine(&[
        0x7003, // MOVEQ #3,D0
        0x5340, // SUBQ.W #1,D0 (offset 2)
        0x6600, 0xfffc, // BNE.W offset 2: (4+2)-4
        0x6000, 0x0004, // BRA.W offset 14: (8+2)+4
        0x4afc, 0x4e75,
    ]);
    vm.run(100).assert_completed();
    assert_eq!(vm.cpu.dar[0], 0);
}

#[test]
fn nested_calls_link_unlink_and_return_preserve_stack() {
    let mut vm = machine(&[
        0x4e56, 0xfff8, // LINK A6,#-8
        0x2d7c, 0x1234, 0x5678, 0xfffc, // MOVE.L #$12345678,-4(A6)
        0x4eb9, 0x0001, 0x001a, // JSR $1001a.L
        0x202e, 0xfffc, // MOVE.L -4(A6),D0
        0x4e5e, 0x4e75, // UNLK A6; RTS
        0x4e56, 0xfffc, 0x722a, 0x4e5e, 0x4e75, // nested LINK/MOVEQ/UNLK/RTS
    ]);
    let result = vm.run(100);
    result.assert_completed();
    assert_eq!(result.registers[0], 0x12345678);
    assert_eq!(result.registers[1], 42);
}

#[test]
fn exceptions_do_not_disappear_into_default_vector_handling() {
    assert!(matches!(
        machine(&[0x4afc]).run(10).outcome,
        Outcome::Exception(Exception::IllegalInstruction(..))
    ));
    assert!(matches!(
        machine(&[0x4e4f]).run(10).outcome,
        Outcome::Exception(Exception::Trap(47, _))
    ));
    for (opcode, write) in [
        (0x3039, false),
        (0x2039, false),
        (0x33c0, true),
        (0x23c0, true),
    ] {
        let result = machine(&[opcode, 0x0002, 0x0001]).run(10);
        match result.outcome {
            Outcome::Exception(Exception::AddressError {
                address: 0x20001,
                access_type,
                ..
            }) => assert_eq!(matches!(access_type, AccessType::Write), write),
            _ => panic!("{result:#?}"),
        }
    }
}

#[test]
fn timeout_stopped_halted_and_abi_violation_are_distinct() {
    let result = machine(&[0x60fe]).run(12); // BRA -2
    assert!(matches!(result.outcome, Outcome::BudgetExhausted));
    assert_eq!(result.steps, 12);
    assert!(matches!(
        machine(&[0x4e72, 0x2700]).run(12).outcome,
        Outcome::Stopped
    ));
    let mut halted = machine(&[0x4e75]);
    halted.cpu.processing_state = ProcessingState::Halted;
    assert!(matches!(halted.run(12).outcome, Outcome::Halted));
    assert!(matches!(
        machine(&[0x742a, 0x4e75]).run(12).outcome,
        Outcome::AbiViolation(_)
    )); // clobber D2
    // Jump to completion without consuming the return address: unbalanced A7.
    assert!(matches!(
        machine(&[0x4ef9, 0, (TRAMPOLINE + 6) as u16])
            .run(12)
            .outcome,
        Outcome::AbiViolation(_)
    ));
}

#[test]
fn memory_violations_and_guards_are_detected() {
    for words in [
        vec![0x1039, 0x0010, 0x0000],                   // read beyond RAM
        vec![0x13c0, 0x0001, 0x0000],                   // write protected code
        vec![0x13c0, (STACK_TOP >> 16) as u16, 0xffff], // unmapped guard above stack
        vec![0x1039, 0x0003, 0x0000],                   // unmapped in-range read
    ] {
        assert!(matches!(
            machine(&words).run(10).outcome,
            Outcome::MemoryViolation(_)
        ));
    }
    let mut memory = Memory::default();
    memory.map(0x10000, &[0; 8], false, true).unwrap();
    assert!(memory.map(0x10004, &[0; 8], true, false).is_err());
    assert!(Machine::new(memory, 0x10001).is_err());
}

#[test]
fn repeated_and_parallel_instances_are_isolated() {
    std::thread::scope(|scope| {
        for value in 0..8u16 {
            scope.spawn(move || {
                for _ in 0..3 {
                    let mut vm = machine(&[0x33fc, value, 0x0002, 0x0000, 0x4e75]);
                    vm.run(20).assert_completed();
                    assert_eq!(vm.cpu.mem.bytes(0x20000, 2).unwrap(), value.to_be_bytes());
                }
            });
        }
    });
}
