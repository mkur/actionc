//! Execute final code, checking both the intended array and fake-pointer guards.
use super::*;
use crate::codegen::tracked_emitter::TrackedEmitter;

const SOURCE: &str = include_str!("../../../tests/fixtures/indexed_word_backing.act");
const ORIGIN: u16 = 0x3000;

fn lower(source: &str) -> MirProgram {
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
    crate::mir6502::lower_program(&nir).unwrap()
}

#[test]
fn indexed_word_backings_preserve_destinations_and_guard_memory() {
    // 128 words is the last directly backed fixed array; 129 uses a descriptor.
    // The unaligned base also tests page carries and a word straddling a page.
    for (length, fixed_base, runtime_bound) in [
        (64, Some(0x4900), false),
        (64, Some(0x49F1), false),
        (128, Some(0x49F1), false),
        (129, Some(0x49F1), false),
        (256, Some(0x49F1), false),
        (129, Some(0x49F1), true),
        (256, Some(0x49F1), true),
        (64, None, false),
    ] {
        let declaration = match fixed_base {
            _ if runtime_bound => "words".to_string(),
            Some(base) => format!("words({length})=${base:04X}"),
            None => format!("words({length})"),
        };
        let mut source = SOURCE.replace("words(64)=$4900", &declaration);
        if runtime_bound {
            // An unsized descriptor bound at runtime also exercises a real
            // pointer load, not an immutable descriptor folded to ConstU16.
            source = source
                .replace("CARD result=", "CARD address=$0606\nCARD result=")
                .replace("arrayBase=words", "words=address\n  arrayBase=words");
        }
        let lowered = lower(&source);
        let pointer_backed = fixed_base.is_some() && length > 128;
        let put = lowered.routines.iter().find(|r| r.name == "Put").unwrap();
        assert!(put.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            matches!(op, MirOp::Store { dst, width: MirWidth::Word, .. }
                if if pointer_backed {
                    matches!(dst, MirAddr::PointerIndex { .. })
                        || (runtime_bound && matches!(dst, MirAddr::ComputedIndex { base: MirValue::Def(_), .. }))
                        || (!runtime_bound && matches!(dst, MirAddr::ComputedIndex { base: MirValue::ConstU16(_), .. }))
                } else {
                    matches!(dst, MirAddr::ComputedIndex { base: MirValue::GlobalAddr(_), .. })
                })
        }), "{declaration}: unexpected lowering: {put:#?}");

        for inline in [false, true] {
            let mut config = Mir6502Config::optimized();
            config.enable_small_leaf_inlining = inline;
            let program = crate::mir6502::materialize_program(lowered.clone(), &config).unwrap();
            let main = program.routines.iter().find(|r| r.name == "Main").unwrap();
            let mut emitter = TrackedEmitter::with_origin(ORIGIN);
            let summary =
                crate::mir6502::emit::emit_program(&program, ORIGIN, &mut emitter).unwrap();
            let bytes = emitter.finish_with_relocations().unwrap().bytes;
            let entry = summary
                .block_ranges
                .iter()
                .find(|(r, b, _)| *r == main.id && *b == main.blocks[0].id)
                .unwrap()
                .2
                .start
                + usize::from(ORIGIN);

            for index in [0, 1, 7, 8, 63, 64, 127, 128, 255]
                .into_iter()
                .filter(|i| *i < length)
            {
                let context =
                    format!("{declaration}, index={index}, inline={inline}, runtime_bound={runtime_bound}");
                let mut memory = [0u8; 65536];
                memory[usize::from(ORIGIN)..usize::from(ORIGIN) + bytes.len()]
                    .copy_from_slice(&bytes);
                memory[0x200..0x600].fill(0xC3);
                memory[0x4800..0x4E00].fill(0xA5);
                memory[0x5A00..0x5C00].fill(0x96);
                memory[0x600] = index as u8;
                memory[0x606..0x608].copy_from_slice(&0x49F1u16.to_le_bytes());
                let mut expected = memory;
                super::leaf_test_cpu::run_memory(&mut memory, entry);
                let base = usize::from(u16::from_le_bytes([memory[0x604], memory[0x605]]));
                if let Some(fixed) = fixed_base {
                    assert_eq!(base, fixed, "{context}");
                }
                expected[base..base + 2].copy_from_slice(&0x5A00u16.to_le_bytes());
                expected[base + index * 2..base + index * 2 + 2]
                    .copy_from_slice(&0xBEEFu16.to_le_bytes());
                assert_eq!(
                    &memory[base..base + length * 2],
                    &expected[base..base + length * 2],
                    "array: {context}"
                );
                assert_eq!(
                    &memory[0x602..0x604],
                    &0xBEEFu16.to_le_bytes(),
                    "readback: {context}"
                );
                assert_eq!(
                    &memory[0x200..0x600],
                    &expected[0x200..0x600],
                    "low-memory guard: {context}"
                );
                assert_eq!(
                    &memory[0x4800..0x4E00],
                    &expected[0x4800..0x4E00],
                    "backing neighbors: {context}"
                );
                assert_eq!(
                    &memory[0x5A00..0x5C00],
                    &expected[0x5A00..0x5C00],
                    "fake-pointer guard: {context}"
                );
            }
        }
    }
}

#[test]
fn dynamic_word_selector_preserves_direct_address_values() {
    let program = MirProgram {
        globals: vec![],
        statics: vec![],
        routines: vec![],
        machine_blocks: vec![],
        runtime_helpers: vec![],
    };
    let layout = MaterializeLayout::new(&program, ORIGIN);
    for base in [
        MirValue::GlobalAddr(crate::nir::SymbolId(0)),
        MirValue::StaticAddr(crate::nir::SymbolId(0)),
        MirValue::ConstU16(0x49F1),
    ] {
        for offset in [0, 3] {
            let ops = vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(0)),
                    src: MirAddr::Direct(MirMem::Param {
                        id: crate::nir::ParamId(0),
                        offset: 0,
                    }),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::ComputedIndex {
                        base: base.clone(),
                        index: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                        elem_size: 2,
                        offset,
                    },
                    src: MirValue::ConstU16(0xBEEF),
                    width: MirWidth::Word,
                },
            ];
            let mut replacement = vec![];
            assert_eq!(
                indexes::try_prepare_dynamic_word_index(
                    &ops,
                    0,
                    RoutineId(0),
                    &layout,
                    &mut replacement
                ),
                2
            );
            assert!(
                matches!(&replacement[0], MirOp::MaterializeIndexedAddress { base: selected, scale: 2, .. } if *selected == base)
            );
            assert!(
                matches!(&replacement[1], MirOp::StoreIndirect { offset: selected, .. } if *selected == offset)
            );
            assert!(
                matches!(&replacement[2], MirOp::StoreIndirect { offset: selected, .. } if *selected == offset + 1)
            );
        }
    }
}
