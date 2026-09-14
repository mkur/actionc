//! Read the small section/symbol manifest produced by compare_mir68k_c.py.
//! GNU binutils owns ELF decoding; no ELF or compiler dependency is added here.
use actionc::{
    mir68k::image::{NativeImage, Segment, ZeroFill},
    target::{TargetId, TargetLayout},
};
use actionc_vm68k_tests::Machine;
use std::{collections::BTreeMap, path::Path};

pub struct Program {
    pub image: NativeImage,
    symbols: Option<BTreeMap<String, (u32, u32)>>,
}
impl Program {
    pub fn action(image: NativeImage) -> Self {
        Self {
            image,
            symbols: None,
        }
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text, |file| {
            std::fs::read(path.parent().unwrap().join(file)).map_err(|e| e.to_string())
        })
    }
    fn parse(text: &str, read: impl Fn(&str) -> Result<Vec<u8>, String>) -> Result<Self, String> {
        let number = |s: &str| s.parse::<u32>().map_err(|e| e.to_string());
        let address = |s: &str| u32::from_str_radix(s, 16).map_err(|e| e.to_string());
        let mut lines = text.lines();
        if lines.next() != Some("m68k-c-reference-v1") {
            return Err("unsupported C image manifest".into());
        }
        let mut image = NativeImage {
            target_layout: TargetLayout::for_target(TargetId::Motorola68000),
            entry: 0,
            segments: vec![],
            zero_fill: vec![],
            symbols: vec![],
        };
        let mut entry = None;
        let mut symbols = BTreeMap::new();
        for line in lines {
            let fields: Vec<_> = line.split_whitespace().collect();
            match fields.as_slice() {
                ["entry", value] if entry.is_none() => entry = Some(address(value)?),
                ["segment", at, size, flags, file] if matches!(*flags, "r" | "rx" | "rw") => {
                    if file.contains(['/', '\\']) || matches!(*file, "." | "..") {
                        return Err("segment must name a file beside the manifest".into());
                    }
                    let bytes = read(file)?;
                    if bytes.len() != number(size)? as usize {
                        return Err("segment length mismatch".into());
                    }
                    image.segments.push(Segment {
                        address: address(at)?,
                        bytes,
                        writable: *flags == "rw",
                        executable: *flags == "rx",
                    });
                }
                ["zero", at, size] => image.zero_fill.push(ZeroFill {
                    address: address(at)?,
                    size: number(size)?,
                    writable: true,
                }),
                ["symbol", name, at, size] => {
                    if symbols
                        .insert((*name).into(), (address(at)?, number(size)?))
                        .is_some()
                    {
                        return Err("duplicate C symbol".into());
                    }
                }
                _ => return Err(format!("invalid C image record: {line}")),
            }
        }
        image.entry = entry.ok_or("missing C entry")?;
        image.verify()?;
        for &(at, size) in symbols.values() {
            let end = at.checked_add(size).ok_or("C symbol extent overflow")?;
            if size == 0
                || !image
                    .segments
                    .iter()
                    .any(|s| at >= s.address && end <= s.address + s.bytes.len() as u32)
                    && !image
                        .zero_fill
                        .iter()
                        .any(|s| at >= s.address && end <= s.address + s.size)
            {
                return Err("C symbol is outside mapped storage".into());
            }
        }
        Ok(Self {
            image,
            symbols: Some(symbols),
        })
    }
    pub fn code_bytes(&self) -> usize {
        self.image
            .segments
            .iter()
            .filter(|s| s.executable)
            .map(|s| s.bytes.len())
            .sum()
    }
    pub fn machine(&self) -> Machine {
        Machine::from_image(&self.image).unwrap()
    }
    fn location(&self, name: &str, width: usize, count: usize) -> u32 {
        assert!(matches!(width, 1 | 2 | 4));
        if let Some(symbols) = &self.symbols {
            let &(address, size) = symbols
                .get(name)
                .unwrap_or_else(|| panic!("missing C symbol {name}"));
            assert_eq!(size as usize, width * count, "C {name} extent");
            address
        } else {
            let symbol = self.image.symbol(name).unwrap();
            if let Some(array) = &symbol.array {
                assert_eq!(array.element_width as usize, width);
                assert_eq!(array.stride as usize, width);
                assert_eq!(array.count, Some(count as u32));
                if array.descriptor {
                    array.backing_address.unwrap()
                } else {
                    symbol.address().unwrap()
                }
            } else {
                assert_eq!(symbol.size as usize, width * count, "Action {name} extent");
                symbol.address().unwrap()
            }
        }
    }
    pub fn write(&self, vm: &mut Machine, name: &str, width: usize, values: &[u32]) {
        let address = self.location(name, width, values.len());
        let bytes: Vec<_> = values
            .iter()
            .flat_map(|v| v.to_be_bytes()[4 - width..].to_vec())
            .collect();
        vm.cpu.mem.write(address, &bytes).unwrap();
    }
    pub fn check(&self, vm: &Machine, name: &str, width: usize, expected: &[u32]) {
        let address = self.location(name, width, expected.len());
        let bytes = vm.cpu.mem.bytes(address, width * expected.len()).unwrap();
        let actual: Vec<u32> = bytes
            .chunks_exact(width)
            .map(|b| b.iter().fold(0, |v, byte| v << 8 | u32::from(*byte)))
            .collect();
        assert_eq!(actual, expected, "{name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MANIFEST: &str = "m68k-c-reference-v1\nentry 10000\nsegment 10000 6 rx code.bin\nzero 11000 4\nsymbol result 11000 4\n";
    fn read(_: &str) -> Result<Vec<u8>, String> {
        Ok(vec![0x4e, 0x75, 0x4e, 0x71, 0x4e, 0x71])
    }
    #[test]
    fn both_line_endings_map_code_zero_fill_and_symbols_into_the_real_vm() {
        for text in [MANIFEST.into(), MANIFEST.replace('\n', "\r\n")] {
            let program = Program::parse(&text, read).unwrap();
            let mut vm = program.machine();
            assert!(vm.cpu.mem.write(0x10000, &[0]).is_err());
            program.check(&vm, "result", 4, &[0]);
            program.write(&mut vm, "result", 4, &[0x12345678]);
            vm.run(100).assert_completed();
            program.check(&vm, "result", 4, &[0x12345678]);
        }
    }
    #[test]
    fn malformed_or_overlapping_images_are_rejected_before_execution() {
        for text in [
            MANIFEST.replace("10000 6", "10000 8"),
            MANIFEST.replace("11000", "10000"),
            MANIFEST.replace("code.bin", "../code.bin"),
            format!("{MANIFEST}entry 10000\n"),
            format!("{MANIFEST}symbol result 11000 4\n"),
            MANIFEST.replace("symbol result 11000", "symbol result 12000"),
            MANIFEST.replace("entry 10000", "entry 10001"),
        ] {
            assert!(Program::parse(&text, read).is_err(), "{text}");
        }
    }
}
