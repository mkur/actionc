use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn first_byte_call_argument_uses_a_not_stack_placeholder() {
    let (formatted, bytes) = compile_mir6502_fixture("call_byte_arg.act");
    assert!(formatted.contains("args=[#$07.b -> a]"));
    assert!(!formatted.contains("args=[#$07.b -> stack $0000+0]"));

    assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x07]));
    assert!(
        bytes
            .windows(3)
            .any(|bytes| bytes[0] == 0x20 || bytes[0] == 0x4C)
    );
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x00, 0x00]));
}

#[test]
fn first_word_call_argument_uses_a_x_not_stack_placeholder() {
    let (formatted, bytes) = compile_mir6502_fixture("call_word_arg.act");
    assert!(formatted.contains("args=[#$1234.w -> a:x]"));
    assert!(!formatted.contains("args=[#$1234.w -> stack $0000+0]"));

    assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x34]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA2, 0x12]));
    assert!(
        bytes
            .windows(3)
            .any(|bytes| bytes[0] == 0x20 || bytes[0] == 0x4C)
    );
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x00, 0x00]));
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x01, 0x00]));
}

#[test]
fn first_int_call_argument_uses_a_x_not_stack_placeholder() {
    let (formatted, bytes) = compile_mir6502_fixture("call_int_arg.act");
    assert!(formatted.contains("args=[#$0102.w -> a:x]"));
    assert!(!formatted.contains("args=[#$0102.w -> stack $0000+0]"));

    assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x02]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA2, 0x01]));
    assert!(
        bytes
            .windows(3)
            .any(|bytes| bytes[0] == 0x20 || bytes[0] == 0x4C)
    );
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x00, 0x00]));
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x01, 0x00]));
}

#[test]
fn mixed_multi_argument_call_loads_register_home_last() {
    let (formatted, bytes) = compile_mir6502_fixture("call_many_args_sargs.act");
    assert!(formatted.contains("#$01.b -> a"));
    assert!(formatted.contains("#$2345.w -> x:y"));
    assert!(formatted.contains("#$06.b -> fixed_zp $A3"));

    let fixed_zp_c = [0xA9, 0x06, 0x85, 0xA3];
    let registers = [0xA9, 0x01, 0xA2, 0x45, 0xA0, 0x23, 0x4C];
    assert!(
        bytes
            .windows(fixed_zp_c.len())
            .any(|bytes| bytes == fixed_zp_c)
    );
    assert!(
        bytes
            .windows(registers.len())
            .any(|bytes| bytes == registers)
    );
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x01, 0x00]));
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x02, 0x00]));
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x8D, 0x03, 0x00]));
}

#[test]
fn user_calls_may_omit_trailing_declared_params() {
    let (formatted, _bytes) = compile_mir6502_fixture("call_omitted_trailing_params.act");

    assert!(formatted.contains("call r0 args=[static_addr s0.w -> a:x]"));
    assert!(
        formatted.contains(
            "call r0 args=[static_addr s1.w -> a:x, word(#$01, #$00).w -> y:fixed_zp $A3]"
        )
    );
    assert!(formatted.contains(
        "call r0 args=[static_addr s2.w -> a:x, word(#$01, #$00).w -> y:fixed_zp $A3, word(#$02, #$00).w -> fixed_zp $A4:fixed_zp $A5, word(#$03, #$00).w -> fixed_zp $A6:fixed_zp $A7]"
    ));
}

#[test]
fn callee_captures_action_abi_params_on_entry() {
    let (_formatted, bytes) = compile_mir6502_fixture("call_card_byte_param_prologue.act");
    let prologue = [
        0x20, 0xF5, 0xA0, // JSR SArgs
        0x00, 0x30, 0x02, // frame $3000, three argument bytes
    ];
    let call_args = [
        0xA9, 0x0A, // LDA #x low
        0xA2, 0x00, // LDX #x high
        0xA0, 0x02, // LDY #y
        0x20, // JSR test
    ];
    assert!(bytes.windows(prologue.len()).any(|bytes| bytes == prologue));
    assert!(
        bytes
            .windows(call_args.len())
            .any(|bytes| bytes == call_args)
    );
    assert!(!bytes.windows(3).any(|bytes| bytes == [0xAD, 0x02, 0x00]));
}

#[test]
fn array_param_string_literal_call_captures_a_x_and_indexes_indirectly() {
    let (formatted, bytes) = compile_mir6502_fixture("array_param_string_length_loop.act");

    assert!(formatted.contains("call r0 args=[static_addr s0.w -> a:x]"));
    assert!(formatted.contains("v1 =.w load param p0+0"));
    assert!(formatted.contains("v2 =.b load computed v1[#$00;1]+0"));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x8E, 0x09, 0x30]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
}

#[test]
fn runtime_helper_set_redirects_sargs_to_generated_routine() {
    let (_formatted, bytes) = compile_mir6502_fixture("runtime_helper_set_sargs.act");
    let generated_sargs_prologue = [
        0x20, 0x03, 0x30, // JSR r_Par
        0x00, 0x30, 0x02, // frame $3000, three argument bytes
    ];
    assert!(
        bytes
            .windows(generated_sargs_prologue.len())
            .any(|bytes| bytes == generated_sargs_prologue)
    );
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x20, 0xF5, 0xA0]));
}

#[test]
fn sargs_byte_params_are_packed_in_callee_storage() {
    let (_formatted, bytes) = compile_mir6502_fixture("call_four_byte_params.act");
    let prologue = [
        0x20, 0xF5, 0xA0, // JSR SArgs
        0x00, 0x30, 0x03, // frame $3000, four argument bytes
    ];
    assert!(bytes.windows(prologue.len()).any(|bytes| bytes == prologue));
    for address in [0x3000u16, 0x3001, 0x3002, 0x3003] {
        let [low, high] = address.to_le_bytes();
        assert!(bytes
            .windows(3)
            .any(|bytes| matches!(bytes, [0xAD | 0x6D, b_low, b_high] if *b_low == low && *b_high == high)));
    }
    for address in [0x3004u16, 0x3008, 0x300C] {
        let [low, high] = address.to_le_bytes();
        assert!(!bytes
            .windows(3)
            .any(|bytes| matches!(bytes, [0xAD | 0x6D, b_low, b_high] if *b_low == low && *b_high == high)));
    }
}

#[test]
fn raw_machine_entry_preserves_action_abi_registers() {
    let (formatted, bytes) = compile_mir6502_fixture("raw_machine_param_entry.act");
    assert!(!formatted.contains("call SArgs"));

    let raw_entry = [
        0x85, 0xA0, // STA $A0
        0x86, 0xA1, // STX $A1
        0x84, 0xA2, // STY $A2
        0x60, // RTS
    ];
    let prologue = [
        0x20, 0xF5, 0xA0, // JSR SArgs
        0x00, 0x30, 0x04, // frame $3000, five argument bytes
    ];
    assert!(
        bytes
            .windows(raw_entry.len())
            .any(|bytes| bytes == raw_entry)
    );
    assert!(!bytes.windows(prologue.len()).any(|bytes| bytes == prologue));
}

fn compile_mir6502_fixture(name: &str) -> (String, Vec<u8>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("mir6502")
        .join(name);
    let loaded = load_program_with_expanded_source(&fixture)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", fixture.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", fixture.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let nir_program = nir::optimize_program(&nir::lower_program(&semir))
        .unwrap_or_else(|err| panic!("optimize NIR for {}: {err:?}", fixture.display()));

    let mir = mir6502::lower_program(&nir_program)
        .unwrap_or_else(|err| panic!("lower MIR6502 for {}: {err:?}", fixture.display()));
    let formatted = mir6502::format_program(&mir);

    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN)
        .unwrap_or_else(|err| panic!("emit MIR6502 for {}: {err:?}", fixture.display()));
    (formatted, output.bytes)
}
