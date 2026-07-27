use actionc::codegen::{CodegenProfile, generate_semir_native_profile_with_origin};
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

fn semir(source: &str) -> semantic::ir::SemProgram {
    let tokens = lexer::tokenize(source).expect("tokenize inline assembler source");
    let program = parser::parse(&tokens).expect("parse inline assembler source");
    let model = semantic::analyze(&program).expect("analyze inline assembler source");
    semantic::ir::lower_program(&program, &model)
}

#[test]
fn inline_asm_emits_in_modern_classic() {
    let output =
        generate_semir_native_profile_with_origin(&semir(SOURCE), 0x3000, CodegenProfile::Modern)
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
    let classic =
        generate_semir_native_profile_with_origin(&semir(valid), 0x3000, CodegenProfile::Modern)
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
    let classic_error =
        generate_semir_native_profile_with_origin(&invalid_semir, 0x3000, CodegenProfile::Modern)
            .unwrap_err();
    assert!(
        classic_error
            .iter()
            .any(|diagnostic| diagnostic.message.contains("requires zero-page storage"))
    );

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
    generate_semir_native_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
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
    generate_semir_native_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
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
    generate_semir_native_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
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
    generate_semir_native_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
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
    generate_semir_native_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
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
        generate_semir_native_profile_with_origin(&semir(source), 0x3000, CodegenProfile::Modern)
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
    let diagnostics =
        generate_semir_native_profile_with_origin(&semir(too_wide), 0x3000, CodegenProfile::Modern)
            .unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("does not fit in one byte"))
    );
}
