//! Native image allocation, linking and symbol metadata. No Atari load format.
use super::{encode, machine::*, *};
use crate::target::TargetLayout;
use std::collections::BTreeMap;

const LIMIT: u32 = 0x0100_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub address: u32,
    pub bytes: Vec<u8>,
    pub writable: bool,
    pub executable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroFill {
    pub address: u32,
    pub size: u32,
    pub writable: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolId {
    Data(Mir68kDataId),
    Routine(RoutineId),
    Parameter {
        routine: RoutineId,
        param: ParamId,
    },
    Automatic {
        routine: RoutineId,
        object: Mir68kFrameObjectId,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolLocation {
    Absolute(u32),
    Frame { routine: RoutineId, offset: i32 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayInfo {
    pub element_width: u32,
    pub stride: u32,
    pub count: Option<u32>,
    pub descriptor: bool,
    pub backing_address: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub location: SymbolLocation,
    pub size: u32,
    pub alignment: u32,
    pub ty: Option<crate::nir::NirType>,
    pub array: Option<ArrayInfo>,
}
impl Symbol {
    pub fn address(&self) -> Result<u32, String> {
        match self.location {
            SymbolLocation::Absolute(a) => Ok(a),
            _ => Err(format!("{} requires an active frame", self.name)),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeImage {
    pub target_layout: TargetLayout,
    pub entry: u32,
    pub segments: Vec<Segment>,
    pub zero_fill: Vec<ZeroFill>,
    pub symbols: Vec<Symbol>,
}
impl NativeImage {
    pub fn symbol(&self, name: &str) -> Result<&Symbol, String> {
        let mut found = self
            .symbols
            .iter()
            .filter(|s| s.name.eq_ignore_ascii_case(name));
        let first = found
            .next()
            .ok_or_else(|| format!("unknown symbol {name}"))?;
        if found.next().is_some() {
            return Err(format!("ambiguous symbol {name}"));
        }
        Ok(first)
    }
    pub fn verify(&self) -> Result<(), String> {
        if self.target_layout != TargetLayout::for_target(TargetId::Motorola68000) {
            return Err("image target is not original MC68000".into());
        }
        let mut ranges = Vec::new();
        for segment in &self.segments {
            let size = u32::try_from(segment.bytes.len()).map_err(|_| "segment too large")?;
            if size == 0 {
                return Err("empty initialized segment".into());
            }
            let end = checked_end(segment.address, size)?;
            if segment.executable && (segment.address & 1 != 0 || size & 1 != 0) {
                return Err("unaligned code segment".into());
            }
            ranges.push((segment.address, end));
        }
        for zero in &self.zero_fill {
            if zero.size == 0 {
                return Err("empty zero-fill region".into());
            }
            ranges.push((zero.address, checked_end(zero.address, zero.size)?));
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
            return Err("overlapping native image regions".into());
        }
        if self.entry & 1 != 0
            || !self.segments.iter().any(|s| {
                s.executable
                    && self.entry >= s.address
                    && self.entry < s.address + s.bytes.len() as u32
            })
        {
            return Err("entry is not in emitted code".into());
        }
        Ok(())
    }
}

pub fn link(
    mir: &Mir68kProgram,
    machine: &MachineProgram,
    origin: u32,
) -> Result<NativeImage, String> {
    verify::verify_contract(mir).map_err(|e| format!("invalid MIR68K: {e:?}"))?;
    if origin & 1 != 0 || origin >= LIMIT {
        return Err("native origin must be even and within the 24-bit address space".into());
    }
    let mut cursor = origin;
    let mut labels = BTreeMap::new();
    for block in &machine.blocks {
        if labels.insert(block.id, cursor).is_some() {
            return Err("duplicate machine block ID".into());
        }
        for op in &block.instructions {
            cursor = checked_end(cursor, encode::size(op)?)?;
        }
    }
    // Prefetch padding is part of the protected executable segment.
    cursor = checked_end(cursor, 4)?;
    let code_end = cursor;
    let mut routines = BTreeMap::new();
    for r in &machine.routines {
        let address = *labels
            .get(&r.entry)
            .ok_or("routine entry has no machine block")?;
        if routines.insert(r.id, address).is_some() {
            return Err("duplicate machine routine ID".into());
        }
    }
    if routines.len() != mir.routines.len()
        || mir.routines.iter().any(|r| !routines.contains_key(&r.id))
    {
        return Err("machine routine identities differ from MIR".into());
    }
    let entry = *routines
        .get(&mir.entry.ok_or("native image requires a program entry")?)
        .ok_or("entry routine was not emitted")?;
    let mut allocations = BTreeMap::new();
    for item in &mir.data {
        match item.placement {
            Mir68kDataPlacement::Allocate => {
                cursor = align(cursor, item.alignment.get())?;
                let start = cursor;
                cursor = checked_end(cursor, item.size.get())?;
                allocations.insert(item.id, start);
            }
            Mir68kDataPlacement::Absolute(address) => {
                if address.address_space != TargetLayout::DATA_ADDRESS_SPACE {
                    return Err("data uses another address space".into());
                }
                let value = physical_address(address.value)?;
                checked_end(value, item.size.get())?;
                allocations.insert(item.id, value);
            }
            Mir68kDataPlacement::Alias { .. } => {}
        }
    }
    let mut pending: Vec<_> = mir
        .data
        .iter()
        .filter(|d| matches!(d.placement, Mir68kDataPlacement::Alias { .. }))
        .collect();
    while !pending.is_empty() {
        let count = pending.len();
        let mut unresolved = Vec::new();
        for item in pending {
            let Mir68kDataPlacement::Alias { target, offset } = item.placement else {
                unreachable!()
            };
            if let Some(base) = allocations.get(&target).copied() {
                let address = checked_end(base, offset.get())?;
                checked_end(address, item.size.get())?;
                allocations.insert(item.id, address);
            } else {
                unresolved.push(item);
            }
        }
        if unresolved.len() == count {
            return Err("unresolved data aliases".into());
        }
        pending = unresolved;
    }
    let resolve = |address: Address| -> Result<u32, String> {
        let base = match address.target {
            Target::Absolute(a) => a,
            Target::Block(id) => *labels.get(&id).ok_or("missing machine block relocation")?,
            Target::Routine(id) => *routines.get(&id).ok_or("missing routine relocation")?,
            Target::Data(id) => *allocations.get(&id).ok_or("missing data relocation")?,
        };
        checked_address(base, address.addend)
    };
    let mut code = Vec::new();
    for op in machine.blocks.iter().flat_map(|b| &b.instructions) {
        code.extend(encode::encode(op, &resolve)?);
    }
    code.extend([0x4e, 0x71, 0x4e, 0x71]);
    debug_assert_eq!(code.len() as u32, code_end - origin);
    let mut image = NativeImage {
        target_layout: mir.target_layout,
        entry,
        segments: vec![Segment {
            address: origin,
            bytes: code,
            writable: false,
            executable: true,
        }],
        zero_fill: Vec::new(),
        symbols: Vec::new(),
    };
    for item in &mir.data {
        let address = allocations[&item.id];
        let mut bytes = item.bytes.clone();
        for reloc in &item.relocations {
            let target = match reloc.target {
                Mir68kRelocationTarget::Data(NirStorageId::Global(id)) => {
                    Target::Data(Mir68kDataId::Global(id))
                }
                Mir68kRelocationTarget::Data(_) => {
                    return Err("static relocation requires an invocation frame".into());
                }
                Mir68kRelocationTarget::Code(id) => Target::Routine(id),
                Mir68kRelocationTarget::ArrayBacking(id) => {
                    Target::Data(Mir68kDataId::ArrayBacking(id))
                }
                Mir68kRelocationTarget::Absolute(a) => Target::Absolute(physical_address(a.value)?),
                Mir68kRelocationTarget::ImageEnd => Target::Absolute(cursor),
            };
            let value = resolve(Address {
                target,
                addend: reloc.addend,
            })?;
            let width = reloc.width.get() as usize;
            let start = reloc.offset.get() as usize;
            if let Some(index) = reloc.byte_index {
                bytes[start] = (value >> (8 * index)) as u8;
            } else {
                if width < 4 && value >= 1u32 << (8 * width) {
                    return Err("relocation value does not fit its width".into());
                }
                bytes[start..start + width].copy_from_slice(&value.to_be_bytes()[4 - width..]);
            }
        }
        if !matches!(item.placement, Mir68kDataPlacement::Alias { .. }) {
            if !bytes.is_empty() {
                image.segments.push(Segment {
                    address,
                    bytes,
                    writable: item.mutable,
                    executable: false,
                });
            }
            if !item.zero_fill.is_zero() {
                image.zero_fill.push(ZeroFill {
                    address: address + item.bytes.len() as u32,
                    size: item.zero_fill.get(),
                    writable: item.mutable,
                });
            }
        }
        if let Some(a) = item.array.as_ref().and_then(|a| a.address_initializer) {
            physical_address(a.value)?;
        }
        let array = item.array.as_ref().map(|a| {
            let descriptor = matches!(item.id, Mir68kDataId::Global(id) if allocations.contains_key(&Mir68kDataId::ArrayBacking(id))) || a.pointer_backed;
            let backing_address = match item.id {
                Mir68kDataId::Global(id) if allocations.contains_key(&Mir68kDataId::ArrayBacking(id)) => Some(allocations[&Mir68kDataId::ArrayBacking(id)]),
                _ if !descriptor || matches!(item.id, Mir68kDataId::ArrayBacking(_)) => Some(address),
                _ => a.address_initializer.map(|a| a.value as u32),
            };
            ArrayInfo { element_width: a.elem_size.get(), stride: a.elem_size.get(), count: a.length, descriptor: descriptor && !matches!(item.id, Mir68kDataId::ArrayBacking(_)), backing_address }
        });
        image.symbols.push(Symbol {
            id: SymbolId::Data(item.id),
            name: item.name.clone(),
            location: SymbolLocation::Absolute(address),
            size: item.size.get(),
            alignment: if address & 1 == 0 {
                item.alignment.get()
            } else {
                1
            },
            ty: item.ty.clone(),
            array,
        });
    }
    for r in &mir.routines {
        let routine_end = routines
            .values()
            .copied()
            .filter(|a| *a > routines[&r.id])
            .min()
            .unwrap_or(code_end - 4);
        image.symbols.push(Symbol {
            id: SymbolId::Routine(r.id),
            name: r.name.clone(),
            location: SymbolLocation::Absolute(routines[&r.id]),
            size: routine_end - routines[&r.id],
            alignment: 2,
            ty: None,
            array: None,
        });
        let physical = machine.routines.iter().find(|p| p.id == r.id).unwrap();
        for parameter in physical
            .frame
            .parameters
            .iter()
            .filter(|p| p.frame_object.is_none())
        {
            let Mir68kAbiHome::StackArgument { offset, size } = parameter.incoming else {
                return Err("parameter symbol lacks stack home".into());
            };
            let name = r
                .param_names
                .iter()
                .find(|(p, _)| *p == parameter.param)
                .map(|(_, n)| n)
                .ok_or("parameter has no display name")?;
            image.symbols.push(Symbol {
                id: SymbolId::Parameter {
                    routine: r.id,
                    param: parameter.param,
                },
                name: format!("{}::{name}", r.name),
                location: SymbolLocation::Frame {
                    routine: r.id,
                    offset: i32::try_from(offset.get())
                        .ok()
                        .and_then(|o| o.checked_add(8))
                        .ok_or("parameter location overflow")?,
                },
                size: size.get(),
                alignment: size.get().min(2),
                ty: r
                    .params
                    .iter()
                    .find(|(p, _)| *p == parameter.param)
                    .map(|(_, ty)| ty.clone()),
                array: None,
            });
        }
        for object in &physical.frame.objects {
            let (name, ty) = match object.owner {
                Mir68kFrameObjectOwner::Local(id) => r
                    .locals
                    .iter()
                    .find(|(l, _, _)| *l == id)
                    .map(|(_, name, ty)| (name.clone(), Some(ty.clone())))
                    .ok_or("frame object has no local metadata")?,
                Mir68kFrameObjectOwner::Param(id) => (
                    r.param_names
                        .iter()
                        .find(|(p, _)| *p == id)
                        .map(|(_, name)| name.clone())
                        .ok_or("parameter has no display name")?,
                    r.params
                        .iter()
                        .find(|(p, _)| *p == id)
                        .map(|(_, t)| t.clone()),
                ),
            };
            image.symbols.push(Symbol {
                id: match object.owner {
                    Mir68kFrameObjectOwner::Param(param) => SymbolId::Parameter {
                        routine: r.id,
                        param,
                    },
                    Mir68kFrameObjectOwner::Local(_) => SymbolId::Automatic {
                        routine: r.id,
                        object: object.id,
                    },
                },
                name: format!("{}::{name}", r.name),
                location: SymbolLocation::Frame {
                    routine: r.id,
                    offset: object.frame_offset,
                },
                size: object.size.get(),
                alignment: object.alignment.get(),
                ty,
                array: None,
            });
        }
    }
    image.verify()?;
    Ok(image)
}

fn checked_end(address: u32, size: u32) -> Result<u32, String> {
    address
        .checked_add(size)
        .filter(|end| *end <= LIMIT)
        .ok_or_else(|| "native extent exceeds the 24-bit address space".into())
}
fn checked_address(address: u32, addend: i64) -> Result<u32, String> {
    i64::from(address)
        .checked_add(addend)
        .filter(|v| *v >= 0 && *v < i64::from(LIMIT))
        .map(|v| v as u32)
        .ok_or_else(|| "relocation address/addend exceeds the 24-bit address space".into())
}
fn align(value: u32, alignment: u32) -> Result<u32, String> {
    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
        .ok_or_else(|| "allocation alignment overflow".into())
}

fn physical_address(value: u64) -> Result<u32, String> {
    if value < u64::from(LIMIT) {
        Ok(value as u32)
    } else {
        Err("address exceeds the 24-bit address space".into())
    }
}
