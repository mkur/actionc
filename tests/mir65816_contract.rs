use actionc::backend::BackendLoweringError;
use actionc::mir65816::*;
use actionc::nir::{self, NirDataAddressEncoding, NirDataAddressTarget, NirDataFragment};
use actionc::source::Span;
use actionc::target::{AddressValue, ByteOffset, ByteSize, TargetId, TargetLayout};
use actionc::{lexer, parser, semantic};

const TARGETS: [TargetId; 2] = [TargetId::Wdc65816Native, TargetId::Wdc65816Small];

fn source_nir(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    program
}

fn lower(program: &nir::NirProgram, optimized: bool) -> Mir65816Program {
    if optimized {
        lower_program(&nir::optimize_program(program).unwrap()).unwrap()
    } else {
        lower_program(program).unwrap()
    }
}

fn global_id(program: &nir::NirProgram, name: &str) -> nir::SymbolId {
    program.globals.iter().find(|g| g.name == name).unwrap().id
}

fn item(program: &Mir65816Program, id: Mir65816DataId) -> &Mir65816Data {
    program.data.iter().find(|d| d.id == id).unwrap()
}

fn operations(program: &Mir65816Program) -> impl Iterator<Item = &Mir65816Op> {
    program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
}

#[test]
fn comparisons_preserve_operand_signedness_and_width_in_both_models() {
    for target in TARGETS {
        for (ty, signed, width) in [
            ("BYTE", false, 1),
            ("INT", true, 2),
            ("CARD", false, 2),
            ("LONGINT", true, 4),
            ("LONGCARD", false, 4),
            ("ADDRESS", false, 3),
            (
                "SIZE",
                false,
                if target == TargetId::Wdc65816Native {
                    3
                } else {
                    2
                },
            ),
        ] {
            let nir = source_nir(
                &format!(
                    "{ty} a,b BYTE lt,le,gt,ge,eq,ne PROC Main()\n\
                 lt=a<b le=a<=b gt=a>b ge=a>=b eq=a=b ne=a#b RETURN"
                ),
                target,
            );
            for optimized in [false, true] {
                let mir = lower(&nir, optimized);
                let comparisons: Vec<_> = operations(&mir)
                    .filter_map(|op| match op {
                        Mir65816Op::Compare {
                            width,
                            signed,
                            operation,
                            ..
                        } => Some((*operation, width.get(), *signed)),
                        _ => None,
                    })
                    .collect();
                for op in [
                    nir::NirCompareOp::Lt,
                    nir::NirCompareOp::Le,
                    nir::NirCompareOp::Gt,
                    nir::NirCompareOp::Ge,
                    nir::NirCompareOp::Eq,
                    nir::NirCompareOp::Ne,
                ] {
                    assert!(
                        comparisons.contains(&(op, width, signed)),
                        "{target} {ty} optimized={optimized}: {comparisons:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn signed_and_unsigned_division_and_remainder_keep_their_integer_facts() {
    for target in TARGETS {
        for (ty, signed, width) in [
            ("INT", true, 2),
            ("CARD", false, 2),
            ("LONGINT", true, 4),
            ("LONGCARD", false, 4),
        ] {
            let nir = source_nir(
                &format!(
                    "{ty} a,b,quotient,remainder PROC Main() quotient=a/b remainder=a MOD b RETURN"
                ),
                target,
            );
            for optimized in [false, true] {
                let mir = lower(&nir, optimized);
                for expected in [nir::NirBinaryOp::Div, nir::NirBinaryOp::Mod] {
                    assert!(operations(&mir).any(|op| matches!(op,
                        Mir65816Op::Binary { width: actual_width, signed: actual_signed, operation, .. }
                        if *operation == expected && actual_width.get() == width && *actual_signed == signed
                    )), "{target} {ty} {expected:?} optimized={optimized}");
                }
            }
        }
    }
}

#[test]
fn globals_keep_storage_initialization_alignment_and_descriptor_identities() {
    let source = "TYPE Pair=[BYTE tag CARD word]\n\
        CARD word,initialized=[$1234]\nLONGINT wide\nBYTE POINTER ptr\nPair current\n\
        BYTE ARRAY bytes(4)=[1 2]\nCARD ARRAY table(3)=[$1234 $5678]\n\
        CARD mmio=$D000\nBYTE high=word+1,mmiohigh=mmio+1\nPROC Main() RETURN";
    for target in TARGETS {
        let nir = source_nir(source, target);
        for optimized in [false, true] {
            let mir = lower(&nir, optimized);
            let global = |name| item(&mir, Mir65816DataId::Global(global_id(&nir, name)));
            for (name, size, alignment) in [
                ("word", 2, 2),
                ("wide", 4, 2),
                ("current", 4, 2),
                ("ptr", mir.data_pointer_width.get(), 1),
            ] {
                let data = global(name);
                assert_eq!(data.size.get(), size, "{name}");
                assert_eq!(data.zero_fill.get(), size, "{name}");
                assert!(data.bytes.is_empty());
                assert_eq!(data.alignment.get(), alignment, "{name}");
                assert_eq!(data.placement, Mir65816DataPlacement::Allocate);
            }
            assert_eq!(global("initialized").bytes, [0x34, 0x12]);
            assert_eq!(global("initialized").alignment.get(), 2);
            let bytes = global("bytes");
            assert_eq!(
                (
                    bytes.size.get(),
                    bytes.zero_fill.get(),
                    bytes.alignment.get()
                ),
                (4, 2, 1)
            );
            assert_eq!(bytes.bytes, [1, 2]);

            let table = global("table");
            let backing_id = Mir65816DataId::ArrayBacking(global_id(&nir, "table"));
            let backing = item(&mir, backing_id);
            assert_ne!(table.id, backing.id);
            assert_eq!(table.size.get(), mir.data_pointer_width.get() + 2);
            assert_eq!(table.bytes.len(), table.size.get() as usize);
            assert_eq!(table.zero_fill, ByteSize::ZERO);
            assert_eq!(
                table.alignment,
                nir.target_layout.data_pointer.alignment_bytes
            );
            assert_eq!(backing.bytes, [0x34, 0x12, 0x78, 0x56]);
            assert_eq!(
                (
                    backing.size.get(),
                    backing.zero_fill.get(),
                    backing.alignment.get()
                ),
                (6, 2, 2)
            );
            assert_eq!(backing.section.as_deref(), Some("global.backing"));
            assert_eq!(table.relocations.len(), 1);
            assert_eq!(
                table.relocations[0].target,
                Mir65816RelocationTarget::ArrayBacking(global_id(&nir, "table"))
            );
            assert_eq!(table.relocations[0].width, mir.data_pointer_width);
            assert_eq!(table.relocations[0].byte_index, None);
            assert!(table.mutable && backing.mutable);

            assert_eq!(
                global("mmio").placement,
                Mir65816DataPlacement::Absolute(AddressValue::data(0xD000))
            );
            assert_eq!(
                global("high").placement,
                Mir65816DataPlacement::Alias {
                    target: Mir65816DataId::Global(global_id(&nir, "word")),
                    offset: ByteOffset::new(1),
                }
            );
            assert_eq!(
                global("mmiohigh").placement,
                Mir65816DataPlacement::Absolute(AddressValue::data(0xD001))
            );
            for name in ["mmio", "high", "mmiohigh"] {
                assert!(global(name).bytes.is_empty());
                assert_eq!(global(name).zero_fill, ByteSize::ZERO);
            }
        }
    }
}

#[test]
fn large_zero_fill_arrays_retain_the_full_extent_without_materializing_bytes() {
    let nir = source_nir(
        "BYTE ARRAY arena(70000) PROC Main() RETURN",
        TargetId::Wdc65816Native,
    );
    for optimized in [false, true] {
        let mir = lower(&nir, optimized);
        let arena = item(&mir, Mir65816DataId::Global(global_id(&nir, "arena")));
        assert_eq!(arena.size.get(), 70000);
        assert_eq!(arena.zero_fill.get(), 70000);
        assert!(arena.bytes.is_empty());
    }
}

fn relocation_nir(target: TargetId) -> nir::NirProgram {
    let mut program = source_nir(
        "BYTE value CARD ARRAY table(2)=[1 2]\n\
        BYTE ARRAY device=$D000 PROC Main() RETURN",
        target,
    );
    let id = global_id(&program, "value");
    let width = program.target_layout.data_pointer.size_bytes;
    let mut image = nir::NirDataImage::default();
    let mut push = |encoding: NirDataAddressEncoding, target, addend| {
        let offset = ByteOffset::try_from(image.bytes.len()).unwrap();
        image
            .bytes
            .resize(image.bytes.len() + usize::from(encoding.width()), 0);
        image.fragments.push(NirDataFragment::Address {
            offset,
            encoding,
            target,
            addend,
            span: Span { start: 0, end: 0 },
        });
    };
    for (byte_index, addend) in [(0, -1), (1, 0x100), (2, 0x10000)] {
        push(
            NirDataAddressEncoding::TargetByte { target, byte_index },
            NirDataAddressTarget::Storage(nir::NirStorageId::Global(id)),
            addend,
        );
    }
    push(
        NirDataAddressEncoding::Pointer {
            address_space: TargetLayout::CODE_ADDRESS_SPACE,
            width,
        },
        NirDataAddressTarget::Routine(program.routines[0].id),
        7,
    );
    push(
        NirDataAddressEncoding::TargetByte {
            target,
            byte_index: 2,
        },
        NirDataAddressTarget::Absolute(AddressValue::code(0x123456)),
        -1,
    );
    for name in ["table", "device"] {
        push(
            NirDataAddressEncoding::Pointer {
                address_space: TargetLayout::DATA_ADDRESS_SPACE,
                width,
            },
            NirDataAddressTarget::Storage(nir::NirStorageId::Global(global_id(&program, name))),
            1,
        );
    }
    // Static and global identities are separate, even with the same numeric ID
    // and display name. The source-only bank selector is supplied as verified
    // NIR because Action!'s existing low/high syntax is tagged for Atari 6502.
    program.statics.push(nir::NirStaticData {
        id,
        name: "value".into(),
        ty: program.globals[0].ty.clone().unwrap(),
        image,
        display: String::new(),
        alignment: ByteSize::new(4),
        mutable: false,
        section: "rodata".into(),
    });
    nir::verify_program(&program).unwrap();
    program
}

#[test]
fn relocations_keep_byte_selection_addends_address_spaces_and_storage_targets() {
    for target in TARGETS {
        let nir = relocation_nir(target);
        for optimized in [false, true] {
            let mir = lower(&nir, optimized);
            let data = item(&mir, Mir65816DataId::Static(global_id(&nir, "value")));
            assert!(!data.mutable);
            assert_eq!(data.alignment.get(), 4);
            assert_eq!(data.section.as_deref(), Some("rodata"));
            assert_eq!(data.size.get() as usize, data.bytes.len());
            assert_eq!(data.zero_fill, ByteSize::ZERO);
            assert!(data.bytes.iter().all(|byte| *byte == 0));
            let relocations = &data.relocations;
            assert_eq!(relocations.len(), 7);
            for (index, addend) in [(0, -1), (1, 0x100), (2, 0x10000)] {
                let relocation = &relocations[index];
                assert_eq!(relocation.offset.get(), index as u32);
                assert_eq!(relocation.byte_index, Some(index as u8));
                assert_eq!(relocation.width, ByteSize::ONE);
                assert_eq!(relocation.addend, addend);
                assert_eq!(relocation.address_space, TargetLayout::DATA_ADDRESS_SPACE);
                assert_eq!(
                    relocation.target,
                    Mir65816RelocationTarget::Data(nir::NirStorageId::Global(global_id(
                        &nir, "value"
                    )))
                );
            }
            assert_eq!(relocations[3].offset.get(), 3);
            assert_eq!(relocations[3].width, mir.code_pointer_width);
            assert_eq!(relocations[3].byte_index, None);
            assert_eq!(
                relocations[3].target,
                Mir65816RelocationTarget::Code(nir.routines[0].id)
            );
            assert_eq!(relocations[3].addend, 7);
            assert_eq!(
                relocations[4].address_space,
                TargetLayout::CODE_ADDRESS_SPACE
            );
            assert_eq!(relocations[4].byte_index, Some(2));
            assert_eq!(
                relocations[4].target,
                Mir65816RelocationTarget::Absolute(AddressValue::code(0x123456))
            );
            assert_eq!(relocations[4].addend, -1);
            assert_eq!(
                relocations[5].target,
                Mir65816RelocationTarget::ArrayBacking(global_id(&nir, "table"))
            );
            assert_eq!(
                relocations[6].target,
                Mir65816RelocationTarget::Absolute(AddressValue::data(0xD000))
            );
            assert_eq!(relocations[5].addend, 1);
            assert_eq!(relocations[6].addend, 1);
        }
    }
}

#[test]
fn invalid_byte_selectors_are_rejected_before_target_lowering() {
    for target in TARGETS {
        for encoding in [
            NirDataAddressEncoding::TargetByte {
                target: TargetId::Atari6502,
                byte_index: 0,
            },
            NirDataAddressEncoding::TargetByte {
                target,
                byte_index: 3,
            },
        ] {
            let mut nir = relocation_nir(target);
            let NirDataFragment::Address {
                encoding: actual, ..
            } = &mut nir.statics[0].image.fragments[0]
            else {
                unreachable!()
            };
            *actual = encoding;
            assert!(matches!(
                lower_program(&nir),
                Err(BackendLoweringError::InvalidNir(_))
            ));
        }
    }
}

#[test]
fn descriptors_keep_size_words_and_link_values_keep_zero_fill_tails() {
    for target in TARGETS {
        let mut nir = source_nir(
            "CARD ARRAY table(2)=[1 2] PROC POINTER callback ADDRESS end PROC Main() RETURN",
            target,
        );
        let pointer_width = nir.target_layout.code_pointer.size_bytes;
        let routine = nir.routines[0].id;
        let table_id = global_id(&nir, "table");
        let callback_id = global_id(&nir, "callback");
        let end_id = global_id(&nir, "end");
        for global in &mut nir.globals {
            if global.id == table_id {
                let Some(nir::NirGlobalInit::Descriptor { size_word, .. }) = &mut global.init
                else {
                    unreachable!()
                };
                *size_word = Some(0x1234);
            } else if global.id == callback_id {
                global.storage_size = pointer_width.saturating_add(ByteSize::new(2));
                global.init = Some(nir::NirGlobalInit::RoutineAddress {
                    routine,
                    descriptor_size: global.storage_size,
                    size_word: Some(0x5678),
                    mutable: false,
                    section: "callbacks".into(),
                });
            } else if global.id == end_id {
                global.init = Some(nir::NirGlobalInit::LinkValue {
                    value: nir::NirLinkValue::ImageEndAddress,
                    width: ByteSize::new(2),
                    mutable: false,
                    section: "links".into(),
                });
            }
        }
        nir::verify_program(&nir).unwrap();
        for optimized in [false, true] {
            let mir = lower(&nir, optimized);
            let table = item(&mir, Mir65816DataId::Global(table_id));
            let callback = item(&mir, Mir65816DataId::Global(callback_id));
            assert_eq!(&table.bytes[usize::from(pointer_width)..], [0x34, 0x12]);
            assert_eq!(&callback.bytes[usize::from(pointer_width)..], [0x78, 0x56]);
            assert_eq!(
                callback.relocations[0].target,
                Mir65816RelocationTarget::Code(routine)
            );
            assert_eq!(
                callback.relocations[0].address_space,
                TargetLayout::CODE_ADDRESS_SPACE
            );
            assert!(!callback.mutable);
            let end = item(&mir, Mir65816DataId::Global(end_id));
            assert_eq!((end.size.get(), end.zero_fill.get()), (3, 1));
            assert_eq!(end.bytes, [0, 0]);
            assert_eq!(
                end.relocations[0].target,
                Mir65816RelocationTarget::ImageEnd
            );
            assert_eq!(end.relocations[0].width.get(), 2);
        }
    }
}

#[test]
fn inconsistent_routine_descriptors_are_diagnosed_without_panicking() {
    for target in TARGETS {
        for oversized in [false, true] {
            let mut nir = source_nir("PROC POINTER callback PROC Main() RETURN", target);
            let width = nir.target_layout.code_pointer.size_bytes;
            let routine = nir.routines[0].id;
            let global = nir
                .globals
                .iter_mut()
                .find(|g| g.name == "callback")
                .unwrap();
            // NIR accepts these routine-address shapes, but does not establish
            // that a size word fits or that the descriptor fits the global.
            global.init = Some(nir::NirGlobalInit::RoutineAddress {
                routine,
                descriptor_size: if oversized {
                    width.saturating_add(ByteSize::new(2))
                } else {
                    width
                },
                size_word: Some(0x1234),
                mutable: true,
                section: "global".into(),
            });
            nir::verify_program(&nir).unwrap();
            let Err(BackendLoweringError::Backend(errors)) = lower_program(&nir) else {
                panic!("expected a descriptor diagnostic");
            };
            let message = if oversized {
                "initialization extent differs from storage extent"
            } else {
                "invalid routine descriptor layout"
            };
            assert!(
                errors.iter().any(|error| error.message.contains(message)),
                "{errors:?}"
            );
        }
    }
}
