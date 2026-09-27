use super::*;
use crate::codegen::CodegenRelocation;
use crate::mir6502::Mir6502Config;

#[path = "../codegen/test_cpu.rs"]
mod cpu;

fn rebase(bytes: &[u8], relocations: &[CodegenRelocation], origin: u16) -> Vec<u8> {
    let mut result = bytes.to_vec();
    for relocation in relocations {
        let target = u16::try_from(
            i32::from(origin) + i32::from(relocation.target_offset) + relocation.addend,
        )
        .unwrap();
        let offset = usize::from(relocation.value_offset);
        match relocation.kind {
            CodegenRelocationKind::Word16 => {
                result[offset..offset + 2].copy_from_slice(&target.to_le_bytes())
            }
            CodegenRelocationKind::Low8 => result[offset] = target as u8,
            CodegenRelocationKind::High8 => result[offset] = (target >> 8) as u8,
            CodegenRelocationKind::Address8 => result[offset] = u8::try_from(target).unwrap(),
            CodegenRelocationKind::Relative8 => {
                result[offset] =
                    i8::try_from(i32::from(target) - (i32::from(origin) + offset as i32 + 1))
                        .unwrap() as u8
            }
        }
    }
    result
}

#[test]
fn rebased_indexed_arrays_execute_at_the_new_address() {
    for (ty, width) in [("BYTE", 1usize), ("CARD", 2)] {
        for index_ty in ["BYTE", "CARD"] {
            for fixed in [false, true] {
                let placement = if fixed { "=$70F1" } else { "" };
                let source = format!(
                    "{ty} ARRAY table(512){placement}\n{index_ty} index=$0600\n\
                     {ty} value=$0602,result=$0604\nCARD tableBase=$0606\n\
                     PROC Main()\ntableBase=table\ntable(index)=value\nresult=table(index)\nRETURN"
                );
                let tokens = crate::lexer::tokenize(&source).unwrap();
                let ast = crate::parser::parse(&tokens).unwrap();
                let model = crate::semantic::analyze(&ast).unwrap();
                let semir = crate::semantic::ir::lower_program(&ast, &model);
                let raw = crate::nir::lower_program(&semir);
                for nir in [raw.clone(), crate::nir::optimize_program(&raw).unwrap()] {
                    for config in [Mir6502Config::default(), Mir6502Config::optimized()] {
                        let original =
                            super::super::generate_output_with_config(&nir, 0x3000, &config)
                                .unwrap();
                        for origin in [0x3000u16, 0x41C7, 0x52FF] {
                            let direct =
                                super::super::generate_output_with_config(&nir, origin, &config)
                                    .unwrap();
                            let bytes = rebase(&original.bytes, &original.relocations, origin);
                            assert_eq!(
                                bytes, direct.bytes,
                                "{ty}/{index_ty}, fixed={fixed}, origin={origin:04X}"
                            );
                            let entry = origin + (original.run_address - original.origin);
                            assert_eq!(entry, direct.run_address);
                            let base = if fixed {
                                0x70F1
                            } else {
                                direct
                                    .skipped_ranges
                                    .iter()
                                    .find(|range| usize::from(range.len) == width * 512)
                                    .unwrap()
                                    .start
                            };
                            for index in [0u16, 1, 127, 128, 255, 256, 511]
                                .into_iter()
                                .filter(|index| index_ty == "CARD" || *index <= 255)
                            {
                                let mut memory = [0xA5; 65536];
                                memory[usize::from(origin)..usize::from(origin) + bytes.len()]
                                    .copy_from_slice(&bytes);
                                memory[0x600..0x602].copy_from_slice(&index.to_le_bytes());
                                memory[0x602..0x604].copy_from_slice(&0xBEEFu16.to_le_bytes());
                                let start = usize::from(base);
                                let end = start + width * 512;
                                let mut expected = memory[start..end + 1].to_vec();
                                let at = usize::from(index) * width;
                                expected[at..at + width]
                                    .copy_from_slice(&0xBEEFu16.to_le_bytes()[..width]);
                                cpu::run_memory(&mut memory, usize::from(entry));
                                assert_eq!(
                                    &memory[start..end + 1],
                                    expected,
                                    "{ty}/{index_ty}, fixed={fixed}, origin={origin:04X}, index={index}"
                                );
                                assert_eq!(
                                    &memory[0x604..0x604 + width],
                                    &0xBEEFu16.to_le_bytes()[..width]
                                );
                                assert_eq!(
                                    u16::from_le_bytes([memory[0x606], memory[0x607]]),
                                    base
                                );
                                assert_eq!(&memory[0x600..0x602], &index.to_le_bytes());
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn split_global_and_static_addresses_relocate_before_byte_selection() {
    let program = MirProgram {
        statics: vec![],
        globals: vec![],
        routines: vec![],
        machine_blocks: vec![],
        runtime_helpers: vec![],
    };
    for output_relative in [false, true] {
        for value in [
            MirValue::GlobalAddr(SymbolId(0)),
            MirValue::StaticAddr(SymbolId(0)),
        ] {
            let placement = MirStoragePlacement::Absolute {
                address: 0x30F1,
                size: 512,
                output_relative,
            };
            let mut layout = MirObjectLayout::default();
            layout.globals.insert(SymbolId(0), placement);
            layout.statics.insert(SymbolId(0), placement);
            let mut ctx = MirEmitContext::with_layout(
                &program,
                0x3000,
                layout,
                MirBranchRelaxationPlan::default(),
            );
            let (lo, hi) = split_value_as_word(&ctx, &value).unwrap();
            let mut emitter = TrackedEmitter::with_origin(0x3000);
            // Form base+$0123 with a low-byte carry at the original origin.
            emitter.emit_lda_imm(0x23);
            emitter.emit_clc();
            assert!(emit_adc_value_to_a(
                &mut ctx,
                RoutineId(0),
                MirBlockId(0),
                &lo,
                &mut emitter
            ));
            emit_sta_mem(ResolvedMem::Absolute(0x600), &mut emitter);
            emitter.emit_lda_imm(1);
            assert!(emit_adc_value_to_a(
                &mut ctx,
                RoutineId(0),
                MirBlockId(0),
                &hi,
                &mut emitter
            ));
            emit_sta_mem(ResolvedMem::Absolute(0x601), &mut emitter);
            emitter.emit_rts();
            let emission = emitter.finish_with_relocations().unwrap();
            assert_eq!(
                emission.relocations.len(),
                if output_relative { 2 } else { 0 }
            );
            for origin in [0x3000u16, 0x41C7, 0x52FF] {
                let bytes = rebase(&emission.bytes, &emission.relocations, origin);
                let mut memory = [0u8; 65536];
                memory[usize::from(origin)..usize::from(origin) + bytes.len()]
                    .copy_from_slice(&bytes);
                cpu::run_memory(&mut memory, usize::from(origin));
                let expected = if output_relative {
                    origin + 0xF1
                } else {
                    0x30F1
                } + 0x123;
                assert_eq!(u16::from_le_bytes([memory[0x600], memory[0x601]]), expected);
            }
        }
    }
}
