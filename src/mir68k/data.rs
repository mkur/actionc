//! Explicit allocation and initialization facts, projected only from NIR.
use super::*;
use crate::nir::{
    NirDataAddressEncoding, NirDataFragment, NirDataImage, NirGlobalBacking, NirGlobalInit,
    NirLocalBacking, NirProgram, NirStorageDuration, NirStorageInit,
};

pub(super) fn lower(program: &NirProgram, errors: &mut Vec<Mir68kDiagnostic>) -> Vec<Mir68kData> {
    let mut data = Vec::new();
    let layout = &program.target_layout;
    for g in &program.globals {
        if g.storage_size.is_zero() {
            continue;
        }
        let placement = match g.backing {
            NirGlobalBacking::Ordinary => Mir68kDataPlacement::Allocate,
            NirGlobalBacking::Absolute(a) => Mir68kDataPlacement::Absolute(a),
            NirGlobalBacking::Alias { target, offset } => Mir68kDataPlacement::Alias {
                target: Mir68kDataId::Global(target),
                offset,
            },
        };
        let alignment = if g.array.is_some() {
            ByteSize::new(2)
        } else {
            g.ty.as_ref()
                .map(|t| super::lower::type_alignment(t, layout))
                .unwrap_or(ByteSize::ONE)
        };
        let mut item = empty(
            Mir68kDataId::Global(g.id),
            g.name.clone(),
            g.storage_size,
            alignment,
        );
        item.placement = placement;
        item.ty = g.ty.clone();
        item.array = g.array.clone();
        if !matches!(placement, Mir68kDataPlacement::Allocate) {
            item.zero_fill = ByteSize::ZERO;
        }
        match &g.init {
            Some(NirGlobalInit::Bytes {
                image,
                zero_fill,
                mutable,
                ..
            }) => {
                project(&mut item, image);
                item.zero_fill = *zero_fill;
                item.mutable = *mutable;
            }
            Some(NirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                mutable,
                ..
            }) => {
                let backing_size = u32::try_from(backing.image.bytes.len())
                    .ok()
                    .and_then(|n| n.checked_add(backing.zero_fill.get()));
                let Some(backing_size) = backing_size else {
                    errors.push(error(&g.name, "array backing extent overflow"));
                    continue;
                };
                let mut elements = empty(
                    Mir68kDataId::ArrayBacking(g.id),
                    format!("{}.__backing", g.name),
                    ByteSize::new(backing_size),
                    alignment,
                );
                project(&mut elements, &backing.image);
                elements.zero_fill = backing.zero_fill;
                elements.mutable = *mutable;
                elements.array = g.array.clone();
                elements.ty = g.ty.clone();
                data.push(elements);
                item.bytes =
                    descriptor(*descriptor_size, *size_word, layout.data_pointer.size_bytes);
                item.zero_fill = ByteSize::ZERO;
                item.mutable = *mutable;
                item.relocations.push(pointer_relocation(
                    Mir68kRelocationTarget::ArrayBacking(g.id),
                    layout.data_pointer.address_space,
                    layout.data_pointer.size_bytes,
                ));
            }
            Some(NirGlobalInit::RoutineAddress {
                routine,
                descriptor_size,
                size_word,
                mutable,
                ..
            }) => {
                item.bytes =
                    descriptor(*descriptor_size, *size_word, layout.code_pointer.size_bytes);
                item.zero_fill = ByteSize::ZERO;
                item.mutable = *mutable;
                item.relocations.push(pointer_relocation(
                    Mir68kRelocationTarget::Code(*routine),
                    layout.code_pointer.address_space,
                    layout.code_pointer.size_bytes,
                ));
            }
            Some(NirGlobalInit::LinkValue { width, mutable, .. }) => {
                item.bytes = vec![0; width.get() as usize];
                item.zero_fill = ByteSize::ZERO;
                item.mutable = *mutable;
                item.relocations.push(pointer_relocation(
                    Mir68kRelocationTarget::ImageEnd,
                    layout.data_pointer.address_space,
                    *width,
                ));
            }
            Some(NirGlobalInit::ZeroFill { bytes, mutable, .. }) => {
                item.size = *bytes;
                item.zero_fill = *bytes;
                item.mutable = *mutable;
            }
            None => {}
        }
        data.push(item);
    }
    for s in &program.statics {
        let mut item = empty(
            Mir68kDataId::Static(s.id),
            s.name.clone(),
            ByteSize::new(s.image.bytes.len() as u32),
            s.alignment,
        );
        project(&mut item, &s.image);
        item.zero_fill = ByteSize::ZERO;
        item.mutable = s.mutable;
        item.ty = Some(s.ty.clone());
        data.push(item);
    }
    for r in &program.routines {
        for l in &r.locals {
            if l.duration == NirStorageDuration::Automatic
                || l.layout.size.is_zero()
                || !matches!(l.backing, NirLocalBacking::Ordinary)
            {
                continue;
            }
            let mut item = empty(
                Mir68kDataId::Local(r.id, l.id),
                format!("{}::{}", r.name, l.name),
                l.layout.size,
                l.layout.alignment,
            );
            item.ty = Some(l.ty.clone());
            match &l.init {
                Some(NirStorageInit::Bytes {
                    image,
                    zero_fill,
                    mutable,
                    ..
                }) => {
                    project(&mut item, image);
                    item.zero_fill = *zero_fill;
                    item.mutable = *mutable;
                }
                Some(NirStorageInit::ZeroFill { bytes, mutable, .. }) => {
                    item.zero_fill = *bytes;
                    item.mutable = *mutable;
                }
                Some(NirStorageInit::Descriptor { .. }) => {
                    errors.push(error(
                        &item.name,
                        "routine-static array descriptor requires an allocation adapter",
                    ));
                }
                None => {}
            }
            data.push(item);
        }
    }
    // Static array address constants name the initial element storage. The
    // descriptor cell is used by executable loads/stores, not these constants.
    for item in &mut data {
        for relocation in &mut item.relocations {
            if let Mir68kRelocationTarget::Data(NirStorageId::Global(id)) = relocation.target
                && let Some(global) = program.globals.iter().find(|g| g.id == id)
                && let Some(array) = &global.array
            {
                if matches!(global.init, Some(NirGlobalInit::Descriptor { .. })) {
                    relocation.target = Mir68kRelocationTarget::ArrayBacking(id);
                } else if let Some(address) = array.address_initializer {
                    relocation.target = Mir68kRelocationTarget::Absolute(address);
                }
            }
        }
    }
    data
}

fn empty(id: Mir68kDataId, name: String, size: ByteSize, alignment: ByteSize) -> Mir68kData {
    Mir68kData {
        id,
        name,
        size,
        zero_fill: size,
        alignment,
        placement: Mir68kDataPlacement::Allocate,
        mutable: true,
        ty: None,
        array: None,
        bytes: Vec::new(),
        relocations: Vec::new(),
    }
}

fn descriptor(size: ByteSize, size_word: Option<u16>, pointer_width: ByteSize) -> Vec<u8> {
    let mut bytes = vec![0; size.get() as usize];
    if let Some(word) = size_word {
        let offset = pointer_width.get() as usize;
        bytes[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
    }
    bytes
}

fn pointer_relocation(
    target: Mir68kRelocationTarget,
    address_space: AddressSpaceId,
    width: ByteSize,
) -> Mir68kRelocation {
    Mir68kRelocation {
        offset: ByteOffset::ZERO,
        byte_index: None,
        width,
        address_space,
        target,
        addend: 0,
    }
}

fn project(item: &mut Mir68kData, image: &NirDataImage) {
    item.bytes = image
        .project_constants(Endian::Big)
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
                    if matches!(target, NirDataAddressTarget::Routine(_)) {
                        crate::target::TargetLayout::CODE_ADDRESS_SPACE
                    } else {
                        crate::target::TargetLayout::DATA_ADDRESS_SPACE
                    },
                    Some(byte_index),
                ),
            };
            item.relocations.push(Mir68kRelocation {
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

fn error(name: &str, message: &str) -> Mir68kDiagnostic {
    Mir68kDiagnostic {
        routine: None,
        block: None,
        message: format!("{name}: {message}"),
    }
}
