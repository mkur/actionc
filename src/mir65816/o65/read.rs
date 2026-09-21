use super::{profile::*, wire};
use std::collections::BTreeSet;

pub(crate) struct Reader<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("input length overflow")?;
        let value = self.bytes.get(self.at..end).ok_or("truncated o65 input")?;
        self.at = end;
        Ok(value)
    }
    pub fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn word(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn long(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn sized(&mut self, wide: bool) -> Result<u32, String> {
        if wide {
            self.long()
        } else {
            Ok(u32::from(self.word()?))
        }
    }
    pub fn count(&mut self, min: usize) -> Result<usize, String> {
        let n = self.long()? as usize;
        self.check_count(n, min)?;
        Ok(n)
    }
    pub fn check_count(&self, n: usize, min: usize) -> Result<(), String> {
        if n > MAX_ITEMS || n > (self.bytes.len() - self.at) / min {
            return Err("o65 count exceeds remaining input or limit".into());
        }
        Ok(())
    }
    pub fn string(&mut self) -> Result<String, String> {
        let n = self.long()? as usize;
        if n > MAX_STRING {
            return Err("o65 string limit".into());
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "invalid descriptor UTF-8".into())
    }
    fn name(&mut self) -> Result<String, String> {
        let available = &self.bytes[self.at..];
        let n = available
            .iter()
            .take(MAX_STRING + 1)
            .position(|b| *b == 0)
            .ok_or("unterminated o65 name")?;
        let s = String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "invalid o65 symbol")?;
        self.byte()?;
        if !valid_name(&s) {
            return Err("invalid o65 symbol name".into());
        }
        Ok(s)
    }
    pub fn boolean(&mut self) -> Result<bool, String> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("invalid boolean".into()),
        }
    }
    pub fn done(&self) -> Result<(), String> {
        if self.at != self.bytes.len() {
            Err("trailing o65 bytes".into())
        } else {
            Ok(())
        }
    }
}
/// Decode the standard wire format. Does not admit an application profile.
pub fn decode(bytes: &[u8]) -> Result<wire::File, String> {
    if bytes.len() > MAX_FILE {
        return Err("o65 file size limit".into());
    }
    let mut r = Reader::new(bytes);
    if r.take(6)? != wire::MAGIC {
        return Err("invalid o65 magic".into());
    }
    let mode = r.word()?;
    if mode & 0x8000 == 0 || mode & !0xba03 != 0 {
        return Err("unsupported o65 CPU/mode".into());
    }
    let wide = mode & 0x2000 != 0;
    let mut bases = [0; 4];
    let mut lengths = [0; 4];
    for i in 0..4 {
        bases[i] = r.sized(wide)?;
        lengths[i] = r.sized(wide)?;
        if bases[i] >= LIMIT
            || lengths[i] >= LIMIT
            || bases[i].checked_add(lengths[i]).is_none_or(|v| v > LIMIT)
        {
            return Err("o65 section address/size limit".into());
        }
    }
    let stack = r.sized(wide)?;
    loop {
        let n = r.byte()?;
        if n == 0 {
            break;
        }
        if n < 2 {
            return Err("invalid header option length".into());
        }
        r.take(n as usize - 1)?;
    }
    let text = r.take(lengths[0] as usize)?.to_vec();
    let data = r.take(lengths[1] as usize)?.to_vec();
    let count = r.sized(wide)? as usize;
    r.check_count(count, 2)?;
    let mut imports = vec![];
    let mut names = BTreeSet::new();
    for _ in 0..count {
        let s = r.name()?;
        if !names.insert(s.clone()) {
            return Err("duplicate o65 import".into());
        }
        imports.push(s);
    }
    let mut relocations = vec![];
    for (section, payload) in [(Section::Text, &text), (Section::Data, &data)] {
        let mut at = -1i64;
        let mut end = 0u32;
        let mut pending_skip = false;
        loop {
            let delta = r.byte()?;
            if delta == 0 {
                if pending_skip {
                    return Err("unterminated relocation skip".into());
                }
                break;
            }
            at = at
                .checked_add(if delta == 255 { 254 } else { i64::from(delta) })
                .ok_or("relocation offset overflow")?;
            if at >= payload.len() as i64 {
                return Err("relocation outside initialized section".into());
            }
            if delta == 255 {
                pending_skip = true;
                continue;
            }
            pending_skip = false;
            let tag = r.byte()?;
            let encoding = Encoding::from_byte(tag & 0xe0)?;
            let target = match tag & 0x1f {
                0 => {
                    let i = r.sized(wide)?;
                    if i as usize >= imports.len() {
                        return Err("invalid import index".into());
                    }
                    Reference::Import(i)
                }
                v => Reference::Section(Section::from_byte(v)?),
            };
            let offset = at as u32;
            if offset < end {
                return Err("overlapping relocation sites".into());
            }
            end = offset
                .checked_add(encoding.width())
                .ok_or("relocation overflow")?;
            let field = payload
                .get(offset as usize..end as usize)
                .ok_or("relocation exceeds initialized storage")?;
            let mut value = 0u32;
            for (i, b) in field.iter().enumerate() {
                value |= u32::from(*b) << (8 * i);
            }
            if encoding == Encoding::High {
                value = (value << 8) | u32::from(r.byte()?);
            }
            if encoding == Encoding::Bank {
                value = (value << 16) | u32::from(r.word()?);
            }
            if relocations.len() >= MAX_ITEMS {
                return Err("relocation count limit".into());
            }
            relocations.push(Relocation {
                section,
                offset,
                encoding,
                target,
                value,
                zero_extend: false,
            });
        }
    }
    let count = r.sized(wide)? as usize;
    r.check_count(count, if wide { 7 } else { 5 })?;
    let mut exports = vec![];
    names.clear();
    for _ in 0..count {
        let name = r.name()?;
        let segment = r.byte()?;
        let value = r.sized(wide)?;
        if !names.insert(name.clone()) || !(1..=4).contains(&segment) {
            return Err("invalid or duplicate o65 export".into());
        }
        if segment > 1 {
            let i = (segment - 2) as usize;
            if value < bases[i] || value > bases[i] + lengths[i] {
                return Err("export outside section".into());
            }
        }
        exports.push(wire::Export {
            name,
            segment,
            value,
        });
    }
    r.done()?;
    Ok(wire::File {
        mode,
        bases,
        lengths,
        stack,
        text,
        data,
        imports,
        relocations,
        exports,
    })
}
fn contract(r: &mut Reader<'_>) -> Result<Contract, String> {
    let abi = r.string()?;
    let signature = r.long()?;
    let n = r.count(12)?;
    let mut arguments = vec![];
    for _ in 0..n {
        arguments.push(Argument {
            offset: r.long()?,
            size: r.long()?,
            alignment: r.long()?,
        });
    }
    let c = Contract {
        abi,
        signature,
        arguments,
        result: r.byte()?,
        incoming: r.long()?,
        stack_peak: r.word()?,
        irq_effect: r.byte()?,
        kind: r.byte()?,
        domains: r.byte()?,
    };
    c.verify()?;
    Ok(c)
}
pub(crate) fn descriptor(bytes: &[u8]) -> Result<Profile, String> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != b"A8O1"
        || r.long()? as usize != bytes.len()
        || r.word()? != 1
        || r.word()? != 0
        || r.byte()? != 3
        || r.byte()? != 0
    {
        return Err("unsupported o65 profile descriptor".into());
    }
    let nmi_extra_stack = r.word()?;
    let entry = r.long()?;
    let n = r.count(46)?;
    let mut routines = vec![];
    for _ in 0..n {
        routines.push(Routine {
            id: r.long()?,
            name: r.string()?,
            offset: r.long()?,
            size: r.long()?,
            contract: contract(&mut r)?,
            frame: r.word()?,
            spill: r.word()?,
            local_peak: r.long()?,
        });
    }
    let n = r.count(24)?;
    let mut objects = vec![];
    for _ in 0..n {
        let kind = r.byte()?;
        let id = r.long()?;
        let name = r.string()?;
        let tag = r.byte()?;
        let section = if tag == 0 {
            None
        } else {
            Some(Section::from_byte(tag)?)
        };
        objects.push(Object {
            kind,
            id,
            name,
            location: Location {
                section,
                offset: r.long()?,
            },
            size: r.long()?,
            alignment: r.long()?,
            mutable: r.boolean()?,
            alias: r.boolean()?,
        });
    }
    let n = r.count(26)?;
    let mut imports = vec![];
    for _ in 0..n {
        imports.push(Import {
            name: r.string()?,
            contract: contract(&mut r)?,
        });
    }
    let n = r.count(16)?;
    let mut relocations = vec![];
    for _ in 0..n {
        let section = Section::from_byte(r.byte()?)?;
        let offset = r.long()?;
        let encoding = Encoding::from_byte(r.byte()?)?;
        let tag = r.byte()?;
        let i = r.long()?;
        let target = if tag == 0 {
            Reference::Import(i)
        } else {
            if i != 0 {
                return Err("invalid section relocation index".into());
            }
            Reference::Section(Section::from_byte(tag)?)
        };
        relocations.push(Relocation {
            section,
            offset,
            encoding,
            target,
            value: r.long()?,
            zero_extend: r.boolean()?,
        });
    }
    r.done()?;
    Ok(Profile {
        nmi_extra_stack,
        entry,
        routines,
        objects,
        imports,
        relocations,
    })
}
