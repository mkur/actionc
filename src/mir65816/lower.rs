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

    let storage = crate::nir::analyze_program_storage(program);
    let mut routines = Vec::with_capacity(program.routines.len());
    for (routine, storage) in program.routines.iter().zip(&storage.routines) {
        let Some(frame) = plan_frame(routine, storage, &mut diagnostics) else {
            continue;
        };
        routines.push(lower_routine(
            program,
            routine,
            frame,
            convention,
            layout.code_pointer.size_bytes,
            &mut diagnostics,
        ));
    }

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
        task_switch_state: Mir65816TaskSwitchState {
            required: vec![
                Mir65816SavedState::Accumulator,
                Mir65816SavedState::X,
                Mir65816SavedState::Y,
                Mir65816SavedState::StackPointer,
                Mir65816SavedState::DirectPage,
                Mir65816SavedState::DataBank,
                Mir65816SavedState::ProgramBank,
                Mir65816SavedState::ProcessorStatus,
            ],
        },
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

fn lower_routine(
    program: &NirProgram,
    routine: &NirRoutine,
    frame: Mir65816FramePlan,
    convention: Mir65816CallConvention,
    code_pointer_width: ByteSize,
    diagnostics: &mut Vec<Mir65816Diagnostic>,
) -> Mir65816Routine {
    let layout = &program.target_layout;
    let mode = boundary_mode();
    let return_form = return_form(convention);
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
                        code_pointer_width,
                        convention,
                        program,
                        routine,
                        &frame,
                        &routine.name,
                        &block.label,
                        diagnostics,
                    )
                })
                .collect();
            let terminator = lower_terminator(
                &block.terminator,
                layout.data_pointer.size_bytes,
                code_pointer_width,
                &routine.name,
                &block.label,
                frame.extent,
                return_form,
                mode,
                diagnostics,
            );
            Mir65816Block {
                id: block.id,
                ops,
                terminator,
            }
        })
        .collect();
    let parameter_copies = frame
        .parameters
        .iter()
        .filter_map(|parameter| {
            parameter
                .frame_object
                .map(|object| (parameter.incoming, object))
        })
        .collect();
    let prologue = Mir65816ProloguePlan {
        required_mode: mode,
        reserve_bytes: frame.extent,
        parameter_copies,
    };
    let epilogue = Mir65816EpiloguePlan {
        restored_mode: mode,
        release_bytes: frame.extent,
        return_form,
    };
    let lowered = Mir65816Routine {
        id: routine.id,
        name: routine.name.clone(),
        convention: routine.convention,
        frame,
        prologue,
        epilogue,
        blocks,
    };
    if let Err(message) = verify_routine_plan(&lowered, code_pointer_width) {
        diagnostics.push(diagnostic(Some(&routine.name), None, message));
    }
    lowered
}

fn boundary_mode() -> Mir65816ModeState {
    Mir65816ModeState {
        native_mode: true,
        accumulator: Mir65816RegisterWidth::Bits16,
        index: Mir65816RegisterWidth::Bits16,
    }
}

fn call_form(convention: Mir65816CallConvention) -> Mir65816CallForm {
    match convention {
        Mir65816CallConvention::Native => Mir65816CallForm::FarJsl,
        Mir65816CallConvention::Small => Mir65816CallForm::NearJsr,
    }
}

fn return_form(convention: Mir65816CallConvention) -> Mir65816ReturnForm {
    match convention {
        Mir65816CallConvention::Native => Mir65816ReturnForm::FarRtl,
        Mir65816CallConvention::Small => Mir65816ReturnForm::NearRts,
    }
}

fn plan_frame(
    routine: &NirRoutine,
    storage: &NirRoutineStorageAnalysis,
    diagnostics: &mut Vec<Mir65816Diagnostic>,
) -> Option<Mir65816FramePlan> {
    // S points immediately below the reserved activation, so the first
    // addressable byte is at displacement one.
    let mut cursor = 1u32;
    let mut objects = Vec::new();
    let mut parameters = Vec::with_capacity(routine.params.len());
    let incoming = abi_stack_homes(
        &routine
            .params
            .iter()
            .map(|param| param.layout.size)
            .collect::<Vec<_>>(),
    )?;

    for (param, incoming) in routine.params.iter().zip(incoming) {
        let facts = storage.homes.get(&NirStorageId::Param(param.id));
        let needs_frame = facts
            .is_some_and(|facts| facts.direct_stores != 0 || facts.requires_addressable_home());
        let frame_object = if needs_frame {
            match allocate_frame_object(
                &mut cursor,
                &mut objects,
                Mir65816FrameObjectOwner::Param(param.id),
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
        parameters.push(Mir65816ParameterPlan {
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
            Mir65816FrameObjectOwner::Local(local.id),
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

    let automatic_bytes = ByteSize::new(cursor - 1);
    let outgoing_bytes = match max_outgoing_bytes(routine) {
        Some(bytes) => bytes,
        None => {
            diagnostics.push(diagnostic(
                Some(&routine.name),
                None,
                "65816 outgoing argument area overflows frame planning",
            ));
            return None;
        }
    };
    let outgoing_offset = ByteOffset::new(cursor);
    let Some(end) = cursor.checked_add(outgoing_bytes.get()) else {
        diagnostics.push(diagnostic(
            Some(&routine.name),
            None,
            "65816 automatic and outgoing areas overflow frame planning",
        ));
        return None;
    };
    let extent = end - 1;
    if extent > u32::from(u8::MAX) {
        diagnostics.push(diagnostic(
            Some(&routine.name),
            None,
            format!(
                "65816 hardware-stack frame requires {extent} bytes; the initial stack-relative strategy supports at most 255"
            ),
        ));
        return None;
    }

    Some(Mir65816FramePlan {
        strategy: Mir65816FrameStrategy::HardwareStackRelative,
        bank: 0,
        objects,
        parameters,
        automatic_bytes,
        saved_state_bytes: ByteSize::ZERO,
        spill_bytes: ByteSize::ZERO,
        outgoing_offset,
        outgoing_bytes,
        extent: ByteSize::new(extent),
    })
}

#[allow(clippy::too_many_arguments)]
fn allocate_frame_object(
    cursor: &mut u32,
    objects: &mut Vec<Mir65816FrameObject>,
    owner: Mir65816FrameObjectOwner,
    size: ByteSize,
    alignment: ByteSize,
    mutable: bool,
    addressable: bool,
) -> Result<Mir65816FrameObjectId, &'static str> {
    let aligned = align_up(*cursor, alignment.get())
        .ok_or("65816 automatic object alignment overflows frame planning")?;
    let end = aligned
        .checked_add(size.get())
        .ok_or("65816 automatic object size overflows frame planning")?;
    if aligned > u32::from(u8::MAX) || end.saturating_sub(1) > u32::from(u8::MAX) {
        return Err(
            "65816 automatic object does not fit the initial 8-bit stack-relative displacement range",
        );
    }
    let id = Mir65816FrameObjectId(
        u32::try_from(objects.len()).map_err(|_| "too many 65816 frame objects")?,
    );
    objects.push(Mir65816FrameObject {
        id,
        owner,
        size,
        alignment,
        stack_offset: ByteOffset::new(aligned),
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

fn abi_stack_homes(sizes: &[ByteSize]) -> Option<Vec<Mir65816AbiHome>> {
    let mut offset = 0u32;
    let mut homes = Vec::with_capacity(sizes.len());
    for size in sizes {
        homes.push(Mir65816AbiHome::StackArgument {
            offset: ByteOffset::new(offset),
            size: *size,
        });
        offset = offset.checked_add(size.get())?;
    }
    Some(homes)
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
        let bytes = call_argument_sizes(signature, args.len())?
            .into_iter()
            .try_fold(0u32, |total, size| total.checked_add(size.get()))?;
        maximum = maximum.max(bytes);
    }
    Some(ByteSize::new(maximum))
}

fn call_plan(
    signature: &crate::nir::NirCallableSignature,
    argument_count: usize,
    result: Option<&crate::nir::NirCallResult>,
    convention: Mir65816CallConvention,
    code_pointer_width: ByteSize,
) -> Option<Mir65816CallPlan> {
    let arguments = abi_stack_homes(&call_argument_sizes(signature, argument_count)?)?;
    let outgoing_bytes = arguments
        .last()
        .map(|home| match home {
            Mir65816AbiHome::StackArgument { offset, size } => {
                ByteSize::new(offset.get() + size.get())
            }
            Mir65816AbiHome::Accumulator | Mir65816AbiHome::AccumulatorAndX => ByteSize::ZERO,
        })
        .unwrap_or(ByteSize::ZERO);
    let result = result.map(|result| {
        if result.ty.width.is_some_and(|width| width.get() > 2)
            || matches!(result.ty.kind, NirTypeKind::Pointer { .. })
        {
            Mir65816AbiHome::AccumulatorAndX
        } else {
            Mir65816AbiHome::Accumulator
        }
    });
    let mode = boundary_mode();
    Some(Mir65816CallPlan {
        convention: signature.convention,
        arguments,
        result,
        outgoing_bytes,
        code_pointer_width,
        call_form: call_form(convention),
        mode_before: mode,
        mode_after: mode,
        activation: Mir65816CallActivation::Fresh,
        net_stack_delta: 0,
    })
}

fn verify_routine_plan(
    routine: &Mir65816Routine,
    code_pointer_width: ByteSize,
) -> Result<(), String> {
    if routine.frame.bank != 0 {
        return Err("65816 hardware-stack frame must remain in bank zero".to_string());
    }
    if routine.frame.extent.get() > u32::from(u8::MAX) {
        return Err("65816 frame exceeds stack-relative displacement range".to_string());
    }
    if routine.prologue.reserve_bytes != routine.frame.extent
        || routine.epilogue.release_bytes != routine.frame.extent
    {
        return Err("65816 prologue and epilogue do not balance the frame extent".to_string());
    }
    for object in &routine.frame.objects {
        let end = object
            .stack_offset
            .get()
            .checked_add(object.size.get())
            .ok_or_else(|| "65816 frame object extent overflowed".to_string())?;
        if object.stack_offset.get() == 0
            || end.saturating_sub(1) > routine.frame.extent.get()
            || object.stack_offset.get() % object.alignment.get() != 0
        {
            return Err(format!(
                "65816 frame object {} has an invalid stack-relative extent",
                object.id.0
            ));
        }
    }
    for block in &routine.blocks {
        for op in &block.ops {
            if let Mir65816Op::Call { target, plan, .. } = op {
                if plan.net_stack_delta != 0 || plan.outgoing_bytes > routine.frame.outgoing_bytes {
                    return Err(
                        "65816 call has an unbalanced or oversized outgoing area".to_string()
                    );
                }
                if plan.code_pointer_width != code_pointer_width {
                    return Err("65816 call uses the wrong code-pointer width".to_string());
                }
                if let Mir65816CallTarget::Indirect(_, width) = target
                    && *width != code_pointer_width
                {
                    return Err(
                        "65816 indirect call width does not match the memory model".to_string()
                    );
                }
                if plan.mode_before != boundary_mode() || plan.mode_after != boundary_mode() {
                    return Err("65816 call does not preserve the M/X boundary state".to_string());
                }
            }
        }
        if let Mir65816Terminator::Return {
            release_frame_bytes,
            form,
            restored_mode,
            ..
        } = block.terminator
            && (release_frame_bytes != routine.frame.extent
                || form != routine.epilogue.return_form
                || restored_mode != routine.epilogue.restored_mode)
        {
            return Err("65816 return does not match the routine epilogue".to_string());
        }
    }
    Ok(())
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
    frame: &Mir65816FramePlan,
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
                    frame,
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
                frame,
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
                    frame,
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
                frame,
            ),
            source: lower_place(
                source,
                data_pointer_width,
                code_pointer_width,
                program,
                nir_routine,
                frame,
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
        } => {
            let Some(signature) = signature.as_ref() else {
                diagnostics.push(diagnostic(
                    Some(routine),
                    Some(block),
                    "65816 call has no verified signature",
                ));
                return None;
            };
            let Some(plan) = call_plan(
                signature,
                args.len(),
                result.as_ref(),
                convention,
                code_pointer_width,
            ) else {
                diagnostics.push(diagnostic(
                    Some(routine),
                    Some(block),
                    "65816 call argument layout exceeds the supported outgoing area",
                ));
                return None;
            };
            Some(Mir65816Op::Call {
                target: lower_callee(callee, data_pointer_width, code_pointer_width),
                signature: Some(signature.id),
                args: args
                    .iter()
                    .map(|value| lower_value(value, data_pointer_width, code_pointer_width))
                    .collect(),
                result: result
                    .as_ref()
                    .map(|result| (result.dest, width(&result.ty))),
                convention,
                plan,
            })
        }
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
    frame: &Mir65816FramePlan,
) -> Mir65816Address {
    match &place.kind {
        NirPlaceKind::Param { id, .. } => {
            let base = frame
                .parameters
                .iter()
                .find(|parameter| parameter.param == *id)
                .and_then(|parameter| parameter.frame_object)
                .map(Mir65816AddressBase::AutomaticFrame)
                .unwrap_or(Mir65816AddressBase::Parameter(*id));
            direct(base)
        }
        NirPlaceKind::Local { id, .. } => lower_local(*id, program, routine, frame),
        NirPlaceKind::Global { id, .. } => lower_global(*id, program),
        NirPlaceKind::Absolute(address) => Mir65816Address {
            base: Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(*address)),
            displacement: ByteOffset::ZERO,
            index: None,
            mode: Mir65816AddressMode::External,
        },
        NirPlaceKind::Deref { addr } => Mir65816Address {
            base: Mir65816AddressBase::Indirect(lower_value(
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
            base: Mir65816AddressBase::Indirect(lower_value(
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
                frame,
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
    frame: &Mir65816FramePlan,
) -> Mir65816Address {
    let Some(local) = routine.locals.iter().find(|local| local.id == id) else {
        return direct(Mir65816AddressBase::Static(NirStorageId::Local(id)));
    };
    match &local.backing {
        NirLocalBacking::Ordinary => {
            let base = frame
                .objects
                .iter()
                .find(|object| object.owner == Mir65816FrameObjectOwner::Local(id))
                .map(|object| Mir65816AddressBase::AutomaticFrame(object.id))
                .unwrap_or(Mir65816AddressBase::Static(NirStorageId::Local(id)));
            direct(base)
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

fn lower_global(id: crate::nir::SymbolId, program: &NirProgram) -> Mir65816Address {
    let Some(global) = program.globals.iter().find(|global| global.id == id) else {
        return direct(Mir65816AddressBase::Static(NirStorageId::Global(id)));
    };
    match &global.backing {
        NirGlobalBacking::Ordinary => direct(Mir65816AddressBase::Static(NirStorageId::Global(id))),
        NirGlobalBacking::Absolute(address) => absolute(*address),
        NirGlobalBacking::Alias { target, offset } => {
            with_displacement(lower_global(*target, program), *offset)
        }
    }
}

fn absolute(address: crate::target::AddressValue) -> Mir65816Address {
    Mir65816Address {
        base: Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(address)),
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir65816AddressMode::External,
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
    let mode = match base {
        Mir65816AddressBase::Static(_) => Mir65816AddressMode::Static,
        Mir65816AddressBase::AutomaticFrame(_) => Mir65816AddressMode::AutomaticFrame,
        Mir65816AddressBase::Parameter(_) => Mir65816AddressMode::Parameter,
        Mir65816AddressBase::External(_) => Mir65816AddressMode::External,
        Mir65816AddressBase::Indirect(_) => Mir65816AddressMode::LongIndirect,
    };
    Mir65816Address {
        base,
        displacement: ByteOffset::ZERO,
        index: None,
        mode,
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
    frame_extent: ByteSize,
    return_form: Mir65816ReturnForm,
    boundary_mode: Mir65816ModeState,
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
        NirTerminator::Return(value) => Mir65816Terminator::Return {
            value: value
                .as_ref()
                .map(|value| lower_value(value, data_pointer_width, code_pointer_width)),
            release_frame_bytes: frame_extent,
            form: return_form,
            restored_mode: boundary_mode,
        },
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
                    mode: Mir65816AddressMode::External,
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
                .any(|block| {
                    matches!(
                        block.terminator,
                        Mir65816Terminator::Return { value: Some(_), .. }
                    )
                })
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

    #[test]
    fn oversized_automatic_frame_is_diagnosed_by_the_initial_strategy() {
        let source = r#"
PROC Main()
  BYTE ARRAY bytes(300)
  bytes(0)=1
RETURN
"#;
        let tokens = crate::lexer::tokenize(source).expect("tokenize oversized frame");
        let source = crate::parser::parse(&tokens).expect("parse oversized frame");
        let model = analyze_with_options(
            &source,
            SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
        )
        .expect("analyze oversized frame");
        let semir = crate::semantic::ir::lower_program(&source, &model);
        let nir = crate::nir::lower_program(&semir);
        let error = super::super::lower_program(&nir).expect_err("oversized frame");
        let crate::backend::BackendLoweringError::Backend(diagnostics) = error else {
            panic!("expected backend frame diagnostic")
        };
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("initial 8-bit stack-relative displacement range")
                || diagnostic
                    .message
                    .contains("initial stack-relative strategy supports at most 255")
        }));
    }
}
