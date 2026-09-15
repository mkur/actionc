//! Target data placement and initialization, derived exclusively from verified NIR.
use super::*;
use crate::nir::{
    NirDataAddressEncoding, NirDataFragment, NirDataImage, NirGlobalBacking, NirGlobalInit,
    NirLinkValue, NirProgram, NirTypeKind,
};
use crate::target::TargetLayout;
use std::collections::BTreeSet;

pub(super) fn lower(
    program: &NirProgram,
    errors: &mut Vec<Mir65816Diagnostic>,
) -> Vec<Mir65816Data> {
    let layout = &program.target_layout;
    let mut data = Vec::new();
    for global in &program.globals {
        // Includes and type declarations have identities but allocate no storage.
        if global.storage_size.is_zero() {
            continue;
        }
        let placement = match global.backing {
            NirGlobalBacking::Ordinary => Mir65816DataPlacement::Allocate,
            NirGlobalBacking::Absolute(address) => Mir65816DataPlacement::Absolute(address),
            NirGlobalBacking::Alias { target, offset } => Mir65816DataPlacement::Alias {
                target: Mir65816DataId::Global(target),
                offset,
            },
        };
        let element_alignment = global
            .ty
            .as_ref()
            .map_or(ByteSize::ONE, |ty| type_alignment(&ty.kind, layout));
        let alignment = if global.array.as_ref().is_some_and(|a| a.pointer_backed) {
            layout.data_pointer.alignment_bytes
        } else {
            element_alignment
        };
        let mut item = empty(
            Mir65816DataId::Global(global.id),
            global.name.clone(),
            global.storage_size,
            alignment,
        );
        item.placement = placement;
        item.ty = global.ty.clone();
        item.array = global.array.clone();
        if placement != Mir65816DataPlacement::Allocate {
            // An external address or alias does not implicitly initialize memory.
            item.zero_fill = ByteSize::ZERO;
        }
        match &global.init {
            Some(NirGlobalInit::Bytes {
                image,
                zero_fill,
                mutable,
                section,
            }) => {
                project(&mut item, image);
                item.zero_fill = *zero_fill;
                item.mutable = *mutable;
                item.section = Some(section.clone());
            }
            Some(NirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                mutable,
                section,
            }) => {
                let Some(backing_size) = ByteSize::try_from(backing.image.bytes.len())
                    .ok()
                    .and_then(|size| size.checked_add(backing.zero_fill))
                else {
                    errors.push(error(&item.name, "array backing extent overflow"));
                    continue;
                };
                let Some(bytes) =
                    descriptor(*descriptor_size, *size_word, layout.data_pointer.size_bytes)
                else {
                    errors.push(error(&item.name, "invalid data descriptor layout"));
                    continue;
                };
                let mut elements = empty(
                    Mir65816DataId::ArrayBacking(global.id),
                    format!("{}.__backing", global.name),
                    backing_size,
                    element_alignment,
                );
                project(&mut elements, &backing.image);
                elements.zero_fill = backing.zero_fill;
                elements.mutable = *mutable;
                elements.section = Some(backing.section.clone());
                elements.ty = global.ty.clone();
                elements.array = global.array.clone();
                data.push(elements);
                item.bytes = bytes;
                item.zero_fill = ByteSize::ZERO;
                item.mutable = *mutable;
                item.section = Some(section.clone());
                item.relocations.push(pointer_relocation(
                    Mir65816RelocationTarget::ArrayBacking(global.id),
                    layout.data_pointer.address_space,
                    layout.data_pointer.size_bytes,
                ));
            }
            Some(NirGlobalInit::RoutineAddress {
                routine,
                descriptor_size,
                size_word,
                mutable,
                section,
            }) => {
                let Some(bytes) =
                    descriptor(*descriptor_size, *size_word, layout.code_pointer.size_bytes)
                else {
                    errors.push(error(&item.name, "invalid routine descriptor layout"));
                    continue;
                };
                item.bytes = bytes;
                item.zero_fill = ByteSize::ZERO;
                item.mutable = *mutable;
                item.section = Some(section.clone());
                item.relocations.push(pointer_relocation(
                    Mir65816RelocationTarget::Code(*routine),
                    layout.code_pointer.address_space,
                    layout.code_pointer.size_bytes,
                ));
            }
            Some(NirGlobalInit::LinkValue {
                value: NirLinkValue::ImageEndAddress,
                width,
                mutable,
                section,
            }) => {
                item.bytes = vec![0; usize::from(*width)];
                item.zero_fill = ByteSize::new(item.size.get() - width.get());
                item.mutable = *mutable;
                item.section = Some(section.clone());
                item.relocations.push(pointer_relocation(
                    Mir65816RelocationTarget::ImageEnd,
                    layout.data_pointer.address_space,
                    *width,
                ));
            }
            Some(NirGlobalInit::ZeroFill {
                bytes,
                mutable,
                section,
            }) => {
                item.size = *bytes;
                item.zero_fill = *bytes;
                item.mutable = *mutable;
                item.section = Some(section.clone());
            }
            None => {}
        }
        data.push(item);
    }
    for static_data in &program.statics {
        let mut item = empty(
            Mir65816DataId::Static(static_data.id),
            static_data.name.clone(),
            ByteSize::try_from(static_data.image.bytes.len()).expect("verified static extent"),
            static_data.alignment,
        );
        project(&mut item, &static_data.image);
        item.zero_fill = ByteSize::ZERO;
        item.mutable = static_data.mutable;
        item.section = Some(static_data.section.clone());
        item.ty = Some(static_data.ty.clone());
        data.push(item);
    }

    // Initializer address constants name the array's elements. Executable
    // accesses to the descriptor cell continue to use its Global identity.
    for item in &mut data {
        for relocation in &mut item.relocations {
            if let Mir65816RelocationTarget::Data(NirStorageId::Global(id)) = relocation.target
                && let Some(global) = program.globals.iter().find(|g| g.id == id)
                && let Some(array) = &global.array
            {
                if matches!(global.init, Some(NirGlobalInit::Descriptor { .. })) {
                    relocation.target = Mir65816RelocationTarget::ArrayBacking(id);
                } else if let Some(address) = array.address_initializer {
                    relocation.target = Mir65816RelocationTarget::Absolute(address);
                }
            }
        }
    }
    validate(&data, errors);
    data
}

fn empty(id: Mir65816DataId, name: String, size: ByteSize, alignment: ByteSize) -> Mir65816Data {
    Mir65816Data {
        id,
        name,
        size,
        alignment,
        placement: Mir65816DataPlacement::Allocate,
        zero_fill: size,
        mutable: true,
        ty: None,
        array: None,
        section: None,
        bytes: Vec::new(),
        relocations: Vec::new(),
    }
}

fn type_alignment(kind: &NirTypeKind, layout: &TargetLayout) -> ByteSize {
    match kind {
        NirTypeKind::Integer(integer) => ByteSize::new(
            integer
                .storage_width()
                .get()
                .min(u32::from(layout.natural_word_alignment_bytes)),
        ),
        NirTypeKind::Pointer { .. } => layout.data_pointer.alignment_bytes,
        NirTypeKind::Callable { .. } => layout.code_pointer.alignment_bytes,
        // NIR carries record size but no field-alignment table. Word alignment
        // is a conservative placement for the selected native record policy.
        NirTypeKind::Record { .. } | NirTypeKind::Real => {
            ByteSize::from(layout.natural_word_alignment_bytes)
        }
        _ => ByteSize::ONE,
    }
}

fn descriptor(size: ByteSize, size_word: Option<u16>, pointer_width: ByteSize) -> Option<Vec<u8>> {
    if size < pointer_width {
        return None;
    }
    let mut bytes = vec![0; usize::from(size)];
    if let Some(word) = size_word {
        let offset = usize::from(pointer_width);
        bytes
            .get_mut(offset..offset + 2)?
            .copy_from_slice(&word.to_le_bytes());
    }
    Some(bytes)
}

fn pointer_relocation(
    target: Mir65816RelocationTarget,
    address_space: AddressSpaceId,
    width: ByteSize,
) -> Mir65816Relocation {
    Mir65816Relocation {
        offset: ByteOffset::ZERO,
        byte_index: None,
        width,
        address_space,
        target,
        addend: 0,
    }
}

fn project(item: &mut Mir65816Data, image: &NirDataImage) {
    item.bytes = image
        .project_constants(Endian::Little)
        .expect("verified data image");
    for fragment in &image.fragments {
        if let NirDataFragment::Address {
            offset,
            encoding,
            target,
            addend,
            ..
        } = fragment
        {
            let (width, address_space, byte_index) = match *encoding {
                NirDataAddressEncoding::Pointer {
                    width,
                    address_space,
                } => (width, address_space, None),
                NirDataAddressEncoding::TargetByte { byte_index, .. } => (
                    ByteSize::ONE,
                    match target {
                        NirDataAddressTarget::Routine(_) => TargetLayout::CODE_ADDRESS_SPACE,
                        NirDataAddressTarget::Storage(_) => TargetLayout::DATA_ADDRESS_SPACE,
                        NirDataAddressTarget::Absolute(address) => address.address_space,
                    },
                    Some(byte_index),
                ),
            };
            item.relocations.push(Mir65816Relocation {
                offset: *offset,
                width,
                address_space,
                byte_index,
                target: relocation_target(*target),
                addend: *addend,
            });
        }
    }
}

fn validate(data: &[Mir65816Data], errors: &mut Vec<Mir65816Diagnostic>) {
    let mut ids = BTreeSet::new();
    for item in data {
        if !ids.insert(item.id) {
            errors.push(error(&item.name, "duplicate data identity"));
        }
        let extent = item.bytes.len() as u64 + u64::from(item.zero_fill.get());
        if extent > u64::from(item.size.get())
            || item.placement == Mir65816DataPlacement::Allocate
                && extent != u64::from(item.size.get())
        {
            errors.push(error(
                &item.name,
                "initialization extent differs from storage extent",
            ));
        }
    }
    for item in data {
        for relocation in &item.relocations {
            let target = match relocation.target {
                Mir65816RelocationTarget::Data(NirStorageId::Global(id)) => {
                    Some(Mir65816DataId::Global(id))
                }
                Mir65816RelocationTarget::ArrayBacking(id) => {
                    Some(Mir65816DataId::ArrayBacking(id))
                }
                _ => None,
            };
            if target.is_some_and(|target| !ids.contains(&target)) {
                errors.push(error(&item.name, "relocation target has no storage"));
            }
        }
    }
}

fn error(name: &str, message: &str) -> Mir65816Diagnostic {
    Mir65816Diagnostic {
        routine: None,
        block: None,
        message: format!("{name}: {message}"),
    }
}
