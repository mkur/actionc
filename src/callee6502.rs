//! Implementation identity for reviewed 6502 contracts. A successful
//! match certifies the recorded claims, independently of instruction effects.
use crate::codegen::{AddressingMode, decode_6502_opcode};

pub(crate) enum OpaqueTarget<'a> {
    Absolute(u16),
    Local(u16),
    Storage { name: &'a str, extent: u16, array: bool },
    LocalArray { name: &'a str, extent: u16 },
    Routine { name: &'a str, contract: u64 },
}

pub(crate) enum BodyItem<'a> {
    Byte(u8),
    OpaqueOperand {
        target: OpaqueTarget<'a>, kind: crate::asm6502::InlineAsmRelocationKind,
        addend: i32, requires_zp: bool, usage: crate::foreign::ForeignSymbolUse,
    },
    Call { name: &'a str, contract: u64 },
    Global {
        name: &'a str,
        extent: u16,
        addend: i32,
    },
}

/// An explicit ASM OPAQUE review binds the ABI and relocatable payload as
/// data. Symbolic operands bind storage declarations or current child reviews.
pub(crate) fn opaque_fingerprint(params: &[u8], result: u8, items: &[BodyItem<'_>], version: &[u8]) -> Option<u64> {
    use crate::asm6502::InlineAsmRelocationKind::*;
    use crate::foreign::ForeignSymbolUse::*;
    if version == ENTRY_VERSION { return None; }
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for &b in bytes { hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3); }
    };
    add(b"actionc-6502-opaque-v2");
    add(version);
    add(params);
    add(&[0, result]);
    for item in items {
        match item {
            BodyItem::Byte(byte) => add(&[0, *byte]),
            BodyItem::OpaqueOperand { target, kind, addend, requires_zp, usage } => {
                let tag = match target {
                    OpaqueTarget::Absolute(_) => 0,
                    OpaqueTarget::Local(_) => 1,
                    OpaqueTarget::Storage { .. } if !matches!(usage, Call | Control | Constant) => 2,
                    OpaqueTarget::LocalArray { .. } if !matches!(usage, Call | Control | Constant) => 4,
                    OpaqueTarget::Routine { .. } if matches!(usage, Call | Control)
                        && *kind == Absolute16 && *addend == 0 && !requires_zp => 3,
                    _ => return None,
                };
                let usage = match usage {
                    Address => 0, Constant => 1, Read => 2, Write => 3, ReadWrite => 4, IndexedRead => 5,
                    IndexedWrite => 6, IndexedReadWrite => 7, Call => 8, Control => 9, PointerRead => 10, IndexedPointerRead => 11,
                };
                add(&[1, tag, match kind { Absolute16 => 0, Byte8 => 1, Low8 => 2, High8 => 3 },
                    u8::from(*requires_zp), usage]);
                match target {
                    OpaqueTarget::Absolute(address) | OpaqueTarget::Local(address) => add(&address.to_le_bytes()),
                    OpaqueTarget::Storage { name, extent, array } => {
                        add(&[u8::from(*array)]);
                        add(&extent.to_le_bytes());
                        add(&u32::try_from(name.len()).ok()?.to_le_bytes());
                        add(name.as_bytes());
                    }
                    OpaqueTarget::LocalArray { name, extent } => {
                        add(&extent.to_le_bytes());
                        add(&u32::try_from(name.len()).ok()?.to_le_bytes());
                        add(name.as_bytes());
                    }
                    OpaqueTarget::Routine { name, contract } => {
                        let name = name.to_ascii_uppercase();
                        add(&u32::try_from(name.len()).ok()?.to_le_bytes());
                        add(name.as_bytes());
                        add(&contract.to_le_bytes());
                    }
                }
                add(&addend.to_le_bytes());
            }
            BodyItem::Call { .. } | BodyItem::Global { .. } => return None,
        }
    }
    Some(hash)
}

pub(crate) fn fingerprint(
    params: &[u8],
    result: u8,
    items: &[BodyItem<'_>],
    version: &[u8],
) -> Option<u64> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    };
    let calls = items.iter().any(|item| matches!(item, BodyItem::Call { .. }));
    if calls {
        if version == ENTRY_VERSION { return None; }
        add(b"actionc-6502-reviewed-calls-v1");
    }
    add(version);
    add(params);
    add(&[0, result]);
    let mut starts = Vec::new();
    let mut targets = Vec::new();
    let mut pc = 0usize;
    let mut iter = items.iter().peekable();
    while let Some(item) = iter.next() {
        let BodyItem::Byte(opcode) = item else { return None };
        let (op, mode, len) = decode_6502_opcode(*opcode)?;
        if version != ENTRY_VERSION {
            starts.push(pc);
        }
        pc = pc.checked_add(len)?;
        add(&[0, *opcode]);
        if op == "RTS" {
            if calls && iter.peek().is_some() { continue; }
            return (iter.peek().is_none()
                && targets.iter().all(|t| starts.binary_search(t).is_ok()))
            .then_some(hash);
        }
        if mode == AddressingMode::Relative {
            let BodyItem::Byte(delta) = iter.peek()? else {
                return None;
            };
            let displacement = *delta as i8;
            if version == ENTRY_VERSION || (!calls && version == CONTRACT_VERSION && displacement < 0) {
                return None;
            }
            targets.push(pc.checked_add_signed(isize::from(displacement))?);
        }
        if op == "JSR" && calls {
            let BodyItem::Call { name, contract } = iter.next()? else { return None };
            let name = name.to_ascii_uppercase();
            add(&[3]);
            add(&u32::try_from(name.len()).ok()?.to_le_bytes());
            add(name.as_bytes());
            add(&contract.to_le_bytes());
            continue;
        }
        if matches!(op, "PHA" | "PLA") && calls { continue; }
        if matches!(
            op,
            "JSR" | "JMP" | "BRK" | "RTI" | "PHA" | "PLA" | "PHP" | "PLP" | "TXS" | "TSX"
        ) {
            return None;
        }
        let mut left = len.checked_sub(1)?;
        while left > 0 {
            match iter.next()? {
                BodyItem::Byte(b) => {
                    add(&[0, *b]);
                    left -= 1;
                }
                BodyItem::Global {
                    name,
                    extent,
                    addend,
                } if left == 2 => {
                    if *addend == 0 {
                        add(&[1]);
                    } else {
                        if version == ENTRY_VERSION || !(0..i32::from(*extent)).contains(addend) {
                            return None;
                        }
                        add(&[2]);
                        add(&addend.to_le_bytes());
                    }
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

/// Resolve each reviewed dependency once per root. The pending None also
/// rejects recursive assembly call graphs.
#[derive(Default)]
pub(crate) struct ReviewDependencies<Id>(std::collections::BTreeMap<Id, Option<u64>>);

impl<Id: Ord + Copy> ReviewDependencies<Id> {
    pub fn new() -> Self { Self(std::collections::BTreeMap::new()) }

    pub fn resolve(&mut self, id: Id, compute: impl FnOnce(&mut Self) -> Option<u64>) -> Option<u64> {
        if let Some(known) = self.0.get(&id) { return *known; }
        self.0.insert(id, None);
        let result = compute(self);
        self.0.insert(id, result);
        result
    }
}

pub(crate) const ENTRY_VERSION: &[u8] = b"actionc-6502-abi-inputs-v1";

/// Buffer extent in bytes. Count parameters are one-based in a source review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferLength<P = u16> {
    Fixed(u16),
    Count { parameter: P, zero_means_256: bool },
}

impl<P: Copy> BufferLength<P> {
    pub(crate) fn map_parameter<T>(
        self,
        locate: impl FnOnce(P) -> Option<T>,
    ) -> Option<BufferLength<T>> {
        Some(match self {
            Self::Fixed(bytes) => BufferLength::Fixed(bytes),
            Self::Count { parameter, zero_means_256 } => BufferLength::Count {
                parameter: locate(parameter)?,
                zero_means_256,
            },
        })
    }

    pub(crate) fn upper_bound(self, read: impl FnOnce(P) -> Option<u8>) -> u16 {
        match self {
            Self::Fixed(bytes) => bytes,
            Self::Count { parameter, zero_means_256 } => {
                buffer_length(read(parameter), zero_means_256)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgumentBuffer {
    pub pointer: u16,
    pub length: BufferLength,
    pub write: bool,
}

impl ArgumentBuffer {
    fn hash_fields(self, mut add: impl FnMut(&[u8])) {
        add(&self.pointer.to_le_bytes());
        match self.length {
            BufferLength::Count { parameter, zero_means_256 } => {
                add(&parameter.to_le_bytes());
                add(&[self.write as u8, zero_means_256 as u8]);
            }
            BufferLength::Fixed(bytes) => {
                // Zero cannot name a one-based count parameter.
                add(&0u16.to_le_bytes());
                add(&bytes.to_le_bytes());
                add(&[self.write as u8]);
            }
        }
    }
}

/// Upper bound for a reviewed byte count, including an unknown ABI value.
pub(crate) fn buffer_length(count: Option<u8>, zero_means_256: bool) -> u16 {
    match count {
        Some(0) if zero_means_256 => 256,
        Some(n) => n.into(),
        None if zero_means_256 => 256,
        None => 255,
    }
}

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
    pub physical_abi_inputs: bool,
    /// Fixed ZP bytes physically written on every normal return.
    pub defines_on_return: std::collections::BTreeSet<u8>,
    pub indirect_writes: Option<crate::ast::AnnotationAddressRange>,
    pub buffers: Vec<ArgumentBuffer>,
    pub reads: Option<Vec<DataRange<Id>>>,
    pub writes: Option<Vec<DataRange<Id>>>,
    pub preserves_zp: Vec<crate::ast::AnnotationZeroPageRange>,
    pub preserves_regs: crate::ast::AnnotationRegisterSet,
    pub returns_a_equals_a0: bool,
}

impl<Id> CallFacts<Id> {
    pub(crate) fn valid_return_definitions(&self) -> bool {
        self.writes.as_ref().is_none_or(|writes| self.defines_on_return.iter().all(|byte| {
            writes.iter().any(|r| r.global.is_none() && (r.start..=r.end).contains(&u16::from(*byte)))
        }))
    }
}

impl<Id> Default for CallFacts<Id> {
    fn default() -> Self {
        Self {
            abi_inputs_only: false,
            physical_abi_inputs: false,
            defines_on_return: Default::default(),
            indirect_writes: None,
            buffers: Vec::new(),
            reads: None,
            writes: None,
            preserves_zp: Vec::new(),
            preserves_regs: Default::default(),
            returns_a_equals_a0: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review<Id> {
    pub fingerprint: u64,
    pub facts: CallFacts<Id>,
}

impl<Id> Review<Id> {
    pub(crate) fn version(&self) -> &'static [u8] {
        if self.facts.indirect_writes.is_some() {
            RANGE_VERSION
        } else if self.facts.buffers.is_empty() {
            CONTRACT_VERSION
        } else {
            BUFFER_VERSION
        }
    }

    pub(crate) fn bind_precondition(&self, hash: u64) -> u64 {
        if self.facts.buffers.iter().any(|b| matches!(b.length, BufferLength::Fixed(_))) {
            let mut hash = hash;
            let mut add = |bytes: &[u8]| {
                for &byte in bytes {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                }
            };
            add(b"fixed-buffer-v1");
            for buffer in &self.facts.buffers {
                buffer.hash_fields(&mut add);
            }
            return hash;
        }
        let Some(range) = self.facts.indirect_writes else { return hash };
        range.start.to_le_bytes().into_iter().chain(range.end.to_le_bytes()).fold(hash,
            |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3))
    }

    pub(crate) fn valid_buffer_signature(&self, widths: &[u16]) -> bool {
        let width = |i: u16| {
            i.checked_sub(1)
                .and_then(|i| widths.get(usize::from(i)))
                .copied()
        };
        if let Some(range) = self.facts.indirect_writes
            && (range.start > range.end || self.facts.writes.is_none() || !self.facts.buffers.is_empty()) {
            return false;
        }
        self.facts.buffers.iter().all(|b| {
            width(b.pointer) == Some(2)
                && match b.length {
                    BufferLength::Fixed(bytes) => bytes != 0,
                    BufferLength::Count { parameter, .. } => width(parameter) == Some(1),
                }
                && (if b.write {
                    self.facts.writes.is_some()
                } else {
                    self.facts.reads.is_some()
                })
        })
    }

    /// Bind a calling assembly review to the selected implementation and every
    /// published guarantee. Symbol IDs differ between backends; use reviewed names.
    pub(crate) fn dependency_hash(&self, name: impl FnMut(&Id) -> Option<String>) -> Option<u64> {
        let f = self.map_ids(name)?.facts;
        let mut hash = self.fingerprint;
        let mut add = |bytes: &[u8]| for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        };
        add(&[f.abi_inputs_only as u8, f.physical_abi_inputs as u8, f.returns_a_equals_a0 as u8,
            f.preserves_regs.a as u8, f.preserves_regs.x as u8, f.preserves_regs.y as u8]);
        if !f.defines_on_return.is_empty() {
            add(b"defines-on-return");
            add(&(f.defines_on_return.len() as u16).to_le_bytes());
            for byte in f.defines_on_return { add(&[byte]); }
        }
        if let Some(r) = f.indirect_writes {
            add(b"indirect-writes"); add(&r.start.to_le_bytes()); add(&r.end.to_le_bytes());
        }
        add(&(f.preserves_zp.len() as u64).to_le_bytes());
        for r in f.preserves_zp { add(&[r.start, r.end]); }
        add(&(f.buffers.len() as u64).to_le_bytes());
        for b in f.buffers {
            b.hash_fields(&mut add);
        }
        for ranges in [f.reads, f.writes] {
            add(&[ranges.is_some() as u8]);
            let ranges = ranges.unwrap_or_default();
            add(&(ranges.len() as u64).to_le_bytes());
            for r in ranges {
                add(&r.start.to_le_bytes()); add(&r.end.to_le_bytes());
                let name = r.global.unwrap_or_default().to_ascii_uppercase();
                add(&(name.len() as u64).to_le_bytes()); add(name.as_bytes());
            }
        }
        Some(hash)
    }

    pub(crate) fn dependencies_match(&self, mut extent: impl FnMut(&Id) -> Option<u16>) -> bool {
        self.facts.valid_return_definitions() && self.facts
            .reads
            .iter()
            .chain(&self.facts.writes)
            .flatten()
            .all(|r| {
                r.start <= r.end
                    && r.global
                        .as_ref()
                        .is_none_or(|id| extent(id).is_some_and(|n| r.end < n))
            })
    }

    pub(crate) fn map_ids<T>(
        &self,
        mut resolve: impl FnMut(&Id) -> Option<T>,
    ) -> Option<Review<T>> {
        let mut ranges = |values: &[DataRange<Id>]| {
            values
                .iter()
                .map(|r| {
                    let global = match &r.global {
                        Some(id) => Some(resolve(id)?),
                        None => None,
                    };
                    Some(DataRange {
                        global,
                        start: r.start,
                        end: r.end,
                    })
                })
                .collect::<Option<Vec<_>>>()
        };
        let f = &self.facts;
        Some(Review {
            fingerprint: self.fingerprint,
            facts: CallFacts {
                physical_abi_inputs: f.physical_abi_inputs,
                defines_on_return: f.defines_on_return.clone(),
                indirect_writes: f.indirect_writes,
                buffers: f.buffers.clone(),
                abi_inputs_only: f.abi_inputs_only,
                preserves_zp: f.preserves_zp.clone(),
                preserves_regs: f.preserves_regs,
                returns_a_equals_a0: f.returns_a_equals_a0,
                reads: match &f.reads {
                    Some(v) => Some(ranges(v)?),
                    None => None,
                },
                writes: match &f.writes {
                    Some(v) => Some(ranges(v)?),
                    None => None,
                },
            },
        })
    }
}

pub(crate) const RANGE_VERSION: &[u8] = b"actionc-6502-indirect-destination-v1";
const BUFFER_VERSION: &[u8] = b"actionc-6502-buffers-v1";
pub(crate) const CONTRACT_VERSION: &[u8] = b"actionc-6502-callee-v1";

pub(crate) fn declaration(
    annotations: &[crate::ast::ActioncAnnotation],
) -> Result<Option<Review<String>>, &'static str> {
    use crate::ast::ActioncAnnotation::*;
    let mut expected = None;
    let mut facts = CallFacts::default();
    for annotation in annotations {
        match annotation {
            Reviewed6502 { fingerprint } => {
                if expected.replace(*fingerprint).is_some() {
                    return Err("duplicate reviewed-6502");
                }
            }
            Buffer(buffer) => facts.buffers.push(*buffer),
            IndirectWriteRange(r) => {
                if facts.indirect_writes.replace(*r).is_some() { return Err("duplicate indirect write precondition"); }
            }
            DefinesOnReturn(bytes) => facts.defines_on_return.extend(bytes),
            PhysicalAbiInputs => facts.physical_abi_inputs = true,
            OrdinaryAbiInputs => facts.abi_inputs_only = true,
            ReadsOnly(items) if facts.reads.is_none() => facts.reads = Some(items.clone()),
            WritesOnly(items) if facts.writes.is_none() => facts.writes = Some(items.clone()),
            ReadsOnly(_) | WritesOnly(_) => return Err("duplicate reviewed memory footprint"),
            ReturnsAEqualsA0 => facts.returns_a_equals_a0 = true,
            Preserves {
                registers,
                zero_page,
            } => {
                facts.preserves_regs.a |= registers.a;
                facts.preserves_regs.x |= registers.x;
                facts.preserves_regs.y |= registers.y;
                facts
                    .preserves_zp
                    .extend(zero_page.ranges.iter().flatten().copied());
            }
            _ => {}
        }
    }
    if !facts.valid_return_definitions() {
        return Err("defines-on-return contains a byte outside the fixed writes-only footprint");
    }
    if let Some(fingerprint) = expected {
        if annotations
            .iter()
            .any(|a| matches!(a, AbiInputsOnly { .. }))
        {
            return Err("use bare abi-inputs-only within a reviewed-6502 contract");
        }
        if annotations
            .iter()
            .any(|a| matches!(a, Preserves { zero_page, .. } if !zero_page.symbols.is_empty()))
        {
            return Err("reviewed-6502 preservation requires numeric zero-page ranges");
        }
        if facts.indirect_writes.is_some() && (facts.writes.is_none() || !facts.buffers.is_empty()) {
            return Err("reviewed indirect writes require writes-only and a single destination model");
        }
        if facts.buffers.iter().any(|b| {
            if b.write {
                facts.writes.is_none()
            } else {
                facts.reads.is_none()
            }
        }) {
            return Err("a reviewed buffer needs a fixed footprint for the same access direction");
        }
        return Ok(Some(Review { fingerprint, facts }));
    }
    if !facts.defines_on_return.is_empty() { return Err("defines-on-return requires reviewed-6502"); }
    if facts.indirect_writes.is_some() || facts.physical_abi_inputs
        || !facts.buffers.is_empty()
        || facts.abi_inputs_only
        || facts.reads.is_some()
        || facts.writes.is_some()
    {
        return Err("complete footprints and bare abi-inputs-only require reviewed-6502");
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_reviews_allow_only_branches_to_local_instruction_starts() {
        let eligible = |bytes: &[u8], version| {
            let items: Vec<_> = bytes.iter().copied().map(BodyItem::Byte).collect();
            fingerprint(&[], 0, &items, version).is_some()
        };
        let conditional = [0x90, 0x01, 0xe8, 0x60];
        let counted_loop = [0x88, 0xd0, 0xfd, 0x60];
        assert!(!eligible(&conditional, ENTRY_VERSION));
        assert!(!eligible(&counted_loop, CONTRACT_VERSION));
        assert!(eligible(&counted_loop, BUFFER_VERSION));
        for version in [BUFFER_VERSION, CONTRACT_VERSION] {
            assert!(eligible(&conditional, version));
            for bytes in [
                &[0xd0, 0xff, 0x60][..],
                &[0xd0, 0x01, 0x60],
                &[0x48, 0x68, 0x60],
                &[0x20, 0, 6, 0x60],
                &[0x60, 0x60],
            ] {
                assert!(!eligible(bytes, version));
            }
        }
    }
}
