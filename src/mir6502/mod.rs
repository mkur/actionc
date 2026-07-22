mod abi;
mod analysis;
mod builtin;
mod call_plan;
mod classify;
mod diagnostics;
mod emit;
mod ir;
mod lower;
mod materialize;
mod passes;
mod printer;
mod rewrite;
mod verify;

pub use diagnostics::MirDiagnostic;
pub use ir::{
    MirAddr, MirAddressConsumer, MirArgHome, MirBinaryOp, MirBlock, MirBlockId, MirBlockParam,
    MirCallAbi, MirCallArg, MirCallResult, MirCallTarget, MirCarryIn, MirCarryOut, MirCompareOp,
    MirCond, MirCondDest, MirDef, MirEdge, MirEdgeArg, MirEffects, MirFixedZpSlot, MirFlag,
    MirFlagTest, MirFrame, MirGlobal, MirGlobalBacking, MirLabel, MirMachineBlockId, MirMem,
    MirMemoryEffect, MirMemoryRegion, MirMemoryRegionKind, MirOp, MirOpRef, MirPhase,
    MirPointerPair, MirProgram, MirReg, MirRegisterSet, MirResultHome, MirRoutine, MirRoutineAbi,
    MirRuntimeHelper, MirRuntimeHelperDecl, MirRuntimeHelperTarget, MirSpillId, MirStatic,
    MirStorageBase, MirStorageClass, MirStorageId, MirStorageSlot, MirTemp, MirTempId,
    MirTerminator, MirUnaryOp, MirUpdateOp, MirValue, MirWidth, MirZpAllocation, MirZpSlot,
    RoutineId,
};
pub use passes::{Mir6502Config, MirPeepholeReportMode};

use crate::nir::NirProgram;

pub fn lower_program(nir: &NirProgram) -> Result<MirProgram, Vec<MirDiagnostic>> {
    lower::lower_program(nir)
}

pub fn verify_program(program: &MirProgram, phase: MirPhase) -> Result<(), Vec<MirDiagnostic>> {
    verify::verify_program(program, phase)
}

pub fn materialize_program(
    program: MirProgram,
    config: &Mir6502Config,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    materialize_program_with_origin(program, config, crate::codegen::CODE_ORIGIN)
}

pub fn materialize_program_with_origin(
    program: MirProgram,
    config: &Mir6502Config,
    origin: u16,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    verify::verify_program(&program, MirPhase::PreMaterialization)
        .map_err(|diagnostics| phase_diagnostics("pre-materialization", diagnostics))?;
    let materialized = materialize::materialize_program(program, config, origin)
        .map_err(|diagnostics| phase_diagnostics("materialization", diagnostics))?;
    verify::verify_program(&materialized, MirPhase::PostMaterialization)
        .map_err(|diagnostics| phase_diagnostics("post-materialization", diagnostics))?;
    Ok(materialized)
}

pub fn format_program(program: &MirProgram) -> String {
    printer::format_program(program)
}

#[cfg(test)]
fn emit_program(
    mir: &MirProgram,
    emitter: &mut crate::codegen::native_emitter::NativeTrackedEmitter,
) -> Result<emit::MirEmissionSummary, Vec<MirDiagnostic>> {
    emit::emit_program(mir, emitter_origin(emitter), emitter)
}

pub fn generate_output(
    nir: &NirProgram,
    origin: u16,
) -> Result<crate::codegen::CodegenOutput, Vec<MirDiagnostic>> {
    generate_output_with_config(nir, origin, &Mir6502Config::default())
}

pub fn generate_output_with_config(
    nir: &NirProgram,
    origin: u16,
    config: &Mir6502Config,
) -> Result<crate::codegen::CodegenOutput, Vec<MirDiagnostic>> {
    let mir = lower_program(nir)?;
    let mir = materialize_program_with_origin(mir, config, origin)?;
    let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(origin);
    let summary = emit::emit_program(&mir, origin, &mut emitter)?;
    let bytes = emitter.finish().map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| MirDiagnostic {
                routine: None,
                block: None,
                message: diagnostic.message,
            })
            .collect::<Vec<_>>()
    })?;
    Ok(codegen_output(bytes, origin, summary))
}

fn codegen_output(
    bytes: Vec<u8>,
    origin: u16,
    summary: emit::MirEmissionSummary,
) -> crate::codegen::CodegenOutput {
    let skipped_ranges = summary.skipped_ranges;
    let routine_addresses = summary.routine_addresses;
    let optimizations = Vec::new();
    let proofs = Vec::new();
    let proof_attempts = Vec::new();
    let run_address = routine_addresses
        .iter()
        .find(|routine| routine.name.eq_ignore_ascii_case("main"))
        .or_else(|| routine_addresses.last())
        .map_or(origin, |routine| routine.address);
    let map = crate::codegen::CodegenMap {
        origin,
        run_address,
        skipped_ranges: skipped_ranges.clone(),
        routine_addresses: routine_addresses.clone(),
        routine_ranges: summary.routine_ranges,
        routine_signatures: summary.routine_signatures,
        storage_symbols: summary.storage_symbols,
        source_ranges: summary.source_ranges,
        routine_effects: summary.routine_effects,
        machine_blocks: summary.machine_blocks,
        optimizations: optimizations.clone(),
        proofs: proofs.clone(),
        proof_attempts: proof_attempts.clone(),
    };
    crate::codegen::CodegenOutput {
        bytes,
        origin,
        run_address,
        skipped_ranges,
        routine_addresses,
        optimizations,
        proofs,
        proof_attempts,
        map,
    }
}

#[cfg(test)]
fn emitter_origin(_emitter: &crate::codegen::native_emitter::NativeTrackedEmitter) -> u16 {
    crate::codegen::CODE_ORIGIN
}

fn phase_diagnostics(phase: &str, diagnostics: Vec<MirDiagnostic>) -> Vec<MirDiagnostic> {
    diagnostics
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.message = format!("{phase}: {}", diagnostic.message);
            diagnostic
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ir::{
        MirEdge, MirGlobalInit, MirMachineAtom, MirMachineBlock, MirMachineByteSelector,
        MirMachineItem,
    };
    use super::*;
    use crate::nir::{
        BlockId, LocalId, NirBlock, NirMachineAtom as NirAtom,
        NirMachineByteSelector as NirByteSelector, NirMachineEffects, NirMachineItem,
        NirMemoryAccess, NirMemoryEffects, NirMemoryRegion, NirMemoryRegionKind, NirOp, NirProgram,
        NirRoutine, NirStorageId, NirTerminator, ParamId, SymbolId,
    };

    #[test]
    fn large_absolute_array_uses_descriptor_storage() {
        let output = generate_mir6502_source(
            r#"
            BYTE ARRAY allocbuf($800)=$2000
            CARD POINTER allocp
            CARD p

            PROC Main()
              allocp = CARD POINTER(@allocbuf)
              p = allocbuf
            RETURN
            "#,
        );

        assert_eq!(&output.bytes[0..4], &[0x00, 0x20, 0x00, 0x20]);
        assert!(bytes_contain(&output.bytes, &[0xA9, 0x00]));
        assert!(bytes_contain(&output.bytes, &[0xA9, 0x20]));
        assert!(bytes_contain(&output.bytes, &[0x8D, 0x06, 0x30]));
        assert!(bytes_contain(&output.bytes, &[0x8D, 0x07, 0x30]));
        assert!(!bytes_contain(&output.bytes, &[0xAD, 0x00, 0x30]));
        assert!(!bytes_contain(&output.bytes, &[0xAD, 0x01, 0x30]));
        assert!(!bytes_contain(&output.bytes, &[0xAD, 0x00, 0x20]));
        assert!(!bytes_contain(&output.bytes, &[0xAD, 0x01, 0x20]));
    }

    #[test]
    fn absolute_array_index_uses_initializer_address_as_element_base() {
        let output = generate_mir6502_source(
            r#"
            BYTE drive=$70B
            BYTE ARRAY dktype_1(8)=$7C3
            BYTE flag

            PROC Main()
              flag = dktype_1(drive) & $02
            RETURN
            "#,
        );

        assert!(!bytes_contain(
            &output.bytes,
            &[0xAD, 0xC3, 0x07, 0x85, 0xAC, 0xAD, 0xC4, 0x07]
        ));
        assert!(
            bytes_contain(&output.bytes, &[0xBD, 0xC3, 0x07])
                || bytes_contain(&output.bytes, &[0xB9, 0xC3, 0x07])
                || bytes_contain(&output.bytes, &[0xA9, 0xC3, 0x85, 0xAC])
        );
    }

    #[test]
    fn scalar_initializer_aliases_prior_storage_symbol() {
        let output = generate_mir6502_source(
            r#"
            CHAR ARRAY fname(15)
            BYTE fnamelen=fname

            PROC Main()
              fnamelen = 7
            RETURN
            "#,
        );

        let fname = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "fname")
            .expect("fname storage symbol");
        let fnamelen = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "fnamelen")
            .expect("fnamelen storage symbol");

        assert_eq!(fnamelen.address, fname.address);
        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA9,
                0x07,
                0x8D,
                fname.address as u8,
                (fname.address >> 8) as u8
            ]
        ));
    }

    #[test]
    fn scalar_initializer_aliases_absolute_backed_global_storage() {
        let output = generate_mir6502_source(
            r#"
            SET $E=$CB
            SET $F=0
            BYTE ARRAY line
            SET $E=$3000
            SET $491=$3000
            BYTE low=line, high=line+1

            PROC Main()
              low=$12
              high=$34
            RETURN
            "#,
        );

        let low = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "low")
            .expect("low storage symbol");
        let high = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "high")
            .expect("high storage symbol");

        assert_eq!(low.address, 0x00CB);
        assert_eq!(high.address, 0x00CC);
        assert!(bytes_contain(&output.bytes, &[0x85, 0xCB]));
        assert!(bytes_contain(&output.bytes, &[0x85, 0xCC]));
    }

    #[test]
    fn current_location_routine_calls_use_only_canonical_register_homes() {
        let output = generate_mir6502_source(
            r#"
            BYTE observedX=$0600, observedY=$0601

            PROC Capture=*(BYTE x,y)
              observedX=x
              observedY=y
            RETURN

            PROC Main()
              Capture($12,$34)
            RETURN
            "#,
        );
        let main = output
            .map
            .routine_ranges
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main routine range");
        let start = usize::from(main.start.wrapping_sub(output.origin));
        let end = usize::from(main.end.wrapping_sub(output.origin));
        let main_bytes = &output.bytes[start..end];

        assert!(bytes_contain(main_bytes, &[0xA9, 0x12]));
        assert!(bytes_contain(main_bytes, &[0xA2, 0x34]));
        assert!(!bytes_contain(main_bytes, &[0x85, 0xA0]));
        assert!(!bytes_contain(main_bytes, &[0x85, 0xA1]));
    }

    #[test]
    fn current_location_machine_a0_read_does_not_invent_a_caller_home() {
        let output = generate_mir6502_source(
            r#"
            BYTE observed=$0600

            PROC Capture=*(BYTE value)
            [$A5 $A0 $8D $00 $06 $60]

            PROC Main()
              Capture($12)
            RETURN
            "#,
        );
        let main = output
            .map
            .routine_ranges
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main routine range");
        let start = usize::from(main.start.wrapping_sub(output.origin));
        let end = usize::from(main.end.wrapping_sub(output.origin));
        let main_bytes = &output.bytes[start..end];

        assert!(bytes_contain(main_bytes, &[0xA9, 0x12]));
        assert!(!bytes_contain(main_bytes, &[0x85, 0xA0]));
    }

    #[test]
    fn current_location_machine_routine_can_explicitly_save_register_arg() {
        let output = generate_mir6502_source(
            r#"
            BYTE observed=$0600

            PROC Capture=*(BYTE value)
            [$85 $A0 $A5 $A0 $8D $00 $06 $60]

            PROC Main()
              Capture($12)
            RETURN
            "#,
        );
        let main = output
            .map
            .routine_ranges
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main routine range");
        let start = usize::from(main.start.wrapping_sub(output.origin));
        let end = usize::from(main.end.wrapping_sub(output.origin));
        let main_bytes = &output.bytes[start..end];

        assert!(bytes_contain(main_bytes, &[0xA9, 0x12]));
        assert!(!bytes_contain(main_bytes, &[0x85, 0xA0]));
        assert!(bytes_contain(
            &output.bytes,
            &[0x85, 0xA0, 0xA5, 0xA0, 0x8D, 0x00, 0x06]
        ));
    }

    #[test]
    fn symbolic_array_initializer_uses_routine_label_descriptor() {
        let output = generate_mir6502_source(
            r#"
            PROC Target=*()
            RETURN

            CARD ARRAY adr(2)=Target

            PROC Main()
            RETURN
            "#,
        );

        assert_eq!(&output.bytes[0..4], &[0x04, 0x30, 0x04, 0x30]);
    }

    #[test]
    fn predefined_variables_use_absolute_builtin_storage() {
        let output = generate_mir6502_source(
            "BYTE d PROC Main() color=3 LIST=1 TRACE=0 d=device d=EOF(1) RETURN",
        );

        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::STA_ABS, 0xFD, 0x02]
        ));
        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::LDA_ZP, 0xB7]
        ));
        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::STA_ABS, 0x9A, 0x04]
        ));
        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::STA_ABS, 0xC3, 0x04]
        ));
        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::LDA_IMM, 0xC0]
        ));
        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::LDA_IMM, 0x05]
        ));
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes[0] == crate::codegen::opcode::LDA_IZY)
        );
    }

    #[test]
    fn source_generation_uses_optimized_nir_for_zero_identity_arithmetic() {
        let output = generate_mir6502_source("CARD x,y PROC Main() y=x+0 RETURN");

        assert!(!bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::ADC_IMM, 0x00]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::SBC_IMM, 0x00]
        ));
    }

    #[test]
    fn source_generation_preserves_expected_word_unary_negation() {
        let literal = generate_mir6502_source("INT s PROC Main() s=-1 RETURN");
        assert!(bytes_contain(
            &literal.bytes,
            &[
                crate::codegen::opcode::LDA_IMM,
                0xFF,
                crate::codegen::opcode::STA_ABS,
                0x00,
                0x30,
                crate::codegen::opcode::STA_ABS,
                0x01,
                0x30,
            ]
        ));

        let variable = generate_mir6502_source("BYTE x=[5] INT s PROC Main() s=-x RETURN");
        assert!(bytes_contain(
            &variable.bytes,
            &[
                crate::codegen::opcode::LDA_IMM,
                0x00,
                crate::codegen::opcode::SEC,
                crate::codegen::opcode::SBC_ABS,
                0x00,
                0x30,
                crate::codegen::opcode::STA_ABS,
                0x01,
                0x30,
                crate::codegen::opcode::LDA_IMM,
                0x00,
                crate::codegen::opcode::SBC_IMM,
                0x00,
                crate::codegen::opcode::STA_ABS,
                0x02,
                0x30,
            ]
        ));
    }

    #[test]
    fn source_generation_evaluates_byte_literal_product_in_expected_word_width() {
        let output = generate_mir6502_source("CARD n PROC Main() n=40*90 RETURN");

        assert!(bytes_contain(
            &output.bytes,
            &[
                crate::codegen::opcode::LDA_IMM,
                0x10,
                crate::codegen::opcode::STA_ABS,
                0x00,
                0x30,
                crate::codegen::opcode::LDA_IMM,
                0x0E,
                crate::codegen::opcode::STA_ABS,
                0x01,
                0x30,
            ]
        ));
    }

    #[test]
    fn source_generation_zero_extends_byte_global_in_word_arithmetic() {
        let output = generate_mir6502_source("BYTE b=[252] CARD n PROC Main() n=4+b RETURN");

        assert!(bytes_contain(
            &output.bytes,
            &[
                crate::codegen::opcode::LDA_IMM,
                0x00,
                crate::codegen::opcode::ADC_IMM,
                0x00,
            ]
        ));
    }

    #[test]
    fn byte_actual_word_call_arg_is_zero_extended() {
        let source = "
            PROC Take(CARD value)
            RETURN

            PROC Main()
              BYTE n
              n=7
              Take(n)
            RETURN
        ";
        let tokens = crate::lexer::tokenize(source).expect("tokenize source");
        let program = crate::parser::parse(&tokens).expect("parse source");
        let model = crate::semantic::analyze(&program).expect("analyze source");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        let nir =
            crate::nir::optimize_program(&crate::nir::lower_program(&semir)).expect("optimize NIR");
        let mir = lower_program(&nir).expect("lower MIR6502");
        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let formatted = format_program(&materialized);

        assert!(formatted.contains("x =.b #0"));
        assert!(
            !formatted.contains("local l0+1"),
            "byte local should not be read as a word high byte:\n{formatted}"
        );
    }

    #[test]
    fn lowering_preserves_structured_local_storage_classes() {
        let source = "BYTE sink PROC Main() BYTE scalar BYTE ARRAY items(4) scalar=items(0) sink=scalar RETURN";
        let tokens = crate::lexer::tokenize(source).expect("tokenize source");
        let program = crate::parser::parse(&tokens).expect("parse source");
        let model = crate::semantic::analyze(&program).expect("analyze source");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        let nir = crate::nir::lower_program(&semir);
        let mir = lower_program(&nir).expect("lower MIR6502");
        let main = mir
            .routines
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main routine");
        let storage = |name: &str| {
            main.frame
                .locals
                .iter()
                .find(|slot| slot.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name} local"))
                .storage
        };

        assert_eq!(storage("scalar"), MirStorageClass::Scalar);
        assert_eq!(storage("items"), MirStorageClass::Array);
    }

    #[test]
    fn byte_actual_word_runtime_helper_arg_is_zero_extended() {
        let source = "
            BYTE n, mode
            CARD result
            CARD ARRAY sizes=[0 $100 $80]

            PROC Main()
              result=n*sizes(mode)
            RETURN
        ";
        let tokens = crate::lexer::tokenize(source).expect("tokenize source");
        let program = crate::parser::parse(&tokens).expect("parse source");
        let model = crate::semantic::analyze(&program).expect("analyze source");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        let nir =
            crate::nir::optimize_program(&crate::nir::lower_program(&semir)).expect("optimize NIR");
        let mir = lower_program(&nir).expect("lower MIR6502");
        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let formatted = format_program(&materialized);

        assert!(formatted.contains("helper mul"));
        assert!(formatted.contains("x =.b #0"));
        assert!(
            !formatted.contains("x =.b load spill"),
            "byte helper operand should not read an invented high-byte spill:\n{formatted}"
        );
    }

    #[test]
    fn source_generation_uses_optimized_nir_for_constant_offset_cancellation() {
        let output = generate_mir6502_source("CARD x,y PROC Main() y=x+2-2 RETURN");

        assert!(!bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::ADC_IMM, 0x02]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::SBC_IMM, 0x02]
        ));
    }

    #[test]
    fn byte_compare_to_byte_sized_word_literal_materializes_in_bool_chain() {
        let output =
            generate_mir6502_source("BYTE x,y,z PROC Main() IF y=0 AND x=$FF THEN z=1 FI RETURN");

        assert!(bytes_contain(
            &output.bytes,
            &[crate::codegen::opcode::CMP_IMM, 0xFF]
        ));
    }

    #[test]
    fn lowers_shell_program() {
        let nir = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let mir = lower_program(&nir).expect("lower shell MIR6502");
        assert_eq!(mir.routines[0].name, "Main");
        assert_eq!(mir.routines[0].blocks[0].label, "bb0");
        assert_eq!(mir.routines[0].id, RoutineId(0));
        assert_eq!(mir.routines[0].blocks[0].id, MirBlockId(0));
        assert!(mir.routines[0].blocks[0].ops.is_empty());
        assert!(matches!(
            mir.routines[0].blocks[0].terminator,
            MirTerminator::Return
        ));
    }

    #[test]
    fn lowers_nir_block_arguments_through_target_parallel_copies() {
        let ty = crate::nir::NirType {
            kind: crate::nir::NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(1),
            pointer: false,
        };
        let nir = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: vec![crate::nir::NirTemp {
                    id: crate::nir::TempId(0),
                    ty: ty.clone(),
                    def: crate::nir::NirTempDef {
                        block: BlockId(1),
                        op_index: None,
                    },
                }],
                notes: Vec::new(),
                blocks: vec![
                    NirBlock {
                        id: BlockId(0),
                        label: "entry".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: NirTerminator::Goto(crate::nir::NirEdge {
                            target: BlockId(1),
                            args: vec![crate::nir::NirValue::ConstU8(1)],
                        }),
                    },
                    NirBlock {
                        id: BlockId(1),
                        label: "join".to_string(),
                        params: vec![crate::nir::NirBlockParam {
                            dest: crate::nir::TempId(0),
                            ty,
                        }],
                        ops: Vec::new(),
                        terminator: NirTerminator::Return(None),
                    },
                ],
            }],
        };

        let mir = lower_program(&nir).expect("lower typed block arguments");
        assert_eq!(
            mir.routines[0].blocks[1].params,
            vec![MirBlockParam {
                dest: MirTempId(0),
                width: MirWidth::Byte,
            }]
        );
        assert_eq!(
            mir.routines[0].blocks[0].terminator,
            MirTerminator::Jump(MirEdge {
                target: MirBlockId(1),
                args: vec![MirEdgeArg {
                    value: MirValue::ConstU8(1),
                    width: MirWidth::Byte,
                }],
            })
        );
        let printed = format_program(&mir);
        assert!(printed.contains("b1 join(v0:byte):"));
        assert!(printed.contains("jump b1(#$01:byte)"));

        let materialized = materialize_program(mir, &Mir6502Config::default())
            .expect("materialize typed block arguments");
        assert!(
            materialized
                .routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .all(|block| block.params.is_empty())
        );
    }

    #[test]
    fn internal_calls_retain_exact_nir_memory_regions() {
        let byte = crate::nir::NirType {
            kind: crate::nir::NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(1),
            pointer: false,
        };
        let empty_routine = |name: &str, block_id: u32| NirRoutine {
            name: name.to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(block_id),
                label: format!("{name}.entry"),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Return(None),
            }],
        };
        let mut main = empty_routine("Main", 0);
        main.blocks[0].ops.push(NirOp::Call {
            callee: crate::nir::NirCallee::User("Touch".to_string()),
            args: Vec::new(),
            result: None,
            signature: Some(crate::nir::NirCallableSignature {
                params: Vec::new(),
                variadic: None,
                result: None,
                kind: "Proc".to_string(),
                abi: "action".to_string(),
            }),
            effects: crate::nir::NirCallEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::None,
                    writes: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                        kind: NirMemoryRegionKind::Storage(NirStorageId::Global(SymbolId(0))),
                        offset: 0,
                        size: 1,
                    }]),
                },
                may_call_os: false,
                opaque: false,
            },
        });
        let nir = NirProgram {
            globals: vec![crate::nir::NirGlobal {
                id: SymbolId(0),
                name: "g".to_string(),
                kind: "Byte".to_string(),
                ty: Some(byte),
                storage_size: 1,
                array: None,
                init: None,
                backing: crate::nir::NirGlobalBacking::Ordinary,
            }],
            statics: Vec::new(),
            routines: vec![empty_routine("Touch", 0), main],
        };

        let mir = lower_program(&nir).expect("lower exact call effects");
        let call = mir.routines[1].blocks[0]
            .ops
            .iter()
            .find_map(|op| match op {
                MirOp::Call { effects, .. } => Some(effects),
                _ => None,
            })
            .expect("lowered call");
        assert_eq!(
            call.memory_writes,
            MirMemoryEffect::Regions(vec![MirMemoryRegion {
                kind: MirMemoryRegionKind::Global(SymbolId(0)),
                offset: 0,
                size: 1,
            }])
        );
    }

    #[test]
    fn rejects_raw_machine_block_items() {
        let nir = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::MachineBlock {
                        items: vec![NirMachineItem::Raw("*".to_string())],
                        effects: NirMachineEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_os: false,
                            opaque: true,
                        },
                    }],
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let diagnostics = lower_program(&nir).expect_err("raw machine item should be rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("unsupported raw machine block item")
        }));
    }

    #[test]
    fn rejects_standalone_machine_block_operators() {
        let nir = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::MachineBlock {
                        items: vec![NirMachineItem::Raw("+".to_string())],
                        effects: NirMachineEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_os: false,
                            opaque: true,
                        },
                    }],
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let diagnostics =
            lower_program(&nir).expect_err("standalone machine operator should be rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("machine block item `+` is not a byte-stream item")
        }));
    }

    #[test]
    fn rejects_out_of_range_machine_block_number_items() {
        let nir = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::MachineBlock {
                        items: vec![NirMachineItem::Raw("$12345".to_string())],
                        effects: NirMachineEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_os: false,
                            opaque: true,
                        },
                    }],
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let diagnostics =
            lower_program(&nir).expect_err("oversized machine number should be rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("machine block item `$12345` does not fit in 16 bits")
        }));
    }

    #[test]
    fn lowers_structured_machine_block_address_items() {
        let nir = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::MachineBlock {
                        items: vec![NirMachineItem::AddressExpr {
                            selector: Some(NirByteSelector::Low),
                            explicit_address: true,
                            atom: NirAtom::Name("TARGET".to_string()),
                            offset: 1,
                            text: "<@TARGET+1".to_string(),
                        }],
                        effects: NirMachineEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_os: false,
                            opaque: true,
                        },
                    }],
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let mir = lower_program(&nir).expect("lower structured machine item");
        assert_eq!(
            mir.machine_blocks[0].items,
            vec![MirMachineItem::AddressExpr {
                selector: Some(MirMachineByteSelector::Low),
                explicit_address: true,
                atom: MirMachineAtom::Name("TARGET".to_string()),
                offset: 1,
                text: "<@TARGET+1".to_string(),
            }]
        );
    }

    #[test]
    fn formats_values_defs_and_direct_addresses() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: vec![MirTemp { id: MirTempId(0) }],
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(MirMem::Absolute(0x3000)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Move {
                            dst: MirDef::VTemp(MirTempId(0)),
                            src: MirValue::ConstU8(7),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let formatted = format_program(&mir);
        assert!(formatted.contains("a =.b load $3000"));
        assert!(formatted.contains("v0 =.b #$07"));
    }

    #[test]
    fn materializes_word_store_and_add_to_byte_lanes() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "g".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::ConstU16(0x1234),
                                width: MirWidth::Word,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::VTemp(MirTempId(0)),
                                left: MirValue::ConstU16(1),
                                right: MirValue::ConstU16(2),
                                width: MirWidth::Word,
                                carry_in: None,
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize word ops");

        let formatted = format_program(&mir);
        assert!(formatted.contains("a =.b #52"));
        assert!(formatted.contains("store.b global g0+0, a"));
        assert!(formatted.contains("a =.b #18"));
        assert!(formatted.contains("store.b global g0+1, a"));
        assert!(formatted.contains("a =.b #1"));
        assert!(formatted.contains("a =.b a add #$02 carry_in=clear carry_out=produce"));
        assert!(formatted.contains("a =.b #0"));
        assert!(formatted.contains("a =.b a add #$00 carry_in=previous"));
        assert!(!formatted.contains("spill"));
    }

    #[test]
    fn materializes_copy_store_consumers_without_spills() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "byte_src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "byte_dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(2),
                        name: "word_src".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 2 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(3),
                        name: "word_dst".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 4 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(4),
                        name: "const_dst".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 6 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![
                        MirTemp { id: MirTempId(0) },
                        MirTemp { id: MirTempId(1) },
                        MirTemp { id: MirTempId(2) },
                        MirTemp { id: MirTempId(3) },
                        MirTemp { id: MirTempId(4) },
                    ],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Move {
                                dst: MirDef::VTemp(MirTempId(1)),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(2)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                width: MirWidth::Word,
                            },
                            MirOp::Move {
                                dst: MirDef::VTemp(MirTempId(3)),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(3),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                                width: MirWidth::Word,
                            },
                            MirOp::Move {
                                dst: MirDef::VTemp(MirTempId(4)),
                                src: MirValue::ConstU16(0x1234),
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(4),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(4))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize direct copy store consumers");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"), "{formatted}");
        assert!(formatted.contains("a =.b load global g0+0"), "{formatted}");
        assert!(formatted.contains("store.b global g1+0, a"), "{formatted}");
        assert!(formatted.contains("a =.b load global g2+0"), "{formatted}");
        assert!(formatted.contains("store.b global g3+0, a"), "{formatted}");
        assert!(formatted.contains("a =.b load global g2+1"), "{formatted}");
        assert!(formatted.contains("store.b global g3+1, a"), "{formatted}");
        assert!(formatted.contains("a =.b #52"), "{formatted}");
        assert!(formatted.contains("store.b global g4+0, a"), "{formatted}");
        assert!(formatted.contains("a =.b #18"), "{formatted}");
        assert!(formatted.contains("store.b global g4+1, a"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("copy store consumers are ready");
    }

    #[test]
    fn materializes_indexed_word_copy_with_two_pointer_pairs() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "CardArray".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 8,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "CardArray".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 8,
                        backing: MirGlobalBacking::Ordinary { offset: 8 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(2),
                        name: "i".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 16 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(3),
                        name: "j".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 17 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![
                        MirTemp { id: MirTempId(0) },
                        MirTemp { id: MirTempId(1) },
                        MirTemp { id: MirTempId(2) },
                    ],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(1)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(3),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(2)),
                                src: MirAddr::ComputedIndex {
                                    base: MirValue::GlobalAddr(SymbolId(0)),
                                    index: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    elem_size: 2,
                                    offset: 0,
                                },
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::ComputedIndex {
                                    base: MirValue::GlobalAddr(SymbolId(1)),
                                    index: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                    elem_size: 2,
                                    offset: 0,
                                },
                                src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize indexed word copy");

        let formatted = format_program(&mir);
        assert!(
            formatted.contains("materialize_indexed (zp$AE),y"),
            "{formatted}"
        );
        assert!(
            formatted.contains("materialize_indexed (zp$AC),y"),
            "{formatted}"
        );
        assert!(
            formatted.contains("a =.b load_indirect (zp$AC),y+0"),
            "{formatted}"
        );
        assert!(
            formatted.contains("store_indirect (zp$AE),y+0 a"),
            "{formatted}"
        );
        assert!(
            formatted.contains("a =.b load_indirect (zp$AC),y+1"),
            "{formatted}"
        );
        assert!(
            formatted.contains("store_indirect (zp$AE),y+1 a"),
            "{formatted}"
        );
        assert!(!formatted.contains("advance (zp$AC),y"), "{formatted}");
        assert!(!formatted.contains("advance (zp$AE),y"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("indexed word copy is ready");
    }

    #[test]
    fn materializes_indexed_word_copy_without_base_pointer_staging() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "CardArray".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 8,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst_ptr".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 8 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(2),
                        name: "i".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 10 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(3),
                        name: "j".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 11 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![
                        MirTemp { id: MirTempId(0) },
                        MirTemp { id: MirTempId(1) },
                        MirTemp { id: MirTempId(2) },
                        MirTemp { id: MirTempId(3) },
                    ],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                width: MirWidth::Word,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(1)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(3),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(2)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(3)),
                                src: MirAddr::ComputedIndex {
                                    base: MirValue::GlobalAddr(SymbolId(0)),
                                    index: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                    elem_size: 2,
                                    offset: 0,
                                },
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::ComputedIndex {
                                    base: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    index: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                    elem_size: 2,
                                    offset: 0,
                                },
                                src: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize indexed word copy with recovered pointer base");

        let formatted = format_program(&mir);
        assert!(
            formatted.contains(
                "materialize_indexed (zp$AE),y <- word(*global g1+0, *global g1+1) + a*2"
            ),
            "{formatted}"
        );
        assert!(
            formatted.contains("materialize_indexed (zp$AC),y <- global_addr g0 + a*2"),
            "{formatted}"
        );
        assert!(!formatted.contains("spill"), "{formatted}");
        assert!(!formatted.contains("store.b zp"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("indexed word copy is ready");
    }

    #[test]
    fn forwards_direct_param_register_homes_to_initial_reloads() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "ptr".to_string(),
                    kind: "Card".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "FreeLike".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame {
                        params: vec![MirStorageSlot {
                            id: MirStorageId(0),
                            name: Some("p".to_string()),
                            storage: MirStorageClass::Scalar,
                            width: MirWidth::Word,
                            base: MirStorageBase::Param(ParamId(0)),
                            offset: 0,
                            mutable: true,
                            init: None,
                        }],
                        ..MirFrame::default()
                    },
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Param {
                                    id: ParamId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize forwarded param homes");

        let formatted = format_program(&mir);
        assert!(formatted.contains("store.b param p0+1, x"), "{formatted}");
        assert!(formatted.contains("store.b param p0+0, a"), "{formatted}");
        assert!(!formatted.contains("a =.b load param p0+0"), "{formatted}");
        assert!(!formatted.contains("a =.b load param p0+1"), "{formatted}");
        assert!(formatted.contains("store.b global g0+0, a"), "{formatted}");
        assert!(formatted.contains("a =.b x"), "{formatted}");
        assert!(formatted.contains("store.b global g0+1, a"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("forwarded params are ready");
    }

    #[test]
    fn materializes_lea_word_array_index_read_without_temp_staging() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "words".to_string(),
                        kind: "CardArray".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 8,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "i".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 8 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(2),
                        name: "out".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 9 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![
                        MirTemp { id: MirTempId(0) },
                        MirTemp { id: MirTempId(1) },
                        MirTemp { id: MirTempId(2) },
                    ],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LeaAddr {
                                dst: MirDef::VTemp(MirTempId(0)),
                                target: MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                },
                                width: MirWidth::Word,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(1)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(2)),
                                src: MirAddr::ComputedIndex {
                                    base: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    index: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                    elem_size: 2,
                                    offset: 0,
                                },
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize lea word array read");

        let formatted = format_program(&mir);
        assert!(formatted.contains(
            "materialize_indexed (zp$AC),scaled_y <- word(storage_addr_lo global g0+0, storage_addr_hi global g0+0) + a*2"
        ));
        assert!(formatted.contains("a =.b load global g1+0"));
        assert!(formatted.contains("a =.b load_indirect (zp$AC),scaled_y+0"));
        assert!(formatted.contains("a =.b load_indirect (zp$AC),scaled_y+1"));
        assert!(formatted.contains("store.b global g2+0, a"));
        assert!(formatted.contains("store.b global g2+1, a"));
        assert!(!formatted.contains("store.b zp0"));
        assert!(!formatted.contains("advance (zp$AC),y"));
        assert!(!formatted.contains("lea"));
    }

    #[test]
    fn materializes_lea_word_array_index_write_without_temp_staging() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "words".to_string(),
                        kind: "CardArray".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 8,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "i".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 8 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LeaAddr {
                                dst: MirDef::VTemp(MirTempId(0)),
                                target: MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                },
                                width: MirWidth::Word,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(1)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::ComputedIndex {
                                    base: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    index: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                    elem_size: 2,
                                    offset: 0,
                                },
                                src: MirValue::ConstU16(0x1234),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize lea word array write");

        let formatted = format_program(&mir);
        assert!(formatted.contains(
            "materialize_indexed (zp$AC),scaled_y <- word(storage_addr_lo global g0+0, storage_addr_hi global g0+0) + a*2"
        ));
        assert!(formatted.contains("a =.b load global g1+0"));
        assert!(formatted.contains("store_indirect (zp$AC),scaled_y+0 a"));
        assert!(formatted.contains("store_indirect (zp$AC),scaled_y+1 a"));
        assert!(!formatted.contains("store.b zp0"));
        assert!(!formatted.contains("advance (zp$AC),y"));
        assert!(!formatted.contains("lea"));
    }

    #[test]
    fn folds_materialized_direct_byte_add_one_to_inc() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "x".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Move {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirValue::PointerCell(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::Reg(MirReg::A),
                                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                                right: MirValue::ConstU8(1),
                                width: MirWidth::Byte,
                                carry_in: Some(MirCarryIn::Clear),
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize byte inc");

        let formatted = format_program(&mir);
        assert!(formatted.contains("inc.b global g0+0"));
        assert!(!formatted.contains("a =.b a add #$01"));
    }

    #[test]
    fn folds_byte_add_one_to_inc_before_call_clobbers_flags() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "x".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Take".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Move {
                                    dst: MirDef::Reg(MirReg::A),
                                    src: MirValue::PointerCell(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Byte,
                                },
                                MirOp::Binary {
                                    op: MirBinaryOp::Add,
                                    dst: MirDef::Reg(MirReg::A),
                                    left: MirValue::Def(MirDef::Reg(MirReg::A)),
                                    right: MirValue::ConstU8(1),
                                    width: MirWidth::Byte,
                                    carry_in: Some(MirCarryIn::Clear),
                                    carry_out: MirCarryOut::Ignore,
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                    width: MirWidth::Byte,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize inc before call");

        let formatted = format_program(&mir);
        assert!(formatted.contains("inc.b global g0+0"), "{formatted}");
        assert!(!formatted.contains("a =.b a add #$01"), "{formatted}");
        assert!(formatted.contains("call r0"), "{formatted}");
    }

    #[test]
    fn does_not_fold_low_byte_add_one_when_carry_is_consumed() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "w".to_string(),
                    kind: "Card".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Move {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirValue::PointerCell(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::Reg(MirReg::A),
                                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                                right: MirValue::ConstU8(1),
                                width: MirWidth::Byte,
                                carry_in: Some(MirCarryIn::Clear),
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::Move {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirValue::PointerCell(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 1,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::Reg(MirReg::A),
                                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                                right: MirValue::ConstU8(0),
                                width: MirWidth::Byte,
                                carry_in: Some(MirCarryIn::FromPrevious),
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 1,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize word add carry chain");

        let formatted = format_program(&mir);
        assert!(formatted.contains("a =.b a add #$01 carry_in=clear"));
        assert!(formatted.contains("a =.b a add #$00 carry_in=previous"));
        assert!(!formatted.contains("inc.b global g0+0"));
    }

    #[test]
    fn materializes_byte_add_sub_carry_facts() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "sum".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "diff".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::VTemp(MirTempId(0)),
                                left: MirValue::ConstU8(1),
                                right: MirValue::ConstU8(2),
                                width: MirWidth::Byte,
                                carry_in: None,
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Sub,
                                dst: MirDef::VTemp(MirTempId(1)),
                                left: MirValue::ConstU8(4),
                                right: MirValue::ConstU8(3),
                                width: MirWidth::Byte,
                                carry_in: None,
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize byte arithmetic");

        let formatted = format_program(&mir);
        assert!(formatted.contains("a =.b #1"));
        assert!(formatted.contains("a =.b a add #$02 carry_in=clear"));
        assert!(formatted.contains("a =.b #4"));
        assert!(formatted.contains("a =.b a sub #$03 carry_in=set"));
        assert!(!formatted.contains("spill"));
        verify_program(&mir, MirPhase::PreEmission).expect("byte arithmetic is emission-ready");
    }

    fn byte_update_program(
        op: MirBinaryOp,
        carry_in: Option<MirCarryIn>,
        carry_out: MirCarryOut,
        mem: MirMem,
        store_addr: MirAddr,
        extra_ops: Vec<MirOp>,
        terminator: MirTerminator,
    ) -> MirProgram {
        let globals = match mem {
            MirMem::Global { id, .. } => vec![MirGlobal {
                id,
                name: "g".to_string(),
                kind: "Byte".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Ordinary { offset: 0 },
                init: None,
            }],
            _ => Vec::new(),
        };
        let mut ops = vec![
            MirOp::Binary {
                op,
                dst: MirDef::VTemp(MirTempId(0)),
                left: MirValue::PointerCell(mem),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in,
                carry_out,
            },
            MirOp::Store {
                dst: store_addr,
                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                width: MirWidth::Byte,
            },
        ];
        ops.extend(extra_ops);
        let mut blocks = vec![MirBlock {
            id: MirBlockId(0),
            label: "bb0".to_string(),
            params: Vec::new(),
            ops,
            terminator: terminator.clone(),
        }];
        if matches!(terminator, MirTerminator::Branch { .. }) {
            blocks.push(MirBlock {
                id: MirBlockId(1),
                label: "bb1".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            });
            blocks.push(MirBlock {
                id: MirBlockId(2),
                label: "bb2".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            });
        }

        MirProgram {
            statics: Vec::new(),
            globals,
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                blocks,
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        }
    }

    fn global_byte_mem() -> MirMem {
        MirMem::Global {
            id: SymbolId(0),
            offset: 0,
        }
    }

    #[test]
    fn materializes_dead_byte_increment_to_inc() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Add,
                None,
                MirCarryOut::Ignore,
                mem.clone(),
                MirAddr::Direct(mem),
                Vec::new(),
                MirTerminator::Return,
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize byte increment");

        let formatted = format_program(&mir);
        assert!(formatted.contains("inc.b global g0+0"));
        assert!(!formatted.contains(" add #$01"));
        verify_program(&mir, MirPhase::PreEmission).expect("increment materialization is ready");
    }

    #[test]
    fn materializes_dead_byte_decrement_to_dec() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Sub,
                None,
                MirCarryOut::Ignore,
                mem.clone(),
                MirAddr::Direct(mem),
                Vec::new(),
                MirTerminator::Return,
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize byte decrement");

        let formatted = format_program(&mir);
        assert!(formatted.contains("dec.b global g0+0"));
        assert!(!formatted.contains(" sub #$01"));
        verify_program(&mir, MirPhase::PreEmission).expect("decrement materialization is ready");
    }

    #[test]
    fn materializes_byte_increment_before_z_branch_to_inc() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Add,
                None,
                MirCarryOut::Ignore,
                mem.clone(),
                MirAddr::Direct(mem),
                Vec::new(),
                MirTerminator::Branch {
                    cond: MirCond::FlagTest(MirFlagTest::ZSet),
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize byte increment before z branch");

        assert!(format_program(&mir).contains("inc.b global g0+0"));
    }

    #[test]
    fn materializes_loaded_byte_increment_when_next_op_clobbers_flags() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "g".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "next".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(mem.clone()),
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::VTemp(MirTempId(1)),
                                left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                right: MirValue::ConstU8(1),
                                width: MirWidth::Byte,
                                carry_in: None,
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(mem),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize loaded byte increment");

        let formatted = format_program(&mir);
        assert!(formatted.contains("inc.b global g0+0"));
        assert!(!formatted.contains(" add #$01"));
    }

    #[test]
    fn does_not_materialize_increment_when_carry_is_live() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Add,
                None,
                MirCarryOut::Produce,
                mem.clone(),
                MirAddr::Direct(mem),
                Vec::new(),
                MirTerminator::Return,
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize carry-live increment");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("inc.b"));
        assert!(formatted.contains(" add #$01"));
    }

    #[test]
    fn materializes_increment_when_result_can_be_reloaded_from_store() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Add,
                None,
                MirCarryOut::Ignore,
                mem.clone(),
                MirAddr::Direct(mem),
                vec![MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize result-live increment");

        let formatted = format_program(&mir);
        assert!(formatted.contains("inc.b global g0+0"));
        assert!(!formatted.contains(" add #$01"));
    }

    #[test]
    fn does_not_materialize_increment_for_absolute_memory() {
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Add,
                None,
                MirCarryOut::Ignore,
                MirMem::Absolute(0xD000),
                MirAddr::Direct(MirMem::Absolute(0xD000)),
                Vec::new(),
                MirTerminator::Return,
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize absolute increment");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("inc.b"));
        assert!(formatted.contains("store.b $D000"));
    }

    #[test]
    fn does_not_materialize_increment_for_indirect_store() {
        let mem = global_byte_mem();
        let mir = materialize_program(
            byte_update_program(
                MirBinaryOp::Add,
                None,
                MirCarryOut::Ignore,
                mem,
                MirAddr::Deref {
                    ptr: MirValue::ConstU16(0x4000),
                    offset: 0,
                },
                Vec::new(),
                MirTerminator::Return,
            ),
            &Mir6502Config::default(),
        )
        .expect("materialize indirect increment");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("inc.b"));
        assert!(formatted.contains("store_indirect"));
    }

    #[test]
    fn materializes_one_use_load_store_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize one-use load/store");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(formatted.contains("a =.b load global g0+0"));
        assert!(formatted.contains("store.b global g1+0, a"));
        assert_eq!(
            materialize::spill_accounting_for_routine(&mir.routines[0]).allocated,
            0
        );
        verify_program(&mir, MirPhase::PreEmission).expect("direct materialization is ready");
    }

    #[test]
    fn materializes_multi_use_load_store_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst1".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(2),
                        name: "dst2".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 2 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 0,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize multi-use load/store");

        let accounting = materialize::spill_accounting_for_routine(&mir.routines[0]);
        assert_eq!(accounting.allocated, 0);
        assert_eq!(accounting.written, 0);
        assert_eq!(accounting.read, 0);
        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(formatted.contains("store.b global g1+0, a"));
        assert!(formatted.contains("store.b global g2+0, a"));
        verify_program(&mir, MirPhase::PreEmission).expect("multi-use materialization is ready");
    }

    #[test]
    fn removes_unused_temp_producers_before_spilling() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: Vec::new(),
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::Binary {
                            op: MirBinaryOp::Add,
                            dst: MirDef::VTemp(MirTempId(0)),
                            left: MirValue::ConstU8(1),
                            right: MirValue::ConstU8(2),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Ignore,
                        }],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize unused temp cleanup");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(!formatted.contains(" add "));
        verify_program(&mir, MirPhase::PreEmission).expect("unused temp cleanup is ready");
    }

    #[test]
    fn collapses_single_use_temp_producers_before_spilling() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LoadImm {
                                dst: MirDef::VTemp(MirTempId(0)),
                                value: 7,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize single-use temp cleanup");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(formatted.contains("a =.b #7"));
        assert!(formatted.contains("store.b global g0+0, a"));
        verify_program(&mir, MirPhase::PreEmission).expect("single-use temp cleanup is ready");
    }

    #[test]
    fn sinks_single_use_arithmetic_after_unrelated_call() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "dst".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Noop".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(1),
                            label: "noop".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Binary {
                                    op: MirBinaryOp::Add,
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    left: MirValue::ConstU8(1),
                                    right: MirValue::ConstU8(2),
                                    width: MirWidth::Byte,
                                    carry_in: None,
                                    carry_out: MirCarryOut::Ignore,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize sunk arithmetic");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"), "{formatted}");
        assert!(formatted.contains("call r0"), "{formatted}");
        assert!(formatted.contains("a =.b #1"), "{formatted}");
        assert!(
            formatted.contains("a =.b a add #$02 carry_in=clear"),
            "{formatted}"
        );
        assert!(formatted.contains("store.b global g0+0, a"), "{formatted}");
        assert!(
            formatted.find("call r0").unwrap() < formatted.find("a =.b #1").unwrap(),
            "{formatted}"
        );
        verify_program(&mir, MirPhase::PreEmission).expect("sunk arithmetic is ready");
    }

    #[test]
    fn reuses_non_overlapping_basic_block_homes() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: (0..4)
                    .map(|id| MirGlobal {
                        id: SymbolId(id),
                        name: format!("g{id}"),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: id as u16 },
                        init: None,
                    })
                    .collect(),
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame {
                        spills: vec![MirSpillId(0), MirSpillId(2)],
                        ..MirFrame::default()
                    },
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 1,
                                width: MirWidth::Byte,
                            },
                            MirOp::Barrier {
                                effects: MirEffects::default(),
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Spill {
                                    id: MirSpillId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 9,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirAddr::Direct(MirMem::Spill {
                                    id: MirSpillId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 2,
                                width: MirWidth::Byte,
                            },
                            MirOp::Barrier {
                                effects: MirEffects::default(),
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Spill {
                                    id: MirSpillId(2),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 9,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(3),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirAddr::Direct(MirMem::Spill {
                                    id: MirSpillId(2),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize block-local spill coloring");

        let accounting = materialize::spill_accounting_for_routine(&mir.routines[0]);
        let formatted = format_program(&mir);
        assert_eq!(accounting.allocated, 0, "{formatted}");
        assert!(formatted.contains("zp0 -> $E0 size=1"), "{formatted}");
        assert!(formatted.contains("store.b zp0, a"), "{formatted}");
        assert!(formatted.contains("a =.b load zp0"), "{formatted}");
        assert!(!formatted.contains("spill"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("colored spills are ready");
    }

    #[test]
    fn forwards_accumulator_for_repeated_spill_loads() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "dst0".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst1".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LoadImm {
                                dst: MirDef::VTemp(MirTempId(0)),
                                value: 7,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize accumulator spill forwarding");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(!formatted.contains("a =.b load spill sp0+0"));
        assert!(formatted.contains("store.b global g0+0, a"));
        assert!(formatted.contains("store.b global g1+0, a"));
        verify_program(&mir, MirPhase::PreEmission).expect("forwarded spills are ready");
    }

    #[test]
    fn removes_dead_spill_store_after_accumulator_forwarding() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "src".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame {
                        spills: vec![MirSpillId(0)],
                        fixed_zero_page: vec![MirFixedZpSlot(0xE6)],
                        ..MirFrame::default()
                    },
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Spill {
                                    id: MirSpillId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::A),
                                src: MirAddr::Direct(MirMem::Spill {
                                    id: MirSpillId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::Y),
                                value: 0,
                                width: MirWidth::Byte,
                            },
                            MirOp::StoreIndirect {
                                consumer: MirAddressConsumer::IndirectIndexedY(
                                    MirPointerPair::Fixed {
                                        lo: MirFixedZpSlot(0xE6),
                                    },
                                ),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                offset: 0,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize dead spill store pruning");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(formatted.contains("a =.b load global g0+0"));
        assert!(formatted.contains("store_indirect (zp$E6),y+0 a"));
        verify_program(&mir, MirPhase::PreEmission).expect("dead spill store pruning is ready");
    }

    #[test]
    fn forwards_accumulator_across_short_byte_op_chain() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![
                        MirTemp { id: MirTempId(0) },
                        MirTemp { id: MirTempId(1) },
                        MirTemp { id: MirTempId(2) },
                    ],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::VTemp(MirTempId(1)),
                                left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                right: MirValue::ConstU8(1),
                                width: MirWidth::Byte,
                                carry_in: Some(MirCarryIn::Clear),
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::And,
                                dst: MirDef::VTemp(MirTempId(2)),
                                left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                right: MirValue::ConstU8(0x0f),
                                width: MirWidth::Byte,
                                carry_in: None,
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize accumulator op chain");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(!formatted.contains("zp0"));
        assert!(formatted.contains("a =.b load global g0+0"));
        assert!(formatted.contains("a =.b a add #$01"));
        assert!(formatted.contains("a =.b a and #$0F"));
        assert!(formatted.contains("store.b global g1+0, a"));
        verify_program(&mir, MirPhase::PreEmission).expect("forwarded op chain is ready");
    }

    #[test]
    fn keeps_byte_op_chain_home_when_intermediate_is_reused() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst0".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(2),
                        name: "dst1".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 2 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![
                        MirTemp { id: MirTempId(0) },
                        MirTemp { id: MirTempId(1) },
                        MirTemp { id: MirTempId(2) },
                    ],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::VTemp(MirTempId(1)),
                                left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                right: MirValue::ConstU8(1),
                                width: MirWidth::Byte,
                                carry_in: Some(MirCarryIn::Clear),
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Xor,
                                dst: MirDef::VTemp(MirTempId(2)),
                                left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                right: MirValue::ConstU8(0x80),
                                width: MirWidth::Byte,
                                carry_in: None,
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(2),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize reused byte op chain");

        let formatted = format_program(&mir);
        assert!(formatted.contains("store.b zp0, a"));
        assert!(formatted.contains("a =.b load zp0"));
        assert!(formatted.contains("store.b global g1+0, a"));
        assert!(formatted.contains("store.b global g2+0, a"));
        verify_program(&mir, MirPhase::PreEmission).expect("reused chain home is ready");
    }

    #[test]
    fn rematerializes_zero_page_pointer_temp_for_deref_after_call() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "screen".to_string(),
                    kind: "BytePointer".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Absolute(0x00E6),
                    init: None,
                }],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Word,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(1)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                                MirOp::Store {
                                    dst: MirAddr::Deref {
                                        ptr: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                        offset: 0,
                                    },
                                    src: MirValue::ConstU8(0x41),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Noop".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(1),
                            label: "bb1".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize zero page pointer deref after call");

        let formatted = format_program(&mir);
        assert!(formatted.contains("call r1"));
        assert!(formatted.contains("store_indirect (zp$E6),y+0 a"));
        assert!(!formatted.contains("spill"));
        assert!(!formatted.contains("fixed_zp $AC"));
        assert!(!formatted.contains("fixed_zp $AD"));
        verify_program(&mir, MirPhase::PreEmission).expect("zero page pointer deref is ready");
    }

    #[test]
    fn sinks_chain_after_original_temp_last_use_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "dst0".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst1".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LoadImm {
                                dst: MirDef::VTemp(MirTempId(0)),
                                value: 7,
                                width: MirWidth::Byte,
                            },
                            MirOp::Binary {
                                op: MirBinaryOp::Add,
                                dst: MirDef::VTemp(MirTempId(1)),
                                left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                right: MirValue::ConstU8(1),
                                width: MirWidth::Byte,
                                carry_in: Some(MirCarryIn::Clear),
                                carry_out: MirCarryOut::Ignore,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize reused accumulator op chain");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("store.b zp0, a"));
        assert!(!formatted.contains("load zp0"));
        assert!(!formatted.contains("spill"));
        let first_store = formatted
            .find("store.b global g0+0, a")
            .expect("original value is stored first");
        let add = formatted
            .find("a =.b a add #$01")
            .expect("the chain producer remains in A");
        let second_store = formatted
            .find("store.b global g1+0, a")
            .expect("chain result is stored last");
        assert!(first_store < add && add < second_store);
        verify_program(&mir, MirPhase::PreEmission).expect("reused chain spill is ready");
    }

    #[test]
    fn forwards_immediate_temp_consumer_to_x_register() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "src".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::Move {
                                dst: MirDef::Reg(MirReg::X),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize X register temp consumer");

        let formatted = format_program(&mir);
        assert!(formatted.contains("x =.b load global g0+0"));
        assert!(!formatted.contains("spill"));
        assert!(!formatted.contains("zp"));
        verify_program(&mir, MirPhase::PreEmission).expect("X consumer forwarding is ready");
    }

    #[test]
    fn does_not_forward_spill_load_across_call() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Callee".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Byte,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(1),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize call barrier spill forwarding");

        assert!(format_program(&mir).contains("a =.b load spill sp0+0"));
        verify_program(&mir, MirPhase::PreEmission).expect("call barrier spill reload is ready");
    }

    #[test]
    fn does_not_forward_spill_load_after_a_clobber() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Byte,
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 9,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize A clobber spill forwarding");

        let formatted = format_program(&mir);
        assert!(formatted.contains("a =.b load global g0+0"), "{formatted}");
        assert!(formatted.contains("store.b global g1+0, a"), "{formatted}");
        assert!(!formatted.contains("spill"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("A clobber spill reload is ready");
    }

    #[test]
    fn does_not_color_spills_live_across_blocks() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "g".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![
                        MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![MirOp::LoadImm {
                                dst: MirDef::VTemp(MirTempId(0)),
                                value: 1,
                                width: MirWidth::Byte,
                            }],
                            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                        },
                        MirBlock {
                            id: MirBlockId(1),
                            label: "bb1".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                                MirOp::LoadImm {
                                    dst: MirDef::VTemp(MirTempId(1)),
                                    value: 2,
                                    width: MirWidth::Byte,
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        },
                    ],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize cross-block spill coloring guard");

        let accounting = materialize::spill_accounting_for_routine(&mir.routines[0]);
        assert_eq!(accounting.allocated, 1);
        verify_program(&mir, MirPhase::PreEmission).expect("cross-block spills are ready");
    }

    #[test]
    fn materializes_one_use_call_result_store_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "dst".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Callee".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: Some(MirCallResult {
                                        dst: MirDef::VTemp(MirTempId(0)),
                                        width: MirWidth::Byte,
                                        home: MirResultHome::ReturnSlot { offset: 0 },
                                    }),
                                    effects: MirEffects::default(),
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize call-result store");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(formatted.contains("call r0"));
        assert!(formatted.contains("a =.b load fixed_zp $A0"));
        assert!(formatted.contains("store.b global g0+0, a"));
        assert_eq!(
            materialize::spill_accounting_for_routine(&mir.routines[1]).allocated,
            0
        );
        verify_program(&mir, MirPhase::PreEmission).expect("call-result materialization is ready");
    }

    #[test]
    fn materializes_one_use_call_result_indirect_store_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "ptr".to_string(),
                    kind: "Byte Pointer".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Absolute(0x00E6),
                    init: None,
                }],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Callee".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: Some(MirCallResult {
                                        dst: MirDef::VTemp(MirTempId(0)),
                                        width: MirWidth::Byte,
                                        home: MirResultHome::ReturnSlot { offset: 0 },
                                    }),
                                    effects: MirEffects::default(),
                                },
                                MirOp::Store {
                                    dst: MirAddr::PointerCell {
                                        ptr: MirMem::Global {
                                            id: SymbolId(0),
                                            offset: 0,
                                        },
                                        offset: 2,
                                    },
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize call-result indirect store");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"));
        assert!(formatted.contains("call r0"));
        assert!(formatted.contains("a =.b load fixed_zp $A0"));
        assert!(
            formatted.contains("store_indirect (zp$E6),y+2 a"),
            "{formatted}"
        );
        assert_eq!(
            materialize::spill_accounting_for_routine(&mir.routines[1]).allocated,
            0
        );
        verify_program(&mir, MirPhase::PreEmission)
            .expect("call-result indirect materialization is ready");
    }

    #[test]
    fn materializes_one_use_call_arg_producers_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Card Pointer".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte Array".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 16,
                        backing: MirGlobalBacking::Ordinary { offset: 2 },
                        init: None,
                    },
                ],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Take".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![
                            MirTemp { id: MirTempId(0) },
                            MirTemp { id: MirTempId(1) },
                            MirTemp { id: MirTempId(2) },
                        ],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Word,
                                },
                                MirOp::LeaAddr {
                                    dst: MirDef::VTemp(MirTempId(1)),
                                    target: MirMem::Global {
                                        id: SymbolId(1),
                                        offset: 0,
                                    },
                                    width: MirWidth::Word,
                                },
                                MirOp::LoadImm {
                                    dst: MirDef::VTemp(MirTempId(2)),
                                    value: 8,
                                    width: MirWidth::Byte,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: vec![
                                            MirArgHome::RegisterPair {
                                                lo: MirReg::A,
                                                hi: MirReg::X,
                                            },
                                            MirArgHome::BytePair {
                                                lo: Box::new(MirArgHome::Reg(MirReg::Y)),
                                                hi: Box::new(MirArgHome::FixedZeroPage(
                                                    MirFixedZpSlot(0xA3),
                                                )),
                                            },
                                            MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA4)),
                                        ],
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: vec![
                                        MirCallArg {
                                            value: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                            width: MirWidth::Word,
                                            home: MirArgHome::RegisterPair {
                                                lo: MirReg::A,
                                                hi: MirReg::X,
                                            },
                                        },
                                        MirCallArg {
                                            value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                            width: MirWidth::Word,
                                            home: MirArgHome::BytePair {
                                                lo: Box::new(MirArgHome::Reg(MirReg::Y)),
                                                hi: Box::new(MirArgHome::FixedZeroPage(
                                                    MirFixedZpSlot(0xA3),
                                                )),
                                            },
                                        },
                                        MirCallArg {
                                            value: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                            width: MirWidth::Byte,
                                            home: MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA4)),
                                        },
                                    ],
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize call-arg producer folding");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"), "{formatted}");
        assert!(formatted.contains("a =.b load global g0+0"), "{formatted}");
        assert!(formatted.contains("x =.b load global g0+1"), "{formatted}");
        assert!(
            formatted.contains("y =.b storage_addr_lo global g1+0"),
            "{formatted}"
        );
        assert!(
            formatted.contains("a =.b storage_addr_hi global g1+0"),
            "{formatted}"
        );
        assert!(formatted.contains("store.b fixed_zp $A3, a"), "{formatted}");
        assert!(formatted.contains("a =.b #8"), "{formatted}");
        assert!(formatted.contains("store.b fixed_zp $A4, a"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("call args are emission-ready");
    }

    #[test]
    fn materializes_one_use_binary_call_arg_producers_without_spill() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "x".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "y".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 2 },
                        init: None,
                    },
                ],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Take".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: (0..6).map(|id| MirTemp { id: MirTempId(id) }).collect(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Word,
                                },
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(1)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(1),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Word,
                                },
                                MirOp::Binary {
                                    op: MirBinaryOp::Add,
                                    dst: MirDef::VTemp(MirTempId(2)),
                                    left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    right: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                    width: MirWidth::Word,
                                    carry_in: None,
                                    carry_out: MirCarryOut::Ignore,
                                },
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(3)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Word,
                                },
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(4)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(1),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Word,
                                },
                                MirOp::Binary {
                                    op: MirBinaryOp::Sub,
                                    dst: MirDef::VTemp(MirTempId(5)),
                                    left: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                                    right: MirValue::Def(MirDef::VTemp(MirTempId(4))),
                                    width: MirWidth::Word,
                                    carry_in: None,
                                    carry_out: MirCarryOut::Ignore,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: vec![
                                            MirArgHome::RegisterPair {
                                                lo: MirReg::A,
                                                hi: MirReg::X,
                                            },
                                            MirArgHome::Reg(MirReg::Y),
                                        ],
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: vec![
                                        MirCallArg {
                                            value: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                                            width: MirWidth::Word,
                                            home: MirArgHome::RegisterPair {
                                                lo: MirReg::A,
                                                hi: MirReg::X,
                                            },
                                        },
                                        MirCallArg {
                                            value: MirValue::Def(MirDef::VTemp(MirTempId(5))),
                                            width: MirWidth::Byte,
                                            home: MirArgHome::Reg(MirReg::Y),
                                        },
                                    ],
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize binary call-arg producer folding");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"), "{formatted}");
        assert!(formatted.contains("y =.b a"), "{formatted}");
        assert!(formatted.contains("store.b fixed_zp $A0, a"), "{formatted}");
        assert!(formatted.contains("x =.b a"), "{formatted}");
        assert!(formatted.contains("a =.b load fixed_zp $A0"), "{formatted}");
        assert!(formatted.contains("call r0 args=[a.b -> a, x.b -> x, y.b -> y]"));
        verify_program(&mir, MirPhase::PreEmission).expect("binary call args are emission-ready");
    }

    #[test]
    fn rematerializes_later_call_arg_address_after_call() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "dst".to_string(),
                    kind: "Byte Array".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 16,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Noop".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(1),
                            label: "noop".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Take".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(2),
                            label: "take".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(2),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::LeaAddr {
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    target: MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    },
                                    width: MirWidth::Word,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: Vec::new(),
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: Vec::new(),
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(1)),
                                    abi: MirCallAbi {
                                        params: vec![MirArgHome::RegisterPair {
                                            lo: MirReg::A,
                                            hi: MirReg::X,
                                        }],
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: vec![MirCallArg {
                                        value: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                        width: MirWidth::Word,
                                        home: MirArgHome::RegisterPair {
                                            lo: MirReg::A,
                                            hi: MirReg::X,
                                        },
                                    }],
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize call-arg address rematerialization");

        let formatted = format_program(&mir);
        assert!(!formatted.contains("spill"), "{formatted}");
        assert!(formatted.contains("call r0"), "{formatted}");
        assert!(
            formatted.contains("a =.b storage_addr_lo global g0+0"),
            "{formatted}"
        );
        assert!(
            formatted.contains("x =.b storage_addr_hi global g0+0"),
            "{formatted}"
        );
        assert!(formatted.contains("call r1"), "{formatted}");
        assert!(
            formatted.find("call r0").unwrap()
                < formatted.find("a =.b storage_addr_lo global g0+0").unwrap(),
            "{formatted}"
        );
        verify_program(&mir, MirPhase::PreEmission).expect("call arg address is emission-ready");
    }

    #[test]
    fn keeps_call_arg_producer_spilled_when_reused_after_call() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "src".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "dst".to_string(),
                        kind: "Byte".to_string(),
                        width: Some(MirWidth::Byte),
                        storage_size: 1,
                        backing: MirGlobalBacking::Ordinary { offset: 1 },
                        init: None,
                    },
                ],
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Take".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: vec![MirTemp { id: MirTempId(0) }],
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Load {
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    src: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    width: MirWidth::Byte,
                                },
                                MirOp::Call {
                                    target: MirCallTarget::Routine(RoutineId(0)),
                                    abi: MirCallAbi {
                                        params: vec![MirArgHome::Reg(MirReg::A)],
                                        result: None,
                                        clobbers: MirRegisterSet::default(),
                                        preserves: MirRegisterSet::default(),
                                    },
                                    args: vec![MirCallArg {
                                        value: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                        width: MirWidth::Byte,
                                        home: MirArgHome::Reg(MirReg::A),
                                    }],
                                    result: None,
                                    effects: MirEffects::default(),
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(1),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                            ],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize reused call arg producer");

        let formatted = format_program(&mir);
        assert!(formatted.contains("spill"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("reused call arg remains valid");
    }

    #[test]
    fn materializes_word_call_arg_to_canonical_register_pair() {
        let mir = materialize_program(
            MirProgram {
                statics: vec![MirStatic {
                    id: SymbolId(0),
                    name: "__str".to_string(),
                    ty: "Char*".to_string(),
                    bytes: b"HI\0".to_vec(),
                    display: "\"HI\"".to_string(),
                    alignment: 1,
                    mutable: false,
                    section: "rodata".to_string(),
                }],
                globals: Vec::new(),
                routines: vec![
                    MirRoutine {
                        id: RoutineId(0),
                        name: "Take".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                    MirRoutine {
                        id: RoutineId(1),
                        name: "Main".to_string(),
                        abi: MirRoutineAbi::Action,
                        frame: MirFrame::default(),
                        temps: Vec::new(),
                        blocks: vec![MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![MirOp::Call {
                                target: MirCallTarget::Routine(RoutineId(0)),
                                abi: MirCallAbi {
                                    params: vec![MirArgHome::RegisterPair {
                                        lo: MirReg::A,
                                        hi: MirReg::X,
                                    }],
                                    result: None,
                                    clobbers: MirRegisterSet::default(),
                                    preserves: MirRegisterSet::default(),
                                },
                                args: vec![MirCallArg {
                                    value: MirValue::StaticAddr(SymbolId(0)),
                                    width: MirWidth::Word,
                                    home: MirArgHome::RegisterPair {
                                        lo: MirReg::A,
                                        hi: MirReg::X,
                                    },
                                }],
                                result: None,
                                effects: MirEffects::default(),
                            }],
                            terminator: MirTerminator::Return,
                        }],
                        effects: MirEffects::default(),
                    },
                ],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize canonical word call arg");

        let formatted = format_program(&mir);
        assert!(
            formatted.contains("a =.b storage_addr_lo static s0+0"),
            "{formatted}"
        );
        assert!(
            formatted.contains("x =.b storage_addr_hi static s0+0"),
            "{formatted}"
        );
        assert!(formatted.contains("call r0 args=[a.b -> a, x.b -> x]"));
        assert!(!formatted.contains("stack $0000"), "{formatted}");
        verify_program(&mir, MirPhase::PreEmission).expect("call arg is emission-ready");
    }

    #[test]
    fn materializes_word_pointer_deref_read_to_indirect_y() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![
                    MirGlobal {
                        id: SymbolId(0),
                        name: "p".to_string(),
                        kind: "Card Pointer".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 0 },
                        init: None,
                    },
                    MirGlobal {
                        id: SymbolId(1),
                        name: "x".to_string(),
                        kind: "Card".to_string(),
                        width: Some(MirWidth::Word),
                        storage_size: 2,
                        backing: MirGlobalBacking::Ordinary { offset: 2 },
                        init: None,
                    },
                ],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::ConstU16(0x0580),
                                width: MirWidth::Word,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Word,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(1)),
                                src: MirAddr::Deref {
                                    ptr: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    offset: 0,
                                },
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(1),
                                    offset: 0,
                                }),
                                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize pointer deref read");

        verify_program(&mir, MirPhase::PreEmission).expect("pointer deref is emission-ready");

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit pointer deref read");
        let bytes = emitter.finish().expect("finish emitter");
        assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
        assert!(bytes.windows(2).any(|bytes| bytes == [0xA0, 0x00]));
        assert!(bytes.windows(2).any(|bytes| bytes == [0xC8, 0xB1]));
    }

    #[test]
    fn materializes_word_pointer_deref_write_to_indirect_y() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "p".to_string(),
                    kind: "Card Pointer".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }],
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                src: MirValue::ConstU16(0x0580),
                                width: MirWidth::Word,
                            },
                            MirOp::Load {
                                dst: MirDef::VTemp(MirTempId(0)),
                                src: MirAddr::Direct(MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                }),
                                width: MirWidth::Word,
                            },
                            MirOp::Store {
                                dst: MirAddr::Deref {
                                    ptr: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    offset: 0,
                                },
                                src: MirValue::ConstU16(0x1234),
                                width: MirWidth::Word,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize pointer deref write");

        verify_program(&mir, MirPhase::PreEmission).expect("pointer deref write is emission-ready");

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit pointer deref write");
        let bytes = emitter.finish().expect("finish emitter");
        assert!(
            bytes
                .windows(5)
                .any(|bytes| bytes == [0xAD, 0x00, 0x30, 0x85, 0xAC])
        );
        assert!(
            bytes
                .windows(5)
                .any(|bytes| bytes == [0xAD, 0x01, 0x30, 0x85, 0xAD])
        );
        assert!(
            !bytes
                .windows(5)
                .any(|bytes| bytes == [0xAD, 0x02, 0x30, 0x85, 0xAC])
        );
        assert!(
            !bytes
                .windows(5)
                .any(|bytes| bytes == [0xAD, 0x03, 0x30, 0x85, 0xAD])
        );
        assert!(
            bytes
                .windows(4)
                .any(|bytes| bytes == [0xA0, 0x00, 0x91, 0xAC])
        );
        assert!(bytes.windows(3).any(|bytes| bytes == [0xC8, 0x91, 0xAC]));
        assert!(bytes.windows(2).any(|bytes| bytes == [0x91, 0xAC]));
        assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x34]));
        assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x12]));
    }

    #[test]
    fn materializes_helper_binary_and_byte_eq_branch() {
        let mir = materialize_program(
            MirProgram {
                statics: Vec::new(),
                globals: vec![MirGlobal {
                    id: SymbolId(0),
                    name: "product".to_string(),
                    kind: "Byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                }],
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
                    blocks: vec![
                        MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::Binary {
                                    op: MirBinaryOp::Mul,
                                    dst: MirDef::VTemp(MirTempId(0)),
                                    left: MirValue::ConstU8(3),
                                    right: MirValue::ConstU8(4),
                                    width: MirWidth::Byte,
                                    carry_in: None,
                                    carry_out: MirCarryOut::Ignore,
                                },
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Global {
                                        id: SymbolId(0),
                                        offset: 0,
                                    }),
                                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                    width: MirWidth::Byte,
                                },
                                MirOp::Compare {
                                    dst: MirCondDest::Temp(MirTempId(1)),
                                    op: MirCompareOp::Eq,
                                    left: MirValue::ConstU8(1),
                                    right: MirValue::ConstU8(1),
                                    width: MirWidth::Byte,
                                    signed: false,
                                },
                            ],
                            terminator: MirTerminator::Branch {
                                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(
                                    1,
                                )))),
                                then_edge: MirEdge::plain(MirBlockId(1)),
                                else_edge: MirEdge::plain(MirBlockId(2)),
                            },
                        },
                        MirBlock {
                            id: MirBlockId(1),
                            label: "then".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        },
                        MirBlock {
                            id: MirBlockId(2),
                            label: "else".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        },
                    ],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            &Mir6502Config::default(),
        )
        .expect("materialize helper and branch");

        let formatted = format_program(&mir);
        assert!(formatted.contains("helper mul"));
        assert!(
            formatted.contains("branch fused b0:") && formatted.contains("z_set ? b1 : b2"),
            "{formatted}"
        );
        assert_eq!(mir.runtime_helpers.len(), 1);
        assert!(matches!(
            mir.runtime_helpers[0].target,
            MirRuntimeHelperTarget::KnownAbsolute(0xA000)
        ));
        verify_program(&mir, MirPhase::PreEmission).expect("resolved helper target verifies");
    }

    #[test]
    fn peephole_compare_branch_fusion_can_be_disabled() {
        let config = Mir6502Config {
            enable_peepholes: false,
            ..Mir6502Config::default()
        };
        let mir = materialize_program(compare_branch_program(Vec::new()), &config)
            .expect("materialize without peepholes");

        let formatted = format_program(&mir);
        assert!(formatted.contains("branch bool v0 ? b1 : b2"));
        assert!(!formatted.contains("branch fused"));
    }

    #[test]
    fn peephole_compare_branch_fusion_preserves_barriers() {
        let mir = materialize_program(
            compare_branch_program(vec![MirOp::Barrier {
                effects: MirEffects::default(),
            }]),
            &Mir6502Config::default(),
        )
        .expect("materialize with barrier");

        let formatted = format_program(&mir);
        assert!(formatted.contains("barrier effects="));
        assert!(formatted.contains("branch bool v0 ? b1 : b2"));
        assert!(!formatted.contains("branch fused"));
    }

    fn compare_branch_program(mut extra_ops: Vec<MirOp>) -> MirProgram {
        let mut ops = vec![MirOp::Compare {
            dst: MirCondDest::Temp(MirTempId(0)),
            op: MirCompareOp::Eq,
            left: MirValue::ConstU8(1),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            signed: false,
        }];
        ops.append(&mut extra_ops);
        MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: vec![MirTemp { id: MirTempId(0) }],
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops,
                        terminator: MirTerminator::Branch {
                            cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(0)))),
                            then_edge: MirEdge::plain(MirBlockId(1)),
                            else_edge: MirEdge::plain(MirBlockId(2)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "then".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "else".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        }
    }

    #[test]
    fn pre_emission_rejects_abstract_values_and_conditions() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: vec![MirTemp { id: MirTempId(0) }],
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::LoadImm {
                            dst: MirDef::VTemp(MirTempId(0)),
                            value: 1,
                            width: MirWidth::Word,
                        }],
                        terminator: MirTerminator::Branch {
                            cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(0)))),
                            then_edge: MirEdge::plain(MirBlockId(1)),
                            else_edge: MirEdge::plain(MirBlockId(1)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "done".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: vec![MirRuntimeHelperDecl {
                helper: MirRuntimeHelper::Mul,
                target: MirRuntimeHelperTarget::Deferred,
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: None,
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                effects: MirEffects::default(),
            }],
        };

        let diagnostics = verify_program(&mir, MirPhase::PreEmission)
            .expect_err("pre-emission rejects pseudo MIR");
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(messages.contains("deferred runtime helper targets"));
        assert!(messages.contains("abstract bool branch conditions"));
        assert!(messages.contains("virtual temp `v0`"));
        assert!(messages.contains("word-width pseudo ops"));
    }

    #[test]
    fn emits_simple_pre_emission_byte_program() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: vec![MirGlobal {
                id: SymbolId(0),
                name: "b".to_string(),
                kind: "Byte".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Ordinary { offset: 0 },
                init: None,
            }],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 7,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Global {
                                id: SymbolId(0),
                                offset: 0,
                            }),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        let summary = emit_program(&mir, &mut emitter).expect("emit pre-emission MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x00, 0xA9, 0x07, 0x8D, 0x00, 0x30, 0x60]);
        assert_eq!(summary.routine_addresses[0].address, 0x3001);
        assert_eq!(summary.routine_ranges[0].start, 0x3001);
        assert_eq!(summary.routine_ranges[0].end, 0x3007);
        assert_eq!(summary.storage_symbols[0].name, "b");
        assert_eq!(summary.storage_symbols[0].address, 0x3000);
        assert_eq!(summary.storage_symbols[0].size, 1);
        assert_eq!(
            summary.source_ranges[0].kind,
            crate::codegen::CodegenSourceRangeKind::Routine
        );
        assert_eq!(summary.source_ranges[0].start, 0x3001);
    }

    #[test]
    fn mir6502_emission_preserves_absolute_global_aliases() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: vec![MirGlobal {
                id: SymbolId(0),
                name: "COLOR".to_string(),
                kind: "Byte".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0x02C8),
                init: None,
            }],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 10,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Global {
                                id: SymbolId(0),
                                offset: 0,
                            }),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit absolute alias MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xA9, 0x0A, 0x8D, 0xC8, 0x02, 0x60]);
    }

    #[test]
    fn mir6502_emission_writes_static_bytes_once() {
        let mir = MirProgram {
            statics: vec![MirStatic {
                id: SymbolId(0),
                name: "s".to_string(),
                ty: "Byte[]".to_string(),
                bytes: vec![1, 2, 3],
                display: "[$01,$02,$03]".to_string(),
                alignment: 1,
                mutable: false,
                section: "rodata".to_string(),
            }],
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(MirMem::Static {
                            id: SymbolId(0),
                            offset: 1,
                        }),
                        width: MirWidth::Byte,
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit static MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![1, 2, 3, 0xAD, 0x01, 0x30, 0x60]);
    }

    #[test]
    fn mir6502_emission_uses_direct_fixed_zero_page_opcodes() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x80)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 0x55,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x80))),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit fixed zero-page MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xA9, 0x55, 0x85, 0x80, 0x60]);
    }

    #[test]
    fn mir6502_emission_places_direct_local_storage() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    locals: vec![MirStorageSlot {
                        id: MirStorageId(0),
                        name: Some("local".to_string()),
                        storage: MirStorageClass::Scalar,
                        width: MirWidth::Byte,
                        base: MirStorageBase::Local(LocalId(0)),
                        offset: 0,
                        mutable: true,
                        init: None,
                    }],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 4,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Local {
                                id: LocalId(0),
                                offset: 0,
                            }),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit local storage MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x00, 0xA9, 0x04, 0x8D, 0x00, 0x30, 0x60]);
    }

    #[test]
    fn mir6502_emission_emits_direct_byte_arithmetic() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 4,
                            width: MirWidth::Byte,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Add,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(3),
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::Clear),
                            carry_out: MirCarryOut::Ignore,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit direct arithmetic MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(
            bytes,
            vec![0xA9, 0x04, 0x18, 0x69, 0x03, 0x8D, 0x00, 0x40, 0x60]
        );
    }

    #[test]
    fn mir6502_emission_emits_byte_sub_and_logic() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 0x10,
                            width: MirWidth::Byte,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Sub,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(0x03),
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::Set),
                            carry_out: MirCarryOut::Ignore,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::And,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(0x0F),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Ignore,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Or,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(0x80),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Ignore,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Xor,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(0xFF),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Ignore,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit byte sub/logic MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(
            bytes,
            vec![
                0xA9, 0x10, 0x38, 0xE9, 0x03, 0x29, 0x0F, 0x09, 0x80, 0x49, 0xFF, 0x60,
            ]
        );
    }

    #[test]
    fn mir6502_emission_emits_eq_ne_and_carry_flag_branches() {
        fn branch_program(test: MirFlagTest) -> Vec<u8> {
            let mir = MirProgram {
                statics: Vec::new(),
                globals: Vec::new(),
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![
                        MirBlock {
                            id: MirBlockId(0),
                            label: "bb0".to_string(),
                            params: Vec::new(),
                            ops: vec![
                                MirOp::LoadImm {
                                    dst: MirDef::Reg(MirReg::A),
                                    value: 5,
                                    width: MirWidth::Byte,
                                },
                                MirOp::Compare {
                                    dst: MirCondDest::Flags,
                                    op: MirCompareOp::Eq,
                                    left: MirValue::Def(MirDef::Reg(MirReg::A)),
                                    right: MirValue::ConstU8(5),
                                    width: MirWidth::Byte,
                                    signed: false,
                                },
                            ],
                            terminator: MirTerminator::Branch {
                                cond: MirCond::FlagTest(test),
                                then_edge: MirEdge::plain(MirBlockId(1)),
                                else_edge: MirEdge::plain(MirBlockId(2)),
                            },
                        },
                        MirBlock {
                            id: MirBlockId(1),
                            label: "then".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        },
                        MirBlock {
                            id: MirBlockId(2),
                            label: "else".to_string(),
                            params: Vec::new(),
                            ops: Vec::new(),
                            terminator: MirTerminator::Return,
                        },
                    ],
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            };

            let mut emitter =
                crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
            emit_program(&mir, &mut emitter).expect("emit branch MIR");
            emitter.finish().expect("finish emitter")
        }

        assert_eq!(
            branch_program(MirFlagTest::ZSet),
            vec![0xA9, 0x05, 0xC9, 0x05, 0xD0, 0x01, 0x60, 0x60]
        );
        assert_eq!(
            branch_program(MirFlagTest::ZClear),
            vec![0xA9, 0x05, 0xC9, 0x05, 0xF0, 0x01, 0x60, 0x60]
        );
        assert_eq!(
            branch_program(MirFlagTest::CClear),
            vec![0xA9, 0x05, 0xC9, 0x05, 0xB0, 0x01, 0x60, 0x60]
        );
        assert_eq!(
            branch_program(MirFlagTest::CSet),
            vec![0xA9, 0x05, 0xC9, 0x05, 0x90, 0x01, 0x60, 0x60]
        );
    }

    #[test]
    fn mir6502_emission_omits_cmp_zero_for_z_branches() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 0,
                                width: MirWidth::Byte,
                            },
                            MirOp::Compare {
                                dst: MirCondDest::Flags,
                                op: MirCompareOp::Eq,
                                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                                right: MirValue::ConstU8(0),
                                width: MirWidth::Byte,
                                signed: false,
                            },
                        ],
                        terminator: MirTerminator::Branch {
                            cond: MirCond::FlagTest(MirFlagTest::ZSet),
                            then_edge: MirEdge::plain(MirBlockId(1)),
                            else_edge: MirEdge::plain(MirBlockId(2)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "then".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "else".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit zero compare branch MIR");
        assert_eq!(
            emitter.finish().expect("finish emitter"),
            vec![0xA9, 0x00, 0xD0, 0x01, 0x60, 0x60]
        );
    }

    #[test]
    fn mir6502_emission_omits_jump_to_next_block() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 1,
                            width: MirWidth::Byte,
                        }],
                        terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "bb1".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 2,
                            width: MirWidth::Byte,
                        }],
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit fallthrough jump MIR");
        assert_eq!(
            emitter.finish().expect("finish emitter"),
            vec![0xA9, 0x01, 0xA9, 0x02, 0x60]
        );
    }

    #[test]
    fn mir6502_emission_emits_materialized_word_byte_lanes() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: vec![
                MirGlobal {
                    id: SymbolId(0),
                    name: "src".to_string(),
                    kind: "Card".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: Some(MirGlobalInit::Bytes {
                        bytes: vec![0x34, 0x12],
                        zero_fill: 0,
                        mutable: true,
                        section: "data".to_string(),
                    }),
                },
                MirGlobal {
                    id: SymbolId(1),
                    name: "dst".to_string(),
                    kind: "Card".to_string(),
                    width: Some(MirWidth::Word),
                    storage_size: 2,
                    backing: MirGlobalBacking::Ordinary { offset: 2 },
                    init: None,
                },
            ],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(MirMem::Global {
                                id: SymbolId(0),
                                offset: 0,
                            }),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Global {
                                id: SymbolId(1),
                                offset: 0,
                            }),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(MirMem::Global {
                                id: SymbolId(0),
                                offset: 1,
                            }),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Global {
                                id: SymbolId(1),
                                offset: 1,
                            }),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit byte-lane word MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(
            bytes,
            vec![
                0x34, 0x12, 0x00, 0x00, 0xAD, 0x00, 0x30, 0x8D, 0x02, 0x30, 0xAD, 0x01, 0x30, 0x8D,
                0x03, 0x30, 0x60,
            ]
        );
    }

    #[test]
    fn mir6502_emission_resolves_virtual_zero_page_direct_memory() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0)],
                    zero_page_allocations: vec![MirZpAllocation {
                        slot: MirZpSlot(0),
                        start: MirFixedZpSlot(0xE0),
                        size: 1,
                    }],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 0x22,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(0))),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(0))),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit virtual zero-page MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xA9, 0x22, 0x85, 0xE0, 0xA5, 0xE0, 0x60]);
    }

    #[test]
    fn mir6502_emission_emits_absolute_indexed_address_forms() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: vec![MirGlobal {
                id: SymbolId(0),
                name: "arr".to_string(),
                kind: "Byte[]".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 4,
                backing: MirGlobalBacking::Ordinary { offset: 0 },
                init: Some(MirGlobalInit::Bytes {
                    bytes: vec![1, 2, 3, 4],
                    zero_fill: 0,
                    mutable: true,
                    section: "data".to_string(),
                }),
            }],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::AbsoluteIndexedX {
                                base: MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 0,
                                },
                            },
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::AbsoluteIndexedY {
                                base: MirMem::Global {
                                    id: SymbolId(0),
                                    offset: 1,
                                },
                            },
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit indexed absolute MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(
            bytes,
            vec![1, 2, 3, 4, 0xBD, 0x00, 0x30, 0x99, 0x01, 0x30, 0x60]
        );
    }

    #[test]
    fn mir6502_emission_emits_zero_page_indexed_x_address_forms() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0)],
                    zero_page_allocations: vec![MirZpAllocation {
                        slot: MirZpSlot(0),
                        start: MirFixedZpSlot(0xE0),
                        size: 1,
                    }],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::ZeroPageIndexedX { base: MirZpSlot(0) },
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::ZeroPageIndexedX { base: MirZpSlot(0) },
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit indexed zero-page MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xB5, 0xE0, 0x95, 0xE0, 0x60]);
    }

    #[test]
    fn mir6502_emission_emits_indirect_indexed_y_address_forms() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0)],
                    zero_page_allocations: vec![MirZpAllocation {
                        slot: MirZpSlot(0),
                        start: MirFixedZpSlot(0xE2),
                        size: 2,
                    }],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::IndirectIndexedY { zp: MirZpSlot(0) },
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::IndirectIndexedY { zp: MirZpSlot(0) },
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit indirect indexed MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xB1, 0xE2, 0x91, 0xE2, 0x60]);
    }

    #[test]
    fn mir6502_emission_uses_stack_carry_for_word_index_address_advance() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::MaterializeAddress {
                            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                                lo: MirFixedZpSlot(0xAC),
                            }),
                            value: MirValue::ConstU16(0x4000),
                        },
                        MirOp::AdvanceAddress {
                            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                                lo: MirFixedZpSlot(0xAC),
                            }),
                            index: MirValue::ConstU8(0x82),
                            scale: 2,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit word index address advance");
        let bytes = emitter.finish().expect("finish emitter");

        assert!(bytes.windows(15).any(|window| window
            == [
                0x0A, 0x08, 0x18, 0x65, 0xAC, 0x85, 0xAC, 0xA9, 0x00, 0x2A, 0x28, 0x65, 0xAD, 0x85,
                0xAD
            ]));
        assert!(
            !bytes
                .windows(2)
                .any(|window| window == [0x85, 0xAE] || window == [0x85, 0xAF])
        );
    }

    #[test]
    fn mir6502_emission_materializes_indexed_address_directly() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::MaterializeIndexedAddress {
                        consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                            lo: MirFixedZpSlot(0xAC),
                        }),
                        base: MirValue::ConstU16(0x40F0),
                        index: MirValue::ConstU8(0x10),
                        scale: 2,
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit indexed address materialization");
        let bytes = emitter.finish().expect("finish emitter");

        assert!(bytes.windows(18).any(|window| window
            == [
                0xA9, 0x10, 0x0A, 0x08, 0x18, 0x69, 0xF0, 0x85, 0xAC, 0xA9, 0x00, 0x2A, 0x28, 0x69,
                0x40, 0x85, 0xAD, 0x60
            ]));
    }

    #[test]
    fn mir6502_emission_keeps_scaled_word_index_in_y() {
        let consumer = MirAddressConsumer::ScaledIndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xAC),
        });
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::MaterializeIndexedAddress {
                            consumer,
                            base: MirValue::ConstU16(0x40F0),
                            index: MirValue::ConstU8(0x82),
                            scale: 2,
                        },
                        MirOp::LoadIndirect {
                            consumer,
                            dst: MirDef::Reg(MirReg::A),
                            offset: 0,
                        },
                        MirOp::LoadIndirect {
                            consumer,
                            dst: MirDef::Reg(MirReg::A),
                            offset: 1,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit scaled-Y indexed address");
        let bytes = emitter.finish().expect("finish emitter");

        assert_eq!(
            bytes,
            vec![
                0xA9, 0x82, 0x0A, 0xA8, 0xA9, 0xF0, 0x85, 0xAC, 0xA9, 0x40, 0x69, 0x00, 0x85, 0xAD,
                0xB1, 0xAC, 0xC8, 0xB1, 0xAC, 0x60,
            ]
        );
    }

    #[test]
    fn mir6502_emission_shares_scaled_y_offset_between_pointer_pairs() {
        let source = MirAddressConsumer::ScaledIndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xAC),
        });
        let destination = MirAddressConsumer::ScaledIndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xAE),
        });
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::MaterializeIndexedAddress {
                            consumer: destination,
                            base: MirValue::ConstU16(0x5000),
                            index: MirValue::ConstU8(3),
                            scale: 2,
                        },
                        MirOp::MaterializeIndexedAddress {
                            consumer: source,
                            base: MirValue::ConstU16(0x4000),
                            index: MirValue::ConstU8(3),
                            scale: 2,
                        },
                        MirOp::LoadIndirect {
                            consumer: source,
                            dst: MirDef::Reg(MirReg::A),
                            offset: 0,
                        },
                        MirOp::StoreIndirect {
                            consumer: destination,
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            offset: 0,
                        },
                        MirOp::LoadIndirect {
                            consumer: source,
                            dst: MirDef::Reg(MirReg::A),
                            offset: 1,
                        },
                        MirOp::StoreIndirect {
                            consumer: destination,
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            offset: 1,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit shared scaled-Y word copy");
        let bytes = emitter.finish().expect("finish emitter");

        assert_eq!(bytes.iter().filter(|byte| **byte == 0xC8).count(), 1);
        assert!(
            bytes
                .windows(9)
                .any(|window| { window == [0xB1, 0xAC, 0x91, 0xAE, 0xC8, 0xB1, 0xAC, 0x91, 0xAE] })
        );
    }

    #[test]
    fn mir6502_emission_emits_direct_routine_calls() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![
                MirRoutine {
                    id: RoutineId(0),
                    name: "Touch".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
                MirRoutine {
                    id: RoutineId(1),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::Call {
                            target: MirCallTarget::Routine(RoutineId(0)),
                            abi: MirCallAbi {
                                params: Vec::new(),
                                result: None,
                                clobbers: MirRegisterSet::default(),
                                preserves: MirRegisterSet::default(),
                            },
                            args: Vec::new(),
                            result: None,
                            effects: MirEffects::default(),
                        }],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
            ],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit direct call MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x60, 0x4C, 0x00, 0x30]);
    }

    #[test]
    fn mir6502_emission_emits_runtime_helper_calls() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::RuntimeHelper {
                        helper: MirRuntimeHelper::Mul,
                        args: Vec::new(),
                        result: None,
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: vec![MirRuntimeHelperDecl {
                helper: MirRuntimeHelper::Mul,
                target: MirRuntimeHelperTarget::KnownAbsolute(0xA000),
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: None,
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                effects: MirEffects::default(),
            }],
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit runtime helper MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x4C, 0x00, 0xA0]);
    }

    #[test]
    fn mir6502_emission_keeps_non_tail_direct_calls_as_jsr() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![
                MirRoutine {
                    id: RoutineId(0),
                    name: "Callee".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
                MirRoutine {
                    id: RoutineId(1),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Call {
                                target: MirCallTarget::Routine(RoutineId(0)),
                                abi: MirCallAbi {
                                    params: Vec::new(),
                                    result: None,
                                    clobbers: MirRegisterSet::default(),
                                    preserves: MirRegisterSet::default(),
                                },
                                args: Vec::new(),
                                result: None,
                                effects: MirEffects::default(),
                            },
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 1,
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
            ],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit non-tail call MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x60, 0x20, 0x00, 0x30, 0xA9, 0x01, 0x60]);
    }

    #[test]
    fn mir6502_emission_accepts_byte_sized_word_constants_for_index_register_moves() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Move {
                            dst: MirDef::Reg(MirReg::X),
                            src: MirValue::ConstU16(0x009B),
                            width: MirWidth::Byte,
                        },
                        MirOp::Move {
                            dst: MirDef::Reg(MirReg::Y),
                            src: MirValue::ConstU16(0x007D),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit byte-sized word constants");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xA2, 0x9B, 0xA0, 0x7D, 0x60]);
    }

    #[test]
    fn mir6502_emission_emits_absolute_builtin_calls() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::Call {
                        target: MirCallTarget::Builtin {
                            name: "Put".to_string(),
                            address: Some(0xE456),
                        },
                        abi: MirCallAbi {
                            params: Vec::new(),
                            result: None,
                            clobbers: MirRegisterSet::default(),
                            preserves: MirRegisterSet::default(),
                        },
                        args: Vec::new(),
                        result: None,
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit builtin call MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x4C, 0x56, 0xE4]);
    }

    #[test]
    fn mir6502_user_system_address_calls_emit_jsr_absolute() {
        let output =
            generate_mir6502_source("BYTE x PROC CIO=$E456() PROC Main() CIO() x=1 RETURN");

        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes == [crate::codegen::opcode::JSR_ABS, 0x56, 0xE4])
        );
    }

    #[test]
    fn mir6502_emission_reports_explicit_unsupported_builtin_targets() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::Call {
                        target: MirCallTarget::Builtin {
                            name: "PrintH".to_string(),
                            address: None,
                        },
                        abi: MirCallAbi {
                            params: Vec::new(),
                            result: None,
                            clobbers: MirRegisterSet::default(),
                            preserves: MirRegisterSet::default(),
                        },
                        args: Vec::new(),
                        result: None,
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        let diagnostics = emit_program(&mir, &mut emitter).expect_err("unsupported builtin");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("builtin call target `PrintH` is unsupported by MIR6502")
        }));
    }

    #[test]
    fn mir6502_emission_emits_structured_machine_block_bytes() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::MachineBlock {
                        id: MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects {
                    opaque: true,
                    ..MirEffects::default()
                },
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![MirMachineItem::Byte(0xEA), MirMachineItem::Word(0x1234)],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        let summary = emit_program(&mir, &mut emitter).expect("emit machine block MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xEA, 0x34, 0x12, 0x60]);
        assert_eq!(summary.machine_blocks[0].address, 0x3000);
        assert_eq!(summary.machine_blocks[0].routine.as_deref(), Some("Main"));
        assert!(summary.machine_blocks[0].summary.contains("2 structured"));
        assert_eq!(summary.routine_effects[0].routine, "Main");
        assert_eq!(summary.routine_effects[0].summary, "opaque");
        assert!(
            summary
                .source_ranges
                .iter()
                .any(|range| range.kind == crate::codegen::CodegenSourceRangeKind::MachineBlock)
        );
    }

    #[test]
    fn mir6502_emission_relocates_machine_block_label_offsets() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![
                MirRoutine {
                    id: RoutineId(0),
                    name: "Target".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
                MirRoutine {
                    id: RoutineId(1),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::MachineBlock {
                            id: MirMachineBlockId(0),
                            effects: MirEffects::default(),
                        }],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
            ],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![
                    MirMachineItem::AddressExpr {
                        selector: None,
                        explicit_address: false,
                        atom: MirMachineAtom::Name("Target".to_string()),
                        offset: 1,
                        text: "Target+1".to_string(),
                    },
                    MirMachineItem::AddressExpr {
                        selector: Some(MirMachineByteSelector::Low),
                        explicit_address: false,
                        atom: MirMachineAtom::Name("Target".to_string()),
                        offset: 2,
                        text: "<Target+2".to_string(),
                    },
                    MirMachineItem::AddressExpr {
                        selector: Some(MirMachineByteSelector::High),
                        explicit_address: false,
                        atom: MirMachineAtom::Name("Target".to_string()),
                        offset: 2,
                        text: ">Target+2".to_string(),
                    },
                ],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit machine block label offsets");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x60, 0x01, 0x30, 0x02, 0x30, 0x60]);
    }

    #[test]
    fn mir6502_emission_emits_machine_block_literal_bytes() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::MachineBlock {
                        id: MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects {
                    opaque: true,
                    ..MirEffects::default()
                },
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![
                    MirMachineItem::StringLiteral("AB".to_string()),
                    MirMachineItem::CharLiteral('C'),
                ],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit machine block literals");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0x02, 0x41, 0x42, 0x00, 0x30, 0x43, 0x9A, 0x60]);
    }

    #[test]
    fn mir6502_constant_negation_fold_is_deferred_until_flag_liveness() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::Unary {
                        op: MirUnaryOp::Neg,
                        dst: MirDef::Reg(MirReg::A),
                        src: MirValue::ConstU8(1),
                        width: MirWidth::Byte,
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit byte unary negation");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xA9, 0x01, 0x49, 0xFF, 0x18, 0x69, 0x01, 0x60]);
    }

    #[test]
    fn mir6502_emission_emits_byte_constant_shifts() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Binary {
                            op: MirBinaryOp::Rsh,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::ConstU8(4),
                            right: MirValue::ConstU8(1),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Ignore,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Lsh,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::ConstU8(3),
                            right: MirValue::ConstU8(2),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Ignore,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit byte constant shifts");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xA9, 0x04, 0x4A, 0xA9, 0x03, 0x0A, 0x0A, 0x60]);
    }

    #[test]
    fn mir6502_emission_relaxes_conditional_branches() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Branch {
                            cond: MirCond::FlagTest(MirFlagTest::ZSet),
                            then_edge: MirEdge::plain(MirBlockId(1)),
                            else_edge: MirEdge::plain(MirBlockId(2)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "bb1".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "bb2".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit relaxed branch");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(bytes, vec![0xD0, 0x01, 0x60, 0x60]);
    }

    #[test]
    fn mir6502_emission_relaxes_any_flag_forward_branches() {
        fn branch_program(then_is_next: bool) -> Vec<u8> {
            let then_block = MirBlock {
                id: MirBlockId(1),
                label: "then".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            };
            let else_block = MirBlock {
                id: MirBlockId(2),
                label: "else".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            };
            let mut blocks = vec![MirBlock {
                id: MirBlockId(0),
                label: "branch".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Branch {
                    cond: MirCond::AnyFlagTest([MirFlagTest::CClear, MirFlagTest::ZSet]),
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            }];
            if then_is_next {
                blocks.extend([then_block, else_block]);
            } else {
                blocks.extend([else_block, then_block]);
            }
            let mir = MirProgram {
                statics: Vec::new(),
                globals: Vec::new(),
                routines: vec![MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks,
                    effects: MirEffects::default(),
                }],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            };

            let mut emitter =
                crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
            emit_program(&mir, &mut emitter).expect("emit any-flag forward branch");
            emitter.finish().expect("finish emitter")
        }

        assert_eq!(
            branch_program(false),
            vec![0x90, 0x03, 0xF0, 0x01, 0x60, 0x60]
        );
        assert_eq!(
            branch_program(true),
            vec![0x90, 0x02, 0xD0, 0x01, 0x60, 0x60]
        );
    }

    #[test]
    fn mir6502_emission_keeps_far_forward_branch_over_jmp_shape() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "branch".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Branch {
                            cond: MirCond::FlagTest(MirFlagTest::ZSet),
                            then_edge: MirEdge::plain(MirBlockId(2)),
                            else_edge: MirEdge::plain(MirBlockId(1)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "padding".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::MachineBlock {
                            id: MirMachineBlockId(0),
                            effects: MirEffects::default(),
                        }],
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "then".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![MirMachineItem::Byte(0xEA); 127],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit far forward branch MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(&bytes[..5], &[0xD0, 0x03, 0x4C, 0x85, 0x30]);
    }

    #[test]
    fn mir6502_emission_relaxes_self_enabling_forward_branch() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "branch".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Branch {
                            cond: MirCond::FlagTest(MirFlagTest::ZSet),
                            then_edge: MirEdge::plain(MirBlockId(2)),
                            else_edge: MirEdge::plain(MirBlockId(1)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "padding".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::MachineBlock {
                            id: MirMachineBlockId(0),
                            effects: MirEffects::default(),
                        }],
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "then".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![MirMachineItem::Byte(0xEA); 125],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit self-enabling forward branch MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(&bytes[..2], &[0xF0, 0x7E]);
    }

    #[test]
    fn mir6502_emission_keeps_far_branch_over_jmp_shape() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(1),
                        label: "then".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "else".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(3),
                        label: "padding".to_string(),
                        params: Vec::new(),
                        ops: vec![MirOp::MachineBlock {
                            id: MirMachineBlockId(0),
                            effects: MirEffects::default(),
                        }],
                        terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(4))),
                    },
                    MirBlock {
                        id: MirBlockId(4),
                        label: "branch".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Branch {
                            cond: MirCond::FlagTest(MirFlagTest::ZSet),
                            then_edge: MirEdge::plain(MirBlockId(1)),
                            else_edge: MirEdge::plain(MirBlockId(2)),
                        },
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![MirMachineItem::Byte(0xEA); 140],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit far branch MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert!(bytes.ends_with(&[0xD0, 0x03, 0x4C, 0x00, 0x30, 0x4C, 0x01, 0x30]));
    }

    #[test]
    fn mir6502_emission_applies_machine_block_byte_stream_width_rules() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: vec![MirGlobal {
                id: SymbolId(0),
                name: "ZP_LABEL".to_string(),
                kind: "byte".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0x00E4),
                init: None,
            }],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::MachineBlock {
                        id: MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![
                    MirMachineItem::Byte(0x34),
                    MirMachineItem::Word(0x0348),
                    MirMachineItem::Word(0xF2F8),
                    MirMachineItem::Word(0xFFFF),
                    MirMachineItem::AddressExpr {
                        selector: Some(MirMachineByteSelector::Low),
                        explicit_address: false,
                        atom: MirMachineAtom::Number(0x0348),
                        offset: 0,
                        text: "<$0348".to_string(),
                    },
                    MirMachineItem::AddressExpr {
                        selector: Some(MirMachineByteSelector::High),
                        explicit_address: false,
                        atom: MirMachineAtom::Number(0x0348),
                        offset: 0,
                        text: ">$0348".to_string(),
                    },
                    MirMachineItem::Name("ZP_LABEL".to_string()),
                    MirMachineItem::AddressExpr {
                        selector: Some(MirMachineByteSelector::High),
                        explicit_address: true,
                        atom: MirMachineAtom::Name("ZP_LABEL".to_string()),
                        offset: 1,
                        text: ">@ZP_LABEL+1".to_string(),
                    },
                ],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        emit_program(&mir, &mut emitter).expect("emit machine block MIR");
        let bytes = emitter.finish().expect("finish emitter");
        assert_eq!(
            bytes,
            vec![
                0x34, 0x48, 0x03, 0xF8, 0xF2, 0xFF, 0xFF, 0x48, 0x03, 0xE4, 0x00, 0x60,
            ]
        );
    }

    #[test]
    fn source_machine_block_caret_symbol_emits_compile_time_address() {
        let output = generate_mir6502_source(
            "BYTE ARRAY screen=$8010,text=$9E80 PROC DL15=*() [78 screen^ 66 text^ 65 DL15]",
        );
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

        assert!(bytes_contain(&output.bytes, &expected));
    }

    #[test]
    fn source_machine_block_caret_symbol_accepts_named_offset() {
        let output = generate_mir6502_source(
            "DEFINE OFF=\"2\" BYTE ARRAY screen=$8010 PROC Main() [screen^+OFF >screen^-OFF]",
        );

        assert!(bytes_contain(&output.bytes, &[0x12, 0x80, 0x80]));
    }

    #[test]
    fn source_machine_block_resolves_runtime_symbols() {
        let output = generate_mir6502_source(
            "PROC Rom=$A326()[] PROC Main() [$20Rom $ADRom+1 $20Break $A5device $A9 EOL $AD EOF] RETURN",
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0x20, 0x26, 0xA3, 0xAD, 0x27, 0xA3, 0x20, 0xDA, 0xA7, 0xA5, 0xB7, 0xA9, 0x9B, 0xAD,
                0xC0, 0x05,
            ],
        ));
    }

    #[test]
    fn source_machine_block_resolves_current_address_expression() {
        let output = generate_mir6502_source_with_origin("PROC Main() [$39*+5] RETURN", 0x3000);

        assert!(bytes_contain(&output.bytes, &[0x39, 0x06, 0x30]));
    }

    #[test]
    fn mir6502_emission_rejects_unresolved_machine_block_names() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::MachineBlock {
                        id: MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![MirMachineItem::Name("TARGET".to_string())],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        let diagnostics = emit_program(&mir, &mut emitter).expect_err("machine name is unresolved");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("machine block reference `TARGET` is not emit-ready")
        }));
    }

    #[test]
    fn mir6502_emission_rejects_unresolved_machine_block_address_exprs() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::MachineBlock {
                        id: MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    }],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: vec![MirMachineBlock {
                id: MirMachineBlockId(0),
                items: vec![MirMachineItem::AddressExpr {
                    selector: None,
                    explicit_address: true,
                    atom: MirMachineAtom::Name("MISSING".to_string()),
                    offset: 0,
                    text: "@MISSING".to_string(),
                }],
            }],
            runtime_helpers: Vec::new(),
        };

        let mut emitter = crate::codegen::native_emitter::NativeTrackedEmitter::with_origin(0x3000);
        let diagnostics =
            emit_program(&mir, &mut emitter).expect_err("machine address expr is unresolved");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(
                "machine block item `@MISSING` cannot be resolved to a compile-time address",
            )
        }));
    }

    #[test]
    fn materialization_allocates_virtual_zero_page_slots() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0), MirZpSlot(1)],
                    fixed_zero_page: vec![MirFixedZpSlot(0xE0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let allocations = &materialized.routines[0].frame.zero_page_allocations;
        assert_eq!(allocations.len(), 2);
        assert_eq!(allocations[0].slot, MirZpSlot(0));
        assert_eq!(allocations[0].start, MirFixedZpSlot(0xE1));
        assert_eq!(allocations[1].slot, MirZpSlot(1));
        assert_eq!(allocations[1].start, MirFixedZpSlot(0xE2));
        verify_program(&materialized, MirPhase::PreEmission).unwrap();
    }

    #[test]
    fn pre_emission_rejects_missing_zero_page_allocation() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let diagnostics = verify_program(&mir, MirPhase::PreEmission).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("is not allocated"))
        );
    }

    #[test]
    fn verifier_rejects_overlapping_zero_page_allocations() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0), MirZpSlot(1)],
                    zero_page_allocations: vec![
                        MirZpAllocation {
                            slot: MirZpSlot(0),
                            start: MirFixedZpSlot(0xE0),
                            size: 2,
                        },
                        MirZpAllocation {
                            slot: MirZpSlot(1),
                            start: MirFixedZpSlot(0xE1),
                            size: 1,
                        },
                    ],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let diagnostics = verify_program(&mir, MirPhase::PreEmission).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("overlaps byte $E1"))
        );
    }

    #[test]
    fn materialization_uses_output_origin_for_local_address_values() {
        let output = generate_mir6502_source_with_origin(
            "CARD r=$86 PROC Main() CHAR ARRAY fnam(4) r=fnam RETURN",
            0x2C00,
        );
        let fnam = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "fnam")
            .expect("fnam storage symbol");

        assert_eq!(fnam.address, 0x2C00);
        assert!(bytes_contain(&output.bytes, &[0xA9, 0x2C, 0x85, 0x87]));
        assert!(!bytes_contain(&output.bytes, &[0xA9, 0x2C, 0x85, 0xE1]));
        assert!(!bytes_contain(&output.bytes, &[0xA9, 0x30, 0x85, 0xE1]));
    }

    #[test]
    fn materialization_fuses_loaded_byte_word_compound_store() {
        let output = generate_mir6502_source_with_origin(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen BYTE dx=$EF, out SET $491=$3000 SET $E=$3000 PROC Main() screen==+dx screen^=out RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x65, 0xEF, 0x85, 0xE6, 0xA5, 0xE7, 0x69, 0x00, 0x85, 0xE7
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0x85, 0xED]));
    }

    #[test]
    fn optimized_materialization_uses_word_inc_for_plus_one() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen SET $491=$3000 SET $E=$3000 PROC Main() screen==+1 screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain(
            &output.bytes,
            &[0xE6, 0xE6, 0xD0, 0x02, 0xE6, 0xE7]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x69, 0x01, 0x85, 0xE6, 0xA5, 0xE7, 0x69, 0x00, 0x85, 0xE7
            ]
        ));
    }

    #[test]
    fn optimized_materialization_uses_carry_inc_for_byte_word_add() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen BYTE h=$EF SET $491=$3000 SET $E=$3000 PROC Main() screen==+h screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x65, 0xEF, 0x85, 0xE6, 0x90, 0x02, 0xE6, 0xE7
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x65, 0xEF, 0x85, 0xE6, 0xA5, 0xE7, 0x69, 0x00, 0x85, 0xE7
            ]
        ));
    }

    #[test]
    fn optimized_materialization_uses_carry_inc_for_const_byte_word_add() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen SET $491=$3000 SET $E=$3000 PROC Main() screen==+40 screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x69, 0x28, 0x85, 0xE6, 0x90, 0x02, 0xE6, 0xE7
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x69, 0x28, 0x85, 0xE6, 0xA5, 0xE7, 0x69, 0x00, 0x85, 0xE7
            ]
        ));
    }

    #[test]
    fn optimized_materialization_uses_carry_inc_for_call_byte_word_add() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen BYTE a=$EF SET $491=$3000 SET $E=$3000 BYTE FUNC F(BYTE x) RETURN(x+3) PROC Main() screen==+F(a) screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain_jsr_to_routine(&output, "F"));
        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x18, 0x65, 0xA0, 0x85, 0xE6, 0x90, 0x02, 0xE6, 0xE7
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[0xA5, 0xE6, 0x85, 0xE0, 0xA5, 0xE7, 0x85, 0xE2, 0xA5, 0xE0]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xA5, 0xA0, 0x85, 0xE0]));
    }

    #[test]
    fn optimized_materialization_uses_borrow_dec_for_call_byte_word_sub() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen BYTE a=$EF SET $491=$3000 SET $E=$3000 BYTE FUNC F(BYTE x) RETURN(x+3) PROC Main() screen==-F(a) screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain_jsr_to_routine(&output, "F"));
        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x38, 0xE5, 0xA0, 0x85, 0xE6, 0xB0, 0x02, 0xC6, 0xE7
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[0xA5, 0xE6, 0x85, 0xE0, 0xA5, 0xE7, 0x85, 0xE2, 0xA5, 0xE0]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xA5, 0xA0, 0x85, 0xE0]));
    }

    #[test]
    fn optimized_materialization_uses_borrow_dec_for_byte_word_sub() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen BYTE h=$EF SET $491=$3000 SET $E=$3000 PROC Main() screen==-h screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x38, 0xE5, 0xEF, 0x85, 0xE6, 0xB0, 0x02, 0xC6, 0xE7
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x38, 0xE5, 0xEF, 0x85, 0xE6, 0xA5, 0xE7, 0xE9, 0x00, 0x85, 0xE7
            ]
        ));
    }

    #[test]
    fn optimized_materialization_uses_borrow_dec_for_const_byte_word_sub() {
        let output = generate_mir6502_source_with_config(
            "SET $491=$E6 SET $492=$00 SET $E=$E6 SET $F=$00 BYTE POINTER screen SET $491=$3000 SET $E=$3000 PROC Main() screen==-40 screen^=$11 RETURN",
            0x3000,
            &Mir6502Config::optimized(),
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA5, 0xE6, 0x38, 0xE9, 0x28, 0x85, 0xE6, 0xB0, 0x02, 0xC6, 0xE7
            ]
        ));
    }

    #[test]
    fn materialization_uses_indirect_source_for_deref_byte_add() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER s,t PROC Main() s=$4000 t=$4100 s^==+t^ RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xAD, 0x02, 0x30, 0x85,
                0xAE, 0xAD, 0x03, 0x30, 0x85, 0xAF, 0xA0, 0x00, 0xB1, 0xAC, 0x18, 0x71, 0xAE, 0x91,
                0xAC
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xB1, 0xAC, 0x85, 0xE2]));
    }

    #[test]
    fn materialization_uses_indirect_source_for_deref_byte_sub() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER s,t PROC Main() s=$4000 t=$4100 s^==-t^ RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xAD, 0x02, 0x30, 0x85,
                0xAE, 0xAD, 0x03, 0x30, 0x85, 0xAF, 0xA0, 0x00, 0xB1, 0xAC, 0x38, 0xF1, 0xAE, 0x91,
                0xAC
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xB1, 0xAC, 0x85, 0xE2]));
    }

    #[test]
    fn materialization_uses_direct_source_for_deref_byte_add() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p BYTE x=$5C PROC Main() p=$4000 x=3 p^==+x RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA0, 0x00, 0xB1, 0xAC,
                0x18, 0x65, 0x5C, 0xA0, 0x00, 0x91, 0xAC
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xA5, 0x5C, 0x85, 0xE2]));
    }

    #[test]
    fn materialization_uses_direct_source_for_deref_byte_sub() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p BYTE x=$5C PROC Main() p=$4000 x=3 p^==-x RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA0, 0x00, 0xB1, 0xAC,
                0x38, 0xE5, 0x5C, 0xA0, 0x00, 0x91, 0xAC
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xA5, 0x5C, 0x85, 0xE2]));
    }

    #[test]
    fn materialization_keeps_deref_byte_rhs_when_it_aliases_pointer_scratch() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p BYTE x=$AC PROC Main() p=$4000 x=3 p^==+x RETURN",
            0x3000,
        );

        assert!(bytes_contain(&output.bytes, &[0xA5, 0xAC, 0x85, 0xE2]));
        assert!(!bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA0, 0x00, 0xB1, 0xAC,
                0x18, 0x65, 0xAC, 0x91, 0xAC
            ]
        ));
    }

    #[test]
    fn materialization_uses_const_source_for_deref_byte_add() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p PROC Main() p=$4000 p^==+3 RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA0, 0x00, 0xB1, 0xAC,
                0x18, 0x69, 0x03, 0xA0, 0x00, 0x91, 0xAC
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[0xAD, 0x00, 0x30, 0x85, 0xE0, 0xAD, 0x01, 0x30, 0x85, 0xE1]
        ));
    }

    #[test]
    fn materialization_uses_const_source_for_deref_byte_sub() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p PROC Main() p=$4000 p^==-3 RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA0, 0x00, 0xB1, 0xAC,
                0x38, 0xE9, 0x03, 0xA0, 0x00, 0x91, 0xAC
            ]
        ));
        assert!(!bytes_contain(
            &output.bytes,
            &[0xAD, 0x00, 0x30, 0x85, 0xE0, 0xAD, 0x01, 0x30, 0x85, 0xE1]
        ));
    }

    #[test]
    fn materialization_uses_direct_source_for_deref_byte_store() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p BYTE x=$5C PROC Main() p=$4000 p^=x RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA5, 0x5C, 0xA0, 0x00,
                0x91, 0xAC
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0xA5, 0x5C, 0x85, 0xE0]));
    }

    #[test]
    fn materialization_reloads_recent_store_for_deref_byte_store() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p BYTE x=$5C PROC Main() p=$4000 x=3 p^=x RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[
                0xA9, 0x03, 0x85, 0x5C, 0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD,
                0xA5, 0x5C, 0xA0, 0x00, 0x91, 0xAC
            ]
        ));
        assert!(!bytes_contain(&output.bytes, &[0x85, 0x5C, 0x85, 0xE0]));
    }

    #[test]
    fn materialization_keeps_deref_byte_store_rhs_when_it_aliases_pointer_scratch() {
        let output = generate_unoptimized_nir_mir6502_source_with_origin(
            "SET $491=$3000 SET $E=$3000 BYTE POINTER p BYTE x=$AC PROC Main() p=$4000 p^=x RETURN",
            0x3000,
        );

        assert!(bytes_contain(&output.bytes, &[0xA5, 0xAC, 0x85, 0xE0]));
        assert!(!bytes_contain(
            &output.bytes,
            &[
                0xAD, 0x00, 0x30, 0x85, 0xAC, 0xAD, 0x01, 0x30, 0x85, 0xAD, 0xA5, 0xAC, 0xA0, 0x00,
                0x91, 0xAC
            ]
        ));
    }

    #[test]
    fn materialization_removes_zero_page_self_store() {
        let output =
            generate_mir6502_source_with_origin("BYTE x=$5C PROC Main() x=x RETURN", 0x3000);

        assert!(bytes_contain(&output.bytes, &[0xA5, 0x5C, 0x60]));
        assert!(!bytes_contain(&output.bytes, &[0xA5, 0x5C, 0x85, 0x5C]));
    }

    #[test]
    fn materialization_keeps_arbitrary_absolute_self_store() {
        let output =
            generate_mir6502_source_with_origin("BYTE x=$0600 PROC Main() x=x RETURN", 0x3000);

        assert!(bytes_contain(
            &output.bytes,
            &[0xAD, 0x00, 0x06, 0x8D, 0x00, 0x06]
        ));
    }

    #[test]
    fn materialization_removes_adjacent_zero_page_store_reload() {
        let output = generate_mir6502_source_with_origin(
            "BYTE x=$5C,y=$5D PROC Main() x=7 y=x RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[0xA9, 0x07, 0x85, 0x5C, 0x85, 0x5D]
        ));
        assert!(!bytes_contain(&output.bytes, &[0x85, 0x5C, 0xA5, 0x5C]));
    }

    #[test]
    fn materialization_keeps_arbitrary_absolute_store_reload() {
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x5D)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 7,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Absolute(0x0600)),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(MirMem::Absolute(0x0600)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x5D))),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        assert!(materialized.routines[0].blocks[0].ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(MirMem::Absolute(0x0600)),
                    width: MirWidth::Byte,
                }
            )
        }));
    }

    #[test]
    fn materialization_uses_direct_rhs_for_staged_byte_compare() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let rhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let lhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5D));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x5C), MirFixedZpSlot(0x5D)],
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(rhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(scratch.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(lhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Compare {
                            dst: MirCondDest::Flags,
                            op: MirCompareOp::Lt,
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::PointerCell(scratch.clone()),
                            width: MirWidth::Byte,
                            signed: false,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    ..
                } if *mem == scratch
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Compare {
                    right: MirValue::PointerCell(mem),
                    ..
                } if *mem == rhs
            )
        }));
    }

    #[test]
    fn materialization_keeps_live_staged_byte_compare_rhs() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let rhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let lhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5D));
        let sink = MirMem::FixedZeroPage(MirFixedZpSlot(0x5E));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![
                        MirFixedZpSlot(0x5C),
                        MirFixedZpSlot(0x5D),
                        MirFixedZpSlot(0x5E),
                    ],
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(rhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(scratch.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(lhs),
                            width: MirWidth::Byte,
                        },
                        MirOp::Compare {
                            dst: MirCondDest::Flags,
                            op: MirCompareOp::Lt,
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::PointerCell(scratch.clone()),
                            width: MirWidth::Byte,
                            signed: false,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(scratch.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(sink.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    ..
                } if *mem == scratch
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Compare {
                    right: MirValue::PointerCell(mem),
                    ..
                } if *mem == rhs
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == rhs
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                } if *mem == sink
            )
        }));
    }

    #[test]
    fn materialization_removes_fully_staged_byte_compare_temps() {
        let lhs_slot = MirMem::ZeroPage(MirZpSlot(0));
        let rhs_slot = MirMem::ZeroPage(MirZpSlot(1));
        let lhs = MirMem::FixedZeroPage(MirFixedZpSlot(0xE0));
        let rhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0xE0), MirFixedZpSlot(0x5C)],
                    virtual_zero_page: vec![MirZpSlot(0), MirZpSlot(1)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(lhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(lhs_slot.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(rhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(rhs_slot.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(lhs_slot.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Compare {
                            dst: MirCondDest::Flags,
                            op: MirCompareOp::Lt,
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::PointerCell(rhs_slot.clone()),
                            width: MirWidth::Byte,
                            signed: false,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    ..
                } if *mem == lhs_slot || *mem == rhs_slot
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    src: MirAddr::Direct(mem),
                    ..
                } if *mem == lhs
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Compare {
                    right: MirValue::PointerCell(mem),
                    ..
                } if *mem == rhs
            )
        }));
    }

    #[test]
    fn materialization_uses_direct_rhs_for_staged_byte_binary() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let rhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let lhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5D));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x5C), MirFixedZpSlot(0x5D)],
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(rhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(scratch.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(lhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Add,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::PointerCell(scratch.clone()),
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::Clear),
                            carry_out: MirCarryOut::Ignore,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    ..
                } if *mem == scratch
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Binary {
                    right: MirValue::PointerCell(mem),
                    ..
                } if *mem == rhs
            )
        }));
    }

    #[test]
    fn materialization_keeps_live_staged_byte_binary_rhs() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let rhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let lhs = MirMem::FixedZeroPage(MirFixedZpSlot(0x5D));
        let sink = MirMem::FixedZeroPage(MirFixedZpSlot(0x5E));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![
                        MirFixedZpSlot(0x5C),
                        MirFixedZpSlot(0x5D),
                        MirFixedZpSlot(0x5E),
                    ],
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(rhs.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(scratch.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(lhs),
                            width: MirWidth::Byte,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Add,
                            dst: MirDef::Reg(MirReg::A),
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::PointerCell(scratch.clone()),
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::Clear),
                            carry_out: MirCarryOut::Ignore,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(scratch.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(sink.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    ..
                } if *mem == scratch
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Binary {
                    right: MirValue::PointerCell(mem),
                    ..
                } if *mem == rhs
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == rhs
            )
        }));
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                } if *mem == sink
            )
        }));
    }

    #[test]
    fn materialization_uses_ssa_lite_source_for_staged_x_load() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let src = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x5C)],
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(src.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(scratch.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::X),
                            src: MirAddr::Direct(scratch.clone()),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == src
            )
        }));
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == scratch
            )
        }));
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    ..
                } if *mem == scratch
            )
        }));
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == src
            )
        }));
    }

    #[test]
    fn materialization_keeps_non_private_staged_x_load() {
        let local = MirMem::Local {
            id: LocalId(0),
            offset: 0,
        };
        let src = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x5C)],
                    locals: vec![MirStorageSlot {
                        id: MirStorageId(0),
                        name: Some("local".to_string()),
                        storage: MirStorageClass::Scalar,
                        base: MirStorageBase::Local(LocalId(0)),
                        offset: 0,
                        width: MirWidth::Byte,
                        mutable: true,
                        init: None,
                    }],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(src),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(local.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::X),
                            src: MirAddr::Direct(local.clone()),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        assert!(materialized.routines[0].blocks[0].ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == local
            )
        }));
    }

    #[test]
    fn materialization_uses_ssa_lite_const_for_staged_y_load() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    virtual_zero_page: vec![MirZpSlot(0)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 7,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(scratch.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::Y),
                            src: MirAddr::Direct(scratch.clone()),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::Y),
                    value: 7,
                    width: MirWidth::Byte,
                }
            )
        }));
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::Y),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == scratch
            )
        }));
        assert!(!ops.iter().any(|op| {
            matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                }
            )
        }));
    }

    #[test]
    fn materialization_keeps_a_load_live_across_y_load() {
        let src = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
        let dst = MirMem::FixedZeroPage(MirFixedZpSlot(0x5D));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    fixed_zero_page: vec![MirFixedZpSlot(0x5C), MirFixedZpSlot(0x5D)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(src.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::Y),
                            value: 0,
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(dst),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        assert!(materialized.routines[0].blocks[0].ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == src
            )
        }));
    }

    #[test]
    fn materialization_does_not_forward_ssa_lite_const_across_call() {
        let scratch = MirMem::ZeroPage(MirZpSlot(0));
        let mir = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![
                MirRoutine {
                    id: RoutineId(0),
                    name: "Main".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame {
                        virtual_zero_page: vec![MirZpSlot(0)],
                        ..MirFrame::default()
                    },
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::LoadImm {
                                dst: MirDef::Reg(MirReg::A),
                                value: 7,
                                width: MirWidth::Byte,
                            },
                            MirOp::Store {
                                dst: MirAddr::Direct(scratch.clone()),
                                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                                width: MirWidth::Byte,
                            },
                            MirOp::Call {
                                target: MirCallTarget::Routine(RoutineId(1)),
                                abi: MirCallAbi {
                                    params: Vec::new(),
                                    result: None,
                                    clobbers: MirRegisterSet {
                                        a: true,
                                        x: true,
                                        y: true,
                                        flags: true,
                                        ..MirRegisterSet::default()
                                    },
                                    preserves: MirRegisterSet::default(),
                                },
                                args: Vec::new(),
                                result: None,
                                effects: MirEffects::default(),
                            },
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::Y),
                                src: MirAddr::Direct(scratch.clone()),
                                width: MirWidth::Byte,
                            },
                        ],
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
                MirRoutine {
                    id: RoutineId(1),
                    name: "Callee".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: vec![MirBlock {
                        id: MirBlockId(0),
                        label: "bb0".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    }],
                    effects: MirEffects::default(),
                },
            ],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized = materialize_program(mir, &Mir6502Config::default()).unwrap();
        let ops = &materialized.routines[0].blocks[0].ops;
        assert!(ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::Y),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == scratch
            )
        }));
    }

    #[test]
    fn materialization_fuses_two_loaded_byte_compare() {
        let output = generate_mir6502_source_with_origin(
            "BYTE i=$E0, n=$5C PROC Main() i=2 n=5 WHILE i<n DO i==+1 OD RETURN",
            0x3000,
        );

        assert!(bytes_contain(
            &output.bytes,
            &[0xA5, 0xE0, 0xC5, 0x5C, 0xB0]
        ));
    }

    #[test]
    fn uninitialized_local_arrays_are_deferred_after_mir_code() {
        let output = generate_mir6502_source_with_origin(
            "BYTE sink PROC LocalOnly() BYTE ARRAY small(4), big(257), initialized(4)=[1 2 3 4] sink=small(0)+big(0)+initialized(0) RETURN PROC Main() LocalOnly() RETURN",
            0x3000,
        );

        let small = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "small")
            .expect("small storage symbol");
        let big = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "big")
            .expect("big storage symbol");
        let initialized = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "initialized")
            .expect("initialized storage symbol");
        let emitted_end = output.origin.wrapping_add(output.bytes.len() as u16);

        assert_eq!(
            output.skipped_ranges,
            vec![
                crate::codegen::SkippedRange {
                    start: small.address,
                    len: 4
                },
                crate::codegen::SkippedRange {
                    start: big.address,
                    len: 257
                }
            ]
        );
        assert!(initialized.address < emitted_end);
        assert!(small.address >= emitted_end);
        assert!(big.address >= emitted_end);
    }

    #[test]
    fn local_sized_byte_array_absolute_initializer_binds_mir_index_base() {
        let output = generate_mir6502_source_with_origin(
            "PROC SetColor(BYTE i) BYTE ARRAY colors(4)=$2C0 colors(i)=$42 RETURN PROC Main() SetColor(1) RETURN",
            0x3000,
        );

        let direct_absolute_store =
            bytes_contain(
                &output.bytes,
                &[0xAC, 0x00, 0x30, 0xA9, 0x42, 0x99, 0xC0, 0x02],
            ) || bytes_contain(&output.bytes, &[0xA9, 0x42, 0x99, 0xC0, 0x02]);
        let indirect_store_through_absolute_base = bytes_contain(
            &output.bytes,
            &[0xA9, 0xC0, 0x85, 0xAC, 0xA9, 0x02, 0x85, 0xAD],
        );

        assert!(
            direct_absolute_store || indirect_store_through_absolute_base,
            "local absolute array did not target $02C0: {:02X?}",
            output.bytes
        );
    }

    #[test]
    fn global_inline_byte_array_computed_index_uses_absolute_y() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY colors(0)=[$68 $0C $96 $38] BYTE i,j,out PROC Main() out=colors((i+j)&3) RETURN",
            0x3000,
        );
        let colors = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "colors")
            .expect("colors storage symbol");
        let [lo, hi] = colors.address.to_le_bytes();

        assert!(bytes_contain(&output.bytes, &[0xB9, lo, hi]));
    }

    #[test]
    fn global_byte_array_inc_dec_uses_native_absolute_x_updates() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY colors(4) BYTE i PROC Main() colors(i)=colors(i)+1 colors(i)=colors(i)-1 RETURN",
            0x3000,
        );
        let colors = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "colors")
            .expect("colors storage symbol");
        let [lo, hi] = colors.address.to_le_bytes();

        assert!(bytes_contain(&output.bytes, &[0xFE, lo, hi]));
        assert!(bytes_contain(&output.bytes, &[0xDE, lo, hi]));
    }

    #[test]
    fn absolute_byte_array_update_avoids_native_read_modify_write() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY colors(4)=$D000 BYTE i PROC Main() colors(i)=colors(i)+1 RETURN",
            0x3000,
        );

        assert!(!bytes_contain(&output.bytes, &[0xFE, 0x00, 0xD0]));
        assert!(!bytes_contain(&output.bytes, &[0xDE, 0x00, 0xD0]));
    }

    #[test]
    fn local_inline_byte_array_computed_index_uses_absolute_y() {
        let output = generate_mir6502_source_with_origin(
            "BYTE i,j,out PROC Main() BYTE ARRAY colors(0)=[$68 $0C $96 $38] out=colors((i+j)&3) RETURN",
            0x3000,
        );
        let colors = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "colors")
            .expect("colors storage symbol");
        let [lo, hi] = colors.address.to_le_bytes();

        assert!(output.skipped_ranges.is_empty());
        assert!(bytes_contain(&output.bytes, &[0xB9, lo, hi]));
    }

    #[test]
    fn local_record_scalars_reserve_full_storage() {
        let output = generate_mir6502_source_with_origin(
            "TYPE REAL=[CARD r1,r2,r3] PROC Main() REAL x,y RETURN",
            0x3000,
        );

        let x = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| {
                symbol.name == "x"
                    && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Main".into())
            })
            .expect("x local storage symbol");
        let y = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| {
                symbol.name == "y"
                    && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Main".into())
            })
            .expect("y local storage symbol");

        assert_eq!(x.size, 6);
        assert_eq!(y.size, 6);
        assert_eq!(y.address, x.address.wrapping_add(6));
        assert!(output.skipped_ranges.is_empty());
    }

    #[test]
    fn define_sized_global_arrays_reserve_inline_mir_storage() {
        let output = generate_mir6502_source_with_origin(
            "DEFINE max=\"255\" BYTE rb INT ARRAY xd(max) BYTE ARRAY alive(max), expl(max) PROC Main() alive(0)=1 expl(1)=2 RETURN",
            0x3000,
        );

        let symbol = |name: &str| {
            output
                .map
                .storage_symbols
                .iter()
                .find(|symbol| {
                    symbol.name == name
                        && symbol.scope == crate::codegen::CodegenSymbolScope::Global
                })
                .unwrap_or_else(|| panic!("{name} storage symbol"))
        };
        let rb = symbol("rb");
        let xd = symbol("xd");
        let alive = symbol("alive");
        let expl = symbol("expl");

        assert_eq!(rb.size, 1);
        assert_eq!(xd.size, 255 * 2);
        assert_eq!(alive.size, 255);
        assert_eq!(expl.size, 255);
        assert_eq!(xd.address, rb.address.wrapping_add(1));
        assert_eq!(alive.address, xd.address.wrapping_add(255 * 2));
        assert_eq!(expl.address, alive.address.wrapping_add(255));
    }

    #[test]
    fn sized_byte_array_char_initializer_preserves_atascii_bytes_in_mir_output() {
        let output = generate_mir6502_source_with_origin(
            r#"BYTE ARRAY shape(6)=['\{$00}'@'\{INV: }'\{$02}'A'\{$FF}] PROC Main() RETURN"#,
            0x3000,
        );
        let shape = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| {
                symbol.name == "shape" && symbol.scope == crate::codegen::CodegenSymbolScope::Global
            })
            .expect("shape storage symbol");
        let start = usize::from(shape.address.wrapping_sub(output.origin));

        assert_eq!(
            &output.bytes[start..start + 6],
            &[0x00, b'@', 0xA0, 0x02, b'A', 0xFF]
        );
    }

    #[test]
    fn unsized_byte_array_initializer_emits_pointer_descriptor_in_mir_output() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY data=[1 2 3 4] BYTE i,out PROC Main() i=1 out=data(i) RETURN",
            0x3000,
        );
        let data = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| {
                symbol.name == "data" && symbol.scope == crate::codegen::CodegenSymbolScope::Global
            })
            .expect("data storage symbol");
        let descriptor = usize::from(data.address.wrapping_sub(output.origin));
        let backing = u16::from_le_bytes([output.bytes[descriptor], output.bytes[descriptor + 1]]);
        let backing_start = usize::from(backing.wrapping_sub(output.origin));

        assert_eq!(
            &output.bytes[backing_start..backing_start + 4],
            &[1, 2, 3, 4]
        );
        assert_eq!(data.size, 2);
    }

    #[test]
    fn large_global_byte_arrays_are_deferred_after_mir_code() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY small(256), big(257) BYTE sink PROC Main() sink=small(0)+big(0) RETURN",
            0x3000,
        );

        let small = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "small")
            .expect("small storage symbol");
        let big = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "big")
            .expect("big storage symbol");

        assert_eq!(
            output.skipped_ranges,
            vec![crate::codegen::SkippedRange {
                start: big.address,
                len: 257
            }]
        );
        assert!(small.address < output.origin.wrapping_add(output.bytes.len() as u16));
        assert!(big.address >= output.origin.wrapping_add(output.bytes.len() as u16));
    }

    #[test]
    fn mir_program_end_word_uses_deferred_storage_high_water() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY buffer PROC UsesBacking() BYTE ARRAY temp(300) temp(0)=1 RETURN PROC Main() UsesBacking() RETURN SET buffer=*",
            0x3000,
        );

        let buffer = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "buffer")
            .expect("buffer storage symbol");
        let offset = usize::from(buffer.address.wrapping_sub(output.origin));
        let patched = u16::from_le_bytes([output.bytes[offset], output.bytes[offset + 1]]);
        let skipped_end = output
            .skipped_ranges
            .iter()
            .map(|range| range.start.wrapping_add(range.len))
            .max()
            .expect("skipped range");

        assert_eq!(patched, skipped_end);
        assert_eq!(output.map.skipped_ranges, output.skipped_ranges);
        assert_eq!(
            crate::codegen::format_load_file(&output).len(),
            6 + output.bytes.len() + 6
        );
    }

    #[test]
    fn mir_program_end_word_follows_all_deferred_storage() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY buffer, global_big(300) BYTE sink PROC UsesBacking() BYTE ARRAY local_big(301) local_big(0)=global_big(0) sink=local_big(0) RETURN PROC Main() UsesBacking() RETURN SET buffer=*",
            0x3000,
        );

        let buffer = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "buffer")
            .expect("buffer storage symbol");
        let offset = usize::from(buffer.address.wrapping_sub(output.origin));
        let patched = u16::from_le_bytes([output.bytes[offset], output.bytes[offset + 1]]);
        let emitted_end = output.origin.wrapping_add(output.bytes.len() as u16);
        let skipped_end = output
            .skipped_ranges
            .iter()
            .map(|range| range.start.wrapping_add(range.len))
            .max()
            .expect("skipped ranges");

        assert_eq!(output.skipped_ranges.len(), 2);
        assert!(
            output
                .skipped_ranges
                .iter()
                .all(|range| range.start >= emitted_end)
        );
        assert_eq!(patched, skipped_end);
    }

    #[test]
    fn mir_program_end_word_applies_to_scalar_card_set_current_location() {
        let output = generate_mir6502_source_with_origin(
            "CARD endprog BYTE ARRAY global_big(300) PROC Main() global_big(0)=1 RETURN SET endprog=*",
            0x3000,
        );

        let endprog = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "endprog")
            .expect("endprog storage symbol");
        let offset = usize::from(endprog.address.wrapping_sub(output.origin));
        let patched = u16::from_le_bytes([output.bytes[offset], output.bytes[offset + 1]]);
        let skipped_end = output
            .skipped_ranges
            .iter()
            .map(|range| range.start.wrapping_add(range.len))
            .max()
            .expect("skipped ranges");

        assert_eq!(patched, skipped_end);
        assert_ne!(patched, 0);
    }

    #[test]
    fn mir_segment_high_water_spans_modules() {
        let output = generate_mir6502_source_with_origin(
            "BYTE ARRAY buffer, first_big(300) MODULE BYTE sink PROC UsesBacking() BYTE ARRAY second_big(301) second_big(0)=first_big(0) sink=second_big(0) RETURN MODULE PROC Main() UsesBacking() RETURN SET buffer=*",
            0x3000,
        );

        let buffer = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "buffer")
            .expect("buffer storage symbol");
        let offset = usize::from(buffer.address.wrapping_sub(output.origin));
        let patched = u16::from_le_bytes([output.bytes[offset], output.bytes[offset + 1]]);
        let skipped_end = output
            .skipped_ranges
            .iter()
            .map(|range| range.start.wrapping_add(range.len))
            .max()
            .expect("skipped ranges");

        assert_eq!(output.skipped_ranges.len(), 2);
        assert_eq!(patched, skipped_end);
        assert_eq!(output.map.skipped_ranges, output.skipped_ranges);
    }

    #[test]
    fn tn_deferred_storage_starts_after_final_mir_bytes() {
        let output = generate_mir6502_sample("samples/tn/modern/TN.ACT", 0x2C00);
        let emitted_end = output.origin.wrapping_add(output.bytes.len() as u16);

        assert!(
            !output.skipped_ranges.is_empty(),
            "TN should defer large local storage ranges"
        );
        assert!(
            output
                .skipped_ranges
                .iter()
                .all(|range| range.start >= emitted_end),
            "skipped ranges must not overlap final code: emitted_end=${emitted_end:04X}, skipped={:?}",
            output.skipped_ranges
        );
    }

    #[test]
    fn classifier_rejects_deref_before_pointer_support() {
        let place = crate::nir::NirPlace {
            kind: crate::nir::NirPlaceKind::Deref {
                addr: crate::nir::NirValue::ConstU16(0x4000),
            },
            ty: Some(crate::nir::NirType {
                kind: crate::nir::NirTypeKind::U8,
                summary: "Byte".to_string(),
                width: Some(1),
                pointer: false,
            }),
        };
        assert!(matches!(
            super::classify::classify_place(&place),
            super::classify::MirPlaceShape::PointerDeref { .. }
        ));
    }

    fn generate_mir6502_source(source: &str) -> crate::codegen::CodegenOutput {
        generate_mir6502_source_with_origin(source, crate::codegen::CODE_ORIGIN)
    }

    fn generate_mir6502_source_with_origin(
        source: &str,
        origin: u16,
    ) -> crate::codegen::CodegenOutput {
        generate_mir6502_source_with_config(source, origin, &Mir6502Config::default())
    }

    fn generate_mir6502_source_with_config(
        source: &str,
        origin: u16,
        config: &Mir6502Config,
    ) -> crate::codegen::CodegenOutput {
        let tokens = crate::lexer::tokenize(source).expect("tokenize source");
        let program = crate::parser::parse(&tokens).expect("parse source");
        let model = crate::semantic::analyze(&program).expect("analyze source");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        let nir =
            crate::nir::optimize_program(&crate::nir::lower_program(&semir)).expect("optimize NIR");
        generate_output_with_config(&nir, origin, config).expect("generate mir6502 output")
    }

    fn generate_unoptimized_nir_mir6502_source_with_origin(
        source: &str,
        origin: u16,
    ) -> crate::codegen::CodegenOutput {
        let tokens = crate::lexer::tokenize(source).expect("tokenize source");
        let program = crate::parser::parse(&tokens).expect("parse source");
        let model = crate::semantic::analyze(&program).expect("analyze source");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        let nir = crate::nir::lower_program(&semir);
        crate::nir::verify_program(&nir).expect("verify lowered NIR");
        generate_output_with_config(&nir, origin, &Mir6502Config::default())
            .expect("generate mir6502 output")
    }

    fn generate_mir6502_sample(path: &str, origin: u16) -> crate::codegen::CodegenOutput {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let program = crate::includes::load_program_with_includes(&path).expect("load sample");
        let model = crate::semantic::analyze(&program).expect("analyze sample");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        let nir =
            crate::nir::optimize_program(&crate::nir::lower_program(&semir)).expect("optimize NIR");
        generate_output(&nir, origin).expect("generate mir6502 output")
    }

    fn bytes_contain(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|window| window == needle)
    }

    fn bytes_contain_jsr_to_routine(output: &crate::codegen::CodegenOutput, name: &str) -> bool {
        let Some(address) = output
            .routine_addresses
            .iter()
            .find_map(|routine| (routine.name == name).then_some(routine.address))
        else {
            return false;
        };
        bytes_contain(
            &output.bytes,
            &[0x20, (address & 0x00FF) as u8, (address >> 8) as u8],
        )
    }
}
