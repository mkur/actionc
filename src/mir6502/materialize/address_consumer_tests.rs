use super::*;
use crate::runtime::Runtime;

fn output(source: &str, runtime: Runtime, optimized: bool) -> crate::codegen::CodegenOutput {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    crate::mir6502::generate_output_with_config_and_runtime(
        &nir,
        0x3000,
        &if optimized {
            Mir6502Config::optimized()
        } else {
            Mir6502Config::default()
        },
        runtime,
    )
    .unwrap()
}

fn memory(output: &crate::codegen::CodegenOutput, runtime: Runtime) -> [u8; 65536] {
    let mut memory = [0u8; 65536];
    if runtime == Runtime::ActionCart {
        let cart = include_bytes!("../../../roms/action.rom");
        memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
        memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
    }
    let origin = usize::from(output.origin);
    memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
    memory
}

#[test]
fn constant_record_accesses_preserve_complete_words_and_fixed_pointer_cells() {
    for offset in [0u16, 254, 255, 256] {
        for width in [MirWidth::Byte, MirWidth::Word] {
            let ty = if width == MirWidth::Byte {
                "BYTE"
            } else {
                "CARD"
            };
            let prefix = if offset == 0 {
                String::new()
            } else {
                format!("BYTE ARRAY pad({offset}) ")
            };
            // A source-visible fixed zero-page pointer exercises rebasing
            // without mutating that cell. A RAM pointer exercises the general path.
            for pointer_cell in [0x600u16, 0xF0] {
                let source = format!(
                    "TYPE Rec=[{prefix}{ty} field]\nRec POINTER p=${pointer_cell:04X}\n{ty} input=$0602,observed=$0604\nPROC Main()\nobserved=p.field\np.field=input\nRETURN\n"
                );
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    for optimized in [false, true] {
                        for crlf in [false, true] {
                            let text = if crlf {
                                source.replace('\n', "\r\n")
                            } else {
                                source.clone()
                            };
                            let compiled = output(&text, runtime, optimized);
                            for base in [0x5000u16, 0x50F1, 0xFF00] {
                                let mut ram = memory(&compiled, runtime);
                                ram[0x4F00..0x5300].fill(0xA5);
                                ram[usize::from(pointer_cell)..usize::from(pointer_cell) + 2]
                                    .copy_from_slice(&base.to_le_bytes());
                                ram[0x602..0x604].copy_from_slice(&0xB327u16.to_le_bytes());
                                let field = base.wrapping_add(offset);
                                let bytes = if width == MirWidth::Byte { 1usize } else { 2 };
                                for lane in 0..bytes {
                                    ram[usize::from(field.wrapping_add(lane as u16))] =
                                        [0xD9, 0x73][lane];
                                }
                                let before = ram;
                                super::leaf_test_cpu::run_memory(
                                    &mut ram,
                                    usize::from(compiled.run_address),
                                );
                                let label = format!(
                                    "offset={offset}/{width:?}/cell={pointer_cell:04X}/base={base:04X}/{runtime:?}/opt={optimized}/crlf={crlf}"
                                );
                                assert_eq!(
                                    &ram[0x604..0x604 + bytes],
                                    &[0xD9, 0x73][..bytes],
                                    "{label}"
                                );
                                let mut expected = before;
                                for lane in 0..bytes {
                                    expected[usize::from(field.wrapping_add(lane as u16))] =
                                        [0x27, 0xB3][lane];
                                }
                                assert_eq!(
                                    &ram[0x4F00..0x5300],
                                    &expected[0x4F00..0x5300],
                                    "{label}: guards"
                                );
                                for lane in 0..bytes {
                                    assert_eq!(
                                        ram[usize::from(field.wrapping_add(lane as u16))],
                                        [0x27, 0xB3][lane],
                                        "{label}"
                                    );
                                }
                                assert_eq!(
                                    &ram[usize::from(pointer_cell)..usize::from(pointer_cell) + 2],
                                    &base.to_le_bytes(),
                                    "{label}: pointer cell"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn embedded_byte_fields_preserve_indexes_effects_and_neighbor_bytes() {
    for offset in [0u16, 254, 255, 256] {
        let prefix = if offset == 0 {
            String::new()
        } else {
            format!("BYTE ARRAY pad({offset}) ")
        };
        for effectful in [false, true] {
            let index = if effectful { "Index()" } else { "indexValue" };
            let source = format!(
                "TYPE Rec=[{prefix}BYTE ARRAY bytes(300)]\nRec POINTER p=$0600\nBYTE indexValue=$0602,input=$0603,observed=$0604,calls=$0605\nBYTE FUNC Index()\ncalls==+1\nRETURN(indexValue)\nPROC Main()\nobserved=p.bytes({index})\np.bytes(indexValue)=input\nRETURN\n"
            );
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for optimized in [false, true] {
                    for crlf in [false, true] {
                        let text = if crlf {
                            source.replace('\n', "\r\n")
                        } else {
                            source.clone()
                        };
                        let compiled = output(&text, runtime, optimized);
                        for base in [0x5000u16, 0x50F1, 0xFF00] {
                            for index in [0u8, 1, 127, 254, 255] {
                                let mut ram = memory(&compiled, runtime);
                                ram[0x4F00..0x5400].fill(0xA5);
                                ram[0x600..0x602].copy_from_slice(&base.to_le_bytes());
                                ram[0x602] = index;
                                ram[0x603] = 0x37;
                                ram[0x605] = 0;
                                let field =
                                    base.wrapping_add(offset).wrapping_add(u16::from(index));
                                ram[usize::from(field)] = 0xD9;
                                let mut expected = ram;
                                expected[usize::from(field)] = 0x37;
                                super::leaf_test_cpu::run_memory(
                                    &mut ram,
                                    usize::from(compiled.run_address),
                                );
                                let label = format!(
                                    "offset={offset}/base={base:04X}/index={index}/{runtime:?}/opt={optimized}/crlf={crlf}/effect={effectful}"
                                );
                                assert_eq!(ram[0x604], 0xD9, "{label}");
                                assert_eq!(
                                    ram[0x605],
                                    u8::from(effectful),
                                    "{label}: single evaluation"
                                );
                                assert_eq!(ram[usize::from(field)], 0x37, "{label}");
                                assert_eq!(
                                    &ram[0x4F00..0x5400],
                                    &expected[0x4F00..0x5400],
                                    "{label}: guards"
                                );
                                assert_eq!(
                                    &ram[0x600..0x602],
                                    &base.to_le_bytes(),
                                    "{label}: pointer"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}


#[test]
fn computed_address_fallback_executes_full_word_scales_and_wrap() {
    let source = "CARD base=$0600,index=$0602,result=$0604 PROC Main() result=base+index RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))).unwrap();
    for stride in [1u16, 2, 20, 255, 256, 65535] {
        for offset in [0u16, 38, 255, 256] {
            let mut mir = crate::mir6502::lower_program(&nir).unwrap();
            let mut found = false;
            for op in mir.routines.iter_mut().flat_map(|r| &mut r.blocks).flat_map(|b| &mut b.ops) {
                if let MirOp::Binary { op: MirBinaryOp::Add, dst, left, right, width: MirWidth::Word, .. } = op {
                    *op = MirOp::ComputeAddress { dst: dst.clone(), base: left.clone(), index: Some(right.clone()), elem_size: stride, offset };
                    found = true;
                }
            }
            assert!(found);
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for optimized in [false, true] {
                    let config = if optimized { Mir6502Config::optimized() } else { Mir6502Config::default() };
                    let mir = crate::mir6502::materialize_program_with_origin_and_runtime(mir.clone(), &config, 0x3000, runtime).unwrap();
                    crate::mir6502::verify_program(&mir, MirPhase::PreEmission).unwrap();
                    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
                    let summary = crate::mir6502::emit::emit_program(&mir, 0x3000, &mut emitter).unwrap();
                    let emission = emitter.finish_with_relocations().unwrap();
                    let compiled = crate::mir6502::codegen_output(emission, 0x3000, summary, runtime, &mir);
                    for base in [0x5000u16, 0x50F1, 0xFFF0] {
                        for index in [0u16, 1, 255, 256, 257, 65535] {
                            let mut ram = memory(&compiled, runtime);
                            ram[0x600..0x602].copy_from_slice(&base.to_le_bytes());
                            ram[0x602..0x604].copy_from_slice(&index.to_le_bytes());
                            super::leaf_test_cpu::run_memory(&mut ram, usize::from(compiled.run_address));
                            assert_eq!(u16::from_le_bytes([ram[0x604], ram[0x605]]), base.wrapping_add(offset).wrapping_add(index.wrapping_mul(stride)), "stride={stride}/offset={offset}/base={base:04X}/index={index}/{runtime:?}/opt={optimized}");
                        }
                    }
                }
            }
        }
    }
}
