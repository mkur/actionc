//! Relocatable MC68000 code/data and metadata, without executable compiler IR.
use super::{encode, image, machine::*, *};
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: u32 = 0x0100_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SectionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Section { id: SectionId, offset: u32 },
    Absolute(u32),
    Frame { routine: RoutineId, offset: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: SectionId,
    pub bytes: Vec<u8>,
    pub zero_fill: u32,
    pub size: u32,
    pub alignment: u32,
    pub writable: bool,
    pub executable: bool,
    /// Fixed storage is supported by the bare consumer only. An uninitialized
    /// external cell need not map any memory despite having a declared size.
    pub fixed_address: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationTarget {
    Location(Location),
    ImageEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    pub section: SectionId,
    pub offset: u32,
    pub width: u32,
    pub byte_index: Option<u8>,
    pub target: RelocationTarget,
    pub addend: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayInfo {
    pub element_width: u32,
    pub stride: u32,
    pub count: Option<u32>,
    pub descriptor: bool,
    pub backing: Option<Location>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub id: image::SymbolId,
    pub name: String,
    pub location: Location,
    pub size: u32,
    pub alignment: u32,
    pub ty: Option<crate::nir::NirType>,
    pub array: Option<ArrayInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub entry: Location,
    pub sections: Vec<Section>,
    pub relocations: Vec<Relocation>,
    pub symbols: Vec<Symbol>,
}

pub fn emit(mir: &Mir68kProgram, machine: &MachineProgram) -> Result<Object, String> {
    verify::verify_contract(mir).map_err(|e| format!("invalid MIR68K: {e:?}"))?;
    let code_id = SectionId(0);
    let code_location = |offset| Location::Section {
        id: code_id,
        offset,
    };
    let mut labels = BTreeMap::new();
    let mut size = 0u32;
    for block in &machine.blocks {
        if labels.insert(block.id, size).is_some() {
            return Err("duplicate machine block ID".into());
        }
        for op in &block.instructions {
            size = end(size, encode::size(op)?)?;
        }
    }
    let code_size = end(size, 4)?;
    let mut routines = BTreeMap::new();
    for routine in &machine.routines {
        let address = *labels
            .get(&routine.entry)
            .ok_or("routine entry has no machine block")?;
        if routines.insert(routine.id, address).is_some() {
            return Err("duplicate machine routine ID".into());
        }
    }
    if routines.len() != mir.routines.len()
        || mir.routines.iter().any(|r| !routines.contains_key(&r.id))
    {
        return Err("machine routine identities differ from MIR".into());
    }
    let entry = code_location(
        *routines
            .get(&mir.entry.ok_or("native image requires a program entry")?)
            .ok_or("entry routine was not emitted")?,
    );
    let mut object = Object {
        entry,
        sections: vec![Section {
            id: code_id,
            bytes: vec![],
            zero_fill: 0,
            size: code_size,
            alignment: 2,
            writable: false,
            executable: true,
            fixed_address: None,
        }],
        relocations: vec![],
        symbols: vec![],
    };
    let mut data = BTreeMap::new();
    let mut data_sections = BTreeMap::new();
    for item in &mir.data {
        let fixed_address = match item.placement {
            Mir68kDataPlacement::Allocate => None,
            Mir68kDataPlacement::Absolute(address) => {
                if address.address_space != crate::target::TargetLayout::DATA_ADDRESS_SPACE {
                    return Err("data uses another address space".into());
                }
                Some(physical(address.value)?)
            }
            Mir68kDataPlacement::Alias { .. } => continue,
        };
        let id = SectionId(object.sections.len() as u32);
        data_sections.insert(item.id, id);
        data.insert(
            item.id,
            fixed_address.map_or(Location::Section { id, offset: 0 }, Location::Absolute),
        );
        object.sections.push(Section {
            id,
            bytes: item.bytes.clone(),
            zero_fill: item.zero_fill.get(),
            size: item.size.get(),
            alignment: item.alignment.get(),
            writable: item.mutable,
            executable: false,
            fixed_address,
        });
    }
    let mut pending: Vec<_> = mir
        .data
        .iter()
        .filter(|d| matches!(d.placement, Mir68kDataPlacement::Alias { .. }))
        .collect();
    while !pending.is_empty() {
        let count = pending.len();
        let mut next = Vec::new();
        for item in pending {
            let Mir68kDataPlacement::Alias { target, offset } = item.placement else {
                unreachable!()
            };
            if let Some(location) = data.get(&target).copied() {
                data.insert(item.id, location.add_offset(offset.get())?);
            } else {
                next.push(item);
            }
        }
        if next.len() == count {
            return Err("unresolved data aliases".into());
        }
        pending = next;
    }
    let locate = |target: Target| -> Result<Location, String> {
        match target {
            Target::Block(id) => labels
                .get(&id)
                .copied()
                .map(code_location)
                .ok_or_else(|| "missing machine block relocation".into()),
            Target::Routine(id) => routines
                .get(&id)
                .copied()
                .map(code_location)
                .ok_or_else(|| "missing routine relocation".into()),
            Target::Data(id) => data
                .get(&id)
                .copied()
                .ok_or_else(|| "missing data relocation".into()),
            Target::Absolute(address) => Ok(Location::Absolute(physical(u64::from(address))?)),
        }
    };
    for instruction in machine.blocks.iter().flat_map(|b| &b.instructions) {
        if let Instruction::BranchRelative { target, .. } = instruction
            && !matches!(locate(target.target)?, Location::Section { id, .. } if id == code_id)
        {
            return Err("relative branch target is outside the code section".into());
        }
        let offset = object.sections[0].bytes.len() as u32;
        let encoded = encode::encode_relocatable(instruction, offset, &|address| {
            let value = match locate(address.target)? {
                Location::Section { id, offset } if id == code_id => offset,
                Location::Absolute(value) => value,
                _ => return Err("relative branch target is outside the code section".into()),
            };
            adjusted(value, address.addend)
        })?;
        for reloc in encoded.relocations {
            object.relocations.push(Relocation {
                section: code_id,
                offset: end(offset, reloc.offset)?,
                width: 4,
                byte_index: None,
                target: RelocationTarget::Location(locate(reloc.address.target)?),
                addend: reloc.address.addend,
            });
        }
        object.sections[0].bytes.extend(encoded.bytes);
    }
    object.sections[0].bytes.extend([0x4e, 0x71, 0x4e, 0x71]);
    for item in &mir.data {
        if !item.relocations.is_empty() && !data_sections.contains_key(&item.id) {
            return Err("alias cannot own initializer relocations".into());
        }
        for reloc in &item.relocations {
            let target = match reloc.target {
                Mir68kRelocationTarget::Data(NirStorageId::Global(id)) => {
                    RelocationTarget::Location(locate(Target::Data(Mir68kDataId::Global(id)))?)
                }
                Mir68kRelocationTarget::Data(_) => {
                    return Err("static relocation requires an invocation frame".into());
                }
                Mir68kRelocationTarget::Code(id) => {
                    RelocationTarget::Location(locate(Target::Routine(id))?)
                }
                Mir68kRelocationTarget::ArrayBacking(id) => RelocationTarget::Location(locate(
                    Target::Data(Mir68kDataId::ArrayBacking(id)),
                )?),
                Mir68kRelocationTarget::Absolute(a) => {
                    RelocationTarget::Location(Location::Absolute(physical(a.value)?))
                }
                Mir68kRelocationTarget::ImageEnd => RelocationTarget::ImageEnd,
            };
            object.relocations.push(Relocation {
                section: data_sections[&item.id],
                offset: reloc.offset.get(),
                width: reloc.width.get(),
                byte_index: reloc.byte_index,
                target,
                addend: reloc.addend,
            });
        }
        let location = data[&item.id];
        let array = item
            .array
            .as_ref()
            .map(|a| -> Result<ArrayInfo, String> {
                let backing = match item.id {
                    Mir68kDataId::Global(id) => data.get(&Mir68kDataId::ArrayBacking(id)).copied(),
                    _ => None,
                };
                let descriptor = backing.is_some() || a.pointer_backed;
                let initial = a
                    .address_initializer
                    .map(|a| physical(a.value).map(Location::Absolute))
                    .transpose()?;
                Ok(ArrayInfo {
                    element_width: a.elem_size.get(),
                    stride: a.elem_size.get(),
                    count: a.length,
                    descriptor: descriptor && !matches!(item.id, Mir68kDataId::ArrayBacking(_)),
                    backing: backing.or_else(|| {
                        if !descriptor || matches!(item.id, Mir68kDataId::ArrayBacking(_)) {
                            Some(location)
                        } else {
                            initial
                        }
                    }),
                })
            })
            .transpose()?;
        object.symbols.push(Symbol {
            id: image::SymbolId::Data(item.id),
            name: item.name.clone(),
            location,
            size: item.size.get(),
            alignment: item.alignment.get(),
            ty: item.ty.clone(),
            array,
        });
    }
    for routine in &mir.routines {
        let start = routines[&routine.id];
        let stop = routines
            .values()
            .copied()
            .filter(|a| *a > start)
            .min()
            .unwrap_or(code_size - 4);
        object.symbols.push(Symbol {
            id: image::SymbolId::Routine(routine.id),
            name: routine.name.clone(),
            location: code_location(start),
            size: stop - start,
            alignment: 2,
            ty: None,
            array: None,
        });
        object
            .symbols
            .extend(image::frame_symbols(routine, machine)?);
    }
    object.verify()?;
    Ok(object)
}

impl Location {
    fn add_offset(self, addend: u32) -> Result<Self, String> {
        match self {
            Self::Section { id, offset } => Ok(Self::Section {
                id,
                offset: end(offset, addend)?,
            }),
            Self::Absolute(address) => Ok(Self::Absolute(end(address, addend)?)),
            Self::Frame { .. } => Err("static relocation requires an invocation frame".into()),
        }
    }
    pub fn resolve(self, bases: &[u32]) -> Result<u32, String> {
        match self {
            Self::Section { id, offset } => end(
                *bases.get(id.0 as usize).ok_or("missing section base")?,
                offset,
            ),
            Self::Absolute(value) => Ok(value),
            Self::Frame { .. } => Err("frame symbol requires an active invocation".into()),
        }
    }
}

impl Object {
    pub fn verify(&self) -> Result<(), String> {
        for (index, section) in self.sections.iter().enumerate() {
            let initialized = (section.bytes.len() as u64) + u64::from(section.zero_fill);
            if section.id.0 as usize != index
                || !section.alignment.is_power_of_two()
                || section.alignment > 2
                || section.size > LIMIT
                || initialized > u64::from(section.size)
                || section.fixed_address.is_none() && initialized != u64::from(section.size)
            {
                return Err("invalid native object section identity, alignment or extent".into());
            }
            if section.executable
                && (section.alignment != 2
                    || section.bytes.len() % 2 != 0
                    || section.zero_fill != 0)
            {
                return Err("invalid native object code extent".into());
            }
            if let Some(address) = section.fixed_address {
                end(address, section.size)?;
            }
        }
        let check_location = |location: Location, size: u32| -> Result<(), String> {
            match location {
                Location::Section { id, offset } => {
                    let section = self
                        .sections
                        .get(id.0 as usize)
                        .ok_or("missing object section")?;
                    if end(offset, size)? > section.size {
                        return Err("object symbol extends past its section".into());
                    }
                }
                Location::Absolute(address) => {
                    end(address, size)?;
                }
                Location::Frame { .. } => {}
            }
            Ok(())
        };
        let Location::Section { id, offset } = self.entry else {
            return Err("object entry must be section-relative".into());
        };
        if !self
            .sections
            .get(id.0 as usize)
            .is_some_and(|s| s.executable && offset < s.bytes.len() as u32 && offset & 1 == 0)
        {
            return Err("object entry is outside code".into());
        }
        let mut occupied = BTreeSet::new();
        for relocation in &self.relocations {
            let section = self
                .sections
                .get(relocation.section.0 as usize)
                .ok_or("missing relocation source section")?;
            if !matches!(relocation.width, 1 | 2 | 4)
                || end(relocation.offset, relocation.width)? > section.bytes.len() as u32
                || relocation
                    .byte_index
                    .is_some_and(|i| i >= 4 || relocation.width != 1)
            {
                return Err("invalid object relocation encoding or extent".into());
            }
            for byte in relocation.offset..relocation.offset + relocation.width {
                if !occupied.insert((relocation.section, byte)) {
                    return Err("overlapping object relocations".into());
                }
            }
            if let RelocationTarget::Location(location) = relocation.target {
                if matches!(location, Location::Frame { .. }) {
                    return Err("relocation cannot target a frame".into());
                }
                check_location(location, 0)?;
            }
        }
        for symbol in &self.symbols {
            check_location(symbol.location, symbol.size)?;
            if let Some(backing) = symbol.array.as_ref().and_then(|a| a.backing) {
                check_location(backing, 0)?;
            }
        }
        Ok(())
    }

    /// Preserve the historical bare layout: CODE, then objects in declaration
    /// order. Fixed regions and aliases never consume allocatable address space.
    pub fn link(&self, origin: u32) -> Result<image::NativeImage, String> {
        self.verify()?;
        if origin & 1 != 0 || origin >= LIMIT {
            return Err("native origin must be even and within the 24-bit address space".into());
        }
        let mut cursor = origin;
        let mut bases = Vec::new();
        for section in &self.sections {
            let address = if let Some(address) = section.fixed_address {
                address
            } else {
                cursor = end(cursor, section.alignment - 1)? & !(section.alignment - 1);
                let start = cursor;
                cursor = end(cursor, section.size)?;
                start
            };
            bases.push(address);
        }
        self.link_sections(&bases, Some(cursor))
    }

    pub fn link_sections(
        &self,
        bases: &[u32],
        image_end: Option<u32>,
    ) -> Result<image::NativeImage, String> {
        self.verify()?;
        if bases.len() != self.sections.len() {
            return Err("section base count mismatch".into());
        }
        let mut payloads: Vec<_> = self.sections.iter().map(|s| s.bytes.clone()).collect();
        for relocation in &self.relocations {
            let base = match relocation.target {
                RelocationTarget::Location(location) => location.resolve(bases)?,
                RelocationTarget::ImageEnd => {
                    image_end.ok_or("ImageEnd requires a contiguous bare layout")?
                }
            };
            let value = adjusted(base, relocation.addend)?;
            let start = relocation.offset as usize;
            let bytes = &mut payloads[relocation.section.0 as usize];
            if let Some(index) = relocation.byte_index {
                bytes[start] = (value >> (8 * index)) as u8;
            } else {
                let width = relocation.width as usize;
                if width < 4 && value >= 1u32 << (8 * width) {
                    return Err("relocation value does not fit its width".into());
                }
                bytes[start..start + width].copy_from_slice(&value.to_be_bytes()[4 - width..]);
            }
        }
        let mut image = image::NativeImage {
            target_layout: crate::target::TargetLayout::for_target(TargetId::Motorola68000),
            entry: self.entry.resolve(bases)?,
            segments: vec![],
            zero_fill: vec![],
            symbols: vec![],
        };
        for ((section, &address), bytes) in self.sections.iter().zip(bases).zip(payloads) {
            end(address, section.size)?;
            if section.fixed_address.is_some_and(|fixed| fixed != address)
                || section.fixed_address.is_none() && address % section.alignment != 0
            {
                return Err("invalid fixed or aligned section base".into());
            }
            if section.zero_fill != 0 {
                image.zero_fill.push(image::ZeroFill {
                    address: address + bytes.len() as u32,
                    size: section.zero_fill,
                    writable: section.writable,
                });
            }
            if !bytes.is_empty() {
                image.segments.push(image::Segment {
                    address,
                    bytes,
                    writable: section.writable,
                    executable: section.executable,
                });
            }
        }
        for symbol in &self.symbols {
            let location = match symbol.location {
                Location::Frame { routine, offset } => {
                    image::SymbolLocation::Frame { routine, offset }
                }
                location => image::SymbolLocation::Absolute(location.resolve(bases)?),
            };
            let array = symbol
                .array
                .as_ref()
                .map(|a| -> Result<image::ArrayInfo, String> {
                    Ok(image::ArrayInfo {
                        element_width: a.element_width,
                        stride: a.stride,
                        count: a.count,
                        descriptor: a.descriptor,
                        backing_address: a.backing.map(|l| l.resolve(bases)).transpose()?,
                    })
                })
                .transpose()?;
            image.symbols.push(image::Symbol {
                id: symbol.id,
                name: symbol.name.clone(),
                location,
                size: symbol.size,
                alignment: if matches!(location, image::SymbolLocation::Absolute(a) if a & 1 != 0) {
                    1
                } else {
                    symbol.alignment
                },
                ty: symbol.ty.clone(),
                array,
            });
        }
        image.verify()?;
        Ok(image)
    }
}

fn end(address: u32, size: u32) -> Result<u32, String> {
    address
        .checked_add(size)
        .filter(|end| *end <= LIMIT)
        .ok_or_else(|| "native extent exceeds the 24-bit address space".into())
}
fn physical(value: u64) -> Result<u32, String> {
    u32::try_from(value)
        .ok()
        .filter(|v| *v < LIMIT)
        .ok_or_else(|| "address exceeds the 24-bit address space".into())
}
fn adjusted(address: u32, addend: i64) -> Result<u32, String> {
    i64::from(address)
        .checked_add(addend)
        .filter(|v| *v >= 0 && *v < i64::from(LIMIT))
        .map(|v| v as u32)
        .ok_or_else(|| "relocation address/addend exceeds the 24-bit address space".into())
}
