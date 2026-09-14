//! Bounded classic Amiga executable consumer of the relocatable native object.
//! Format: Amiga ROM Kernel Reference Manual: AmigaDOS, first edition (2024),
//! section 11.2, tables 11.2–11.6. No object-file extensions or memory classes.
//! https://developer.amigaos3.net/sites/default/files/downloads/2024-10/Amiga_ROM_Kernel_Reference_Manual_DOS.pdf
use super::object::*;
use std::collections::BTreeMap;

const HEADER: u32 = 1011;
const CODE: u32 = 1001;
const DATA: u32 = 1002;
const BSS: u32 = 1003;
const RELOC32: u32 = 1004;
const END: u32 = 1010;
const LIMIT: u32 = 0x0100_0000;

pub struct Executable {
    pub bytes: Vec<u8>,
    /// Packed logical sections and symbols, before OS allocation and padding.
    pub object: Object,
}

pub fn emit(object: &Object) -> Result<Executable, String> {
    let object = pack(object)?;
    let bytes = serialize(&object)?;
    Ok(Executable { bytes, object })
}

/// An intermediate no-OS executable can tail-jump to the Action! entry.
/// The Amiga process runtime supplies its own entry wrapper instead.
pub fn entry_thunk(object: &Object) -> Result<Object, String> {
    object.verify()?;
    let Location::Section { id, .. } = object.entry else {
        unreachable!()
    };
    let shift = |location| -> Result<Location, String> {
        match location {
            Location::Section {
                id: section,
                offset,
            } if section == id => Ok(Location::Section {
                id,
                offset: checked_add(offset, 6)?,
            }),
            other => Ok(other),
        }
    };
    let mut output = object.clone();
    let code = &mut output.sections[id.0 as usize];
    if code.fixed_address.is_some() {
        return Err("entry thunk requires movable code".into());
    }
    code.size = checked_add(code.size, 6)?;
    code.bytes.splice(..0, [0x4e, 0xf9, 0, 0, 0, 0]); // JMP absolute long.
    for relocation in &mut output.relocations {
        if relocation.section == id {
            relocation.offset = checked_add(relocation.offset, 6)?;
        }
        if let RelocationTarget::Location(location) = &mut relocation.target {
            *location = shift(*location)?;
        }
    }
    for symbol in &mut output.symbols {
        symbol.location = shift(symbol.location)?;
        if let Some(backing) = symbol.array.as_mut().and_then(|a| a.backing.as_mut()) {
            *backing = shift(*backing)?;
        }
    }
    output.relocations.push(Relocation {
        section: id,
        offset: 2,
        width: 4,
        byte_index: None,
        target: RelocationTarget::Location(shift(object.entry)?),
        addend: 0,
    });
    output.entry = Location::Section { id, offset: 0 };
    output.verify()?;
    Ok(output)
}

fn pack(input: &Object) -> Result<Object, String> {
    input.verify()?;
    let Location::Section {
        id: code,
        offset: 0,
    } = input.entry
    else {
        return Err("Amiga entry must be at the start of CODE".into());
    };
    if input.sections.iter().filter(|s| s.executable).count() != 1
        || input.sections[code.0 as usize].fixed_address.is_some()
    {
        return Err("Amiga requires one movable code section".into());
    }
    let mut output = Object {
        entry: Location::Section {
            id: SectionId(0),
            offset: 0,
        },
        sections: vec![],
        relocations: vec![],
        symbols: vec![],
    };
    let mut mappings = BTreeMap::new();
    for kind in [CODE, DATA, BSS] {
        let id = SectionId(output.sections.len() as u32);
        let mut section = Section {
            id,
            bytes: vec![],
            zero_fill: 0,
            size: 0,
            alignment: 2,
            writable: kind != CODE,
            executable: kind == CODE,
            fixed_address: None,
        };
        for source in &input.sections {
            if let Some(address) = source.fixed_address {
                if !source.bytes.is_empty() || source.zero_fill != 0 {
                    return Err("Amiga cannot initialize fixed-address storage".into());
                }
                mappings.insert(source.id, Location::Absolute(address));
                continue;
            }
            let source_kind = if source.executable {
                CODE
            } else if source.bytes.iter().any(|b| *b != 0)
                || input.relocations.iter().any(|r| r.section == source.id)
            {
                DATA
            } else {
                BSS
            };
            if source_kind != kind {
                continue;
            }
            let offset = checked_add(section.size, source.alignment - 1)? & !(source.alignment - 1);
            section.size = checked_add(offset, source.size)?;
            mappings.insert(source.id, Location::Section { id, offset });
            if kind != BSS {
                // Keep every object contiguous, including its explicit zero tail.
                section.bytes.resize(section.size as usize, 0);
                section.bytes[offset as usize..offset as usize + source.bytes.len()]
                    .copy_from_slice(&source.bytes);
            }
        }
        if kind == BSS {
            section.zero_fill = section.size;
        }
        // Keep zero-sized declarations mapped, even if this section has no bytes.
        if kind == CODE
            || mappings
                .values()
                .any(|loc| matches!(loc, Location::Section { id: mapped, .. } if *mapped == id))
        {
            output.sections.push(section);
        }
    }
    let locate = |location| -> Result<Location, String> {
        match location {
            Location::Section { id, offset } => {
                match mappings.get(&id).ok_or("unmapped Amiga object")? {
                    Location::Section { id, offset: base } => Ok(Location::Section {
                        id: *id,
                        offset: checked_add(*base, offset)?,
                    }),
                    Location::Absolute(base) => Ok(Location::Absolute(checked_add(*base, offset)?)),
                    _ => unreachable!(),
                }
            }
            other => Ok(other),
        }
    };
    for relocation in &input.relocations {
        let Location::Section { id, offset } = locate(Location::Section {
            id: relocation.section,
            offset: relocation.offset,
        })?
        else {
            return Err("Amiga relocation source must be movable".into());
        };
        let target = match relocation.target {
            RelocationTarget::Location(location) => RelocationTarget::Location(locate(location)?),
            RelocationTarget::ImageEnd => {
                return Err("ImageEnd is undefined for independently loaded Amiga hunks".into());
            }
        };
        output.relocations.push(Relocation {
            section: id,
            offset,
            target,
            ..relocation.clone()
        });
    }
    for source in &input.symbols {
        let mut symbol = source.clone();
        symbol.location = locate(symbol.location)?;
        if let Some(backing) = symbol.array.as_mut().and_then(|a| a.backing.as_mut()) {
            *backing = locate(*backing)?;
        }
        output.symbols.push(symbol);
    }
    output.verify()?;
    Ok(output)
}

fn serialize(object: &Object) -> Result<Vec<u8>, String> {
    let mut payloads: Vec<_> = object.sections.iter().map(|s| s.bytes.clone()).collect();
    let mut groups: BTreeMap<(SectionId, SectionId), Vec<u32>> = BTreeMap::new();
    for relocation in &object.relocations {
        let (base, target) = match relocation.target {
            RelocationTarget::Location(Location::Section { id, offset }) => (offset, Some(id)),
            RelocationTarget::Location(Location::Absolute(address)) => (address, None),
            _ => return Err("unsupported Amiga relocation target".into()),
        };
        let value = i64::from(base)
            .checked_add(relocation.addend)
            .filter(|n| (0..i64::from(LIMIT)).contains(n))
            .ok_or("Amiga relocation addend exceeds the 24-bit address range")?
            as u32;
        if let Some(target) = target {
            if relocation.width != 4
                || relocation.byte_index.is_some()
                || relocation.offset & 1 != 0
            {
                return Err("movable Amiga addresses require even full-width RELOC32 sites".into());
            }
            groups
                .entry((relocation.section, target))
                .or_default()
                .push(relocation.offset);
        }
        let start = relocation.offset as usize;
        let bytes = &mut payloads[relocation.section.0 as usize];
        if let Some(index) = relocation.byte_index {
            bytes[start] = (value >> (8 * index)) as u8;
        } else {
            let width = relocation.width as usize;
            if width < 4 && value >= 1u32 << (8 * width) {
                return Err("absolute relocation does not fit its width".into());
            }
            bytes[start..start + width].copy_from_slice(&value.to_be_bytes()[4 - width..]);
        }
    }
    let mut bytes = Vec::new();
    for value in [
        HEADER,
        0,
        object.sections.len() as u32,
        0,
        object.sections.len() as u32 - 1,
    ] {
        word(&mut bytes, value);
    }
    for section in &object.sections {
        word(&mut bytes, longwords(section.size)?);
    }
    for (section, mut payload) in object.sections.iter().zip(payloads) {
        let kind = if section.executable {
            CODE
        } else if section.bytes.is_empty() {
            BSS
        } else {
            DATA
        };
        let words = longwords(section.size)?;
        word(&mut bytes, kind);
        word(&mut bytes, words);
        if kind != BSS {
            payload.resize((words * 4) as usize, 0);
            bytes.extend(payload);
        }
        for ((source, target), offsets) in &mut groups {
            if *source != section.id {
                continue;
            }
            offsets.sort_unstable();
            // DOS 36+ has a count bug; split large groups into separate records.
            for chunk in offsets.chunks(65535) {
                for value in [RELOC32, chunk.len() as u32, target.0] {
                    word(&mut bytes, value);
                }
                for &offset in chunk {
                    word(&mut bytes, offset);
                }
                word(&mut bytes, 0);
            }
        }
        word(&mut bytes, END);
    }
    Ok(bytes)
}

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_be_bytes());
}
fn checked_add(value: u32, addend: u32) -> Result<u32, String> {
    value
        .checked_add(addend)
        .filter(|n| *n <= LIMIT)
        .ok_or_else(|| "Amiga section exceeds the 24-bit address range".into())
}
fn longwords(bytes: u32) -> Result<u32, String> {
    Ok(checked_add(bytes, 3)? / 4)
}
