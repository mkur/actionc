//! Compact checked Task application profile. Debug maps and complete contracts
//! stay in the host report; standard o65 records are the only on-disk fixups.
use super::{
    Artifact, Placement, Region,
    profile::*,
    read,
    relocate::{RangeIndex, disjoint},
    wire,
};
use crate::mir65816::image::{Segment, ZeroFill};
use std::collections::BTreeSet;

const ABI_REVISION: u16 = crate::mir65816::abi::generated::ABI_VERSION as u16;
const HEADER: &[u8; 8] = &[
    b'A', b'8', b'C', b'3', ABI_REVISION as u8, (ABI_REVISION >> 8) as u8, 0, 0,
];

pub(super) fn descriptor(p: &Profile) -> Vec<u8> {
    let mut bytes = HEADER.to_vec();
    for i in &p.imports {
        bytes.extend(i.contract.signature.to_le_bytes());
    }
    bytes
}

fn ordinary(c: &Contract) -> bool {
    c.abi == crate::mir65816::abi::generated::ABI_NAME
        && c.kind == 0
        && c.stack_peak == 0
        && c.irq_effect == 0
        && c.domains == 1
}

pub(super) fn admit(a: &Artifact) -> Result<(), String> {
    let p = &a.profile;
    let entry = p
        .routines
        .iter()
        .find(|r| r.offset == p.entry)
        .ok_or("missing compact entry")?;
    if p.nmi_extra_stack != 0
        || p.version != 1
        || entry.contract.signature != compact_entry_signature()
        || !entry.contract.arguments.is_empty()
        || entry.contract.result != 4
        || entry.contract.incoming != 1
        || p.objects.iter().any(|o| o.location.section.is_none())
        || p.imports.iter().skip(1).any(|i| !ordinary(&i.contract))
    {
        return Err(format!(
            "compact v3 requires a checked no-argument LONGINT function entry (signature {:08x}, got {:08x}), Task-only preserving imports, no absolute storage, arithmetic fault or extra NMI allowance",
            compact_entry_signature(),
            entry.contract.signature
        ));
    }
    // Run the rich host proof checks before omitting them from the file.
    let file = wire::File {
        mode: wire::MODE,
        bases: [0; 4],
        lengths: [a.text.len() as u32, a.data.len() as u32, a.bss, 0],
        stack: 0,
        text: a.text.clone(),
        data: a.data.clone(),
        imports: p.imports.iter().map(|i| i.name.clone()).collect(),
        relocations: p
            .relocations
            .iter()
            .map(|r| {
                let mut r = r.clone();
                r.value &= match r.encoding {
                    Encoding::Low => 255,
                    Encoding::High => 65535,
                    _ => 0xffffff,
                };
                r
            })
            .collect(),
        exports: vec![
            wire::Export {
                name: ENTRY.into(),
                segment: 2,
                value: p.entry,
            },
            wire::Export {
                name: COMPACT_DESCRIPTOR.into(),
                segment: 2,
                value: a.text.len() as u32,
            },
        ],
    };
    super::relocate::validate_profile(&file, p, a.text.len() as u32)
}

#[derive(Debug, Clone)]
pub struct Import {
    pub name: String,
    pub signature: u32,
}
#[derive(Debug, Clone)]
pub struct Info {
    pub entry: u32,
    pub text_bytes: u32,
    pub imports: Vec<Import>,
}

fn application(bytes: &[u8]) -> Result<(wire::File, Info), String> {
    let mut f = read::decode(bytes)?;
    if f.mode != wire::MODE
        || f.bases != [0; 4]
        || f.lengths[3] != 0
        || f.stack != 0
        || f.exports.len() != 2
    {
        return Err("unsupported compact o65 header".into());
    }
    let export = |name: &str| {
        f.exports
            .iter()
            .find(|e| e.name == name && e.segment == 2)
            .map(|e| e.value)
            .ok_or("missing compact export")
    };
    let start = export(COMPACT_DESCRIPTOR)?;
    let entry = export(ENTRY)?;
    let raw = f
        .text
        .get(start as usize..)
        .ok_or("compact descriptor outside text")?;
    if raw.len() != 8 + 4 * f.imports.len()
        || !raw.starts_with(HEADER)
        || entry >= start
        || f.imports.first().map(String::as_str) != Some(OVERFLOW)
    {
        return Err("invalid compact descriptor/entry".into());
    }
    let mut imports = vec![];
    for (n, name) in f.imports.iter().enumerate() {
        let signature = u32::from_le_bytes(raw[8 + 4 * n..12 + 4 * n].try_into().unwrap());
        if !valid_name(name)
            || (n == 0 && signature != 0)
            || (n != 0
                && (name == OVERFLOW
                    || name == ARITHMETIC_FAULT
                    || name == ENTRY
                    || name.starts_with("__a816_o65_")))
        {
            return Err("invalid compact import".into());
        }
        imports.push(Import {
            name: name.clone(),
            signature,
        });
    }
    // Metadata is not loaded. Bounds for relocation sites/targets use code and
    // readonly objects only. Low/high records contain truncated addends; their
    // complete symbolic targets and zero-extension containers are checked by
    // admit(), not reconstructed from nonexistent runtime proofs.
    f.text.truncate(start as usize);
    f.lengths[0] = start;
    for r in &f.relocations {
        if r.encoding == Encoding::Word
            || r.offset + r.encoding.width() > f.lengths[r.section.index()]
        {
            return Err("invalid compact relocation site/type".into());
        }
        match r.target {
            Reference::Import(_) if r.value != 0 => return Err("compact import addend".into()),
            Reference::Section(s)
                if matches!(r.encoding, Encoding::Bank | Encoding::Long)
                    && r.value > f.lengths[s.index()] =>
            {
                return Err("compact relocation target outside section".into());
            }
            _ => {}
        }
    }
    Ok((
        f,
        Info {
            entry,
            text_bytes: start,
            imports,
        },
    ))
}

pub fn inspect(bytes: &[u8]) -> Result<Info, String> {
    Ok(application(bytes)?.1)
}

pub struct Image {
    pub segments: Vec<Segment>,
    pub zero_fill: Vec<ZeroFill>,
    pub entry: u32,
}

pub fn relocate(bytes: &[u8], placement: &Placement) -> Result<Image, String> {
    let (mut f, info) = application(bytes)?;
    if placement.nmi_extra_stack != 0
        || placement.providers.len() != info.imports.len()
        || placement.allowed.len() > MAX_ITEMS
        || placement.reserved.len() > MAX_ITEMS
    {
        return Err("invalid compact placement contract".into());
    }
    let allowed = RangeIndex::new(&placement.allowed)?;
    let reserved = RangeIndex::new(&placement.reserved)?;
    let mut allocations = vec![];
    for i in 0..3 {
        let region = Region {
            address: placement.bases[i],
            size: f.lengths[i],
        };
        region.end()?;
        if region.size == 0 {
            if region.address != 0 {
                return Err("empty section must have zero base".into());
            }
        } else {
            if region.address < 65536
                || region.address % if i == 0 { 65536 } else { 4 } != 0
                || !allowed.contains(region)
                || reserved.overlaps(region)
            {
                return Err("invalid compact section placement".into());
            }
            allocations.push(region);
        }
    }
    let mut names = BTreeSet::new();
    for p in &placement.providers {
        if !names.insert(&p.name) {
            return Err("duplicate provider".into());
        }
    }
    let mut addresses = vec![];
    for (index, import) in info.imports.iter().enumerate() {
        let p = placement
            .providers
            .iter()
            .find(|p| p.name == import.name)
            .ok_or("unresolved compact import")?;
        p.contract.verify()?;
        if p.contract.signature != import.signature
            || (if index == 0 {
                p.contract != Contract::overflow()
            } else {
                !ordinary(&p.contract)
            })
            || p.address == 0
            || p.size == 0
        {
            return Err("incompatible compact provider".into());
        }
        let region = Region {
            address: p.address,
            size: p.size,
        };
        region.end()?;
        if reserved.overlaps(region) {
            return Err("provider overlaps reserved region".into());
        }
        allocations.push(region);
        addresses.push(p.address);
    }
    disjoint(allocations)?;
    for r in &f.relocations {
        let base = match r.target {
            Reference::Section(s) => placement.bases[s.index()],
            Reference::Import(i) => addresses[i as usize],
        };
        let value = base + r.value;
        if matches!(r.encoding, Encoding::Bank | Encoding::Long) && value >= LIMIT {
            return Err("relocated address exceeds 24 bits".into());
        }
        let value = match r.encoding {
            Encoding::High => value >> 8,
            Encoding::Bank => value >> 16,
            _ => value,
        };
        let target = if r.section == Section::Text {
            &mut f.text
        } else {
            &mut f.data
        };
        target[r.offset as usize..(r.offset + r.encoding.width()) as usize]
            .copy_from_slice(&value.to_le_bytes()[..r.encoding.width() as usize]);
    }
    let mut segments = vec![Segment {
        address: placement.bases[0],
        bytes: f.text,
        writable: false,
        executable: true,
    }];
    if !f.data.is_empty() {
        segments.push(Segment {
            address: placement.bases[1],
            bytes: f.data,
            writable: true,
            executable: false,
        });
    }
    let zero_fill = if f.lengths[2] == 0 {
        vec![]
    } else {
        vec![ZeroFill {
            address: placement.bases[2],
            size: f.lengths[2],
            writable: true,
        }]
    };
    Ok(Image {
        segments,
        zero_fill,
        entry: placement.bases[0] + info.entry,
    })
}
