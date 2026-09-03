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
) -> Result<Mir68kProgram, Vec<Mir68kDiagnostic>> {
    let program = input.program();
    let layout = input.target_layout();
    if layout.abi != AbiId::Motorola68kNative {
        return Err(vec![diagnostic(None, None, "invalid ABI for MIR68K")]);
    }
    debug_assert_eq!(input.target(), TargetId::Motorola68000);
    debug_assert_eq!(layout.endian, Endian::Big);

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
            }) => data.push(Mir68kData {
                name: global.name.clone(),
                bytes: vec![0; descriptor_size.as_usize().unwrap_or(0)],
                alignment: layout.code_pointer.alignment_bytes,
                relocations: vec![Mir68kRelocation {
                    offset: ByteOffset::ZERO,
                    width: layout.code_pointer.size_bytes,
                    address_space: layout.code_pointer.address_space,
                    target: Mir68kRelocationTarget::Code(routine.0),
                    addend: 0,
                }],
            }),
            Some(NirGlobalInit::LinkValue {
                value: NirLinkValue::ImageEndAddress,
                width,
                ..
            }) => data.push(Mir68kData {
                name: global.name.clone(),
                bytes: vec![0; width.as_usize().unwrap_or(0)],
                alignment: layout.data_pointer.alignment_bytes,
                relocations: vec![Mir68kRelocation {
                    offset: ByteOffset::ZERO,
                    width: *width,
                    address_space: layout.data_pointer.address_space,
                    target: Mir68kRelocationTarget::ImageEnd,
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
        .map(|routine| lower_routine(program, routine, &mut diagnostics))
        .collect();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(Mir68kProgram {
        endian: layout.endian,
        architectural_address_bits: layout.address_bits,
        data_pointer_width: layout.data_pointer.size_bytes,
        code_pointer_width: layout.code_pointer.size_bytes,
        data,
        runtime_bindings: input
            .runtime_bindings()
            .iter()
            .map(|binding| Mir68kRuntimeBinding {
                symbol: binding.symbol,
                target: binding.target,
            })
            .collect(),
        routines,
    })
}

fn lower_routine(
    program: &NirProgram,
    routine: &NirRoutine,
    diagnostics: &mut Vec<Mir68kDiagnostic>,
) -> Mir68kRoutine {
    let layout = &program.target_layout;
    let blocks = routine
        .blocks
        .iter()
        .map(|block| Mir68kBlock {
            id: block.id,
            ops: block
                .ops
                .iter()
                .filter_map(|op| lower_op(op, program, routine, &block.label, diagnostics))
                .collect(),
            terminator: lower_terminator(
                &block.terminator,
                layout.data_pointer.size_bytes,
                layout.code_pointer.size_bytes,
                &routine.name,
                &block.label,
                diagnostics,
            ),
        })
        .collect();
    Mir68kRoutine {
        name: routine.name.clone(),
        blocks,
    }
}

fn lower_data_image(
    name: String,
    image: &NirDataImage,
    alignment: ByteSize,
    endian: Endian,
) -> Mir68kData {
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
            Some(Mir68kRelocation {
                offset: *offset,
                width,
                address_space,
                target: relocation_target(*target),
                addend: *addend,
            })
        })
        .collect();
    Mir68kData {
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

fn lower_op(
    op: &NirOp,
    program: &NirProgram,
    routine: &NirRoutine,
    block: &str,
    diagnostics: &mut Vec<Mir68kDiagnostic>,
) -> Option<Mir68kOp> {
    let data_width = program.target_layout.data_pointer.size_bytes;
    let code_width = program.target_layout.code_pointer.size_bytes;
    let width = |ty: &NirType| ty.width.expect("verified scalar NIR type has width");
    match op {
        NirOp::Load { dest, ty, place } | NirOp::VolatileLoad { dest, ty, place } => {
            let address = lower_place(place, data_width, code_width, program, routine);
            let width = width(ty);
            Some(Mir68kOp::Load {
                dest: *dest,
                width,
                access: access(width, &address),
                address,
                volatile: matches!(op, NirOp::VolatileLoad { .. }),
            })
        }
        NirOp::AddrOf { dest, ty, place } => Some(Mir68kOp::AddressOf {
            dest: *dest,
            address: lower_place(place, data_width, code_width, program, routine),
            width: width(ty),
        }),
        NirOp::Store { place, src, ty } | NirOp::VolatileStore { place, src, ty } => {
            let address = lower_place(place, data_width, code_width, program, routine);
            let width = width(ty);
            Some(Mir68kOp::Store {
                access: access(width, &address),
                address,
                value: lower_value(src, data_width, code_width),
                width,
                volatile: matches!(op, NirOp::VolatileStore { .. }),
            })
        }
        NirOp::CopyBytes {
            destination,
            source,
            size,
            ..
        } => Some(Mir68kOp::Copy {
            destination: lower_place(destination, data_width, code_width, program, routine),
            source: lower_place(source, data_width, code_width, program, routine),
            bytes: *size,
            overlap_safe: true,
        }),
        NirOp::Unary { dest, ty, op, src } => Some(Mir68kOp::Unary {
            dest: *dest,
            width: width(ty),
            operation: *op,
            value: lower_value(src, data_width, code_width),
        }),
        NirOp::Cast {
            dest,
            src,
            from,
            to,
            kind,
        } => Some(Mir68kOp::Cast {
            dest: *dest,
            from: width(from),
            to: width(to),
            kind: *kind,
            value: lower_value(src, data_width, code_width),
        }),
        NirOp::PointerOffset {
            dest,
            ty,
            base,
            offset,
            subtract,
        } => Some(Mir68kOp::PointerOffset {
            dest: *dest,
            width: width(ty),
            base: lower_value(base, data_width, code_width),
            offset: lower_value(offset, data_width, code_width),
            subtract: *subtract,
        }),
        NirOp::Binary {
            dest,
            ty,
            op,
            left,
            right,
        } => Some(Mir68kOp::Binary {
            dest: *dest,
            width: width(ty),
            operation: *op,
            left: lower_value(left, data_width, code_width),
            right: lower_value(right, data_width, code_width),
        }),
        NirOp::Compare {
            dest,
            operand_ty,
            op,
            left,
            right,
            ..
        } => Some(Mir68kOp::Compare {
            dest: *dest,
            width: width(operand_ty),
            operation: *op,
            left: lower_value(left, data_width, code_width),
            right: lower_value(right, data_width, code_width),
        }),
        NirOp::Call {
            callee,
            args,
            result,
            signature,
            ..
        } => Some(Mir68kOp::Call {
            target: lower_callee(callee, data_width, code_width),
            signature: signature.as_ref().map(|signature| signature.id),
            args: args
                .iter()
                .map(|value| lower_value(value, data_width, code_width))
                .collect(),
            result: result
                .as_ref()
                .map(|result| (result.dest, width(&result.ty))),
        }),
        NirOp::Real(_) => {
            diagnostics.push(diagnostic(
                Some(&routine.name),
                Some(block),
                "native REAL lowering is outside the MIR68K canary",
            ));
            None
        }
        NirOp::ForeignCode { .. } => {
            diagnostics.push(diagnostic(
                Some(&routine.name),
                Some(block),
                "foreign code has no MIR68K canary lowering",
            ));
            None
        }
        NirOp::Unsupported { note } => {
            diagnostics.push(diagnostic(Some(&routine.name), Some(block), note));
            None
        }
    }
}

fn access(width: ByteSize, address: &Mir68kAddress) -> Mir68kAccess {
    match width.get() {
        1 => Mir68kAccess::Byte,
        2 if address
            .base_alignment
            .is_some_and(|alignment| alignment.get() >= 2)
            && address.displacement.get() % 2 == 0 =>
        {
            Mir68kAccess::NativeAlignedWord
        }
        2 => Mir68kAccess::BytewisePackedOddWord {
            endian: Endian::Big,
        },
        _ => Mir68kAccess::Bytes(width),
    }
}

fn lower_place(
    place: &NirPlace,
    data_width: ByteSize,
    code_width: ByteSize,
    program: &NirProgram,
    routine: &NirRoutine,
) -> Mir68kAddress {
    match &place.kind {
        NirPlaceKind::Param { id, .. } => {
            let alignment = routine
                .params
                .iter()
                .find(|param| param.id == *id)
                .map(|param| param.layout.alignment);
            direct(Mir68kAddressBase::Param(*id), alignment)
        }
        NirPlaceKind::Local { id, .. } => lower_local(*id, program, routine),
        NirPlaceKind::Global { id, .. } => lower_global(*id, program),
        NirPlaceKind::Absolute(address) => absolute(*address),
        NirPlaceKind::Deref { addr } => Mir68kAddress {
            base: Mir68kAddressBase::Pointer(lower_value(addr, data_width, code_width)),
            base_alignment: None,
            displacement: ByteOffset::ZERO,
            index: None,
            mode: Mir68kAddressMode::AddressIndirect,
        },
        NirPlaceKind::Index {
            base_addr,
            index,
            elem_size,
            ..
        } => Mir68kAddress {
            base: Mir68kAddressBase::Pointer(lower_value(base_addr, data_width, code_width)),
            base_alignment: None,
            displacement: ByteOffset::ZERO,
            index: Some(Mir68kIndex {
                value: lower_value(index, data_width, code_width),
                stride: *elem_size,
            }),
            mode: Mir68kAddressMode::Indexed,
        },
        NirPlaceKind::Field { base, offset, .. } => with_displacement(
            lower_place(base, data_width, code_width, program, routine),
            *offset,
        ),
    }
}

fn lower_local(
    id: crate::nir::LocalId,
    program: &NirProgram,
    routine: &NirRoutine,
) -> Mir68kAddress {
    let Some(local) = routine.locals.iter().find(|local| local.id == id) else {
        return direct(Mir68kAddressBase::Local(id), None);
    };
    match &local.backing {
        NirLocalBacking::Ordinary => direct(
            Mir68kAddressBase::Local(id),
            Some(local.layout.alignment),
        ),
        NirLocalBacking::Absolute(address) => absolute(*address),
        NirLocalBacking::Alias { target, offset, .. } => {
            with_displacement(lower_local(*target, program, routine), *offset)
        }
        NirLocalBacking::GlobalAlias { target, offset, .. } => {
            with_displacement(lower_global(*target, program), *offset)
        }
    }
}

fn lower_global(id: SymbolId, program: &NirProgram) -> Mir68kAddress {
    let Some(global) = program.globals.iter().find(|global| global.id == id) else {
        return direct(Mir68kAddressBase::Global(id), None);
    };
    match &global.backing {
        NirGlobalBacking::Ordinary => direct(
            Mir68kAddressBase::Global(id),
            global
                .ty
                .as_ref()
                .map(|ty| type_alignment(ty, &program.target_layout)),
        ),
        NirGlobalBacking::Absolute(address) => absolute(*address),
        NirGlobalBacking::Alias { target, offset } => {
            with_displacement(lower_global(*target, program), *offset)
        }
    }
}

fn direct(base: Mir68kAddressBase, base_alignment: Option<ByteSize>) -> Mir68kAddress {
    Mir68kAddress {
        base,
        base_alignment,
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir68kAddressMode::FrameOrStatic,
    }
}

fn absolute(address: AddressValue) -> Mir68kAddress {
    Mir68kAddress {
        base: Mir68kAddressBase::Absolute(address),
        base_alignment: Some(if address.value % 2 == 0 {
            ByteSize::new(2)
        } else {
            ByteSize::ONE
        }),
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir68kAddressMode::Absolute,
    }
}

fn type_alignment(ty: &NirType, layout: &crate::target::TargetLayout) -> ByteSize {
    match &ty.kind {
        crate::nir::NirTypeKind::U16 | crate::nir::NirTypeKind::I16 => ByteSize::new(2),
        crate::nir::NirTypeKind::Pointer { address_space, .. }
            if *address_space == layout.data_pointer.address_space =>
        {
            layout.data_pointer.alignment_bytes
        }
        crate::nir::NirTypeKind::Callable { address_space, .. }
            if *address_space == layout.code_pointer.address_space =>
        {
            layout.code_pointer.alignment_bytes
        }
        crate::nir::NirTypeKind::Record { .. }
            if layout.record_layout == crate::target::RecordLayoutPolicy::Natural =>
        {
            ByteSize::from(layout.natural_word_alignment_bytes)
        }
        _ => ByteSize::ONE,
    }
}

fn with_displacement(mut address: Mir68kAddress, offset: ByteOffset) -> Mir68kAddress {
    address.displacement = address
        .displacement
        .checked_add(offset)
        .expect("verified displacement fits ByteOffset");
    address
}

fn lower_value(value: &NirValue, data_width: ByteSize, code_width: ByteSize) -> Mir68kValue {
    match value {
        NirValue::ConstU8(value) => Mir68kValue::U8(*value),
        NirValue::ConstU16(value) => Mir68kValue::U16(*value),
        NirValue::Null { ty } => Mir68kValue::Null(ty.width.unwrap_or(data_width)),
        NirValue::AddressConst { address, ty } => {
            Mir68kValue::Address(*address, ty.width.unwrap_or(data_width))
        }
        NirValue::StaticAddr { id, ty, .. } => {
            Mir68kValue::StaticAddress(*id, ty.width.unwrap_or(data_width))
        }
        NirValue::Temp { id, ty } => Mir68kValue::Temp(*id, ty.width.unwrap_or(data_width)),
        NirValue::Param(id) => Mir68kValue::Param(*id),
        NirValue::GlobalAddr(id) => Mir68kValue::GlobalAddress(*id, data_width),
        NirValue::RoutineAddr { id, ty, .. } => {
            Mir68kValue::RoutineAddress(id.0, ty.width.unwrap_or(code_width))
        }
    }
}

fn lower_callee(
    callee: &NirCallee,
    data_width: ByteSize,
    code_width: ByteSize,
) -> Mir68kCallTarget {
    match callee {
        NirCallee::User { id, .. } => Mir68kCallTarget::Direct(id.0),
        NirCallee::Builtin(name) => Mir68kCallTarget::Builtin(name.clone()),
        NirCallee::Runtime { symbol, .. } => Mir68kCallTarget::Runtime(*symbol),
        NirCallee::Indirect { target, ty } => Mir68kCallTarget::Indirect(
            lower_value(target, data_width, code_width),
            ty.width.unwrap_or(code_width),
        ),
    }
}

fn lower_terminator(
    terminator: &NirTerminator,
    data_width: ByteSize,
    code_width: ByteSize,
    routine: &str,
    block: &str,
    diagnostics: &mut Vec<Mir68kDiagnostic>,
) -> Mir68kTerminator {
    match terminator {
        NirTerminator::Open => {
            diagnostics.push(diagnostic(
                Some(routine),
                Some(block),
                "open block reached MIR68K lowering",
            ));
            Mir68kTerminator::Exit
        }
        NirTerminator::Fallthrough => Mir68kTerminator::Fallthrough,
        NirTerminator::Goto(edge) => Mir68kTerminator::Goto(edge.target),
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => Mir68kTerminator::Branch {
            condition: lower_value(condition, data_width, code_width),
            then_block: then_edge.target,
            else_block: else_edge.target,
        },
        NirTerminator::Return(value) => Mir68kTerminator::Return(
            value
                .as_ref()
                .map(|value| lower_value(value, data_width, code_width)),
        ),
        NirTerminator::Exit => Mir68kTerminator::Exit,
    }
}

fn diagnostic(
    routine: Option<&str>,
    block: Option<&str>,
    message: impl Into<String>,
) -> Mir68kDiagnostic {
    Mir68kDiagnostic {
        routine: routine.map(str::to_string),
        block: block.map(str::to_string),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{
        NirDataAddressEncoding, NirDataFragment, NirDataImage, NirStaticData, NirTypeKind,
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

    fn lower_source() -> NirProgram {
        let tokens = crate::lexer::tokenize(SOURCE).expect("tokenize 68k canary");
        let program = crate::parser::parse(&tokens).expect("parse 68k canary");
        let model = analyze_with_options(
            &program,
            SemanticOptions::modern().with_target(TargetId::Motorola68000),
        )
        .expect("analyze 68k canary");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        crate::nir::lower_program(&semir)
    }

    fn force_one_packed_odd_word(program: &mut NirProgram) {
        for op in program
            .routines
            .iter_mut()
            .flat_map(|routine| &mut routine.blocks)
            .flat_map(|block| &mut block.ops)
        {
            let place = match op {
                NirOp::Load { place, .. }
                | NirOp::VolatileLoad { place, .. }
                | NirOp::Store { place, .. }
                | NirOp::VolatileStore { place, .. } => Some(place),
                _ => None,
            };
            if let Some(NirPlace {
                kind: NirPlaceKind::Field { offset, ty, .. },
                ..
            }) = place
                && ty.width == Some(ByteSize::new(2))
            {
                *offset = ByteOffset::new(1);
                break;
            }
        }
    }

    fn add_address_relocation_probe(program: &mut NirProgram) {
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
        program.statics.push(NirStaticData {
            id: SymbolId(0xF000_0002),
            name: "relocation_probe".to_string(),
            ty: crate::nir::NirType {
                kind: NirTypeKind::U8,
                summary: "relocation probe".to_string(),
                width: Some(ByteSize::ONE),
                pointer: false,
            },
            image: NirDataImage {
                bytes: vec![0; 8],
                fragments: vec![
                    NirDataFragment::Address {
                        offset: ByteOffset::ZERO,
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
                            width: ByteSize::new(4),
                        },
                        target: NirDataAddressTarget::Storage(NirStorageId::Global(data)),
                        addend: 0,
                        span: Span { start: 0, end: 0 },
                    },
                    NirDataFragment::Address {
                        offset: ByteOffset::new(4),
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: crate::target::TargetLayout::CODE_ADDRESS_SPACE,
                            width: ByteSize::new(4),
                        },
                        target: NirDataAddressTarget::Routine(crate::nir::RoutineId(code)),
                        addend: 0,
                        span: Span { start: 0, end: 0 },
                    },
                ],
            },
            display: String::new(),
            alignment: ByteSize::new(2),
            mutable: true,
            section: "data".to_string(),
        });
    }

    #[test]
    fn canary_lowers_big_endian_data_32_bit_addresses_and_odd_packed_words() {
        let mut nir = lower_source();
        force_one_packed_odd_word(&mut nir);
        add_address_relocation_probe(&mut nir);
        let mir = super::super::lower_program(&nir).expect("lower 68k canary");

        assert_eq!(mir.endian, Endian::Big);
        assert_eq!(mir.architectural_address_bits, 32);
        assert_eq!(mir.data_pointer_width, ByteSize::new(4));
        assert_eq!(mir.code_pointer_width, ByteSize::new(4));
        assert!(
            mir.data
                .iter()
                .any(|data| data.bytes.windows(2).any(|bytes| bytes == [0x23, 0x45]))
        );
        let probe = mir
            .data
            .iter()
            .find(|data| data.name == "relocation_probe")
            .expect("lower relocation probe");
        assert!(matches!(
            probe.relocations[0].target,
            Mir68kRelocationTarget::Data(_)
        ));
        assert!(matches!(
            probe.relocations[1].target,
            Mir68kRelocationTarget::Code(_)
        ));
        assert!(
            probe
                .relocations
                .iter()
                .all(|relocation| relocation.width == ByteSize::new(4))
        );

        let ops = mir
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .collect::<Vec<_>>();
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Load {
                address: Mir68kAddress {
                    mode: Mir68kAddressMode::AddressIndirect,
                    ..
                },
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Store {
                address: Mir68kAddress {
                    mode: Mir68kAddressMode::Absolute,
                    ..
                },
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Store {
                access: Mir68kAccess::BytewisePackedOddWord {
                    endian: Endian::Big
                },
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Load {
                access: Mir68kAccess::NativeAlignedWord,
                ..
            } | Mir68kOp::Store {
                access: Mir68kAccess::NativeAlignedWord,
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Copy {
                source: Mir68kAddress {
                    index: Some(Mir68kIndex { stride, .. }),
                    ..
                },
                bytes,
                ..
            } if *bytes == ByteSize::new(4) && *stride == ByteSize::new(4)
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Binary { width, .. } if *width == ByteSize::ONE
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Binary { width, .. } if *width == ByteSize::new(2)
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Call {
                target: Mir68kCallTarget::Direct(_),
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Mir68kOp::Call {
                target: Mir68kCallTarget::Indirect(_, width),
                ..
            } if *width == ByteSize::new(4)
        )));
        assert!(
            mir.routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .any(|block| { matches!(block.terminator, Mir68kTerminator::Branch { .. }) })
        );
        assert!(
            mir.routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .any(|block| { matches!(block.terminator, Mir68kTerminator::Return(Some(_))) })
        );
    }
}
