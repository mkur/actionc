use actionc::codegen::{CodegenProfile, generate_semir_profile_with_origin};
use actionc::{lexer, mir6502, nir, parser, semantic};

const SOURCE: &str = r#"
BYTE target=$4000
BYTE ARRAY data(4)

PROC Draw()
RETURN

PROC Main()
ASM
    lda #$2A
    sta target
    ldx #2
loop:
    dex
    bne loop
    lda #<data
    sta $80
    lda #>data
    sta $81
    jsr Draw
ENDASM
RETURN
"#;

const FIXED_ARRAY_SOURCE: &str = r#"
BYTE ARRAY displayList($400)=$5000

PROC Main()
ASM
    lda #0
    sta displayList+8
ENDASM
RETURN
"#;

const FIXED_ARRAY_NEGATIVE_SOURCE: &str = r#"
BYTE ARRAY displayList($400)=$5000

PROC Main()
ASM
    lda #0
    sta displayList-10
ENDASM
RETURN
"#;

const FIXED_ARRAY_ADDRESS_FORMS_SOURCE: &str = r#"
BYTE ARRAY displayList($400)=$5000

PROC Main()
ASM
    lda #<displayList
    sta $80
    lda #>displayList
    sta $81
    lda #0
    sta displayList
    sta displayList+8
ENDASM
RETURN
"#;

const SELF_MODIFYING_SOURCE: &str = r#"
PROC Main()
ASM
    lda patch:#0
    sta patch
    lda source:$ff00,y
    sta source+1
ENDASM
RETURN
"#;

fn semir(source: &str) -> semantic::ir::SemProgram {
    let tokens = lexer::tokenize(source).expect("tokenize inline assembler source");
    let program = parser::parse(&tokens).expect("parse inline assembler source");
    let model = semantic::analyze(&program).expect("analyze inline assembler source");
    semantic::ir::lower_program(&program, &model)
}

fn inline_payload(
    op: &nir::NirOp,
) -> Option<(&[u8], &[nir::NirForeignRelocation], &nir::NirMachineEffects)> {
    let nir::NirOp::ForeignCode { code, effects } = op else {
        return None;
    };
    let nir::NirForeignCodePayload::Bytes { bytes, relocations } = &code.payload else {
        return None;
    };
    Some((bytes, relocations, effects))
}

#[test]
fn inline_asm_emits_in_modern_classic() {
    let output = generate_semir_profile_with_origin(&semir(SOURCE), 0x3000, CodegenProfile::Modern)
        .expect("emit modern/classic inline assembler");

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x2A, 0x8D, 0x00, 0x40])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0xCA, 0xD0, 0xFD])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0xA9, 0x30, 0x85, 0x81])
    );
}

#[test]
fn inline_asm_classic_paths_keep_absolute_width_for_zero_page_symbols() {
    let source = r#"
BYTE source=$58, sink=$0600

PROC Main()
ASM
    lda source
    sta sink
ENDASM
RETURN
"#;
    let semir = semir(source);
    let ast_classic = generate_semir_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern)
        .expect("emit AST/classic absolute operand whose resolved address is in zero page");
    assert!(
        ast_classic
            .bytes
            .windows(6)
            .any(|bytes| bytes == [0xAD, 0x58, 0x00, 0x8D, 0x00, 0x06]),
        "AST/classic output: {:02X?}",
        ast_classic.bytes
    );

    let nir = nir::optimize_program(&nir::lower_program(&semir)).unwrap();
    let mir = mir6502::generate_output(&nir, 0x3000)
        .expect("emit MIR absolute operand whose resolved address is in zero page");
    assert!(
        mir.bytes
            .windows(6)
            .any(|bytes| bytes == [0xAD, 0x58, 0x00, 0x8D, 0x00, 0x06])
    );
}

#[test]
fn inline_asm_accepts_single_character_action_symbols() {
    let source = r#"
BYTE u=$7

PROC Main()
ASM
    lda u
ENDASM
RETURN
"#;
    generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
        .expect("emit single-character Action symbol in classic inline assembler");

    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    mir6502::generate_output(&nir, 0x3000)
        .expect("emit single-character Action symbol in MIR inline assembler");
}

#[test]
fn inline_asm_emits_in_mir6502() {
    let nir = nir::optimize_program(&nir::lower_program(&semir(SOURCE)))
        .expect("optimize inline assembler NIR");
    let output = mir6502::generate_output(&nir, 0x3000).expect("emit MIR6502 inline assembler");

    assert!(
        output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x2A, 0x8D, 0x00, 0x40])
    );
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0xCA, 0xD0, 0xFD])
    );
    assert!(
        output
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0xA9, 0x30, 0x85, 0x81])
    );
}

#[test]
fn inline_asm_self_modification_labels_emit_in_all_backends() {
    let semir = semir(SELF_MODIFYING_SOURCE);
    let classic = generate_semir_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern)
        .expect("emit MADS self-modification labels from classic");
    let nir = nir::optimize_program(&nir::lower_program(&semir))
        .expect("optimize self-modifying inline assembler NIR");
    let mir = mir6502::generate_output(&nir, 0x3000)
        .expect("emit MADS self-modification labels from MIR6502");

    for (backend, output) in [("classic", classic), ("MIR6502", mir)] {
        let start = output
            .bytes
            .windows(3)
            .position(|bytes| bytes == [0xA9, 0x00, 0x8D])
            .unwrap_or_else(|| panic!("{backend} omitted the self-modifying sequence"));
        let patched_operand = output.origin.wrapping_add(start as u16 + 1);
        assert_eq!(
            &output.bytes[start + 3..start + 5],
            &patched_operand.to_le_bytes(),
            "{backend} did not target the immediate operand byte"
        );

        let absolute = start + 5;
        assert_eq!(&output.bytes[absolute..absolute + 3], &[0xB9, 0x00, 0xFF]);
        let high_operand = output.origin.wrapping_add(absolute as u16 + 2);
        assert_eq!(
            &output.bytes[absolute + 4..absolute + 6],
            &high_operand.to_le_bytes()
        );
    }
}

#[test]
fn inline_asm_self_code_writes_have_conservative_nir_effects() {
    let program = nir::lower_program(&semir(SELF_MODIFYING_SOURCE));
    let (_, relocations, effects) = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .find_map(inline_payload)
        .expect("self-modifying inline assembler NIR operation");

    assert!(relocations.iter().any(|relocation| {
        relocation.target == nir::NirForeignCodeTarget::InlineOffset(nir::ByteOffset::new(1))
            && relocation.symbol_use == actionc::foreign::ForeignSymbolUse::Write
    }));
    assert_eq!(effects.memory.writes, nir::NirMemoryAccess::Unknown);
    nir::verify_program(&program).expect("self-modifying inline assembler NIR must verify");
}

#[test]
fn inline_asm_fixed_array_addend_targets_declared_backing_address_in_nir() {
    let program = nir::lower_program(&semir(FIXED_ARRAY_SOURCE));
    let global = program
        .globals
        .iter()
        .find(|global| global.name == "displayList")
        .expect("fixed array global");
    assert_eq!(
        global.storage_size,
        nir::ByteSize::new(4),
        "array retains its descriptor cell"
    );
    assert_eq!(
        global
            .array
            .as_ref()
            .and_then(|array| array.address_initializer),
        Some(nir::AddressValue::data(0x5000))
    );

    let (_, relocations, effects) = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .find_map(inline_payload)
        .expect("inline assembler NIR operation");
    assert_eq!(relocations.len(), 1);
    assert_eq!(
        relocations[0].target,
        nir::NirForeignCodeTarget::Absolute(nir::AddressValue::data(0x5000))
    );
    assert_eq!(relocations[0].addend, 8);
    assert_eq!(
        effects.memory.writes,
        nir::NirMemoryAccess::Regions(vec![nir::NirMemoryRegion {
            kind: nir::NirMemoryRegionKind::AbsoluteRange(
                actionc::target::TargetLayout::DATA_ADDRESS_SPACE,
            ),
            offset: nir::ByteOffset::new(0x5008),
            size: nir::ByteSize::ONE,
        }])
    );
    nir::verify_program(&program).expect("fixed-array inline assembler NIR must verify");
}

#[test]
fn inline_asm_negative_fixed_array_addend_targets_absolute_region_in_nir() {
    let program = nir::lower_program(&semir(FIXED_ARRAY_NEGATIVE_SOURCE));
    let (_, relocations, effects) = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .find_map(inline_payload)
        .expect("inline assembler NIR operation");

    assert_eq!(relocations.len(), 1);
    assert_eq!(
        relocations[0].target,
        nir::NirForeignCodeTarget::Absolute(nir::AddressValue::data(0x5000))
    );
    assert_eq!(relocations[0].addend, -10);
    assert_eq!(
        effects.memory.writes,
        nir::NirMemoryAccess::Regions(vec![nir::NirMemoryRegion {
            kind: nir::NirMemoryRegionKind::AbsoluteRange(
                actionc::target::TargetLayout::DATA_ADDRESS_SPACE,
            ),
            offset: nir::ByteOffset::new(0x4ff6),
            size: nir::ByteSize::ONE,
        }])
    );
    nir::verify_program(&program).expect("negative fixed-array addend must verify");
}

#[test]
fn inline_asm_negative_fixed_array_addend_emits_in_maintained_backends() {
    let semir = semir(FIXED_ARRAY_NEGATIVE_SOURCE);
    let ast_classic = generate_semir_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern)
        .expect("emit negative fixed-array addend from AST/classic");
    let nir = nir::optimize_program(&nir::lower_program(&semir))
        .expect("optimize negative fixed-array inline assembler NIR");
    let mir = mir6502::generate_output(&nir, 0x3000)
        .expect("emit negative fixed-array addend from MIR6502");

    for (backend, output) in [("AST/classic", ast_classic), ("MIR6502", mir)] {
        assert!(
            output
                .bytes
                .windows(5)
                .any(|bytes| bytes == [0xA9, 0x00, 0x8D, 0xF6, 0x4F]),
            "{backend} did not emit STA $4FF6: {:02X?}",
            output.bytes
        );
    }
}

#[test]
fn inline_asm_absolute_addend_underflow_is_rejected_by_nir_verifier() {
    let source = r#"
BYTE ARRAY low($400)=$0005

PROC Main()
ASM
    lda #0
    sta low-10
ENDASM
RETURN
"#;
    let program = nir::lower_program(&semir(source));
    let diagnostics = nir::verify_program(&program).expect_err("absolute underflow must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("absolute relocation result is outside")
    }));
}

#[test]
fn inline_asm_negative_storage_addend_uses_conservative_effects() {
    let source = r#"
BYTE value

PROC Main()
ASM
    lda #0
    sta value-1
ENDASM
RETURN
"#;
    let program = nir::lower_program(&semir(source));
    let effects = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .find_map(inline_payload)
        .map(|(_, _, effects)| effects)
        .expect("inline assembler effects");

    assert_eq!(effects.memory.writes, nir::NirMemoryAccess::Unknown);
    nir::verify_program(&program).expect("conservative negative storage effect must verify");
}

#[test]
fn inline_asm_fixed_array_addend_emits_declared_backing_address_in_maintained_backends() {
    let semir = semir(FIXED_ARRAY_ADDRESS_FORMS_SOURCE);
    let ast_classic = generate_semir_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern)
        .expect("emit fixed-array operand from AST/classic");
    let nir = nir::optimize_program(&nir::lower_program(&semir))
        .expect("optimize fixed-array inline assembler NIR");
    let mir =
        mir6502::generate_output(&nir, 0x3000).expect("emit fixed-array operand from MIR6502");

    for (backend, output) in [("AST/classic", ast_classic), ("MIR6502", mir)] {
        assert!(
            output
                .bytes
                .windows(4)
                .any(|bytes| bytes == [0xA9, 0x00, 0x85, 0x80]),
            "{backend} did not emit the low backing-address byte: {:02X?}",
            output.bytes
        );
        assert!(
            output
                .bytes
                .windows(4)
                .any(|bytes| bytes == [0xA9, 0x50, 0x85, 0x81]),
            "{backend} did not emit the high backing-address byte: {:02X?}",
            output.bytes
        );
        assert!(
            output
                .bytes
                .windows(8)
                .any(|bytes| bytes == [0xA9, 0x00, 0x8D, 0x00, 0x50, 0x8D, 0x08, 0x50]),
            "{backend} did not emit STA $5000 followed by STA $5008: {:02X?}",
            output.bytes
        );
    }
}

#[test]
fn inline_asm_dynamic_array_keeps_descriptor_storage_target_in_nir() {
    let source = r#"
BYTE ARRAY dynamic

PROC Main()
ASM
    lda dynamic
ENDASM
RETURN
"#;
    let program = nir::lower_program(&semir(source));
    let global = program
        .globals
        .iter()
        .find(|global| global.name == "dynamic")
        .expect("dynamic array global");
    assert_eq!(
        global.storage_size,
        nir::ByteSize::new(2),
        "array retains its pointer cell"
    );
    assert!(
        global
            .array
            .as_ref()
            .is_some_and(|array| array.pointer_backed && array.address_initializer.is_none())
    );

    let relocation = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .find_map(inline_payload)
        .and_then(|(_, relocations, _)| relocations.first())
        .expect("inline assembler relocation");
    assert_eq!(
        relocation.target,
        nir::NirForeignCodeTarget::Storage(nir::NirStorageId::Global(global.id))
    );
    nir::verify_program(&program).expect("dynamic-array inline assembler NIR must verify");
}

#[test]
fn inline_asm_diagnostics_keep_action_source_offsets() {
    let source = "PROC Main()\nASM\n    lda #missing\nENDASM\nRETURN\n";
    let tokens = lexer::tokenize(source).unwrap();
    let program = parser::parse(&tokens).unwrap();
    let diagnostics =
        semantic::analyze(&program).expect_err("unknown immediate constant must fail");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("undefined inline assembler symbol")
    }));
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.span.start >= 16)
    );
}

#[test]
fn inline_asm_indirect_pointer_requires_proven_zero_page_storage() {
    let valid = r#"
CARD ptr=$A0
PROC Main()
ASM
    ldy #0
    lda (ptr),y
ENDASM
RETURN
"#;
    let classic = generate_semir_profile_with_origin(&semir(valid), 0x3000, CodegenProfile::Modern)
        .expect("emit fixed-zero-page indirect operand");
    assert!(classic.bytes.windows(2).any(|bytes| bytes == [0xB1, 0xA0]));

    let nir = nir::optimize_program(&nir::lower_program(&semir(valid))).unwrap();
    let mir =
        mir6502::generate_output(&nir, 0x3000).expect("emit MIR fixed-zero-page indirect operand");
    assert!(mir.bytes.windows(2).any(|bytes| bytes == [0xB1, 0xA0]));

    let invalid = r#"
CARD ptr
PROC Main()
ASM
    ldy #0
    lda (ptr),y
ENDASM
RETURN
"#;
    let invalid_semir = semir(invalid);
    let invalid_nir = nir::optimize_program(&nir::lower_program(&invalid_semir)).unwrap();
    let mir_error = mir6502::generate_output(&invalid_nir, 0x3000).unwrap_err();
    assert!(
        mir_error
            .iter()
            .any(|diagnostic| diagnostic.message.contains("zero-page"))
    );
}

#[test]
fn inline_asm_references_parameter_and_local_homes() {
    let source = r#"
PROC Touch(BYTE value)
  BYTE local
  ASM
      lda value
      sta local
  ENDASM
RETURN
"#;
    generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
        .expect("classic emitter retains inline assembler parameter/local homes");
    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    mir6502::generate_output(&nir, 0x3000)
        .expect("MIR emitter retains inline assembler parameter/local homes");
}

#[test]
fn inline_asm_unknown_and_ineligible_action_objects_are_semantic_errors() {
    let unknown = "PROC Main()\nASM\n  lda missing\nENDASM\nRETURN\n";
    let tokens = lexer::tokenize(unknown).unwrap();
    let program = parser::parse(&tokens).unwrap();
    let diagnostics = semantic::analyze(&program).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("undefined inline assembler symbol")
    }));

    let invalid_call = "BYTE value\nPROC Main()\nASM\n  jsr value\nENDASM\nRETURN\n";
    let tokens = lexer::tokenize(invalid_call).unwrap();
    let program = parser::parse(&tokens).unwrap();
    let diagnostics = semantic::analyze(&program).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("incompatible"))
    );
}

#[test]
fn inline_asm_indirect_jump_accepts_proc_pointer_storage() {
    let source = r#"
PROC POINTER handler

PROC Dispatch()
ASM
    jmp (handler)
ENDASM
"#;
    generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
        .expect("emit indirect jump through PROC POINTER storage");

    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    mir6502::generate_output(&nir, 0x3000)
        .expect("emit MIR indirect jump through PROC POINTER storage");
}

#[test]
fn inline_asm_resolves_exact_address_locals_before_mir_emission() {
    let source = r#"
PROC Dispatch()
  CARD handler=$BFFA
  ASM
      jmp (handler)
  ENDASM
"#;
    generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
        .expect("emit exact-address local from classic inline assembler");

    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    let output = mir6502::generate_output(&nir, 0x3000)
        .expect("emit exact-address local from MIR inline assembler");
    assert!(
        output
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0x6C, 0xFA, 0xBF])
    );
}

#[test]
fn inline_asm_absolute_relocations_feed_known_callee_exit_state() {
    let source = r#"
BYTE sink=$0600

BYTE FUNC MachineValue=*()
ASM
    lda #$2A
    sta $A0
    rts
ENDASM

PROC Main()
sink = MachineValue()
RETURN
"#;
    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    let output = mir6502::generate_output(&nir, 0x3000)
        .expect("emit known inline-assembler callee exit state");

    assert!(
        !output
            .bytes
            .windows(5)
            .any(|bytes| bytes == [0xA5, 0xA0, 0x8D, 0x00, 0x06])
    );
}

#[test]
fn terminal_inline_asm_satisfies_function_return_flow() {
    let source = r#"
BYTE FUNC MachineValue()
ASM
    lda #$2A
    sta $A0
    rts
ENDASM
"#;
    generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
        .expect("emit function implemented by terminal inline assembler");

    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    mir6502::generate_output(&nir, 0x3000)
        .expect("emit MIR function implemented by terminal inline assembler");
}

#[test]
fn inline_asm_starts_a_new_statement_after_assignment_or_call() {
    let source = r#"
CARD value

PROC Touch()
RETURN

PROC Main()
value = $1B48
Touch()
ASM
    lda #$2A
ENDASM
RETURN
"#;
    generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
        .expect("emit inline assembler after ordinary statements");

    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    mir6502::generate_output(&nir, 0x3000)
        .expect("emit MIR inline assembler after ordinary statements");
}

#[test]
fn inline_asm_accepts_byte_action_constants_without_address_selectors() {
    let source = r#"
DEFINE VALUE="$2A"
PROC Main()
ASM
    lda #VALUE
ENDASM
RETURN
"#;
    let classic =
        generate_semir_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
            .expect("emit Action constant in classic inline assembler");
    assert!(classic.bytes.windows(2).any(|bytes| bytes == [0xA9, 0x2A]));

    let nir = nir::optimize_program(&nir::lower_program(&semir(source))).unwrap();
    let mir = mir6502::generate_output(&nir, 0x3000)
        .expect("emit Action constant in MIR inline assembler");
    assert!(mir.bytes.windows(2).any(|bytes| bytes == [0xA9, 0x2A]));

    let too_wide = r#"
DEFINE VALUE="$1234"
PROC Main()
ASM
    lda #VALUE
ENDASM
RETURN
"#;
    let too_wide_nir = nir::optimize_program(&nir::lower_program(&semir(too_wide))).unwrap();
    let diagnostics = mir6502::generate_output(&too_wide_nir, 0x3000).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("does not fit in one byte"))
    );
}
