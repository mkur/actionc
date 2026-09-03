use std::fmt;
use std::str::FromStr;

/// A byte count in the compiler's target-independent layout model.
///
/// This is deliberately wider than Action!'s 16-bit `CARD`: object sizes are
/// compiler facts and must not inherit a source scalar's range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteSize(u32);

impl ByteSize {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);

    pub const fn new(bytes: u32) -> Self {
        Self(bytes)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub fn as_usize(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }

    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn checked_mul(self, count: u32) -> Option<Self> {
        match self.0.checked_mul(count) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_power_of_two(self) -> bool {
        self.0.is_power_of_two()
    }
}

/// A byte displacement within an object or data image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(u32);

impl ByteOffset {
    pub const ZERO: Self = Self(0);

    pub const fn new(bytes: u32) -> Self {
        Self(bytes)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub fn as_usize(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }

    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn checked_add_size(self, size: ByteSize) -> Option<Self> {
        match self.0.checked_add(size.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl From<u8> for ByteSize {
    fn from(value: u8) -> Self {
        Self(u32::from(value))
    }
}

impl From<u16> for ByteSize {
    fn from(value: u16) -> Self {
        Self(u32::from(value))
    }
}

impl TryFrom<usize> for ByteSize {
    type Error = std::num::TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self)
    }
}

impl From<ByteSize> for u32 {
    fn from(value: ByteSize) -> Self {
        value.0
    }
}

impl From<ByteSize> for u64 {
    fn from(value: ByteSize) -> Self {
        u64::from(value.0)
    }
}

impl From<ByteSize> for usize {
    fn from(value: ByteSize) -> Self {
        value.0 as Self
    }
}

impl From<u8> for ByteOffset {
    fn from(value: u8) -> Self {
        Self(u32::from(value))
    }
}

impl From<u16> for ByteOffset {
    fn from(value: u16) -> Self {
        Self(u32::from(value))
    }
}

impl TryFrom<usize> for ByteOffset {
    type Error = std::num::TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self)
    }
}

impl From<ByteOffset> for u32 {
    fn from(value: ByteOffset) -> Self {
        value.0
    }
}

impl From<ByteOffset> for u64 {
    fn from(value: ByteOffset) -> Self {
        u64::from(value.0)
    }
}

impl From<ByteOffset> for usize {
    fn from(value: ByteOffset) -> Self {
        value.0 as Self
    }
}

impl TryFrom<ByteSize> for u16 {
    type Error = std::num::TryFromIntError;

    fn try_from(value: ByteSize) -> Result<Self, Self::Error> {
        u16::try_from(value.0)
    }
}

impl TryFrom<ByteOffset> for u16 {
    type Error = std::num::TryFromIntError;

    fn try_from(value: ByteOffset) -> Result<Self, Self::Error> {
        u16::try_from(value.0)
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl fmt::Display for ByteOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuId {
    Mos6502,
    Wdc65816,
    Motorola68000,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformId {
    Atari8Bit,
    Generic65816,
    Generic68k,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbiId {
    AtariActionPacked,
    Wdc65816Native,
    Wdc65816Small,
    Motorola68kNative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TargetId {
    #[default]
    Atari6502,
    Wdc65816Native,
    Wdc65816Small,
    Motorola68000,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AddressSpaceId(pub u8);

/// A target address. Absolute addresses are never represented as Action!
/// `CARD` values in NIR, even when both happen to be 16-bit on Atari.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AddressValue {
    pub address_space: AddressSpaceId,
    pub value: u64,
}

impl AddressValue {
    pub const fn new(address_space: AddressSpaceId, value: u64) -> Self {
        Self {
            address_space,
            value,
        }
    }

    pub const fn data(value: u64) -> Self {
        Self::new(TargetLayout::DATA_ADDRESS_SPACE, value)
    }

    pub const fn code(value: u64) -> Self {
        Self::new(TargetLayout::CODE_ADDRESS_SPACE, value)
    }

    pub const fn checked_add_signed(self, addend: i64) -> Option<Self> {
        match self.value.checked_add_signed(addend) {
            Some(value) => Some(Self::new(self.address_space, value)),
            None => None,
        }
    }
}

impl fmt::UpperHex for AddressValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointerLayout {
    pub address_space: AddressSpaceId,
    pub size_bytes: ByteSize,
    pub alignment_bytes: ByteSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordLayoutPolicy {
    Packed,
    Natural,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetLayout {
    pub target: TargetId,
    pub cpu: CpuId,
    pub platform: PlatformId,
    pub abi: AbiId,
    pub endian: Endian,
    pub address_bits: u8,
    pub link_address_bits: u8,
    pub data_pointer: PointerLayout,
    pub code_pointer: PointerLayout,
    pub record_layout: RecordLayoutPolicy,
    pub natural_word_alignment_bytes: u8,
}

impl TargetLayout {
    pub const DATA_ADDRESS_SPACE: AddressSpaceId = AddressSpaceId(0);
    pub const CODE_ADDRESS_SPACE: AddressSpaceId = AddressSpaceId(1);

    pub const fn for_target(target: TargetId) -> Self {
        match target {
            TargetId::Atari6502 => Self::atari_6502(),
            TargetId::Wdc65816Native => Self::wdc_65816_native(),
            TargetId::Wdc65816Small => Self::wdc_65816_small(),
            TargetId::Motorola68000 => Self::motorola_68000(),
        }
    }

    pub const fn atari_6502() -> Self {
        Self {
            target: TargetId::Atari6502,
            cpu: CpuId::Mos6502,
            platform: PlatformId::Atari8Bit,
            abi: AbiId::AtariActionPacked,
            endian: Endian::Little,
            address_bits: 16,
            link_address_bits: 16,
            data_pointer: PointerLayout {
                address_space: Self::DATA_ADDRESS_SPACE,
                size_bytes: ByteSize::new(2),
                alignment_bytes: ByteSize::new(1),
            },
            code_pointer: PointerLayout {
                address_space: Self::CODE_ADDRESS_SPACE,
                size_bytes: ByteSize::new(2),
                alignment_bytes: ByteSize::new(1),
            },
            record_layout: RecordLayoutPolicy::Packed,
            natural_word_alignment_bytes: 1,
        }
    }

    pub const fn wdc_65816_native() -> Self {
        Self {
            target: TargetId::Wdc65816Native,
            cpu: CpuId::Wdc65816,
            platform: PlatformId::Generic65816,
            abi: AbiId::Wdc65816Native,
            endian: Endian::Little,
            address_bits: 24,
            link_address_bits: 24,
            data_pointer: PointerLayout {
                address_space: Self::DATA_ADDRESS_SPACE,
                size_bytes: ByteSize::new(3),
                alignment_bytes: ByteSize::new(1),
            },
            code_pointer: PointerLayout {
                address_space: Self::CODE_ADDRESS_SPACE,
                size_bytes: ByteSize::new(3),
                alignment_bytes: ByteSize::new(1),
            },
            record_layout: RecordLayoutPolicy::Natural,
            natural_word_alignment_bytes: 2,
        }
    }

    pub const fn wdc_65816_small() -> Self {
        Self {
            target: TargetId::Wdc65816Small,
            cpu: CpuId::Wdc65816,
            platform: PlatformId::Generic65816,
            abi: AbiId::Wdc65816Small,
            endian: Endian::Little,
            address_bits: 24,
            link_address_bits: 24,
            data_pointer: PointerLayout {
                address_space: Self::DATA_ADDRESS_SPACE,
                size_bytes: ByteSize::new(2),
                alignment_bytes: ByteSize::new(1),
            },
            code_pointer: PointerLayout {
                address_space: Self::CODE_ADDRESS_SPACE,
                size_bytes: ByteSize::new(2),
                alignment_bytes: ByteSize::new(1),
            },
            record_layout: RecordLayoutPolicy::Natural,
            natural_word_alignment_bytes: 2,
        }
    }

    pub const fn motorola_68000() -> Self {
        Self {
            target: TargetId::Motorola68000,
            cpu: CpuId::Motorola68000,
            platform: PlatformId::Generic68k,
            abi: AbiId::Motorola68kNative,
            endian: Endian::Big,
            address_bits: 32,
            link_address_bits: 24,
            data_pointer: PointerLayout {
                address_space: Self::DATA_ADDRESS_SPACE,
                size_bytes: ByteSize::new(4),
                alignment_bytes: ByteSize::new(2),
            },
            code_pointer: PointerLayout {
                address_space: Self::CODE_ADDRESS_SPACE,
                size_bytes: ByteSize::new(4),
                alignment_bytes: ByteSize::new(2),
            },
            record_layout: RecordLayoutPolicy::Natural,
            natural_word_alignment_bytes: 2,
        }
    }
}

impl Default for TargetLayout {
    fn default() -> Self {
        Self::atari_6502()
    }
}

impl TargetId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Atari6502 => "atari-6502",
            Self::Wdc65816Native => "wdc-65816-native",
            Self::Wdc65816Small => "wdc-65816-small",
            Self::Motorola68000 => "motorola-68000",
        }
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TargetId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "atari" | "atari-6502" | "6502" => Ok(Self::Atari6502),
            "65816" | "65816-native" | "wdc-65816-native" => Ok(Self::Wdc65816Native),
            "65816-small" | "wdc-65816-small" => Ok(Self::Wdc65816Small),
            "68k" | "68000" | "m68000" | "motorola-68000" => Ok(Self::Motorola68000),
            _ => Err(format!(
                "unknown target `{value}`; expected atari-6502, wdc-65816-native, wdc-65816-small, or motorola-68000"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_matrix_keeps_action_scalars_separate_from_pointer_width() {
        let atari = TargetLayout::atari_6502();
        let native = TargetLayout::wdc_65816_native();
        let small = TargetLayout::wdc_65816_small();
        let m68k = TargetLayout::motorola_68000();

        assert_eq!(
            (atari.address_bits, atari.data_pointer.size_bytes),
            (16, ByteSize::new(2))
        );
        assert_eq!(
            (native.address_bits, native.data_pointer.size_bytes),
            (24, ByteSize::new(3))
        );
        assert_eq!(
            (small.address_bits, small.data_pointer.size_bytes),
            (24, ByteSize::new(2))
        );
        assert_eq!(
            (m68k.address_bits, m68k.data_pointer.size_bytes),
            (32, ByteSize::new(4))
        );
        assert_eq!(m68k.link_address_bits, 24);
        assert_eq!(m68k.endian, Endian::Big);
    }
}
