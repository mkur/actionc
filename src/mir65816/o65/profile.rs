//! Public contracts are independent of compiler IR and fixed-image JSON.
use serde::{Deserialize, Serialize};

pub const ID: &str = "actionc.o65.experimental.v1";
pub const ENTRY: &str = "__a816_entry_v1";
pub const DESCRIPTOR: &str = "__a816_o65_profile_v1";
pub const OVERFLOW: &str = "__a816_stack_overflow_v1";
pub const LIMIT: u32 = 0x1000000;
pub const MAX_ITEMS: usize = 1_000_000;
pub const MAX_FILE: usize = 128 * 1024 * 1024;
pub const MAX_STRING: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub profile: String,
    pub nmi_extra_stack: u16,
    pub imports: Vec<Binding>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            profile: ID.into(),
            nmi_extra_stack: 0,
            imports: vec![],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub symbol: u32,
    pub name: String,
    pub stack_peak: u16,
    pub checks_stack: bool,
    #[serde(default = "both_domains")]
    pub domains: u8,
    #[serde(default)]
    pub irq_effect: super::super::image::IrqEffect,
}
fn both_domains() -> u8 {
    3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Section {
    Text = 2,
    Data = 3,
    Bss = 4,
}
impl Section {
    pub fn index(self) -> usize {
        self as usize - 2
    }
    pub fn from_byte(v: u8) -> Result<Self, String> {
        match v {
            2 => Ok(Self::Text),
            3 => Ok(Self::Data),
            4 => Ok(Self::Bss),
            _ => Err("invalid profile section".into()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub section: Option<Section>,
    pub offset: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Encoding {
    Low = 0x20,
    High = 0x40,
    Word = 0x80,
    Bank = 0xa0,
    Long = 0xc0,
}
impl Encoding {
    pub fn width(self) -> u32 {
        match self {
            Self::Word => 2,
            Self::Long => 3,
            _ => 1,
        }
    }
    pub fn from_byte(v: u8) -> Result<Self, String> {
        match v {
            0x20 => Ok(Self::Low),
            0x40 => Ok(Self::High),
            0x80 => Ok(Self::Word),
            0xa0 => Ok(Self::Bank),
            0xc0 => Ok(Self::Long),
            _ => Err("unsupported o65 relocation type".into()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    Section(Section),
    Import(u32),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    pub section: Section,
    pub offset: u32,
    pub encoding: Encoding,
    pub target: Reference,
    /// Complete effective offset; enables bounds checks even for split bytes.
    pub value: u32,
    pub zero_extend: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub offset: u32,
    pub size: u32,
    pub alignment: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub abi: String,
    pub signature: u32,
    pub arguments: Vec<Argument>,
    pub result: u8,
    pub incoming: u32,
    pub stack_peak: u16,
    pub irq_effect: u8,
    /// 0 = returning checked task/IRQ scalar routine; 1 = raw overflow entry.
    pub kind: u8,
    pub domains: u8,
}
impl Contract {
    pub fn overflow() -> Self {
        Self {
            abi: super::super::abi::generated::ABI_NAME.into(),
            signature: 0,
            arguments: vec![],
            result: 0,
            incoming: 0,
            stack_peak: 0,
            irq_effect: 0,
            kind: 1,
            domains: 3,
        }
    }
    pub fn verify(&self) -> Result<(), String> {
        if self.abi != super::super::abi::generated::ABI_NAME
            || self.kind > 1
            || self.result > 4
            || self.irq_effect > 2
            || !(1..=3).contains(&self.domains)
        {
            return Err("invalid native import contract".into());
        }
        if self.kind == 1 {
            if self != &Self::overflow() {
                return Err("invalid raw overflow contract".into());
            }
            return Ok(());
        }
        let mut next = 0u32;
        for a in &self.arguments {
            if !(1..=4).contains(&a.size)
                || ![1, 2, 4].contains(&a.alignment)
                || a.offset % a.alignment != 0
                || a.offset < next
            {
                return Err("invalid argument contract".into());
            }
            next = a.offset.checked_add(a.size).ok_or("argument overflow")?;
        }
        // Native argument extent is odd, including a padding byte for no args.
        if self.incoming != (next | 1) || self.incoming > 251 {
            return Err("invalid incoming extent".into());
        }
        if (self.irq_effect == 1 && (!self.arguments.is_empty() || self.result != 1))
            || (self.irq_effect == 2
                && (self.result != 0 || self.arguments.len() != 1 || self.arguments[0].size != 1))
        {
            return Err("IRQ-state import has an incompatible signature".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub name: String,
    pub contract: Contract,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    pub id: u32,
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub contract: Contract,
    pub frame: u16,
    pub spill: u16,
    pub local_peak: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub kind: u8,
    pub id: u32,
    pub name: String,
    pub location: Location,
    pub size: u32,
    pub alignment: u32,
    pub mutable: bool,
    pub alias: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub nmi_extra_stack: u16,
    pub entry: u32,
    pub routines: Vec<Routine>,
    pub objects: Vec<Object>,
    pub imports: Vec<Import>,
    pub relocations: Vec<Relocation>,
}
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_STRING
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
