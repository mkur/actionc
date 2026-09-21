//! Standard o65 wire representation; profile admission is a separate operation.
use super::profile::*;
use std::collections::BTreeSet;
pub const MODE: u16 = 0xa202;
pub const MAGIC: &[u8; 6] = b"\x01\x00o65\x00";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub name: String,
    pub segment: u8,
    pub value: u32,
}
#[derive(Debug, Clone)]
pub struct File {
    pub mode: u16,
    pub bases: [u32; 4],
    pub lengths: [u32; 4],
    pub stack: u32,
    pub text: Vec<u8>,
    pub data: Vec<u8>,
    pub imports: Vec<String>,
    pub relocations: Vec<Relocation>,
    pub exports: Vec<Export>,
}
fn string(bytes: &mut Vec<u8>, s: &str) -> Result<(), String> {
    if !valid_name(s) {
        return Err("invalid o65 symbol name".into());
    }
    bytes.extend(s.as_bytes());
    bytes.push(0);
    Ok(())
}
pub fn encode(file: &File) -> Result<Vec<u8>, String> {
    if file.mode != MODE
        || file.bases.iter().any(|b| *b >= LIMIT)
        || file.lengths.iter().any(|n| *n >= LIMIT)
        || file.lengths[0] as usize != file.text.len()
        || file.lengths[1] as usize != file.data.len()
        || file.lengths[3] != 0
    {
        return Err("unsupported o65 header".into());
    }
    if file.imports.len() > MAX_ITEMS
        || file.exports.len() > MAX_ITEMS
        || file.relocations.len() > MAX_ITEMS
    {
        return Err("o65 table limit".into());
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend(file.mode.to_le_bytes());
    for i in 0..4 {
        bytes.extend(file.bases[i].to_le_bytes());
        bytes.extend(file.lengths[i].to_le_bytes());
    }
    bytes.extend(file.stack.to_le_bytes());
    bytes.push(0);
    bytes.extend(&file.text);
    bytes.extend(&file.data);
    bytes.extend((file.imports.len() as u32).to_le_bytes());
    let mut names = BTreeSet::new();
    for name in &file.imports {
        if !names.insert(name) {
            return Err("duplicate o65 import".into());
        }
        string(&mut bytes, name)?;
    }
    let mut relocs = file.relocations.iter().collect::<Vec<_>>();
    relocs.sort_by_key(|r| (r.section as u8, r.offset));
    if relocs.iter().any(|r| r.section == Section::Bss) {
        return Err("relocation in BSS".into());
    }
    for section in [Section::Text, Section::Data] {
        let mut last = -1i64;
        let mut end = 0u32;
        for r in relocs.iter().filter(|r| r.section == section) {
            let next = r
                .offset
                .checked_add(r.encoding.width() + u32::from(r.zero_extend))
                .ok_or("relocation extent overflow")?;
            if r.offset < end || next > file.lengths[section.index()] {
                return Err("overlapping or out-of-range relocation".into());
            }
            end = next;
            let mut delta = i64::from(r.offset) - last;
            last = i64::from(r.offset);
            while delta > 254 {
                bytes.push(255);
                delta -= 254;
            }
            bytes.push(delta as u8);
            bytes.push(
                r.encoding as u8
                    | match r.target {
                        Reference::Import(i) => {
                            if i as usize >= file.imports.len() {
                                return Err("invalid import index".into());
                            }
                            0
                        }
                        Reference::Section(s) => s as u8,
                    },
            );
            if let Reference::Import(i) = r.target {
                bytes.extend(i.to_le_bytes());
            }
            match r.encoding {
                Encoding::High => bytes.push(r.value as u8),
                Encoding::Bank => bytes.extend((r.value as u16).to_le_bytes()),
                _ => {}
            }
        }
        bytes.push(0);
    }
    bytes.extend((file.exports.len() as u32).to_le_bytes());
    names.clear();
    for e in &file.exports {
        if !names.insert(&e.name) || !(1..=4).contains(&e.segment) {
            return Err("invalid or duplicate export".into());
        }
        string(&mut bytes, &e.name)?;
        bytes.push(e.segment);
        bytes.extend(e.value.to_le_bytes());
    }
    if bytes.len() > MAX_FILE {
        return Err("o65 file size limit".into());
    }
    Ok(bytes)
}
