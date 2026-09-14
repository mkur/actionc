//! Record inspection using compiler-reported array extents and queried offsets.
#![allow(dead_code)]
use actionc::{compiler::native::artifacts::NativeArtifact, mir68k::image::SymbolView};
use actionc_vm68k_tests::Machine;

pub fn read(vm: &Machine, address: u32, width: u32) -> u32 {
    assert!(matches!(width, 1 | 2 | 4));
    vm.cpu
        .mem
        .bytes(address, width as usize)
        .unwrap()
        .iter()
        .fold(0, |v, b| v << 8 | u32::from(*b))
}
pub fn write(vm: &mut Machine, address: u32, width: u32, value: u32) {
    assert!(matches!(width, 1 | 2 | 4));
    vm.cpu
        .mem
        .write(address, &value.to_be_bytes()[4 - width as usize..])
        .unwrap();
}
pub fn scalar(vm: &Machine, image: &NativeArtifact, name: &str) -> u32 {
    vm.read_scalar(image.symbol(name).unwrap()).unwrap()
}
pub fn set(vm: &mut Machine, image: &NativeArtifact, name: &str, value: u32) {
    vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
}

#[derive(Clone, Copy, Debug)]
pub struct Records {
    pub base: u32,
    pub count: u32,
    pub stride: u32,
}
impl Records {
    pub fn new(vm: &Machine, image: &NativeArtifact, name: &str, count: u32, size: u32) -> Self {
        let symbol = image.symbol(name).unwrap();
        let array = symbol.array.as_ref().unwrap();
        assert_eq!(array.count, Some(count));
        assert_eq!(array.element_width, size);
        assert!(array.stride >= size);
        let base = if array.descriptor {
            read(vm, symbol.address().unwrap(), 4)
        } else {
            symbol.address().unwrap()
        };
        vm.cpu
            .mem
            .bytes(base, (count * array.stride) as usize)
            .unwrap();
        Self {
            base,
            count,
            stride: array.stride,
        }
    }
    pub fn address(self, index: u32, offset: u32, size: u32) -> u32 {
        assert!(index < self.count);
        assert!(
            offset
                .checked_add(size)
                .is_some_and(|end| end <= self.stride)
        );
        self.base
            .checked_add(index.checked_mul(self.stride).unwrap())
            .unwrap()
            .checked_add(offset)
            .unwrap()
    }
    pub fn read(self, vm: &Machine, index: u32, offset: u32, width: u32) -> u32 {
        read(vm, self.address(index, offset, width), width)
    }
    pub fn write(self, vm: &mut Machine, index: u32, offset: u32, width: u32, value: u32) {
        write(vm, self.address(index, offset, width), width, value);
    }
    pub fn from_wire(self, address: u32, wire_base: u32, wire_stride: u32) -> Result<u32, String> {
        match slot(address, wire_base, wire_stride, self.count)? {
            None => Ok(0),
            Some(index) => self
                .base
                .checked_add(index.checked_mul(self.stride).ok_or("pointer overflow")?)
                .ok_or_else(|| "pointer overflow".into()),
        }
    }
    pub fn to_wire(self, address: u32, wire_base: u32, wire_stride: u32) -> Result<u32, String> {
        match slot(address, self.base, self.stride, self.count)? {
            None => Ok(0),
            Some(index) => wire_base
                .checked_add(index.checked_mul(wire_stride).ok_or("pointer overflow")?)
                .ok_or_else(|| "pointer overflow".into()),
        }
    }
    pub fn check_padding(self, vm: &Machine, fields: &[(u32, u32)], expected: u8) {
        let mut covered = vec![false; self.stride as usize];
        for &(offset, size) in fields {
            self.address(0, offset, size);
            for byte in &mut covered[offset as usize..(offset + size) as usize] {
                assert!(!*byte);
                *byte = true;
            }
        }
        for index in 0..self.count {
            for (offset, covered) in covered.iter().enumerate() {
                if !covered {
                    assert_eq!(
                        vm.cpu
                            .mem
                            .bytes(self.address(index, offset as u32, 1), 1)
                            .unwrap(),
                        &[expected],
                        "padding: {index}/{offset}"
                    );
                }
            }
        }
    }
}

pub fn slot(address: u32, base: u32, stride: u32, count: u32) -> Result<Option<u32>, String> {
    if address == 0 {
        return Ok(None);
    }
    if stride == 0 {
        return Err("zero record stride".into());
    }
    let offset = address.checked_sub(base).ok_or("pointer below pool")?;
    if offset % stride != 0 || offset / stride >= count {
        return Err("pointer outside pool or inside a record".into());
    }
    Ok(Some(offset / stride))
}
