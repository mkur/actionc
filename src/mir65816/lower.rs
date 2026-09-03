use super::*;
use crate::backend::VerifiedNir;
use crate::nir::{
    NirCallee, NirDataAddressEncoding, NirDataFragment, NirDataImage, NirGlobalBacking,
    NirGlobalInit, NirLinkValue, NirLocalBacking, NirOp, NirPlace, NirPlaceKind, NirProgram,
    NirRoutine, NirTerminator, NirType, NirValue,
};
use crate::target::{AbiId, ByteOffset, ByteSize, Endian, TargetId};

pub(super) fn lower_program(
    input: VerifiedNir<'_>,
) -> Result<Mir65816Program, Vec<Mir65816Diagnostic>> {
    let program = input.program();
    let layout = input.target_layout();
    let convention = match layout.abi {
        AbiId::Wdc65816Native => Mir65816CallConvention::Native,
        AbiId::Wdc65816Small => Mir65816CallConvention::Small,
        _ => return Err(vec![diagnostic(None, None, "invalid ABI for MIR65816")]),
    };
    debug_assert!(matches!(
        input.target(),
        TargetId::Wdc65816Native | TargetId::Wdc65816Small
    ));
    debug_assert_eq!(layout.endian, Endian::Little);

    let mut diagnostics = Vec::new();
    let mut data = Vec::new();
    for global in &program.globals {
        match &global.init {
            Some(NirGlobalInit::Bytes { image, .. }) => data.push(lower_data_image(
                global.name.clone(),
                image,
                ByteSize::ONE,
                layout.endian,
            )),
            Some(NirGlobalInit::Descriptor { backing, .. }) => data.push(lower_data_image(
                format!("{}.__backing", global.name),
                &backing.image,
                ByteSize::ONE,
                layout.endian,
            )),
            Some(NirGlobalInit::RoutineAddress {
                routine,
                descriptor_size,
                ..
            }) => data.push(Mir65816Data {
                name: global.name.clone(),
                bytes: vec![0; descriptor_size.as_usize().unwrap_or(0)],
                alignment: ByteSize::ONE,
                relocations: vec![Mir65816Relocation {
                    offset: ByteOffset::ZERO,
                    width: layout.code_pointer.size_bytes,
                    address_space: layout.code_pointer.address_space,
                    target: Mir65816RelocationTarget::Code(routine.0),
                    addend: 0,
                }],
            }),
            Some(NirGlobalInit::LinkValue {
                value: NirLinkValue::ImageEndAddress,
                width,
                ..
            }) => data.push(Mir65816Data {
                name: global.name.clone(),
                bytes: vec![0; width.as_usize().unwrap_or(0)],
                alignment: ByteSize::ONE,
                relocations: vec![Mir65816Relocation {
                    offset: ByteOffset::ZERO,
                    width: *width,
                    address_space: layout.data_pointer.address_space,
                    target: Mir65816RelocationTarget::ImageEnd,
                    addend: 0,
                }],
            }),
            Some(NirGlobalInit::ZeroFill { .. }) | None => {}
        }
    }
    for static_data in &program.statics {
        data.push(lower_data_image(
            static_data.name.clone(),
            &static_data.image,
            static_data.alignment,
            layout.endian,
        ));
    }

    let routines = program
        .routines
        .iter()
        .map(|routine| {
            let blocks = routine
                .blocks
                .iter()
                .map(|block| {
                    let ops = block
                        .ops
                        .iter()
                        .filter_map(|op| {
                            lower_op(
                                op,
                                layout.data_pointer.size_bytes,
                                layout.code_pointer.size_bytes,
                                convention,
                                program,
                                routine,
                                &routine.name,
                                &block.label,
                                &mut diagnostics,
                            )
                        })
                        .collect();
                    let terminator = lower_terminator(
                        &block.terminator,
                        layout.data_pointer.size_bytes,
                        layout.code_pointer.size_bytes,
                        &routine.name,
                        &block.label,
                        &mut diagnostics,
                    );
                    Mir65816Block {
                        id: block.id,
                        ops,
                        terminator,
                    }
                })
                .collect();
            Mir65816Routine {
                name: routine.name.clone(),
                blocks,
            }
        })
        .collect();

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(Mir65816Program {
        target: input.target(),
        endian: layout.endian,
        architectural_address_bits: layout.address_bits,
        data_pointer_width: layout.data_pointer.size_bytes,
        code_pointer_width: layout.code_pointer.size_bytes,
        call_convention: convention,
        data,
        runtime_bindings: input
            .runtime_bindings()
            .iter()
            .map(|binding| Mir65816RuntimeBinding {
                symbol: binding.symbol,
                target: binding.target,
            })
            .collect(),
        routines,
    })
}

fn lower_data_image(
    name: String,
    image: &NirDataImage,
    alignment: ByteSize,
    endian: Endian,
) -> Mir65816Data {
    let bytes = image
        .project_constants(endian)
        .expect("verified data image projects to bytes");
    let relocations = image
        .fragments
        .iter()
        .filter_map(|fragment| {
            let NirDataFragment::Address {
                offset,
                encoding,
                target,
                addend,
                ..
            } = fragment
            else {
                return None;
            };
            let (width, address_space) = match encoding {
                NirDataAddressEncoding::Pointer {
                    width,
                    address_space,
                } => (*width, *address_space),
                NirDataAddressEncoding::TargetByte { .. } => (ByteSize::ONE, target_space(*target)),
            };
            Some(Mir65816Relocation {
                offset: *offset,
                width,
                address_space,
                target: relocation_target(*target),
                addend: *addend,
            })
        })
        .collect();
    Mir65816Data {
        name,
        bytes,
        alignment,
        relocations,
    }
}

fn target_space(target: NirDataAddressTarget) -> crate::target::AddressSpaceId {
    match target {
        NirDataAddressTarget::Routine(_) => crate::target::TargetLayout::CODE_ADDRESS_SPACE,
        NirDataAddressTarget::Storage(_) | NirDataAddressTarget::Absolute(_) => {
            crate::target::TargetLayout::DATA_ADDRESS_SPACE
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_op(
    op: &NirOp,
    data_pointer_width: ByteSize,
    code_pointer_width: ByteSize,
    convention: Mir65816CallConvention,
    program: &NirProgram,
    nir_routine: &NirRoutine,
    routine: &str,
    block: &str,
    diagnostics: &mut Vec<Mir65816Diagnostic>,
) -> Option<Mir65816Op> {
    let width = |ty: &NirType| ty.width.expect("verified scalar NIR type has width");
    match op {
        NirOp::Load { dest, ty, place } | NirOp::VolatileLoad { dest, ty, place } => {
            Some(Mir65816Op::Load {
                dest: *dest,
                width: width(ty),
                address: lower_place(
                    place,
                    data_pointer_width,
                    code_pointer_width,
                    program,
                    nir_routine,
                ),
                volatile: matches!(op, NirOp::VolatileLoad { .. }),
            })
        }
        NirOp::AddrOf { dest, ty, place } => Some(Mir65816Op::AddressOf {
            dest: *dest,
            address: lower_place(
                place,
                data_pointer_width,
                code_pointer_width,
                program,
                nir_routine,
            ),
            width: width(ty),
        }),
        NirOp::Store { place, src, ty } | NirOp::VolatileStore { place, src, ty } => {
            Some(Mir65816Op::Store {
                address: lower_place(
                    place,
                    data_pointer_width,
                    code_pointer_width,
                    program,
                    nir_routine,
                ),
                value: lower_value(src, data_pointer_width, code_pointer_width),
                width: width(ty),
                volatile: matches!(op, NirOp::VolatileStore { .. }),
            })
        }
        NirOp::CopyBytes {
            destination,
            source,
            size,
            ..
        } => Some(Mir65816Op::Copy {
            destination: lower_place(
                destination,
                data_pointer_width,
                code_pointer_width,
                program,
                nir_routine,
            ),
            source: lower_place(
                source,
                data_pointer_width,
                code_pointer_width,
                program,
                nir_routine,
            ),
            bytes: *size,
            overlap_safe: true,
        }),
        NirOp::Unary { dest, ty, op, src } => Some(Mir65816Op::Unary {
            dest: *dest,
            width: width(ty),
            operation: *op,
            value: lower_value(src, data_pointer_width, code_pointer_width),
        }),
        NirOp::Cast {
            dest,
            src,
            from,
            to,
            kind,
        } => Some(Mir65816Op::Cast {
            dest: *dest,
            from: width(from),
            to: width(to),
            kind: *kind,
            value: lower_value(src, data_pointer_width, code_pointer_width),
        }),
        NirOp::PointerOffset {
            dest,
            ty,
            base,
            offset,
            subtract,
        } => Some(Mir65816Op::PointerOffset {
            dest: *dest,
            width: width(ty),
            base: lower_value(base, data_pointer_width, code_pointer_width),
            offset: lower_value(offset, data_pointer_width, code_pointer_width),
            subtract: *subtract,
        }),
        NirOp::Binary {
            dest,
            ty,
            op,
            left,
            right,
        } => Some(Mir65816Op::Binary {
            dest: *dest,
            width: width(ty),
            operation: *op,
            left: lower_value(left, data_pointer_width, code_pointer_width),
            right: lower_value(right, data_pointer_width, code_pointer_width),
        }),
        NirOp::Compare {
            dest,
            operand_ty,
            op,
            left,
            right,
            ..
        } => Some(Mir65816Op::Compare {
            dest: *dest,
            width: width(operand_ty),
            operation: *op,
            left: lower_value(left, data_pointer_width, code_pointer_width),
            right: lower_value(right, data_pointer_width, code_pointer_width),
        }),
        NirOp::Call {
            callee,
            args,
            result,
            signature,
            ..
        } => Some(Mir65816Op::Call {
            target: lower_callee(callee, data_pointer_width, code_pointer_width),
            signature: signature.as_ref().map(|signature| signature.id),
            args: args
                .iter()
                .map(|value| lower_value(value, data_pointer_width, code_pointer_width))
                .collect(),
            result: result
                .as_ref()
                .map(|result| (result.dest, width(&result.ty))),
            convention,
        }),
        NirOp::Real(_) => {
            diagnostics.push(diagnostic(
                Some(routine),
                Some(block),
                "native REAL lowering is outside the MIR65816 canary",
            ));
            None
        }
        NirOp::ForeignCode { .. } => {
            diagnostics.push(diagnostic(
                Some(routine),
                Some(block),
                "foreign code has no MIR65816 canary lowering",
            ));
            None
        }
        NirOp::Unsupported { note } => {
            diagnostics.push(diagnostic(Some(routine), Some(block), note));
            None
        }
    }
}

fn lower_place(
    place: &NirPlace,
    data_pointer_width: ByteSize,
    code_pointer_width: ByteSize,
    program: &NirProgram,
    routine: &NirRoutine,
) -> Mir65816Address {
    match &place.kind {
        NirPlaceKind::Param { id, .. } => direct(Mir65816AddressBase::Param(*id)),
        NirPlaceKind::Local { id, .. } => lower_local(*id, program, routine),
        NirPlaceKind::Global { id, .. } => lower_global(*id, program),
        NirPlaceKind::Absolute(address) => Mir65816Address {
            base: Mir65816AddressBase::Absolute(*address),
            displacement: ByteOffset::ZERO,
            index: None,
            mode: Mir65816AddressMode::AbsoluteLong,
        },
        NirPlaceKind::Deref { addr } => Mir65816Address {
            base: Mir65816AddressBase::Pointer(lower_value(
                addr,
                data_pointer_width,
                code_pointer_width,
            )),
            displacement: ByteOffset::ZERO,
            index: None,
            mode: Mir65816AddressMode::LongIndirect,
        },
        NirPlaceKind::Index {
            base_addr,
            index,
            elem_size,
            ..
        } => Mir65816Address {
            base: Mir65816AddressBase::Pointer(lower_value(
                base_addr,
                data_pointer_width,
                code_pointer_width,
            )),
            displacement: ByteOffset::ZERO,
            index: Some(Mir65816Index {
                value: lower_value(index, data_pointer_width, code_pointer_width),
                stride: *elem_size,
            }),
            mode: Mir65816AddressMode::LongIndexed,
        },
        NirPlaceKind::Field { base, offset, .. } => {
            let mut address = lower_place(
                base,
                data_pointer_width,
                code_pointer_width,
                program,
                routine,
            );
            address.displacement = address
                .displacement
                .checked_add(*offset)
                .expect("verified field displacement fits ByteOffset");
            address
        }
    }
}

fn lower_local(
    id: crate::nir::LocalId,
    program: &NirProgram,
    routine: &NirRoutine,
) -> Mir65816Address {
    let Some(local) = routine.locals.iter().find(|local| local.id == id) else {
        return direct(Mir65816AddressBase::Local(id));
    };
    match &local.backing {
        NirLocalBacking::Ordinary => direct(Mir65816AddressBase::Local(id)),
        NirLocalBacking::Absolute(address) => absolute(*address),
        NirLocalBacking::Alias { target, offset, .. } => {
            with_displacement(lower_local(*target, program, routine), *offset)
        }
        NirLocalBacking::GlobalAlias { target, offset, .. } => {
            with_displacement(lower_global(*target, program), *offset)
        }
    }
}

fn lower_global(id: crate::nir::SymbolId, program: &NirProgram) -> Mir65816Address {
    let Some(global) = program.globals.iter().find(|global| global.id == id) else {
        return direct(Mir65816AddressBase::Global(id));
    };
    match &global.backing {
        NirGlobalBacking::Ordinary => direct(Mir65816AddressBase::Global(id)),
        NirGlobalBacking::Absolute(address) => absolute(*address),
        NirGlobalBacking::Alias { target, offset } => {
            with_displacement(lower_global(*target, program), *offset)
        }
    }
}

fn absolute(address: crate::target::AddressValue) -> Mir65816Address {
    Mir65816Address {
        base: Mir65816AddressBase::Absolute(address),
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir65816AddressMode::AbsoluteLong,
    }
}

fn with_displacement(mut address: Mir65816Address, displacement: ByteOffset) -> Mir65816Address {
    address.displacement = address
        .displacement
        .checked_add(displacement)
        .expect("verified alias displacement fits ByteOffset");
    address
}

fn direct(base: Mir65816AddressBase) -> Mir65816Address {
    Mir65816Address {
        base,
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir65816AddressMode::FrameOrStatic,
    }
}

fn lower_value(
    value: &NirValue,
    data_pointer_width: ByteSize,
    code_pointer_width: ByteSize,
) -> Mir65816Value {
    match value {
        NirValue::ConstU8(value) => Mir65816Value::U8(*value),
        NirValue::ConstU16(value) => Mir65816Value::U16(*value),
        NirValue::Null { ty } => Mir65816Value::Null(ty.width.unwrap_or(data_pointer_width)),
        NirValue::AddressConst { address, ty } => {
            Mir65816Value::Address(*address, ty.width.unwrap_or(data_pointer_width))
        }
        NirValue::StaticAddr { id, ty, .. } => {
            Mir65816Value::StaticAddress(*id, ty.width.unwrap_or(data_pointer_width))
        }
        NirValue::Temp { id, ty } => {
            Mir65816Value::Temp(*id, ty.width.unwrap_or(data_pointer_width))
        }
        NirValue::Param(id) => Mir65816Value::Param(*id),
        NirValue::GlobalAddr(id) => Mir65816Value::GlobalAddress(*id, data_pointer_width),
        NirValue::RoutineAddr { id, ty, .. } => {
            Mir65816Value::RoutineAddress(id.0, ty.width.unwrap_or(code_pointer_width))
        }
    }
}

fn lower_callee(
    callee: &NirCallee,
    data_pointer_width: ByteSize,
    code_pointer_width: ByteSize,
) -> Mir65816CallTarget {
    match callee {
        NirCallee::User { id, .. } => Mir65816CallTarget::Direct(id.0),
        NirCallee::Builtin(name) => Mir65816CallTarget::Builtin(name.clone()),
        NirCallee::Runtime { symbol, .. } => Mir65816CallTarget::Runtime(*symbol),
        NirCallee::Indirect { target, ty } => Mir65816CallTarget::Indirect(
            lower_value(target, data_pointer_width, code_pointer_width),
            ty.width.unwrap_or(code_pointer_width),
        ),
    }
}

fn lower_terminator(
    terminator: &NirTerminator,
    data_pointer_width: ByteSize,
    code_pointer_width: ByteSize,
    routine: &str,
    block: &str,
    diagnostics: &mut Vec<Mir65816Diagnostic>,
) -> Mir65816Terminator {
    match terminator {
        NirTerminator::Open => {
            diagnostics.push(diagnostic(
                Some(routine),
                Some(block),
                "open block reached MIR65816 lowering",
            ));
            Mir65816Terminator::Exit
        }
        NirTerminator::Fallthrough => Mir65816Terminator::Fallthrough,
        NirTerminator::Goto(edge) => Mir65816Terminator::Goto(edge.target),
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => Mir65816Terminator::Branch {
            condition: lower_value(condition, data_pointer_width, code_pointer_width),
            then_block: then_edge.target,
            else_block: else_edge.target,
        },
        NirTerminator::Return(value) => Mir65816Terminator::Return(
            value
                .as_ref()
                .map(|value| lower_value(value, data_pointer_width, code_pointer_width)),
        ),
        NirTerminator::Exit => Mir65816Terminator::Exit,
    }
}

fn diagnostic(
    routine: Option<&str>,
    block: Option<&str>,
    message: impl Into<String>,
) -> Mir65816Diagnostic {
    Mir65816Diagnostic {
        routine: routine.map(str::to_string),
        block: block.map(str::to_string),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{
        NirDataAddressEncoding, NirDataFragment, NirDataImage, NirStaticData, NirTypeKind, SymbolId,
    };
    use crate::semantic::{SemanticOptions, analyze_with_options};
    use crate::source::Span;

    const SOURCE: &str = r#"
TYPE Pair=[BYTE tag CARD word]
Pair ARRAY pairs(2)=[1 $2345 2 $6789]
Pair current
BYTE hardware=$1234, value, flag
CARD total
BYTE POINTER bp
PROC POINTER callback

BYTE FUNC Inc(BYTE n)
RETURN(n+1)

PROC Sink()
RETURN

PROC Main()
  bp=@value
  bp^=1
  value=bp^
  current=pairs(1)
  current.word=total
  total=current.word
  total=total+1
  hardware=value
  IF flag THEN
    total=Inc(value)
  ELSE
    total=2
  FI
  callback=@Sink
  callback()
RETURN
"#;

    fn lower_source(target: TargetId) -> crate::nir::NirProgram {
        let tokens = crate::lexer::tokenize(SOURCE).expect("tokenize 65816 canary");
        let program = crate::parser::parse(&tokens).expect("parse 65816 canary");
        let model = analyze_with_options(&program, SemanticOptions::modern().with_target(target))
            .expect("analyze 65816 canary");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        crate::nir::lower_program(&semir)
    }

    fn add_address_relocation_probe(program: &mut crate::nir::NirProgram) {
        let data = program
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case("value"))
            .expect("value global")
            .id;
        let code = program
            .routines
            .iter()
            .position(|routine| routine.name.eq_ignore_ascii_case("Sink"))
            .expect("Sink routine") as u32;
        let ty = crate::nir::NirType {
            kind: NirTypeKind::U8,
            summary: "relocation probe".to_string(),
            width: Some(ByteSize::ONE),
            pointer: false,
        };
        program.statics.push(NirStaticData {
            id: SymbolId(0xF000_0001),
            name: "relocation_probe".to_string(),
            ty,
            image: NirDataImage {
                bytes: vec![0; 6],
                fragments: vec![
                    NirDataFragment::Address {
                        offset: ByteOffset::ZERO,
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
                            width: ByteSize::new(3),
                        },
                        target: NirDataAddressTarget::Storage(NirStorageId::Global(data)),
                        addend: 0,
                        span: Span { start: 0, end: 0 },
                    },
                    NirDataFragment::Address {
                        offset: ByteOffset::new(3),
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: crate::target::TargetLayout::CODE_ADDRESS_SPACE,
                            width: ByteSize::new(3),
                        },
                        target: NirDataAddressTarget::Routine(crate::nir::RoutineId(code)),
                        addend: 0,
                        span: Span { start: 0, end: 0 },
                    },
                ],
            },
            display: String::new(),
            alignment: ByteSize::ONE,
            mutable: true,
            section: "data".to_string(),
        });
    }

    #[test]
    fn native_canary_lowers_portable_operations_and_24_bit_addresses() {
        let mut nir = lower_source(TargetId::Wdc65816Native);
        add_address_relocation_probe(&mut nir);
        let mir = super::super::lower_program(&nir).expect("lower 65816 canary");

        assert_eq!(mir.architectural_address_bits, 24);
        assert_eq!(mir.data_pointer_width, ByteSize::new(3));
        assert_eq!(mir.code_pointer_width, ByteSize::new(3));
        assert_eq!(mir.endian, Endian::Little);
        let probe = mir
            .data
            .iter()
            .find(|data| data.name == "relocation_probe")
            .expect("lower relocation probe");
        assert_eq!(probe.relocations.len(), 2);
        assert!(matches!(
            probe.relocations[0].target,
            Mir65816RelocationTarget::Data(_)
        ));
        assert!(matches!(
            probe.relocations[1].target,
            Mir65816RelocationTarget::Code(_)
        ));
        assert!(
            probe
                .relocations
                .iter()
                .all(|relocation| relocation.width == ByteSize::new(3))
        );

        let ops = mir
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .collect::<Vec<_>>();
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Load {
                address: Mir65816Address {
                    mode: Mir65816AddressMode::LongIndirect,
                    ..
                },
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Store {
                address: Mir65816Address {
                    mode: Mir65816AddressMode::AbsoluteLong,
                    ..
                },
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Copy {
                source: Mir65816Address {
                    index: Some(Mir65816Index { stride, .. }),
                    ..
                },
                bytes,
                ..
            } if *bytes == ByteSize::new(4) && *stride == ByteSize::new(4)
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Store { address, width, .. }
                if *width == ByteSize::new(2)
                    && address.displacement == ByteOffset::new(2)
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Binary { width, .. } if *width == ByteSize::ONE
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Binary { width, .. } if *width == ByteSize::new(2)
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Call {
                target: Mir65816CallTarget::Direct(_),
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir65816Op::Call {
                target: Mir65816CallTarget::Indirect(_, width),
                ..
            } if *width == ByteSize::new(3)
        )));
        assert!(
            mir.routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .any(|block| { matches!(block.terminator, Mir65816Terminator::Branch { .. }) })
        );
        assert!(
            mir.routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .any(|block| { matches!(block.terminator, Mir65816Terminator::Return(Some(_))) })
        );
    }

    #[test]
    fn small_model_keeps_24_bit_architecture_with_16_bit_pointers() {
        let nir = lower_source(TargetId::Wdc65816Small);
        let mir = super::super::lower_program(&nir).expect("lower 65816 small-model canary");
        assert_eq!(mir.architectural_address_bits, 24);
        assert_eq!(mir.data_pointer_width, ByteSize::new(2));
        assert_eq!(mir.code_pointer_width, ByteSize::new(2));
        assert_eq!(mir.call_convention, Mir65816CallConvention::Small);
    }
}
