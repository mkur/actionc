use super::super::image::{Segment, ZeroFill};
use super::{profile::*, read, wire};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub address: u32,
    pub size: u32,
}
impl Region {
    fn end(self) -> Result<u32, String> {
        self.address
            .checked_add(self.size)
            .filter(|e| self.address < LIMIT && *e <= LIMIT)
            .ok_or("region exceeds 24-bit address space".into())
    }
    fn overlaps(self, other: Self) -> bool {
        self.size != 0
            && other.size != 0
            && u64::from(self.address) < u64::from(other.address) + u64::from(other.size)
            && u64::from(other.address) < u64::from(self.address) + u64::from(self.size)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub name: String,
    pub address: u32,
    pub size: u32,
    pub contract: Contract,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub bases: [u32; 3],
    pub allowed: Vec<Region>,
    pub reserved: Vec<Region>,
    pub nmi_extra_stack: u16,
    pub providers: Vec<Provider>,
}
/// A successful, fully private load; construction cannot write guest memory.
#[derive(Debug, Clone)]
pub struct RelocatedImage {
    segments: Vec<Segment>,
    zero_fill: Vec<ZeroFill>,
    profile: Profile,
    bases: [u32; 3],
    entry: u32,
    stack_overflow: u32,
}
impl RelocatedImage {
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    pub fn zero_fill(&self) -> &[ZeroFill] {
        &self.zero_fill
    }
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    pub fn entry(&self) -> u32 {
        self.entry
    }
    pub fn stack_overflow(&self) -> u32 {
        self.stack_overflow
    }
    pub fn task_headroom(&self) -> u32 {
        26 + u32::from(self.profile.nmi_extra_stack)
    }
    pub fn irq_headroom(&self) -> u32 {
        13 + u32::from(self.profile.nmi_extra_stack)
    }
    pub fn location(&self, l: Location) -> u32 {
        l.offset + l.section.map_or(0, |s| self.bases[s.index()])
    }
    pub fn routine_address(&self, r: &Routine) -> u32 {
        self.bases[0] + r.offset
    }
}
fn disjoint(mut ranges: Vec<Region>) -> Result<(), String> {
    ranges.retain(|r| r.size != 0);
    ranges.sort_by_key(|r| r.address);
    if ranges.windows(2).any(|r| r[0].overlaps(r[1])) {
        return Err("overlapping o65 regions".into());
    }
    Ok(())
}
fn export(file: &wire::File, name: &str) -> Result<u32, String> {
    let e = file
        .exports
        .iter()
        .find(|e| e.name == name)
        .ok_or("missing o65 profile export")?;
    if e.segment != 2 {
        return Err("profile export must reference text".into());
    }
    Ok(e.value)
}
fn validate_profile(file: &wire::File, p: &Profile, start: u32) -> Result<(), String> {
    if file.mode != wire::MODE
        || file.bases != [0; 4]
        || file.lengths[3] != 0
        || file.stack != 0
        || file.exports.len() != 2
        || u32::from(p.nmi_extra_stack) + 26 > 65535
    {
        return Err("unsupported o65 application profile".into());
    }
    if export(file, ENTRY)? != p.entry {
        return Err("entry descriptor/export mismatch".into());
    }
    let mut ids = BTreeSet::new();
    let mut occupied = [vec![], vec![], vec![]];
    for r in &p.routines {
        if !ids.insert(r.id)
            || r.size == 0
            || r.size > 65535
            || (r.offset & 65535) + r.size > 65535
            || r.offset.checked_add(r.size).is_none_or(|e| e > start)
            || r.frame > 254
            || r.frame % 2 != 0
            || r.spill > r.frame
            || r.local_peak < u32::from(r.frame)
            || r.local_peak > 65535
            || r.contract.kind != 0
            || r.contract.stack_peak != 0
        {
            return Err("invalid o65 routine/frame map".into());
        }
        if r.contract
            .arguments
            .iter()
            .any(|a| u32::from(r.frame) + 4 + a.offset + a.size > 256)
        {
            return Err("invalid incoming displacement map".into());
        }
        occupied[0].push(Region {
            address: r.offset,
            size: r.size,
        });
    }
    if !p.routines.iter().any(|r| r.offset == p.entry) {
        return Err("entry is not a routine start".into());
    }
    let mut objects = BTreeSet::new();
    for o in &p.objects {
        if o.kind > 2 || !objects.insert((o.kind, o.id)) || ![1, 2, 4].contains(&o.alignment) {
            return Err("invalid object map".into());
        }
        Region {
            address: o.location.offset,
            size: o.size,
        }
        .end()?;
        if let Some(s) = o.location.section {
            let bound = if s == Section::Text {
                start
            } else {
                file.lengths[s.index()]
            };
            if o.location.offset % o.alignment != 0
                || o.location
                    .offset
                    .checked_add(o.size)
                    .is_none_or(|e| e > bound)
                || o.mutable != (s != Section::Text)
            {
                return Err("invalid object placement/permissions".into());
            }
            if !o.alias {
                occupied[s.index()].push(Region {
                    address: o.location.offset,
                    size: o.size,
                });
            }
        }
        if o.alias
            && !p.objects.iter().any(|owner| {
                !owner.alias
                    && owner.location.section == o.location.section
                    && owner.location.offset <= o.location.offset
                    && u64::from(owner.location.offset) + u64::from(owner.size)
                        >= u64::from(o.location.offset) + u64::from(o.size)
            })
        {
            return Err("alias has no containing allocation".into());
        }
    }
    for ranges in occupied {
        disjoint(ranges)?;
    }
    if p.imports.len() != file.imports.len()
        || p.imports.is_empty()
        || p.imports[0].name != OVERFLOW
        || p.imports[0].contract != Contract::overflow()
    {
        return Err("missing raw overflow binding".into());
    }
    for (i, name) in p.imports.iter().zip(&file.imports) {
        if i.name != *name
            || !valid_name(name)
            || (name != OVERFLOW
                && (i.contract.kind != 0 || name == ENTRY || name.starts_with("__a816_o65_")))
        {
            return Err("import descriptor mismatch".into());
        }
    }
    if p.relocations.len() != file.relocations.len() {
        return Err("relocation descriptor count mismatch".into());
    }
    let mut ends = [0u32; 2];
    for (proof, wire) in p.relocations.iter().zip(&file.relocations) {
        if proof.section == Section::Bss
            || proof.encoding == Encoding::Word
            || (proof.zero_extend && proof.encoding != Encoding::Long)
        {
            return Err("unsupported profile relocation".into());
        }
        let mask = match proof.encoding {
            Encoding::Low => 0xff,
            Encoding::High | Encoding::Word => 0xffff,
            _ => 0xffffff,
        };
        if proof.section != wire.section
            || proof.offset != wire.offset
            || proof.encoding != wire.encoding
            || proof.target != wire.target
            || proof.value & mask != wire.value
        {
            return Err("relocation descriptor/payload mismatch".into());
        }
        let index = proof.section.index();
        let end = proof
            .offset
            .checked_add(proof.encoding.width() + u32::from(proof.zero_extend))
            .ok_or("relocation extent overflow")?;
        let payload = if index == 0 { &file.text } else { &file.data };
        if proof.offset < ends[index] || end > if index == 0 { start } else { file.lengths[1] } {
            return Err("relocation overlaps another site or descriptor".into());
        }
        ends[index] = end;
        if proof.zero_extend && payload[(end - 1) as usize] != 0 {
            return Err("nonzero high byte in address container".into());
        }
        match proof.target {
            Reference::Import(i) => {
                if i as usize >= p.imports.len() || proof.value != 0 {
                    return Err("unsupported import addend/index".into());
                }
            }
            Reference::Section(s) => {
                if proof.value > file.lengths[s.index()] {
                    return Err("target offset outside section".into());
                }
            }
        }
    }
    Ok(())
}

fn application(bytes: &[u8]) -> Result<(wire::File, Profile), String> {
    let file = read::decode(bytes)?;
    let start = export(&file, DESCRIPTOR)?;
    let raw = file
        .text
        .get(start as usize..)
        .ok_or("descriptor outside text")?;
    let p = read::descriptor(raw)?;
    validate_profile(&file, &p, start)?;
    Ok((file, p))
}

/// Inspect and validate the self-contained application contract before placement.
pub fn inspect(bytes: &[u8]) -> Result<Profile, String> {
    Ok(application(bytes)?.1)
}

pub fn relocate(bytes: &[u8], placement: &Placement) -> Result<RelocatedImage, String> {
    let (file, p) = application(bytes)?;
    if placement.nmi_extra_stack != p.nmi_extra_stack {
        return Err("platform NMI allowance mismatch".into());
    }
    if placement.allowed.len() > MAX_ITEMS
        || placement.reserved.len() > MAX_ITEMS
        || placement.providers.len() != p.imports.len()
    {
        return Err("platform table count mismatch".into());
    }
    for region in placement.allowed.iter().chain(&placement.reserved) {
        region.end()?;
    }
    let mut allocations = vec![];
    for i in 0..3 {
        let region = Region {
            address: placement.bases[i],
            size: file.lengths[i],
        };
        region.end()?;
        if region.size == 0 {
            if region.address != 0 {
                return Err("empty section must have zero base".into());
            }
            continue;
        }
        if region.address < 65536 || region.address % if i == 0 { 65536 } else { 4 } != 0 {
            return Err("invalid o65 section alignment/bank placement".into());
        }
        if !placement.allowed.iter().any(|a| {
            a.address <= region.address && a.end().is_ok_and(|e| e >= region.end().unwrap())
        }) || placement.reserved.iter().any(|r| r.overlaps(region))
        {
            return Err("section outside allowed RAM or in reserved region".into());
        }
        if p.objects.iter().any(|o| {
            o.location.section.is_none()
                && region.overlaps(Region {
                    address: o.location.offset,
                    size: o.size,
                })
        }) {
            return Err("section overlaps absolute storage".into());
        }
        allocations.push(region);
    }
    let mut addresses = vec![];
    let mut names = BTreeSet::new();
    for provider in &placement.providers {
        if !names.insert(&provider.name) {
            return Err("duplicate provider".into());
        }
    }
    for import in &p.imports {
        let provider = placement
            .providers
            .iter()
            .find(|i| i.name == import.name)
            .ok_or("unresolved o65 import")?;
        if provider.contract != import.contract || provider.size == 0 {
            return Err("incompatible o65 provider contract".into());
        }
        let region = Region {
            address: provider.address,
            size: provider.size,
        };
        region.end()?;
        if placement.reserved.iter().any(|r| r.overlaps(region)) {
            return Err("provider overlaps reserved region".into());
        }
        allocations.push(region);
        addresses.push(provider.address);
    }
    disjoint(allocations)?;
    // Validate full values first. Standard records independently drive patching.
    let mut patches = vec![];
    for (proof, r) in p.relocations.iter().zip(&file.relocations) {
        let base = match r.target {
            Reference::Section(s) => placement.bases[s.index()],
            Reference::Import(i) => addresses[i as usize],
        };
        base.checked_add(proof.value)
            .filter(|v| *v < LIMIT)
            .ok_or("relocated address exceeds 24 bits")?;
        let value = u64::from(base) + u64::from(r.value);
        let selected = match r.encoding {
            Encoding::Low => value & 255,
            Encoding::High => (value >> 8) & 255,
            Encoding::Bank => (value >> 16) & 255,
            Encoding::Word => value & 65535,
            Encoding::Long => value & 0xffffff,
        };
        patches.push((r.section, r.offset, r.encoding.width(), selected as u32));
    }
    let mut text = file.text;
    let mut data = file.data;
    for (section, offset, width, value) in patches {
        let output = if section == Section::Text {
            &mut text
        } else {
            &mut data
        };
        output[offset as usize..(offset + width) as usize]
            .copy_from_slice(&value.to_le_bytes()[..width as usize]);
    }
    let mut segments = vec![];
    let mut routines = p.routines.iter().collect::<Vec<_>>();
    routines.sort_by_key(|r| r.offset);
    let mut cursor = 0usize;
    for r in routines {
        let begin = r.offset as usize;
        let end = begin + r.size as usize;
        if cursor < begin {
            segments.push(Segment {
                address: placement.bases[0] + cursor as u32,
                bytes: text[cursor..begin].to_vec(),
                writable: false,
                executable: false,
            });
        }
        segments.push(Segment {
            address: placement.bases[0] + r.offset,
            bytes: text[begin..end].to_vec(),
            writable: false,
            executable: true,
        });
        cursor = end;
    }
    if cursor < text.len() {
        segments.push(Segment {
            address: placement.bases[0] + cursor as u32,
            bytes: text[cursor..].to_vec(),
            writable: false,
            executable: false,
        });
    }
    if !data.is_empty() {
        segments.push(Segment {
            address: placement.bases[1],
            bytes: data,
            writable: true,
            executable: false,
        });
    }
    let zero_fill = if file.lengths[2] == 0 {
        vec![]
    } else {
        vec![ZeroFill {
            address: placement.bases[2],
            size: file.lengths[2],
            writable: true,
        }]
    };
    Ok(RelocatedImage {
        segments,
        zero_fill,
        entry: placement.bases[0] + p.entry,
        stack_overflow: addresses[0],
        profile: p,
        bases: placement.bases,
    })
}
