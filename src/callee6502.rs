//! Implementation identity for reviewed 6502 leaf contracts. A successful
//! match certifies the recorded claims, independently of instruction effects.
use crate::codegen::{AddressingMode, decode_6502_opcode};

pub(crate) enum BodyItem<'a> {
    Byte(u8),
    Global { name: &'a str, extent: u16 },
}

pub(crate) fn fingerprint(params: &[u8], result: u8, items: &[BodyItem<'_>], version: &[u8]) -> Option<u64> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    };
    add(version);
    add(params);
    add(&[0, result]);
    let mut iter = items.iter().peekable();
    while let Some(BodyItem::Byte(opcode)) = iter.next() {
        let (op, mode, len) = decode_6502_opcode(*opcode)?;
        add(&[0, *opcode]);
        if op == "RTS" {
            return iter.peek().is_none().then_some(hash);
        }
        if mode == AddressingMode::Relative || matches!(op,
            "JSR" | "JMP" | "BRK" | "RTI" | "PHA" | "PLA" | "PHP" | "PLP" | "TXS" | "TSX") {
            return None;
        }
        let mut left = len.checked_sub(1)?;
        while left > 0 {
            match iter.next()? {
                BodyItem::Byte(b) => { add(&[0, *b]); left -= 1; }
                BodyItem::Global { name, extent } if left == 2 => {
                    add(&[1]);
                    add(&extent.to_le_bytes());
                    add(&u32::try_from(name.len()).ok()?.to_le_bytes());
                    add(name.as_bytes());
                    left -= 2;
                }
                _ => return None,
            }
        }
    }
    None
}

pub(crate) const ENTRY_VERSION: &[u8] = b"actionc-6502-abi-inputs-v1";

/// Inclusive byte range. For a global, start and end are offsets from its base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRange<Id> {
    pub global: Option<Id>,
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFacts<Id> {
    pub abi_inputs_only: bool,
    pub reads: Option<Vec<DataRange<Id>>>,
    pub writes: Option<Vec<DataRange<Id>>>,
    pub preserves_zp: Vec<crate::ast::AnnotationZeroPageRange>,
    pub preserves_regs: crate::ast::AnnotationRegisterSet,
    pub returns_a_equals_a0: bool,
}

impl<Id> Default for CallFacts<Id> {
    fn default() -> Self {
        Self { abi_inputs_only: false, reads: None, writes: None, preserves_zp: Vec::new(),
            preserves_regs: Default::default(), returns_a_equals_a0: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review<Id> {
    pub fingerprint: u64,
    pub facts: CallFacts<Id>,
}

impl<Id> Review<Id> {
    pub(crate) fn dependencies_match(&self, mut extent: impl FnMut(&Id) -> Option<u16>) -> bool {
        self.facts.reads.iter().chain(&self.facts.writes).flatten().all(|r| {
            r.start <= r.end && r.global.as_ref().is_none_or(|id| extent(id).is_some_and(|n| r.end < n))
        })
    }

    pub(crate) fn map_ids<T>(&self, mut resolve: impl FnMut(&Id) -> Option<T>) -> Option<Review<T>> {
        let mut ranges = |values: &[DataRange<Id>]| values.iter().map(|r| {
            let global = match &r.global { Some(id) => Some(resolve(id)?), None => None };
            Some(DataRange { global, start: r.start, end: r.end })
        }).collect::<Option<Vec<_>>>();
        let f = &self.facts;
        Some(Review { fingerprint: self.fingerprint, facts: CallFacts {
            abi_inputs_only: f.abi_inputs_only, preserves_zp: f.preserves_zp.clone(),
            preserves_regs: f.preserves_regs, returns_a_equals_a0: f.returns_a_equals_a0,
            reads: match &f.reads { Some(v) => Some(ranges(v)?), None => None },
            writes: match &f.writes { Some(v) => Some(ranges(v)?), None => None },
        }})
    }
}

pub(crate) const CONTRACT_VERSION: &[u8] = b"actionc-6502-callee-v1";

pub(crate) fn declaration(annotations: &[crate::ast::ActioncAnnotation]) -> Result<Option<Review<String>>, &'static str> {
    use crate::ast::ActioncAnnotation::*;
    let mut expected = None;
    let mut facts = CallFacts::default();
    for annotation in annotations {
        match annotation {
            Reviewed6502 { fingerprint } => {
                if expected.replace(*fingerprint).is_some() { return Err("duplicate reviewed-6502"); }
            }
            OrdinaryAbiInputs => facts.abi_inputs_only = true,
            ReadsOnly(items) if facts.reads.is_none() => facts.reads = Some(items.clone()),
            WritesOnly(items) if facts.writes.is_none() => facts.writes = Some(items.clone()),
            ReadsOnly(_) | WritesOnly(_) => return Err("duplicate reviewed memory footprint"),
            ReturnsAEqualsA0 => facts.returns_a_equals_a0 = true,
            Preserves { registers, zero_page } => {
                facts.preserves_regs.a |= registers.a;
                facts.preserves_regs.x |= registers.x;
                facts.preserves_regs.y |= registers.y;
                facts.preserves_zp.extend(zero_page.ranges.iter().flatten().copied());
            }
            _ => {}
        }
    }
    if let Some(fingerprint) = expected {
        if annotations.iter().any(|a| matches!(a, AbiInputsOnly { .. })) {
            return Err("use bare abi-inputs-only within a reviewed-6502 contract");
        }
        if annotations.iter().any(|a| matches!(a, Preserves { zero_page, .. } if !zero_page.symbols.is_empty())) {
            return Err("reviewed-6502 preservation requires numeric zero-page ranges");
        }
        return Ok(Some(Review { fingerprint, facts }));
    }
    if facts.abi_inputs_only || facts.reads.is_some() || facts.writes.is_some() {
        return Err("complete footprints and bare abi-inputs-only require reviewed-6502");
    }
    Ok(None)
}
