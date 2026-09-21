//! Typed, unplaced address fixups shared by native image formats.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Routine(RoutineId),
    Data(Mir65816DataId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Code(RoutineId, u32),
    Runtime(RuntimeSymbolId),
    Data(Mir65816DataId),
    Absolute(u32),
    ImageEnd,
    StackOverflow,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixup {
    pub owner: Owner,
    pub offset: u32,
    pub width: u32,
    pub selector: Option<u8>,
    pub target: Target,
    pub addend: i64,
}

pub fn collect(
    program: &Mir65816Program,
    machine: &emit::MachineProgram,
) -> Result<Vec<Fixup>, String> {
    let mut result = Vec::new();
    let mut sites = BTreeMap::<Owner, Vec<(u32, u32)>>::new();
    let mut owners = BTreeSet::new();
    for r in &machine.routines {
        let owner = Owner::Routine(r.id);
        if !owners.insert(owner) {
            return Err("duplicate machine routine".into());
        }
        let ranges = sites.entry(owner).or_default();
        emit::layout::validate_branches(&r.code, None)?;
        for s in r.code.conditional_branches.iter().filter(|s| s.short) {
            let off = u32::try_from(s.offset + 1).map_err(|_| "branch offset overflow")?;
            ranges.push((off, off.checked_add(1).ok_or("branch offset overflow")?));
        }
        for &(offset, _) in &r.code.return_fixups {
            let off = u32::try_from(offset).map_err(|_| "PER offset overflow")?;
            ranges.push((off, off.checked_add(2).ok_or("PER offset overflow")?));
        }
        for f in &r.code.fixups {
            let target = match f.target {
                emit::Target::Label(id) => {
                    let offset = *r.code.labels.get(&id).ok_or("unresolved code label")?;
                    if offset >= r.code.bytes.len() {
                        return Err("label is outside routine code".into());
                    }
                    Target::Code(r.id, u32::try_from(offset).map_err(|_| "label overflow")?)
                }
                emit::Target::Routine(id) => Target::Code(id, 0),
                emit::Target::Runtime(id) => Target::Runtime(id),
                emit::Target::Data(id) => Target::Data(id),
                emit::Target::StackOverflow => Target::StackOverflow,
            };
            result.push(Fixup {
                owner,
                offset: u32::try_from(f.offset).map_err(|_| "fixup offset overflow")?,
                width: if f.byte.is_some() { 1 } else { 3 },
                selector: f.byte,
                target,
                addend: i64::from(f.addend),
            });
        }
    }
    for d in &program.data {
        let owner = Owner::Data(d.id);
        if !owners.insert(owner) {
            return Err("duplicate data identity".into());
        }
        if d.placement != Mir65816DataPlacement::Allocate
            && (!d.bytes.is_empty() || !d.relocations.is_empty())
        {
            return Err("external/alias storage cannot own initialized bytes".into());
        }
        for f in &d.relocations {
            let target = match f.target {
                Mir65816RelocationTarget::Data(NirStorageId::Global(id)) => {
                    Target::Data(Mir65816DataId::Global(id))
                }
                Mir65816RelocationTarget::ArrayBacking(id) => {
                    Target::Data(Mir65816DataId::ArrayBacking(id))
                }
                Mir65816RelocationTarget::Code(id) => Target::Code(id, 0),
                Mir65816RelocationTarget::Absolute(a) => Target::Absolute(
                    u32::try_from(a.value)
                        .ok()
                        .filter(|v| *v < 0x1000000)
                        .ok_or("absolute address exceeds 24 bits")?,
                ),
                Mir65816RelocationTarget::ImageEnd => Target::ImageEnd,
                _ => return Err("relocation refers to invocation storage".into()),
            };
            result.push(Fixup {
                owner,
                offset: f.offset.get(),
                width: f.width.get(),
                selector: f.byte_index,
                target,
                addend: f.addend,
            });
        }
    }
    for f in &result {
        if !(1..=4).contains(&f.width) || f.selector.is_some_and(|s| s > 2 || f.width != 1) {
            return Err("invalid relocation byte selector or width".into());
        }
        sites.entry(f.owner).or_default().push((
            f.offset,
            f.offset
                .checked_add(f.width)
                .ok_or("relocation offset overflow")?,
        ));
    }
    for (owner, mut ranges) in sites {
        let size = match owner {
            Owner::Routine(id) => machine
                .routines
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .code
                .bytes
                .len(),
            Owner::Data(id) => program
                .data
                .iter()
                .find(|d| d.id == id)
                .unwrap()
                .bytes
                .len(),
        };
        ranges.sort_unstable();
        if ranges
            .iter()
            .any(|&(a, b)| a >= b || b as u64 > size as u64)
        {
            return Err("relocation exceeds initialized storage".into());
        }
        if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
            return Err("overlapping relocation sites".into());
        }
    }
    Ok(result)
}

pub(crate) fn routine_bytes(r: &emit::MachineRoutine, base: u32) -> Result<Vec<u8>, String> {
    emit::layout::validate_branches(&r.code, Some(base))?;
    let mut bytes = r.code.bytes.clone();
    for &(offset, continuation) in &r.code.return_fixups {
        let resume = *r
            .code
            .labels
            .get(&continuation)
            .ok_or("unresolved PER continuation")?;
        if offset == 0
            || offset.checked_add(2).is_none_or(|end| end > bytes.len())
            || bytes[offset - 1] != 0x62
            || resume == 0
            || resume >= bytes.len()
            || (base + offset as u32 + 2) >> 16 != base >> 16
            || (base + resume as u32) >> 16 != base >> 16
        {
            return Err("invalid PER instruction/continuation placement".into());
        }
        let delta = i16::try_from(resume as i64 - 1 - (offset as i64 + 2))
            .map_err(|_| "PER continuation exceeds signed relative range")?;
        bytes[offset..offset + 2].copy_from_slice(&delta.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn patch(
    bytes: &mut [u8],
    offset: usize,
    value: i64,
    selector: Option<u8>,
    width: u32,
) -> Result<(), String> {
    if !(0..0x1000000).contains(&value) || !(1..=4).contains(&width) {
        return Err("relocation value exceeds the 24-bit address space".into());
    }
    let value = value as u32;
    let value = if let Some(byte) = selector {
        if byte > 2 || width != 1 {
            return Err("invalid relocation byte selector".into());
        }
        (value >> (byte * 8)) & 0xff
    } else {
        if width < 4 && value >= (1 << (width * 8)) {
            return Err("relocation does not fit its destination".into());
        }
        value
    };
    let output = bytes
        .get_mut(
            offset
                ..offset
                    .checked_add(width as usize)
                    .ok_or("relocation offset overflow")?,
        )
        .ok_or("relocation exceeds initialized storage")?;
    output.copy_from_slice(&value.to_le_bytes()[..width as usize]);
    Ok(())
}
