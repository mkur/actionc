//! Independent bounded loader for the classic executable subset used by tests.
//! Uses only file bytes and chosen allocation bases, never compiler linker data.
use crate::{MEMORY_SIZE, Machine, Memory, STACK_BOTTOM, STACK_TOP};
use std::collections::BTreeSet;

#[derive(Debug)]
pub struct Segment {
    pub kind: u32,
    pub bytes: Vec<u8>,
}
#[derive(Debug)]
pub struct File {
    segments: Vec<Segment>,
    relocations: Vec<(usize, usize, usize)>,
}

impl File {
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        // The harness is deliberately more bounded than the 24-bit machine.
        if bytes.len() > 32 * MEMORY_SIZE || bytes.len() % 4 != 0 {
            return Err("invalid HUNK file size".into());
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.word()? != 1011 || reader.word()? != 0 {
            return Err("unsupported HUNK header or resident libraries".into());
        }
        let count = reader.word()? as usize;
        if !(1..=3).contains(&count) || reader.word()? != 0 || reader.word()? as usize != count - 1
        {
            return Err("unsupported HUNK allocation table".into());
        }
        let mut sizes = Vec::new();
        let mut total = 0usize;
        for _ in 0..count {
            let words = reader.word()?;
            if words & 0xc0000000 != 0 {
                return Err("HUNK memory classes are unsupported".into());
            }
            let size = (words as usize)
                .checked_mul(4)
                .ok_or("HUNK size overflow")?;
            total = total.checked_add(size).ok_or("HUNK total overflow")?;
            if total > MEMORY_SIZE {
                return Err("HUNK allocation exceeds harness RAM".into());
            }
            sizes.push(size);
        }
        let mut file = Self {
            segments: vec![],
            relocations: vec![],
        };
        let mut patched = BTreeSet::new();
        for (source, &size) in sizes.iter().enumerate() {
            let kind = reader.word()?;
            if !matches!(kind, 1001..=1003)
                || (source == 0 && kind != 1001)
                || (source != 0 && kind == 1001)
            {
                return Err("unsupported HUNK segment type or entry".into());
            }
            let payload = (reader.word()? as usize)
                .checked_mul(4)
                .ok_or("HUNK payload overflow")?;
            // Writer uses full payload/BSS extents, including deterministic padding.
            if payload != size || source == 0 && size < 4 {
                return Err("inconsistent HUNK payload extent".into());
            }
            let data = if kind == 1003 {
                vec![0; size]
            } else {
                reader.take(size)?.to_vec()
            };
            loop {
                match reader.word()? {
                    1010 => break,
                    1004 => loop {
                        let n = reader.word()? as usize;
                        if n == 0 {
                            break;
                        }
                        if n > 65535 {
                            return Err("oversized HUNK relocation group".into());
                        }
                        let target = reader.word()? as usize;
                        if target >= count {
                            return Err("missing HUNK relocation target".into());
                        }
                        for _ in 0..n {
                            let offset = reader.word()? as usize;
                            if offset & 1 != 0 || offset.checked_add(4).is_none_or(|end| end > size)
                            {
                                return Err("invalid HUNK relocation extent or alignment".into());
                            }
                            for byte in offset..offset + 4 {
                                if !patched.insert((source, byte)) {
                                    return Err("overlapping HUNK relocations".into());
                                }
                            }
                            file.relocations.push((source, target, offset));
                        }
                    },
                    _ => return Err("unsupported HUNK record or missing terminator".into()),
                }
            }
            file.segments.push(Segment { kind, bytes: data });
        }
        if reader.offset != bytes.len() {
            return Err("trailing HUNK data".into());
        }
        Ok(file)
    }

    pub fn load(&self, bases: &[u32]) -> Result<Machine, String> {
        if bases.len() != self.segments.len() {
            return Err("HUNK load-base count mismatch".into());
        }
        for (segment, &base) in self.segments.iter().zip(bases) {
            let end = base
                .checked_add(segment.bytes.len() as u32)
                .ok_or("HUNK allocation overflow")?;
            if base & 3 != 0
                || base < 0x1000
                || end > 0x01000000
                || end as usize > MEMORY_SIZE
                || base < STACK_TOP && end > STACK_BOTTOM
            {
                return Err("HUNK allocation violates address or reserved-memory bounds".into());
            }
        }
        let mut data: Vec<_> = self.segments.iter().map(|s| s.bytes.clone()).collect();
        for &(source, target, offset) in &self.relocations {
            let cell = &mut data[source][offset..offset + 4];
            let value = u32::from_be_bytes(cell.try_into().unwrap())
                .checked_add(bases[target])
                .filter(|v| *v < 0x01000000)
                .ok_or("HUNK relocated address overflow")?;
            cell.copy_from_slice(&value.to_be_bytes());
        }
        let mut memory = Memory::default();
        for ((segment, bytes), &base) in self.segments.iter().zip(data).zip(bases) {
            memory.map(base, &bytes, segment.kind != 1001, segment.kind == 1001)?;
        }
        Machine::new(memory, bases[0])
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn word(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or("HUNK read overflow")?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or("truncated HUNK file")?;
        self.offset = end;
        Ok(bytes)
    }
}
