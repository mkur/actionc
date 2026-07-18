use super::*;
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::semantic::analyze;

#[test]
fn immediate_selects_low_and_high_bytes() {
    let immediate = Immediate::new(0x1234);
    assert_eq!(immediate.low(), 0x34);
    assert_eq!(immediate.high(), 0x12);
    assert_eq!(immediate.byte(2), 0x00);
}

#[test]
fn absolute_exposes_address_bytes_and_offsets() {
    let absolute = Absolute::new(0x1234);
    assert_eq!(absolute.address(), 0x1234);
    assert_eq!(absolute.low(), 0x34);
    assert_eq!(absolute.high(), 0x12);
    assert_eq!(absolute.offset(2).address(), 0x1236);
}

#[test]
fn zero_page_exposes_address_and_offsets() {
    let zero_page = ZeroPage::new(0xFE);
    assert_eq!(zero_page.address(), 0xFE);
    assert_eq!(zero_page.offset(1).address(), 0xFF);
    assert_eq!(zero_page.offset(2).address(), 0x00);
}

#[test]
fn indexed_operand_helpers_wrap_base_operands() {
    assert_eq!(AbsoluteX::new(0x1234).absolute().address(), 0x1234);
    assert_eq!(ZeroPageX::new(0x80).zero_page().address(), 0x80);

    let pointer = runtime_zp::AFCUR;
    assert_eq!(IndexedIndirectX::new(pointer).pointer(), pointer);
    assert_eq!(IndirectIndexedY::new(pointer).pointer(), pointer);
}

#[test]
fn runtime_zero_page_locations_match_action_abi() {
    assert_eq!(runtime_zp::ARGS.address(), 0xA0);
    assert_eq!(runtime_zp::ARG0.address(), 0xA0);
    assert_eq!(runtime_zp::AFCUR.address(), 0x84);
    assert_eq!(runtime_zp::AFCUR.offset(1).address(), 0x85);
    assert_eq!(runtime_zp::VALUE_TEMP.address(), 0xAA);
    assert_eq!(runtime_zp::ELEMENT_ADDR.address(), 0xAC);
    assert_eq!(runtime_zp::ELEMENT_ADDR.offset(1).address(), 0xAD);
    assert_eq!(runtime_zp::ARRAY_ADDR.address(), 0xAE);
    assert_eq!(runtime_zp::ARRAY_ADDR.offset(1).address(), 0xAF);
    assert_eq!(runtime_zp::DEVICE.address(), 0xB7);
    assert_eq!(runtime_zp::ADDR.address(), 0xC0);
    assert_eq!(runtime_zp::TOKEN.address(), 0xC2);
}

#[test]
fn cartridge_runtime_target_uses_initialized_helper_vector_contents() {
    let helpers = RuntimeHelperTargets::default_for_target(RuntimeTarget::Cartridge);

    assert_eq!(
        helpers.target(RuntimeHelperSlot::Mul),
        RuntimeHelperTarget::Absolute(runtime_helper::CARTRIDGE_MUL)
    );
    assert_eq!(
        helpers.target(RuntimeHelperSlot::Div),
        RuntimeHelperTarget::Absolute(runtime_helper::CARTRIDGE_DIV)
    );
    assert_eq!(
        helpers.target(RuntimeHelperSlot::SArgs),
        RuntimeHelperTarget::Absolute(runtime_helper::CARTRIDGE_SARGS)
    );
}

#[test]
fn standalone_slots_runtime_target_uses_helper_vector_addresses() {
    let helpers = RuntimeHelperTargets::default_for_target(RuntimeTarget::StandaloneSlots);

    assert_eq!(
        helpers.target(RuntimeHelperSlot::Mul),
        RuntimeHelperTarget::Absolute(runtime_helper::MUL_SLOT)
    );
    assert_eq!(
        helpers.target(RuntimeHelperSlot::Div),
        RuntimeHelperTarget::Absolute(runtime_helper::DIV_SLOT)
    );
    assert_eq!(
        helpers.target(RuntimeHelperSlot::SArgs),
        RuntimeHelperTarget::Absolute(runtime_helper::SARGS_SLOT)
    );
}

#[test]
fn emitter_outputs_basic_6502_bytes() {
    let mut emitter = Emitter::new();
    emitter.emit_lda_imm(0x2A);
    emitter.emit_sta_abs(0x0600);
    emitter.emit_rts();

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0xA9, 0x2A, 0x8D, 0x00, 0x06, 0x60]
    );
}

#[test]
fn formats_readable_code_listing() {
    assert_eq!(
        format_listing(&[
            opcode::LDA_IMM,
            0x2A,
            opcode::STA_ABS,
            0x00,
            0x06,
            opcode::BNE_REL,
            0xF9,
            opcode::RTS,
        ]),
        "3000  A9 2A     LDA #$2A\n3002  8D 00 06  STA $0600\n3005  D0 F9     BNE $3000\n3007  60        RTS"
    );
}

#[test]
fn disassembles_runtime_r2_machine_block_opcodes() {
    assert_eq!(
        format_listing_with_origin(
            &[
                0xF0, 0x1B, 0xCA, 0x86, 0xC1, 0xAA, 0xF0, 0x15, 0x86, 0xC0, 0xA9, 0x00, 0xA2, 0x08,
                0x0A, 0x06, 0xC0, 0x90, 0x02, 0x65, 0xC1, 0xCA, 0xD0, 0xF6, 0x18, 0x65, 0x87, 0x85,
                0x87, 0xA5, 0x86, 0xA6, 0x87, 0x60,
            ],
            0x2C13,
        ),
        "2C13  F0 1B     BEQ $2C30\n\
             2C15  CA        DEX\n\
             2C16  86 C1     STX $C1\n\
             2C18  AA        TAX\n\
             2C19  F0 15     BEQ $2C30\n\
             2C1B  86 C0     STX $C0\n\
             2C1D  A9 00     LDA #$00\n\
             2C1F  A2 08     LDX #$08\n\
             2C21  0A        ASL A\n\
             2C22  06 C0     ASL $C0\n\
             2C24  90 02     BCC $2C28\n\
             2C26  65 C1     ADC $C1\n\
             2C28  CA        DEX\n\
             2C29  D0 F6     BNE $2C21\n\
             2C2B  18        CLC\n\
             2C2C  65 87     ADC $87\n\
             2C2E  85 87     STA $87\n\
             2C30  A5 86     LDA $86\n\
             2C32  A6 87     LDX $87\n\
             2C34  60        RTS"
    );
}

#[test]
fn decoder_recognizes_all_legal_nmos_6502_opcodes() {
    let legal_nmos_opcodes = [
        0x00, 0x01, 0x05, 0x06, 0x08, 0x09, 0x0A, 0x0D, 0x0E, 0x10, 0x11, 0x15, 0x16, 0x18, 0x19,
        0x1D, 0x1E, 0x20, 0x21, 0x24, 0x25, 0x26, 0x28, 0x29, 0x2A, 0x2C, 0x2D, 0x2E, 0x30, 0x31,
        0x35, 0x36, 0x38, 0x39, 0x3D, 0x3E, 0x40, 0x41, 0x45, 0x46, 0x48, 0x49, 0x4A, 0x4C, 0x4D,
        0x4E, 0x50, 0x51, 0x55, 0x56, 0x58, 0x59, 0x5D, 0x5E, 0x60, 0x61, 0x65, 0x66, 0x68, 0x69,
        0x6A, 0x6C, 0x6D, 0x6E, 0x70, 0x71, 0x75, 0x76, 0x78, 0x79, 0x7D, 0x7E, 0x81, 0x84, 0x85,
        0x86, 0x88, 0x8A, 0x8C, 0x8D, 0x8E, 0x90, 0x91, 0x94, 0x95, 0x96, 0x98, 0x99, 0x9A, 0x9D,
        0xA0, 0xA1, 0xA2, 0xA4, 0xA5, 0xA6, 0xA8, 0xA9, 0xAA, 0xAC, 0xAD, 0xAE, 0xB0, 0xB1, 0xB4,
        0xB5, 0xB6, 0xB8, 0xB9, 0xBA, 0xBC, 0xBD, 0xBE, 0xC0, 0xC1, 0xC4, 0xC5, 0xC6, 0xC8, 0xC9,
        0xCA, 0xCC, 0xCD, 0xCE, 0xD0, 0xD1, 0xD5, 0xD6, 0xD8, 0xD9, 0xDD, 0xDE, 0xE0, 0xE1, 0xE4,
        0xE5, 0xE6, 0xE8, 0xE9, 0xEA, 0xEC, 0xED, 0xEE, 0xF0, 0xF1, 0xF5, 0xF6, 0xF8, 0xF9, 0xFD,
        0xFE,
    ];

    assert_eq!(legal_nmos_opcodes.len(), 151);
    for opcode in 0x00..=0xFF {
        let is_legal_nmos = legal_nmos_opcodes.contains(&opcode);
        assert_eq!(
            decode_instruction(opcode).is_some(),
            is_legal_nmos,
            "unexpected NMOS 6502 decode status for ${opcode:02X}"
        );
    }
}

#[test]
fn decoder_leaves_undocumented_6502_opcodes_as_data() {
    for opcode in [
        0x02, 0x03, 0x04, 0x07, 0x0B, 0x0C, 0x0F, 0x12, 0x13, 0x14, 0x17, 0x1A, 0x1B, 0x1C, 0x1F,
    ] {
        assert!(
            decode_instruction(opcode).is_none(),
            "undocumented opcode ${opcode:02X} should stay as .BYTE"
        );
    }
}

#[test]
fn formats_sargs_metadata_as_data_after_known_helper_call() {
    assert_eq!(
        format_listing(&[
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_SARGS.low(),
            runtime_helper::CARTRIDGE_SARGS.high(),
            0x03,
            0x30,
            0x02,
            opcode::RTS,
        ]),
        "3000  20 F5 A0  JSR $A0F5\n3003  03 30 02  .BYTE $03,$30,$02\n3006  60        RTS"
    );
}

#[test]
fn custom_origin_controls_absolute_label_patches() {
    let output =
        generate_source_with_origin("PROC Foo() RETURN PROC Main() Foo() RETURN", 0x4000).unwrap();

    assert_eq!(output.origin, 0x4000);
    assert_eq!(output.run_address, 0x4001);
    assert_eq!(
        output.bytes,
        vec![opcode::RTS, opcode::JSR_ABS, 0x00, 0x40, opcode::RTS]
    );
}

#[test]
fn formats_original_style_atari_load_file() {
    let output =
        generate_source_with_origin("PROC Foo() RETURN PROC Main() Foo() RETURN", 0x4000).unwrap();

    assert_eq!(
        format_load_file(&output),
        vec![
            0xFF,
            0xFF,
            0x00,
            0x40,
            0x04,
            0x40,
            opcode::RTS,
            opcode::JSR_ABS,
            0x00,
            0x40,
            opcode::RTS,
            0xE2,
            0x02,
            0xE3,
            0x02,
            0x01,
            0x40,
        ]
    );
}

#[test]
fn compatible_generation_places_storage_in_segment() {
    let output =
        generate_compatible_source_with_origin("BYTE x PROC Main() x=1 RETURN", 0x3000).unwrap();

    assert_eq!(output.origin, 0x3000);
    assert_eq!(
        output.bytes,
        vec![
            0x00,
            opcode::JMP_ABS,
            0x04,
            0x30,
            opcode::LDY_IMM,
            0x01,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::RTS,
            opcode::RTS
        ]
    );
    assert_eq!(output.run_address, 0x3001);
    assert_eq!(format_load_file(&output)[2..6], [0x00, 0x30, 0x0A, 0x30]);
}

#[test]
fn legacy_generation_keeps_storage_out_of_code_segment() {
    let output = generate_source_with_origin("BYTE x PROC Main() x=1 RETURN", 0x3000).unwrap();

    assert_eq!(output.origin, 0x3000);
    assert_eq!(output.run_address, 0x3000);
    assert_eq!(
        output.bytes,
        vec![
            opcode::LDA_IMM,
            0x01,
            opcode::STA_ABS,
            0x00,
            0x06,
            opcode::RTS
        ]
    );
}

#[test]
fn compatible_load_file_runad_points_to_main_trampoline() {
    let output = generate_compatible_source_with_origin(
        "PROC Helper() RETURN PROC Main() Helper() RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(output.run_address, 0x3004);
    assert_eq!(
        output.bytes,
        vec![
            opcode::JMP_ABS,
            0x03,
            0x30,
            opcode::RTS,
            opcode::JMP_ABS,
            0x07,
            0x30,
            opcode::JSR_ABS,
            0x00,
            0x30,
            opcode::RTS,
            opcode::RTS,
        ]
    );
    assert_eq!(
        format_load_file(&output),
        vec![
            0xFF,
            0xFF,
            0x00,
            0x30,
            0x0B,
            0x30,
            opcode::JMP_ABS,
            0x03,
            0x30,
            opcode::RTS,
            opcode::JMP_ABS,
            0x07,
            0x30,
            opcode::JSR_ABS,
            0x00,
            0x30,
            opcode::RTS,
            opcode::RTS,
            0xE2,
            0x02,
            0xE3,
            0x02,
            0x04,
            0x30,
        ]
    );
}

#[test]
fn compatible_load_file_runad_falls_back_to_last_routine_without_main() {
    let output = generate_compatible_source_with_origin(
        "PROC First() RETURN PROC NavInit() First() RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(output.run_address, 0x3004);
    assert_eq!(
        &format_load_file(&output)[format_load_file(&output).len() - 2..],
        &[0x04, 0x30]
    );
}

#[test]
fn compatible_generation_zero_fills_storage_deterministically() {
    let output = generate_compatible_source_with_origin(
        "BYTE g CARD w BYTE FUNC F(BYTE x) BYTE t t=x RETURN(t) PROC Main() g=F(1) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..5], &[0x00; 5]);
    assert_eq!(output.bytes[5], opcode::JMP_ABS);
}

#[test]
fn compatible_generation_uses_set_code_origin_when_default_origin_is_unchanged() {
    let tokens = tokenize("SET $E=$2C00 SET $491=$2C00 PROC Main() RETURN").unwrap();
    let program = parse(&tokens).unwrap();
    analyze(&program).unwrap();
    let output = generate_compatible_with_origin(&program, CODE_ORIGIN).unwrap();

    assert_eq!(output.origin, 0x2C00);
}

#[test]
fn compatible_set_code_pointer_can_allocate_zero_page_storage() {
    let output = generate_compatible_source_with_origin(
            "SET $E=$E6 SET $F=$00 SET $491=$E6 SET $492=$00 BYTE POINTER screen SET $E=$3000 SET $491=$3000 PROC Main() screen=$4000 RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(output.bytes[0], opcode::JMP_ABS);
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xE6])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xE7])
    );
}

#[test]
fn compatible_set_symbol_to_current_location_patches_array_pointer_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY buffer PROC Main() buffer(0)=7 RETURN SET buffer=*",
        0x3000,
    )
    .unwrap();

    let patched = u16::from_le_bytes([output.bytes[0], output.bytes[1]]);
    assert_eq!(
        patched,
        output
            .origin
            .wrapping_add(output.bytes.len() as u16)
            .wrapping_sub(1)
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30,])
    );
}

#[test]
fn compatible_set_symbol_to_current_location_uses_deferred_storage_high_water() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY buffer PROC UsesBacking() BYTE ARRAY temp(300) temp(0)=1 RETURN PROC Main() UsesBacking() RETURN SET buffer=*",
        0x3000,
    )
    .unwrap();

    let buffer = storage_symbol(&output, CodegenSymbolScope::Global, "BUFFER");
    let offset = usize::from(buffer.address.wrapping_sub(output.origin));
    let patched = u16::from_le_bytes([output.bytes[offset], output.bytes[offset + 1]]);
    let skipped_end = output
        .skipped_ranges
        .iter()
        .map(|range| range.start.wrapping_add(range.len))
        .max()
        .unwrap();

    assert_eq!(patched, skipped_end);
}

#[test]
fn compatible_sized_array_numeric_initializer_binds_absolute_address() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY screen(10)=$4000 BYTE x PROC Main() x=screen(0) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x40])
    );
}

#[test]
fn compatible_large_sized_array_absolute_initializer_binds_base_address() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY allocbuf($800)=$2000 CARD POINTER allocp PROC Main() allocp=CARD POINTER(@allocbuf) RETURN",
        0x2C00,
    )
    .unwrap();

    let allocbuf = storage_symbol(&output, CodegenSymbolScope::Global, "ALLOCBUF");
    assert_eq!(allocbuf.address, 0x2C00);
    assert_eq!(allocbuf.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(&output.bytes[..6], &[0x00, 0x20, 0x00, 0x20, 0x00, 0x00]);
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x20, opcode::STA_ABS, 0x05, 0x2C])
    );
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ABS, 0x04, 0x2C])
    );
}

#[test]
fn compatible_large_fixed_array_matches_original_descriptor_and_run_address() {
    let output = generate_compatible_source_with_origin(
        "SET $491=$3000 SET $E=$3000 MODULE BYTE ARRAY allocbuf($800)=$2000 PROC Main=*() [$60]",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        output.bytes,
        [0x00, 0x20, 0x00, 0x20, opcode::RTS, opcode::RTS]
    );
    assert_eq!(routine_address(&output, "Main"), Some(0x3004));
    assert_eq!(output.run_address, 0x3004);
    assert_eq!(
        format_load_file(&output),
        [
            0xFF,
            0xFF,
            0x00,
            0x30,
            0x05,
            0x30,
            0x00,
            0x20,
            0x00,
            0x20,
            opcode::RTS,
            opcode::RTS,
            0xE2,
            0x02,
            0xE3,
            0x02,
            0x04,
            0x30,
        ]
    );
}

#[test]
fn compatible_large_fixed_array_indexes_through_original_descriptor() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY allocbuf($800)=$2000 BYTE out PROC Main() out=allocbuf(1) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..5], &[0x00, 0x20, 0x00, 0x20, 0x00]);
    assert!(output.bytes.windows(18).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
        ]));
}

#[test]
fn compatible_scalar_numeric_initializer_binds_absolute_address() {
    let output = generate_compatible_source_with_origin(
        "BYTE WSYNC=$D40A PROC Main() WSYNC=0 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STY_ABS, 0x0A, 0xD4])
    );
}

#[test]
fn compatible_scalar_symbol_initializer_aliases_existing_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE POINTER screen BYTE scl=screen, sch=screen+1 PROC Main() scl=$11 sch=$22 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x00, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x01, 0x30])
    );
}

#[test]
fn compatible_local_scalar_symbol_initializer_aliases_global_storage() {
    let output = generate_compatible_source_with_origin(
        "CARD state PROC Main() BYTE high=state+1 high=$42 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "HIGH"
            && symbol.scope == CodegenSymbolScope::Routine("Main".to_string())
            && symbol.address == 0x3001
            && symbol.size == 1
    }));
}

#[test]
fn compatible_current_location_local_numeric_initializer_binds_absolute_address() {
    let output = generate_compatible_source_with_origin(
        "PROC Main=*() BYTE skstat=$D20F WHILE skstat&$04 DO OD RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "SKSTAT"
            && symbol.scope == CodegenSymbolScope::Routine("Main".to_string())
            && symbol.address == 0xD20F
            && symbol.size == 1
    }));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x0F, 0xD2])
    );
}

#[test]
fn compatible_current_location_local_symbol_initializer_aliases_global_storage() {
    let output = generate_compatible_source_with_origin(
        "CARD state PROC Main=*() BYTE high=state+1 high=$42 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "HIGH"
            && symbol.scope == CodegenSymbolScope::Routine("Main".to_string())
            && symbol.address == 0x3001
            && symbol.size == 1
    }));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x01, 0x30])
    );
}

#[test]
fn modern_current_location_local_numeric_initializer_binds_absolute_address() {
    let output = generate_profile_source_with_origin(
        "PROC Main=*() BYTE skstat=$D20F WHILE skstat&$04 DO OD RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "SKSTAT"
            && symbol.scope == CodegenSymbolScope::Routine("Main".to_string())
            && symbol.address == 0xD20F
            && symbol.size == 1
    }));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x0F, 0xD2])
    );
}

#[test]
fn compatible_pointer_numeric_initializers_store_pointer_values() {
    let output = generate_compatible_source_with_origin(
        "BYTE POINTER bp=$4000 CARD POINTER cp=$4100 INT POINTER ip=$4200 PROC Main() RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..6], &[0x00, 0x40, 0x00, 0x41, 0x00, 0x42]);
    for (name, address, pointee_size) in [("BP", 0x3000, 1), ("CP", 0x3002, 2), ("IP", 0x3004, 2)] {
        let symbol = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == name)
            .unwrap_or_else(|| panic!("missing storage symbol {name}"));
        assert_eq!(symbol.address, address);
        assert_eq!(symbol.pointee_size, Some(pointee_size));
    }
}

#[test]
fn compatible_local_pointer_numeric_initializers_store_pointer_values() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() BYTE POINTER bp=$4000 CARD POINTER cp=$4100 INT POINTER ip=$4200 RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..6], &[0x00, 0x40, 0x00, 0x41, 0x00, 0x42]);
    for (name, address, pointee_size) in [("BP", 0x3000, 1), ("CP", 0x3002, 2), ("IP", 0x3004, 2)] {
        let symbol = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == name)
            .unwrap_or_else(|| panic!("missing storage symbol {name}"));
        assert_eq!(symbol.address, address);
        assert_eq!(symbol.pointee_size, Some(pointee_size));
    }
}

#[test]
fn compatible_local_scalar_initializer_binds_absolute_address() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() BYTE WSYNC=$D40A WSYNC=0 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STY_ABS, 0x0A, 0xD4])
    );
}

#[test]
fn local_scalar_raw_initializer_accepts_symbolic_truth_values() {
    let output = generate_profile_source_with_origin(
        "DEFINE true=\"1\", false=\"0\" PROC Main() BYTE first=[true], second=[false] RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(&output.bytes[..2], &[1, 0]);
}

#[test]
fn compatible_generation_peepholes_byte_inc_but_not_dec() {
    let output =
        generate_compatible_source_with_origin("BYTE x PROC Main() x==+1 x==-1 RETURN", 0x3000)
            .unwrap();

    assert_eq!(
        output.bytes,
        vec![
            0x00,
            opcode::JMP_ABS,
            0x04,
            0x30,
            opcode::INC_ABS,
            0x00,
            0x30,
            opcode::SEC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::SBC_IMM,
            0x01,
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::RTS,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_for_step_uses_byte_inc_peephole() {
    let output = generate_compatible_source_with_origin(
        "BYTE i PROC Main() FOR i=1 TO 2 DO OD RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::INC_ABS, 0x00, 0x30])
    );
}

#[test]
fn compatible_negative_for_step_uses_original_add_shape() {
    let output = generate_compatible_source_with_origin(
        "BYTE i PROC Main() FOR i=3 TO 1 STEP -1 DO OD RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x01,
            opcode::CMP_ABS,
            0x00,
            0x30,
            opcode::BCS_REL,
            0x03,
        ]));
    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_IMM,
            0xFF,
            opcode::STA_ABS,
            0x00,
            0x30,
        ]));
}

#[test]
fn compatible_generation_calls_original_multiply_helper_for_constant_product() {
    let output =
        generate_compatible_source_with_origin("CARD w PROC Main() w=12*34 RETURN", 0x3000)
            .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0xA0])
    );
    assert!(
        !output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x98, opcode::STA_ABS, 0x00])
    );
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x0C,
            opcode::LDX_IMM,
            0x00,
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_MUL.low()
        ]));
}

#[test]
fn modern_word_call_argument_preserves_runtime_multiply_high_byte() {
    let output = generate_profile_source_with_origin(
        "BYTE zdx=$5C,zdy=$5D CARD buf CARD FUNC Alloc(CARD n) RETURN(n) \
         PROC Main() buf=Alloc((zdx+1)*(zdy+1)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_MUL.low(),
            runtime_helper::CARTRIDGE_MUL.high(),
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::TXA,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::LDA_ZP,
        ]));
    assert!(
        !output.bytes.windows(6).any(|bytes| bytes
            == [
                opcode::JSR_ABS,
                runtime_helper::CARTRIDGE_MUL.low(),
                runtime_helper::CARTRIDGE_MUL.high(),
                opcode::LDX_IMM,
                0x00,
                opcode::JSR_ABS,
            ]),
        "word call argument must not discard the runtime multiply high byte"
    );
}

#[test]
fn card_store_preserves_runtime_multiply_high_byte_through_byte_subtract() {
    for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
        let output = generate_profile_source_with_origin(
            "BYTE a,b CARD w PROC Main() w=(a+1)*b-1 RETURN",
            0x3000,
            profile,
        )
        .unwrap();

        assert!(output.bytes.windows(3).any(|bytes| bytes
            == [
                opcode::JSR_ABS,
                runtime_helper::CARTRIDGE_MUL.low(),
                runtime_helper::CARTRIDGE_MUL.high()
            ]));
        assert!(
            output
                .bytes
                .windows(4)
                .any(|bytes| bytes == [opcode::TXA, opcode::SBC_IMM, 0x00, opcode::STA_ABS])
                || output.bytes.windows(5).any(|bytes| bytes
                    == [
                        opcode::LDA_ZP,
                        runtime_zp::ARRAY_ADDR.offset(1).address(),
                        opcode::SBC_IMM,
                        0x00,
                        opcode::STA_ABS,
                    ]),
            "CARD high byte should subtract through the runtime multiply high result"
        );
        assert!(
            !output
                .bytes
                .windows(5)
                .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ABS, 0x03, 0x30]),
            "CARD high byte must not be forced to zero"
        );
    }
}

#[test]
fn compatible_generation_calls_original_shift_helpers_for_constant_counts() {
    let output = generate_compatible_source_with_origin(
        "CARD x,w PROC Main() w=x LSH 1 w=x RSH 1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_LSH.low(),
            runtime_helper::CARTRIDGE_LSH.high(),
        ]));
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_RSH.low(),
            runtime_helper::CARTRIDGE_RSH.high(),
        ]));
}

#[test]
fn compatible_generation_can_override_runtime_helper_vector_slots() {
    let output = generate_compatible_source_with_origin(
        "SET $4E8=$4000 CARD w PROC Main() w=12*34 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x40])
    );
    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0xA0])
    );
}

#[test]
fn compatible_generation_can_set_runtime_helper_vector_to_fixed_routine_name() {
    let output = generate_compatible_source_with_origin(
        "SET $4E8=Helper CARD w PROC Main() w=12*34 RETURN PROC Helper=$4000()",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x40])
    );
}

#[test]
fn compatible_generation_can_set_runtime_helper_vector_to_generated_routine_label() {
    let output = generate_compatible_source_with_origin(
        "SET $4E8=Helper CARD w PROC Helper() RETURN PROC Main() w=12*34 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x02, 0x30])
    );
    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0xA0])
    );
}

#[test]
fn compatible_generation_can_set_sargs_vector_to_generated_routine_label() {
    let output = generate_compatible_source_with_origin(
        "BYTE seen SET $4EE=Helper PROC Helper() RETURN PROC Take(BYTE a,b,c) seen=a RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x01, 0x30, 0x05, 0x30, 0x02])
    );
}

#[test]
fn compatible_parameter_prologue_uses_sargs_for_three_argument_bytes() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,c PROC Take(BYTE x,y,z) a=x b=y c=z RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xF5, 0xA0, 0x03, 0x30, 0x02])
    );
}

#[test]
fn compatible_parameter_prologue_keeps_direct_stores_for_two_argument_bytes() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b PROC Take(BYTE x,y) a=x b=y RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xF5, 0xA0])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x02, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STX_ABS, 0x03, 0x30])
    );
}

#[test]
fn compatible_sized_card_array_uses_descriptor_to_unsaved_backing_storage() {
    let output = generate_compatible_source_with_origin(
        "CARD ARRAY ca(4) PROC Main() ca(0)=$1234 RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[2..4], &[0x08, 0x00]);
    let backing = u16::from_le_bytes([output.bytes[0], output.bytes[1]]);
    assert_eq!(usize::from(backing - output.origin), output.bytes.len());
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_local_sized_non_byte_arrays_use_descriptor_backing_storage() {
    let output = generate_compatible_source_with_origin(
            "PROC LocalArrays() BYTE ARRAY bytes(4) CARD ARRAY words(3) INT ARRAY nums(2) RETURN PROC Main() LocalArrays() RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(&output.bytes[2..4], &[0x04, 0x00]);
    assert_eq!(&output.bytes[6..8], &[0x06, 0x00]);
    assert_eq!(&output.bytes[10..12], &[0x04, 0x00]);

    let backing_base = output.origin.wrapping_add(output.bytes.len() as u16);
    let words_backing = u16::from_le_bytes([output.bytes[4], output.bytes[5]]);
    let nums_backing = u16::from_le_bytes([output.bytes[8], output.bytes[9]]);
    assert_eq!(nums_backing, backing_base);
    assert_eq!(words_backing, backing_base.wrapping_add(4));
}

#[test]
fn compatible_local_initialized_non_byte_array_keeps_descriptor_after_backing() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() INT ARRAY value(4)=[0 1 $FFFF 0] RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[0..8], &[0, 0, 1, 0, 0xFF, 0xFF, 0, 0]);
    assert_eq!(&output.bytes[8..12], &[0x00, 0x30, 0, 0]);
}

#[test]
fn compatible_inline_byte_array_dynamic_index_uses_absolute_x() {
    let output = generate_compatible_source_with_origin(
        "BYTE out PROC Main() BYTE i BYTE ARRAY bytes(4) i=1 out=bytes(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDX_ABS, 0x01, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS_X, 0x02, 0x30])
    );
}

#[test]
fn classic_global_inline_byte_array_computed_index_uses_absolute_x() {
    let source = "BYTE ARRAY colors(0)=[$68 $0C $96 $38] BYTE i,j,out PROC Main() out=colors((i+j)&3) RETURN";

    for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
        let output = generate_profile_source_with_origin(source, 0x3000, profile).unwrap();

        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes[0] == opcode::LDA_ABS_X)
        );
        assert!(
            !output
                .bytes
                .windows(2)
                .any(|bytes| { bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()] })
        );
    }
}

#[test]
fn classic_local_inline_byte_array_computed_index_uses_absolute_x() {
    let source = "BYTE i,j,out PROC Main() BYTE ARRAY colors(0)=[$68 $0C $96 $38] out=colors((i+j)&3) RETURN";

    for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
        let output = generate_profile_source_with_origin(source, 0x3000, profile).unwrap();

        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes[0] == opcode::LDA_ABS_X)
        );
        assert!(
            !output
                .bytes
                .windows(2)
                .any(|bytes| { bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()] })
        );
    }
}

#[test]
fn classic_computed_inline_byte_array_assignment_preserves_target_across_rhs() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY quickmul(48) BYTE i CARD h PROC Main() quickmul(i+24)=h rsh 8 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_local_sized_byte_array_absolute_initializer_binds_base_address() {
    let output = generate_compatible_source_with_origin(
        "BYTE out PROC Main() BYTE i BYTE ARRAY ports(4)=$278 i=1 out=ports(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS_X, 0x78, 0x02])
    );
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::LDA_ABS_X,
            output.origin.wrapping_add(2) as u8,
            (output.origin.wrapping_add(2) >> 8) as u8
        ]));
}

#[test]
fn compatible_inline_word_array_byte_shift_index_is_inlined() {
    let output = generate_compatible_source_with_origin(
            "INT out PROC Main() BYTE i BYTE ARRAY ports(4)=$278 INT ARRAY value(4)=[0 1 $FFFF 0] i=1 out=value((ports(i)&$C) RSH 2) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| { bytes == [opcode::LDA_ABS_X, 0x78, 0x02] })
    );
    assert!(
        output.bytes.windows(5).any(|bytes| {
            bytes == [opcode::AND_IMM, 0x0C, opcode::STA_ZP, 0xAE, opcode::LDA_ZP]
        })
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LSR_A, opcode::LSR_A])
    );
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_RSH.low(),
            runtime_helper::CARTRIDGE_RSH.high()
        ]));
}

#[test]
fn compatible_array_pointer_plus_byte_return_carries_high_byte() {
    let output = generate_compatible_source_with_origin(
        "CARD FUNC BasePlus(BYTE ARRAY s BYTE ptr) RETURN(s+ptr)",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(17).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_ABS,
            0x02,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::RTS
        ]));
}

#[test]
fn compatible_pointer_plus_own_byte_deref_uses_direct_carry_chain() {
    let output = generate_compatible_source_with_origin(
        "CARD FUNC BasePlusLen(CHAR POINTER s) RETURN(s+s^+1)",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        count_pair(
            &output.bytes,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address()
        ),
        1
    );
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LDY_IMM,
            0x00,
            opcode::CLC,
            opcode::ADC_IZY,
            runtime_zp::ARRAY_ADDR.address()
        ]));
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::CLC
        ]));
}

#[test]
fn compatible_descriptor_array_expression_index_uses_element_addr() {
    let output = generate_compatible_source_with_origin(
        "BYTE i,b BYTE ARRAY data=[1 2 3] PROC Main() i=1 b=data(i+1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(18).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::CLC,
            opcode::LDA_ABS,
            0x05,
            0x30,
            opcode::ADC_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ABS,
            0x06,
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_initialized_arrays_use_inline_original_layout() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY ba=[1 2] CARD ARRAY ca=[10 20] PROC Main() RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        &output.bytes[..15],
        &[
            1,
            2,
            0x00,
            0x30,
            10,
            0,
            20,
            0,
            0x04,
            0x30,
            opcode::JMP_ABS,
            0x0D,
            0x30,
            opcode::RTS,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_initialized_word_arrays_use_descriptor_storage() {
    let output = generate_compatible_source_with_origin(
        "CARD ARRAY table=[10 20] CARD out PROC Main() out=table(1) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..8], &[10, 0, 20, 0, 0x00, 0x30, 0, 0]);
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JMP_ABS, 0x0B, 0x30,])
    );
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x01,
            opcode::ASL_A,
            opcode::PHP,
            opcode::CLC,
            opcode::ADC_ABS,
        ]));
}

#[test]
fn compatible_unsized_arrays_are_pointer_backed() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY ba BYTE x PROC Main() ba(0)=$11 x=ba(0) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..3], &[0x00, 0x00, 0x00]);
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn modern_pointer_byte_array_dynamic_index_loads_with_effective_y() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY ba BYTE i,x PROC Main() ba=$4000 i=3 x=ba(i) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDY_ABS, 0x02, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(!output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_ABS,
            0x02,
            0x30
        ]));
}

#[test]
fn modern_inline_byte_array_scalar_index_load_uses_proof_guided_absolute_y() {
    let output = generate_profile_source_with_origin(
        "BYTE out PROC Main() BYTE i BYTE ARRAY bytes(4) i=1 out=bytes(i) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS_Y, 0x02, 0x30])
    );
    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS_X, 0x02, 0x30])
    );
}

#[test]
fn modern_pointer_card_array_dynamic_index_prepares_effective_address_once() {
    let output = generate_profile_source_with_origin(
        "CARD ARRAY ca BYTE i CARD x PROC Main() ca=$4000 i=3 x=ca(i) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x02, 0x30, opcode::ASL_A, opcode::TAY])
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        2
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x03, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x04, 0x30])
    );
}

#[test]
fn modern_pointer_card_array_arithmetic_uses_effective_address_source() {
    let output = generate_profile_source_with_origin(
        "CARD POINTER cp CARD out,add BYTE i PROC Main() cp=$4000 i=3 add=$0100 out=cp(i)+add RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::ASL_A, opcode::TAY])
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        2
    );
    assert!(
        !output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::ASL_A, opcode::PHP, opcode::CLC, opcode::ADC_ABS,])
    );
}

#[test]
fn modern_same_record_pointer_byte_fields_reuse_base_pointer() {
    let output = generate_profile_source_with_origin(
        "TYPE Item=[BYTE kind,state] Item first Item POINTER p BYTE out PROC Main() p=first out=p.kind+p.state RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
            .count(),
        0
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::ADC_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn modern_same_record_pointer_word_fields_reuse_base_pointer() {
    let output = generate_profile_source_with_origin(
        "TYPE Item=[CARD lo,hi] Item first Item POINTER p CARD out PROC Main() p=first out=p.lo+p.hi RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
            .count(),
        0
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::ADC_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        2
    );
}

#[test]
fn modern_same_record_pointer_word_bitwise_fields_reuse_base_pointer() {
    let output = generate_profile_source_with_origin(
        "TYPE Item=[CARD mask,bits] Item first Item POINTER p CARD out PROC Main() p=first out=p.mask& p.bits RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
            .count(),
        0
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::AND_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        2
    );
}

#[test]
fn modern_pointer_byte_array_dynamic_index_store_uses_effective_y() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY ba BYTE i,x PROC Main() ba=$4000 i=3 x=$11 ba(i)=x RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDY_ABS, 0x02, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(!output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_ABS,
            0x02,
            0x30
        ]));
}

#[test]
fn modern_pointer_card_array_dynamic_index_store_prepares_effective_address_once() {
    let output = generate_profile_source_with_origin(
        "CARD ARRAY ca BYTE i CARD x PROC Main() ca=$4000 i=3 x=$1234 ca(i)=x RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        2
    );
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::INY
        ]));
}

#[test]
fn modern_same_effective_address_call_assignment_reuses_pointer() {
    let output = generate_profile_source_with_origin(
            "BYTE FUNC Internal(BYTE ch) RETURN(ch+1) BYTE POINTER s BYTE i PROC Main() s=$4000 i=2 s(i)=Internal(s(i)) RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count()
            >= 1
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn modern_byte_index_call_arg_loads_with_effective_y() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER s BYTE i PROC Put(BYTE ch) RETURN PROC Main() s=$4000 i=2 Put(s(i)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDY_ABS, 0x02, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(output.bytes.windows(7).all(|bytes| bytes
        != [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_ABS,
            0x02,
            0x30
        ]));
}

#[test]
fn modern_inline_byte_array_call_arg_uses_proof_guided_absolute_y() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY a(4) BYTE i PROC Put(BYTE ch) RETURN PROC Main() i=2 Put(a(i)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| {
        bytes == [opcode::STA_ABS, 0x04, 0x30, opcode::TAY]
            || bytes == [opcode::LDY_ABS, 0x04, 0x30, opcode::LDA_ABS_Y]
    }));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS_Y, 0x00, 0x30])
    );
    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS_X, 0x00, 0x30])
    );
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::EffectiveAddressLowered
            && optimization.message.contains("absolute,Y")
    }));
}

#[test]
fn modern_inline_byte_array_call_arg_records_rejected_proof_attempt() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY a(4) CARD i PROC Put(BYTE ch) RETURN PROC Main() i=2 Put(a(i)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.proof_attempts.iter().any(|attempt| {
        !attempt.accepted
            && attempt.kind == "index-address"
            && attempt.summary.contains("got mode Unsupported")
    }));
    assert!(
        output
            .proofs
            .iter()
            .all(|proof| proof.kind != "index-address")
    );
}

#[test]
fn modern_word_index_call_arg_loads_with_effective_y() {
    let output = generate_profile_source_with_origin(
            "CARD POINTER p BYTE i PROC Sink(CARD value) RETURN PROC Main() p=$4000 i=2 Sink(p(i)) RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
            .count()
            >= 2
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| { bytes[0] == opcode::LDA_IZY && bytes[2] == opcode::TAX })
    );
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::LDX_ZP
        ]));
}

#[test]
fn compatible_unsized_array_assignment_updates_pointer() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE ARRAY target STRING source(0)=\"ABC\" BYTE len PROC Main() target=source len=target(0) RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(&output.bytes[..6], &[0x00, 0x00, 0x03, b'A', b'B', b'C']);
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x02,
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::LDA_IMM,
            0x30,
            opcode::STA_ABS
        ]));
}

#[test]
fn compatible_unsized_array_string_assignment_emits_new_storage() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY target BYTE len PROC Main() target=\"ONE\" len=target(0) target=\"TWO\" len=target(0) RETURN",
            0x3000,
        )
        .unwrap();

    let one_offset = output
        .bytes
        .windows(4)
        .position(|bytes| bytes == [0x03, b'O', b'N', b'E'])
        .unwrap();
    let two_offset = output
        .bytes
        .windows(4)
        .position(|bytes| bytes == [0x03, b'T', b'W', b'O'])
        .unwrap();
    let one_address = output.origin.wrapping_add(one_offset as u16);
    let two_address = output.origin.wrapping_add(two_offset as u16);

    assert_eq!(output.bytes[one_offset - 3], opcode::JMP_ABS);
    assert_eq!(output.bytes[two_offset - 3], opcode::JMP_ABS);
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            (one_address >> 8) as u8,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::LDA_IMM
        ]));
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            (two_address >> 8) as u8,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::LDA_IMM
        ]));
}

#[test]
fn compatible_local_unsized_array_assignment_uses_pointer_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE x PROC Main() BYTE ARRAY target target=\"OK\" x=target(1) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..3], &[0x00, 0x00, 0x00]);

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'O', b'K'])
        .unwrap();
    let literal_address = output.origin.wrapping_add(literal_offset as u16);

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            (literal_address >> 8) as u8,
            opcode::STA_ABS,
            0x02,
            0x30,
            opcode::LDA_IMM
        ]));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_string_initializers_are_length_prefixed() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" STRING one(0)=\"A\" CHAR ARRAY fixed(6)=\"ATARI!\" BYTE x PROC Main() x=one(1) RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(
        &output.bytes[..10],
        &[0x01, b'A', 0x06, b'A', b'T', b'A', b'R', b'I', b'!', 0x00]
    );
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::STA_ABS, 0x09, 0x30])
    );
}

#[test]
fn compatible_unsized_byte_array_string_initializer_is_inline_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY text=\"HI\" BYTE len PROC Main() len=text(0) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..4], &[0x02, b'H', b'I', 0x00]);
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ABS, 0x03, 0x30])
    );
}

#[test]
fn compatible_unsized_byte_array_address_initializer_is_pointer_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY LBuff=$580 PROC Main() LBuff(0)=1 RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..2], &[0x80, 0x05]);
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ZP,])
    );
}

#[test]
fn compatible_while_exit_does_not_reuse_condition_y_for_following_store() {
    let output = generate_compatible_source_with_origin(
        "BYTE i BYTE POINTER ptr PROC Main() WHILE ptr^='0 DO ptr==+1 OD i=0 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDY_IMM, 0x00, opcode::STY_ABS, 0x00, 0x30])
    );
}

#[test]
fn compatible_sized_byte_array_storage_carries_original_length_words() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY bytes(4) BYTE x PROC Main() bytes(0)=1 x=bytes(0) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..5], &[0x00, 0x00, 0x04, 0x00, 0x00]);
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ABS, 0x04, 0x30])
    );
}

#[test]
fn compatible_global_sized_byte_array_boundary_matches_original_threshold() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY g255(255) PROC MarkG255=*() [$60] MODULE BYTE ARRAY g256(256) PROC MarkG256=*() [$60] MODULE BYTE ARRAY g257(257) PROC MarkG257=*() [$60] MODULE PROC Main() RETURN",
            0x3000,
        )
        .unwrap();

    let g255 = storage_symbol(&output, CodegenSymbolScope::Global, "G255");
    let g256 = storage_symbol(&output, CodegenSymbolScope::Global, "G256");
    let g257 = storage_symbol(&output, CodegenSymbolScope::Global, "G257");
    assert_eq!(g255.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(g255.address, 0x3000);
    assert_eq!(g256.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(g256.address, 0x3100);
    assert_eq!(g257.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(g257.address, 0x3201);
    assert_eq!(routine_address(&output, "MarkG257"), Some(0x3205));
    assert_eq!(output.skipped_ranges.len(), 1);
    assert_eq!(output.skipped_ranges[0].len, 257);
}

#[test]
fn compatible_global_array_sizes_accept_numeric_defines() {
    let output = generate_compatible_source_with_origin(
        "DEFINE max=\"255\" BYTE ARRAY g255(max),g256(max+1),g257(max+2) PROC Main() RETURN",
        0x3000,
    )
    .unwrap();

    let g255 = storage_symbol(&output, CodegenSymbolScope::Global, "G255");
    let g256 = storage_symbol(&output, CodegenSymbolScope::Global, "G256");
    let g257 = storage_symbol(&output, CodegenSymbolScope::Global, "G257");
    assert_eq!(g255.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(g255.address, 0x3000);
    assert_eq!(g256.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(g256.address, 0x30FF);
    assert_eq!(g257.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(g257.address, 0x31FF);
    assert_eq!(output.skipped_ranges.len(), 1);
    assert_eq!(output.skipped_ranges[0].len, 257);
}

#[test]
fn compatible_define_sized_arrays_match_original_gem_layout() {
    let output = generate_compatible_source_with_origin(
            "DEFINE max=\"255\" BYTE rb,bl,ms,wa,ex,gm,nm,gemtaken,numbots,winner,winning,playto,maxbots INT ARRAY xd(2),yd(2),bxd(2),byd(2) CARD ARRAY linept(2) BYTE ARRAY alive(max),expl(max),fire(max) PROC Main() RETURN",
            0x3000,
        )
        .unwrap();

    let xd = storage_symbol(&output, CodegenSymbolScope::Global, "XD");
    let yd = storage_symbol(&output, CodegenSymbolScope::Global, "YD");
    let linept = storage_symbol(&output, CodegenSymbolScope::Global, "LINEPT");
    let alive = storage_symbol(&output, CodegenSymbolScope::Global, "ALIVE");
    let expl = storage_symbol(&output, CodegenSymbolScope::Global, "EXPL");
    let fire = storage_symbol(&output, CodegenSymbolScope::Global, "FIRE");
    assert_eq!(xd.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(xd.address, 0x300D);
    assert_eq!(yd.address, 0x3011);
    assert_eq!(linept.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(linept.address, 0x301D);
    assert_eq!(alive.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(alive.address, 0x3021);
    assert_eq!(expl.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(expl.address, 0x3120);
    assert_eq!(fire.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(fire.address, 0x321F);
}

#[test]
fn compatible_local_sized_byte_array_storage_carries_original_length_low_byte() {
    let output = generate_compatible_source_with_origin(
        "PROC LocalOnly() BYTE ARRAY buf(3) buf(0)=1 RETURN PROC Main() LocalOnly() RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            0x00,
            0x00,
            0x03,
            opcode::JMP_ABS,
            0x06,
            0x30,
            opcode::LDA_IMM
        ]));
}

#[test]
fn compatible_local_sized_byte_array_boundary_matches_original_threshold() {
    let output = generate_compatible_source_with_origin(
            "BYTE sink PROC Local255() BYTE ARRAY l255(255) sink=l255(0) RETURN PROC Local256() BYTE ARRAY l256(256) sink=l256(0) RETURN PROC Local257() BYTE ARRAY l257(257) sink=l257(0) RETURN PROC Main() Local255() Local256() Local257() RETURN",
            0x3000,
        )
        .unwrap();

    let l255 = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("Local255".to_string()),
        "L255",
    );
    let l256 = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("Local256".to_string()),
        "L256",
    );
    let l257 = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("Local257".to_string()),
        "L257",
    );
    assert_eq!(l255.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(l256.array, Some(CodegenArrayStorage::Inline));
    assert_eq!(l257.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(output.skipped_ranges.len(), 1);
    assert_eq!(output.skipped_ranges[0].len, 257);
}

#[test]
fn modern_machine_block_addresses_descriptor_backed_local_array_backing() {
    let output = generate_profile_source_with_origin(
        "PROC LocalOnly() BYTE ARRAY buf(256) [ $99 buf ] RETURN PROC Main() LocalOnly() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let buf = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("LocalOnly".to_string()),
        "BUF",
    );
    assert_eq!(buf.array, Some(CodegenArrayStorage::Descriptor));
    assert_eq!(output.skipped_ranges.len(), 1);
    assert_eq!(output.skipped_ranges[0].len, 256);
    let backing = output.skipped_ranges[0].start;
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ABS_Y,
            Immediate::new(backing).low(),
            Immediate::new(backing).high()
        ]));
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ABS_Y,
            Immediate::new(buf.address).low(),
            Immediate::new(buf.address).high()
        ]));
}

#[test]
fn compatible_large_local_sized_byte_arrays_use_skipped_backing_storage() {
    let output = generate_compatible_source_with_origin(
        "PROC LocalOnly() BYTE ARRAY buf(300) buf(0)=1 RETURN PROC Main() LocalOnly() RETURN",
        0x3000,
    )
    .unwrap();

    let backing = u16::from_le_bytes([output.bytes[0], output.bytes[1]]);
    assert_eq!(&output.bytes[2..4], &[0x2C, 0x01]);
    assert_eq!(
        output.skipped_ranges,
        vec![SkippedRange {
            start: backing,
            len: 300
        }]
    );
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JMP_ABS,
            (output.origin.wrapping_add(7)) as u8,
            (output.origin.wrapping_add(7) >> 8) as u8
        ]));
}

#[test]
fn compatible_local_256_byte_array_is_inline_storage() {
    let output = generate_compatible_source_with_origin(
            "PROC LocalOnly() BYTE ARRAY buf(256) buf(0)=1 buf(255)=2 RETURN PROC Main() LocalOnly() RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.skipped_ranges.is_empty());
    assert_eq!(output.bytes[0], 0x00);
    assert_eq!(output.bytes[255], 0x00);
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x00, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ABS, 0xFF, 0x30, opcode::RTS])
    );
}

#[test]
fn modern_large_local_sized_byte_arrays_use_skipped_backing_storage() {
    let output = generate_profile_source_with_origin(
        "PROC LocalOnly() BYTE ARRAY buf(300) buf(0)=1 RETURN PROC Main() LocalOnly() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let backing = u16::from_le_bytes([output.bytes[0], output.bytes[1]]);
    assert_eq!(&output.bytes[2..4], &[0x2C, 0x01]);
    assert_eq!(
        output.skipped_ranges,
        vec![SkippedRange {
            start: backing,
            len: 300
        }]
    );
    let local_only = routine_address(&output, "LocalOnly").expect("LocalOnly address");
    assert_eq!(local_only, output.origin.wrapping_add(4));
    assert_ne!(
        output.bytes[usize::from(local_only.wrapping_sub(output.origin))],
        opcode::JMP_ABS,
        "descriptor-backed storage must not force an entry trampoline"
    );
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.routine.as_deref() == Some("LocalOnly")
            && optimization.kind == CodegenOptimizationKind::TrampolineElided
            && optimization.bytes_saved == 3
    }));
}

#[test]
fn load_file_omits_uncovered_skipped_array_backing_storage() {
    let output = generate_profile_source_with_origin(
        "PROC LocalOnly() BYTE ARRAY buf(300) buf(0)=1 RETURN PROC Main() LocalOnly() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();
    let load = format_load_file(&output);
    let main_data_end = 6 + output.bytes.len();

    assert_eq!(load[main_data_end..main_data_end + 2], RUNAD.to_le_bytes());
    assert_eq!(load.len(), main_data_end + 6);
}

#[test]
fn compatible_unsized_initialized_word_arrays_use_two_byte_descriptor_cells() {
    let output = generate_compatible_source_with_origin(
        "CARD ARRAY a=[0 1], b=[0 2] PROC Main() RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output.bytes.starts_with(&[
            0x00, 0x00, 0x01, 0x00, 0x00, 0x30, 0x00, 0x00, 0x02, 0x00, 0x06, 0x30,
        ]),
        "unexpected descriptor/backing layout: {:02X?}",
        &output.bytes[..12]
    );
}

#[test]
fn compatible_local_unsized_initialized_word_arrays_keep_initial_pad() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() BYTE x CARD ARRAY a=[0 1], b=[0 2] RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output.bytes.starts_with(&[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x30, 0x00, 0x00, 0x02, 0x00, 0x09,
        ]),
        "unexpected local descriptor/backing layout: {:02X?}",
        &output.bytes[..14]
    );
}

#[test]
fn compatible_descriptor_array_assignment_keeps_distinct_source_and_target_pointers() {
    let output = generate_compatible_source_with_origin(
            "PROC CopyOne() CARD ARRAY left(300), right(300) right(0)=left(0) RETURN PROC Main() CopyOne() RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_record_storage_is_packed_inline() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb CARD gw Pair rec PROC Main() rec.tag=$11 rec.word=$2233 gb=rec.tag gw=rec.word RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(&output.bytes[..6], &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x11, opcode::STA_ABS, 0x03, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x33, opcode::STA_ABS, 0x04, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x22, opcode::STA_ABS, 0x05, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x03, 0x30, opcode::STA_ABS, 0x00, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x04, 0x30, opcode::STA_ABS, 0x01, 0x30])
    );
}

#[test]
fn compatible_local_record_storage_is_packed_inline() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb CARD gw PROC Main() Pair rec rec.tag=$44 rec.word=$5566 gb=rec.tag gw=rec.word RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(&output.bytes[..6], &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(&output.bytes[6..9], &[opcode::JMP_ABS, 0x09, 0x30]);
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x44, opcode::STA_ABS, 0x03, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x04, 0x30, opcode::STA_ABS, 0x01, 0x30])
    );
}

#[test]
fn compatible_record_pointer_field_access_uses_indirect_indexed_y() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb CARD gw Pair rec Pair POINTER rp PROC Main() rp=@rec rp.tag=$11 rp.word=$2233 gb=rp.tag gw=rp.word RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x06,
            0x30,
            opcode::ADC_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address()
        ]));
}

#[test]
fn compatible_routine_local_record_type_field_access() {
    let output = generate_compatible_source_with_origin(
            "PROC Main() TYPE IOCB=[BYTE id,num,cmd,stat CARD badr,padr,blen] IOCB POINTER iptr iptr=$340 iptr.cmd=7 RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_commutative_add_materializes_complex_right_operand() {
    let output = generate_compatible_source_with_origin(
        "BYTE chan CARD iptr PROC Main() iptr=$340+(chan LSH 4) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(10).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::CLC,
        ]));
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_LSH.low(),
            runtime_helper::CARTRIDGE_LSH.high()
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::ADC_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ABS,
            0x01
        ]));
}

#[test]
fn compatible_byte_constant_shift_call_argument_inlines_accumulator_shift() {
    let output = generate_compatible_source_with_origin(
        "BYTE chan PROC CIO=$E456(BYTE areg,xreg) PROC Main() CIO(0,chan LSH 4) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
        ]));
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_LSH.low(),
            runtime_helper::CARTRIDGE_LSH.high()
        ]));
}

#[test]
fn compatible_array_pointer_can_take_function_return_address() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY ptr CARD FUNC Addr() RETURN($340) PROC Main() ptr=Addr() RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x02, 0x30])
    );
}

#[test]
fn compatible_for_bound_subtracts_dynamic_word_array_expression_directly() {
    let output = generate_compatible_source_with_origin(
            "CARD ctr CARD ARRAY sizes=[0 $100 $80] BYTE mode PROC Main() FOR ctr=0 TO sizes(mode)-1 DO ctr==+1 OD RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2] == opcode::SBC_IMM
            && bytes[3] == 0x01
            && bytes[4] == opcode::STA_ABS
    }));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ADDR.address()])
    );
}

#[test]
fn compatible_word_for_bound_caches_dynamic_expression_once() {
    let output = generate_compatible_source_with_origin(
            "CARD ctr CARD ARRAY sizes=[0 $100 $80] BYTE mode PROC Main() FOR ctr=0 TO sizes(mode)-1 DO ctr==+1 OD RETURN",
            0x3000,
        )
        .unwrap();

    let store = output
        .bytes
        .windows(13)
        .position(|bytes| {
            bytes[0] == opcode::SBC_IMM
                && bytes[1] == 0x01
                && bytes[2] == opcode::STA_ABS
                && bytes[5] == opcode::INY
                && bytes[6] == opcode::LDA_IZY
                && bytes[7] == runtime_zp::ARRAY_ADDR.address()
                && bytes[8] == opcode::SBC_IMM
                && bytes[9] == 0x00
                && bytes[10] == opcode::STA_ABS
        })
        .unwrap()
        + 2;
    let low = [output.bytes[store + 1], output.bytes[store + 2]];
    let high = [output.bytes[store + 9], output.bytes[store + 10]];

    assert!(output.bytes.windows(15).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            low[0],
            low[1],
            opcode::CMP_ABS,
            0x00,
            0x30,
            opcode::LDA_ABS,
            high[0],
            high[1],
            opcode::SBC_ABS,
            0x01,
            0x30,
            opcode::BCS_REL,
            0x05,
            opcode::JMP_ABS,
        ]));
}

#[test]
fn modern_profile_centralizes_for_end_cache_in_routine_storage() {
    let output = generate_profile_source_with_origin(
        "CARD ctr CARD ARRAY sizes=[0 $100 $80] BYTE mode PROC Main() FOR ctr=0 TO sizes(mode)-1 DO ctr==+1 OD RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();
    let main_address = routine_address(&output, "Main").unwrap();

    let low = output
        .map
        .source_ranges
        .iter()
        .find(|range| {
            range.kind == CodegenSourceRangeKind::StorageInitializer
                && range.name.as_deref() == Some("modern hidden FOR end cache low")
        })
        .expect("low cache range");
    let high = output
        .map
        .source_ranges
        .iter()
        .find(|range| {
            range.kind == CodegenSourceRangeKind::StorageInitializer
                && range.name.as_deref() == Some("modern hidden FOR end cache high")
        })
        .expect("high cache range");

    assert!(low.start < main_address);
    assert!(high.start < main_address);
    assert_eq!(low.end, low.start + 1);
    assert_eq!(high.end, high.start + 1);
    assert!(output.bytes.windows(4).any(|bytes| {
        bytes
            == [
                opcode::LDA_ABS,
                (low.start & 0x00FF) as u8,
                (low.start >> 8) as u8,
                opcode::CMP_ABS,
            ]
    }));
}

#[test]
fn compatible_call_left_plus_materialized_byte_preserves_right_after_call() {
    let output = generate_compatible_source_with_origin(
        "BYTE i BYTE ARRAY offsets=[0 124] CARD out CARD FUNC Base(BYTE n) RETURN($8500+(n*$100)) PROC Main() i=1 out=Base(i)+offsets(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(15).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            0x08,
            0x30,
            opcode::LDA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
        ]));
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ADDR.address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
        ]));
}

#[test]
fn compatible_runtime_arithmetic_left_plus_materialized_byte_preserves_right_after_helper() {
    let output = generate_compatible_source_with_origin(
        "BYTE i,n,d BYTE ARRAY offsets=[0 124] CARD out PROC Main() i=1 n=250 d=2 out=(n/d)+offsets(i) out=(n MOD d)+offsets(i) RETURN",
        0x3000,
    )
    .unwrap();

    for helper in [runtime_helper::CARTRIDGE_DIV, runtime_helper::CARTRIDGE_MOD] {
        let helper_pos = output
            .bytes
            .windows(3)
            .position(|bytes| bytes == [opcode::JSR_ABS, helper.low(), helper.high()])
            .expect("runtime helper call");
        let materialized_rhs_pos = output.bytes[helper_pos..]
            .windows(2)
            .position(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ADDR.address()])
            .map(|offset| helper_pos + offset)
            .expect("materialized right operand");
        let push_pos = output.bytes[helper_pos..materialized_rhs_pos]
            .iter()
            .position(|byte| *byte == opcode::PHA)
            .map(|offset| helper_pos + offset)
            .expect("left value pushed before materializing right operand");
        let pop_pos = output.bytes[materialized_rhs_pos..]
            .iter()
            .position(|byte| *byte == opcode::PLA)
            .map(|offset| materialized_rhs_pos + offset)
            .expect("left value restored after materializing right operand");

        assert!(helper_pos < push_pos);
        assert!(push_pos < materialized_rhs_pos);
        assert!(materialized_rhs_pos < pop_pos);
    }
}

#[test]
fn compatible_word_add_uses_dynamic_indexed_operand_directly() {
    let output = generate_compatible_source_with_origin(
            "CARD base,out CARD ARRAY waste=[0 768 384] BYTE mode PROC Main() out=base+waste(mode) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_runtime_multiply_loads_dynamic_indexed_word_operand_directly() {
    let output = generate_compatible_source_with_origin(
        "BYTE n,mode CARD out CARD ARRAY sizes=[0 $100 $80] PROC Main() out=n*sizes(mode) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::AFCUR.offset(1).address()
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::AFCUR.address()
        ]));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_byte_for_bound_caches_indexed_expression_once() {
    let output = generate_compatible_source_with_origin(
        "BYTE i CHAR ARRAY s(0)=\"AB\" PROC Main() FOR i=1 TO s(0) DO OD RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::STA_ABS
            && bytes[3] == opcode::LDA_ABS
            && bytes[1..3] == bytes[4..6]
            && bytes[6] == opcode::CMP_ABS
            && bytes[9] == opcode::BCS_REL
    }));
}

#[test]
fn compatible_runtime_shift_allows_dynamic_count_expression() {
    let output = generate_compatible_source_with_origin(
            "BYTE width,ntemp,temp BYTE ARRAY values=[0 1 0 3] PROC Main() temp=values(width) LSH (ntemp LSH 1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::ASL_A,])
    );
    assert_eq!(
        output
            .bytes
            .windows(3)
            .filter(|bytes| {
                *bytes
                    == [
                        opcode::JSR_ABS,
                        runtime_helper::CARTRIDGE_LSH.low(),
                        runtime_helper::CARTRIDGE_LSH.high(),
                    ]
            })
            .count(),
        1
    );
}

#[test]
fn compatible_record_pointer_parameter_field_access() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb Pair rec Pair POINTER gp PROC Touch(Pair POINTER rp) rp.tag=$77 gb=rp.tag RETURN PROC Main() gp=@rec Touch(gp) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::STX_ABS, 0x07, 0x30, opcode::STA_ABS, 0x06, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_record_value_argument_passes_base_address() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb Pair rec PROC Touch(Pair POINTER rp) rp.tag=$77 gb=rp.tag RETURN PROC Main() Touch(rec) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDX_IMM,
            0x30,
            opcode::LDA_IMM,
            0x01,
            opcode::JSR_ABS,
            0x06,
            0x30
        ]));
}

#[test]
fn modern_card_value_argument_to_pointer_param_passes_stored_pointer_value() {
    let output = generate_profile_source_with_origin(
        "BYTE x PROC Draw(BYTE POINTER p) RETURN PROC Main() CARD menu menu=$4321 x=0 Draw(menu) x=1 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let menu = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("Main".to_string()),
        "MENU",
    );
    let menu_high = Absolute::new(menu.address.wrapping_add(1));
    assert!(output.bytes.windows(4).any(|bytes| {
        bytes
            == [
                opcode::LDX_ABS,
                menu_high.low(),
                menu_high.high(),
                opcode::JSR_ABS,
            ]
    }));
    assert!(!output.bytes.windows(4).any(|bytes| {
        bytes
            == [
                opcode::LDX_IMM,
                Absolute::new(menu.address).high(),
                opcode::LDA_IMM,
                Absolute::new(menu.address).low(),
            ]
    }));
}

#[test]
fn compatible_routine_data_address_cast_passes_pointer_value_to_call() {
    let output = generate_compatible_source_with_origin(
        "PROC Menu=*() [\"Yes\" 'Y 0] BYTE seen PROC Take(BYTE POINTER p) seen=p^ RETURN PROC Main() Take(BYTE POINTER(@Menu)) RETURN",
        0x3000,
    )
    .unwrap();
    let menu = routine_address(&output, "Menu").expect("Menu address");
    let take = routine_address(&output, "Take").expect("Take address");

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            (menu >> 8) as u8,
            opcode::TAX,
            opcode::LDA_IMM,
            (menu & 0x00FF) as u8,
            opcode::JSR_ABS,
            (take & 0x00FF) as u8,
            (take >> 8) as u8,
        ]));
}

#[test]
fn compatible_record_value_argument_works_with_staged_calls() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb Pair rec PROC Touch(Pair POINTER rp,BYTE i) rp.tag=i RETURN PROC Main() gb=1 Touch(rec,gb+1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDY_ZP,
            runtime_zp::ARGS.offset(2).address(),
            opcode::LDX_IMM,
            0x30,
            opcode::LDA_IMM,
            0x01,
            opcode::JSR_ABS,
        ]));
}

#[test]
fn compatible_record_pointer_argument_after_byte_parameter_is_accepted() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] Pair rec PROC Touch(BYTE prefix,Pair POINTER rp) rp.tag=prefix RETURN PROC Main() Touch($55,rec) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.contains(&opcode::JSR_ABS));
}

#[test]
fn compatible_record_fields_can_be_call_arguments() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] BYTE gb CARD gw Pair rec PROC Take(BYTE b,CARD w) gb=b gw=w RETURN PROC Main() rec.tag=$12 rec.word=$3456 Take(rec.tag,rec.word) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x03, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDX_ABS, 0x04, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDY_ABS, 0x05, 0x30])
    );
}

#[test]
fn compatible_global_string_constant_indexes_load_and_store_directly() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" STRING text(0)=\"ABCD\" BYTE a PROC Main() text(1)='Z a=text(1) RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(&output.bytes[..6], &[0x04, b'A', b'B', b'C', b'D', 0x00]);
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            b'Z',
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::LDA_ABS
        ]));
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::STA_ABS, 0x05, 0x30])
    );
}

#[test]
fn compatible_local_string_initializers_precede_routine_trampoline() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE g0,g1,g2,g3 PROC LocalStrings() STRING local(0)=\"LOCAL\" CHAR ARRAY raw(3)=\"XYZ\" g0=local(0) g1=local(1) g2=raw(0) g3=raw(2) RETURN PROC Main() LocalStrings() RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(
        &output.bytes[..14],
        &[
            0x00, 0x00, 0x00, 0x00, 0x05, b'L', b'O', b'C', b'A', b'L', 0x03, b'X', b'Y', b'Z'
        ]
    );
    assert_eq!(&output.bytes[14..17], &[opcode::JMP_ABS, 0x11, 0x30]);
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x04, 0x30, opcode::STA_ABS, 0x00, 0x30])
    );
}

#[test]
fn compatible_string_parameters_use_pointer_backed_indexing() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" STRING text(0)=\"ABC\" BYTE a PROC Touch(STRING s, BYTE i) a=s(0) a=s(i) s(1)='Z RETURN PROC Main() Touch(text,2) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_pointer_call_index_and_deref_values() {
    let output = generate_compatible_source_with_origin(
        "BYTE b,k BYTE POINTER ps INT POINTER args PROC Main() b=ps(0) b=ps(k) b=args^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_negated_pointer_deref_call_argument_is_staged() {
    let output = generate_compatible_source_with_origin(
            "INT POINTER args\nCARD out\nCARD FUNC F(INT n)\nRETURN(n)\nPROC Main()\nout=F(-args^)\nRETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARGS.address()])
    );
    assert!(output.bytes.windows(10).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::LDY_IMM,
            0x00,
            opcode::SBC_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0x00,
        ]));
    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::INY,
            opcode::SBC_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_negated_pointer_index_prepares_source_before_zero_subtract() {
    let output = generate_compatible_source_with_origin(
        "INT POINTER args CARD out PROC Main() out=-args(1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(17).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::SEC,
            opcode::LDA_IMM,
            0x00,
            opcode::LDY_IMM,
            0x00,
            opcode::SBC_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ABS,
            0x02,
            0x30,
            opcode::LDA_IMM,
            0x00,
            opcode::INY,
            opcode::SBC_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
        ]));
}

#[test]
fn compatible_named_local_string_argument_passes_base_address() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE x PROC Take(STRING s) x=s(1) RETURN PROC Main() STRING local(0)=\"OK\" Take(local) RETURN",
            0x3000,
        )
        .unwrap();

    let local_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'O', b'K'])
        .unwrap();
    let local_address = output.origin.wrapping_add(local_offset as u16);
    let low = (local_address & 0x00FF) as u8;
    let high = (local_address >> 8) as u8;

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDX_IMM,
            high,
            opcode::LDA_IMM,
            low,
            opcode::JSR_ABS,
            0x03,
            0x30
        ]));
}

#[test]
fn compatible_string_literal_arguments_emit_inline_storage() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE x0,x1 PROC Take(STRING s) x0=s(0) x1=s(1) RETURN PROC Main() Take(\"HI\") RETURN",
            0x3000,
        )
        .unwrap();

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'H', b'I'])
        .unwrap();
    let jump_offset = literal_offset - 3;
    let literal_address = output.origin.wrapping_add(literal_offset as u16);
    let after_literal = literal_address.wrapping_add(3);

    assert_eq!(output.bytes[jump_offset], opcode::JMP_ABS);
    assert_eq!(
        u16::from_le_bytes([output.bytes[jump_offset + 1], output.bytes[jump_offset + 2]]),
        after_literal
    );
    assert_eq!(
        &output.bytes[literal_offset + 3..literal_offset + 10],
        &[
            opcode::LDX_IMM,
            (literal_address >> 8) as u8,
            opcode::LDA_IMM,
            (literal_address & 0x00FF) as u8,
            opcode::JSR_ABS,
            0x04,
            0x30
        ]
    );
}

#[test]
fn compatible_string_literals_preserve_atascii_high_bit_bytes() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE x PROC Take(STRING s) x=s(1) RETURN PROC Main() Take(\"Ôï\") RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0x02, 0xD4, 0xEF])
    );
}

#[test]
fn compatible_string_literal_arguments_work_with_staged_calls() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE one PROC Take(STRING s, BYTE i) one=s(i) RETURN PROC Main() one=1 Take(\"HI\",one+1) RETURN",
            0x3000,
        )
        .unwrap();

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'H', b'I'])
        .unwrap();
    let literal_address = output.origin.wrapping_add(literal_offset as u16);

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            (literal_address & 0x00FF) as u8,
            opcode::STA_ZP,
            runtime_zp::ARGS.address()
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            (literal_address >> 8) as u8,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address()
        ]));
}

#[test]
fn modern_profile_defers_first_string_literal_arg_in_staged_calls() {
    let output = generate_profile_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE one PROC Take(STRING s, BYTE i) one=s(i) RETURN PROC Main() one=1 Take(\"HI\",one+1) RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'H', b'I'])
        .unwrap();
    let literal_address = output.origin.wrapping_add(literal_offset as u16);
    let low = (literal_address & 0x00FF) as u8;
    let high = (literal_address >> 8) as u8;

    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            low,
            opcode::STA_ZP,
            runtime_zp::ARGS.address()
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            high,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address()
        ]));
    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDX_IMM,
            high,
            opcode::LDA_IMM,
            low,
            opcode::JMP_ABS,
            0x04,
            0x30
        ]));
}

#[test]
fn modern_profile_centralizes_string_literals_in_routine_storage() {
    let output = generate_profile_source_with_origin(
        "DEFINE STRING=\"CHAR ARRAY\" BYTE x PROC Take(STRING s) x=s(1) RETURN PROC Main() Take(\"HI\") RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'H', b'I'])
        .unwrap();
    let literal_address = output.origin.wrapping_add(literal_offset as u16);
    let main_address = routine_address(&output, "Main").unwrap();
    let main_offset = usize::from(main_address.wrapping_sub(output.origin));

    assert!(literal_address < main_address);
    assert_ne!(output.bytes[main_offset], opcode::JMP_ABS);
    assert_ne!(
        output.bytes.get(literal_offset.saturating_sub(3)),
        Some(&opcode::JMP_ABS)
    );
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::StorageInitializer
            && range.name.as_deref() == Some("modern hidden string literal")
            && range.start == literal_address
            && range.end == literal_address + 3
    }));
}

#[test]
fn modern_profile_pools_string_literals_in_current_location_routines() {
    let output = generate_profile_source_with_origin(
        "DEFINE STRING=\"CHAR ARRAY\" BYTE x PROC Take(STRING s) x=s(1) RETURN PROC Main=*() Take(\"HI\") RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'H', b'I'])
        .unwrap();
    let literal_address = output.origin.wrapping_add(literal_offset as u16);
    let main_address = routine_address(&output, "Main").unwrap();
    let main_offset = usize::from(main_address.wrapping_sub(output.origin));

    assert_eq!(literal_address, main_address.wrapping_add(3));
    assert_eq!(output.bytes[main_offset], opcode::JMP_ABS);
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::StorageInitializer
            && range.name.as_deref() == Some("modern hidden string literal")
            && range.start == literal_address
            && range.end == literal_address + 3
    }));
}

#[test]
fn modern_profile_pools_distinct_current_location_string_literals() {
    let output = generate_profile_source_with_origin(
        "DEFINE STRING=\"CHAR ARRAY\" BYTE x PROC Take(STRING s) x=s(1) RETURN PROC Main=*() Take(\"HI\") Take(\"BY\") RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let hi_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'H', b'I'])
        .unwrap();
    let by_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'B', b'Y'])
        .unwrap();
    let hi_address = output.origin.wrapping_add(hi_offset as u16);
    let by_address = output.origin.wrapping_add(by_offset as u16);
    let main_address = routine_address(&output, "Main").unwrap();
    let main_offset = usize::from(main_address.wrapping_sub(output.origin));

    assert_ne!(hi_address, by_address);
    assert_eq!(hi_address, main_address.wrapping_add(3));
    assert!(by_address > main_address);
    assert_eq!(output.bytes[main_offset], opcode::JMP_ABS);
    assert_eq!(
        output.bytes[usize::from(hi_address.wrapping_sub(output.origin).wrapping_sub(3))],
        opcode::JMP_ABS
    );
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::StorageInitializer
            && range.name.as_deref() == Some("modern hidden string literal")
            && range.start == by_address
            && range.end == by_address + 3
    }));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDX_IMM,
            (hi_address >> 8) as u8,
            opcode::LDA_IMM,
            (hi_address & 0x00FF) as u8
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDX_IMM,
            (by_address >> 8) as u8,
            opcode::LDA_IMM,
            (by_address & 0x00FF) as u8
        ]));
}

#[test]
fn modern_profile_keeps_current_location_machine_data_unpooled() {
    let output = generate_profile_source_with_origin(
        "PROC Data=*() [\"R\" 'R 0] PROC Main() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let data_address = routine_address(&output, "Data").unwrap();
    let data_offset = usize::from(data_address.wrapping_sub(output.origin));

    assert_eq!(output.bytes[data_offset], 1);
    assert_ne!(output.bytes[data_offset], opcode::JMP_ABS);
}

#[test]
fn modern_profile_stages_array_name_as_card_argument_address() {
    let output = generate_profile_source_with_origin(
            "BYTE ARRAY fname(15) BYTE FUNC One() RETURN(1) PROC Xio(BYTE chn, cmd, aux1, aux2 CARD s) RETURN PROC Main() Xio(One(),0,0,0,fname) RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();
    assert!(!output.bytes.is_empty());
}

#[test]
fn compatible_array_parameters_are_passed_as_base_pointers() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY ba(4) CARD ARRAY ca(4) BYTE x CARD w PROC Touch(BYTE ARRAY bp, CARD ARRAY cp, BYTE i) bp(i)=$22 x=bp(i) cp(i)=$3344 w=cp(i) RETURN PROC Main() Touch(ba,ca,1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xF5, 0xA0, 0x0B, 0x30, 0x04])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xA3])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x10, 0x30])
    );
}

#[test]
fn compatible_indexed_card_plus_constant_stages_without_temp_copy() {
    let output = generate_compatible_source_with_origin(
            "CARD ARRAY v, table=[$3100 $3200] BYTE out BYTE FUNC Value(CARD p) RETURN(0) BYTE FUNC Probe(BYTE i) RETURN(Value(v(i)+1)) PROC Main() v=table out=Probe(1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(18).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::INY,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::LDX_ZP,
            runtime_zp::ARGS.offset(1).address(),
        ]));
}

#[test]
fn compatible_generation_places_routine_storage_before_trampoline() {
    let output = generate_compatible_source_with_origin(
            "BYTE g,r BYTE FUNC Inner(BYTE x) RETURN(x+1) BYTE FUNC Outer(BYTE a,b) BYTE t t=Inner(a) RETURN(t+b) PROC Main() g=Inner(1) r=Outer(g,3) RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(output.run_address, 0x3033);
    assert_eq!(
        output.bytes,
        vec![
            0x00,
            0x00, // globals
            0x00, // Inner.x
            opcode::JMP_ABS,
            0x06,
            0x30,
            opcode::STA_ABS,
            0x02,
            0x30,
            opcode::CLC,
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::ADC_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::RTS,
            0x00,
            0x00,
            0x00, // Outer.a, Outer.b, Outer.t
            opcode::JMP_ABS,
            0x18,
            0x30,
            opcode::STX_ABS,
            0x13,
            0x30,
            opcode::STA_ABS,
            0x12,
            0x30,
            opcode::LDA_ABS,
            0x12,
            0x30,
            opcode::JSR_ABS,
            0x03,
            0x30,
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ABS,
            0x14,
            0x30,
            opcode::CLC,
            opcode::LDA_ABS,
            0x14,
            0x30,
            opcode::ADC_ABS,
            0x13,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::RTS,
            opcode::JMP_ABS,
            0x36,
            0x30,
            opcode::LDA_IMM,
            0x01,
            opcode::JSR_ABS,
            0x03,
            0x30,
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::LDX_IMM,
            0x03,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::JSR_ABS,
            0x15,
            0x30,
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::RTS,
            opcode::RTS,
        ]
    );
}

#[test]
fn emitter_outputs_immediate_addressing_helpers() {
    let mut emitter = Emitter::new();
    let immediate = Immediate::new(0x1234);
    emitter.emit_lda_immediate(immediate, 0);
    emitter.emit_adc_immediate(immediate, 1);
    emitter.emit_sbc_immediate(immediate, 2);

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0xA9, 0x34, 0x69, 0x12, 0xE9, 0x00]
    );
}

#[test]
fn emitter_outputs_absolute_addressing_helpers() {
    let mut emitter = Emitter::new();
    let absolute = Absolute::new(0x1234);
    emitter.emit_lda_absolute(absolute);
    emitter.emit_adc_absolute(absolute.offset(1));
    emitter.emit_sbc_absolute(absolute.offset(2));
    emitter.emit_ldx_absolute(absolute.offset(3));
    emitter.emit_ldy_absolute(absolute.offset(4));
    emitter.emit_sta_absolute(absolute.offset(5));
    emitter.emit_inc_absolute(absolute.offset(6));
    emitter.emit_dec_absolute(absolute.offset(7));

    assert_eq!(
        emitter.finish().unwrap(),
        vec![
            0xAD, 0x34, 0x12, 0x6D, 0x35, 0x12, 0xED, 0x36, 0x12, 0xAE, 0x37, 0x12, 0xAC, 0x38,
            0x12, 0x8D, 0x39, 0x12, 0xEE, 0x3A, 0x12, 0xCE, 0x3B, 0x12,
        ]
    );
}

#[test]
fn emitter_outputs_zero_page_addressing_helpers() {
    let mut emitter = Emitter::new();
    let zero_page = runtime_zp::AFCUR;
    emitter.emit_lda_zero_page(zero_page);
    emitter.emit_adc_zero_page(zero_page.offset(1));
    emitter.emit_sbc_zero_page(runtime_zp::ARG0);
    emitter.emit_sta_zero_page(runtime_zp::TOKEN);
    emitter.emit_sty_zero_page(runtime_zp::DEVICE);

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0xA5, 0x84, 0x65, 0x85, 0xE5, 0xA0, 0x85, 0xC2, 0x84, 0xB7,]
    );
}

#[test]
fn emitter_outputs_indexed_addressing_helpers() {
    let mut emitter = Emitter::new();
    emitter.emit_lda_absolute_x(AbsoluteX::new(0x1234));
    emitter.emit_sta_absolute_x(AbsoluteX::new(0x1235));
    emitter.emit_lda_zero_page_x(ZeroPageX::new(0x80));
    emitter.emit_sta_zero_page_x(ZeroPageX::new(0x81));

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0xBD, 0x34, 0x12, 0x9D, 0x35, 0x12, 0xB5, 0x80, 0x95, 0x81,]
    );
}

#[test]
fn emitter_outputs_indirect_indexed_addressing_helpers() {
    let mut emitter = Emitter::new();
    let izx = IndexedIndirectX::new(runtime_zp::AFCUR);
    let izy = IndirectIndexedY::new(runtime_zp::ARG0);
    emitter.emit_lda_indexed_indirect_x(izx);
    emitter.emit_sta_indexed_indirect_x(izx);
    emitter.emit_adc_indexed_indirect_x(izx);
    emitter.emit_sbc_indexed_indirect_x(izx);
    emitter.emit_lda_indirect_indexed_y(izy);
    emitter.emit_sta_indirect_indexed_y(izy);
    emitter.emit_adc_indirect_indexed_y(izy);
    emitter.emit_sbc_indirect_indexed_y(izy);
    emitter.emit_eor_indirect_indexed_y(izy);
    emitter.emit_cmp_indirect_indexed_y(izy);

    assert_eq!(
        emitter.finish().unwrap(),
        vec![
            0xA1, 0x84, 0x81, 0x84, 0x61, 0x84, 0xE1, 0x84, 0xB1, 0xA0, 0x91, 0xA0, 0x71, 0xA0,
            0xF1, 0xA0, 0x51, 0xA0, 0xD1, 0xA0,
        ]
    );
}

#[test]
fn emitter_outputs_jsr_helpers() {
    let mut emitter = Emitter::new();
    emitter.emit_jsr_absolute(Absolute::new(0x1234));
    emitter.emit_jsr_label("target", Span::new(0, 0));
    emitter.bind_label("target", Span::new(0, 0)).unwrap();
    emitter.emit_rts();

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0x20, 0x34, 0x12, 0x20, 0x06, 0x30, opcode::RTS]
    );
}

#[test]
fn emitter_outputs_logic_and_shift_helpers() {
    let mut emitter = Emitter::new();
    emitter.emit_and_immediate(Immediate::new(0x1234), 0);
    emitter.emit_ora_immediate(Immediate::new(0x1234), 1);
    emitter.emit_eor_absolute(Absolute::new(0x0600));
    emitter.emit_asl_a();
    emitter.emit_lsr_a();
    emitter.emit_rol_a();
    emitter.emit_ror_a();

    assert_eq!(
        emitter.finish().unwrap(),
        vec![
            0x29, 0x34, 0x09, 0x12, 0x4D, 0x00, 0x06, 0x0A, 0x4A, 0x2A, 0x6A,
        ]
    );
}

#[test]
fn emitter_patches_absolute_labels() {
    let mut emitter = Emitter::new();
    emitter.emit_jmp_label("done", Span::new(0, 0));
    emitter.emit_lda_imm(1);
    emitter.bind_label("done", Span::new(0, 0)).unwrap();
    emitter.emit_rts();

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0x4C, 0x05, 0x30, 0xA9, 0x01, 0x60]
    );
}

#[test]
fn emitter_patches_relative_labels() {
    let mut emitter = Emitter::new();
    emitter.emit_branch_label(opcode::BNE_REL, "done", Span::new(0, 0));
    emitter.emit_lda_imm(1);
    emitter.bind_label("done", Span::new(0, 0)).unwrap();
    emitter.emit_rts();

    assert_eq!(
        emitter.finish().unwrap(),
        vec![0xD0, 0x02, 0xA9, 0x01, 0x60]
    );
}

#[test]
fn generates_empty_proc_as_rts() {
    let output = generate_source("PROC Main()").unwrap();
    assert_eq!(output.bytes, vec![opcode::RTS]);
}

#[test]
fn compatible_bodyless_proc_falls_through_to_next_routine() {
    let output = generate_compatible_source_with_origin(
            "BYTE seen=$A0 PROC Empty() PROC Mark=*() [$A9 $55 $85 seen $60] PROC Main() Empty() RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(
        &output.bytes[..9],
        &[
            opcode::JMP_ABS,
            0x03,
            0x30,
            opcode::LDA_IMM,
            0x55,
            opcode::STA_ZP,
            0xA0,
            opcode::RTS,
            opcode::JMP_ABS,
        ]
    );
}

#[test]
fn compatible_final_implicit_return_uses_program_sentinel_rts() {
    let output = generate_compatible_source_with_origin("BYTE x PROC Main() x=1", 0x3000).unwrap();

    assert!(output.bytes.ends_with(&[
        opcode::LDY_IMM,
        0x01,
        opcode::STY_ABS,
        0x00,
        0x30,
        opcode::RTS,
    ]));
    assert!(!output.bytes.ends_with(&[opcode::RTS, opcode::RTS]));
}

#[test]
fn generates_return_only_proc_as_rts() {
    let output = generate_source("PROC Main() RETURN").unwrap();
    assert_eq!(output.bytes, vec![opcode::RTS]);
}

#[test]
fn generates_machine_block_bytes() {
    let output = generate_source("PROC Main() [$00] RETURN").unwrap();
    assert_eq!(output.bytes, vec![0x00, opcode::RTS]);
}

#[test]
fn machine_block_tail_does_not_get_implicit_rts() {
    let output = generate_source("PROC Main() [$A9 $01 $60]").unwrap();
    assert_eq!(output.bytes, vec![opcode::LDA_IMM, 0x01, opcode::RTS]);
}

#[test]
fn machine_block_emits_named_variable_address() {
    let output = generate_source("CARD TempVec PROC Main() [$6C TempVec] RETURN").unwrap();
    assert_eq!(output.bytes, vec![0x6C, 0x00, 0x06, opcode::RTS]);
}

#[test]
fn machine_block_emits_caret_symbol_address() {
    let output = generate_source(
        "BYTE ARRAY screen=$8010,text=$9E80 PROC DL15=*() [78 screen^ 66 text^ 65 DL15]",
    )
    .unwrap();
    let routine = output
        .routine_addresses
        .iter()
        .find(|routine| routine.name == "DL15")
        .expect("expected DL15 routine address");
    let expected = [
        0x4E,
        0x10,
        0x80,
        0x42,
        0x80,
        0x9E,
        0x41,
        (routine.address & 0x00FF) as u8,
        (routine.address >> 8) as u8,
    ];

    assert_eq!(output.bytes, expected);
}

#[test]
fn machine_block_emits_caret_symbol_address_with_named_offset() {
    let output = generate_source(
        "DEFINE OFF=\"2\" BYTE ARRAY screen=$8010 PROC Main() [screen^+OFF >screen^-OFF] RETURN",
    )
    .unwrap();

    assert_eq!(output.bytes, vec![0x12, 0x80, 0x80, opcode::RTS],);
}

#[test]
fn machine_block_splits_compact_opcode_and_storage_symbol() {
    let output = generate_profile_source_with_origin(
        "BYTE device=$B7 PROC Main() [$A5device] RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(output.bytes, vec![opcode::LDA_ZP, 0xB7, opcode::RTS]);
}

#[test]
fn machine_block_emits_already_bound_routine_label_offset() {
    let output = generate_profile_source_with_origin(
        "PROC Target=*() [$60] PROC Main() [$4CTarget+1] RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();
    let target = routine_address(&output, "Target").expect("Target address");
    let expected = [
        opcode::JMP_ABS,
        target.wrapping_add(1) as u8,
        (target.wrapping_add(1) >> 8) as u8,
    ];

    assert!(
        output
            .bytes
            .windows(expected.len())
            .any(|bytes| bytes == expected)
    );
}

#[test]
fn machine_block_emits_low_high_numeric_define_bytes() {
    let output = generate_source("DEFINE ADDR=\"$1234\" PROC Main() [<ADDR >ADDR] RETURN").unwrap();
    assert_eq!(output.bytes, vec![0x34, 0x12, opcode::RTS]);
}

#[test]
fn machine_block_emits_low_high_routine_label_bytes() {
    let output =
        generate_source("PROC Target() RETURN PROC Main() [<Target >Target] RETURN").unwrap();
    let target = output
        .routine_addresses
        .iter()
        .find(|routine| routine.name == "Target")
        .expect("expected Target routine address");
    let expected = [
        target.address as u8,
        (target.address >> 8) as u8,
        opcode::RTS,
    ];

    assert!(
        output
            .bytes
            .windows(expected.len())
            .any(|bytes| bytes == expected)
    );
}

#[test]
fn compatible_machine_block_emits_current_location_jump_table_bytes() {
    let output = generate_compatible_source_with_origin(
        "PROC View() RETURN PROC Copy() RETURN PROC Jmp=*() [<View >View <Copy >Copy] PROC Main() RETURN",
        0x3000,
    )
    .unwrap();
    let view = routine_address(&output, "View").expect("View address");
    let copy = routine_address(&output, "Copy").expect("Copy address");
    let expected = [
        (view & 0x00FF) as u8,
        (view >> 8) as u8,
        (copy & 0x00FF) as u8,
        (copy >> 8) as u8,
    ];

    assert!(
        output
            .bytes
            .windows(expected.len())
            .any(|bytes| bytes == expected)
    );
}

#[test]
fn compatible_machine_block_indirect_jump_can_use_proc_pointer_storage() {
    let output = generate_compatible_source_with_origin(
        "PROC POINTER ErrorHandler PROC Target() RETURN PROC Main() ErrorHandler=@Target [$6C ErrorHandler]",
        0x3000,
    )
    .unwrap();
    let handler_jump = output
        .bytes
        .windows(3)
        .find(|bytes| bytes[0] == opcode::JMP_IND)
        .expect("indirect jump through ErrorHandler");
    let handler_address = u16::from_le_bytes([handler_jump[1], handler_jump[2]]);

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ABS,
            ((handler_address + 1) & 0x00FF) as u8,
            ((handler_address + 1) >> 8) as u8,
        ]));
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ABS,
            (handler_address & 0x00FF) as u8,
            (handler_address >> 8) as u8,
        ]));
}

#[test]
fn local_machine_define_expands_as_statement() {
    let output = generate_source("PROC Main() DEFINE Save=\"[$A9 1]\" Save RETURN").unwrap();
    assert_eq!(output.bytes, vec![opcode::LDA_IMM, 0x01, opcode::RTS]);
}

#[test]
fn local_scalar_machine_define_expands_inside_machine_block() {
    let output =
        generate_source("PROC Main() DEFINE STX=\"$86\" BYTE sp=$A2 [ STX sp ] RETURN").unwrap();
    assert_eq!(output.bytes, vec![opcode::STX_ZP, 0x00, opcode::RTS]);
}

#[test]
fn machine_block_emits_named_variable_address_with_offset() {
    let output = generate_source("CARD ptr PROC Main() [$AD ptr+1] RETURN").unwrap();
    assert_eq!(output.bytes, vec![opcode::LDA_ABS, 0x01, 0x06, opcode::RTS]);
}

#[test]
fn semir_native_machine_block_wraps_ffff_symbol_offset() {
    let output = generate_semir_native_source_with_origin(
        "PROC Main() CHAR ARRAY fnam(40) [$99 fnam+$FFFF] RETURN",
        0x3000,
        CodegenProfile::Compat,
    )
    .unwrap();
    let fnam = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("Main".to_string()),
        "fnam",
    );
    let expected = fnam.address.wrapping_sub(1);

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ABS_Y,
            (expected & 0x00FF) as u8,
            (expected >> 8) as u8,
        ]));
}

#[test]
fn compatible_machine_block_uses_opcode_width_for_symbol_operands() {
    let output = generate_compatible_source_with_origin(
        "CARD r=$86 PROC Main() [$A5 r+1 $9D $344 $20 $56 $E4] RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| {
        bytes
            == [
                opcode::LDA_ZP,
                0x87,
                opcode::STA_ABS_X,
                0x44,
                0x03,
                opcode::JSR_ABS,
                0x56,
                0xE4,
            ]
    }));
}

#[test]
fn compatible_fixed_zero_page_alias_uses_zero_page_stores() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() BYTE ARRAY buf(4) CARD r=$86 r=buf RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x30, opcode::STA_ZP, 0x87])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ZP, 0x86])
    );
    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x86, 0x00])
    );
}

#[test]
fn compatible_stack_card_args_stage_high_byte_first() {
    let output = generate_compatible_source_with_origin(
        "CARD r PROC Take(BYTE a,b,c,d CARD w) RETURN PROC Main() Take(1,2,3,4,r) RETURN",
        0x3000,
    )
    .unwrap();
    let high = output
        .bytes
        .windows(5)
        .position(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::STA_ZP, 0xA5])
        .unwrap();
    let low = output
        .bytes
        .windows(5)
        .position(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ZP, 0xA4])
        .unwrap();

    assert!(high < low);
}

#[test]
fn compatible_pre_code_card_pointer_defers_to_output_origin() {
    let output = generate_compatible_source_with_origin(
        "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 \
             BYTE POINTER screen BYTE scl=screen, sch=screen+1 CARD POINTER allocp \
             SET $491=$3000 SET $E=$3000 PROC Main=*() allocp==+2 [$60]",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        &output.bytes[..4],
        &[0x00, 0x00, opcode::CLC, opcode::LDA_ZP]
    );
    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            0xE8,
            opcode::ADC_IMM,
            0x02,
            opcode::STA_ZP,
            0xE8,
            opcode::LDA_ZP,
            0xE9,
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            0xE9,
            opcode::RTS,
        ]));
    assert_eq!(output.run_address, 0x3002);
}

#[test]
fn generates_byte_constant_assignment() {
    let output = generate_source("BYTE x PROC Main() x=1 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![0xA9, 0x01, 0x8D, 0x00, 0x06, opcode::RTS]
    );
}

#[test]
fn generates_card_constant_assignment_little_endian() {
    let output = generate_source("CARD x PROC Main() x=$1234 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x34,
            0x8D,
            0x00,
            0x06,
            0xA9,
            0x12,
            0x8D,
            0x01,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn lays_out_multiple_scalar_globals_deterministically() {
    let output = generate_source("BYTE a CARD b PROC Main() a=1 b=2 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x01,
            0x8D,
            0x00,
            0x06,
            0xA9,
            0x02,
            0x8D,
            0x01,
            0x06,
            0xA9,
            0x00,
            0x8D,
            0x02,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_variable_copy() {
    let output = generate_source("BYTE y BYTE x PROC Main() x=y RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![0xAD, 0x00, 0x06, 0x8D, 0x01, 0x06, opcode::RTS]
    );
}

#[test]
fn generates_card_variable_copy() {
    let output = generate_source("CARD y CARD x PROC Main() x=y RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x01,
            0x06,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_addition_assignment() {
    let output = generate_source("BYTE y BYTE x PROC Main() x=y+1 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x01,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_compound_add_assignment() {
    let output = generate_source("BYTE x PROC Main() x==+1 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x00,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_card_addition_assignment_with_carry() {
    let output = generate_source("CARD y CARD x PROC Main() x=y+$0102 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x18,
            0x69,
            0x02,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x01,
            0x06,
            0x69,
            0x01,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_card_subtraction_assignment_with_borrow() {
    let output = generate_source("CARD y CARD x PROC Main() x=y-$0001 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x38,
            0xE9,
            0x01,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x01,
            0x06,
            0xE9,
            0x00,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_card_compound_subtract_assignment() {
    let output = generate_source("CARD x PROC Main() x==-$0001 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x38,
            0xE9,
            0x01,
            0x8D,
            0x00,
            0x06,
            0xAD,
            0x01,
            0x06,
            0xE9,
            0x00,
            0x8D,
            0x01,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_and_assignment() {
    let output = generate_source("BYTE y BYTE x PROC Main() x=y&$0F RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![0xAD, 0x00, 0x06, 0x29, 0x0F, 0x8D, 0x01, 0x06, opcode::RTS]
    );
}

#[test]
fn generates_byte_compound_xor_assignment() {
    let output = generate_source("BYTE x PROC Main() x==!128 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![0xAD, 0x00, 0x06, 0x49, 0x80, 0x8D, 0x00, 0x06, opcode::RTS]
    );
}

#[test]
fn generates_card_or_assignment() {
    let output = generate_source("CARD y CARD x PROC Main() x=y%$0102 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x09,
            0x02,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x01,
            0x06,
            0x09,
            0x01,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_xor_assignment() {
    let output = generate_source("BYTE y BYTE z BYTE x PROC Main() x=y!z RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x4D,
            0x01,
            0x06,
            0x8D,
            0x02,
            0x06,
            opcode::RTS
        ]
    );
}

#[test]
fn folds_constant_multiply_divide_and_mod_assignment() {
    let output =
        generate_source("CARD x BYTE y BYTE z PROC Main() x=6*7 y=20/3 z=20 MOD 3 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x2A,
            0x8D,
            0x00,
            0x06,
            0xA9,
            0x00,
            0x8D,
            0x01,
            0x06,
            0xA9,
            0x06,
            0x8D,
            0x02,
            0x06,
            0xA9,
            0x02,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_runtime_multiply_assignment() {
    let output = generate_source("INT a,b,x PROC Main() x=a*b RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x03,
            0x06,
            0x85,
            0x85,
            0xAD,
            0x02,
            0x06,
            0x85,
            0x84,
            0xAD,
            0x01,
            0x06,
            0xAA,
            0xAD,
            0x00,
            0x06,
            0x20,
            0xE8,
            0x04,
            0x8D,
            0x04,
            0x06,
            0x8A,
            0x8D,
            0x05,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_runtime_compound_multiply_assignment() {
    let output = generate_source("INT b,x PROC Main() x==*b RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x01,
            0x06,
            0x85,
            0x85,
            0xAD,
            0x00,
            0x06,
            0x85,
            0x84,
            0xAD,
            0x03,
            0x06,
            0xAA,
            0xAD,
            0x02,
            0x06,
            0x20,
            0xE8,
            0x04,
            0x8D,
            0x02,
            0x06,
            0x8A,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_runtime_divide_and_mod_assignment() {
    let div = generate_source("INT a,b,x PROC Main() x=a/b RETURN").unwrap();
    assert_eq!(
        div.bytes,
        vec![
            0xAD,
            0x03,
            0x06,
            0x85,
            0x85,
            0xAD,
            0x02,
            0x06,
            0x85,
            0x84,
            0xAD,
            0x01,
            0x06,
            0xAA,
            0xAD,
            0x00,
            0x06,
            0x20,
            0xEA,
            0x04,
            0x8D,
            0x04,
            0x06,
            0x8A,
            0x8D,
            0x05,
            0x06,
            opcode::RTS,
        ]
    );

    let rem = generate_source("INT a,b,x PROC Main() x=a MOD b RETURN").unwrap();
    assert_eq!(
        rem.bytes,
        vec![
            0xAD,
            0x03,
            0x06,
            0x85,
            0x85,
            0xAD,
            0x02,
            0x06,
            0x85,
            0x84,
            0xAD,
            0x01,
            0x06,
            0xAA,
            0xAD,
            0x00,
            0x06,
            0x20,
            0xEC,
            0x04,
            0x8D,
            0x04,
            0x06,
            0x8A,
            0x8D,
            0x05,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_runtime_shift_assignment() {
    let lsh = generate_source("CARD a BYTE b CARD x PROC Main() x=a LSH b RETURN").unwrap();
    assert_eq!(
        lsh.bytes,
        vec![
            0xAD,
            0x02,
            0x06,
            0x85,
            0x84,
            0xAD,
            0x01,
            0x06,
            0xAA,
            0xAD,
            0x00,
            0x06,
            0x20,
            0xE4,
            0x04,
            0x8D,
            0x03,
            0x06,
            0x8A,
            0x8D,
            0x04,
            0x06,
            opcode::RTS,
        ]
    );

    let rsh = generate_source("CARD a BYTE b CARD x PROC Main() x=a RSH b RETURN").unwrap();
    assert_eq!(
        rsh.bytes,
        vec![
            0xAD,
            0x02,
            0x06,
            0x85,
            0x84,
            0xAD,
            0x01,
            0x06,
            0xAA,
            0xAD,
            0x00,
            0x06,
            0x20,
            0xE6,
            0x04,
            0x8D,
            0x03,
            0x06,
            0x8A,
            0x8D,
            0x04,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_runtime_byte_shift_loads_direct_lvalue_after_count() {
    let output = generate_compatible_source_with_origin(
        "BYTE n,c,out BYTE ARRAY pmtop(8)=$D008 PROC Main() out=(pmtop(n) RSH c)&1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDX_ABS,
            0x00,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::AFCUR.address(),
            opcode::LDA_ABS_X,
            0x08,
            0xD0,
            opcode::LDX_IMM,
            0x00,
        ]));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn generates_byte_left_shift_assignment() {
    let output = generate_source("BYTE y BYTE x PROC Main() x=y LSH 2 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![0xAD, 0x00, 0x06, 0x0A, 0x0A, 0x8D, 0x01, 0x06, opcode::RTS]
    );
}

#[test]
fn generates_card_left_shift_assignment() {
    let output = generate_source("CARD y CARD x PROC Main() x=y LSH 1 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x01,
            0x06,
            0x8D,
            0x03,
            0x06,
            0xAD,
            0x02,
            0x06,
            0x0A,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x03,
            0x06,
            0x2A,
            0x8D,
            0x03,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_card_right_shift_assignment() {
    let output = generate_source("CARD y CARD x PROC Main() x=y RSH 1 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x01,
            0x06,
            0x8D,
            0x03,
            0x06,
            0xAD,
            0x03,
            0x06,
            0x4A,
            0x8D,
            0x03,
            0x06,
            0xAD,
            0x02,
            0x06,
            0x6A,
            0x8D,
            0x02,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn folds_shift_past_scalar_width_to_zero() {
    let output = generate_source("BYTE x PROC Main() x=1 LSH 16 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![0xA9, 0x00, 0x8D, 0x00, 0x06, opcode::RTS]
    );
}

#[test]
fn generates_if_else_control_flow() {
    let output = generate_source("BYTE x PROC Main() IF x THEN x=1 ELSE x=2 FI RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0xD0,
            0x03,
            0x4C,
            0x10,
            0x30,
            0xA9,
            0x01,
            0x8D,
            0x00,
            0x06,
            0x4C,
            0x15,
            0x30,
            0xA9,
            0x02,
            0x8D,
            0x00,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_elseif_array_index_condition_is_accepted() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY data(4) BYTE i,x PROC Main() IF 0 THEN x=1 ELSEIF data(i)=$07 THEN x=2 ELSE x=3 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_ABS_X, 0x00])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::EOR_IMM, 0x07])
    );
}

#[test]
fn compatible_elseif_pointer_deref_condition_is_accepted() {
    let output = generate_compatible_source_with_origin(
            "BYTE cell,x BYTE POINTER p PROC Main() p=@cell IF 0 THEN x=1 ELSEIF p^=$07 THEN x=2 ELSE x=3 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::EOR_IMM, 0x07])
    );
}

#[test]
fn compatible_pointer_deref_compare_uses_indirect_indexed_rhs() {
    let output = generate_compatible_source_with_origin(
            "BYTE a,b,x BYTE POINTER p,q PROC Main() p=@a q=@b IF p^=q^ THEN x=1 FI IF p^>q^ THEN x=2 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::EOR_IZY, runtime_zp::ELEMENT_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::CMP_IZY, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_bitwise_if_condition_is_materialized_before_branch() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,x PROC Main() IF a AND b THEN x=1 ELSE x=2 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(11).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::AND_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::BNE_REL,
        ]));
}

#[test]
fn modern_bitwise_if_condition_branches_from_accumulator_flags() {
    let output = generate_profile_source_with_origin(
        "BYTE a,b,x PROC Main() IF a AND b THEN x=1 ELSE x=2 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| matches!(
        bytes,
        [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::AND_ABS,
            0x01,
            0x30,
            opcode::BNE_REL | opcode::BEQ_REL,
        ]
    )));
    assert!(!output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::BNE_REL,
        ]));
}

#[test]
fn compatible_bitwise_compare_to_zero_materializes_condition() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,x PROC Main() IF (a&1)=0 THEN x=1 ELSE x=2 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::AND_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ZP
        ]));
}

#[test]
fn compatible_logical_and_condition_short_circuits_comparisons() {
    let output = generate_compatible_source_with_origin(
            "TYPE BLOCK=[CARD size,next] DEFINE NULL=\"0\" BLOCK POINTER current CARD nBytes BYTE x PROC Main() WHILE (current<>NULL) AND (current.size<nBytes) DO x=1 OD RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::CMP_ABS
        ]));
}

#[test]
fn compatible_logical_and_false_branches_to_final_false_block() {
    let output = generate_compatible_source_with_origin(
        "BYTE FUNC IsUpper(BYTE c) IF (c>='A) AND (c<='Z) THEN RETURN(1) FI RETURN(0)",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[14..17], &[opcode::JMP_ABS, 0x20, 0x30]);
    assert_eq!(&output.bytes[24..27], &[opcode::JMP_ABS, 0x20, 0x30]);
}

#[test]
fn compatible_logical_or_condition_short_circuits_comparisons() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,x PROC Main() IF (a=1) OR (b=2) THEN x=1 ELSE x=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| bytes[1] == 0x01)
            .any(|bytes| bytes[0] == opcode::EOR_IMM || bytes[0] == opcode::CMP_IMM)
    );
    assert!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| bytes[1] == 0x02)
            .any(|bytes| bytes[0] == opcode::EOR_IMM || bytes[0] == opcode::CMP_IMM)
    );
}

#[test]
fn compatible_else_zero_store_does_not_reuse_y_from_true_branch() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,x PROC Main() IF a=b THEN x=1 ELSE x=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDY_IMM, 0x00, opcode::STY_ABS, 0x02])
    );
    assert!(
        !output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::DEY, opcode::STY_ABS, 0x02, 0x30])
    );
}

#[test]
fn compatible_pointer_addition_equality_condition_materializes_left_operand() {
    let output = generate_compatible_source_with_origin(
            "TYPE BLOCK=[CARD size,next] BLOCK POINTER last,target CARD nBytes BYTE x PROC Main() IF last+last.size=target THEN x=1 FI IF target+nBytes=last THEN x=2 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(output.bytes.windows(7).any(|bytes| {
        bytes[0] == opcode::LDA_ZP
            && bytes[1] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[2] == opcode::EOR_ABS
            && bytes[5] == opcode::BNE_REL
    }));
}

#[test]
fn compatible_function_call_condition_branches_on_return_slot() {
    let output = generate_compatible_source_with_origin(
            "BYTE x BYTE FUNC IsNonzero(BYTE c) RETURN(c) PROC Main() IF IsNonzero(x) THEN x=2 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address(), opcode::BNE_REL])
    );
}

#[test]
fn compatible_graphics_color_builtin_uses_runtime_location() {
    let output =
        generate_compatible_source_with_origin("PROC Main() color=2 RETURN", 0x3000).unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ABS, 0xFD, 0x02, opcode::RTS,])
    );
}

#[test]
fn compatible_graphics_builtin_uses_cartridge_entry() {
    let output =
        generate_compatible_source_with_origin("PROC Main() Graphics(1) RETURN", 0x3000).unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x54, 0xA6])
    );
}

#[test]
fn compatible_error_builtin_uses_runtime_entry() {
    let output =
        generate_compatible_source_with_origin("PROC Main() Error(71,0,71) RETURN", 0x3000)
            .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xCB, 0x04])
    );
}

#[test]
fn compatible_plot_builtin_uses_cartridge_entry_and_stages_expression_args() {
    let output = generate_compatible_source_with_origin(
        "CARD x,x1 BYTE y,y1 PROC Main() Plot(x+x1,y+y1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xC3, 0xA6,])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARGS.address(),])
    );
}

#[test]
fn compatible_drawto_builtin_uses_cartridge_entry() {
    let output =
        generate_compatible_source_with_origin("PROC Main() DrawTo(1,2) RETURN", 0x3000).unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x8C, 0xA6,])
    );
}

#[test]
fn compatible_call_stages_unary_neg_arguments() {
    let output = generate_compatible_source_with_origin(
        "INT theta PROC Turn(INT theta) RETURN PROC Main() theta=1 Turn(-theta) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARGS.address(),])
    );
}

#[test]
fn compatible_scompare_builtin_branches_on_signed_return() {
    let output = generate_compatible_source_with_origin(
            "CHAR ARRAY a=\"A\",b=\"B\" BYTE x PROC Main() IF SCompare(a,b)<0 THEN x=1 ELSEIF SCompare(a,b)>0 THEN x=2 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x64, 0xA8])
    );
    assert!(output.bytes.contains(&opcode::BMI_REL));
}

#[test]
fn compatible_io_builtins_use_cartridge_entries() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY text=\"X\" PROC Main() PrintD(6,text) InputS(text) InputSD(6,text) InputMD(6,text,7) PrintF(\"%H%E\",1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x86, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x8C, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x93, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x99, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xCC, 0xA3])
    );
}

#[test]
fn compatible_resident_byte_array_call_can_drive_scalar_if() {
    let output = generate_compatible_source_with_origin(
        "BYTE x PROC Main() IF EOF(1) THEN x=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0xC1, 0x05, opcode::BNE_REL])
    );
}

#[test]
fn compatible_xio_builtin_uses_cartridge_entry_and_abi_slots() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY fname=\"D:FOO\" PROC Main() XIO(5,0,32,0,0,fname) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(3).address(),
            opcode::LDA_IMM
        ]));
    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x20,
            opcode::LDX_IMM,
            0x00,
            opcode::LDA_IMM,
            0x05,
            opcode::JSR_ABS
        ]));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xDE, 0xA4])
    );
}

#[test]
fn compatible_zero_builtin_uses_cartridge_entry() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY buf(8) PROC Main() Zero(buf,8) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x8A, 0xA7])
    );
}

#[test]
fn compatible_break_builtin_uses_cartridge_entry() {
    let output =
        generate_compatible_source_with_origin("PROC Main() Break() RETURN", 0x3000).unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xDA, 0xA7])
    );
}

#[test]
fn compatible_sized_byte_array_numeric_initializer_is_inline_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY buf(4)=[1 2 3 4] PROC Main() Zero(buf,4) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..4], &[1, 2, 3, 4]);
    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x04,
            opcode::LDX_IMM,
            0x30,
            opcode::LDA_IMM,
            0x00,
            opcode::JSR_ABS
        ]));
}

#[test]
fn compatible_sized_byte_array_char_initializer_preserves_atascii_bytes() {
    let output = generate_compatible_source_with_origin(
        r#"BYTE ARRAY shape(6)=['\{$00}'@'\{INV: }'\{$02}'A'\{$FF}] PROC Main() RETURN"#,
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..6], &[0x00, b'@', 0xA0, 0x02, b'A', 0xFF]);
}

#[test]
fn compatible_open_close_builtins_use_cartridge_entries() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() Close(6) Open(6,\"S:\",$1C,0) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x79, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x44, 0xA4])
    );
}

#[test]
fn compatible_put_builtins_and_device_use_cartridge_locations() {
    let output = generate_compatible_source_with_origin(
        "BYTE d PROC Main() d=device PutD(0,'A) PutDE(0) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_ZP, 0xB7])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xD1, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0xDA, 0xA4])
    );
}

#[test]
fn compatible_print_put_short_builtins_use_cartridge_entries() {
    let output = generate_compatible_source_with_origin(
        "BYTE b PROC Main() Print(\"X\") PrintBE(b) Put(b) PutE() RETURN",
        0x3000,
    )
    .unwrap();

    for address in [0xA47F, 0xA4EC, 0xA4CE, 0xA4CC] {
        assert!(output.bytes.windows(3).any(|bytes| bytes
            == [
                opcode::JSR_ABS,
                Absolute::new(address).low(),
                Absolute::new(address).high()
            ]));
    }
}

#[test]
fn compatible_additional_resident_builtins_use_cartridge_entries() {
    let output = generate_compatible_source_with_origin(
        "BYTE b CARD c PROC Main() b=Peek(88) c=PeekC(88) Poke(c,b) PokeC(c,c) \
         b=Stick(0) b=STrig(0) Sound(0,1,2,3) SndRst() b=InputB() b=GetD(1) RETURN",
        0x3000,
    )
    .unwrap();

    for address in [
        0xA767, 0xA777, 0xA781, 0xA74E, 0xAD2F, 0xA704, 0xA721, 0xA588, 0xA4AD,
    ] {
        assert!(output.bytes.windows(3).any(|bytes| bytes
            == [
                opcode::JSR_ABS,
                Absolute::new(address).low(),
                Absolute::new(address).high()
            ]));
    }
}

#[test]
fn compatible_printe_builtin_uses_cartridge_entry() {
    let output = generate_compatible_source_with_origin(
        "PROC Main() PrintE(\"X\") Printe(\"Y\") RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        count_jsr_to(&output.bytes, 0xA46C),
        2,
        "PrintE and case variants should resolve to the resident cartridge entry"
    );
}

#[test]
fn compatible_indexed_rand_assignment_preserves_dynamic_target() {
    let output = generate_compatible_source_with_origin(
        "CARD i BYTE ARRAY d(500) PROC Main() FOR i=0 TO 1 DO d(i)=Rand(0) OD RETURN",
        0x3000,
    )
    .unwrap();

    let pointer = runtime_zp::ARRAY_ADDR.address();
    assert!(output.bytes.windows(19).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            pointer + 1,
            opcode::PHA,
            opcode::LDA_ZP,
            pointer,
            opcode::PHA,
            opcode::LDA_IMM,
            0x00,
            opcode::JSR_ABS,
            0xF1,
            0xA6,
            opcode::PLA,
            opcode::STA_ZP,
            pointer,
            opcode::PLA,
            opcode::STA_ZP,
            pointer + 1,
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDY_IMM,
            0x00,
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, pointer])
    );
}

#[test]
fn compatible_allows_routine_assignment_retargeting() {
    let output = generate_compatible_source_with_origin(
        "PROC A() RETURN PROC B() RETURN PROC T() PROC Main() T=A T() T=B T() RETURN",
        0x3000,
    )
    .unwrap();
    let t_address = output
        .routine_addresses
        .iter()
        .find(|routine| routine.name == "T")
        .expect("T routine address")
        .address;

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            Absolute::new(t_address).low(),
            Absolute::new(t_address).high()
        ]));
}

#[test]
fn compatible_routine_name_assignment_to_card_slot() {
    let output = generate_compatible_source_with_origin(
        "CARD handler PROC Target() RETURN PROC Main() handler=@Target RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x01, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x00, 0x30])
    );
}

#[test]
fn modern_proc_pointer_call_emits_indirect_trampoline() {
    let output = generate_profile_source_with_origin(
        "BYTE seen PROC Target() seen=1 RETURN PROC POINTER p PROC Main() p=@Target p() seen=2 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::JMP_ABS && bytes[6] == opcode::JMP_IND
    }));
}

#[test]
fn modern_public_routine_pointer_targets_direct_entry_after_local_storage() {
    let output = generate_profile_source_with_origin(
        "BYTE seen PROC Target() BYTE local local=1 seen=local RETURN PROC POINTER p PROC Main() p=@Target p() seen=2 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let target = routine_address(&output, "Target").expect("Target address");
    let local = storage_symbol(
        &output,
        CodegenSymbolScope::Routine("Target".to_string()),
        "LOCAL",
    );
    assert_eq!(target, local.address.wrapping_add(local.size));
    assert_ne!(
        output.bytes[usize::from(target.wrapping_sub(output.origin))],
        opcode::JMP_ABS,
        "the address-observable entry should be the routine prologue itself"
    );
    let pointer = storage_symbol(&output, CodegenSymbolScope::Global, "P");
    assert!(output.bytes.windows(10).any(|bytes| {
        bytes
            == [
                opcode::LDA_IMM,
                Absolute::new(target).high(),
                opcode::STA_ABS,
                Absolute::new(pointer.address.wrapping_add(1)).low(),
                Absolute::new(pointer.address.wrapping_add(1)).high(),
                opcode::LDA_IMM,
                Absolute::new(target).low(),
                opcode::STA_ABS,
                Absolute::new(pointer.address).low(),
                Absolute::new(pointer.address).high(),
            ]
    }));
    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::JMP_ABS && bytes[6] == opcode::JMP_IND
    }));
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.routine.as_deref() == Some("Target")
            && optimization.kind == CodegenOptimizationKind::TrampolineElided
    }));
}

#[test]
fn compatible_proc_pointer_call_emits_indirect_trampoline() {
    let output = generate_compatible_source_with_origin(
        "BYTE seen PROC Target() seen=1 RETURN PROC POINTER p PROC Main() p=@Target p() seen=2 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::JMP_ABS && bytes[6] == opcode::JMP_IND
    }));
}

#[test]
fn modern_func_pointer_call_result_can_be_assigned() {
    let output = generate_profile_source_with_origin(
        "BYTE FUNC Get() RETURN(7) BYTE FUNC POINTER f BYTE x PROC Main() f=@Get x=f() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::JMP_ABS && bytes[6] == opcode::JMP_IND
    }));
}

#[test]
fn compatible_func_pointer_call_result_can_be_assigned() {
    let output = generate_compatible_source_with_origin(
        "BYTE FUNC Get() RETURN(7) BYTE FUNC POINTER f BYTE x PROC Main() f=@Get x=f() RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::JMP_ABS && bytes[6] == opcode::JMP_IND
    }));
}

#[test]
fn compatible_card_alias_decl_reserves_vector_cells() {
    let output = generate_compatible_source_with_origin(
        "CARD Timer2=$21A,TempVec,Start,Select,Option PROC Main() RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..16], &[0; 16]);
    assert_eq!(
        output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case("TempVec"))
            .map(|symbol| symbol.address),
        Some(0x3000)
    );
    assert_eq!(
        output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case("Start"))
            .map(|symbol| symbol.address),
        Some(0x3004)
    );
}

#[test]
fn compatible_card_alias_decl_pads_initialized_following_card() {
    let output = generate_compatible_source_with_origin(
            "CARD APPMHI=$000E,globalWord=[0],vectorA,vectorB BYTE ARRAY table(2)=[1 2] PROC Main() RETURN",
            0x3000,
        )
        .unwrap();

    let symbol_address = |name: &str| {
        output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
            .map(|symbol| symbol.address)
    };
    assert_eq!(symbol_address("globalWord"), Some(0x3003));
    assert_eq!(symbol_address("vectorA"), Some(0x3005));
    assert_eq!(symbol_address("vectorB"), Some(0x3009));
    assert_eq!(symbol_address("table"), Some(0x300D));
    assert_eq!(&output.bytes[..13], &[0; 13]);
    assert_eq!(&output.bytes[13..15], &[1, 2]);
}

#[test]
fn compatible_bitwise_zero_if_branches_from_materialized_result() {
    let output = generate_compatible_source_with_origin(
        "BYTE console=$D01F,hit PROC Main() IF (console&1)=0 THEN hit=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::AND_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::BEQ_REL,
            0x03,
            opcode::JMP_ABS,
        ]));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::CMP_IMM, 0x00])
    );
}

#[test]
fn compatible_runtime_shift_materializes_left_arithmetic_operand() {
    let output = generate_compatible_source_with_origin(
        "CARD low,high,mid PROC Main() mid=(low+high) RSH 1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_RSH.low(),
            runtime_helper::CARTRIDGE_RSH.high()
        ]));
}

#[test]
fn compatible_subtract_prepares_indexed_right_before_carry_chain() {
    let output = generate_compatible_source_with_origin(
            "CARD out,hi BYTE mode CARD ARRAY mask=[0 $F800 $FC00],size=[0 $800 $400] PROC Main() out=(hi-size(mode)-$80)&mask(mode) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::SBC_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::LDA_ABS,
            0x04,
            0x30,
        ]));
}

#[test]
fn compatible_runtime_shift_inlines_byte_constant_shift_count() {
    let output = generate_compatible_source_with_origin(
        "BYTE out,value,n PROC Main() out=value LSH (n LSH 1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(14).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::ASL_A,
            opcode::STA_ZP,
            runtime_zp::AFCUR.address(),
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::LDX_IMM,
            0x00,
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_LSH.low(),
            runtime_helper::CARTRIDGE_LSH.high()
        ]));
    assert_eq!(
        output
            .bytes
            .windows(3)
            .filter(|bytes| {
                *bytes
                    == [
                        opcode::JSR_ABS,
                        runtime_helper::CARTRIDGE_LSH.low(),
                        runtime_helper::CARTRIDGE_LSH.high(),
                    ]
            })
            .count(),
        1
    );
}

#[test]
fn compatible_add_runtime_multiply_preserves_left_to_right_materialization() {
    let output = generate_compatible_source_with_origin(
            "CARD out,base BYTE mode,n CARD ARRAY waste=[0 768 384],size=[0 $100 $80] PROC Main() out=base+waste(mode)+(n*size(mode)) RETURN",
            0x3000,
        )
        .unwrap();

    let waste_index = output
        .bytes
        .windows(4)
        .position(|bytes| bytes == [opcode::ADC_ABS, 0x0C, 0x30, opcode::STA_ZP])
        .unwrap();
    let size_index = output
        .bytes
        .windows(4)
        .position(|bytes| bytes == [opcode::ADC_ABS, 0x14, 0x30, opcode::STA_ZP])
        .unwrap();

    assert!(waste_index < size_index);
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::INY,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_ABS,
        ]));
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::PHA,
        ]));
    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_ZP,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_add_runtime_multiply_keeps_scalar_addend_out_of_mul_clobbers() {
    let output = generate_compatible_source_with_origin(
        "CARD out,base BYTE n PROC Main() out=base+768+(n*$100) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(16).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ABS,
            0x03,
            0x30,
            opcode::ADC_IMM,
            0x03,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::LDA_IMM,
        ]));
    assert!(output.bytes.windows(20).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_MUL.low(),
            runtime_helper::CARTRIDGE_MUL.high(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::TXA,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::CLC,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::ADC_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
        ]));
}

#[test]
fn modern_comparison_materializes_function_call_operands() {
    let output = generate_profile_source_with_origin(
            "INT a,b BYTE x INT FUNC Id(INT n) RETURN(n) PROC Main() IF Id(a)+0<Id(b) THEN x=1 FI RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ADDR.address()])
    );
}

#[test]
fn compatible_profile_rejects_function_calls_as_call_arguments() {
    assert_compatible_diagnostic_contains(
        "BYTE FUNC F() RETURN(1) PROC Take(BYTE a) RETURN PROC Main() Take(F()) RETURN",
        "function calls as routine call arguments",
    );
}

#[test]
fn compatible_profile_rejects_function_calls_in_arithmetic() {
    assert_compatible_diagnostic_contains(
        "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() out=F()+G() RETURN",
        "function calls in arithmetic expressions",
    );
}

#[test]
fn compatible_profile_accepts_single_call_runtime_multiply_operand() {
    generate_compatible_source_with_origin(
        "INT out,x INT FUNC F(INT n) RETURN(n) PROC Main() out=x*F(x) RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn compatible_profile_accepts_comparisons_with_function_calls_on_both_sides() {
    generate_compatible_source_with_origin(
            "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() IF F()=G() THEN out=1 FI RETURN",
            0x3000,
        )
        .unwrap();
}

#[test]
fn compatible_profile_accepts_zero_identity_arithmetic_around_call() {
    generate_compatible_source_with_origin(
        "BYTE out BYTE FUNC F() RETURN(1) PROC Main() IF F()+0=1 THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn compatible_profile_accepts_single_call_constant_adjustment() {
    generate_compatible_source_with_origin(
        "BYTE out BYTE FUNC F() RETURN(2) PROC Main() out=F()-1 out=1+F() RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn compatible_profile_accepts_single_call_add_to_non_call_operand() {
    generate_compatible_source_with_origin(
        "CARD out,y CARD FUNC F(BYTE n) RETURN(n) PROC Main() out=F(0)+y RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn compatible_profile_accepts_single_call_constant_shift() {
    generate_compatible_source_with_origin(
        "BYTE out BYTE FUNC F(BYTE n) RETURN(n) PROC Main() out=F(15) LSH 4 RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn compatible_materialized_signed_call_compare_uses_signed_branch() {
    let output = generate_compatible_source_with_origin(
            "INT a,b BYTE out INT FUNC Id(INT n) RETURN(n) PROC Main() IF Id(a)<Id(b) THEN out=1 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.contains(&opcode::BMI_REL));
    assert!(!output.bytes.contains(&opcode::BCC_REL));
    assert_eq!(
        output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::PHA)
            .count(),
        2
    );
    assert_eq!(
        output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::PLA)
            .count(),
        2
    );
    assert_eq!(count_pair(&output.bytes, opcode::STA_ZP, 0xC0), 0);
    assert_eq!(count_pair(&output.bytes, opcode::STA_ZP, 0xC1), 0);
}

#[test]
fn compatible_profile_rejects_indexed_assignment_with_calls_on_both_sides() {
    assert_compatible_diagnostic_contains(
        "BYTE ARRAY a(4) BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() a(F())=G() RETURN",
        "indexed assignments with function calls on both sides",
    );
}

#[test]
fn compatible_profile_does_not_treat_array_calls_as_routine_call_extensions() {
    generate_compatible_source_with_origin(
        "BYTE ARRAY a(4) PROC Take(BYTE value) RETURN PROC Main() Take(a(1)) RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn compatible_profile_accepts_original_valid_single_call_surfaces() {
    generate_compatible_source_with_origin(
            "BYTE ARRAY a(4) BYTE out BYTE FUNC F() RETURN(1) PROC Main() IF F()=1 THEN out=a(F()) FI a(F())=2 RETURN",
            0x3000,
        )
        .unwrap();
}

#[test]
fn modern_profile_accepts_evaluation_order_extensions() {
    let sources = [
        "BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Take(BYTE a,b) RETURN PROC Main() Take(F(),G()) RETURN",
        "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() out=F()+G() RETURN",
        "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() IF F()=G() THEN out=1 FI RETURN",
        "BYTE ARRAY a(4) BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() a(F())=G() RETURN",
    ];

    for source in sources {
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();
    }
}

#[test]
fn compatible_nested_word_addition_materializes_left_operand_once() {
    let output =
        generate_compatible_source_with_origin("INT a,b,c,d PROC Main() d=a+b+c+1 RETURN", 0x3000)
            .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ELEMENT_ADDR.offset(1).address()])
    );
}

#[test]
fn compatible_adjacent_byte_one_stores_reuse_y_in_straight_line_code() {
    let output =
        generate_compatible_source_with_origin("BYTE a,b PROC Main() a=1 b=1 RETURN", 0x3000)
            .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x01,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::STY_ABS,
            0x01,
            0x30,
        ]));
}

#[test]
fn compatible_byte_constant_stores_keep_y_live_across_a_stores_and_dey_to_zero() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,c,d PROC Main() a=1 b=2 c=1 d=0 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(18).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x01,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::LDA_IMM,
            0x02,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::STY_ABS,
            0x02,
            0x30,
            opcode::DEY,
            opcode::STY_ABS,
            0x03,
            0x30,
            opcode::RTS,
        ]));
}

#[test]
fn compatible_isolated_zero_array_store_uses_accumulator() {
    let output = generate_compatible_source_with_origin(
        "PROC ARRAYTEST() BYTE ARRAY ba(64) BYTE i ba(0)=0 ba(1)=2 ba(4)=3 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ABS, 0x00, 0x30,])
    );
    assert!(
        !output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDY_IMM, 0x00, opcode::STY_ABS, 0x00, 0x30,])
    );
}

#[test]
fn compatible_byte_constant_stores_walk_y_from_zero_to_one() {
    let output = generate_compatible_source_with_origin(
        "BYTE a CARD b,c PROC Main() a=1 a=0 b=$0100 c=$0001 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(24).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x01,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::DEY,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::INY,
            opcode::STY_ABS,
            0x02,
            0x30,
            opcode::DEY,
            opcode::STY_ABS,
            0x01,
            0x30,
            opcode::STY_ABS,
            0x04,
            0x30,
            opcode::INY,
            opcode::STY_ABS,
            0x03,
            0x30,
        ]));
}

#[test]
fn compatible_byte_one_store_walks_from_known_y_zero() {
    let output = generate_compatible_source_with_origin(
        "BYTE n,v PROC Read(CHAR ARRAY s) v=s(0) n=1 RETURN PROC Main() Read(\"A\") RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::INY,
            opcode::STY_ABS,
            0x00,
        ]));
}

#[test]
fn compatible_zero_page_word_zero_store_reuses_known_y() {
    let output = generate_compatible_source_with_origin(
        "BYTE zp_i=$E1 CARD zp_sum=$E4 PROC Main() zp_i=0 zp_sum=0 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x00,
            opcode::STY_ZP,
            0xE1,
            opcode::STY_ZP,
            0xE5,
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STY_ZP, 0xE4])
    );
    assert!(
        !output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ZP, 0xE5])
    );
}

#[test]
fn compatible_byte_zero_store_and_compare_use_original_short_shapes() {
    let output = generate_compatible_source_with_origin(
        "BYTE x,y PROC Main() x=0 IF x=0 THEN y=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDY_IMM, 0x00, opcode::STY_ABS, 0x00, 0x30,])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::BEQ_REL,])
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::EOR_IMM, 0x00])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::INY, opcode::STY_ABS, 0x01, 0x30,])
    );
}

#[test]
fn compatible_byte_function_return_constant_compare_branches_directly() {
    let output = generate_compatible_source_with_origin(
        "BYTE y BYTE FUNC F() RETURN(10) PROC Main() IF F()=$0A THEN y=1 ELSE y=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    let jsr = output
        .bytes
        .windows(3)
        .position(|bytes| bytes[0] == opcode::JSR_ABS)
        .unwrap();
    assert_eq!(
        &output.bytes[jsr + 3..jsr + 9],
        &[
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::EOR_IMM,
            0x0A,
            opcode::BEQ_REL,
            0x03,
        ]
    );
    assert!(
        !output.bytes[jsr + 3..]
            .windows(2)
            .take(4)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_byte_function_return_zero_compare_omits_eor_zero() {
    let output = generate_compatible_source_with_origin(
        "BYTE y BYTE FUNC F() RETURN(0) PROC Main() IF F()=0 THEN y=1 ELSE y=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    let jsr = output
        .bytes
        .windows(3)
        .position(|bytes| bytes[0] == opcode::JSR_ABS)
        .unwrap();
    assert_eq!(
        &output.bytes[jsr + 3..jsr + 6],
        &[opcode::LDA_ZP, runtime_zp::ARGS.address(), opcode::BEQ_REL,]
    );
    assert!(
        !output.bytes[jsr + 3..jsr + 8]
            .windows(2)
            .any(|bytes| bytes == [opcode::EOR_IMM, 0x00])
    );
}

#[test]
fn compatible_byte_function_return_ordered_constant_compare_branches_directly() {
    let output = generate_compatible_source_with_origin(
        "BYTE y BYTE FUNC F() RETURN(10) PROC Main() IF F()<11 THEN y=1 ELSE y=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    let jsr = output
        .bytes
        .windows(3)
        .position(|bytes| bytes[0] == opcode::JSR_ABS)
        .unwrap();
    assert_eq!(
        &output.bytes[jsr + 3..jsr + 9],
        &[
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::CMP_IMM,
            0x0B,
            opcode::BCC_REL,
            0x03,
        ]
    );
    assert!(
        !output.bytes[jsr + 3..]
            .windows(2)
            .take(4)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
    );
}

#[test]
fn compatible_single_call_boolean_and_signed_conditions_compile() {
    generate_compatible_source_with_origin(
            "BYTE y BYTE FUNC F() RETURN(10) INT FUNC S() RETURN(-2) PROC Main() IF F() AND 1 THEN y=1 FI IF S()<3 THEN y=2 FI RETURN",
            0x3000,
        )
        .unwrap();
}

#[test]
fn compatible_word_constant_stores_preserve_y_hint_across_a_stores() {
    let output = generate_compatible_source_with_origin(
        "BYTE a CARD b,c PROC Main() a=1 a=0 b=$FFFF c=$0001 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(26).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x01,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::DEY,
            opcode::STY_ABS,
            0x00,
            0x30,
            opcode::LDA_IMM,
            0xFF,
            opcode::STA_ABS,
            0x02,
            0x30,
            opcode::LDA_IMM,
            0xFF,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::STY_ABS,
            0x04,
            0x30,
            opcode::INY,
            opcode::STY_ABS,
            0x03,
            0x30,
        ]));
}

#[test]
fn compatible_byte_eq_true_label_can_reuse_y_one_store_hint() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,x PROC Main() a=1 b=1 IF a=b THEN x=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::BEQ_REL,
            0x03,
            opcode::JMP_ABS,
            0x1C,
            0x30,
            opcode::STY_ABS,
            0x02,
            0x30,
        ]));
}

#[test]
fn compatible_byte_equality_condition_uses_eor_branch_shape() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,x PROC Main() IF a=b THEN x=1 ELSE x=2 FI IF a#b THEN x=3 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::EOR_ABS,
            0x01,
            0x30,
            opcode::BEQ_REL,
        ]));
    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::EOR_ABS,
            0x01,
            0x30,
            opcode::BNE_REL,
        ]));
}

#[test]
fn generates_while_control_flow_with_comparison() {
    let output = generate_source("BYTE x PROC Main() WHILE x<3 DO x=x+1 OD RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0xC9,
            0x03,
            0x90,
            0x03,
            0x4C,
            0x16,
            0x30,
            0xAD,
            0x00,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x00,
            0x06,
            0x4C,
            0x00,
            0x30,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_signed_int_less_than_comparison() {
    let output = generate_source("INT a,b BYTE x PROC Main() IF a<b THEN x=1 FI RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x01,
            0x06,
            0x4D,
            0x03,
            0x06,
            0x10,
            0x08,
            0xAD,
            0x01,
            0x06,
            0x30,
            0x18,
            0x4C,
            0x22,
            0x30,
            0xAD,
            0x01,
            0x06,
            0xCD,
            0x03,
            0x06,
            0x90,
            0x0D,
            0xD0,
            0x08,
            0xAD,
            0x00,
            0x06,
            0xCD,
            0x02,
            0x06,
            0x90,
            0x03,
            0x4C,
            0x2A,
            0x30,
            0xA9,
            0x01,
            0x8D,
            0x04,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_signed_int_greater_equal_comparison() {
    let output = generate_source("INT a,b BYTE x PROC Main() IF a>=b THEN x=1 FI RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x01,
            0x06,
            0x4D,
            0x03,
            0x06,
            0x10,
            0x08,
            0xAD,
            0x01,
            0x06,
            0x30,
            0x18,
            0x4C,
            0x22,
            0x30,
            0xAD,
            0x01,
            0x06,
            0xCD,
            0x03,
            0x06,
            0x90,
            0x0D,
            0xD0,
            0x08,
            0xAD,
            0x00,
            0x06,
            0xCD,
            0x02,
            0x06,
            0x90,
            0x03,
            0x4C,
            0x28,
            0x30,
            0x4C,
            0x2D,
            0x30,
            0xA9,
            0x01,
            0x8D,
            0x04,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_signed_comparisons_use_original_subtract_sign_shape() {
    let output = generate_compatible_source_with_origin(
        "INT a,b BYTE x PROC Main() IF a<b THEN x=1 FI IF a>=b THEN x=2 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_ABS,
            0x02,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::SBC_ABS,
            0x03,
            0x30,
            opcode::BMI_REL,
        ]));
    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_ABS,
            0x02,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::SBC_ABS,
            0x03,
            0x30,
            opcode::BPL_REL,
        ]));
}

#[test]
fn compatible_signed_index_compare_to_zero_branches_without_materialization() {
    let output = generate_compatible_source_with_origin(
        "INT ARRAY nums(4) BYTE i,x PROC Main() i=1 IF nums(i)<0 THEN x=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2] == opcode::CMP_IMM
            && bytes[3] == 0x00
            && bytes[4] == opcode::INY
            && bytes[5] == opcode::LDA_IZY
            && bytes[6] == runtime_zp::ARRAY_ADDR.address()
    }));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::CMP_IMM,
            0x00,
            opcode::BCC_REL,
            0x03,
        ]));
}

#[test]
fn compatible_signed_pointer_deref_compare_to_zero_uses_pointee_shape() {
    let output = generate_compatible_source_with_origin(
        "INT POINTER args BYTE y PROC Main() IF args^<0 THEN y=1 ELSE y=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(10).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::CMP_IMM,
            0x00,
            opcode::INY,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::SBC_IMM,
            0x00,
            opcode::BMI_REL,
        ]));
    assert!(!output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::CMP_IMM,
            0x00,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ABS,
            0x01,
        ]));
}

#[test]
fn compatible_signed_zero_less_than_pointer_deref_reuses_prepared_pointer() {
    let output = generate_compatible_source_with_origin(
        "INT POINTER args BYTE y PROC Main() IF args^>0 THEN y=1 ELSE y=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::LDY_IMM,
            0x00,
            opcode::CMP_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_IMM,
            0x00,
            opcode::INY,
        ]));
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| {
                *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()]
                    || *bytes == [opcode::STY_ZP, runtime_zp::ARRAY_ADDR.address()]
            })
            .count(),
        1
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.offset(1).address()])
            .count(),
        1
    );
}

#[test]
fn compatible_signed_pointer_index_compare_to_zero_uses_pointee_shape() {
    let output = generate_compatible_source_with_origin(
        "INT POINTER args BYTE i,y PROC Main() i=1 IF args(i)<0 THEN y=1 ELSE y=0 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(10).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::CMP_IMM,
            0x00,
            opcode::INY,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::SBC_IMM,
            0x00,
            opcode::BMI_REL,
        ]));
    assert!(!output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::CMP_IMM,
            0x00,
            opcode::BCC_REL,
        ]));
}

#[test]
fn compatible_signed_compare_true_label_can_reuse_y_one_store_hint() {
    let output = generate_compatible_source_with_origin(
        "INT a,b BYTE x PROC Main() a=1 IF a<b THEN x=1 ELSE x=2 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::BMI_REL
            && bytes[1] == 0x03
            && bytes[2] == opcode::JMP_ABS
            && bytes[5] == opcode::STY_ABS
    }));
    assert!(!output.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::BMI_REL
            && bytes[1] == 0x03
            && bytes[2] == opcode::JMP_ABS
            && bytes[5] == opcode::LDY_IMM
            && bytes[6] == 0x01
            && bytes[7] == opcode::STY_ABS
    }));
}

#[test]
fn compatible_word_equality_uses_original_eor_ora_shape() {
    let output = generate_compatible_source_with_origin(
        "CARD a,b BYTE x PROC Main() IF a=b THEN x=1 FI IF a#b THEN x=2 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::EOR_ABS,
            0x02,
            0x30,
            opcode::BNE_REL,
            0x06,
            opcode::ORA_ABS,
            0x01,
            0x30,
            opcode::EOR_ABS,
            0x03,
        ]));
}

#[test]
fn compatible_unsigned_word_comparisons_use_subtract_carry_shape() {
    let output = generate_compatible_source_with_origin(
        "CARD a,b BYTE x PROC Main() IF a<b THEN x=1 FI IF a>=b THEN x=2 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_ABS,
            0x02,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::SBC_ABS,
            0x03,
            0x30,
            opcode::BCC_REL,
        ]));
    assert!(output.bytes.windows(13).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_ABS,
            0x02,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::SBC_ABS,
            0x03,
            0x30,
            opcode::BCS_REL,
        ]));
}

#[test]
fn compatible_byte_less_equal_uses_reversed_carry_shape() {
    let output = generate_compatible_source_with_origin(
        "BYTE a,b,x PROC Main() IF a<=b THEN x=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::CMP_ABS,
            0x00,
            0x30,
            opcode::BCS_REL,
        ]));
}

#[test]
fn generates_do_until_control_flow() {
    let output = generate_source("BYTE x PROC Main() DO x=x+1 UNTIL x=3 OD RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x00,
            0x06,
            0xAD,
            0x00,
            0x06,
            0xC9,
            0x03,
            0xD0,
            0x03,
            0x4C,
            0x16,
            0x30,
            0x4C,
            0x00,
            0x30,
            opcode::RTS,
        ]
    );
}

#[test]
fn rejects_complex_until_condition_instead_of_emitting_buggy_code() {
    let err = generate_source("BYTE x,y PROC Main() DO x==+1 UNTIL (x+1)=y OD RETURN").unwrap_err();

    assert!(
        err.iter()
            .any(|diagnostic| diagnostic.message.contains("UNTIL conditions"))
    );
}

#[test]
fn compatible_final_infinite_do_omits_unreachable_implicit_rts() {
    let output = generate_compatible_source_with_origin(
        "BYTE x PROC Spin() DO x==+1 OD PROC Next() RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        !output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JMP_ABS, 0x04, 0x30, opcode::RTS,])
    );
}

#[test]
fn compatible_final_do_with_direct_exit_keeps_implicit_rts() {
    let output =
        generate_compatible_source_with_origin("BYTE x PROC Spin() DO EXIT OD", 0x3000).unwrap();

    assert!(output.bytes.ends_with(&[opcode::RTS]));
}

#[test]
fn generates_exit_to_loop_end() {
    let output = generate_source("BYTE x PROC Main() WHILE x DO EXIT OD x=1 RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xAD,
            0x00,
            0x06,
            0xD0,
            0x03,
            0x4C,
            0x0E,
            0x30,
            0x4C,
            0x0E,
            0x30,
            0x4C,
            0x00,
            0x30,
            0xA9,
            0x01,
            0x8D,
            0x00,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_simple_positive_for_loop() {
    let output =
        generate_source("BYTE i BYTE x PROC Main() FOR i=1 TO 3 DO x=x+1 OD RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x01,
            0x8D,
            0x00,
            0x06,
            0xAD,
            0x00,
            0x06,
            0xC9,
            0x03,
            0x90,
            0x05,
            0xF0,
            0x03,
            0x4C,
            0x26,
            0x30,
            0xAD,
            0x01,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x01,
            0x06,
            0xAD,
            0x00,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x00,
            0x06,
            0x4C,
            0x05,
            0x30,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_negative_for_step_loop() {
    let output =
        generate_source("BYTE i BYTE x PROC Main() FOR i=3 TO 1 STEP -1 DO x=x+1 OD RETURN")
            .unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x03,
            0x8D,
            0x00,
            0x06,
            0xA9,
            0x01,
            0xCD,
            0x00,
            0x06,
            0x90,
            0x05,
            0xF0,
            0x03,
            0x4C,
            0x26,
            0x30,
            0xAD,
            0x01,
            0x06,
            0x18,
            0x69,
            0x01,
            0x8D,
            0x01,
            0x06,
            0xAD,
            0x00,
            0x06,
            0x38,
            0xE9,
            0x01,
            0x8D,
            0x00,
            0x06,
            0x4C,
            0x05,
            0x30,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_zero_argument_user_proc_call() {
    let output = generate_source("PROC Helper() RETURN PROC Main() Helper() RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![opcode::RTS, 0x20, 0x00, 0x30, opcode::RTS]
    );
}

#[test]
fn generates_user_proc_call_arguments_with_original_abi() {
    let output =
        generate_source("PROC Foo(BYTE a) BYTE x x=a RETURN PROC Main() Foo(7) RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0x8D,
            0x00,
            0x06,
            0xAD,
            0x00,
            0x06,
            0x8D,
            0x01,
            0x06,
            opcode::RTS,
            0xA9,
            0x07,
            0x20,
            0x00,
            0x30,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_user_proc_call_allows_omitted_trailing_arguments() {
    let output = generate_compatible_source_with_origin(
        "PROC Foo(BYTE a,b,c) RETURN PROC Main() Foo($11) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x11,
            opcode::JSR_ABS,
            0x03,
            0x30,
            opcode::RTS
        ]));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDX_IMM, 0x11] || bytes == [opcode::LDY_IMM, 0x11])
    );
}

#[test]
fn compatible_staged_call_loads_only_supplied_argument_bytes() {
    let output = generate_compatible_source_with_origin(
        "BYTE x PROC Foo(BYTE a,b,c) RETURN PROC Main() x=$10 Foo(x+1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address()])
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()])
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDY_ZP, runtime_zp::ARGS.offset(2).address()])
    );
}

#[test]
fn compatible_call_register_args_load_zero_page_bytes_directly() {
    let output = generate_compatible_source_with_origin(
        "SET $491=$5C SET $492=$00 SET $E=$5C SET $F=$00 BYTE x,y \
             SET $491=$3000 SET $E=$3000 \
             PROC Foo(BYTE a,b,c) RETURN PROC Main() Foo(0,x,y) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDX_ZP, 0x5C])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDY_ZP, 0x5D])
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::TAX, opcode::JSR_ABS])
    );
}

#[test]
fn generates_function_return_through_args_zero_page() {
    let output =
        generate_source("BYTE x BYTE FUNC Inc(BYTE a) RETURN(a+1) PROC Main() x=Inc(7) RETURN")
            .unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0x8D,
            0x01,
            0x06,
            0xAD,
            0x01,
            0x06,
            0x18,
            0x69,
            0x01,
            0x85,
            0xA0,
            opcode::RTS,
            0xA9,
            0x07,
            0x20,
            0x00,
            0x30,
            0xA5,
            0xA0,
            0x8D,
            0x00,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_signed_unary_negation_return() {
    let output = generate_source("INT FUNC Neg(INT n) RETURN(-n)").unwrap();

    assert!(output.bytes.windows(14).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_IMM,
            0x00,
            opcode::SBC_ABS,
            0x00,
            0x06,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0x00,
            opcode::SBC_ABS,
            0x01,
            0x06,
            opcode::STA_ZP,
        ]));
}

#[test]
fn generates_numeric_address_system_proc_call() {
    let output = generate_source("PROC Sys=$1234() PROC Main() Sys() RETURN").unwrap();
    assert_eq!(output.bytes, vec![0x20, 0x34, 0x12, opcode::RTS]);
}

#[test]
fn generates_system_proc_call_arguments_with_original_abi() {
    let output =
        generate_source("PROC Sys=$1234(BYTE a,b,c) PROC Main() Sys(1,2,3) RETURN").unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA0,
            0x03,
            0xA2,
            0x02,
            0xA9,
            0x01,
            0x20,
            0x34,
            0x12,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_fixed_address_call_allows_omitted_trailing_arguments() {
    let output = generate_compatible_source_with_origin(
        "PROC Sys=$1234(BYTE a,b,c) PROC Main() Sys($11) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        output.bytes,
        vec![
            opcode::JMP_ABS,
            0x03,
            0x30,
            opcode::LDA_IMM,
            0x11,
            opcode::JSR_ABS,
            0x34,
            0x12,
            opcode::RTS,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_fixed_address_proc_uses_caller_byte_abi() {
    let output = generate_compatible_source_with_origin(
            "BYTE gb PROC Sys=$1234(BYTE a CARD w BYTE b BYTE POINTER p) PROC Main() Sys($11,$2233,$44,@gb) RETURN",
            0x3000,
        )
        .unwrap();

    assert_eq!(&output.bytes[..4], &[0x00, opcode::JMP_ABS, 0x04, 0x30]);
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x44, opcode::STA_ZP, 0xA3])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ZP, 0xA4])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x30, opcode::STA_ZP, 0xA5])
    );
    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x22,
            opcode::LDX_IMM,
            0x33,
            opcode::LDA_IMM,
            0x11,
            opcode::JSR_ABS,
            0x34,
            0x12
        ]));
}

#[test]
fn compatible_call_word_arg_reuses_record_field_pointer_for_both_bytes() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[CARD size,next] Pair rec Pair POINTER p PROC Sys=$1234(BYTE a,b,c CARD v) PROC Main() p=@rec Sys(1,2,3,p.next) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2] == opcode::STA_ZP
            && bytes[3] == runtime_zp::ARGS.offset(4).address()
            && bytes[4] == opcode::DEY
            && bytes[5] == opcode::LDA_IZY
            && bytes[6] == runtime_zp::ARRAY_ADDR.address()
            && bytes[7] == opcode::STA_ZP
            && bytes[8] == runtime_zp::ARGS.offset(3).address()
    }));
    assert!(!output.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2] == opcode::STA_ZP
            && bytes[4..]
                .windows(3)
                .any(|inner| inner == [opcode::ADC_IMM, 0x02, opcode::STA_ZP])
    }));
}

#[test]
fn compatible_fixed_address_proc_ignores_empty_machine_body() {
    let output = generate_compatible_source_with_origin(
        "PROC Sys=$1234(BYTE a) [] PROC Main() Sys(7) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        output.bytes,
        vec![
            opcode::JMP_ABS,
            0x03,
            0x30,
            opcode::LDA_IMM,
            0x07,
            opcode::JSR_ABS,
            0x34,
            0x12,
            opcode::RTS,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_array_constant_index_load_and_store() {
    let output =
        generate_source("BYTE ARRAY data(4) BYTE x PROC Main() data(2)=7 x=data(2) RETURN")
            .unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x07,
            0x8D,
            0x02,
            0x06,
            0xAD,
            0x02,
            0x06,
            0x8D,
            0x04,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_card_array_constant_index_load_and_store() {
    let output =
        generate_source("CARD ARRAY words(2) CARD x PROC Main() words(1)=$1234 x=words(1) RETURN")
            .unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x34,
            0x8D,
            0x02,
            0x06,
            0xA9,
            0x12,
            0x8D,
            0x03,
            0x06,
            0xAD,
            0x02,
            0x06,
            0x8D,
            0x04,
            0x06,
            0xAD,
            0x03,
            0x06,
            0x8D,
            0x05,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_byte_array_dynamic_index_load_and_store() {
    let output =
        generate_source("BYTE ARRAY data(4) BYTE i BYTE x PROC Main() data(i)=7 x=data(i) RETURN")
            .unwrap();
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_IMM,
            0x00
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ADDR.address()])
    );
}

#[test]
fn compatible_hex_array_dimension_allocates_requested_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY data($04) BYTE x PROC Main() data(3)=7 x=data(3) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(&output.bytes[..5], &[0x00, 0x00, 0x04, 0x00, 0x00]);
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x07, opcode::STA_ABS, 0x03, 0x30])
    );
    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x03, 0x30, opcode::STA_ABS, 0x04, 0x30])
    );
}

#[test]
fn generates_card_array_dynamic_index_load_and_store() {
    let output = generate_source(
        "CARD ARRAY words(4) BYTE i CARD x PROC Main() words(i)=$1234 x=words(i) RETURN",
    )
    .unwrap();
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::ASL_A, opcode::CLC, opcode::ADC_IMM, 0x00])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ADDR.address()])
    );
}

#[test]
fn compatible_card_array_index_uses_zero_page_scalar_directly() {
    let output = generate_compatible_source_with_origin(
        "BYTE i=$E0 CARD ARRAY words(4) CARD x PROC Main() words(i)=$1234 x=words(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ZP, 0xE0, opcode::ASL_A, opcode::PHP])
    );
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address()
        ]));
}

#[test]
fn compatible_indirect_assignment_preserves_pointer_across_call_rhs() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY p BYTE i BYTE FUNC F() [$A9 $99 $85 $AE $A9 $88 $85 $AF $A9 $22 $85 $A0 $60] \
             PROC Main() p=$4000 i=1 p(i)=F() RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PHA,
            opcode::JSR_ABS,
            0x03,
        ]));
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
        ]));
}

#[test]
fn modern_indirect_call_assignment_skips_pointer_save_for_known_preserving_callee() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE x BYTE FUNC Id(BYTE v) RETURN(v) \
             PROC Main() p=$4000 x=7 p^=Id(x) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
    }));
}

#[test]
fn modern_indirect_call_assignment_skips_pointer_save_when_arg_uses_same_pointer() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE FUNC Id(BYTE v) RETURN(v) \
             PROC Main() p=$4000 p^=Id(p^) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
            && optimization
                .message
                .contains("prepared call argument through")
    }));
}

#[test]
fn modern_indirect_call_assignment_keeps_pointer_save_when_callee_writes_pointer() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE ARRAY s BYTE FUNC ReadS(BYTE v) RETURN(s(0)) \
             PROC Main() p=$4000 p^=ReadS(1) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.contains(&opcode::PHA));
    assert!(output.bytes.contains(&opcode::PLA));
}

#[test]
fn modern_indirect_call_assignment_uses_alternate_pointer_for_indirect_arg() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE FUNC Id(BYTE v) RETURN(v) \
             PROC Main() p=$4000 p(1)=Id(p(0)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ABS,
            0x01,
        ]));
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
            && optimization
                .message
                .contains("prepared call argument through")
    }));
}

#[test]
fn modern_indirect_call_assignment_uses_alternate_pointer_for_complex_index_arg() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY s BYTE i BYTE FUNC Id(BYTE v) RETURN(v) \
             PROC Main() i=1 s(13-i)=Id(s(12-i)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ADDR.address(),
            opcode::CLC,
            opcode::LDA_ABS,
        ]));
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ABS,
        ]));
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
            && optimization
                .message
                .contains("prepared call argument through")
    }));
}

#[test]
fn modern_absolute_x_call_assignment_reloads_preserved_zero_page_index() {
    let output = generate_profile_source_with_origin(
        "BYTE i=$E0 BYTE ARRAY a(4) BYTE FUNC Id(BYTE v) RETURN(v) \
             PROC Main() i=1 a(i)=Id(a(0)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDX_ZP, 0xE0,])
    );
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization.message.contains("reloaded index")
    }));
}

#[test]
fn modern_absolute_x_call_assignment_reloads_preserved_absolute_index() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY a(4) BYTE i BYTE FUNC Id(BYTE v) RETURN(v) \
             PROC Main() i=1 a(i)=Id(a(0)) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::LDX_ABS,
            (0x3004u16).to_le_bytes()[0],
            (0x3004u16).to_le_bytes()[1],
        ]));
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization.message.contains("reloaded index from $3004")
    }));
}

#[test]
fn modern_profile_skips_redundant_zero_page_constant_store_after_known_call() {
    let output = generate_profile_source_with_origin(
            "BYTE z=$C0 \n;@actionc preserves $C0\nPROC Keep=*() [$60] PROC Main() z=7 Keep() z=7 RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_eq!(count_pair(&output.bytes, opcode::STA_ZP, 0xC0), 1);
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization
                .message
                .contains("skipped redundant store of #$07")
    }));
}

#[test]
fn modern_profile_stores_preserved_zero_page_constant_without_reload() {
    let output = generate_profile_source_with_origin(
            "BYTE z=$C0 BYTE out \n;@actionc preserves $C0\nPROC Keep=*() [$60] PROC Main() z=1 Keep() out=z RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_eq!(count_pair(&output.bytes, opcode::LDA_ZP, 0xC0), 0);
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization
                .message
                .contains("stored known zero-page value #$01 directly")
    }));
}

#[test]
fn modern_profile_logs_call_preserved_zero_page_facts() {
    let output = generate_profile_source_with_origin(
        "BYTE z=$C0 \n;@actionc preserves $C0\nPROC Keep=*() [$60] PROC Main() z=1 Keep() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::CallFactPreserved
            && optimization.bytes_saved == 0
            && optimization
                .message
                .contains("preserved 1 stable zero-page fact")
    }));
}

#[test]
fn modern_profile_uses_annotated_call_register_preservation() {
    let output = generate_profile_source_with_origin(
        "BYTE a,b ;@actionc preserves A\nPROC Keep=*() [$60] PROC Main() a=7 b=a Keep() b=a RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::CallFactPreserved
            && optimization.message.contains("preserved 1 register fact")
    }));
}

#[test]
fn modern_profile_honors_annotated_zero_page_clobber() {
    let output = generate_profile_source_with_origin(
            ";@actionc clobbers $C0\nPROC Touch=*() [$60] PROC Main() BYTE z=$C0 z=7 Touch() z=7 RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_eq!(count_pair(&output.bytes, opcode::STA_ZP, 0xC0), 2);
    assert!(!output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization
                .message
                .contains("skipped redundant store of #$07")
    }));
}

#[test]
fn modern_profile_resolves_symbolic_zero_page_clobber() {
    let output = generate_profile_source_with_origin(
            "BYTE z=$C0 \n;@actionc clobbers z\nPROC Touch=*() [$60] PROC Main() z=7 Touch() z=7 RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_eq!(count_pair(&output.bytes, opcode::STA_ZP, 0xC0), 2);
}

#[test]
fn modern_profile_resolves_symbolic_zero_page_preserve() {
    let output = generate_profile_source_with_origin(
        "BYTE z=$C0 \n;@actionc preserves z\nPROC Keep=*() [$60] PROC Main() z=7 Keep() z=7 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(count_pair(&output.bytes, opcode::STA_ZP, 0xC0), 1);
}

#[test]
fn modern_profile_logs_call_preserved_memory_facts() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p PROC Keep() RETURN PROC Main() p=$4000 p^=1 Keep() p^=2 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::CallFactPreserved
            && optimization.bytes_saved == 0
            && optimization.message.contains("stable memory fact")
    }));
}

#[test]
fn compatible_indirect_card_assignment_keeps_source_and_target_pointers_separate() {
    let output = generate_compatible_source_with_origin(
        "CARD POINTER p CARD ARRAY words(4) BYTE i PROC Main() p=$4000 i=1 words(i)=p^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::DEY,
            opcode::LDA_IZY,
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_indirect_self_word_negation_uses_second_pointer_and_low_temp() {
    let output = generate_compatible_source_with_origin(
        "INT POINTER ip=$4000 PROC Main() ip^=-ip^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_IMM,
            0x00,
            opcode::LDY_IMM,
            0x00,
            opcode::SBC_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::VALUE_TEMP.address(),
        ]));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_descriptor_array_copy_preserves_target_pointer() {
    let output = generate_compatible_source_with_origin(
            "CARD ARRAY gw(4) PROC Copy() CARD ARRAY lw(4) lw(0)=gw(0) RETURN PROC Main() Copy() RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::DEY,
            opcode::LDA_IZY,
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_descriptor_array_expression_preserves_target_pointer() {
    let output = generate_compatible_source_with_origin(
            "BYTE i CARD ARRAY gw(4) PROC Copy() CARD ARRAY lw(4) i=1 lw(i)=gw(i)+$0100 RETURN PROC Main() Copy() RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_IMM,
            0x00,
            opcode::ROL_A,
            opcode::PLP,
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::ADC_IMM,
            0x01,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
        ]));
}

#[test]
fn compatible_descriptor_array_add_scalar_uses_separate_source_pointer() {
    let output = generate_compatible_source_with_origin(
            "BYTE i CARD ARRAY words(4) PROC Copy(CARD ARRAY dst, src BYTE idx) dst(1)=src(0)+idx RETURN PROC Main() i=1 Copy(words,words,i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[2] == opcode::ADC_ABS
            && bytes[5] == opcode::STA_IZY
    }));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PHA,
        ]));
    assert!(!output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
        ]));
}

#[test]
fn compatible_descriptor_array_call_index_scales_byte_return_directly() {
    let output = generate_compatible_source_with_origin(
            "CARD ARRAY words(4) CARD w BYTE FUNC NextIndex(BYTE x) x==+1 RETURN(x) PROC Main() words(NextIndex(0))=w+$20 RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::JSR_ABS
            && bytes[3] == opcode::LDA_ZP
            && bytes[4] == runtime_zp::ARGS.address()
            && bytes[5] == opcode::ASL_A
            && bytes[6] == opcode::PHP
            && bytes[7] == opcode::CLC
    }));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ADDR.offset(1).address(),
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
        ]));
}

#[test]
fn compatible_pointer_byte_scalar_index_adds_index_directly() {
    let output = generate_compatible_source_with_origin(
        "BYTE POINTER p BYTE i,x PROC Main() p=$4000 i=1 p(i)=$BB x=p(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ADC_ABS,
            0x02,
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ADDR.offset(1).address(),
            opcode::LDA_ABS,
            0x02,
        ]));
}

#[test]
fn compatible_pointer_word_scalar_index_scales_index_directly() {
    let output = generate_compatible_source_with_origin(
        "CARD POINTER p BYTE i CARD x PROC Main() p=$4000 i=1 p(i)=$1234 x=p(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::ASL_A,
            opcode::PHP,
            opcode::CLC,
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ADDR.offset(1).address(),
            opcode::LDA_ABS,
            0x02,
        ]));
}

#[test]
fn compatible_indirect_byte_copy_preserves_target_pointer() {
    let output = generate_compatible_source_with_origin(
            "BYTE i BYTE ARRAY dst,src PROC Copy() i=1 dst=$4000 src=$5000 dst(i)=src(i) RETURN PROC Main() Copy() RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_indirect_byte_self_add_uses_separate_source_pointer() {
    let output = generate_compatible_source_with_origin(
        "BYTE POINTER p PROC Main() p=$4000 p^=p^+1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_IMM,
            0x01,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::PHA, opcode::PHA])
    );
}

#[test]
fn compatible_indirect_byte_lvalue_add_constant_uses_separate_source_pointer() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY ps PROC Main() ps(0)=ps(1)+1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_IMM,
            0x01,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::RTS,
        ]));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::PHA, opcode::PHA])
    );
}

#[test]
fn compatible_indirect_byte_lvalue_add_simple_uses_separate_source_pointer() {
    let output = generate_compatible_source_with_origin(
        "BYTE POINTER src,dst BYTE i PROC Main() src=$4000 dst=$4100 i=1 dst(i)=src^+i RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::DEY,
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_ABS,
            0x04,
            0x30,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_indirect_word_lvalue_add_lvalue_uses_three_pointer_pairs() {
    let output = generate_compatible_source_with_origin(
            "CARD POINTER src,dst CARD ARRAY words(4) BYTE i PROC Main() src=$4000 dst=$4100 i=1 dst(i)=src^+words(i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_IZY,
            runtime_zp::VALUE_TEMP.address(),
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
        ]));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_indirect_word_lvalue_sub_byte_lvalue_keeps_target_pointer() {
    let output = generate_compatible_source_with_origin(
            "INT POINTER dst INT ARRAY nums(4) BYTE POINTER bp BYTE i PROC Main() dst=$4000 bp=$4100 i=1 dst(i)=nums(i)-bp^ RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::SBC_IZY,
            runtime_zp::VALUE_TEMP.address(),
        ]));
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address(), opcode::DEY])
    );
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_indirect_byte_compound_add_lvalue_uses_direct_adc() {
    let output = generate_compatible_source_with_origin(
        "BYTE POINTER s,t PROC Main() s=$4000 t=$4100 s^==+t^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PHA,
        ]));
}

#[test]
fn modern_indirect_byte_compound_increment_updates_prepared_pointer_directly() {
    let output = generate_profile_source_with_origin(
        "BYTE x BYTE POINTER p PROC Main() p=@x p^==+1 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_IMM,
            0x01,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn modern_indirect_byte_compound_decrement_updates_prepared_pointer_directly() {
    let output = generate_profile_source_with_origin(
        "BYTE x BYTE POINTER p PROC Main() p=@x p^==-1 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::SBC_IMM,
            0x01,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn modern_pointer_deref_condition_preserves_prepared_pointer_for_update() {
    let output = generate_profile_source_with_origin(
        "BYTE x BYTE POINTER p PROC Main() p=@x IF p^#0 THEN p^==-1 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let low_load_preps = output
        .bytes
        .windows(5)
        .filter(|bytes| {
            *bytes
                == [
                    opcode::LDA_ABS,
                    0x01,
                    0x30,
                    opcode::STA_ZP,
                    runtime_zp::ARRAY_ADDR.address(),
                ]
        })
        .count();
    let direct_low_preps = output
        .bytes
        .windows(2)
        .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
        .count();
    assert_eq!(low_load_preps + direct_low_preps, 1);
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::SBC_IMM,
            0x01,
            opcode::STA_IZY,
        ]));
}

#[test]
fn modern_pointer_deref_nested_condition_reuses_prepared_simple_load() {
    let source = "BYTE FUNC F(CHAR POINTER s,t) s==+1 t==+1 IF s^#t^ THEN IF s^=': THEN RETURN(0) FI FI RETURN(1)";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(count_pair(&compatible.bytes, opcode::LDA_ABS, 0x00), 2);
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_ABS, 0x00), 1);
    assert!(
        modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address(),])
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
            && optimization
                .message
                .contains("reused prepared pointer $AE/$AF for simple load")
    }));
}

#[test]
fn modern_pointer_deref_compare_rhs_reuses_prepared_pointer() {
    let source = "BYTE x BYTE POINTER p PROC Main() p=@x IF p^ THEN IF x<p^ THEN x=1 FI FI RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!modern.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(
        modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::CMP_IZY, runtime_zp::ARRAY_ADDR.address(),])
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
            && optimization
                .message
                .contains("reused prepared pointer $AE/$AF for simple load")
    }));
}

#[test]
fn modern_record_pointer_fields_use_base_pointer_with_y_offsets() {
    let source = "TYPE Block=[CARD size,next] Block b Block POINTER p CARD a,c PROC Main() p=@b a=p.size c=p.next RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::ADC_IMM, 0x02])
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::ADC_IMM, 0x02])
    );
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDY_IMM, 0x03, opcode::LDA_IZY,])
    );
    assert_eq!(
        count_pair(
            &modern.bytes,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address()
        ),
        1
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
            && optimization.message.contains("record-base:P")
    }));
}

#[test]
fn modern_byte_compare_rhs_pointer_deref_prepares_before_left_load() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE a,out PROC Main() p=$4000 a=7 IF a=p^ THEN out=1 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let pointer_setup = output
        .bytes
        .windows(2)
        .position(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()])
        .unwrap();
    let compare = output
        .bytes
        .windows(2)
        .position(|bytes| bytes == [opcode::CMP_IZY, runtime_zp::ELEMENT_ADDR.address()])
        .unwrap();

    assert!(pointer_setup < compare);
    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::LDY_IMM,
            0x00,
            opcode::CMP_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
}

#[test]
fn modern_word_compare_rhs_pointer_deref_reuses_effective_address() {
    let output = generate_profile_source_with_origin(
        "CARD POINTER p CARD a BYTE out PROC Main() p=$4000 a=7 IF a=p^ THEN out=1 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::EOR_IZY, runtime_zp::ELEMENT_ADDR.address()])
            .count()
            >= 2
    );
    let low_pointer_stores = output
        .bytes
        .windows(2)
        .filter(|bytes| {
            *bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address()]
                || *bytes == [opcode::STY_ZP, runtime_zp::ELEMENT_ADDR.address()]
        })
        .count();
    assert_eq!(low_pointer_stores, 1);
}

#[test]
fn modern_byte_lvalue_plus_constant_materializes_directly() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER menu BYTE FUNC Key(BYTE POINTER menu) menu ==+ menu^ + 3 RETURN(menu^)",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| bytes
        == [
            opcode::CLC,
            opcode::LDY_IMM,
            0x00,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_IMM,
            0x03,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::CLC,
            opcode::ADC_IMM,
        ]));
}

#[test]
fn modern_pointer_setup_stores_known_x_directly_to_zero_page() {
    let output = generate_profile_source_with_origin(
        "BYTE FUNC Key(BYTE POINTER menu) menu ==+ menu^ + 3 RETURN(menu^)",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STX_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::CLC,
        ]));
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::TXA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
        ]));
}

#[test]
fn modern_pointer_setup_stores_known_accumulator_directly_to_zero_page() {
    let output = generate_profile_source_with_origin(
        "BYTE FUNC Key(BYTE POINTER menu) menu ==+ menu^ + 3 RETURN(menu^)",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(
        !output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x00, 0x30, opcode::LDA_ABS, 0x00, 0x30,])
    );
}

#[test]
fn modern_pointer_compound_return_reuses_prepared_updated_pointer() {
    let output = generate_profile_source_with_origin(
        "BYTE FUNC Key(BYTE POINTER menu) menu ==+ menu^ + 3 RETURN(menu^)",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_ABS,
        ]));
}

#[test]
fn compatible_byte_ordered_compare_with_indexed_operand_is_direct() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE n STRING s PROC Copy(STRING src) n=0 WHILE n<=src(0) DO n==+1 OD RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2] == opcode::CMP_ABS
            && bytes[5] == opcode::BCS_REL
    }));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ABS,
            0x02,
        ]));
}

#[test]
fn compatible_byte_greater_equal_constant_branches_from_left_operand_compare() {
    let output = generate_compatible_source_with_origin(
        "BYTE n,x PROC Main() IF n>=4 THEN x=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_IMM,
            0x04,
            opcode::BCS_REL,
            0x03,
        ]));
    assert!(
        !output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x04, opcode::CMP_ABS, 0x00, 0x30])
    );
}

#[test]
fn compatible_byte_indexed_greater_than_constant_preserves_constant_left_compare() {
    let output = generate_compatible_source_with_origin(
        "BYTE i,x BYTE ARRAY values=[0 64 128 191] PROC Main() i=3 IF values(i)>190 THEN x=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0xBE,
            opcode::LDY_IMM,
            0x00,
            opcode::CMP_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0xBE,
            opcode::CLC,
            opcode::LDA_ABS,
            0x04,
            0x30,
            opcode::ADC_ABS,
        ]));
}

#[test]
fn compatible_byte_indexed_bitwise_compound_updates_in_place() {
    let output = generate_compatible_source_with_origin(
        "BYTE i BYTE ARRAY data(4) PROC Main() i=1 data(i)==!$20 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::LDA_ABS_X
            && bytes[3] == opcode::EOR_IMM
            && bytes[4] == 0x20
            && bytes[5] == opcode::STA_ABS_X
    }));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
}

#[test]
fn compatible_byte_indexed_inc_dec_compound_updates_in_place() {
    let output = generate_compatible_source_with_origin(
        "BYTE i BYTE ARRAY data(4) PROC Main() i=1 data(i)==+1 data(i)==-1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::INC_ABS_X, 0x01, 0x30, opcode::LDX_ABS,])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::DEC_ABS_X, 0x01, 0x30])
    );
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
}

#[test]
fn compatible_indirect_byte_bitwise_compound_uses_indexed_rhs() {
    let output = generate_compatible_source_with_origin(
            "BYTE n BYTE ARRAY mask(4)=[$FC $F3 $CF $3F] BYTE ARRAY p CARD i PROC Main() p=$4000 i=1 n=2 p(i)==&mask(n) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2] == opcode::LDX_ABS
            && bytes[5] == opcode::AND_ABS_X
    }));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_byte_compound_bitwise_chain_is_left_associative() {
    let output = generate_compatible_source_with_origin(
        "BYTE old,mask,temp PROC Main() old==&mask%temp RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::LDA_ABS
            && bytes[3] == opcode::AND_ABS
            && bytes[6] == opcode::ORA_ABS
            && bytes[9] == opcode::STA_ABS
    }));
}

#[test]
fn compatible_indirect_byte_compound_bitwise_chain_preserves_target_pointer() {
    let output = generate_compatible_source_with_origin(
            "BYTE mask,i,y BYTE ARRAY p,pm PROC Main() p=$4000 pm=$5000 i=1 y=2 p(i+y)==&mask%pm(i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(14).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[1] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[2] == opcode::AND_ABS
            && bytes[5] == opcode::STA_ZP
            && bytes[6] == runtime_zp::VALUE_TEMP.address()
    }));
    assert!(output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::LDA_ZP
            && bytes[1] == runtime_zp::VALUE_TEMP.address()
            && bytes[2] == opcode::ORA_IZY
            && bytes[3] == runtime_zp::ARRAY_ADDR.address()
            && bytes[4] == opcode::STA_IZY
            && bytes[5] == runtime_zp::ELEMENT_ADDR.address()
    }));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_indirect_byte_bitwise_assignment_uses_separate_source_pointer() {
    let output = generate_compatible_source_with_origin(
            "BYTE POINTER src,dst BYTE i,mask PROC Main() src=$4000 dst=$4100 i=1 mask=$7F dst(i)=src^&mask RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::DEY,
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::AND_ABS,
            0x05,
            0x30,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_indirect_byte_bitwise_lvalue_assignment_uses_three_pointer_pairs() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY src,dst,other BYTE i PROC Main() src=$4000 dst=$4100 other=$4200 i=1 dst(i)=src(i)!other(i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::EOR_IZY,
            runtime_zp::VALUE_TEMP.address(),
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_indirect_word_bitwise_lvalue_assignment_uses_three_pointer_pairs() {
    let output = generate_compatible_source_with_origin(
            "CARD ARRAY src,dst,other BYTE i PROC Main() src=$4000 dst=$4100 other=$4200 i=1 dst(i)=src(i)%other(i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ORA_IZY,
            runtime_zp::VALUE_TEMP.address(),
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_inline_byte_array_same_index_copy_loads_index_once() {
    let output = generate_compatible_source_with_origin(
        "BYTE i BYTE ARRAY src(4) BYTE ARRAY dst(4) PROC Main() i=1 dst(i)=src(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::LDX_ABS
            && bytes[3] == opcode::LDA_ABS_X
            && bytes[6] == opcode::STA_ABS_X
    }));
    assert!(!output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::LDX_ABS
            && bytes[3] == opcode::LDX_ABS
            && bytes[1] == bytes[4]
            && bytes[2] == bytes[5]
    }));
}

#[test]
fn compatible_inline_byte_array_same_index_add_uses_absolute_x_source() {
    let output = generate_compatible_source_with_origin(
            "BYTE i BYTE ARRAY src(4) BYTE ARRAY dst(4) BYTE ARRAY text(0)=\"A\" PROC Main() i=1 dst(i)=src(i)+text(0) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::CLC
            && bytes[1] == opcode::LDA_ABS_X
            && bytes[4] == opcode::ADC_ABS
            && bytes[7] == opcode::STA_ABS_X
    }));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_scalar_plus_inline_byte_array_uses_absolute_x_rhs() {
    let output = generate_compatible_source_with_origin(
        "BYTE i,sum BYTE ARRAY temp(4) PROC Main() i=1 sum=2 sum=sum+temp(i) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(13).any(|bytes| {
        bytes[0] == opcode::CLC
            && bytes[1] == opcode::LDA_ABS
            && bytes[4] == opcode::LDX_ABS
            && bytes[7] == opcode::ADC_ABS_X
            && bytes[10] == opcode::STA_ABS
    }));
    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
}

#[test]
fn compatible_record_field_arithmetic_materializes_indirect_rhs() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag,flags] BYTE x,out Pair rec Pair POINTER p PROC Main() p=@rec x=5 out=x+p.tag RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::CLC,
        ]));
    assert!(output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_ABS,
        ]));
}

#[test]
fn compatible_record_value_assignment_to_record_pointer_stores_address() {
    let output = generate_compatible_source_with_origin(
        "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p PROC Main() p=rec RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x30,
            opcode::STA_ABS,
            0x04,
            0x30,
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ABS,
        ]));
    assert!(
        !output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ABS, 0x03, 0x30,])
    );
}

#[test]
fn compatible_record_pointer_field_copy_uses_separate_pointers() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag,flags] Pair left Pair right Pair POINTER p Pair POINTER q PROC Main() p=@left q=@right q.flags=p.tag RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PHA,
        ]));
}

#[test]
fn compatible_record_pointer_field_add_uses_separate_pointers() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag,flags] Pair rec Pair POINTER p BYTE out PROC Main() p=@rec out=p.tag+p.flags RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
}

#[test]
fn compatible_record_pointer_word_field_adds_directly_to_word_slot() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p CARD total PROC Main() p=@rec total=total+p.word RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_ABS,
            0x05,
        ]));
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x05, 0x30, opcode::INY,])
    );
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]));
}

#[test]
fn compatible_record_pointer_word_zero_branch_checks_high_byte() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD next] Pair rec Pair POINTER p BYTE out PROC Main() p=@rec IF p.next=0 THEN out=1 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ORA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]
        || bytes
            == [
                opcode::LDA_IZY,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::INY,
                opcode::ORA_IZY,
            ]));
}

#[test]
fn compatible_record_pointer_word_constant_compare_reuses_field_pointer() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p BYTE out PROC Main() p=@rec IF p.word>$0100 THEN out=1 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::CMP_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDA_IMM,
            0x01,
            opcode::INY,
            opcode::SBC_IZY,
        ]));
    assert!(!output.bytes.windows(14).any(|bytes| {
        bytes[0] == opcode::CMP_IZY
            && bytes[1] == runtime_zp::ARRAY_ADDR.address()
            && bytes[2..]
                .windows(3)
                .any(|inner| inner == [opcode::ADC_IMM, 0x02, opcode::STA_ZP])
    }));
}

#[test]
fn compatible_record_pointer_word_variable_compare_reuses_field_pointer() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p CARD limit BYTE out PROC Main() p=@rec IF p.word<limit THEN out=1 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::CMP_ABS
            && bytes[3] == opcode::INY
            && bytes[4] == opcode::LDA_IZY
            && bytes[5] == runtime_zp::ARRAY_ADDR.address()
            && bytes[6] == opcode::SBC_ABS
    }));
    assert!(!output.bytes.windows(14).any(|bytes| {
        bytes[0] == opcode::CMP_ABS
            && bytes[3..]
                .windows(3)
                .any(|inner| inner == [opcode::ADC_IMM, 0x01, opcode::STA_ZP])
    }));
}

#[test]
fn compatible_record_pointer_word_variable_equality_reuses_field_pointer() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p CARD limit BYTE out PROC Main() p=@rec IF p.word=limit THEN out=1 FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::EOR_ABS
            && bytes[3] == opcode::BNE_REL
            && bytes[5] == opcode::INY
            && bytes[6] == opcode::ORA_IZY
            && bytes[7] == runtime_zp::ARRAY_ADDR.address()
            && bytes[8] == opcode::EOR_ABS
    }));
    assert!(!output.bytes.windows(14).any(|bytes| {
        bytes[0] == opcode::EOR_ABS
            && bytes[3..]
                .windows(3)
                .any(|inner| inner == [opcode::ADC_IMM, 0x01, opcode::STA_ZP])
    }));
}

#[test]
fn compatible_record_pointer_word_equality_true_path_reuses_y_one() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[CARD size,next] PROC Main(Pair POINTER p,q CARD n) IF p.size=n THEN q.next=p.next FI RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ELEMENT_ADDR.offset(1).address()
            && bytes[2] == opcode::LDA_IZY
            && bytes[3] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[4] == opcode::STA_IZY
            && bytes[5] == runtime_zp::ARRAY_ADDR.address()
    }));
}

#[test]
fn compatible_materialized_word_equality_uses_original_eor_shape() {
    let output = generate_compatible_source_with_origin(
        "CARD a,b,c BYTE out PROC Main() IF a+b=c THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(11).any(|bytes| {
        bytes[0] == opcode::LDA_ZP
            && bytes[1] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[2] == opcode::EOR_ABS
            && bytes[5] == opcode::BNE_REL
            && bytes[7] == opcode::ORA_ZP
            && bytes[8] == runtime_zp::ELEMENT_ADDR.offset(1).address()
            && bytes[9] == opcode::EOR_ABS
    }));
}

#[test]
fn compatible_byte_array_word_index_compare_uses_two_indirect_pointers() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY data CARD i,j BYTE out PROC Main() IF data(i)>data(j) THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| {
        bytes
            == [
                opcode::LDA_IZY,
                runtime_zp::ELEMENT_ADDR.address(),
                opcode::CMP_IZY,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::BCC_REL,
                0x03,
            ]
    }));
    assert!(!output.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[2] == opcode::LDA_ZP
            && bytes[3] == runtime_zp::ELEMENT_ADDR.address()
    }));
}

#[test]
fn compatible_card_array_word_index_compare_uses_two_indirect_pointers() {
    let output = generate_compatible_source_with_origin(
        "CARD ARRAY data CARD i,j BYTE out PROC Main() IF data(i)>data(j) THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(11).any(|bytes| {
        bytes
            == [
                opcode::LDA_IZY,
                runtime_zp::ELEMENT_ADDR.address(),
                opcode::CMP_IZY,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::INY,
                opcode::LDA_IZY,
                runtime_zp::ELEMENT_ADDR.address(),
                opcode::SBC_IZY,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::BCC_REL,
                0x03,
            ]
    }));
}

#[test]
fn compatible_int_array_word_index_compare_uses_signed_indirect_branch() {
    let output = generate_compatible_source_with_origin(
        "INT ARRAY data CARD i,j BYTE out PROC Main() IF data(i)>data(j) THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(11).any(|bytes| {
        bytes
            == [
                opcode::LDA_IZY,
                runtime_zp::ELEMENT_ADDR.address(),
                opcode::CMP_IZY,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::INY,
                opcode::LDA_IZY,
                runtime_zp::ELEMENT_ADDR.address(),
                opcode::SBC_IZY,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::BMI_REL,
                0x03,
            ]
    }));
}

#[test]
fn compatible_signed_call_greater_than_zero_uses_reversed_zero_subtract() {
    let output = generate_compatible_source_with_origin(
        "INT FUNC Value() RETURN(1) BYTE out PROC Main() IF Value()>0 THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(10).any(|bytes| {
        bytes
            == [
                opcode::LDA_IMM,
                0x00,
                opcode::CMP_ZP,
                runtime_zp::ARGS.address(),
                opcode::LDA_IMM,
                0x00,
                opcode::SBC_ZP,
                runtime_zp::ARGS.offset(1).address(),
                opcode::BMI_REL,
                0x03,
            ]
    }));
}

#[test]
fn compatible_materialized_unsigned_word_compare_uses_subtract_chain() {
    let output = generate_compatible_source_with_origin(
        "CARD low,high BYTE out PROC Main() IF high+1>low+1 THEN out=1 FI RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(10).any(|bytes| {
        bytes[0] == opcode::LDA_ZP
            && bytes[2] == opcode::CMP_ZP
            && bytes[4] == opcode::LDA_ZP
            && bytes[6] == opcode::SBC_ZP
            && bytes[8] == opcode::BCC_REL
            && bytes[9] == 0x03
    }));
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::BNE_REL, 0x06])
    );
}

#[test]
fn compatible_record_pointer_word_compound_subtract_keeps_target_pointer() {
    let output = generate_compatible_source_with_origin(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p CARD n PROC Main() p=@rec p.word==-n RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(15).any(|bytes| {
        bytes[0] == opcode::SEC
            && bytes[1] == opcode::LDY_IMM
            && bytes[2] == 0x00
            && bytes[3] == opcode::LDA_IZY
            && bytes[4] == runtime_zp::ARRAY_ADDR.address()
            && bytes[5] == opcode::SBC_ABS
            && bytes[8] == opcode::STA_ZP
            && bytes[9] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[10] == opcode::INY
            && bytes[11] == opcode::LDA_IZY
            && bytes[12] == runtime_zp::ARRAY_ADDR.address()
            && bytes[13] == opcode::SBC_ABS
    }));
    assert!(!output.bytes.windows(6).any(|bytes| {
        bytes
            == [
                opcode::LDA_ZP,
                runtime_zp::ARRAY_ADDR.offset(1).address(),
                opcode::PHA,
                opcode::LDA_ZP,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::PHA,
            ]
    }));
}

#[test]
fn compatible_record_pointer_word_compound_add_indirect_rhs_keeps_target_pointer() {
    let output = generate_compatible_source_with_origin(
        "TYPE Pair=[CARD size,next] PROC Main(Pair POINTER p,q) p.size==+q.size RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::CLC
            && bytes[1] == opcode::LDY_IMM
            && bytes[2] == 0x00
            && bytes[3] == opcode::LDA_IZY
            && bytes[4] == runtime_zp::ARRAY_ADDR.address()
            && bytes[5] == opcode::ADC_IZY
            && bytes[6] == runtime_zp::ELEMENT_ADDR.address()
            && bytes[7] == opcode::STA_ZP
            && bytes[8] == runtime_zp::VALUE_TEMP.address()
            && bytes[9] == opcode::INY
            && bytes[10] == opcode::LDA_IZY
            && bytes[11] == runtime_zp::ARRAY_ADDR.address()
    }));
    assert!(!output.bytes.windows(6).any(|bytes| {
        bytes
            == [
                opcode::LDA_ZP,
                runtime_zp::ARRAY_ADDR.offset(1).address(),
                opcode::PHA,
                opcode::LDA_ZP,
                runtime_zp::ARRAY_ADDR.address(),
                opcode::PHA,
            ]
    }));
}

#[test]
fn modern_nested_call_argument_staging_preserves_earlier_args() {
    let output = generate_profile_source_with_origin(
            "BYTE got CARD gotw BYTE FUNC Inc(BYTE x) RETURN(x+1) CARD FUNC Pair(BYTE a,b) RETURN(a+(b LSH 8)) PROC Take(BYTE a CARD w BYTE b CARD c) got=a gotw=w RETURN PROC Main() Take(5,$1234,6,Pair(7,8)) RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::PHA)
            .count()
            >= 6
    );
    assert!(
        output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::PLA)
            .count()
            >= 6
    );
    assert!(output.bytes.windows(18).any(|bytes| bytes
        == [
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(5).address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(4).address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(3).address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(2).address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::PLA,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
        ]));
    assert!(output.bytes.windows(7).any(|bytes| matches!(
        bytes,
        [
            opcode::LDY_ZP,
            arg2,
            opcode::LDX_ZP,
            arg1,
            opcode::LDA_ZP,
            arg0,
            opcode::JSR_ABS | opcode::JMP_ABS,
        ] if *arg2 == runtime_zp::ARGS.offset(2).address()
            && *arg1 == runtime_zp::ARGS.offset(1).address()
            && *arg0 == runtime_zp::ARGS.address()
    )));
}

#[test]
fn compatible_staged_word_call_arguments_emit_left_to_right_without_nested_calls() {
    let output = generate_compatible_source_with_origin(
        "PROC Take(INT a,b) RETURN PROC Main(INT x,y,x1,y1) Take(x+x1,y+y1) RETURN",
        0x3000,
    )
    .unwrap();

    let arg_stores: Vec<u8> = output
        .bytes
        .windows(2)
        .filter_map(|bytes| {
            let target = bytes[1];
            (bytes[0] == opcode::STA_ZP
                && target >= runtime_zp::ARGS.address()
                && target <= runtime_zp::ARGS.offset(3).address())
            .then_some(target)
        })
        .collect();

    assert_eq!(
        arg_stores,
        vec![
            runtime_zp::ARGS.address(),
            runtime_zp::ARGS.offset(1).address(),
            runtime_zp::ARGS.offset(2).address(),
            runtime_zp::ARGS.offset(3).address(),
        ]
    );
}

#[test]
fn compatible_staged_call_defers_simple_byte_register_arg() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY data BYTE i BYTE FUNC Pair(BYTE a,b) RETURN(a+b) PROC Main() i=1 i=Pair(data(i),i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDX_ABS, 0x02, 0x30, opcode::LDA_ZP,])
    );
    assert!(!output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x02,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
        ]));
}

#[test]
fn compatible_staged_call_defers_low_byte_of_card_register_arg() {
    let output = generate_compatible_source_with_origin(
        "INT POINTER args BYTE ARRAY s CARD out \
             CARD FUNC F(INT n, CARD base, BYTE ARRAY text) RETURN(n) \
             PROC Main() out=F(-args^,10,s) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDY_IMM, 0x0A])
    );
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(3).address(),
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(2).address(),
            opcode::LDY_ZP,
            runtime_zp::ARGS.offset(2).address(),
        ]));
}

#[test]
fn compatible_staged_call_defers_string_pointer_register_arg() {
    let output = generate_compatible_source_with_origin(
            "DEFINE STRING=\"CHAR ARRAY\" BYTE mode PROC Open(BYTE channel, STRING name, BYTE aux1, aux2) RETURN PROC Main() Open(6,\"S:\",(mode&$F0)!$1C,mode) RETURN",
            0x3000,
        )
        .unwrap();

    let literal_offset = output
        .bytes
        .windows(3)
        .position(|bytes| bytes == [0x02, b'S', b':'])
        .unwrap();
    let literal_address = output.origin.wrapping_add(literal_offset as u16);

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            (literal_address >> 8) as u8,
            opcode::LDX_IMM,
            (literal_address & 0x00FF) as u8,
            opcode::LDA_IMM,
            0x06,
            opcode::JSR_ABS,
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::LDA_IMM,
            (literal_address >> 8) as u8,
        ]));
}

#[test]
fn compatible_constant_negative_word_call_arg_loads_registers_directly() {
    let output = generate_compatible_source_with_origin(
        "INT out INT FUNC NegI(INT x) RETURN(x) PROC Main() out=NegI(-3) out=NegI(1-4) RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(5)
            .filter(|bytes| *bytes
                == [
                    opcode::LDX_IMM,
                    0xFF,
                    opcode::LDA_IMM,
                    0xFD,
                    opcode::JSR_ABS,
                ])
            .count(),
        2
    );
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0xFF,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
        ]));
}

#[test]
fn compatible_pointer_deref_first_word_call_arg_stages_through_args() {
    let output = generate_compatible_source_with_origin(
        "CARD POINTER args BYTE ARRAY s CARD out \
             CARD FUNC F(CARD n, CARD base, BYTE ARRAY text) RETURN(n) \
             PROC Main() out=F(args^,10,s) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(18).any(|bytes| bytes
        == [
            opcode::LDY_IMM,
            0x01,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::DEY,
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(3).address(),
            opcode::LDA_ABS,
            0x03,
            0x30,
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDX_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
        ]));
}

#[test]
fn compatible_byte_shaped_shift_zero_extends_when_returning_card() {
    let output = generate_compatible_source_with_origin(
        "CARD FUNC Pair(BYTE lo,hi) RETURN(lo+(hi LSH 8))",
        0x3000,
    )
    .unwrap();

    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_LSH.low(),
            runtime_helper::CARTRIDGE_LSH.high(),
        ]));
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ARG0.offset(1).address(),
        ]));
}

#[test]
fn compatible_byte_constant_shift_to_byte_inlines_accumulator_shift() {
    let output = generate_compatible_source_with_origin(
        "BYTE hue,out PROC Main() out=hue LSH 4 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::ASL_A,
            opcode::STA_ABS,
        ]));
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_LSH.low(),
            runtime_helper::CARTRIDGE_LSH.high(),
        ]));
}

#[test]
fn modern_byte_constant_shift_materializes_complex_left_operand() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY my(4)=[0 0 0 0] BYTE m,out PROC Main() out=(my(m)-30) RSH 1 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LSR_A, opcode::STA_ABS, 0x05, 0x30,])
    );
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_RSH.low(),
            runtime_helper::CARTRIDGE_RSH.high(),
        ]));
}

#[test]
fn compatible_in_place_card_shift_by_constant_uses_memory_shift() {
    let output =
        generate_compatible_source_with_origin("CARD w PROC Main() w=w RSH 1 RETURN", 0x3000)
            .unwrap();

    assert!(
        output
            .bytes
            .windows(6)
            .any(|bytes| bytes == [opcode::LSR_ABS, 0x01, 0x30, opcode::ROR_ABS, 0x00, 0x30,])
    );
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_RSH.low(),
            runtime_helper::CARTRIDGE_RSH.high(),
        ]));
}

#[test]
fn modern_in_place_byte_shift_by_constant_uses_memory_shift() {
    let source = "BYTE gap PROC Main() gap==RSH 1 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LSR_A,
            opcode::STA_ABS,
            0x00,
            0x30,
        ]));
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LSR_ABS, 0x00, 0x30])
    );
    assert!(!modern.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LSR_A,
            opcode::STA_ABS,
            0x00,
            0x30,
        ]));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization
                .message
                .contains("shifted absolute byte in memory")
    }));
}

#[test]
fn modern_profile_branches_from_recently_stored_accumulator_value() {
    let source = "BYTE src,gap,out PROC Main() gap=src RSH 1 IF gap THEN out=1 FI RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(10).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LSR_A,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
        ]));
    assert!(modern.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LSR_A,
            opcode::STA_ABS,
            0x01,
            0x30,
        ]));
    assert!(
        !modern
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30])
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization
                .message
                .contains("suppressed accumulator reload from known memory alias")
    }));
}

#[test]
fn modern_profile_reuses_untracked_zero_page_materialization_chain() {
    let source = "BYTE src BYTE t=$E0 BYTE u=$AC PROC Main() t=src u=t RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xE0, opcode::LDA_ZP, 0xE0])
    );
    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ZP])
    );
    assert!(
        modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xAC])
    );
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_ZP, 0xE0), 0);
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization
                .message
                .contains("stored accumulator directly instead of reloading slot copy source")
    }));
}

#[test]
fn compatible_folded_zero_add_does_not_materialize_temp() {
    let output = generate_compatible_source_with_origin(
        "CARD FUNC Pair(BYTE lo,hi) RETURN(lo+(hi LSH 8))",
        0x3000,
    )
    .unwrap();

    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ELEMENT_ADDR.address(),])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ZP,])
    );
}

#[test]
fn compatible_indexed_pointer_assignment_keeps_source_and_target_pointers_separate() {
    let output = generate_compatible_source_with_origin(
        "CARD POINTER dst,src PROC Main() dst=$4000 src=$5000 dst(1)=src(1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::DEY,
            opcode::LDA_IZY,
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
}

#[test]
fn compatible_indexed_pointer_assignment_uses_separate_source_pointer_for_deref_rhs() {
    let output = generate_compatible_source_with_origin(
        "CARD POINTER q PROC Main() q=$4000 q(1)=q^+1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::LDA_IZY,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_IMM,
            0x01,
            opcode::STA_IZY,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::INY,
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::PHA,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::PHA,
        ]));
}

#[test]
fn compatible_absolute_x_assignment_preserves_index_across_call_rhs() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY src(3) BYTE ARRAY dst(4) BYTE i BYTE FUNC At(BYTE ARRAY s, BYTE n) RETURN(s(n)) PROC Main() src(1)=7 i=1 dst(i)=At(src,i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes[0] == opcode::LDA_ABS && bytes[3] == opcode::PHA)
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::TXA, opcode::PHA])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::PLA, opcode::TAX, opcode::LDA_ZP])
    );
    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ABS_X,
            0x03,
        ]));
}

#[test]
fn compatible_absolute_x_call_assignment_pushes_scalar_index_directly() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY src(3) BYTE ARRAY dst(4) BYTE i BYTE FUNC At(BYTE ARRAY s, BYTE n) RETURN(s(n)) PROC Main() i=1 dst(i)=At(src,i+1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes[0] == opcode::LDA_ABS && bytes[3] == opcode::PHA)
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::TXA, opcode::PHA])
    );
}

#[test]
fn compatible_inline_byte_array_call_index_load_uses_return_as_x() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY gb(4) BYTE i,out BYTE FUNC NextIndex(BYTE x) x==+1 RETURN(x) PROC Main() i=1 out=gb(NextIndex(i)) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            0x07,
            0x30,
            opcode::LDX_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_ABS_X,
            0x00,
        ]));
    assert!(!output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ADDR.address(),
        ]));
}

#[test]
fn compatible_inline_byte_array_call_index_store_keeps_return_until_store() {
    let output = generate_compatible_source_with_origin(
            "BYTE ARRAY gb(4) BYTE ARRAY lb(4) BYTE i BYTE FUNC NextIndex(BYTE x) x==+1 RETURN(x) PROC Main() i=1 gb(NextIndex(i))=lb(i) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::JSR_ABS
            && bytes[3] == opcode::LDX_ABS
            && bytes[6] == opcode::LDA_ABS_X
            && bytes[7] == 0x04
    }));
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDX_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ABS_X,
            0x00,
            0x30,
            opcode::RTS,
        ]));
}

#[test]
fn compatible_dynamic_array_indexes_accept_expressions() {
    let output = generate_compatible_source_with_origin(
            "BYTE i,b CARD c INT n BYTE ARRAY ba=[1 2 3 4] CARD ARRAY ca=[10 20 30 40] INT ARRAY ia(4) PROC Main() i=1 ba(i+1)=7 b=ba(i+1) ca(i+1)=$1234 c=ca(i+1) ia(i+1)=-3 n=ia(i+1) RETURN",
            0x3000,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::ADC_IMM,
            0x01,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address()
        ]));
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ELEMENT_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ELEMENT_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::ASL_A, opcode::PHP, opcode::CLC, opcode::ADC_ABS,])
    );
}

#[test]
fn compatible_indexed_byte_plus_negative_constant_keeps_constant_inline() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY sdir=[1 1 1 1 1 1 1 1 1 1 2 2 2 0 2 1 1 1 0 2 0 0 0 1 1 1 1 2 1 0 1 1] INT ARRAY xd(4) BYTE stk PROC Main() stk=15 xd(0)=sdir(stk*2)+-1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_IMM,
            0xFF,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
        ]));
    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::ADC_IMM,
            0xFF,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
        ]));
    assert!(!output.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::ADC_ZP,
        ]));
}

#[test]
fn generates_pointer_address_assignment_and_deref() {
    let output = generate_source(
        "BYTE ARRAY data(4) BYTE POINTER p BYTE x PROC Main() p=@data p^=9 x=p^ RETURN",
    )
    .unwrap();
    assert_eq!(
        output.bytes,
        vec![
            0xA9,
            0x00,
            0x8D,
            0x04,
            0x06,
            0xA9,
            0x06,
            0x8D,
            0x05,
            0x06,
            0xAD,
            0x04,
            0x06,
            0x85,
            0xC0,
            0xAD,
            0x05,
            0x06,
            0x85,
            0xC1,
            0xA9,
            0x09,
            0xA0,
            0x00,
            0x91,
            0xC0,
            0xAD,
            0x04,
            0x06,
            0x85,
            0xC0,
            0xAD,
            0x05,
            0x06,
            0x85,
            0xC1,
            0xA0,
            0x00,
            0xB1,
            0xC0,
            0x8D,
            0x06,
            0x06,
            opcode::RTS,
        ]
    );
}

#[test]
fn generates_array_name_as_pointer_value() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY data(4) BYTE POINTER p BYTE x PROC Main() p=data p^=$11 x=p^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x00, opcode::STA_ABS])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x30, opcode::STA_ABS])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_array_name_can_be_dereferenced_as_pointer() {
    let output = generate_compatible_source_with_origin(
        "CARD ARRAY data CARD x PROC Main() data=$4000 data^=$1234 x=data^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_pointer_deref_uses_action_pointer_temp() {
    let output = generate_compatible_source_with_origin(
        "BYTE ARRAY data(4) BYTE POINTER p BYTE x PROC Main() p=data p^=$11 x=p^ RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_zero_page_pointer_deref_uses_pointer_directly() {
    let output = generate_compatible_source_with_origin(
        "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 \
             BYTE POINTER screen CARD POINTER allocp SET $491=$3000 SET $E=$3000 \
             CARD FUNC Main=*() screen^=$11 allocp^=$1234 allocp==-2 RETURN(allocp^)",
        0x3000,
    )
    .unwrap();

    assert_eq!(output.run_address, 0x3002);
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, 0xE6])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_IZY, 0xE8])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_IZY, 0xE8])
    );
    assert!(
        !output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
    );
}

#[test]
fn compatible_scalar_initializer_expression_does_not_hide_following_pointer_decl() {
    generate_compatible_source_with_origin(
            "BYTE POINTER screen BYTE scl=screen, sch=screen+1 CARD POINTER allocp MODULE CARD FUNC Alloc(CARD n) allocp==+n RETURN(allocp-n)",
            0x3000,
        )
        .unwrap();
}

#[test]
fn compatible_current_location_routine_binds_machine_block_label() {
    let output = generate_compatible_source_with_origin(
        "PROC Helper=*() [$60] PROC Main() [$20 Helper] RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(output.bytes[0], opcode::RTS);
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x30])
    );
}

#[test]
fn modern_symbolic_local_array_initializer_points_at_current_location_routine() {
    let output = generate_profile_source_with_origin(
        "PROC Jmp=*() [$34 $12] PROC Make() CARD ARRAY adr(1)=Jmp CARD go go=adr(0) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0x00, 0x30, 0x00, 0x30]),
        "expected local array descriptor to point at Jmp table; bytes={:02X?}",
        output.bytes
    );
}

#[test]
fn compatible_current_location_routine_params_do_not_allocate_storage() {
    let output = generate_compatible_source_with_origin(
        "BYTE zx=$FF BYTE zy=$FE PROC Position=*(BYTE x,y) [$85 zx $86 zy $E6 zy $60]",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        output.bytes,
        vec![
            opcode::STA_ZP,
            0xFF,
            opcode::STX_ZP,
            0xFE,
            opcode::INC_ZP,
            0xFE,
            opcode::RTS,
            opcode::RTS,
        ]
    );
}

#[test]
fn compatible_zero_page_byte_compound_assign_uses_inc_and_subtract_dec() {
    let output = generate_compatible_source_with_origin(
        "BYTE kx=$FF, ky=$FE PROC Main() kx==+1 ky==-1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::INC_ZP, 0xFF])
    );
    assert!(output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_ZP,
            0xFE,
            opcode::SBC_IMM,
            0x01,
            opcode::STA_ZP,
            0xFE
        ]));
}

#[test]
fn modern_zero_page_byte_compound_subtract_one_uses_dec() {
    let output = generate_profile_source_with_origin(
        "BYTE kx=$FF, ky=$FE PROC Main() kx==+1 ky==-1 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::INC_ZP, 0xFF])
    );
    assert!(
        output
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::DEC_ZP, 0xFE])
    );
    assert!(!output.bytes.windows(7).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_ZP,
            0xFE,
            opcode::SBC_IMM,
            0x01,
            opcode::STA_ZP,
            0xFE
        ]));
}

#[test]
fn compatible_zero_page_word_compound_increment_uses_inc_carry() {
    let output = generate_compatible_source_with_origin(
        "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen \
             SET $491=$3000 SET $E=$3000 PROC Main() screen==+1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::INC_ZP,
            0xE6,
            opcode::BNE_REL,
            0x02,
            opcode::INC_ZP,
            0xE7,
        ]));
}

#[test]
fn compatible_array_pointer_compound_increment_uses_inc_carry() {
    let output =
        generate_compatible_source_with_origin("BYTE ARRAY p PROC Main() p==+1 RETURN", 0x3000)
            .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::INC_ABS,
            0x00,
            0x30,
            opcode::BNE_REL,
            0x03,
            opcode::INC_ABS,
            0x01,
            0x30,
        ]));
}

#[test]
fn compatible_array_pointer_compound_decrement_preserves_high_byte() {
    let output =
        generate_compatible_source_with_origin("PROC Dec(BYTE ARRAY s) s==-1 RETURN", 0x3000)
            .unwrap();

    assert!(output.bytes.windows(15).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::SEC,
            opcode::SBC_IMM,
            0x01,
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::SBC_IMM,
            0x00,
            opcode::STA_ABS,
        ]));

    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::RTS,
        ]));
}

#[test]
fn compatible_word_inc_peephole_preserves_y_for_following_one_store() {
    let output = generate_compatible_source_with_origin(
        "BYTE i=$E0 BYTE ARRAY p CARD ARRAY words(1) \
             PROC Main() p(0)=0 words(0)=p p==+1 i=1 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::INY, opcode::STY_ZP, 0xE0])
    );
}

#[test]
fn compatible_runtime_multiply_materializes_byte_exprs_as_bytes() {
    let output = generate_compatible_source_with_origin(
        "BYTE zdx=$5C,zdy=$5D CARD buf PROC Main() buf=(zdx+1)*(zdy+1) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::ADC_IMM, 0x00, opcode::STA_ZP])
    );
    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::LDX_IMM,
            0x00,
            opcode::JSR_ABS,
        ]));
}

#[test]
fn compatible_call_argument_subtracts_materialized_runtime_multiply_rhs() {
    let output = generate_compatible_source_with_origin(
        "BYTE r,y PROC Main() r=Rand(30) y=Rand(160-(2*r)) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(5).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_MUL.low(),
            runtime_helper::CARTRIDGE_MUL.high(),
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
        ]));
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::SEC,
            opcode::LDA_IMM,
            160,
            opcode::SBC_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_ZP,
        ]));
}

#[test]
fn compatible_compound_assignment_accepts_runtime_call_rhs() {
    let output = generate_compatible_source_with_origin(
        "CARD scrn PROC Main() scrn=Peek(88) scrn==+Peek(89)*256 RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(count_jsr_to(&output.bytes, 0xA767), 2);
}

#[test]
fn compatible_runtime_byte_div_mod_loads_low_before_zero_high() {
    let output = generate_compatible_source_with_origin(
        "BYTE b,i PROC Main() i=b/10 b==mod 10 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LDX_IMM,
            0x00,
            opcode::JSR_ABS,
        ]));
    assert!(!output.bytes.windows(6).any(|bytes| bytes
        == [
            opcode::LDA_IMM,
            0x00,
            opcode::TAX,
            opcode::LDA_ABS,
            0x00,
            0x30,
        ]));
}

#[test]
fn compatible_runtime_multiply_add_sub_byte_digit_accum_uses_word_temps() {
    let output = generate_compatible_source_with_origin(
        "BYTE width,c PROC Main() width=10*width+c-48 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(26).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            runtime_helper::CARTRIDGE_MUL.low(),
            runtime_helper::CARTRIDGE_MUL.high(),
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::TXA,
            opcode::STA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::CLC,
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.address(),
            opcode::ADC_ABS,
            0x01,
            0x30,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::LDA_ZP,
            runtime_zp::ARRAY_ADDR.offset(1).address(),
            opcode::ADC_IMM,
            0x00,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::SEC,
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
            opcode::SBC_IMM,
        ]));
}

#[test]
fn compatible_runtime_multiply_add_sub_word_digit_accum_subtracts_high_byte() {
    let output = generate_compatible_source_with_origin(
        "INT prcisn BYTE c PROC Main() prcisn=10*prcisn+c-48 RETURN",
        0x3000,
    )
    .unwrap();

    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::LDA_ZP,
            runtime_zp::ELEMENT_ADDR.offset(1).address(),
            opcode::SBC_IMM,
            0x00,
            opcode::STA_ABS,
            0x01,
            0x30,
            opcode::RTS,
        ]));
}

#[test]
fn compatible_machine_block_menu_data_emits_strings_chars_and_defines() {
    let output = generate_compatible_source_with_origin(
        "DEFINE nil=\"0\" PROC Menu=*() [\"Yes\" 'Y nil]",
        0x3000,
    )
    .unwrap();

    assert_eq!(
        &output.bytes[..9],
        &[3, b'Y', b'e', b's', 0x00, 0x30, b'Y', 0x9A, 0]
    );
}

#[test]
fn compatible_current_location_routine_can_be_pointer_argument() {
    let output = generate_compatible_source_with_origin(
        "PROC Menu=*() [$00] PROC Take(CARD p) RETURN PROC Main() Take(Menu) RETURN",
        0x3000,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x30, opcode::TAX, opcode::LDA_IMM, 0x00])
    );
}

#[test]
fn compatible_arithmetic_can_subtract_array_pointer_value() {
    generate_compatible_source_with_origin(
        "BYTE ARRAY buffer CARD len, memtop=$2E5 PROC Main() len=memtop-buffer RETURN",
        0x3000,
    )
    .unwrap();
}

#[test]
fn rejects_bare_array_assignment_without_index() {
    let err = generate_source("BYTE ARRAY data(4) PROC Main() data=1 RETURN").unwrap_err();
    assert!(err[0].message.contains("assignment targets"));
}

#[test]
fn compat_profile_matches_compatible_entry_point() {
    let source = "BYTE a PROC Main() a=1 RETURN";
    let compatible = generate_compatible_source_with_origin(source, 0x3000).unwrap();
    let profiled =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();

    assert_eq!(profiled.origin, compatible.origin);
    assert_eq!(profiled.bytes, compatible.bytes);
    assert_eq!(profiled.skipped_ranges, compatible.skipped_ranges);
}

#[test]
fn codegen_output_includes_structured_map() {
    let output = generate_source_with_origin(
        "BYTE a CARD b PROC Main(BYTE p) BYTE l l=p a=l RETURN",
        0x3000,
    )
    .unwrap();

    assert_eq!(output.map.origin, output.origin);
    assert_eq!(output.map.run_address, output.run_address);
    assert_eq!(output.map.skipped_ranges, output.skipped_ranges);
    assert_eq!(output.map.routine_addresses, output.routine_addresses);
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::Routine
            && range.name.as_deref() == Some("Main")
            && range.start == output.run_address
            && range.end > range.start
    }));
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::Statement
            && range.name.as_deref() == Some("assignment")
            && range.end > range.start
    }));
    assert!(
        output
            .map
            .routine_addresses
            .iter()
            .any(|routine| routine.name == "Main" && routine.address == output.run_address)
    );
    assert!(
        output
            .map
            .routine_ranges
            .iter()
            .any(|routine| routine.name == "Main"
                && routine.start == output.run_address
                && routine.end > routine.start)
    );
    assert!(
        output
            .map
            .storage_symbols
            .windows(2)
            .all(|symbols| symbols[0].name <= symbols[1].name)
    );
    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "A"
            && symbol.scope == CodegenSymbolScope::Global
            && symbol.kind == CodegenSymbolKind::Storage
            && symbol.address == 0x0600
            && symbol.size == 1
            && symbol.address_space == CodegenAddressSpace::Absolute
    }));
    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "B"
            && symbol.scope == CodegenSymbolScope::Global
            && symbol.kind == CodegenSymbolKind::Storage
            && symbol.address == 0x0601
            && symbol.size == 2
            && symbol.address_space == CodegenAddressSpace::Absolute
    }));
    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "P"
            && symbol.scope == CodegenSymbolScope::Routine("Main".to_string())
            && symbol.kind == CodegenSymbolKind::Parameter
            && symbol.size == 1
    }));
    assert!(output.map.storage_symbols.iter().any(|symbol| {
        symbol.name == "L"
            && symbol.scope == CodegenSymbolScope::Routine("Main".to_string())
            && symbol.kind == CodegenSymbolKind::Local
            && symbol.size == 1
    }));
}

#[test]
fn compatible_map_includes_declaration_storage_source_ranges() {
    let output =
        generate_compatible_source_with_origin("BYTE a PROC Main() BYTE l a=l RETURN", 0x3000)
            .unwrap();

    assert!(
        output.map.source_ranges.iter().any(|range| {
            range.kind == CodegenSourceRangeKind::Declaration
                && range.name.as_deref() == Some("a")
                && range.start == 0x3000
                && range.end > range.start
        }),
        "{:?}",
        output.map.source_ranges
    );
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::StorageInitializer
            && range.name.as_deref() == Some("a")
            && range.start == 0x3000
            && range.end > range.start
    }));
    assert!(output.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::StorageInitializer
            && range.name.as_deref() == Some("Main storage")
            && range.end > range.start
    }));
}

#[test]
fn modern_profile_suppresses_redundant_lda_immediates() {
    let source = "BYTE a,b PROC Main() a=2 b=2 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(modern.origin, compatible.origin);
    assert_eq!(modern.skipped_ranges, compatible.skipped_ranges);
    assert_eq!(count_pair(&compatible.bytes, opcode::LDA_IMM, 2), 2);
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_IMM, 2), 1);
}

#[test]
fn modern_profile_does_not_suppress_lda_immediate_across_call() {
    let source = "BYTE a PROC Touch() RETURN PROC Main() a=2 Touch() a=2 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(modern.origin, compatible.origin);
    assert_eq!(modern.skipped_ranges, compatible.skipped_ranges);
    assert_eq!(count_pair(&compatible.bytes, opcode::LDA_IMM, 2), 2);
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_IMM, 2), 2);
}

#[test]
fn modern_profile_does_not_suppress_lda_immediate_across_label_join() {
    let source = "BYTE a,b PROC Main() a=2 IF b THEN FI a=2 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(modern.origin, compatible.origin);
    assert_eq!(modern.skipped_ranges, compatible.skipped_ranges);
    assert_eq!(count_pair(&compatible.bytes, opcode::LDA_IMM, 2), 2);
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_IMM, 2), 2);
}

#[test]
fn modern_profile_reuses_zero_page_memory_aliases_and_immediates() {
    let source = "BYTE a=$A0,b PROC Main() a=$44 b=a b=$44 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(modern.origin, compatible.origin);
    assert_eq!(modern.skipped_ranges, compatible.skipped_ranges);
    assert_eq!(count_pair(&compatible.bytes, opcode::LDA_IMM, 0x44), 2);
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_IMM, 0x44), 1);
    assert_eq!(count_pair(&compatible.bytes, opcode::LDA_ZP, 0xA0), 1);
    assert_eq!(count_pair(&modern.bytes, opcode::LDA_ZP, 0xA0), 0);
    assert_eq!(compatible.bytes.len(), modern.bytes.len() + 8);
}

#[test]
fn modern_profile_keeps_carry_only_arithmetic_compatible() {
    let source = "CARD a,b PROC Main() a==+b a==-1 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(modern.origin, compatible.origin);
    assert_eq!(modern.skipped_ranges, compatible.skipped_ranges);
    let mut compatible_without_trampoline = compatible.bytes.clone();
    compatible_without_trampoline.drain(4..7);
    compatible_without_trampoline.pop();
    assert_eq!(modern.bytes, compatible_without_trampoline);
}

#[test]
fn modern_profile_elides_empty_routine_trampoline() {
    let source = "BYTE a,b,c,d PROC Main() a=1 b=2 c=1 d=0 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(&compatible.bytes[4..7], &[opcode::JMP_ABS, 0x07, 0x30]);
    assert_eq!(&modern.bytes[4..6], &[opcode::LDY_IMM, 0x01]);
    assert_eq!(modern.run_address, 0x3004);
    assert_eq!(modern.bytes.len() + 4, compatible.bytes.len());
    assert_eq!(modern.bytes.last(), Some(&opcode::RTS));
}

#[test]
fn modern_profile_elides_entry_trampoline_after_explicit_routine_storage() {
    let output = generate_profile_source_with_origin(
        "BYTE g PROC Copy(BYTE value) BYTE local local=value g=local RETURN PROC Main() Copy(1) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let copy = routine_address(&output, "Copy").expect("Copy address");
    let scope = CodegenSymbolScope::Routine("Copy".to_string());
    let value = storage_symbol(&output, scope.clone(), "VALUE");
    let local = storage_symbol(&output, scope, "LOCAL");
    assert!(value.address < copy);
    assert!(local.address < copy);
    assert_eq!(
        &output.bytes[usize::from(copy.wrapping_sub(output.origin))..][..3],
        &[
            opcode::STA_ABS,
            Absolute::new(value.address).low(),
            Absolute::new(value.address).high(),
        ],
        "the stable routine entry should bind directly to the parameter prologue"
    );
    assert!(output.optimizations.iter().any(|optimization| {
        optimization.routine.as_deref() == Some("Copy")
            && optimization.kind == CodegenOptimizationKind::TrampolineElided
            && optimization.bytes_saved == 3
    }));
}

#[test]
fn modern_main_run_address_is_direct_entry_after_routine_storage() {
    let output = generate_profile_source_with_origin(
        "BYTE g PROC Main(BYTE value) BYTE local local=value g=local RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let main = routine_address(&output, "Main").expect("Main address");
    let scope = CodegenSymbolScope::Routine("Main".to_string());
    let value = storage_symbol(&output, scope.clone(), "VALUE");
    let local = storage_symbol(&output, scope, "LOCAL");
    assert_eq!(output.run_address, main);
    assert!(value.address < main);
    assert!(local.address < main);
    assert_eq!(
        &output.bytes[usize::from(main.wrapping_sub(output.origin))..][..3],
        &[
            opcode::STA_ABS,
            Absolute::new(value.address).low(),
            Absolute::new(value.address).high(),
        ]
    );
    assert!(
        output.map.routine_ranges.iter().any(|routine| {
            routine.name == "Main" && routine.start < main && routine.end > main
        })
    );
}

#[test]
fn modern_profile_rejects_routine_assignment_retargeting() {
    let err = generate_profile_source_with_origin(
        "PROC A() RETURN PROC T() PROC Main() T=A T() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap_err();

    assert!(
        err.iter()
            .any(|diagnostic| diagnostic.message.contains("cannot assign to routine name"))
    );
}

#[test]
fn modern_profile_keeps_program_rts_for_final_bodyless_routine() {
    let output =
        generate_profile_source_with_origin("PROC Main()", 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(output.bytes, vec![opcode::RTS]);
}

#[test]
fn modern_extension_call_arguments_evaluate_left_to_right() {
    let output = generate_profile_source_with_origin(
            "BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) BYTE FUNC H() RETURN(3) PROC Take(BYTE a,b,c) RETURN PROC Main() Take(F(),G(),H()) RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G", "H", "Take"]);
}

#[test]
fn modern_extension_arithmetic_operands_evaluate_left_to_right() {
    let output = generate_profile_source_with_origin(
        "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() out=F()+G() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G"]);
}

#[test]
fn modern_extension_comparison_operands_evaluate_left_to_right() {
    let output = generate_profile_source_with_origin(
            "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() IF F()=G() THEN out=1 FI RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G"]);
}

#[test]
fn modern_extension_indexed_assignment_evaluates_target_before_rhs() {
    let output = generate_profile_source_with_origin(
            "BYTE ARRAY a(4) BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() a(F())=G() RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G"]);
}

#[test]
fn modern_extension_bitwise_conditions_evaluate_left_to_right() {
    let output = generate_profile_source_with_origin(
            "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() IF F() AND G() THEN out=1 FI RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G"]);
}

#[test]
fn modern_extension_logical_conditions_evaluate_left_to_right() {
    let output = generate_profile_source_with_origin(
            "BYTE out BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() IF (F()=1) AND (G()=2) THEN out=1 FI IF (F()=1) OR (G()=2) THEN out=2 FI RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G", "F", "G"]);
}

#[test]
fn modern_extension_loop_conditions_evaluate_left_to_right() {
    let output = generate_profile_source_with_origin(
            "BYTE FUNC F() RETURN(1) BYTE FUNC G() RETURN(2) PROC Main() WHILE F()=G() DO EXIT OD DO EXIT UNTIL F()=G() OD RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_call_or_tail_jump_order(&output, &["F", "G", "F", "G"]);
}

#[test]
fn compare_branch_guard_accepts_expected_flag_consumers() {
    debug_assert_compare_branch_opcode(opcode::BEQ_REL, CompareBranchFlags::Equality);
    debug_assert_compare_branch_opcode(opcode::BNE_REL, CompareBranchFlags::Equality);
    debug_assert_compare_branch_opcode(opcode::BCC_REL, CompareBranchFlags::UnsignedOrder);
    debug_assert_compare_branch_opcode(opcode::BCS_REL, CompareBranchFlags::UnsignedOrder);
    debug_assert_compare_branch_opcode(opcode::BMI_REL, CompareBranchFlags::SignedOrder);
    debug_assert_compare_branch_opcode(opcode::BPL_REL, CompareBranchFlags::SignedOrder);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not match compare flag source")]
fn compare_branch_guard_rejects_signed_branch_from_unsigned_compare() {
    debug_assert_compare_branch_opcode(opcode::BMI_REL, CompareBranchFlags::UnsignedOrder);
}

#[test]
fn call_abi_guard_accepts_packed_args_and_return_slot() {
    let info = RoutineInfo {
        label: "routine:F".to_string(),
        params: vec![
            StorageSlot::zero_page(runtime_zp::ARGS.address(), 1),
            StorageSlot::zero_page(runtime_zp::ARGS.offset(1).address(), 2),
        ],
        return_slot: Some(StorageSlot::zero_page(runtime_zp::ARGS.address(), 2)),
        system_address: None,
        facts: RoutineFacts::default(),
        effects: RoutineEffects::unknown(),
    };

    debug_assert_call_abi_shape("F", &info, 2);
    debug_assert_call_arg_byte_shape(info.params[1], 2, 1);
    debug_assert_call_return_slot_shape("F", info.return_slot.unwrap());
}

#[test]
fn routine_internal_abi_defaults_to_public_return_slot() {
    let slot = StorageSlot::zero_page(runtime_zp::ARGS.address(), 2);
    let abi = RoutineInternalAbi::from_public_result_and_facts(Some(slot), RoutineFacts::default());

    assert_eq!(abi.public_result_slot(), Some(slot));
    assert_eq!(abi.result_byte(0), Some(InternalResultByte::PublicSlot(0)));
    assert_eq!(abi.result_byte(1), Some(InternalResultByte::PublicSlot(1)));
    assert!(!abi.result_byte_is_register_a(0));
    assert!(!abi.result_byte_is_register_a(1));
}

#[test]
fn routine_internal_abi_records_accumulator_result_bytes() {
    let slot = StorageSlot::zero_page(runtime_zp::ARGS.address(), 2);
    let facts = RoutineFacts {
        returns_a_equals_a0: true,
        returns_a_equals_a1: false,
    };
    let abi = RoutineInternalAbi::from_public_result_and_facts(Some(slot), facts);

    assert_eq!(abi.public_result_slot(), Some(slot));
    assert_eq!(abi.result_byte(0), Some(InternalResultByte::RegisterA));
    assert_eq!(abi.result_byte(1), Some(InternalResultByte::PublicSlot(1)));
    assert!(abi.result_byte_is_register_a(0));
    assert!(!abi.result_byte_is_register_a(1));
}

#[test]
fn virtual_temp_allocator_uses_non_overlapping_zero_page_candidates() {
    let mut temps = VirtualTempAllocator::default();
    let first = temps
        .allocate_zero_page(VirtualTempWidth::Word, VirtualTempPurpose::Expression)
        .unwrap();
    let second = temps
        .allocate_zero_page(VirtualTempWidth::Word, VirtualTempPurpose::Address)
        .unwrap();

    let first = temps.get(first).unwrap();
    let second = temps.get(second).unwrap();
    let (
        VirtualTempHome::ZeroPage {
            slot: first_slot, ..
        },
        VirtualTempHome::ZeroPage {
            slot: second_slot, ..
        },
    ) = (first.home, second.home)
    else {
        panic!("expected zero-page virtual temp homes");
    };

    assert_eq!(
        first_slot,
        StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2)
    );
    assert_eq!(
        second_slot,
        StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), 2)
    );
    assert!(!storage_slots_overlap(first_slot, second_slot));
}

#[test]
fn byte_virtual_temp_can_use_value_temp_zero_page() {
    let mut temps = VirtualTempAllocator::default();
    let id = temps
        .allocate_zero_page(VirtualTempWidth::Byte, VirtualTempPurpose::Expression)
        .unwrap();

    assert_eq!(
        temps.get(id).unwrap().home,
        VirtualTempHome::ZeroPage {
            slot: StorageSlot::zero_page(runtime_zp::VALUE_TEMP.address(), 1),
            volatility: ZeroPageTempVolatility::ClobberedByAnyCall,
        }
    );
}

#[test]
fn zero_page_virtual_temp_is_not_call_preserved_by_default() {
    let home = VirtualTempHome::ZeroPage {
        slot: StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2),
        volatility: ZeroPageTempVolatility::ClobberedByAnyCall,
    };

    assert!(!zero_page_temp_survives_effects(
        home,
        RoutineEffects::known_empty()
    ));
}

#[test]
fn preserved_zero_page_virtual_temp_checks_known_call_effects() {
    let home = VirtualTempHome::ZeroPage {
        slot: StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2),
        volatility: ZeroPageTempVolatility::PreservedAcrossKnownCall,
    };
    let mut effects = RoutineEffects::known_empty();

    assert!(zero_page_temp_survives_effects(home, effects));
    effects.record_zero_page_write(runtime_zp::ELEMENT_ADDR.offset(1));
    assert!(!zero_page_temp_survives_effects(home, effects));
    assert!(!zero_page_temp_survives_effects(
        home,
        RoutineEffects::unknown()
    ));
}

#[test]
fn configurable_zero_page_pool_preserves_default_candidate_order() {
    let byte_candidates = zero_page_temp_candidates(VirtualTempWidth::Byte);
    let word_candidates = zero_page_temp_candidates(VirtualTempWidth::Word);

    assert_eq!(
        byte_candidates
            .iter()
            .map(|candidate| candidate.slot)
            .collect::<Vec<_>>(),
        vec![
            StorageSlot::zero_page(runtime_zp::VALUE_TEMP.address(), 1),
            StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 1),
            StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), 1),
            StorageSlot::zero_page(runtime_zp::ADDR.address(), 1),
        ]
    );
    assert_eq!(
        word_candidates
            .iter()
            .map(|candidate| candidate.slot)
            .collect::<Vec<_>>(),
        vec![
            StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2),
            StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), 2),
            StorageSlot::zero_page(runtime_zp::ADDR.address(), 2),
        ]
    );
}

#[test]
fn configurable_zero_page_pool_supports_sliding_modern_ranges() {
    let pool =
        ZeroPageTempPool::with_ranges(vec![ZeroPageTempRange::sliding(ZeroPage::new(0xD0), 8)]);
    let mut temps = VirtualTempAllocator::default();
    let first = temps
        .allocate_zero_page_from_pool(
            VirtualTempWidth::Word,
            VirtualTempPurpose::Expression,
            &pool,
        )
        .unwrap();
    let second = temps
        .allocate_zero_page_from_pool(VirtualTempWidth::Word, VirtualTempPurpose::Address, &pool)
        .unwrap();

    assert_eq!(
        temps.get(first).unwrap().home,
        VirtualTempHome::ZeroPage {
            slot: StorageSlot::zero_page(0xD0, 2),
            volatility: ZeroPageTempVolatility::ClobberedByAnyCall,
        }
    );
    assert_eq!(
        temps.get(second).unwrap().home,
        VirtualTempHome::ZeroPage {
            slot: StorageSlot::zero_page(0xD2, 2),
            volatility: ZeroPageTempVolatility::ClobberedByAnyCall,
        }
    );
}

#[test]
fn configurable_zero_page_pool_honors_reserved_ranges() {
    let pool =
        ZeroPageTempPool::with_ranges(vec![ZeroPageTempRange::sliding(ZeroPage::new(0xD0), 8)])
            .with_reserved(vec![ZeroPageTempRange::fixed(ZeroPage::new(0xD2), 2)]);
    let word_candidates = pool.candidates(VirtualTempWidth::Word);

    assert_eq!(
        word_candidates
            .iter()
            .map(|candidate| candidate.slot.address)
            .collect::<Vec<_>>(),
        vec![0xD0, 0xD4, 0xD5, 0xD6]
    );
}

#[test]
fn expression_side_effect_facts_distinguish_array_calls_from_routine_calls() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.layout.symbols.insert(
        normalize_name("arr"),
        StorageSlot::array(0x4000, 1, ArrayStorage::Inline),
    );
    let array_expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("arr")),
        args: vec![test_name_expr("i")],
    });
    let call_expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("Fn")),
        args: Vec::new(),
    });

    let array_facts = generator.expr_side_effect_facts(&array_expr);
    assert!(!array_facts.has_routine_call);
    assert!(array_facts.reads_memory);
    assert!(
        generator
            .expr_side_effect_facts(&call_expr)
            .has_routine_call
    );
}

#[test]
fn expression_side_effect_facts_track_volatile_and_pointer_reads() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("p"), StorageSlot::pointer(0x3000, 1));
    let volatile = test_name_expr("DEVICE");
    let pointer_deref = test_expr(ExprKind::Unary {
        op: UnaryOp::Deref,
        expr: Box::new(test_name_expr("p")),
    });

    let volatile_facts = generator.expr_side_effect_facts(&volatile);
    let pointer_facts = generator.expr_side_effect_facts(&pointer_deref);

    assert!(volatile_facts.reads_memory);
    assert!(volatile_facts.reads_volatile);
    assert!(!volatile_facts.can_duplicate());
    assert!(pointer_facts.reads_pointer);
    assert!(pointer_facts.reads_memory);
    assert!(!pointer_facts.can_reorder());
}

#[test]
fn expression_side_effect_facts_mark_binary_calls_as_order_sensitive() {
    let generator = test_generator(CodegenProfile::Modern);
    let expr = test_expr(ExprKind::Binary {
        op: BinaryOp::Add,
        left: Box::new(test_expr(ExprKind::Call {
            callee: Box::new(test_name_expr("Left")),
            args: Vec::new(),
        })),
        right: Box::new(test_expr(ExprKind::Call {
            callee: Box::new(test_name_expr("Right")),
            args: Vec::new(),
        })),
    });

    let facts = generator.expr_side_effect_facts(&expr);

    assert!(facts.has_routine_call);
    assert!(facts.evaluation_order_sensitive);
    assert!(!facts.can_duplicate());
}

#[test]
fn index_address_proof_classifies_inline_byte_array_byte_index_as_absolute_y() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.layout.symbols.insert(
        normalize_name("arr"),
        StorageSlot::array(0x4000, 1, ArrayStorage::Inline),
    );
    generator
        .layout
        .symbols
        .insert(normalize_name("i"), StorageSlot::absolute(0x3000, 1));
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("arr")),
        args: vec![test_name_expr("i")],
    });

    let proof = generator.index_address_proof(&expr).unwrap();

    assert_eq!(
        proof.base,
        StorageSlot::array(0x4000, 1, ArrayStorage::Inline)
    );
    assert_eq!(proof.element_size, 1);
    assert_eq!(proof.index_width, Some(1));
    assert_eq!(proof.mode, IndexAddressMode::AbsoluteY);
    assert_eq!(proof.reject_reason, None);
}

#[test]
fn index_address_proof_requires_scaling_for_word_arrays() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.layout.symbols.insert(
        normalize_name("words"),
        StorageSlot::array(0x4000, 2, ArrayStorage::Inline),
    );
    generator
        .layout
        .symbols
        .insert(normalize_name("i"), StorageSlot::absolute(0x3000, 1));
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("words")),
        args: vec![test_name_expr("i")],
    });

    let proof = generator.index_address_proof(&expr).unwrap();

    assert_eq!(proof.element_size, 2);
    assert_eq!(proof.mode, IndexAddressMode::NeedsScaling);
    assert_eq!(
        proof.reject_reason,
        Some(IndexAddressRejectReason::ElementNeedsScaling)
    );
}

#[test]
fn index_address_proof_rejects_word_index_for_direct_y_lowering() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.layout.symbols.insert(
        normalize_name("arr"),
        StorageSlot::array(0x4000, 1, ArrayStorage::Inline),
    );
    generator
        .layout
        .symbols
        .insert(normalize_name("i"), StorageSlot::absolute(0x3000, 2));
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("arr")),
        args: vec![test_name_expr("i")],
    });

    let proof = generator.index_address_proof(&expr).unwrap();

    assert_eq!(proof.mode, IndexAddressMode::Unsupported);
    assert_eq!(
        proof.reject_reason,
        Some(IndexAddressRejectReason::NonByteIndex)
    );
}

#[test]
fn index_address_proof_rejects_effectful_indexes() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.layout.symbols.insert(
        normalize_name("arr"),
        StorageSlot::array(0x4000, 1, ArrayStorage::Inline),
    );
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("arr")),
        args: vec![test_expr(ExprKind::Call {
            callee: Box::new(test_name_expr("NextIndex")),
            args: Vec::new(),
        })],
    });

    let proof = generator.index_address_proof(&expr).unwrap();

    assert_eq!(proof.mode, IndexAddressMode::Unsupported);
    assert_eq!(
        proof.reject_reason,
        Some(IndexAddressRejectReason::IndexHasSideEffects)
    );
}

#[test]
fn index_address_proof_classifies_pointer_byte_arrays_as_indirect_y() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("p"), StorageSlot::pointer(0x3000, 1));
    generator
        .layout
        .symbols
        .insert(normalize_name("i"), StorageSlot::absolute(0x3002, 1));
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("p")),
        args: vec![test_name_expr("i")],
    });

    let proof = generator.index_address_proof(&expr).unwrap();

    assert_eq!(proof.mode, IndexAddressMode::IndirectY);
    assert_eq!(proof.reject_reason, None);
}

#[test]
fn pointer_dereference_proof_classifies_direct_deref() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("p"), StorageSlot::pointer(0x3000, 1));
    let expr = test_expr(ExprKind::Unary {
        op: UnaryOp::Deref,
        expr: Box::new(test_name_expr("p")),
    });

    let proof = generator.pointer_dereference_proof(&expr).unwrap();

    assert_eq!(proof.kind, PointerDereferenceKind::Direct);
    assert_eq!(proof.pointee_size, 1);
    assert_eq!(proof.mode, PointerDereferenceMode::IndirectY);
    assert_eq!(proof.reject_reason, None);
}

#[test]
fn pointer_dereference_proof_classifies_indexed_pointer() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("p"), StorageSlot::pointer(0x3000, 1));
    generator
        .layout
        .symbols
        .insert(normalize_name("i"), StorageSlot::absolute(0x3002, 1));
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("p")),
        args: vec![test_name_expr("i")],
    });

    let proof = generator.pointer_dereference_proof(&expr).unwrap();

    assert_eq!(proof.kind, PointerDereferenceKind::Indexed);
    assert_eq!(proof.mode, PointerDereferenceMode::IndirectY);
    assert_eq!(proof.index.unwrap().mode, IndexAddressMode::IndirectY);
}

#[test]
fn pointer_dereference_proof_marks_word_pointer_index_as_needing_scaling() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("p"), StorageSlot::pointer(0x3000, 2));
    generator
        .layout
        .symbols
        .insert(normalize_name("i"), StorageSlot::absolute(0x3002, 1));
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("p")),
        args: vec![test_name_expr("i")],
    });

    let proof = generator.pointer_dereference_proof(&expr).unwrap();

    assert_eq!(proof.kind, PointerDereferenceKind::Indexed);
    assert_eq!(proof.mode, PointerDereferenceMode::NeedsIndexScaling);
    assert_eq!(
        proof.reject_reason,
        Some(PointerDereferenceRejectReason::ElementNeedsScaling)
    );
}

#[test]
fn pointer_dereference_proof_classifies_record_pointer_field_offsets() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let mut fields = HashMap::new();
    fields.insert(
        normalize_name("wide"),
        RecordField {
            offset: 0xFE,
            size: 2,
            signed: false,
        },
    );
    generator.record_layouts.layouts.push(RecordLayout {
        size: 0x100,
        fields,
    });
    generator.layout.symbols.insert(
        normalize_name("rp"),
        StorageSlot::pointer(0x3000, 0x100).record(Some(0)),
    );
    let expr = test_expr(ExprKind::Field {
        base: Box::new(test_name_expr("rp")),
        field: "wide".to_string(),
    });

    let proof = generator.pointer_dereference_proof(&expr).unwrap();

    assert_eq!(proof.kind, PointerDereferenceKind::RecordField);
    assert_eq!(proof.pointee_size, 2);
    assert_eq!(proof.mode, PointerDereferenceMode::IndirectYWithOffset);
    assert_eq!(proof.field.unwrap().offset, 0xFE);
}

#[test]
fn pointer_dereference_proof_rejects_record_field_offsets_that_do_not_fit_y() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let mut fields = HashMap::new();
    fields.insert(
        normalize_name("too_far"),
        RecordField {
            offset: 0xFF,
            size: 2,
            signed: false,
        },
    );
    generator.record_layouts.layouts.push(RecordLayout {
        size: 0x101,
        fields,
    });
    generator.layout.symbols.insert(
        normalize_name("rp"),
        StorageSlot::pointer(0x3000, 0x101).record(Some(0)),
    );
    let expr = test_expr(ExprKind::Field {
        base: Box::new(test_name_expr("rp")),
        field: "too_far".to_string(),
    });

    let proof = generator.pointer_dereference_proof(&expr).unwrap();

    assert_eq!(proof.mode, PointerDereferenceMode::NeedsAddressArithmetic);
    assert_eq!(
        proof.reject_reason,
        Some(PointerDereferenceRejectReason::FieldOffsetTooWide)
    );
}

#[test]
fn value_availability_proof_tracks_constants_and_storage_slots() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("w"), StorageSlot::absolute(0x3000, 2));
    let constant = test_expr(ExprKind::Number(crate::lexer::NumberLiteral {
        text: "$1234".to_string(),
        kind: crate::lexer::NumberKind::Card,
        value: Some(0x1234),
    }));
    let storage = test_name_expr("w");

    let constant = generator.value_availability_proof(&constant);
    let storage = generator.value_availability_proof(&storage);

    assert_eq!(constant.source, ValueAvailabilitySource::Constant);
    assert_eq!(
        constant.bytes,
        [
            Some(ValueByteAvailability::Constant(0x34)),
            Some(ValueByteAvailability::Constant(0x12)),
        ]
    );
    assert_eq!(storage.source, ValueAvailabilitySource::Storage);
    assert_eq!(
        storage.bytes[1],
        Some(ValueByteAvailability::Slot {
            slot: StorageSlot::absolute(0x3000, 2),
            byte_index: 1,
        })
    );
}

#[test]
fn value_availability_proof_uses_internal_call_result_registers() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.routines.insert(
        normalize_name("Get"),
        RoutineInfo {
            label: "Get".to_string(),
            params: Vec::new(),
            return_slot: Some(StorageSlot::zero_page(runtime_zp::ARGS.address(), 1)),
            system_address: None,
            facts: RoutineFacts {
                returns_a_equals_a0: true,
                returns_a_equals_a1: false,
            },
            effects: RoutineEffects::known_empty(),
        },
    );
    let expr = test_expr(ExprKind::Call {
        callee: Box::new(test_name_expr("Get")),
        args: Vec::new(),
    });

    let proof = generator.value_availability_proof(&expr);

    assert_eq!(proof.source, ValueAvailabilitySource::RoutineReturn);
    assert_eq!(
        proof.bytes[0],
        Some(ValueByteAvailability::Register(RegisterName::A))
    );
}

#[test]
fn routine_visibility_facts_notice_retargetable_routines() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.routines.insert(
        normalize_name("Draw"),
        RoutineInfo {
            label: "Draw".to_string(),
            params: Vec::new(),
            return_slot: None,
            system_address: None,
            facts: RoutineFacts::default(),
            effects: RoutineEffects::known_empty(),
        },
    );
    generator
        .routine_assignment_targets
        .insert(normalize_name("Draw"));

    let facts = generator.routine_visibility_facts("Draw").unwrap();

    assert!(facts.retargetable);
    assert!(facts.address_taken);
    assert!(!facts.internal_only_candidate);
}

#[test]
fn routine_boundary_proof_classifies_system_routines_as_public_boundaries() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.routines.insert(
        normalize_name("Put"),
        RoutineInfo {
            label: "Put".to_string(),
            params: Vec::new(),
            return_slot: None,
            system_address: Some(0xE456),
            facts: RoutineFacts::default(),
            effects: RoutineEffects::unknown(),
        },
    );

    let proof = generator.routine_boundary_proof("Put").unwrap();

    assert_eq!(proof.kind, RoutineBoundaryKind::System);
    assert!(proof.public_entry_required);
    assert!(!proof.internal_abi_candidate);
}

#[test]
fn routine_boundary_proof_marks_plain_routines_as_internal_candidates() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.routines.insert(
        normalize_name("Helper"),
        RoutineInfo {
            label: "Helper".to_string(),
            params: Vec::new(),
            return_slot: None,
            system_address: None,
            facts: RoutineFacts::default(),
            effects: RoutineEffects::known_empty(),
        },
    );

    let proof = generator.routine_boundary_proof("Helper").unwrap();

    assert_eq!(proof.kind, RoutineBoundaryKind::InternalCandidate);
    assert!(proof.internal_only_candidate);
    assert!(proof.internal_abi_candidate);
    assert!(!proof.public_entry_required);
}

#[test]
fn routine_boundary_proof_requires_patchable_entry_for_retargetable_routines() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.routines.insert(
        normalize_name("Draw"),
        RoutineInfo {
            label: "Draw".to_string(),
            params: Vec::new(),
            return_slot: None,
            system_address: None,
            facts: RoutineFacts::default(),
            effects: RoutineEffects::known_empty(),
        },
    );
    generator
        .routine_assignment_targets
        .insert(normalize_name("Draw"));

    let proof = generator.routine_boundary_proof("Draw").unwrap();

    assert_eq!(proof.kind, RoutineBoundaryKind::Retargetable);
    assert!(proof.address_taken);
    assert!(proof.public_entry_required);
    assert!(proof.patchable_entry_required);
}

#[test]
fn call_boundary_proof_wraps_zero_page_survival_checks() {
    let home = VirtualTempHome::ZeroPage {
        slot: StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2),
        volatility: ZeroPageTempVolatility::PreservedAcrossKnownCall,
    };
    let mut effects = RoutineEffects::known_empty();

    assert!(call_boundary_proof(home, effects).survives);
    effects.record_zero_page_write(runtime_zp::ELEMENT_ADDR);
    assert!(!call_boundary_proof(home, effects).survives);
}

#[test]
fn zero_page_temp_lifetime_proof_finds_first_blocking_call() {
    let home = VirtualTempHome::ZeroPage {
        slot: StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2),
        volatility: ZeroPageTempVolatility::PreservedAcrossKnownCall,
    };
    let safe = RoutineEffects::known_empty();
    let mut clobbering = RoutineEffects::known_empty();
    clobbering.record_zero_page_write(runtime_zp::ELEMENT_ADDR.offset(1));

    let proof = zero_page_temp_lifetime_proof(home, &[safe, clobbering]);

    assert_eq!(proof.calls_crossed, 2);
    assert!(!proof.survives_all_calls);
    assert_eq!(proof.first_blocking_call, Some(1));
}

#[test]
fn zero_page_temp_placement_proof_skips_occupied_pool_slots() {
    let pool =
        ZeroPageTempPool::with_ranges(vec![ZeroPageTempRange::sliding(ZeroPage::new(0xD0), 8)]);
    let occupied = [StorageSlot::zero_page(0xD0, 2)];

    let proof = zero_page_temp_placement_proof(VirtualTempWidth::Word, &pool, &occupied);

    assert_eq!(
        proof.candidate.unwrap().slot,
        StorageSlot::zero_page(0xD2, 2)
    );
    assert!(!proof.blocked_by_occupied_slot);
}

#[test]
fn zero_page_temp_placement_proof_reports_full_pool_blockage() {
    let pool =
        ZeroPageTempPool::with_ranges(vec![ZeroPageTempRange::sliding(ZeroPage::new(0xD0), 2)]);
    let occupied = [StorageSlot::zero_page(0xD0, 2)];

    let proof = zero_page_temp_placement_proof(VirtualTempWidth::Word, &pool, &occupied);

    assert_eq!(proof.candidate, None);
    assert!(proof.blocked_by_occupied_slot);
}

#[test]
fn assignment_width_guard_accepts_scalar_pointer_and_deref_targets() {
    let target = test_name_expr("p");
    let value = test_name_expr("q");
    debug_assert_assignment_width_shape(
        &target,
        &value,
        StorageSlot::pointer(0x3000, 1),
        Some(2),
        false,
    );

    let deref_target = Expr {
        kind: ExprKind::Unary {
            op: UnaryOp::Deref,
            expr: Box::new(test_name_expr("p")),
        },
        text: "p^".to_string(),
        span: Span::new(0, 0),
    };
    debug_assert_assignment_width_shape(
        &deref_target,
        &test_name_expr("b"),
        StorageSlot::indirect_indexed_y(runtime_zp::ARRAY_ADDR, 1),
        Some(1),
        false,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "string assignment value must be written into pointer-width storage")]
fn assignment_width_guard_rejects_string_to_byte_target() {
    let target = test_name_expr("b");
    let value = Expr {
        kind: ExprKind::String("HELLO".to_string()),
        text: "\"HELLO\"".to_string(),
        span: Span::new(0, 0),
    };
    debug_assert_assignment_width_shape(
        &target,
        &value,
        StorageSlot::absolute(0x3000, 1),
        Some(2),
        false,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "pointer assignment value must fit")]
fn assignment_width_guard_rejects_oversized_pointer_value() {
    let target = test_name_expr("p");
    let value = test_name_expr("large");
    debug_assert_assignment_width_shape(
        &target,
        &value,
        StorageSlot::pointer(0x3000, 1),
        Some(3),
        false,
    );
}

#[test]
fn assignment_width_guard_accepts_record_address_assignment_to_pointer() {
    let target = test_name_expr("rp");
    let value = test_name_expr("r");
    debug_assert_assignment_width_shape(
        &target,
        &value,
        StorageSlot::pointer(0x3000, 4).record(Some(0)),
        Some(4),
        true,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "must be packed from ARGS")]
fn call_abi_guard_rejects_sparse_arg_slots() {
    let info = RoutineInfo {
        label: "routine:F".to_string(),
        params: vec![
            StorageSlot::zero_page(runtime_zp::ARGS.address(), 1),
            StorageSlot::zero_page(runtime_zp::ARGS.offset(2).address(), 1),
        ],
        return_slot: None,
        system_address: None,
        facts: RoutineFacts::default(),
        effects: RoutineEffects::unknown(),
    };

    debug_assert_call_abi_shape("F", &info, 2);
}

#[test]
fn slot_shape_guard_uses_pointer_width_for_unsized_array_storage() {
    let slot = StorageSlot {
        array: Some(ArrayStorage::Pointer),
        ..StorageSlot::absolute(0x3000, 1)
    };

    assert_eq!(slot_accessible_byte_size(slot), 2);
    debug_assert_slot_byte_access(slot, 1, "store");
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "must stay inside slot width")]
fn slot_shape_guard_rejects_out_of_bounds_scalar_byte_access() {
    debug_assert_slot_byte_access(StorageSlot::absolute(0x3000, 1), 1, "load");
}

#[test]
fn indirect_pointer_guard_accepts_runtime_scratch_pointers() {
    for pointer in [
        runtime_zp::ARRAY_ADDR,
        runtime_zp::ELEMENT_ADDR,
        runtime_zp::ADDR,
    ] {
        let slot = StorageSlot::indirect_indexed_y(pointer, 1);
        debug_assert_prepared_indirect_slot(slot, pointer, "test");
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "must use one of the Action! runtime pointer registers")]
fn indirect_pointer_guard_rejects_accidental_abi_pointer() {
    debug_assert_scratch_indirect_pointer(runtime_zp::ARGS, "test");
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "must use distinct indirect pointers")]
fn indirect_copy_guard_rejects_aliasing_source_and_target_pointers() {
    let source = StorageSlot::indirect_indexed_y(runtime_zp::ARRAY_ADDR, 1);
    let target = StorageSlot::indirect_indexed_y(runtime_zp::ARRAY_ADDR, 1);

    debug_assert_indirect_slots_do_not_alias(source, target, "test");
}

#[test]
fn runtime_helper_guard_accepts_known_cartridge_and_slot_targets() {
    debug_assert_runtime_helper_abi_shape(
        RuntimeHelperSlot::Mul,
        &runtime_helper::CARTRIDGE_MUL.into(),
        StorageSlot::absolute(0x3000, 2),
        true,
    );
    debug_assert_runtime_helper_abi_shape(
        RuntimeHelperSlot::Lsh,
        &runtime_helper::LSH_SLOT.into(),
        StorageSlot::zero_page(runtime_zp::ARGS.address(), 1),
        false,
    );
    debug_assert_sargs_helper_abi(&runtime_helper::CARTRIDGE_SARGS.into(), 0x3000, 3);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "belongs to Rsh")]
fn runtime_helper_guard_rejects_mismatched_known_target() {
    debug_assert_runtime_helper_abi_shape(
        RuntimeHelperSlot::Lsh,
        &runtime_helper::CARTRIDGE_RSH.into(),
        StorageSlot::absolute(0x3000, 2),
        false,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "requires right operand low/high")]
fn runtime_helper_guard_rejects_mul_without_right_high_byte() {
    debug_assert_runtime_helper_abi_shape(
        RuntimeHelperSlot::Mul,
        &runtime_helper::CARTRIDGE_MUL.into(),
        StorageSlot::absolute(0x3000, 2),
        false,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "only used for stack argument frames")]
fn sargs_guard_rejects_tiny_argument_frame() {
    debug_assert_sargs_helper_abi(&runtime_helper::CARTRIDGE_SARGS.into(), 0x3000, 2);
}

#[test]
fn modern_profile_suppresses_redundant_ldx_immediates() {
    let mut compatible = test_generator(CodegenProfile::Compat);
    compatible.emit_ldx_imm(7);
    compatible.emit_ldx_imm(7);

    let mut modern = test_generator(CodegenProfile::Modern);
    modern.emit_ldx_imm(7);
    modern.emit_ldx_imm(7);

    assert_eq!(
        compatible.emitter.bytes,
        [opcode::LDX_IMM, 7, opcode::LDX_IMM, 7]
    );
    assert_eq!(modern.emitter.bytes, [opcode::LDX_IMM, 7]);
    assert_eq!(
        modern.optimizations[0].kind,
        CodegenOptimizationKind::RegisterReloadRemoved
    );
}

#[test]
fn modern_profile_suppresses_redundant_ldy_immediates() {
    let mut compatible = test_generator(CodegenProfile::Compat);
    compatible.emit_ldy_imm(9);
    compatible.emit_ldy_imm(9);

    let mut modern = test_generator(CodegenProfile::Modern);
    modern.emit_ldy_imm(9);
    modern.emit_ldy_imm(9);

    assert_eq!(
        compatible.emitter.bytes,
        [opcode::LDY_IMM, 9, opcode::LDY_IMM, 9]
    );
    assert_eq!(modern.emitter.bytes, [opcode::LDY_IMM, 9]);
    assert_eq!(
        modern.optimizations[0].kind,
        CodegenOptimizationKind::RegisterReloadRemoved
    );
}

#[test]
fn modern_profile_uses_value_proof_for_call_result_byte_loads() {
    let source =
        "BYTE out BYTE FUNC Get() RETURN(7) PROC Main() IF Get()=7 THEN out=1 ELSE out=2 FI RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address()] })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::CallResultMaterializationRemoved
            && optimization
                .message
                .contains("proven accumulator call result")
    }));
}

#[test]
fn modern_profile_uses_value_proof_for_storage_byte_loads() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("w"), StorageSlot::absolute(0x3000, 2));

    assert!(generator.emit_proven_simple_value_byte(&test_name_expr("w"), 1));

    assert_eq!(generator.emitter.bytes, [opcode::LDA_ABS, 0x01, 0x30]);
}

#[test]
fn modern_profile_uses_value_proof_for_storage_assignment() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .layout
        .symbols
        .insert(normalize_name("w"), StorageSlot::absolute(0x3000, 2));
    let target = StorageSlot::absolute(0x3010, 2);

    assert!(generator.emit_proven_simple_value_to_slot(&test_name_expr("w"), target));

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_ABS,
            0x01,
            0x30,
            opcode::STA_ABS,
            0x11,
            0x30,
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::STA_ABS,
            0x10,
            0x30,
        ]
    );
}

#[test]
fn modern_profile_reuses_known_y_for_constant_store() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldy_imm(0);
    generator.emit_store_constant(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDY_IMM, 0, opcode::STY_ABS, 0x00, 0x30]
    );
    assert!(generator.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ConstantStoreReusedRegister
            && optimization.message.contains("Y=#$00")
    }));
}

#[test]
fn modern_profile_reuses_known_x_for_constant_store() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::zero_page(0x80, 1);

    generator.emit_ldx_imm(0);
    generator.emit_store_constant(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDX_IMM, 0, opcode::STX_ZP, 0x80]
    );
    assert!(generator.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ConstantStoreReusedRegister
            && optimization.message.contains("X=#$00")
    }));
}

#[test]
fn modern_profile_logs_trampoline_and_final_rts_optimizations() {
    let compatible =
        generate_profile_source_with_origin("PROC Main() RETURN", 0x3000, CodegenProfile::Compat)
            .unwrap();
    let modern =
        generate_profile_source_with_origin("PROC Main() RETURN", 0x3000, CodegenProfile::Modern)
            .unwrap();

    assert!(compatible.optimizations.is_empty());
    assert!(
        modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::TrampolineElided })
    );
    assert!(
        modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::FinalRtsRemoved })
    );
}

#[test]
fn modern_profile_lowers_call_before_return_to_tail_jump() {
    let compatible = generate_profile_source_with_origin(
        "PROC First() RETURN PROC NavInit() First() RETURN",
        0x3000,
        CodegenProfile::Compat,
    )
    .unwrap();
    let modern = generate_profile_source_with_origin(
        "PROC First() RETURN PROC NavInit() First() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        compatible
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x30, opcode::RTS])
    );
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JMP_ABS, 0x00, 0x30])
    );
    assert!(
        !modern
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x30, opcode::RTS])
    );
    assert!(
        modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::TailCall })
    );
}

#[test]
fn modern_profile_can_debug_lower_one_routine_with_compat_profile() {
    let modern = generate_profile_source_with_origin(
        "PROC First() RETURN\n;@actionc profile compat\nPROC NavInit() First() RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let first = routine_address(&modern, "First").expect("missing First");
    let first_operand = first.to_le_bytes();

    assert!(
        modern.bytes.windows(4).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    first_operand[0],
                    first_operand[1],
                    opcode::RTS,
                ]
        }),
        "debug compat routine should keep call plus RTS shape"
    );
    assert!(
        !modern.optimizations.iter().any(|optimization| {
            optimization.routine.as_deref() == Some("NavInit")
                && optimization.kind == CodegenOptimizationKind::TailCall
        }),
        "debug compat routine should not record modern body optimizations"
    );
    assert!(
        modern.optimizations.iter().any(|optimization| {
            optimization.routine.as_deref() == Some("First")
                && optimization.kind == CodegenOptimizationKind::TrampolineElided
        }),
        "other routines in the same modern build should stay modern"
    );
}

#[test]
fn modern_debug_compat_routine_reuses_modern_hidden_string_layout() {
    let modern = generate_profile_source_with_origin(
        "DEFINE STRING=\"CHAR ARRAY\" BYTE x PROC Take(STRING s) x=s(1) RETURN \
         ;@actionc profile compat\nPROC Main() Take(\"HI\") RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let literal_offsets: Vec<usize> = modern
        .bytes
        .windows(3)
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == [0x02, b'H', b'I']).then_some(offset))
        .collect();
    assert_eq!(
        literal_offsets.len(),
        1,
        "debug compat body should reuse the modern hidden literal, not emit a second inline copy"
    );

    let literal_address = modern.origin.wrapping_add(literal_offsets[0] as u16);
    let main_address = routine_address(&modern, "Main").unwrap();

    assert!(literal_address < main_address);
    assert!(modern.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::StorageInitializer
            && range.name.as_deref() == Some("modern hidden string literal")
            && range.start == literal_address
            && range.end == literal_address + 3
    }));
}

#[test]
fn modern_profile_keeps_debug_compat_call_return_slot_materialization() {
    let modern = generate_profile_source_with_origin(
        ";@actionc profile compat\nBYTE FUNC T1(BYTE n) BYTE a a=n RETURN(a) \
         BYTE FUNC T2(BYTE n) BYTE a a=n RETURN(a) \
         PROC T3() BYTE b,c b=T1(1) c=T2(1) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let t1 = routine_address(&modern, "T1").expect("missing T1");
    let t2 = routine_address(&modern, "T2").expect("missing T2");
    let t1_operand = t1.to_le_bytes();
    let t2_operand = t2.to_le_bytes();

    assert!(
        modern.bytes.windows(5).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    t1_operand[0],
                    t1_operand[1],
                    opcode::LDA_ZP,
                    runtime_zp::ARGS.address(),
                ]
        }),
        "debug compat callee should force caller to read the public return slot"
    );
    assert!(
        modern.bytes.windows(4).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    t2_operand[0],
                    t2_operand[1],
                    opcode::STA_ABS,
                ]
        }),
        "ordinary modern callee can still publish accumulator return facts"
    );
}

#[test]
fn modern_profile_keeps_join_return_after_conditional_call() {
    let modern = generate_profile_source_with_origin(
        "BYTE ioerr PROC Handle() RETURN PROC NavError=*() IF ioerr # 136 THEN Handle() FI RETURN PROC Next() BYTE x x=1 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x01, 0x30, opcode::RTS])
    );
    assert!(
        !modern
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JMP_ABS, 0x01, 0x30, 0x00])
    );
}

#[test]
fn modern_profile_lowers_final_proc_call_to_tail_jump() {
    let compatible = generate_profile_source_with_origin(
        "PROC First() RETURN PROC NavInit() First()",
        0x3000,
        CodegenProfile::Compat,
    )
    .unwrap();
    let modern = generate_profile_source_with_origin(
        "PROC First() RETURN PROC NavInit() First()",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        compatible
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x30, opcode::RTS])
    );
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::JMP_ABS, 0x00, 0x30])
    );
    assert!(
        !modern
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::JSR_ABS, 0x00, 0x30, opcode::RTS])
    );
    assert!(
        modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::TailCall })
    );
}

#[test]
fn modern_profile_rewrites_trailing_jsr_rts_to_jmp() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator.emitter.emit_jsr_absolute(Absolute::new(0x3456));
    generator.emit_return_rts(Span::new(0, 0));

    assert_eq!(generator.emitter.bytes, [opcode::JMP_ABS, 0x56, 0x34]);
    assert!(generator.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::TailCall
            && optimization.message.contains("plus RTS to tail JMP")
    }));
}

#[test]
fn compatible_profile_keeps_trailing_jsr_rts_shape() {
    let mut generator = test_generator(CodegenProfile::Compat);
    generator.emitter.emit_jsr_absolute(Absolute::new(0x3456));
    generator.emit_return_rts(Span::new(0, 0));

    assert_eq!(
        generator.emitter.bytes,
        [opcode::JSR_ABS, 0x56, 0x34, opcode::RTS]
    );
}

#[test]
fn modern_profile_rewrites_jmp_to_return_label_as_rts() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);
    generator.emit_jmp_label("done", span);
    generator.bind_codegen_label("done".to_string(), span);
    generator.emit_return_rts(span);

    assert_eq!(generator.emitter.bytes, [opcode::RTS, opcode::RTS]);
    assert!(
        generator
            .optimizations
            .iter()
            .any(|optimization| optimization.kind == CodegenOptimizationKind::JumpToRtsRemoved)
    );
}

#[test]
fn modern_profile_keeps_jmp_to_return_label_when_resolved_branch_crosses_deleted_operands() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);
    generator.emitter.bytes.extend([opcode::BEQ_REL, 0x03]);
    generator.emit_jmp_label("done", span);
    generator.bind_codegen_label("done".to_string(), span);
    generator.emit_return_rts(span);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::BEQ_REL, 0x03, opcode::JMP_ABS, 0, 0, opcode::RTS]
    );
    assert!(
        generator
            .optimizations
            .iter()
            .all(|optimization| optimization.kind != CodegenOptimizationKind::JumpToRtsRemoved)
    );
}

#[test]
fn compatible_profile_keeps_jmp_to_return_label() {
    let mut generator = test_generator(CodegenProfile::Compat);
    let span = Span::new(0, 0);
    generator.emit_jmp_label("done", span);
    generator.bind_codegen_label("done".to_string(), span);
    generator.emit_return_rts(span);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::JMP_ABS, 0, 0, opcode::RTS]
    );
}

#[test]
fn modern_profile_skips_first_byte_arg_store_when_value_is_already_in_accumulator() {
    let source = "BYTE ARRAY s \n;@actionc preserves $AE/$AF\n;@actionc clobbers $A0\n;@actionc returns A=$A0\nBYTE FUNC Internal=*(BYTE ch) [$85 $A0 $60] PROC Main() s(1)=Internal(s(0)) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[2] == opcode::STA_ZP
            && bytes[3] == runtime_zp::ARGS.address()
            && bytes[4] == opcode::LDA_ZP
            && bytes[5] == runtime_zp::ARGS.address()
            && bytes[6] == opcode::JSR_ABS
    }));
    assert!(!modern.bytes.windows(5).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[2] == opcode::STA_ZP
            && bytes[3] == runtime_zp::ARGS.address()
            && bytes[4] == opcode::JSR_ABS
    }));
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| { bytes[0] == opcode::LDA_IZY && bytes[2] == opcode::JSR_ABS })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
    }));
}

#[test]
fn modern_profile_skips_first_byte_arg_store_for_byte_expression_in_accumulator() {
    let source =
        "BYTE a,b PROC Sink=*(BYTE ch) [$85 $A0 $60] PROC Main() a=1 b=2 Sink(a+b) b=3 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::ADC_ABS
            && bytes[3] == opcode::STA_ZP
            && bytes[4] == runtime_zp::ARGS.address()
            && bytes[5] == opcode::LDA_ZP
    }));
    assert!(!modern.bytes.windows(5).any(|bytes| {
        bytes[0] == opcode::ADC_ABS
            && bytes[3] == opcode::STA_ZP
            && bytes[4] == runtime_zp::ARGS.address()
    }));
    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| { bytes[0] == opcode::ADC_ABS && bytes[3] == opcode::JSR_ABS })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
    }));
}

#[test]
fn modern_profile_stages_runtime_helper_operand_in_tail_call_arg() {
    let source = "PROC Putchar=*(BYTE ch) [$85 $A0 $60] PROC PrintB(BYTE b) Putchar('0%b MOD 10) RETURN PROC Main() PrintB(1) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert_eq!(
        count_jsr_to(&modern.bytes, runtime_helper::CARTRIDGE_MOD.address()),
        1
    );
    assert!(
        !modern
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_IMM, 0x30, opcode::JMP_ABS, bytes[3], bytes[4]])
    );
}

#[test]
fn modern_profile_forwards_final_second_arg_byte_from_accumulator_to_x() {
    let source = "BYTE a,b PROC Sink=*(BYTE first, BYTE second) [$85 $A0 $86 $A1 $60] \
            PROC Main() a=1 b=2 Sink(a,b+1) a=3 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::ADC_IMM
            && bytes[2] == opcode::STA_ZP
            && bytes[3] == runtime_zp::ARGS.offset(1).address()
            && bytes[4] == opcode::LDX_ZP
            && bytes[5] == runtime_zp::ARGS.offset(1).address()
    }));
    assert!(!modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ARGS.offset(1).address()
            && bytes[2] == opcode::LDX_ZP
            && bytes[3] == runtime_zp::ARGS.offset(1).address()
    }));
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| { bytes[0] == opcode::ADC_IMM && bytes[2] == opcode::TAX })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization.message.contains("A to X")
    }));
}

#[test]
fn modern_profile_reuses_x_immediate_for_matching_accumulator_argument() {
    let source = "PROC Sink=*(BYTE first, BYTE second) [$60] PROC Main() Sink(0,0) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(5)
            .any(|bytes| { bytes == [opcode::LDX_IMM, 0, opcode::LDA_IMM, 0, opcode::JSR_ABS] })
    );
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| { bytes == [opcode::LDX_IMM, 0, opcode::TXA] })
    );
    assert!(
        !modern
            .bytes
            .windows(5)
            .any(|bytes| { bytes == [opcode::LDX_IMM, 0, opcode::LDA_IMM, 0, opcode::JSR_ABS] })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization.message.contains("TXA")
    }));
}

#[test]
fn modern_profile_forwards_final_third_arg_byte_from_accumulator_to_y() {
    let source = "BYTE a,b PROC Sink=*(BYTE first, BYTE second, BYTE third) [$85 $A0 $86 $A1 $84 $A2 $60] \
            PROC Main() a=1 b=2 Sink(a,1,b+1) a=3 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(6).any(|bytes| {
        bytes[0] == opcode::ADC_IMM
            && bytes[2] == opcode::STA_ZP
            && bytes[3] == runtime_zp::ARGS.offset(2).address()
            && bytes[4] == opcode::LDY_ZP
            && bytes[5] == runtime_zp::ARGS.offset(2).address()
    }));
    assert!(!modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ARGS.offset(2).address()
            && bytes[2] == opcode::LDY_ZP
            && bytes[3] == runtime_zp::ARGS.offset(2).address()
    }));
    assert!(
        modern
            .bytes
            .windows(3)
            .any(|bytes| { bytes[0] == opcode::ADC_IMM && bytes[2] == opcode::TAY })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization.message.contains("A to Y")
    }));
}

#[test]
fn modern_profile_forwards_final_word_arg_directly_into_a_x() {
    let source = "CARD ARRAY w BYTE i PROC Sink=*(CARD value) [$85 $A0 $86 $A1 $60] \
            PROC Main() i=1 Sink(w(i)+1) i=2 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(!modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::LDA_ZP
            && bytes[1] == runtime_zp::ARGS.address()
            && bytes[2] == opcode::LDA_ZP
            && bytes[3] == runtime_zp::ARGS.address()
    }));
    assert!(modern.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::PHA
            && bytes[1] == opcode::INY
            && bytes[2] == opcode::LDA_IZY
            && bytes[4] == opcode::ADC_IMM
            && bytes[6] == opcode::TAX
            && bytes[7] == opcode::PLA
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStackForwarded
            && optimization
                .message
                .contains("through stack directly into A/X")
    }));
}

#[test]
fn modern_profile_forwards_staged_word_lvalue_directly_into_a_x() {
    let source = "CARD ARRAY w BYTE i,j PROC Sink=*(CARD value) [$85 $A0 $86 $A1 $60] \
            PROC Main() i=1 j=2 Sink(w(i+j)) i=3 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::LDA_IZY && bytes[2] == opcode::TAX && bytes[3] == opcode::DEY
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization
                .message
                .contains("word lvalue directly into A/X")
    }));
}

#[test]
fn modern_profile_forwards_first_word_lvalue_before_direct_y_arg() {
    let source = "CARD ARRAY w BYTE i,y PROC Sink=*(CARD value, BYTE yy) [$85 $A0 $86 $A1 $84 $A2 $60] \
            PROC Main() i=1 y=17 Sink(w(i+1),y) i=3 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(modern.bytes.windows(7).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[2] == opcode::TAX
            && bytes[3] == opcode::DEY
            && bytes[4] == opcode::LDA_IZY
            && bytes[6] == opcode::LDY_ABS
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization
                .message
                .contains("word lvalue directly into A/X")
    }));
}

#[test]
fn modern_profile_forwards_second_byte_expression_before_direct_y_arg() {
    let source = "BYTE x PROC Sink=*(BYTE first, BYTE second, BYTE third) [$85 $A0 $86 $A1 $84 $A2 $60] \
            PROC Main() x=5 Sink(0,x-1,0) x=7 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(modern.bytes.windows(7).any(|bytes| {
        bytes[0] == opcode::SBC_IMM
            && bytes[2] == opcode::TAX
            && bytes[3] == opcode::LDY_IMM
            && bytes[4] == 0
            && bytes[5] == opcode::TYA
            && bytes[6] == opcode::JSR_ABS
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization.message.contains("A to X")
    }));
}

#[test]
fn modern_profile_keeps_array_call_argument_before_computed_y_arg() {
    let source = "BYTE a,i CARD ARRAY w PROC Store=*(CARD p, BYTE value) [$85 $A0 $86 $A1 $98 $A0 $00 $91 $A0 $60] \
            PROC Main() i=1 a=$80 Store(w(i),a!$7F) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    let pointer_stage = modern.bytes.windows(4).position(|bytes| {
        bytes[0] == opcode::LDA_IZY && bytes[2] == opcode::TAX && bytes[3] == opcode::DEY
    });
    let computed_y = modern.bytes.windows(4).position(|bytes| {
        bytes[0] == opcode::EOR_IMM
            && bytes[1] == 0x7F
            && bytes[2] == opcode::TAY
            && bytes[3] == opcode::PLA
    });

    assert!(pointer_stage.is_some());
    assert!(computed_y.is_some());
    assert!(pointer_stage.unwrap() < computed_y.unwrap());
}

#[test]
fn modern_profile_forwards_word_high_before_computed_y_arg() {
    let source = "BYTE a,i CARD ARRAY w PROC Store=*(CARD p, BYTE value) [$85 $A0 $86 $A1 $98 $A0 $00 $91 $A0 $60] \
            PROC Main() i=1 a=$80 Store(w(i),a!$7F) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(modern.bytes.windows(8).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[2] == opcode::TAX
            && bytes[3] == opcode::DEY
            && bytes[4] == opcode::LDA_IZY
            && bytes[6] == opcode::PHA
            && bytes[7] == opcode::LDA_ABS
    }));
    assert!(modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::EOR_IMM
            && bytes[1] == 0x7F
            && bytes[2] == opcode::TAY
            && bytes[3] == opcode::PLA
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStackForwarded
            && optimization.message.contains("kept low byte on stack")
    }));
}

#[test]
fn modern_profile_forwards_word_high_before_direct_y_and_late_constant_arg() {
    let source = "BYTE y,i CARD ARRAY w PROC Store=*(CARD p, BYTE yy, BYTE flag) [$85 $A0 $86 $A1 $84 $A2 $60] \
            PROC Main() i=1 y=17 Store(w(i),y,0) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| { bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address()] })
    );
    assert!(modern.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::LDA_IZY
            && bytes[2] == opcode::TAX
            && bytes[3] == opcode::LDA_IMM
            && bytes[4] == 0
            && bytes[5] == opcode::STA_ZP
            && bytes[6] == runtime_zp::ARGS.offset(3).address()
            && bytes[7] == opcode::DEY
            && bytes[8] == opcode::LDA_IZY
            && bytes[10] == opcode::LDY_ABS
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization.message.contains("late constants")
    }));
}

#[test]
fn modern_profile_reloads_staged_y_arg_after_late_pointer_arg() {
    let source = "BYTE c,files,ch CHAR FUNC Range(CHAR ARRAY s, BYTE max, BYTE POINTER p) RETURN(max) \
            PROC Main() files=9 c=0 ch=Range(\"-=\",files-1,@c) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    let staged_max = modern.bytes.windows(4).position(|bytes| {
        bytes
            == [
                opcode::SBC_IMM,
                1,
                opcode::STA_ZP,
                runtime_zp::ARGS.offset(2).address(),
            ]
    });
    let staged_pointer_low = modern
        .bytes
        .windows(2)
        .position(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(3).address()]);
    let y_reload = modern
        .bytes
        .windows(2)
        .position(|bytes| bytes == [opcode::LDY_ZP, runtime_zp::ARGS.offset(2).address()]);

    assert!(
        staged_max.is_some(),
        "computed max argument should be staged in $A2; bytes={:02X?}",
        modern.bytes
    );
    assert!(
        staged_pointer_low.is_some(),
        "late pointer argument should be staged after max; bytes={:02X?}",
        modern.bytes
    );
    assert!(
        y_reload.is_some(),
        "Y must be reloaded from staged max after late pointer staging; bytes={:02X?}",
        modern.bytes
    );
    assert!(staged_max.unwrap() < staged_pointer_low.unwrap());
    assert!(staged_pointer_low.unwrap() < y_reload.unwrap());
}

#[test]
fn modern_profile_reuses_accumulator_for_staged_high_byte_zero() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(0);
    generator.emit_sta_zero_page(runtime_zp::ARGS.offset(1));
    generator.emit_load_staged_call_registers(2, &StagedCallRegisterPlan::default());

    assert!(generator.emitter.bytes.windows(5).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ARGS.offset(1).address()
            && bytes[2] == opcode::TAX
            && bytes[3] == opcode::LDA_ZP
            && bytes[4] == runtime_zp::ARGS.address()
    }));
    assert!(generator.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::RegisterReloadRemoved
            && optimization.message.contains("TAX")
    }));
}

#[test]
fn modern_profile_forwards_zero_extended_first_word_arg_directly() {
    let source = "BYTE a,b PROC Sink=*(CARD value) [$85 $A0 $86 $A1 $60] \
            PROC Main() a=3 b=4 Sink(a+b) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(7).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ARGS.address()
            && bytes[2] == opcode::LDA_IMM
            && bytes[3] == 0
            && bytes[4] == opcode::STA_ZP
            && bytes[5] == runtime_zp::ARGS.offset(1).address()
            && bytes[6] == opcode::LDX_ZP
    }));
    assert!(modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::LDX_IMM
            && bytes[1] == 0
            && matches!(bytes[2], opcode::JSR_ABS | opcode::JMP_ABS)
    }));
    assert!(!modern.bytes.windows(4).any(|bytes| {
        bytes[0] == opcode::PHA
            && bytes[1] == opcode::LDX_IMM
            && bytes[2] == 0
            && bytes[3] == opcode::PLA
    }));
    assert!(!modern.bytes.windows(9).any(|bytes| {
        bytes[0] == opcode::STA_ZP
            && bytes[1] == runtime_zp::ARGS.address()
            && bytes[2] == opcode::LDA_IMM
            && bytes[3] == 0
            && bytes[4] == opcode::STA_ZP
            && bytes[5] == runtime_zp::ARGS.offset(1).address()
            && bytes[6] == opcode::TAX
            && bytes[7] == opcode::LDA_ZP
            && bytes[8] == runtime_zp::ARGS.address()
    }));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStackForwarded
            && optimization
                .message
                .contains("zero-extended staged first argument")
    }));
}

#[test]
fn modern_profile_forwards_final_word_arg_directly_into_x_y() {
    let source = "BYTE a,i CARD ARRAY w PROC Sink=*(BYTE first, CARD second) [$85 $A0 $86 $A1 $84 $A2 $60] \
            PROC Main() a=7 i=1 Sink(a,w(i)+1) i=2 RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(2).any(|bytes| {
        bytes[0] == opcode::STA_ZP && bytes[1] == runtime_zp::ARGS.offset(1).address()
    }));
    assert!(compatible.bytes.windows(2).any(|bytes| {
        bytes[0] == opcode::LDX_ZP && bytes[1] == runtime_zp::ARGS.offset(1).address()
    }));
    assert!(compatible.bytes.windows(2).any(|bytes| {
        bytes[0] == opcode::STA_ZP && bytes[1] == runtime_zp::ARGS.offset(2).address()
    }));
    assert!(compatible.bytes.windows(2).any(|bytes| {
        bytes[0] == opcode::LDY_ZP && bytes[1] == runtime_zp::ARGS.offset(2).address()
    }));
    assert!(!modern.bytes.windows(2).any(|bytes| {
        bytes[0] == opcode::STA_ZP && bytes[1] == runtime_zp::ARGS.offset(1).address()
    }));
    assert!(!modern.bytes.windows(2).any(|bytes| {
        bytes[0] == opcode::STA_ZP && bytes[1] == runtime_zp::ARGS.offset(2).address()
    }));
    assert!(modern.bytes.contains(&opcode::TAX));
    assert!(modern.bytes.contains(&opcode::TAY));
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization.message.contains("X/Y")
    }));
}

#[test]
fn compatible_machine_block_effects_avoid_preserving_untouched_zero_page_pointer() {
    let source = "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen \
            SET $491=$3000 SET $E=$3000 \n;@actionc preserves $AE/$AF\n;@actionc clobbers $A0\n;@actionc returns A=$A0\nBYTE FUNC Internal=*(BYTE ch) [$85 $A0 $60] \
            PROC Main() screen^=Internal(1) RETURN";
    let output = generate_compatible_source_with_origin(source, 0x3000).unwrap();

    assert!(!output.bytes.contains(&opcode::PHA));
    assert!(!output.bytes.contains(&opcode::PLA));
    assert!(output.bytes.windows(8).any(|bytes| bytes
        == [
            opcode::JSR_ABS,
            0x00,
            0x30,
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDY_IMM,
            0x00,
            opcode::STA_IZY,
        ]));
}

#[test]
fn modern_unannotated_machine_block_is_call_effect_barrier() {
    let source = "BYTE z=$C0 PROC Touch=*() [$85 $C0 $60] PROC Main() z=1 Touch() z=1 RETURN";
    let output =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    let stores_to_z = output
        .bytes
        .windows(2)
        .filter(|bytes| {
            matches!(bytes[0], opcode::STA_ZP | opcode::STX_ZP | opcode::STY_ZP) && bytes[1] == 0xC0
        })
        .count();
    assert!(
        stores_to_z >= 2,
        "unannotated machine block must invalidate memory facts; bytes={:02X?}",
        output.bytes
    );
}

#[test]
fn modern_profile_uses_annotated_accumulator_return_fact() {
    let source = "BYTE ARRAY s BYTE x ;@actionc returns A=$A0\nBYTE FUNC Internal=*(BYTE ch) [$85 $A0 $60] PROC Main() x=Internal(s(0)) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| { bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::STA_ABS })
    );
    assert!(
        !modern
            .bytes
            .windows(4)
            .any(|bytes| { bytes == [opcode::JSR_ABS, bytes[1], bytes[2], opcode::LDA_ZP,] })
    );
}

#[test]
fn modern_profile_infers_byte_function_accumulator_return_fact() {
    let source = "BYTE out BYTE FUNC Id(BYTE v) RETURN(v) PROC Main() out=Id(7) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(5)
            .any(|bytes| bytes[0] == opcode::JSR_ABS
                && bytes[3] == opcode::LDA_ZP
                && bytes[4] == runtime_zp::ARGS.address())
    );
    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::STA_ABS)
    );
    assert_eq!(
        count_pair(&modern.bytes, opcode::LDA_ZP, runtime_zp::ARGS.address()),
        0
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::CallResultMaterializationRemoved
            && optimization.message.contains("byte return")
    }));
}

#[test]
fn modern_profile_forwards_recent_slot_store_into_late_call_argument() {
    let source = "CARD allocp=$E8 PROC MovePage=*(CARD dst, src BYTE len) [$60] PROC Main() allocp==-4 MovePage($5A, allocp, 4) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xE9, opcode::LDA_ZP, 0xE9,])
    );
    assert!(modern.bytes.windows(4).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            0xE9,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(3).address(),
        ]));
    assert!(
        !modern
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ZP, 0xE9, opcode::LDA_ZP, 0xE9,])
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStoreRemoved
            && optimization
                .message
                .contains("forwarded recently stored accumulator into call argument")
    }));
}

#[test]
fn modern_profile_forwards_single_byte_call_result_argument_without_stack() {
    let source = "BYTE FUNC B() RETURN(7) BYTE FUNC C(BYTE x) RETURN(x) PROC Main() C(B()) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::PHA, opcode::PLA])
    );
    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| { bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::JMP_ABS })
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::ArgumentStackForwarded
            && optimization
                .message
                .contains("forwarded single call result argument without stack staging")
    }));
}

#[test]
fn modern_profile_forwards_single_byte_call_result_to_word_argument_without_stack() {
    let source = "BYTE FUNC B() RETURN(7) PROC Takes(CARD c) RETURN PROC Main() Takes(B()) RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::PHA, opcode::PLA])
    );
    assert!(
        modern
            .bytes
            .windows(5)
            .any(|bytes| bytes[0] == opcode::JSR_ABS
                && bytes[3] == opcode::LDX_IMM
                && bytes[4] == 0)
    );
    assert!(!modern.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::LDX_ZP
        ]));
}

#[test]
fn modern_profile_infers_word_function_accumulator_high_return_fact() {
    let source = "CARD out CARD FUNC Id(CARD v) RETURN(v+1) PROC Main() out=Id($1234) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(5)
            .any(|bytes| bytes[0] == opcode::JSR_ABS
                && bytes[3] == opcode::LDA_ZP
                && bytes[4] == runtime_zp::ARGS.offset(1).address())
    );
    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::STA_ABS)
    );
    assert_eq!(
        count_pair(
            &modern.bytes,
            opcode::LDA_ZP,
            runtime_zp::ARGS.offset(1).address()
        ),
        0
    );
}

#[test]
fn modern_profile_stores_word_return_low_byte_directly_when_in_accumulator() {
    let source = "CARD out CARD FUNC Id(CARD v) RETURN(v) PROC Main() out=Id($1234) RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(5)
            .any(|bytes| bytes[0] == opcode::JSR_ABS
                && bytes[3] == opcode::LDA_ZP
                && bytes[4] == runtime_zp::ARGS.offset(1).address())
    );
    assert!(
        compatible
            .bytes
            .windows(5)
            .any(|bytes| bytes[0] == opcode::STA_ABS
                && bytes[3] == opcode::LDA_ZP
                && bytes[4] == runtime_zp::ARGS.address())
    );
    assert!(
        modern
            .bytes
            .windows(4)
            .any(|bytes| bytes[0] == opcode::JSR_ABS && bytes[3] == opcode::STA_ABS)
    );
    assert_eq!(
        count_pair(&modern.bytes, opcode::LDA_ZP, runtime_zp::ARGS.address()),
        0
    );
    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::CallResultMaterializationRemoved
            && optimization.message.contains("low byte of word return")
    }));
}

#[test]
fn modern_profile_does_not_infer_accumulator_return_fact_for_call_through_unknown_return() {
    let source = "BYTE out BYTE FUNC Raw=*() [$A9 $01 $85 $A0 $60] BYTE FUNC Id() RETURN(Raw()) PROC Main() out=Id() RETURN";
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        modern
            .bytes
            .windows(5)
            .any(|bytes| bytes[0] == opcode::JSR_ABS
                && bytes[3] == opcode::LDA_ZP
                && bytes[4] == runtime_zp::ARGS.address())
    );
}

#[test]
fn modern_profile_omits_if_join_jump_after_returning_branch() {
    let source = "BYTE FUNC IsProtected(BYTE i) IF i=1 THEN RETURN(1) ELSE RETURN(0) FI";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        compatible
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::RTS, opcode::JMP_ABS])
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::RTS, opcode::JMP_ABS])
    );
    assert!(
        !modern
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::RTS, opcode::RTS])
    );
}

#[test]
fn modern_profile_inverts_short_false_branches() {
    let compatible = generate_profile_source_with_origin(
        "BYTE a,b PROC Main() IF a THEN b=1 FI RETURN",
        0x3000,
        CodegenProfile::Compat,
    )
    .unwrap();
    let modern = generate_profile_source_with_origin(
        "BYTE a,b PROC Main() IF a THEN b=1 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(compatible.bytes.contains(&opcode::JMP_ABS));
    assert!(!modern.bytes.contains(&opcode::JMP_ABS));
    assert!(
        modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::BranchInverted })
    );
}

#[test]
fn modern_profile_fixed_point_inverts_empty_while_backedge() {
    let source = "BYTE a PROC Main() WHILE a DO OD RETURN";
    let compatible =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Compat).unwrap();
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(compatible.bytes.windows(5).any(|bytes| {
        invert_branch_opcode(bytes[0]).is_some() && bytes[1] == 3 && bytes[2] == opcode::JMP_ABS
    }));
    assert!(
        !modern.bytes.windows(5).any(|bytes| {
            invert_branch_opcode(bytes[0]).is_some() && bytes[1] == 3 && bytes[2] == opcode::JMP_ABS
        }),
        "modern bytes: {:02X?}; optimizations: {:?}",
        modern.bytes,
        modern.optimizations
    );
    assert_eq!(
        modern
            .optimizations
            .iter()
            .filter(|optimization| {
                optimization.routine.as_deref() == Some("Main")
                    && optimization.kind == CodegenOptimizationKind::BranchInverted
            })
            .count(),
        2,
        "the routine-end fixed point should remove the shape exposed by eager inversion"
    );
}

#[test]
fn modern_profile_inverts_dynamic_for_exit_branch_over_jump() {
    let source = concat!(
        "BYTE i,n BYTE FUNC Flag(BYTE x) RETURN(x) ",
        "BYTE FUNC Main(BYTE c) FOR i=c TO n-1 DO ",
        "IF Flag(i) THEN RETURN(i) FI OD RETURN(255)"
    );
    let modern =
        generate_profile_source_with_origin(source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(modern.optimizations.iter().any(|optimization| {
        optimization.routine.as_deref() == Some("Main")
            && optimization.kind == CodegenOptimizationKind::BranchInverted
            && optimization.message.contains("fallthrough label")
    }));
}

#[test]
fn modern_routine_finalizer_inverts_structured_branch_over_jump() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);
    generator
        .emitter
        .emit_branch_label(opcode::BNE_REL, "fallthrough", span);
    generator
        .emitter
        .bind_label("compare:done:0", span)
        .unwrap();
    generator
        .emitter
        .bind_label("condition:false:1", span)
        .unwrap();
    generator.emitter.emit_jmp_label("target", span);
    generator.emitter.bind_label("fallthrough", span).unwrap();
    generator.emitter.emit_u8(0xEA);
    generator.emitter.bind_label("target", span).unwrap();

    generator.finalize_modern_branch_inversions(0);

    assert_eq!(generator.emitter.bytes, [opcode::BEQ_REL, 0x00, 0xEA]);
    assert!(generator.emitter.patches.iter().any(|patch| {
        patch.offset == 1 && patch.kind == PatchKind::Relative8 && patch.label == "target"
    }));
    assert!(!generator.emitter.labels.contains_key("compare:done:0"));
    assert!(!generator.emitter.labels.contains_key("condition:false:1"));
    assert!(generator.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::BranchInverted
            && optimization.message.contains("fallthrough label")
    }));
}

#[test]
fn modern_debug_compat_routine_keeps_branch_over_jump_shape() {
    let modern = generate_profile_source_with_origin(
        "BYTE a ;@actionc profile compat\nPROC Main() WHILE a DO OD RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(modern.bytes.windows(5).any(|bytes| {
        invert_branch_opcode(bytes[0]).is_some() && bytes[1] == 3 && bytes[2] == opcode::JMP_ABS
    }));
    assert!(!modern.optimizations.iter().any(|optimization| {
        optimization.routine.as_deref() == Some("Main")
            && optimization.kind == CodegenOptimizationKind::BranchInverted
    }));
}

#[test]
fn modern_profile_adjusts_machine_block_metadata_after_late_branch_inversion() {
    let modern = generate_profile_source_with_origin(
        "BYTE a PROC Main() WHILE a DO OD [$EA $60]",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    let machine_block = modern
        .map
        .machine_blocks
        .first()
        .expect("missing machine block analysis");
    let machine_offset = machine_block.address.wrapping_sub(modern.origin) as usize;
    assert_eq!(modern.bytes.get(machine_offset), Some(&0xEA));
    assert!(modern.map.source_ranges.iter().any(|range| {
        range.kind == CodegenSourceRangeKind::MachineBlock
            && range.start == machine_block.address
            && range.end == machine_block.address + 2
    }));
}

#[test]
fn modern_profile_does_not_delete_labeled_jump_instruction() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);
    generator
        .emitter
        .emit_branch_label(opcode::BEQ_REL, "fallthrough", span);
    generator.emitter.bind_label("jump-entry", span).unwrap();
    generator.emitter.emit_jmp_label("target", span);
    generator.emitter.bind_label("fallthrough", span).unwrap();
    generator.emitter.emit_u8(0xEA);
    generator.emitter.bind_label("target", span).unwrap();

    generator.finalize_modern_branch_inversions(0);

    assert_eq!(generator.emitter.bytes[2], opcode::JMP_ABS);
    assert!(generator.optimizations.is_empty());
}

#[test]
fn modern_profile_does_not_rewrite_across_resolved_machine_branch() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);
    generator.emitter.emit_u8(opcode::BNE_REL);
    generator.emitter.emit_u8(8);
    generator.emitter.emit_u8(0xEA);
    generator.emitter.emit_u8(0xEA);
    generator
        .emitter
        .emit_branch_label(opcode::BEQ_REL, "fallthrough", span);
    generator.emitter.emit_jmp_label("target", span);
    generator.emitter.bind_label("fallthrough", span).unwrap();
    generator.emitter.emit_u8(0xEA);
    generator.emitter.bind_label("target", span).unwrap();
    generator.source_ranges.push(CodegenSourceRange {
        kind: CodegenSourceRangeKind::MachineBlock,
        name: Some("machine block".to_string()),
        source_span: span,
        start: 0x3000,
        end: 0x3002,
    });

    generator.finalize_modern_branch_inversions(0);

    assert_eq!(generator.emitter.bytes[6], opcode::JMP_ABS);
    assert!(generator.optimizations.is_empty());
}

#[test]
fn modern_profile_signed_zero_compare_uses_high_byte_sign_branch() {
    let output = generate_profile_source_with_origin(
        "INT n BYTE x PROC Main() IF n<0 THEN x=1 FI IF n>=0 THEN x=2 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::BPL_REL, bytes[4],])
    );
    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::BMI_REL, bytes[4],])
    );
    assert!(
        !output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x00, 0x30, opcode::CMP_IMM, 0x00,])
    );
}

#[test]
fn modern_profile_reversed_signed_zero_compare_uses_high_byte_sign_branch() {
    let output = generate_profile_source_with_origin(
        "INT n BYTE x PROC Main() IF 0>n THEN x=1 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x01, 0x30, opcode::BPL_REL, bytes[4],])
    );
}

#[test]
fn modern_profile_signed_zero_gt_and_le_keep_composite_branch_shape() {
    let output = generate_profile_source_with_origin(
        "INT n BYTE gt,le PROC Main() IF n>0 THEN gt=1 FI IF n<=0 THEN le=1 FI RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(output.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::LDA_ABS
            && bytes[1] == 0x01
            && bytes[2] == 0x30
            && bytes[3] == opcode::BMI_REL
            && bytes[5] == opcode::BNE_REL
            && bytes[7] == opcode::LDA_ABS
            && bytes[8] == 0x00
            && bytes[9] == 0x30
            && bytes[10] == opcode::BNE_REL
    }));
    assert!(output.bytes.windows(12).any(|bytes| {
        bytes[0] == opcode::LDA_ABS
            && bytes[1] == 0x01
            && bytes[2] == 0x30
            && bytes[3] == opcode::BMI_REL
            && bytes[5] == opcode::BNE_REL
            && bytes[7] == opcode::LDA_ABS
            && bytes[8] == 0x00
            && bytes[9] == 0x30
            && bytes[10] == opcode::BEQ_REL
    }));
}

#[test]
fn modern_profile_signed_zero_materialized_compare_uses_shared_slot_branch() {
    let output = generate_profile_source_with_origin(
            "INT FUNC Id(INT n) RETURN(n) BYTE x PROC Main() IF Id(1)>=0 THEN x=1 FI IF Id(1)<=0 THEN x=2 FI RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert!(output.bytes.windows(4).any(|bytes| {
        bytes
            == [
                opcode::LDA_ZP,
                runtime_zp::ARGS.offset(1).address(),
                opcode::BMI_REL,
                bytes[3],
            ]
    }));
    assert!(output.bytes.windows(10).any(|bytes| {
        bytes[0] == opcode::LDA_ZP
            && bytes[1] == runtime_zp::ARGS.offset(1).address()
            && bytes[2] == opcode::BMI_REL
            && bytes[4] == opcode::BNE_REL
            && bytes[6] == opcode::LDA_ZP
            && bytes[7] == runtime_zp::ARGS.address()
            && bytes[8] == opcode::BEQ_REL
    }));
}

#[test]
fn modern_profile_specializes_canonical_abs_return_routine() {
    let output = generate_profile_source_with_origin(
        "INT FUNC Abs(INT n) IF n<0 THEN RETURN(-n) FI RETURN(n)",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output.bytes,
        vec![
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::STX_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::TXA,
            opcode::BPL_REL,
            0x0D,
            opcode::SEC,
            opcode::LDA_IMM,
            0x00,
            opcode::SBC_ZP,
            runtime_zp::ARGS.address(),
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0x00,
            opcode::SBC_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::RTS,
        ]
    );
}

#[test]
fn modern_profile_keeps_long_false_branches_when_out_of_range() {
    let mut source = "BYTE a,b PROC Main() IF a THEN ".to_string();
    for _ in 0..80 {
        source.push_str("b==+1 ");
    }
    source.push_str("FI RETURN");
    let modern =
        generate_profile_source_with_origin(&source, 0x3000, CodegenProfile::Modern).unwrap();

    assert!(
        !modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::BranchInverted })
    );
    assert!(
        !modern
            .optimizations
            .iter()
            .any(|optimization| { optimization.kind == CodegenOptimizationKind::JumpToRtsRemoved })
    );
}

#[test]
fn modern_profile_reuses_prepared_pointer_and_array_index_addresses() {
    let output = generate_profile_source_with_origin(
            "BYTE ARRAY a(8) BYTE POINTER p BYTE i,x,y,z PROC Main() i=2 x=a(i) y=a(i) x=p(i) y=p(i) z=0 RETURN",
            0x3000,
            CodegenProfile::Modern,
        )
        .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| {
                *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()]
                    || *bytes == [opcode::STY_ZP, runtime_zp::ARRAY_ADDR.address()]
            })
            .count(),
        2
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.offset(1).address()])
            .count(),
        2
    );
}

#[test]
fn modern_profile_suppresses_redundant_known_pointer_reload() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE x,y PROC Main() p=$4000 x=p^ y=p^ RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.offset(1).address()])
            .count(),
        1
    );
}

#[test]
fn modern_profile_suppresses_known_pointer_reload_after_preserving_call() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p PROC Keep() RETURN PROC Main() p=$4000 p^=1 Keep() p^=2 RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.offset(1).address()])
            .count(),
        1
    );
}

#[test]
fn modern_profile_logs_pointer_reload_optimization() {
    let mut generator = test_generator(CodegenProfile::Modern);
    generator
        .local_symbols
        .insert(normalize_name("p"), StorageSlot::pointer(0x3000, 1));
    let expr = Expr {
        kind: ExprKind::Unary {
            op: UnaryOp::Deref,
            expr: Box::new(test_name_expr("p")),
        },
        text: "p^".to_string(),
        span: Span::new(0, 2),
    };
    let fact = generator.prepared_pointer_fact(&expr).unwrap();
    generator
        .processor
        .mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact);

    let slot = generator
        .reusable_lvalue_slot_with_pointer(&expr, runtime_zp::ARRAY_ADDR)
        .unwrap();

    assert_eq!(slot.space, AddressSpace::IndirectIndexedY);
    assert!(generator.optimizations.iter().any(|optimization| {
        optimization.kind == CodegenOptimizationKind::PointerReloadRemoved
    }));
}

#[test]
fn modern_profile_reloads_pointer_after_known_pointer_change() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE x,y PROC Main() p=$4000 x=p^ p=$4100 y=p^ RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| {
                *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()]
                    || *bytes == [opcode::STY_ZP, runtime_zp::ARRAY_ADDR.address()]
            })
            .count(),
        2
    );
    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.offset(1).address()])
            .count(),
        2
    );
}

#[test]
fn modern_profile_reuses_accumulator_for_matching_slot_index_load() {
    let output = generate_profile_source_with_origin(
        "BYTE ARRAY ba(64) BYTE i PROC ARRAYTEST() FOR i=0 TO 16 DO ba(i)=i OD RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::LDA_ABS, 0x40, 0x30, opcode::TAX])
    );
    assert!(
        !output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [opcode::LDX_ABS, 0x40, 0x30])
    );
}

#[test]
fn modern_profile_reuses_matching_slot_index_load_when_flags_were_clobbered() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_slot_byte(slot, 0);
    generator.emit_cmp_imm(0);
    generator.emit_ldx_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_IMM,
            0x00,
            opcode::TAX,
        ]
    );
}

#[test]
fn modern_profile_reuses_accumulator_for_matching_slot_y_load() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_slot_byte(slot, 0);
    generator.emit_ldy_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDA_ABS, 0x00, 0x30, opcode::TAY]
    );
}

#[test]
fn modern_profile_reuses_matching_slot_y_load_when_flags_were_clobbered() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_slot_byte(slot, 0);
    generator.emit_cmp_imm(0);
    generator.emit_ldy_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::CMP_IMM,
            0x00,
            opcode::TAY,
        ]
    );
}

#[test]
fn modern_profile_reuses_x_for_matching_accumulator_load() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_lda_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDX_ABS, 0x00, 0x30, opcode::TXA]
    );
}

#[test]
fn modern_profile_reuses_y_for_matching_accumulator_load() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldy_slot_byte(slot, 0);
    generator.emit_lda_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDY_ABS, 0x00, 0x30, opcode::TYA]
    );
}

#[test]
fn modern_profile_reuses_matching_accumulator_load_when_flags_were_clobbered() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_cmp_imm(0);
    generator.emit_lda_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDX_ABS,
            0x00,
            0x30,
            opcode::CMP_IMM,
            0x00,
            opcode::TXA,
        ]
    );
}

#[test]
fn modern_profile_suppresses_redundant_matching_x_slot_load() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_ldx_slot_byte(slot, 0);

    assert_eq!(generator.emitter.bytes, [opcode::LDX_ABS, 0x00, 0x30]);
}

#[test]
fn modern_profile_suppresses_redundant_matching_y_slot_load() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldy_slot_byte(slot, 0);
    generator.emit_ldy_slot_byte(slot, 0);

    assert_eq!(generator.emitter.bytes, [opcode::LDY_ABS, 0x00, 0x30]);
}

#[test]
fn modern_profile_keeps_redundant_matching_x_slot_load_when_flags_were_clobbered() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_cmp_imm(0);
    generator.emit_ldx_slot_byte(slot, 0);

    assert_eq!(
        generator
            .emitter
            .bytes
            .windows(3)
            .filter(|bytes| *bytes == [opcode::LDX_ABS, 0x00, 0x30])
            .count(),
        2
    );
}

#[test]
fn modern_optimization_torture_reuses_straight_line_slot_loads() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_slot_byte(slot, 0);
    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_ldy_slot_byte(slot, 0);
    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_ldy_slot_byte(slot, 0);
    generator.emit_lda_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::TAX,
            opcode::TAY,
            opcode::TXA
        ]
    );
}

#[test]
fn modern_optimization_torture_reuses_after_same_slot_store() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_lda_imm(7);
    generator.emit_sta_slot_byte(slot, 0);
    generator.emit_ldx_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDX_ABS,
            0x00,
            0x30,
            opcode::LDA_IMM,
            0x07,
            opcode::STA_ABS,
            0x00,
            0x30,
            opcode::TAX,
        ]
    );
}

#[test]
fn modern_optimization_torture_reloads_after_call_boundary() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldx_slot_byte(slot, 0);
    generator.processor.invalidate_after_call();
    generator.emitter.emit_jsr_absolute(Absolute::new(0x4000));
    generator.emit_ldx_slot_byte(slot, 0);

    assert_eq!(
        generator
            .emitter
            .bytes
            .windows(3)
            .filter(|bytes| *bytes == [opcode::LDX_ABS, 0x00, 0x30])
            .count(),
        2
    );
}

#[test]
fn modern_optimization_torture_reloads_after_label_join() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_ldy_slot_byte(slot, 0);
    generator.bind_codegen_label("join".to_string(), Span::new(0, 0));
    generator.emit_ldy_slot_byte(slot, 0);

    assert_eq!(
        generator
            .emitter
            .bytes
            .windows(3)
            .filter(|bytes| *bytes == [opcode::LDY_ABS, 0x00, 0x30])
            .count(),
        2
    );
}

#[test]
fn modern_optimization_does_not_forward_recent_a_store_across_label() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);
    let source = StorageSlot::zero_page(0xE1, 1);
    let target = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 1);

    generator.emit_lda_imm(9);
    generator.emit_sta_slot_byte(source, 0);
    generator.bind_codegen_label("loop".to_string(), span);
    generator.emit_copy_slot_byte_to_slot_byte(source, 0, target, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_IMM,
            9,
            opcode::STA_ZP,
            0xE1,
            opcode::LDA_ZP,
            0xE1,
            opcode::STA_ZP,
            runtime_zp::ELEMENT_ADDR.address(),
        ]
    );
}

#[test]
fn modern_profile_reuses_straight_line_label_cache_load() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let span = Span::new(0, 0);

    generator.emit_lda_imm(0x12);
    generator.emit_sta_absolute_label("cache", span);
    generator.emit_lda_absolute_label("cache", span);
    generator.bind_codegen_label("cache".to_string(), span);
    generator.emitter.emit_u8(0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDA_IMM, 0x12, opcode::STA_ABS, 0x00, 0x00, 0x00]
    );
    assert_eq!(
        generator.optimizations[0].kind,
        CodegenOptimizationKind::RegisterReloadRemoved
    );
}

#[test]
fn compatible_profile_keeps_straight_line_label_cache_load() {
    let mut generator = test_generator(CodegenProfile::Compat);
    let span = Span::new(0, 0);

    generator.emit_lda_imm(0x12);
    generator.emit_sta_absolute_label("cache", span);
    generator.emit_lda_absolute_label("cache", span);
    generator.bind_codegen_label("cache".to_string(), span);
    generator.emitter.emit_u8(0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_IMM,
            0x12,
            opcode::STA_ABS,
            0x00,
            0x00,
            opcode::LDA_ABS,
            0x00,
            0x00,
            0x00,
        ]
    );
}

#[test]
fn modern_optimization_torture_compatible_profile_keeps_load_shape() {
    let mut generator = test_generator(CodegenProfile::Compat);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_slot_byte(slot, 0);
    generator.emit_ldx_slot_byte(slot, 0);
    generator.emit_ldy_slot_byte(slot, 0);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::LDX_ABS,
            0x00,
            0x30,
            opcode::LDY_ABS,
            0x00,
            0x30
        ]
    );
}

#[test]
fn modern_profile_reuses_prepared_pointer_when_only_index_changes() {
    let output = generate_profile_source_with_origin(
        "BYTE POINTER p BYTE i,x,y PROC Main() p=$4000 i=2 x=p(i) i=3 y=p(i) RETURN",
        0x3000,
        CodegenProfile::Modern,
    )
    .unwrap();

    assert_eq!(
        output
            .bytes
            .windows(2)
            .filter(|bytes| *bytes == [opcode::STA_ZP, runtime_zp::ARRAY_ADDR.address()])
            .count(),
        1
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [opcode::STA_ABS, 0x02, 0x30, opcode::TAY])
    );
}

#[test]
fn processor_state_tracks_y_immediates() {
    let mut state = ProcessorState::default();
    assert_eq!(state.a_immediate(), None);
    assert_eq!(state.x_immediate(), None);
    assert_eq!(state.y_immediate(), None);

    state.set_a_immediate(5);
    state.set_x_immediate(6);
    state.set_y_immediate(7);
    assert_eq!(state.a_immediate(), Some(5));
    assert_eq!(state.x_immediate(), Some(6));
    assert_eq!(state.y_immediate(), Some(7));

    state.clear_a();
    state.clear_x();
    state.clear_y();
    assert_eq!(state.a_immediate(), None);
    assert_eq!(state.x_immediate(), None);
    assert_eq!(state.y_immediate(), None);

    state.set_y_hint(Some(3));
    assert_eq!(state.y_immediate(), Some(3));

    state.reset();
    assert_eq!(state.a_immediate(), None);
    assert_eq!(state.x_immediate(), None);
    assert_eq!(state.y_immediate(), None);
}

#[test]
fn processor_state_tracks_action_zero_page_registers() {
    let mut state = ProcessorState::default();

    state.set_a_immediate(0x44);
    state.set_zp_from_a(runtime_zp::ARGS);
    assert_eq!(state.zp_immediate(runtime_zp::ARGS), Some(0x44));
    assert_eq!(state.zp_immediate(ZeroPage::new(0x10)), None);

    state.set_x_immediate(0x55);
    state.set_zp_from_x(runtime_zp::ARGS.offset(1));
    assert_eq!(state.zp_immediate(runtime_zp::ARGS.offset(1)), Some(0x55));

    state.invalidate_zp(runtime_zp::ARGS);
    assert_eq!(state.zp_immediate(runtime_zp::ARGS), None);
    assert_eq!(state.zp_immediate(runtime_zp::ARGS.offset(1)), Some(0x55));

    state.invalidate_all_zp();
    assert_eq!(state.zp_immediate(runtime_zp::ARGS.offset(1)), None);
}

#[test]
fn processor_state_preserves_stable_zero_page_facts_after_known_calls() {
    let mut state = ProcessorState::default();
    state.set_a_immediate(0x44);
    state.set_zp_from_a(runtime_zp::ARGS);
    state.set_zp_value(
        runtime_zp::ARGS.offset(1),
        RegisterValue::Fact(ValueFact::AddressByte {
            address: 0x3456,
            byte_index: 1,
        }),
    );
    state.set_zp_value(
        runtime_zp::ARRAY_ADDR,
        RegisterValue::Fact(ValueFact::SlotByte {
            slot: StorageSlot::absolute(0x3000, 1),
            byte_index: 0,
        }),
    );

    let mut effects = RoutineEffects::known_empty();
    effects.record_zero_page_write(runtime_zp::ARGS.offset(1));
    state.invalidate_after_known_call(effects);

    assert_eq!(state.a_value_fact(), ValueFact::Register(RegisterName::A));
    assert_eq!(state.zp_immediate(runtime_zp::ARGS), Some(0x44));
    assert_eq!(state.zp_immediate(runtime_zp::ARGS.offset(1)), None);
    assert_eq!(
        state.zp_value(runtime_zp::ARRAY_ADDR),
        RegisterValue::Unknown
    );
}

#[test]
fn runtime_helper_effects_match_action_scratch_ranges() {
    let lsh = runtime_helper_effects(RuntimeHelperSlot::Lsh);
    assert!(lsh.known);
    assert!(lsh.writes_zero_page(ZeroPage::new(0x85)));
    assert!(!lsh.writes_zero_page(runtime_zp::ARGS));

    let mul = runtime_helper_effects(RuntimeHelperSlot::Mul);
    for address in 0x82..=0x87 {
        assert!(mul.writes_zero_page(ZeroPage::new(address)));
    }
    for address in 0xC0..=0xC2 {
        assert!(mul.writes_zero_page(ZeroPage::new(address)));
    }
    assert!(!mul.writes_zero_page(runtime_zp::ARRAY_ADDR));

    let div = runtime_helper_effects(RuntimeHelperSlot::Div);
    for address in 0x82..=0x87 {
        assert!(div.writes_zero_page(ZeroPage::new(address)));
    }
    assert!(div.writes_zero_page(ZeroPage::new(0xC2)));
    assert!(!div.writes_zero_page(ZeroPage::new(0xC0)));

    let sargs = runtime_helper_effects(RuntimeHelperSlot::SArgs);
    assert!(sargs.writes_unknown_absolute);
    for address in [0x82, 0x83, 0x84, 0x85, 0xA0, 0xA1, 0xA2] {
        assert!(sargs.writes_zero_page(ZeroPage::new(address)));
    }
}

#[test]
fn processor_state_tracks_prepared_indirect_pointers() {
    let mut state = ProcessorState::default();

    let fact = || PreparedPointerFact {
        key: "deref:IP".to_string(),
        deps: vec![PreparedDependency {
            address: 0x3000,
            size: 2,
        }],
    };

    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact());
    assert!(state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:IP"));
    assert!(!state.prepared_pointer_matches(runtime_zp::ELEMENT_ADDR, "deref:IP"));

    state.set_a_immediate(0x12);
    state.set_zp_from_a(runtime_zp::ARRAY_ADDR);
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:IP"));

    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact());
    state.invalidate_zp(runtime_zp::ARRAY_ADDR.offset(1));
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:IP"));

    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact());
    state.invalidate_prepared_pointers_touching_range(0x3001, 1);
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:IP"));

    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact());
    state.invalidate_all_zp();
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:IP"));
}

#[test]
fn processor_state_preserves_prepared_pointers_across_known_calls() {
    let mut state = ProcessorState::default();
    let zero_page_fact = PreparedPointerFact {
        key: "index:ZP:1:I".to_string(),
        deps: vec![
            PreparedDependency {
                address: 0xE0,
                size: 1,
            },
            PreparedDependency {
                address: 0xE1,
                size: 1,
            },
        ],
    };

    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, zero_page_fact.clone());
    state.invalidate_after_known_call(RoutineEffects::known_empty());
    assert!(state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "index:ZP:1:I"));

    let mut writes_dependency = RoutineEffects::known_empty();
    writes_dependency.record_zero_page_write(ZeroPage::new(0xE1));
    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, zero_page_fact.clone());
    state.invalidate_after_known_call(writes_dependency);
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "index:ZP:1:I"));

    let mut writes_pointer = RoutineEffects::known_empty();
    writes_pointer.record_zero_page_write(runtime_zp::ARRAY_ADDR);
    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, zero_page_fact);
    state.invalidate_after_known_call(writes_pointer);
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "index:ZP:1:I"));
}

#[test]
fn processor_state_preserves_absolute_prepared_deps_across_known_calls() {
    let mut state = ProcessorState::default();
    let fact = PreparedPointerFact {
        key: "deref:P".to_string(),
        deps: vec![PreparedDependency {
            address: 0x3000,
            size: 2,
        }],
    };

    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact.clone());
    state.invalidate_after_known_call(RoutineEffects::known_empty());
    assert!(state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:P"));

    let mut unrelated_write = RoutineEffects::known_empty();
    unrelated_write.record_absolute_write(0x3002, 1);
    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact.clone());
    state.invalidate_after_known_call(unrelated_write);
    assert!(state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:P"));

    let mut overlapping_write = RoutineEffects::known_empty();
    overlapping_write.record_absolute_write(0x3001, 1);
    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact.clone());
    state.invalidate_after_known_call(overlapping_write);
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:P"));

    let mut unknown_write = RoutineEffects::known_empty();
    unknown_write.record_unknown_absolute_write();
    state.mark_prepared_pointer(runtime_zp::ARRAY_ADDR, fact);
    state.invalidate_after_known_call(unknown_write);
    assert!(!state.prepared_pointer_matches(runtime_zp::ARRAY_ADDR, "deref:P"));
}

#[test]
fn processor_state_preserves_stable_memory_facts_after_known_calls() {
    let mut state = ProcessorState::default();
    let pointer = StorageSlot::pointer(0x3000, 1);
    let source = StorageSlot::absolute(0x3010, 1);
    let target = StorageSlot::absolute(0x3020, 1);

    state.set_memory_address_word(pointer, 0x4000);
    state.set_memory_byte(
        target,
        0,
        ValueFact::SlotByte {
            slot: source,
            byte_index: 0,
        },
    );

    let mut unrelated_write = RoutineEffects::known_empty();
    unrelated_write.record_absolute_write(0x3030, 1);
    state.invalidate_after_known_call(unrelated_write);

    assert_eq!(state.memory_address_word(pointer), Some(0x4000));
    assert_eq!(
        state.memory_value(target, 0),
        Some(ValueFact::SlotByte {
            slot: source,
            byte_index: 0,
        })
    );

    let mut source_write = RoutineEffects::known_empty();
    source_write.record_absolute_write(0x3010, 1);
    state.invalidate_after_known_call(source_write);

    assert_eq!(state.memory_address_word(pointer), Some(0x4000));
    assert_eq!(state.memory_value(target, 0), None);

    let mut pointer_write = RoutineEffects::known_empty();
    pointer_write.record_absolute_write(0x3001, 1);
    state.invalidate_after_known_call(pointer_write);

    assert_eq!(state.memory_address_word(pointer), None);
}

#[test]
fn processor_state_tracks_known_carry() {
    let mut generator = test_generator(CodegenProfile::Modern);

    assert_eq!(generator.processor.carry(), FlagValue::Unknown);
    generator.emit_clc();
    assert_eq!(generator.processor.carry(), FlagValue::Known(false));
    generator.emit_sec();
    assert_eq!(generator.processor.carry(), FlagValue::Known(true));
    generator.emit_adc_imm(1);
    assert_eq!(generator.processor.carry(), FlagValue::Unknown);
    generator.emit_sec();
    generator.emit_plp();
    assert_eq!(generator.processor.carry(), FlagValue::Unknown);
}

#[test]
fn processor_state_tracks_semantic_value_flags() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(0);
    assert_eq!(generator.processor.zero(), SemanticFlagFact::Known(true));
    assert_eq!(
        generator.processor.negative(),
        SemanticFlagFact::Known(false)
    );

    generator.emit_lda_imm(0x80);
    assert_eq!(generator.processor.zero(), SemanticFlagFact::Known(false));
    assert_eq!(
        generator.processor.negative(),
        SemanticFlagFact::Known(true)
    );

    generator.emit_lda_absolute(Absolute::new(0x3000));
    let expected = SemanticFlagFact::FromValue(ValueFact::SlotByte {
        slot: StorageSlot::absolute(0x3000, 1),
        byte_index: 0,
    });
    assert_eq!(generator.processor.zero(), expected);
    assert_eq!(generator.processor.negative(), expected);
}

#[test]
fn processor_state_tracks_semantic_compare_flags() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(5);
    generator.emit_cmp_imm(3);
    assert_eq!(generator.processor.carry(), FlagValue::Known(true));
    assert_eq!(generator.processor.zero(), SemanticFlagFact::Known(false));
    assert_eq!(
        generator.processor.negative(),
        SemanticFlagFact::Known(false)
    );
    assert_eq!(
        generator.processor.compare(),
        Some(CompareFact::Byte {
            left: ValueFact::Immediate(5),
            right: ValueFact::Immediate(3),
        })
    );

    let slot = StorageSlot::absolute(0x3000, 1);
    generator.emit_cmp_slot_byte(slot, 0);
    let compare = CompareFact::Byte {
        left: ValueFact::Immediate(5),
        right: ValueFact::SlotByte {
            slot,
            byte_index: 0,
        },
    };
    assert_eq!(generator.processor.carry(), FlagValue::Unknown);
    assert_eq!(
        generator.processor.zero(),
        SemanticFlagFact::FromCompare(compare)
    );
    assert_eq!(generator.processor.compare(), Some(compare));
}

#[test]
fn processor_state_tracks_register_value_provenance() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_absolute(Absolute::new(0x3000));
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::SlotByte {
            slot,
            byte_index: 0
        }
    );

    generator.emit_tax();
    assert_eq!(
        generator.processor.x_value_fact(),
        ValueFact::SlotByte {
            slot,
            byte_index: 0
        }
    );

    generator.emit_tay();
    assert_eq!(
        generator.processor.y_value_fact(),
        ValueFact::SlotByte {
            slot,
            byte_index: 0
        }
    );

    generator.emit_sta_zero_page(runtime_zp::ARGS);
    generator.emit_ldx_zero_page(runtime_zp::ARGS);
    assert_eq!(
        generator.processor.x_value_fact(),
        ValueFact::SlotByte {
            slot,
            byte_index: 0
        }
    );
}

#[test]
fn processor_state_invalidates_register_aliases_when_source_register_changes() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_ldy_imm(0);
    generator.emit_lda_indirect_indexed_y(IndirectIndexedY::new(runtime_zp::ARRAY_ADDR));
    generator.emit_tax();
    assert_eq!(
        generator.processor.x_value_fact(),
        ValueFact::Register(RegisterName::A)
    );

    generator.emit_lda_imm(0x12);

    assert_eq!(
        generator.processor.x_value_fact(),
        ValueFact::Register(RegisterName::X)
    );
}

#[test]
fn processor_state_invalidates_memory_aliases_when_source_register_changes_by_subtract() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let staged = StorageSlot::zero_page(runtime_zp::ARGS.offset(2).address(), 1);

    generator.emit_ldy_imm(0);
    generator.emit_lda_indirect_indexed_y(IndirectIndexedY::new(runtime_zp::ARRAY_ADDR));
    generator.emit_sta_slot_byte(staged, 0);
    assert_eq!(
        generator.processor.memory_value(staged, 0),
        Some(ValueFact::Register(RegisterName::A))
    );

    generator.emit_sec();
    generator.emit_sbc_imm(1);

    assert_eq!(generator.processor.memory_value(staged, 0), None);
    assert_eq!(generator.processor.a_value_fact(), ValueFact::Unknown);
}

#[test]
fn processor_state_invalidates_stored_a_aliases_after_destructive_accumulator_ops() {
    fn assert_invalidates_old_a_alias(name: &str, mut emit: impl FnMut(&mut Generator)) {
        let mut generator = test_generator(CodegenProfile::Modern);
        let staged = StorageSlot::zero_page(runtime_zp::ARGS.offset(2).address(), 1);

        generator.emit_ldy_imm(0);
        generator.emit_lda_indirect_indexed_y(IndirectIndexedY::new(runtime_zp::ARRAY_ADDR));
        generator.emit_sta_slot_byte(staged, 0);
        assert_eq!(
            generator.processor.memory_value(staged, 0),
            Some(ValueFact::Register(RegisterName::A)),
            "{name} setup should stage an old-A alias"
        );

        emit(&mut generator);

        assert_eq!(
            generator.processor.memory_value(staged, 0),
            None,
            "{name} must invalidate memory facts derived from old A"
        );
    }

    assert_invalidates_old_a_alias("LDA #imm", |generator| generator.emit_lda_imm(0x12));
    assert_invalidates_old_a_alias("LDA abs", |generator| {
        generator.emit_lda_absolute(Absolute::new(0x3000));
    });
    assert_invalidates_old_a_alias("ADC #imm", |generator| generator.emit_adc_imm(1));
    assert_invalidates_old_a_alias("SBC #imm", |generator| {
        generator.emit_sec();
        generator.emit_sbc_imm(1);
    });
    assert_invalidates_old_a_alias("AND #imm", |generator| generator.emit_and_imm(0x0f));
    assert_invalidates_old_a_alias("ORA #imm", |generator| generator.emit_ora_imm(0xf0));
    assert_invalidates_old_a_alias("EOR #imm", |generator| generator.emit_eor_imm(0xff));
    assert_invalidates_old_a_alias("ASL A", |generator| generator.emit_asl_a());
    assert_invalidates_old_a_alias("LSR A", |generator| generator.emit_lsr_a());
    assert_invalidates_old_a_alias("ROL A", |generator| generator.emit_rol_a());
    assert_invalidates_old_a_alias("ROR A", |generator| generator.emit_ror_a());
    assert_invalidates_old_a_alias("TXA", |generator| {
        generator.emit_ldx_imm(0x34);
        generator.emit_txa();
    });
    assert_invalidates_old_a_alias("TYA", |generator| generator.emit_tya());
    assert_invalidates_old_a_alias("PLA", |generator| generator.emit_pla());
}

#[test]
fn processor_state_does_not_treat_unknown_registers_as_reload_proofs() {
    let mut state = ProcessorState::default();

    assert!(state.accumulator_value_matches(ValueFact::Register(RegisterName::A)));
    assert!(state.x_value_matches(ValueFact::Register(RegisterName::X)));
    assert!(state.y_value_matches(ValueFact::Register(RegisterName::Y)));
    assert!(!state.accumulator_matches_load_result(ValueFact::Register(RegisterName::A)));
    assert!(!state.accumulator_matches_load_result(ValueFact::Unknown));

    state.set_a_fact(ValueFact::Unknown);
    state.set_x_fact(ValueFact::Unknown);
    state.set_y_fact(ValueFact::Unknown);

    assert!(!state.accumulator_value_matches(ValueFact::Unknown));
    assert!(!state.x_value_matches(ValueFact::Unknown));
    assert!(!state.y_value_matches(ValueFact::Unknown));
    assert!(!state.accumulator_matches_load_result(ValueFact::Unknown));
}

#[test]
fn array_pointer_value_load_helpers_update_processor_state() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let inline_array = StorageSlot::array(0x3456, 1, ArrayStorage::Inline);
    let pointer_array = StorageSlot::array(0x3000, 1, ArrayStorage::Pointer);

    generator.emit_lda_imm(0x99);
    generator.emit_load_array_pointer_value_slot_byte(inline_array, 0);
    assert_eq!(generator.processor.a_immediate(), Some(0x56));

    generator.emit_load_array_pointer_value_slot_byte(pointer_array, 1);
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::SlotByte {
            slot: StorageSlot::absolute(0x3001, 1),
            byte_index: 0,
        }
    );
}

#[test]
fn processor_state_tracks_accumulator_logic_result_facts() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let slot = StorageSlot::absolute(0x3000, 1);

    generator.emit_lda_absolute(Absolute::new(0x3000));
    generator.emit_and_imm(0x0f);
    let expected = ValueFact::Logic {
        op: LogicFactOp::And,
        left: ValueAtomFact::SlotByte {
            slot,
            byte_index: 0,
        },
        right: ValueAtomFact::Immediate(0x0f),
    };
    assert_eq!(generator.processor.a_value_fact(), expected);
    assert_eq!(
        generator.processor.zero(),
        SemanticFlagFact::FromValue(expected)
    );

    generator.emit_lda_imm(0xf0);
    generator.emit_eor_imm(0xff);
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::Immediate(0x0f)
    );
    assert_eq!(generator.processor.zero(), SemanticFlagFact::Known(false));
    assert_eq!(
        generator.processor.negative(),
        SemanticFlagFact::Known(false)
    );
}

#[test]
fn processor_state_tracks_word_compare_subtract_chains() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let left = StorageSlot::absolute(0x3000, 2);
    let right = StorageSlot::absolute(0x3010, 2);

    generator.emit_lda_slot_byte(left, 0);
    generator.emit_cmp_slot_byte(right, 0);
    generator.emit_lda_slot_byte(left, 1);
    generator.emit_sbc_slot_byte(right, 1);

    let low = ByteCompareFact {
        left: ValueAtomFact::SlotByte {
            slot: left,
            byte_index: 0,
        },
        right: ValueAtomFact::SlotByte {
            slot: right,
            byte_index: 0,
        },
    };
    let high = ByteCompareFact {
        left: ValueAtomFact::SlotByte {
            slot: left,
            byte_index: 1,
        },
        right: ValueAtomFact::SlotByte {
            slot: right,
            byte_index: 1,
        },
    };
    let compare = CompareFact::WordSubtract { low, high };

    assert_eq!(generator.processor.compare(), Some(compare));
    assert_eq!(
        generator.processor.negative(),
        SemanticFlagFact::FromCompare(compare)
    );
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::Subtract {
            left: high.left,
            right: high.right,
            borrow: Some(low),
        }
    );
}

#[test]
fn processor_state_tracks_memory_content_aliases() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let source = StorageSlot::absolute(0x3000, 1);
    let target = StorageSlot::absolute(0x3010, 1);

    generator.emit_lda_slot_byte(source, 0);
    generator.emit_sta_slot_byte(target, 0);
    generator.emit_lda_slot_byte(target, 0);
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::SlotByte {
            slot: source,
            byte_index: 0,
        }
    );

    generator.emit_lda_imm(7);
    generator.emit_sta_slot_byte(source, 0);
    generator.emit_lda_slot_byte(target, 0);
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::SlotByte {
            slot: target,
            byte_index: 0,
        }
    );

    generator.emit_lda_imm(3);
    generator.emit_sta_slot_byte(target, 0);
    generator.emit_lda_slot_byte(target, 0);
    assert_eq!(generator.processor.a_value_fact(), ValueFact::Immediate(3));
}

#[test]
fn processor_state_does_not_alias_indirect_indexed_loads_across_y_changes() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let source = IndirectIndexedY::new(runtime_zp::ARRAY_ADDR);

    generator.emit_ldy_imm(0);
    generator.emit_lda_indirect_indexed_y(source);
    generator.emit_sta_zero_page(runtime_zp::ARGS);
    generator.emit_iny();
    generator.emit_lda_indirect_indexed_y(source);
    generator.emit_sta_zero_page(runtime_zp::ARGS.offset(1));
    generator.emit_ldx_zero_page(runtime_zp::ARGS.offset(1));
    generator.emit_lda_zero_page(runtime_zp::ARGS);

    assert!(
        generator
            .emitter
            .bytes
            .windows(2)
            .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address()])
    );
    assert!(!generator.emitter.bytes.windows(3).any(|bytes| bytes
        == [
            opcode::LDX_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::TXA,
        ]));
}

#[test]
fn modern_profile_suppresses_redundant_slot_load_from_memory_alias() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let source = StorageSlot::absolute(0x3000, 1);
    let target = StorageSlot::absolute(0x3010, 1);

    generator.emit_lda_slot_byte(source, 0);
    generator.emit_sta_slot_byte(target, 0);
    generator.emit_lda_slot_byte(target, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ABS, 0x10, 0x30,]
    );
}

#[test]
fn modern_profile_keeps_slot_load_when_flags_were_clobbered() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let source = StorageSlot::absolute(0x3000, 1);
    let target = StorageSlot::absolute(0x3010, 1);

    generator.emit_lda_slot_byte(source, 0);
    generator.emit_sta_slot_byte(target, 0);
    generator.emit_cmp_imm(0);
    generator.emit_lda_slot_byte(target, 0);

    assert_eq!(
        generator
            .emitter
            .bytes
            .windows(3)
            .filter(|bytes| bytes[0] == opcode::LDA_ABS)
            .count(),
        2
    );
}

#[test]
fn processor_state_tracks_known_pointer_address_words() {
    let mut generator = test_generator(CodegenProfile::Compat);
    let pointer = StorageSlot::absolute(0x3000, 2);
    let copy = StorageSlot::absolute(0x3010, 2);
    let address = 0x3456;

    generator.emit_store_array_pointer_address(pointer, Absolute::new(address));
    assert_eq!(
        generator.processor.memory_address_word(pointer),
        Some(address)
    );

    generator.emit_copy_slot_to_slot(pointer, copy);
    assert_eq!(generator.processor.memory_address_word(copy), Some(address));

    generator.emit_lda_slot_byte(copy, 0);
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::AddressByte {
            address,
            byte_index: 0,
        }
    );
    generator.emit_lda_slot_byte(copy, 1);
    assert_eq!(
        generator.processor.a_value_fact(),
        ValueFact::AddressByte {
            address,
            byte_index: 1,
        }
    );
}

#[test]
fn modern_profile_reuses_tracked_zero_page_load_values() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(0x44);
    generator.emit_sta_zero_page(runtime_zp::ARGS);
    generator.emit_lda_zero_page(runtime_zp::ARGS);
    generator.emit_lda_imm(0x44);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_IMM,
            0x44,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
        ]
    );
    assert_eq!(
        generator.optimizations[0].kind,
        CodegenOptimizationKind::RegisterReloadRemoved
    );
}

#[test]
fn modern_profile_reuses_untracked_zero_page_memory_alias() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let source = StorageSlot::absolute(0x3000, 1);
    let scratch = StorageSlot::zero_page(0xE0, 1);

    generator.emit_lda_slot_byte(source, 0);
    generator.emit_sta_slot_byte(scratch, 0);
    generator.emit_lda_slot_byte(scratch, 0);

    assert_eq!(
        generator.emitter.bytes,
        [opcode::LDA_ABS, 0x00, 0x30, opcode::STA_ZP, 0xE0,]
    );
}

#[test]
fn modern_profile_reuses_value_only_zero_page_load_before_shift() {
    let mut generator = test_generator(CodegenProfile::Modern);
    let source = StorageSlot::absolute(0x3000, 1);
    let scratch = ZeroPage::new(0xE0);

    generator.emit_lda_slot_byte(source, 0);
    generator.emit_sta_zero_page(scratch);
    generator.emit_cmp_imm(0);
    generator.emit_lda_zero_page_value_only(scratch);
    generator.emit_asl_a();

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_ABS,
            0x00,
            0x30,
            opcode::STA_ZP,
            0xE0,
            opcode::CMP_IMM,
            0x00,
            opcode::ASL_A,
        ]
    );
}

#[test]
fn zero_page_tracking_is_cleared_at_label_joins() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(0x44);
    generator.emit_sta_zero_page(runtime_zp::ARGS);
    generator.bind_codegen_label("join".to_string(), Span::new(0, 0));
    generator.emit_lda_zero_page(runtime_zp::ARGS);
    generator.emit_lda_imm(0x44);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_IMM,
            0x44,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0x44,
        ]
    );
}

#[test]
fn absolute_stores_alias_tracked_zero_page_registers() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(0x66);
    generator.emit_sta_absolute(Absolute::new(runtime_zp::ARGS.address() as u16));
    generator.emit_lda_zero_page(runtime_zp::ARGS);
    generator.emit_lda_imm(0x66);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_IMM,
            0x66,
            opcode::STA_ABS,
            runtime_zp::ARGS.address(),
            0x00,
        ]
    );
}

#[test]
fn absolute_mutations_invalidate_tracked_zero_page_aliases() {
    let mut generator = test_generator(CodegenProfile::Modern);

    generator.emit_lda_imm(0x66);
    generator.emit_sta_zero_page(runtime_zp::ARGS);
    generator.emit_inc_absolute(Absolute::new(runtime_zp::ARGS.address() as u16));
    generator.emit_lda_zero_page(runtime_zp::ARGS);
    generator.emit_lda_imm(0x66);

    assert_eq!(
        generator.emitter.bytes,
        [
            opcode::LDA_IMM,
            0x66,
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::INC_ABS,
            runtime_zp::ARGS.address(),
            0x00,
            opcode::LDA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0x66,
        ]
    );
}

fn generate_source(source: &str) -> Result<CodegenOutput, Vec<Diagnostic>> {
    generate_source_with_origin(source, CODE_ORIGIN)
}

fn generate_source_with_origin(
    source: &str,
    origin: u16,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let tokens = tokenize(source).unwrap();
    let program = parse(&tokens)?;
    analyze(&program)?;
    generate_with_origin(&program, origin)
}

fn generate_compatible_source_with_origin(
    source: &str,
    origin: u16,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    generate_profile_source_with_origin(source, origin, CodegenProfile::Compat)
}

fn generate_profile_source_with_origin(
    source: &str,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let tokens = tokenize(source).unwrap();
    let program = parse(&tokens)?;
    analyze(&program)?;
    generate_profile_with_origin(&program, origin, profile)
}

fn generate_semir_native_source_with_origin(
    source: &str,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let tokens = tokenize(source).unwrap();
    let program = parse(&tokens)?;
    let model = analyze(&program)?;
    let semir = crate::semantic::ir::lower_program(&program, &model);
    generate_semir_native_profile_with_origin(&semir, origin, profile)
}

fn assert_compatible_diagnostic_contains(source: &str, needle: &str) {
    let diagnostics =
        generate_compatible_source_with_origin(source, 0x3000).expect_err("expected diagnostic");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(needle)),
        "expected diagnostic containing `{needle}`, got {diagnostics:?}"
    );
}

fn storage_symbol<'a>(
    output: &'a CodegenOutput,
    scope: CodegenSymbolScope,
    name: &str,
) -> &'a CodegenStorageSymbol {
    output
        .map
        .storage_symbols
        .iter()
        .find(|symbol| symbol.scope == scope && symbol.name == name)
        .unwrap_or_else(|| panic!("missing storage symbol {scope:?}::{name}"))
}

fn routine_address(output: &CodegenOutput, name: &str) -> Option<u16> {
    output
        .routine_addresses
        .iter()
        .find(|routine| routine.name.eq_ignore_ascii_case(name))
        .map(|routine| routine.address)
}

fn count_pair(bytes: &[u8], opcode: u8, operand: u8) -> usize {
    bytes
        .windows(2)
        .filter(|bytes| *bytes == [opcode, operand])
        .count()
}

fn count_jsr_to(bytes: &[u8], address: u16) -> usize {
    let address = Absolute::new(address);
    bytes
        .windows(3)
        .filter(|bytes| *bytes == [opcode::JSR_ABS, address.low(), address.high()])
        .count()
}

fn assert_call_or_tail_jump_order(output: &CodegenOutput, names: &[&str]) {
    let expected = names
        .iter()
        .map(|name| {
            output
                .routine_addresses
                .iter()
                .find(|routine| routine.name.eq_ignore_ascii_case(name))
                .map(|routine| routine.address)
                .unwrap_or_else(|| panic!("missing routine address for {name}"))
        })
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    for bytes in output.bytes.windows(3) {
        if matches!(bytes[0], opcode::JSR_ABS | opcode::JMP_ABS) {
            let address = u16::from_le_bytes([bytes[1], bytes[2]]);
            if expected.contains(&address) {
                found.push(address);
            }
        }
    }
    assert_eq!(found, expected);
}

fn test_name_expr(name: &str) -> Expr {
    Expr {
        kind: ExprKind::Name(name.to_string()),
        text: name.to_string(),
        span: Span::new(0, 0),
    }
}

fn test_expr(kind: ExprKind) -> Expr {
    Expr {
        kind,
        text: String::new(),
        span: Span::new(0, 0),
    }
}

fn test_generator(profile: CodegenProfile) -> Generator {
    Generator {
        emitter: Emitter::with_origin(0x3000),
        layout: StorageLayout::empty(0x3000),
        record_layouts: RecordLayouts::default(),
        routines: HashMap::new(),
        callable_pointers: HashMap::new(),
        numeric_defines: HashMap::new(),
        machine_defines: HashMap::new(),
        runtime_helpers: RuntimeHelperTargets::default_for_target(RuntimeTarget::Cartridge),
        routine_assignment_targets: HashSet::new(),
        local_symbols: HashMap::new(),
        local_callable_pointers: HashMap::new(),
        current_return_slot: None,
        diagnostics: Vec::new(),
        label_counter: 0,
        exit_labels: Vec::new(),
        profile,
        segment_storage: true,
        processor: ProcessorState::default(),
        straight_line_store_y: None,
        y_constant_store_lookahead: None,
        label_store_y_hints: HashMap::new(),
        label_byte_values: HashMap::new(),
        last_label_position: None,
        compatible_cursor: Some(0x3000),
        skipped_ranges: Vec::new(),
        last_routine_label: None,
        last_routine_ended_with_rts: false,
        routine_addresses: Vec::new(),
        routine_ranges: Vec::new(),
        routine_signatures: Vec::new(),
        current_routine_effects: None,
        current_routine_has_effect_contract: false,
        current_inferred_routine_facts: None,
        current_modern_routine_layout: ModernRoutineLayout::default(),
        preserve_modern_routine_layout: false,
        machine_blocks: Vec::new(),
        optimizations: Vec::new(),
        proofs: Vec::new(),
        proof_attempts: Vec::new(),
        branch_inversion_candidates: Vec::new(),
        storage_symbols: Vec::new(),
        source_ranges: Vec::new(),
        deferred_output_cursor: 0x3000,
        suppress_implicit_rts_once: false,
        inline_byte_constant_shift: false,
    }
}
