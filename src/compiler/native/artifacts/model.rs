use crate::{
    mir68k::{
        Mir68kDataId,
        image::{ArrayInfo, NativeImage, SymbolView, ZeroFill},
    },
    nir::NirTypeKind,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const FORMAT: &str = "actionc-native";
pub const VERSION: u32 = 2;
pub const ADDRESS_LIMIT: u32 = 0x0100_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub target: String,
    pub endian: String,
    pub pointer_width: u32,
    pub link_address_bits: u32,
    pub entry: u32,
    pub machine_listing: Option<String>,
    pub segments: Vec<SegmentSpec>,
    pub zero_fill: Vec<ZeroFill>,
    pub symbols: Vec<ArtifactSymbol>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentSpec {
    pub file: String,
    pub address: u32,
    pub size: u32,
    pub writable: bool,
    pub executable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Identity {
    Global { id: u32 },
    Static { id: u32 },
    ArrayBacking { owner: u32 },
    StaticLocal { routine: u32, id: u32 },
    Routine { id: u32 },
    Automatic { routine: u32, object: u32 },
    Parameter { routine: u32, id: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Location {
    Absolute { address: u32 },
    Frame { routine: u32, offset: i32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    Integer,
    Boolean,
    DataPointer,
    Callable,
    Record,
    Opaque,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolLayout {
    pub kind: LayoutKind,
    pub width: Option<u32>,
    pub signed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSymbol {
    pub name: String,
    pub identity: Identity,
    pub location: Location,
    pub size: u32,
    pub alignment: u32,
    #[serde(rename = "type")]
    pub layout: Option<SymbolLayout>,
    pub array: Option<ArrayInfo>,
}
impl SymbolView for ArtifactSymbol {
    fn name(&self) -> &str {
        &self.name
    }
    fn size(&self) -> u32 {
        self.size
    }
    fn address(&self) -> Result<u32, String> {
        match self.location {
            Location::Absolute { address } => Ok(address),
            _ => Err(format!("{} requires an active frame", self.name)),
        }
    }
    fn array(&self) -> Option<&ArrayInfo> {
        self.array.as_ref()
    }
    fn scalar_type(&self) -> Option<(u32, bool)> {
        let layout = self.layout.as_ref()?;
        if matches!(
            layout.kind,
            LayoutKind::Integer
                | LayoutKind::Boolean
                | LayoutKind::DataPointer
                | LayoutKind::Callable
        ) {
            Some((layout.width?, layout.signed))
        } else {
            None
        }
    }
}
impl ArtifactSymbol {
    pub fn from_symbol(symbol: &crate::mir68k::image::Symbol) -> Self {
        use crate::mir68k::image::{SymbolId, SymbolLocation};
        let identity = match symbol.id {
            SymbolId::Data(Mir68kDataId::Global(id)) => Identity::Global { id: id.0 },
            SymbolId::Data(Mir68kDataId::Static(id)) => Identity::Static { id: id.0 },
            SymbolId::Data(Mir68kDataId::ArrayBacking(id)) => {
                Identity::ArrayBacking { owner: id.0 }
            }
            SymbolId::Data(Mir68kDataId::Local(routine, id)) => Identity::StaticLocal {
                routine: routine.0,
                id: id.0,
            },
            SymbolId::Routine(id) => Identity::Routine { id: id.0 },
            SymbolId::Automatic { routine, object } => Identity::Automatic {
                routine: routine.0,
                object: object.0,
            },
            SymbolId::Parameter { routine, param } => Identity::Parameter {
                routine: routine.0,
                id: param.0,
            },
        };
        let location = match symbol.location {
            SymbolLocation::Absolute(address) => Location::Absolute { address },
            SymbolLocation::Frame { routine, offset } => Location::Frame {
                routine: routine.0,
                offset,
            },
        };
        let mut layout = symbol.ty.as_ref().map(|ty| SymbolLayout {
            kind: match ty.kind {
                NirTypeKind::Integer(_) => LayoutKind::Integer,
                NirTypeKind::Bool => LayoutKind::Boolean,
                NirTypeKind::Pointer { .. } => LayoutKind::DataPointer,
                NirTypeKind::Callable { .. } => LayoutKind::Callable,
                NirTypeKind::Record { .. } => LayoutKind::Record,
                _ => LayoutKind::Opaque,
            },
            width: ty.width.map(|w| w.get()),
            signed: ty.kind.integer().is_some_and(|i| i.signed),
        });
        // Static initializer blocks can carry an element type without array
        // metadata. They are inspectable bytes, not a single scalar of that type.
        if matches!(symbol.id, SymbolId::Data(Mir68kDataId::Static(_)))
            && symbol.array.is_none()
            && layout
                .as_ref()
                .is_some_and(|t| t.width.is_some_and(|w| w != symbol.size))
        {
            layout = Some(SymbolLayout {
                kind: LayoutKind::Opaque,
                width: Some(symbol.size),
                signed: false,
            });
        }
        Self {
            name: symbol.name.clone(),
            identity,
            location,
            size: symbol.size,
            alignment: symbol.alignment,
            layout,
            array: symbol.array.clone(),
        }
    }
}
impl Manifest {
    pub(super) fn from_image(image: &NativeImage) -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            target: "motorola-68000".into(),
            endian: "big".into(),
            pointer_width: 4,
            link_address_bits: 24,
            entry: image.entry,
            machine_listing: None,
            segments: vec![],
            zero_fill: image.zero_fill.clone(),
            symbols: image
                .symbols
                .iter()
                .map(ArtifactSymbol::from_symbol)
                .collect(),
        }
    }
    pub fn symbol(&self, name: &str) -> Result<&ArtifactSymbol, String> {
        let mut matches = self
            .symbols
            .iter()
            .filter(|s| s.name.eq_ignore_ascii_case(name));
        let symbol = matches
            .next()
            .ok_or_else(|| format!("unknown symbol {name}"))?;
        if matches.next().is_some() {
            return Err(format!("ambiguous symbol {name}"));
        }
        Ok(symbol)
    }
    pub fn verify(&self) -> Result<(), String> {
        if self.format != FORMAT || self.version != VERSION {
            return Err("unsupported native artifact format/version".into());
        }
        if self.target != "motorola-68000"
            || self.endian != "big"
            || self.pointer_width != 4
            || self.link_address_bits != 24
        {
            return Err("incompatible native artifact target/layout".into());
        }
        let mut files = BTreeSet::new();
        for s in &self.segments {
            validate_filename(&s.file)?;
            if !files.insert(&s.file) {
                return Err("duplicate segment file".into());
            }
            extent(s.address, s.size)?;
            if s.size == 0 {
                return Err("empty segment".into());
            }
        }
        if let Some(file) = &self.machine_listing {
            validate_filename(file)?;
        }
        for zero in &self.zero_fill {
            extent(zero.address, zero.size)?;
        }
        crate::mir68k::image::verify_region_extents(
            self.entry,
            &self
                .segments
                .iter()
                .map(|s| (s.address, s.size, s.executable))
                .collect::<Vec<_>>(),
            &self.zero_fill,
        )?;
        let routines: BTreeSet<_> = self
            .symbols
            .iter()
            .filter_map(|s| match s.identity {
                Identity::Routine { id } => Some(id),
                _ => None,
            })
            .collect();
        let mut identities = BTreeSet::new();
        for symbol in &self.symbols {
            if !identities.insert(&symbol.identity) {
                return Err("duplicate symbol identity".into());
            }
            if symbol.name.is_empty()
                || symbol.alignment == 0
                || !symbol.alignment.is_power_of_two()
                || symbol.alignment > ADDRESS_LIMIT
            {
                return Err("invalid symbol name/alignment".into());
            }
            match (&symbol.identity, &symbol.location) {
                (
                    Identity::Automatic { routine, .. } | Identity::Parameter { routine, .. },
                    Location::Frame {
                        routine: owner,
                        offset,
                    },
                ) if routine == owner && routines.contains(routine) => {
                    if *offset < -32768
                        || i64::from(*offset) + i64::from(symbol.size) > 32768
                        || i64::from(*offset).rem_euclid(i64::from(symbol.alignment)) != 0
                    {
                        return Err("invalid frame symbol extent/alignment".into());
                    }
                }
                (Identity::Automatic { .. } | Identity::Parameter { .. }, _)
                | (_, Location::Frame { .. }) => {
                    return Err("invalid symbol identity/location".into());
                }
                (_, Location::Absolute { address }) => {
                    let end = extent(*address, symbol.size)?;
                    if address % symbol.alignment != 0 {
                        return Err("unaligned symbol".into());
                    }
                    if matches!(symbol.identity, Identity::Routine { .. })
                        && (symbol.size == 0
                            || !self.segments.iter().any(|s| {
                                s.executable && *address >= s.address && end <= s.address + s.size
                            }))
                    {
                        return Err("routine symbol outside executable memory".into());
                    }
                }
            }
            if let Some(layout) = &symbol.layout {
                let valid = match layout.kind {
                    LayoutKind::Integer => matches!(layout.width, Some(1 | 2 | 4)),
                    LayoutKind::Boolean => layout.width == Some(1) && !layout.signed,
                    LayoutKind::DataPointer | LayoutKind::Callable => {
                        layout.width == Some(4) && !layout.signed
                    }
                    LayoutKind::Record | LayoutKind::Opaque => {
                        !layout.signed && layout.width.is_none_or(|w| w <= ADDRESS_LIMIT)
                    }
                };
                if !valid {
                    return Err("invalid symbol type layout".into());
                }
                if symbol.array.is_none()
                    && layout.width.is_some_and(|w| {
                        if matches!(symbol.location, Location::Frame { .. }) {
                            w > symbol.size
                        } else {
                            w != symbol.size
                        }
                    })
                {
                    return Err("symbol type/extent mismatch".into());
                }
            }
            if let Some(array) = &symbol.array {
                if array.element_width == 0
                    || array.stride < array.element_width
                    || array.stride > ADDRESS_LIMIT
                    || (array.descriptor && !matches!(symbol.size, 4 | 6))
                {
                    return Err(format!(
                        "invalid array layout for {} (size {}, {array:?})",
                        symbol.name, symbol.size
                    ));
                }
                let bytes = array
                    .count
                    .map(|c| c.checked_mul(array.stride).ok_or("array extent overflow"))
                    .transpose()?;
                if bytes.is_some_and(|n| n > ADDRESS_LIMIT || !array.descriptor && n > symbol.size)
                {
                    return Err("array exceeds symbol extent".into());
                }
                if let Some(base) = array.backing_address {
                    extent(base, bytes.unwrap_or(0))?;
                }
            }
        }
        Ok(())
    }
}
pub(super) fn extent(address: u32, size: u32) -> Result<u32, String> {
    let end = address.checked_add(size).ok_or("native address overflow")?;
    if address >= ADDRESS_LIMIT || end > ADDRESS_LIMIT {
        return Err("native address exceeds 24-bit bus".into());
    }
    Ok(end)
}
pub(super) fn validate_filename(name: &str) -> Result<(), String> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\', ':', '\0']) {
        return Err("artifact payload must name a file beside the manifest".into());
    }
    Ok(())
}
