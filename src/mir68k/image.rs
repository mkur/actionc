//! Native image allocation, linking and symbol metadata. No Atari load format.
use super::{machine::*, *};
use crate::target::TargetLayout;

const LIMIT: u32 = 0x0100_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub address: u32,
    pub bytes: Vec<u8>,
    pub writable: bool,
    pub executable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Execution and inspection views shared by linked images and artifact readers.
/// Neither interface reconstructs executable NIR from serialized metadata.
pub trait ImageView {
    fn entry(&self) -> u32;
    fn segments(&self) -> &[Segment];
    fn zero_fill(&self) -> &[ZeroFill];
    fn verify(&self) -> Result<(), String>;
}
impl ImageView for NativeImage {
    fn entry(&self) -> u32 {
        self.entry
    }
    fn segments(&self) -> &[Segment] {
        &self.segments
    }
    fn zero_fill(&self) -> &[ZeroFill] {
        &self.zero_fill
    }
    fn verify(&self) -> Result<(), String> {
        NativeImage::verify(self)
    }
}
pub trait SymbolView {
    fn name(&self) -> &str;
    fn size(&self) -> u32;
    fn address(&self) -> Result<u32, String>;
    fn array(&self) -> Option<&ArrayInfo>;
    /// Width and signedness only for inspectable scalar values.
    fn scalar_type(&self) -> Option<(u32, bool)>;
    fn scalar_width(&self) -> Result<u32, String> {
        let (width, _) = self.scalar_type().ok_or("symbol has no scalar type")?;
        if self.array().is_some() || self.size() != width || !matches!(width, 1 | 2 | 4) {
            return Err("symbol is not a supported scalar".into());
        }
        Ok(width)
    }
}
impl SymbolView for Symbol {
    fn name(&self) -> &str {
        &self.name
    }
    fn size(&self) -> u32 {
        self.size
    }
    fn address(&self) -> Result<u32, String> {
        Symbol::address(self)
    }
    fn array(&self) -> Option<&ArrayInfo> {
        self.array.as_ref()
    }
    fn scalar_type(&self) -> Option<(u32, bool)> {
        use crate::nir::NirTypeKind;
        let ty = self.ty.as_ref()?;
        let signed = match &ty.kind {
            NirTypeKind::Integer(i) => i.signed,
            NirTypeKind::Bool | NirTypeKind::Pointer { .. } | NirTypeKind::Callable { .. } => false,
            _ => return None,
        };
        Some((ty.width?.get(), signed))
    }
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
        verify_regions(
            self.target_layout,
            self.entry,
            &self.segments,
            &self.zero_fill,
        )
    }
}

pub fn verify_regions(
    layout: TargetLayout,
    entry: u32,
    segments: &[Segment],
    zero_fill: &[ZeroFill],
) -> Result<(), String> {
    if layout != TargetLayout::for_target(TargetId::Motorola68000) {
        return Err("image target is not original MC68000".into());
    }
    let extents = segments
        .iter()
        .map(|s| {
            Ok((
                s.address,
                u32::try_from(s.bytes.len()).map_err(|_| "segment too large")?,
                s.executable,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    verify_region_extents(entry, &extents, zero_fill)
}

/// Validate declared extents before an artifact loader allocates payload bytes.
pub fn verify_region_extents(
    entry: u32,
    segments: &[(u32, u32, bool)],
    zero_fill: &[ZeroFill],
) -> Result<(), String> {
    let mut ranges = Vec::new();
    for &(address, size, executable) in segments {
        if size == 0 {
            return Err("empty initialized segment".into());
        }
        let end = checked_end(address, size)?;
        if executable && (address & 1 != 0 || size & 1 != 0) {
            return Err("unaligned code segment".into());
        }
        ranges.push((address, end));
    }
    for zero in zero_fill {
        if zero.size == 0 {
            return Err("empty zero-fill region".into());
        }
        ranges.push((zero.address, checked_end(zero.address, zero.size)?));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
        return Err("overlapping native image regions".into());
    }
    if entry & 1 != 0
        || !segments.iter().any(|&(address, size, executable)| {
            executable && entry >= address && entry < address + size
        })
    {
        return Err("entry is not in emitted code".into());
    }
    Ok(())
}

pub fn link(
    mir: &Mir68kProgram,
    machine: &MachineProgram,
    origin: u32,
) -> Result<NativeImage, String> {
    super::object::emit(mir, machine)?.link(origin)
}

pub(super) fn frame_symbols(
    r: &Mir68kRoutine,
    machine: &MachineProgram,
) -> Result<Vec<super::object::Symbol>, String> {
    let mut symbols = Vec::new();
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
        symbols.push(super::object::Symbol {
            id: SymbolId::Parameter {
                routine: r.id,
                param: parameter.param,
            },
            name: format!("{}::{name}", r.name),
            location: super::object::Location::Frame {
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
        symbols.push(super::object::Symbol {
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
            location: super::object::Location::Frame {
                routine: r.id,
                offset: object.frame_offset,
            },
            size: object.size.get(),
            alignment: object.alignment.get(),
            ty,
            array: None,
        });
    }
    Ok(symbols)
}

fn checked_end(address: u32, size: u32) -> Result<u32, String> {
    address
        .checked_add(size)
        .filter(|end| *end <= LIMIT)
        .ok_or_else(|| "native extent exceeds the 24-bit address space".into())
}
