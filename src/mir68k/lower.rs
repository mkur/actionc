use super::*;
use crate::backend::VerifiedNir;
use crate::nir::{
    NirCallee, NirDataAddressEncoding, NirDataFragment, NirDataImage, NirGlobalBacking,
    NirGlobalInit, NirLinkValue, NirLocalBacking, NirOp, NirPlace, NirPlaceKind, NirProgram,
    NirRoutine, NirRoutineStorageAnalysis, NirStorageClass, NirStorageDuration, NirStorageId,
    NirTerminator, NirType, NirTypeKind, NirValue,
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

    let storage = crate::nir::analyze_program_storage(program);
    let mut routines = Vec::with_capacity(program.routines.len());
    for (routine, storage) in program.routines.iter().zip(&storage.routines) {
        if matches!(
            routine.convention,
            crate::nir::NirCallConvention::External(_)
        ) {
            diagnostics.push(diagnostic(
                Some(&routine.name),
                None,
                "68k external-ABI routine entry requires a target adapter",
            ));
            continue;
        }
        let Some(frame) = plan_frame(routine, storage, &mut diagnostics) else {
            continue;
        };
        routines.push(lower_routine(program, routine, frame, &mut diagnostics));
    }
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
    frame: Mir68kFramePlan,
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
                .filter_map(|op| lower_op(op, program, routine, &frame, &block.label, diagnostics))
                .collect(),
            terminator: lower_terminator(
                &block.terminator,
                layout.data_pointer.size_bytes,
                layout.code_pointer.size_bytes,
                &routine.name,
                &block.label,
                frame.extent,
                diagnostics,
            ),
        })
        .collect();
    let parameter_copies = frame
        .parameters
        .iter()
        .filter_map(|parameter| {
            parameter
                .frame_object
                .map(|destination| Mir68kParameterCopy {
                    source: parameter.incoming,
                    destination,
                })
        })
        .collect();
    let prologue = Mir68kProloguePlan {
        frame_pointer: Mir68kRegister::A(6),
        link_bytes: frame.extent,
        saved_registers: Vec::new(),
        parameter_copies,
    };
    let epilogue = Mir68kEpiloguePlan {
        frame_pointer: Mir68kRegister::A(6),
        unlink_bytes: frame.extent,
        restored_registers: Vec::new(),
    };
    let lowered = Mir68kRoutine {
        id: routine.id,
        name: routine.name.clone(),
        convention: routine.convention,
        frame,
        prologue,
        epilogue,
        blocks,
    };
    if let Err(message) = verify_routine_plan(&lowered) {
        diagnostics.push(diagnostic(Some(&routine.name), None, message));
    }
    lowered
}

fn plan_frame(
    routine: &NirRoutine,
    storage: &NirRoutineStorageAnalysis,
    diagnostics: &mut Vec<Mir68kDiagnostic>,
) -> Option<Mir68kFramePlan> {
    let mut cursor = 0u32;
    let mut objects = Vec::new();
    let mut parameters = Vec::with_capacity(routine.params.len());
    let parameter_sizes = routine
        .params
        .iter()
        .map(|param| param.layout.size)
        .collect::<Vec<_>>();
    let incoming = abi_stack_homes(&parameter_sizes)?;

    for (param, incoming) in routine.params.iter().zip(incoming) {
        let facts = storage.homes.get(&NirStorageId::Param(param.id));
        let needs_frame = facts
            .is_some_and(|facts| facts.direct_stores != 0 || facts.requires_addressable_home());
        let frame_object = if needs_frame {
            match allocate_frame_object(
                &mut cursor,
                &mut objects,
                Mir68kFrameObjectOwner::Param(param.id),
                param.layout.size,
                param.layout.alignment,
                facts.is_some_and(|facts| facts.direct_stores != 0),
                facts.is_some_and(|facts| facts.requires_addressable_home()),
            ) {
                Ok(id) => Some(id),
                Err(message) => {
                    diagnostics.push(diagnostic(Some(&routine.name), None, message));
                    return None;
                }
            }
        } else {
            None
        };
        parameters.push(Mir68kParameterPlan {
            param: param.id,
            incoming,
            frame_object,
        });
    }

    for local in &routine.locals {
        if local.duration != NirStorageDuration::Automatic
            || !matches!(local.backing, NirLocalBacking::Ordinary)
        {
            continue;
        }
        let facts = storage.homes.get(&NirStorageId::Local(local.id));
        if let Err(message) = allocate_frame_object(
            &mut cursor,
            &mut objects,
            Mir68kFrameObjectOwner::Local(local.id),
            local.layout.size,
            local.layout.alignment,
            facts.is_some_and(|facts| facts.direct_stores != 0),
            local.storage != NirStorageClass::Scalar
                || facts.is_some_and(|facts| facts.requires_addressable_home()),
        ) {
            diagnostics.push(diagnostic(Some(&routine.name), None, message));
            return None;
        }
    }

    let automatic_bytes = ByteSize::new(cursor);
    let outgoing_bytes = match max_outgoing_bytes(routine) {
        Some(bytes) => bytes,
        None => {
            diagnostics.push(diagnostic(
                Some(&routine.name),
                None,
                "68k outgoing argument area exceeds the supported frame size",
            ));
            return None;
        }
    };
    let Some(used) = cursor.checked_add(outgoing_bytes.get()) else {
        diagnostics.push(diagnostic(
            Some(&routine.name),
            None,
            "68k automatic and outgoing areas overflow the frame size",
        ));
        return None;
    };
    let Some(extent) = align_up(used, 2) else {
        diagnostics.push(diagnostic(
            Some(&routine.name),
            None,
            "68k frame extent cannot be even-aligned",
        ));
        return None;
    };
    let Ok(frame_offset) = i32::try_from(extent).map(|extent| -extent) else {
        diagnostics.push(diagnostic(
            Some(&routine.name),
            None,
            "68k frame extent exceeds signed displacement planning",
        ));
        return None;
    };

    Some(Mir68kFramePlan {
        objects,
        parameters,
        automatic_bytes,
        saved_register_bytes: ByteSize::ZERO,
        spill_bytes: ByteSize::ZERO,
        outgoing: Mir68kOutgoingArea {
            frame_offset,
            size: outgoing_bytes,
        },
        extent: ByteSize::new(extent),
    })
}

#[allow(clippy::too_many_arguments)]
fn allocate_frame_object(
    cursor: &mut u32,
    objects: &mut Vec<Mir68kFrameObject>,
    owner: Mir68kFrameObjectOwner,
    size: ByteSize,
    alignment: ByteSize,
    mutable: bool,
    addressable: bool,
) -> Result<Mir68kFrameObjectId, &'static str> {
    let end = cursor
        .checked_add(size.get())
        .and_then(|end| align_up(end, alignment.get()))
        .ok_or("68k automatic object layout overflows the frame size")?;
    let frame_offset = -i32::try_from(end)
        .map_err(|_| "68k automatic object exceeds signed frame displacement planning")?;
    let id = Mir68kFrameObjectId(
        u32::try_from(objects.len()).map_err(|_| "too many 68k frame objects")?,
    );
    objects.push(Mir68kFrameObject {
        id,
        owner,
        size,
        alignment,
        frame_offset,
        mutable,
        addressable,
    });
    *cursor = end;
    Ok(id)
}

fn align_up(value: u32, alignment: u32) -> Option<u32> {
    debug_assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment.checked_sub(1)?)
        .map(|value| value & !(alignment - 1))
}

fn abi_stack_homes(sizes: &[ByteSize]) -> Option<Vec<Mir68kAbiHome>> {
    let mut offset = 0u32;
    let mut homes = Vec::with_capacity(sizes.len());
    for size in sizes {
        offset = align_up(offset, 2)?;
        homes.push(Mir68kAbiHome::StackArgument {
            offset: ByteOffset::new(offset),
            size: *size,
        });
        offset = offset.checked_add(align_up(size.get(), 2)?)?;
    }
    Some(homes)
}

fn max_outgoing_bytes(routine: &NirRoutine) -> Option<ByteSize> {
    let mut maximum = 0u32;
    for op in routine.blocks.iter().flat_map(|block| &block.ops) {
        let NirOp::Call {
            args, signature, ..
        } = op
        else {
            continue;
        };
        let signature = signature.as_ref()?;
        let sizes = call_argument_sizes(signature, args.len())?;
        let homes = abi_stack_homes(&sizes)?;
        let bytes = homes
            .last()
            .map(|home| match home {
                Mir68kAbiHome::StackArgument { offset, size } => {
                    offset.get() + align_up(size.get(), 2).expect("valid ABI alignment")
                }
                Mir68kAbiHome::DataRegister(_) | Mir68kAbiHome::AddressRegister(_) => 0,
            })
            .unwrap_or(0);
        maximum = maximum.max(bytes);
    }
    Some(ByteSize::new(maximum))
}

fn call_argument_sizes(
    signature: &crate::nir::NirCallableSignature,
    count: usize,
) -> Option<Vec<ByteSize>> {
    (0..count)
        .map(|index| {
            signature
                .params
                .get(index)
                .or(signature.variadic.as_ref())?
                .width
        })
        .collect()
}

fn call_plan(
    signature: &crate::nir::NirCallableSignature,
    argument_count: usize,
    result: Option<&crate::nir::NirCallResult>,
) -> Option<Mir68kCallPlan> {
    let arguments = abi_stack_homes(&call_argument_sizes(signature, argument_count)?)?;
    let outgoing_bytes = arguments
        .last()
        .map(|home| match home {
            Mir68kAbiHome::StackArgument { offset, size } => {
                ByteSize::new(offset.get() + align_up(size.get(), 2).expect("valid ABI alignment"))
            }
            Mir68kAbiHome::DataRegister(_) | Mir68kAbiHome::AddressRegister(_) => ByteSize::ZERO,
        })
        .unwrap_or(ByteSize::ZERO);
    let result = result.map(|result| {
        if matches!(
            result.ty.kind,
            NirTypeKind::Pointer { .. } | NirTypeKind::Callable { .. }
        ) {
            Mir68kAbiHome::AddressRegister(0)
        } else {
            Mir68kAbiHome::DataRegister(0)
        }
    });
    Some(Mir68kCallPlan {
        convention: signature.convention,
        arguments,
        result,
        outgoing_bytes,
        activation: Mir68kCallActivation::Fresh,
        net_stack_delta: 0,
    })
}

fn verify_routine_plan(routine: &Mir68kRoutine) -> Result<(), String> {
    if routine.frame.extent.get() % 2 != 0 {
        return Err("68k frame extent is not even".to_string());
    }
    if routine.prologue.link_bytes != routine.frame.extent
        || routine.epilogue.unlink_bytes != routine.frame.extent
    {
        return Err("68k prologue and epilogue do not balance the frame extent".to_string());
    }
    let extent = i64::from(routine.frame.extent.get());
    for object in &routine.frame.objects {
        let start = i64::from(object.frame_offset);
        let end = start + i64::from(object.size.get());
        if start >= 0 || start < -extent || end > 0 {
            return Err(format!(
                "68k frame object {} is outside the planned frame",
                object.id.0
            ));
        }
        if object.frame_offset.unsigned_abs() % object.alignment.get() != 0 {
            return Err(format!(
                "68k frame object {} does not satisfy alignment {}",
                object.id.0, object.alignment
            ));
        }
    }
    for parameter in &routine.frame.parameters {
        if parameter
            .frame_object
            .is_some_and(|id| !routine.frame.objects.iter().any(|object| object.id == id))
        {
            return Err(format!(
                "68k parameter {} refers to a missing frame object",
                parameter.param.0
            ));
        }
    }
    for block in &routine.blocks {
        for op in &block.ops {
            if let Mir68kOp::Call { plan, .. } = op {
                if plan.net_stack_delta != 0 {
                    return Err("68k call plan leaves an unbalanced stack".to_string());
                }
                if plan.outgoing_bytes > routine.frame.outgoing.size {
                    return Err("68k call exceeds the routine outgoing area".to_string());
                }
            }
        }
        if let Mir68kTerminator::Return {
            restore_frame_bytes,
            ..
        } = block.terminator
            && restore_frame_bytes != routine.frame.extent
        {
            return Err("68k return does not restore the complete frame".to_string());
        }
    }
    Ok(())
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
    frame: &Mir68kFramePlan,
    block: &str,
    diagnostics: &mut Vec<Mir68kDiagnostic>,
) -> Option<Mir68kOp> {
    let data_width = program.target_layout.data_pointer.size_bytes;
    let code_width = program.target_layout.code_pointer.size_bytes;
    let width = |ty: &NirType| ty.width.expect("verified scalar NIR type has width");
    match op {
        NirOp::Load { dest, ty, place } | NirOp::VolatileLoad { dest, ty, place } => {
            let address = lower_place(place, data_width, code_width, program, routine, frame);
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
            address: lower_place(place, data_width, code_width, program, routine, frame),
            width: width(ty),
        }),
        NirOp::Store { place, src, ty } | NirOp::VolatileStore { place, src, ty } => {
            let address = lower_place(place, data_width, code_width, program, routine, frame);
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
            destination: lower_place(destination, data_width, code_width, program, routine, frame),
            source: lower_place(source, data_width, code_width, program, routine, frame),
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
        } => {
            let Some(signature) = signature.as_ref() else {
                diagnostics.push(diagnostic(
                    Some(&routine.name),
                    Some(block),
                    "68k call has no verified signature",
                ));
                return None;
            };
            if matches!(
                signature.convention,
                crate::nir::NirCallConvention::External(_)
            ) {
                diagnostics.push(diagnostic(
                    Some(&routine.name),
                    Some(block),
                    "68k external-ABI call requires a target adapter",
                ));
                return None;
            }
            let Some(plan) = call_plan(signature, args.len(), result.as_ref()) else {
                diagnostics.push(diagnostic(
                    Some(&routine.name),
                    Some(block),
                    "68k call argument layout exceeds the supported outgoing area",
                ));
                return None;
            };
            Some(Mir68kOp::Call {
                target: lower_callee(callee, data_width, code_width),
                signature: Some(signature.id),
                args: args
                    .iter()
                    .map(|value| lower_value(value, data_width, code_width))
                    .collect(),
                result: result
                    .as_ref()
                    .map(|result| (result.dest, width(&result.ty))),
                plan,
            })
        }
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
    frame: &Mir68kFramePlan,
) -> Mir68kAddress {
    match &place.kind {
        NirPlaceKind::Param { id, .. } => {
            let alignment = routine
                .params
                .iter()
                .find(|param| param.id == *id)
                .map(|param| param.layout.alignment);
            let base = frame
                .parameters
                .iter()
                .find(|parameter| parameter.param == *id)
                .and_then(|parameter| parameter.frame_object)
                .map(Mir68kAddressBase::AutomaticFrame)
                .unwrap_or(Mir68kAddressBase::Parameter(*id));
            direct(base, alignment)
        }
        NirPlaceKind::Local { id, .. } => lower_local(*id, program, routine, frame),
        NirPlaceKind::Global { id, .. } => lower_global(*id, program),
        NirPlaceKind::Absolute(address) => absolute(*address),
        NirPlaceKind::Deref { addr } => Mir68kAddress {
            base: Mir68kAddressBase::Indirect(lower_value(addr, data_width, code_width)),
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
            base: Mir68kAddressBase::Indirect(lower_value(base_addr, data_width, code_width)),
            base_alignment: None,
            displacement: ByteOffset::ZERO,
            index: Some(Mir68kIndex {
                value: lower_value(index, data_width, code_width),
                stride: *elem_size,
            }),
            mode: Mir68kAddressMode::Indexed,
        },
        NirPlaceKind::Field { base, offset, .. } => with_displacement(
            lower_place(base, data_width, code_width, program, routine, frame),
            *offset,
        ),
    }
}

fn lower_local(
    id: crate::nir::LocalId,
    program: &NirProgram,
    routine: &NirRoutine,
    frame: &Mir68kFramePlan,
) -> Mir68kAddress {
    let Some(local) = routine.locals.iter().find(|local| local.id == id) else {
        return direct(Mir68kAddressBase::Static(NirStorageId::Local(id)), None);
    };
    match &local.backing {
        NirLocalBacking::Ordinary => {
            let base = frame
                .objects
                .iter()
                .find(|object| object.owner == Mir68kFrameObjectOwner::Local(id))
                .map(|object| Mir68kAddressBase::AutomaticFrame(object.id))
                .unwrap_or(Mir68kAddressBase::Static(NirStorageId::Local(id)));
            direct(base, Some(local.layout.alignment))
        }
        NirLocalBacking::Absolute(address) => absolute(*address),
        NirLocalBacking::Alias { target, offset, .. } => {
            with_displacement(lower_local(*target, program, routine, frame), *offset)
        }
        NirLocalBacking::GlobalAlias { target, offset, .. } => {
            with_displacement(lower_global(*target, program), *offset)
        }
    }
}

fn lower_global(id: SymbolId, program: &NirProgram) -> Mir68kAddress {
    let Some(global) = program.globals.iter().find(|global| global.id == id) else {
        return direct(Mir68kAddressBase::Static(NirStorageId::Global(id)), None);
    };
    match &global.backing {
        NirGlobalBacking::Ordinary => direct(
            Mir68kAddressBase::Static(NirStorageId::Global(id)),
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
    let mode = match base {
        Mir68kAddressBase::Static(_) => Mir68kAddressMode::Static,
        Mir68kAddressBase::AutomaticFrame(_) => Mir68kAddressMode::AutomaticFrame,
        Mir68kAddressBase::Parameter(_) => Mir68kAddressMode::Parameter,
        Mir68kAddressBase::External(_) => Mir68kAddressMode::External,
        Mir68kAddressBase::Indirect(_) => Mir68kAddressMode::AddressIndirect,
    };
    Mir68kAddress {
        base,
        base_alignment,
        displacement: ByteOffset::ZERO,
        index: None,
        mode,
    }
}

fn absolute(address: AddressValue) -> Mir68kAddress {
    Mir68kAddress {
        base: Mir68kAddressBase::External(Mir68kExternalAddress::Absolute(address)),
        base_alignment: Some(if address.value % 2 == 0 {
            ByteSize::new(2)
        } else {
            ByteSize::ONE
        }),
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir68kAddressMode::External,
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
    frame_extent: ByteSize,
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
        NirTerminator::Return(value) => Mir68kTerminator::Return {
            value: value
                .as_ref()
                .map(|value| lower_value(value, data_width, code_width)),
            restore_frame_bytes: frame_extent,
        },
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
                    mode: Mir68kAddressMode::External,
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
                .any(|block| {
                    matches!(
                        block.terminator,
                        Mir68kTerminator::Return { value: Some(_), .. }
                    )
                })
        );
    }
}
