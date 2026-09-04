use actionc::lexer::tokenize;
use actionc::nir::{
    self, NirCallee, NirDataFragment, NirIntegerRole, NirIntegerType, NirOp, NirTypeKind, NirValue,
};
use actionc::parser::parse;
use actionc::semantic::{SemanticOptions, analyze_with_options, ir};
use actionc::target::TargetId;
use actionc::{mir68k, mir65816};

fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let tokens = tokenize(source).expect("tokenize native type source");
    let program = parse(&tokens).expect("parse native type source");
    let model = analyze_with_options(&program, SemanticOptions::modern().with_target(target))
        .expect("analyze native type source");
    let semir = ir::lower_program(&program, &model);
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir).expect("verify native type NIR");
    nir
}

#[test]
fn long_and_ulong_are_contextual_fixed_width_types() {
    let program = lower(
        "LONG signedValue=-200000
ULONG unsignedValue=$FEDCBA98
CONST LONG Negative=-200000
CONST ULONG Mask=$FEDCBA98

PROC Main()
  LONG local
  signedValue=Negative
  unsignedValue=Mask
  local=signedValue+1
RETURN
",
        TargetId::Motorola68000,
    );

    let global_types = program
        .globals
        .iter()
        .filter_map(|global| global.ty.as_ref().map(|ty| ty.kind.clone()))
        .collect::<Vec<_>>();
    assert!(global_types.contains(&NirTypeKind::Integer(NirIntegerType::I32)));
    assert!(global_types.contains(&NirTypeKind::Integer(NirIntegerType::U32)));
    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    assert!(
        main.locals
            .iter()
            .any(|local| local.ty.kind == NirTypeKind::Integer(NirIntegerType::I32))
    );
    assert!(program.globals.iter().any(|global| {
        matches!(
            &global.init,
            Some(actionc::nir::NirGlobalInit::Bytes { image, .. })
                if image.fragments.iter().any(|fragment| matches!(
                    fragment,
                    NirDataFragment::Integer { width, value, .. }
                        if width.get() == 4 && *value == 0xFEDC_BA98
                ))
        )
    }));
}

#[test]
fn a_declared_type_can_shadow_the_long_alias() {
    let program = lower(
        "TYPE LONG=[BYTE value]
LONG item
PROC Main()
  item.value=1
RETURN
",
        TargetId::Motorola68000,
    );
    assert!(program.globals.iter().any(|global| {
        global.ty.as_ref().is_some_and(
            |ty| matches!(&ty.kind, NirTypeKind::Record { name, .. } if name.ends_with("LONG")),
        )
    }));
}

#[test]
fn declared_types_can_shadow_address_and_size_aliases() {
    for name in ["ADDRESS", "SIZE"] {
        let source =
            format!("TYPE {name}=[BYTE value]\n{name} item\nPROC Main() item.value=1 RETURN");
        let program = lower(&source, TargetId::Motorola68000);
        assert!(program.globals.iter().any(|global| {
            global.ty.as_ref().is_some_and(
                |ty| matches!(&ty.kind, NirTypeKind::Record { name: actual, .. } if actual.ends_with(name)),
            )
        }));
    }
}

#[test]
fn long_and_ulong_do_not_become_lexer_keywords() {
    let tokens = tokenize("BYTE long, ulong").expect("tokenize identifiers");
    assert!(tokens.iter().any(
        |token| matches!(&token.kind, actionc::lexer::TokenKind::Ident(name) if name == "long")
    ));
    assert!(tokens.iter().any(
        |token| matches!(&token.kind, actionc::lexer::TokenKind::Ident(name) if name == "ulong")
    ));
}

#[test]
fn native_backends_accept_wide_integer_operations() {
    let source = "LONG left, result
ULONG right
PROC Main()
  left=-200000
  right=$FEDCBA98
  result=left+LONG(right)
RETURN
";
    let m68k = lower(source, TargetId::Motorola68000);
    actionc::mir68k::lower_program(&m68k).expect("lower LONG operations to MIR68K");

    let m65816 = lower(source, TargetId::Wdc65816Native);
    actionc::mir65816::lower_program(&m65816).expect("lower LONG operations to MIR65816");
}

#[test]
fn mir6502_rejects_wide_runtime_values_without_truncating() {
    let program = lower(
        "LONG value PROC Main() value=70000 RETURN",
        TargetId::Atari6502,
    );
    let diagnostics = actionc::mir6502::lower_program(&program).expect_err("reject LONG on 6502");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("wider than 16 bits"))
    );
}

#[test]
fn functions_can_return_wide_values_and_data_pointers() {
    let program = lower(
        "BYTE storage
BYTE POINTER FUNC Address()
  RETURN(@storage)
ULONG FUNC Wide()
  RETURN($FEDCBA98)
PROC Main()
  BYTE POINTER p
  ULONG value
  p=Address()
  value=Wide()
RETURN
",
        TargetId::Motorola68000,
    );
    let address = program
        .routines
        .iter()
        .find(|routine| routine.name == "Address")
        .expect("Address routine");
    assert!(matches!(
        address.signature.result.as_ref().map(|ty| &ty.kind),
        Some(NirTypeKind::Pointer { .. })
    ));
    let wide = program
        .routines
        .iter()
        .find(|routine| routine.name == "Wide")
        .expect("Wide routine");
    assert_eq!(
        wide.signature.result.as_ref().map(|ty| &ty.kind),
        Some(&NirTypeKind::Integer(NirIntegerType::U32))
    );
    actionc::mir68k::lower_program(&program).expect("lower generalized results to MIR68K");
}

#[test]
fn callable_pointer_prototypes_survive_all_storage_shapes_and_indirect_calls() {
    let program = lower(
        "TYPE Holder=[BYTE FUNC POINTER callback(BYTE value)]
BYTE FUNC POINTER global(BYTE value)
BYTE FUNC POINTER ARRAY table(2)(BYTE value)

BYTE FUNC Echo(BYTE value)
  RETURN(value)

PROC Invoke(BYTE FUNC POINTER callback(BYTE value))
  callback(7)
RETURN

PROC Main()
  Holder holder
  BYTE FUNC POINTER local(BYTE value)
  global=@Echo
  local=@Echo
  holder.callback=@Echo
  table(0)=@Echo
  Invoke(local)
  local(1)
RETURN
",
        TargetId::Motorola68000,
    );

    let global = program
        .globals
        .iter()
        .find(|global| global.name == "global")
        .expect("typed global callback");
    assert!(matches!(
        global.ty.as_ref().map(|ty| &ty.kind),
        Some(NirTypeKind::Callable { .. })
    ));

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let indirect = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            NirOp::Call {
                callee: NirCallee::Indirect { .. },
                signature,
                ..
            } => signature.as_ref(),
            _ => None,
        })
        .expect("indirect typed call");
    assert_eq!(indirect.params.len(), 1);
    assert_eq!(indirect.params[0].kind, NirTypeKind::U8);
    assert_eq!(
        indirect.result.as_ref().map(|ty| &ty.kind),
        Some(&NirTypeKind::U8)
    );
}

#[test]
fn callable_pointer_prototypes_enforce_assignment_and_indirect_arity() {
    let source = "BYTE FUNC TakesByte(BYTE value) RETURN(value)
BYTE FUNC POINTER takesCard(CARD value)
BYTE FUNC POINTER byteCallback(BYTE value)
PROC Main()
  takesCard=@TakesByte
  byteCallback()
RETURN
";
    let tokens = tokenize(source).expect("tokenize callable diagnostics source");
    let program = parse(&tokens).expect("parse callable diagnostics source");
    let diagnostics = analyze_with_options(
        &program,
        SemanticOptions::modern().with_target(TargetId::Motorola68000),
    )
    .expect_err("reject incompatible callable use");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot assign")),
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("expects 1 argument(s), got 0") }),
        "{diagnostics:#?}"
    );
}

#[test]
fn address_and_size_follow_each_target_data_model() {
    let source = "ADDRESS addr
SIZE distance
BYTE data
PROC Main()
  BYTE POINTER ptr
  addr=ADDRESS(@data)
  distance=addr-addr
  addr=addr+distance
  ptr=BYTE POINTER(addr)
RETURN
";
    for (target, address_bits, size_bits) in [
        (TargetId::Atari6502, 16, 16),
        (TargetId::Wdc65816Small, 24, 16),
        (TargetId::Wdc65816Native, 24, 24),
        (TargetId::Motorola68000, 32, 32),
    ] {
        let program = lower(source, target);
        let address = program
            .globals
            .iter()
            .find(|global| global.name == "addr")
            .and_then(|global| global.ty.as_ref())
            .expect("ADDRESS global");
        assert_eq!(
            address.kind,
            NirTypeKind::Integer(actionc::nir::NirIntegerType::address(address_bits))
        );
        let size = program
            .globals
            .iter()
            .find(|global| global.name == "distance")
            .and_then(|global| global.ty.as_ref())
            .expect("SIZE global");
        assert_eq!(
            size.kind,
            NirTypeKind::Integer(actionc::nir::NirIntegerType::size(size_bits))
        );
        match target {
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&program).expect("lower ADDRESS to MIR68K");
            }
            TargetId::Wdc65816Native | TargetId::Wdc65816Small => {
                actionc::mir65816::lower_program(&program).expect("lower ADDRESS to MIR65816");
            }
            TargetId::Atari6502 => {}
        }
    }
}

#[test]
fn address_rejects_unrelated_arithmetic() {
    let source = "ADDRESS address SIZE count PROC Main() address=address*count RETURN";
    let tokens = tokenize(source).expect("tokenize ADDRESS arithmetic");
    let program = parse(&tokens).expect("parse ADDRESS arithmetic");
    let diagnostics = analyze_with_options(
        &program,
        SemanticOptions::modern().with_target(TargetId::Motorola68000),
    )
    .expect_err("reject ADDRESS multiplication");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("operator * is not valid for ADDRESS")
    }));
}

#[test]
fn static_address_integers_keep_target_width_and_endianness() {
    for (target, literal, width, endian) in [
        (
            TargetId::Atari6502,
            "$BEEF",
            2,
            actionc::target::Endian::Little,
        ),
        (
            TargetId::Wdc65816Native,
            "$ABCDEF",
            3,
            actionc::target::Endian::Little,
        ),
        (
            TargetId::Motorola68000,
            "$FEDCBA98",
            4,
            actionc::target::Endian::Big,
        ),
    ] {
        let source = format!("ADDRESS origin=[{literal}] PROC Main() RETURN");
        let program = lower(&source, target);
        assert_eq!(program.target_layout.endian, endian);
        let init = program
            .globals
            .iter()
            .find(|global| global.name == "origin")
            .and_then(|global| global.init.as_ref())
            .unwrap_or_else(|| panic!("ADDRESS initializer: {:#?}", program.globals));
        assert!(matches!(
            init,
            actionc::nir::NirGlobalInit::Bytes { image, .. }
                if image.fragments.iter().any(|fragment| matches!(
                    fragment,
                    NirDataFragment::Integer {
                        width: actual_width,
                        ..
                    } if actual_width.get() == width
                ))
        ));
    }
}

#[test]
fn layout_queries_produce_target_sized_values() {
    let source = "TYPE Pair=[BYTE tag CARD value]
BYTE ARRAY values(257)
SIZE result
SIZE FUNC Measure(SIZE bias)
  SIZE index
  result=SIZEOF(values)+ELEMENTS(values)+ALIGNOF(Pair)+OFFSETOF(Pair,value)
  FOR index=0 TO ELEMENTS(values)-1 DO
  OD
  RETURN(result+bias)
";

    for (target, size_bits) in [
        (TargetId::Atari6502, 16),
        (TargetId::Wdc65816Small, 16),
        (TargetId::Wdc65816Native, 24),
        (TargetId::Motorola68000, 32),
    ] {
        let program = lower(source, target);
        let size_type = NirIntegerType::size(size_bits);
        let result = program
            .globals
            .iter()
            .find(|global| global.name == "result")
            .and_then(|global| global.ty.as_ref())
            .expect("SIZE result global");
        assert_eq!(result.kind, NirTypeKind::Integer(size_type));

        let measure = program
            .routines
            .iter()
            .find(|routine| routine.name == "Measure")
            .expect("SIZE-returning routine");
        assert_eq!(
            measure
                .signature
                .result
                .as_ref()
                .and_then(|ty| ty.kind.integer()),
            Some(size_type)
        );
        assert_eq!(measure.signature.params[0].kind.integer(), Some(size_type));
        assert!(
            measure
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| {
                    match op {
                        NirOp::Store { src, .. } => matches!(
                            src,
                            NirValue::IntegerConst { ty, .. }
                                if ty.role == NirIntegerRole::Size && ty.bits == size_bits
                        ),
                        NirOp::Binary { left, right, .. } => {
                            [left, right].into_iter().any(|value| {
                                matches!(
                                    value,
                                    NirValue::IntegerConst { ty, .. }
                                        if ty.role == NirIntegerRole::Size && ty.bits == size_bits
                                )
                            })
                        }
                        _ => false,
                    }
                })
        );
    }
}

#[test]
fn wide_objects_follow_the_selected_size_model() {
    let source = "BYTE ARRAY values(70000) PROC Main() RETURN";

    for target in [TargetId::Wdc65816Native, TargetId::Motorola68000] {
        let program = lower(source, target);
        let values = program
            .globals
            .iter()
            .find(|global| global.name == "values")
            .expect("wide array global");
        assert_eq!(values.storage_size.get(), 70_000);
        assert_eq!(
            values.array.as_ref().and_then(|array| array.length),
            Some(70_000)
        );
    }

    for target in [TargetId::Atari6502, TargetId::Wdc65816Small] {
        let tokens = tokenize(source).expect("tokenize wide array");
        let ast = parse(&tokens).expect("parse wide array");
        let diagnostics = analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
            .expect_err("16-bit SIZE model must reject a 70000-element array");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("does not fit the target's 16-bit SIZE type")
        }));
    }
}

#[test]
fn native_type_surface_corpus_survives_optimization_and_backend_planning() {
    let source = include_str!("../fixtures/native/type_surface.act");

    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let lowered = lower(source, target);
        let optimized = nir::optimize_program(&lowered).unwrap_or_else(|diagnostics| {
            panic!("optimize type surface for {target}: {diagnostics:?}")
        });
        nir::verify_program(&optimized).unwrap_or_else(|diagnostics| {
            panic!("verify optimized type surface for {target}: {diagnostics:?}")
        });

        let address_bits = lowered.target_layout.address_integer_bits;
        let size_bits = lowered.target_layout.size_integer_bits;
        let main = lowered
            .routines
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main NIR routine");
        let indirect_signature = main
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find_map(|op| match op {
                NirOp::Call {
                    callee: NirCallee::Indirect { .. },
                    signature: Some(signature),
                    ..
                } => Some(signature),
                _ => None,
            })
            .expect("typed callback call");
        assert_eq!(
            indirect_signature.params[0].kind,
            NirTypeKind::Integer(NirIntegerType::address(address_bits))
        );
        assert_eq!(
            indirect_signature.params[1].kind,
            NirTypeKind::Integer(NirIntegerType::size(size_bits))
        );
        assert_eq!(
            indirect_signature.params[2].kind,
            NirTypeKind::Integer(NirIntegerType::U32)
        );
        assert_eq!(
            indirect_signature
                .result
                .as_ref()
                .map(|result| &result.kind),
            Some(&NirTypeKind::Integer(NirIntegerType::U32))
        );

        let casts = lowered
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .filter_map(|op| match op {
                NirOp::Cast { from, to, kind, .. } => Some((from, to, kind)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(casts.iter().any(|(from, to, kind)| {
            **kind == nir::NirCastKind::PointerToInteger
                && from.width == Some(lowered.target_layout.data_pointer.size_bytes)
                && to.kind == NirTypeKind::Integer(NirIntegerType::address(address_bits))
        }));
        assert!(casts.iter().any(|(from, to, kind)| {
            **kind == nir::NirCastKind::IntegerToPointer
                && from.kind == NirTypeKind::Integer(NirIntegerType::address(address_bits))
                && to.width == Some(lowered.target_layout.data_pointer.size_bytes)
        }));

        match target {
            TargetId::Motorola68000 => {
                check_mir68k_type_surface(&lowered);
                check_mir68k_type_surface(&optimized);
            }
            TargetId::Wdc65816Native | TargetId::Wdc65816Small => {
                check_mir65816_type_surface(&lowered, target);
                check_mir65816_type_surface(&optimized, target);
            }
            TargetId::Atari6502 => unreachable!(),
        }
    }
}

fn check_mir68k_type_surface(program: &nir::NirProgram) {
    let mir = mir68k::lower_program(program).expect("lower type surface to MIR68K");
    let main = mir
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main MIR68K routine");
    assert!(main.frame.extent.get() >= 12);
    let indirect = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            mir68k::Mir68kOp::Call {
                target: mir68k::Mir68kCallTarget::Indirect(_, width),
                plan,
                ..
            } => Some((*width, plan)),
            _ => None,
        })
        .expect("MIR68K indirect callback");
    assert_eq!(indirect.0.get(), 4);
    assert_eq!(
        indirect.1.result,
        Some(mir68k::Mir68kAbiHome::DataRegister(0))
    );
    assert_eq!(indirect.1.outgoing_bytes.get(), 12);
    assert_eq!(
        indirect.1.arguments,
        vec![
            mir68k::Mir68kAbiHome::StackArgument {
                offset: actionc::target::ByteOffset::new(0),
                size: actionc::target::ByteSize::new(4),
            },
            mir68k::Mir68kAbiHome::StackArgument {
                offset: actionc::target::ByteOffset::new(4),
                size: actionc::target::ByteSize::new(4),
            },
            mir68k::Mir68kAbiHome::StackArgument {
                offset: actionc::target::ByteOffset::new(8),
                size: actionc::target::ByteSize::new(4),
            },
        ]
    );
    assert!(main.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            mir68k::Mir68kOp::Call {
                result: Some((_, width)),
                plan: mir68k::Mir68kCallPlan {
                    result: Some(mir68k::Mir68kAbiHome::AddressRegister(0)),
                    ..
                },
                ..
            } if width.get() == 4
        )
    }));
}

fn check_mir65816_type_surface(program: &nir::NirProgram, target: TargetId) {
    let mir = mir65816::lower_program(program).expect("lower type surface to MIR65816");
    let main = mir
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main MIR65816 routine");
    assert!(main.frame.extent.get() > 0);
    let indirect = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            mir65816::Mir65816Op::Call {
                target: mir65816::Mir65816CallTarget::Indirect(_, width),
                plan,
                ..
            } => Some((*width, plan)),
            _ => None,
        })
        .expect("MIR65816 indirect callback");
    let (code_width, size_width, outgoing) = match target {
        TargetId::Wdc65816Native => (3, 3, 10),
        TargetId::Wdc65816Small => (2, 2, 9),
        _ => unreachable!(),
    };
    assert_eq!(indirect.0.get(), code_width);
    assert_eq!(
        indirect.1.result,
        Some(mir65816::Mir65816AbiHome::AccumulatorAndX)
    );
    assert_eq!(indirect.1.outgoing_bytes.get(), outgoing);
    assert_eq!(indirect.1.arguments.len(), 3);
    assert_eq!(
        indirect.1.arguments[0],
        mir65816::Mir65816AbiHome::StackArgument {
            offset: actionc::target::ByteOffset::new(0),
            size: actionc::target::ByteSize::new(3),
        }
    );
    assert_eq!(
        indirect.1.arguments[1],
        mir65816::Mir65816AbiHome::StackArgument {
            offset: actionc::target::ByteOffset::new(3),
            size: actionc::target::ByteSize::new(size_width),
        }
    );
    assert!(
        main.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(
                op,
                mir65816::Mir65816Op::Call {
                    result: Some((_, width)),
                    plan: mir65816::Mir65816CallPlan {
                        result: Some(mir65816::Mir65816AbiHome::Accumulator),
                        ..
                    },
                    ..
                } if target == TargetId::Wdc65816Small && width.get() == 2
            )
        }) || target == TargetId::Wdc65816Native
    );
}
