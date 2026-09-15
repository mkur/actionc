//! Freestanding, explicitly placed native images. The platform establishes the
//! ABI execution domain and calls `entry`; this image contains no reset/IRQ stub.
use super::{
    emit::{self, Target},
    *,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: u32 = 0x1000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyImport {
    pub symbol: u32,
    pub signature: u32,
    pub abi: String,
    pub address: u32,
    pub size: u32,
    /// Maximum reservation below this callee's entry S, excluding its caller's
    /// JSL return address. The assembly implementation must perform its checks.
    pub stack_peak: u16,
    pub checks_stack: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkOptions {
    pub code_origin: u32,
    pub data_origin: u32,
    /// Raw nonreturning __a816_stack_overflow_v1 adapter, supplied by the platform.
    pub stack_overflow: u32,
    pub nmi_extra_stack: u16,
    pub imports: Vec<AssemblyImport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub address: u32,
    pub bytes: Vec<u8>,
    pub writable: bool,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZeroFill {
    pub address: u32,
    pub size: u32,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub offset: u32,
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub id: u32,
    pub name: String,
    pub signature: u32,
    pub address: u32,
    pub size: u32,
    pub arguments: Vec<Argument>,
    pub outgoing_bytes: u32,
    pub result_bytes: u8,
    pub fixed_frame: u16,
    pub spill_bytes: u16,
    /// Local cost only. Calls/recursion require each callee's checked reservation.
    pub local_stack_peak: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSymbol {
    pub kind: String,
    pub id: u32,
    pub name: String,
    pub address: u32,
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub format: String,
    pub version: u32,
    pub target: String,
    pub abi: String,
    pub entry: u32,
    pub stack_overflow: u32,
    pub task_headroom: u32,
    pub irq_headroom: u32,
    pub segments: Vec<Segment>,
    pub zero_fill: Vec<ZeroFill>,
    pub routines: Vec<Routine>,
    pub data: Vec<DataSymbol>,
    pub imports: Vec<AssemblyImport>,
}

impl Image {
    pub fn verify(&self) -> Result<(), String> {
        if self.format != "actionc-65816-image"
            || self.version != 1
            || self.target != "wdc-65816-native"
            || self.abi != abi::generated::ABI_NAME
        {
            return Err("unsupported native image identity".into());
        }
        if self.stack_overflow >= LIMIT
            || self.task_headroom < 26
            || self.task_headroom - 26
                != self
                    .irq_headroom
                    .checked_sub(13)
                    .ok_or("invalid IRQ headroom")?
            || self.task_headroom > 65535
        {
            return Err("invalid platform stack contract".into());
        }
        let mut extents = Vec::new();
        for segment in &self.segments {
            let size = u32::try_from(segment.bytes.len()).map_err(|_| "segment overflow")?;
            extents.push((segment.address, end(segment.address, size)?));
            if segment.executable && segment.writable {
                return Err("native code must be read-only".into());
            }
        }
        for zero in &self.zero_fill {
            extents.push((zero.address, end(zero.address, zero.size)?));
        }
        for import in &self.imports {
            if import.abi != self.abi || !import.checks_stack {
                return Err("assembly import must implement the checked native ABI".into());
            }
            extents.push((import.address, end(import.address, import.size)?));
        }
        extents.sort_unstable();
        if extents.windows(2).any(|p| p[0].1 > p[1].0) {
            return Err("overlapping native image/import regions".into());
        }
        if !self.segments.iter().any(|s| {
            s.executable && self.entry >= s.address && self.entry < s.address + s.bytes.len() as u32
        }) {
            return Err("native entry is outside emitted code".into());
        }
        if self.segments.iter().any(|s| {
            self.stack_overflow >= s.address
                && self.stack_overflow < s.address + s.bytes.len() as u32
        }) || self
            .zero_fill
            .iter()
            .any(|s| self.stack_overflow >= s.address && self.stack_overflow < s.address + s.size)
        {
            return Err("platform stack-overflow adapter overlaps emitted storage".into());
        }
        Ok(())
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let image: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        image.verify()?;
        Ok(image)
    }
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.verify()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

fn end(address: u32, size: u32) -> Result<u32, String> {
    address
        .checked_add(size)
        .filter(|&end| size > 0 && address < LIMIT && end <= LIMIT)
        .ok_or("native region extent exceeds 24 bits or is empty".into())
}

fn address(value: AddressValue) -> Result<u32, String> {
    u32::try_from(value.value)
        .ok()
        .filter(|&v| v < LIMIT)
        .ok_or("absolute address exceeds 24 bits".into())
}

pub fn link(
    program: &Mir65816Program,
    machine: &emit::MachineProgram,
    options: &LinkOptions,
) -> Result<Image, String> {
    verify_program(program).map_err(|e| format!("invalid MIR65816: {e:?}"))?;
    if program.call_convention != Mir65816CallConvention::Native {
        return Err("image requires the native ABI".into());
    }
    if program
        .native_abi
        .as_ref()
        .is_none_or(|abi| !abi.unsupported_signatures.is_empty())
    {
        return Err("image contains signatures outside native ABI v1".into());
    }
    if options.code_origin >= LIMIT
        || options.data_origin >= LIMIT
        || options.stack_overflow >= LIMIT
    {
        return Err("native link address exceeds 24 bits".into());
    }
    let mut routines = BTreeMap::new();
    let mut runtime = BTreeMap::new();
    let mut imports = BTreeMap::new();
    for import in &options.imports {
        if imports
            .insert(RuntimeSymbolId(import.symbol), import)
            .is_some()
        {
            return Err("duplicate assembly import".into());
        }
    }
    let mut used_imports = BTreeSet::new();
    for r in &program.routines {
        if r.entry.external {
            let symbol = r
                .entry
                .external_symbol
                .ok_or("external entry lacks an interface identity")?;
            let import = imports
                .get(&symbol)
                .ok_or_else(|| format!("{}: unresolved assembly import", r.name))?;
            if import.signature != r.signature.0 {
                return Err(format!("{}: assembly signature mismatch", r.name));
            }
            if r.entry.placement != crate::nir::NirRoutinePlacement::Relocatable {
                return Err("assembly imports require relocatable interface declarations".into());
            }
            routines.insert(r.id, import.address);
            runtime.insert(symbol, import.address);
            used_imports.insert(symbol);
        }
    }
    for binding in &program.runtime_bindings {
        if let Some(import) = imports.get(&binding.symbol) {
            if let Some(crate::nir::NirRuntimeTarget::Absolute(a)) = binding.target {
                if address(a)? != import.address {
                    return Err("assembly binding address mismatch".into());
                }
            }
            runtime.insert(binding.symbol, import.address);
        }
    }
    for r in &program.routines {
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            if let Mir65816Op::Call {
                target: Mir65816CallTarget::Runtime(symbol),
                signature,
                ..
            } = op
            {
                let import = imports
                    .get(symbol)
                    .ok_or("runtime call requires an assembly import")?;
                if signature.map(|s| s.0) != Some(import.signature) {
                    return Err("runtime import signature mismatch".into());
                }
                runtime.insert(*symbol, import.address);
                used_imports.insert(*symbol);
            }
        }
    }
    if used_imports.len() != imports.len() {
        return Err("assembly import has no declared interface".into());
    }
    let mut code_cursor = options.code_origin;
    let mut seen = BTreeSet::new();
    for r in &machine.routines {
        if !seen.insert(r.id)
            || !program
                .routines
                .iter()
                .any(|p| p.id == r.id && !p.entry.external)
        {
            return Err("machine routine identity mismatch".into());
        }
        let size = u32::try_from(r.code.bytes.len()).map_err(|_| "code extent overflow")?;
        // Leave the final byte of each bank unused. No emitted instruction or
        // return continuation relies on PC wrapping into another code bank.
        if size == 0 || size > 65535 {
            return Err("native routine exceeds one code bank".into());
        }
        if (code_cursor & 0xffff) + size > 65535 {
            code_cursor = (code_cursor | 0xffff)
                .checked_add(1)
                .ok_or("code bank overflow")?;
        }
        let next = end(code_cursor, size)?;
        if routines.insert(r.id, code_cursor).is_some() {
            return Err("duplicate routine placement".into());
        }
        code_cursor = next;
    }
    if routines.len() != program.routines.len() {
        return Err("missing emitted routine".into());
    }
    let mut data = BTreeMap::new();
    let mut data_cursor = options.data_origin;
    for item in &program.data {
        let location = match item.placement {
            Mir65816DataPlacement::Allocate => {
                data_cursor = abi::align_up(data_cursor, item.alignment.get())
                    .ok_or("data alignment overflow")?;
                let result = data_cursor;
                data_cursor = end(result, item.size.get())?;
                result
            }
            Mir65816DataPlacement::Absolute(a) => {
                let a = address(a)?;
                end(a, item.size.get())?;
                a
            }
            Mir65816DataPlacement::Alias { .. } => continue,
        };
        if data.insert(item.id, location).is_some() {
            return Err("duplicate data identity".into());
        }
    }
    for _ in 0..program.data.len() {
        for item in &program.data {
            if data.contains_key(&item.id) {
                continue;
            }
            if let Mir65816DataPlacement::Alias { target, offset } = item.placement {
                if let Some(base) = data.get(&target).copied() {
                    let owner = program
                        .data
                        .iter()
                        .find(|d| d.id == target)
                        .ok_or("missing alias owner")?;
                    if offset
                        .get()
                        .checked_add(item.size.get())
                        .is_none_or(|size| size > owner.size.get())
                    {
                        return Err("alias exceeds its owner".into());
                    }
                    data.insert(
                        item.id,
                        base.checked_add(offset.get()).ok_or("alias overflow")?,
                    );
                }
            }
        }
    }
    if data.len() != program.data.len() {
        return Err("unresolved or cyclic data alias".into());
    }
    let image_end = code_cursor.max(
        if program
            .data
            .iter()
            .any(|d| d.placement == Mir65816DataPlacement::Allocate)
        {
            data_cursor
        } else {
            0
        },
    );
    let entries = program
        .routines
        .iter()
        .filter(|r| r.entry.program)
        .collect::<Vec<_>>();
    if entries.len() != 1 || entries[0].entry.external {
        return Err("native image requires one emitted program entry".into());
    }
    let mut image = Image {
        format: "actionc-65816-image".into(),
        version: 1,
        target: "wdc-65816-native".into(),
        abi: abi::generated::ABI_NAME.into(),
        entry: routines[&entries[0].id],
        stack_overflow: options.stack_overflow,
        task_headroom: abi::generated::INTERRUPT_TASK_OR_BOOTSTRAP_HEADROOM_BASE_BYTES
            + u32::from(options.nmi_extra_stack),
        irq_headroom: abi::generated::INTERRUPT_IRQ_HEADROOM_BASE_BYTES
            + u32::from(options.nmi_extra_stack),
        segments: vec![],
        zero_fill: vec![],
        routines: vec![],
        data: vec![],
        imports: options.imports.clone(),
    };
    for r in &machine.routines {
        let base = routines[&r.id];
        let source = program.routines.iter().find(|p| p.id == r.id).unwrap();
        let mut bytes = r.code.bytes.clone();
        for &(offset, continuation) in &r.code.return_fixups {
            let resume = *r
                .code
                .labels
                .get(&continuation)
                .ok_or("unresolved PER continuation")?;
            if offset == 0
                || offset + 2 > bytes.len()
                || bytes[offset - 1] != 0x62
                || resume == 0
                || resume >= bytes.len()
                || (base + offset as u32 + 2) >> 16 != base >> 16
                || (base + resume as u32) >> 16 != base >> 16
            {
                return Err("invalid PER instruction/continuation placement".into());
            }
            let delta = i16::try_from(resume as i64 - 1 - (offset as i64 + 2))
                .map_err(|_| "PER continuation exceeds signed relative range")?;
            bytes[offset..offset + 2].copy_from_slice(&delta.to_le_bytes());
        }
        for fixup in &r.code.fixups {
            let target = match fixup.target {
                Target::Label(label) => {
                    let offset = *r.code.labels.get(&label).ok_or("unresolved code label")?;
                    if offset >= bytes.len() {
                        return Err("label is outside routine code".into());
                    }
                    base + offset as u32
                }
                Target::Routine(id) => *routines.get(&id).ok_or("unresolved routine relocation")?,
                Target::Runtime(id) => *runtime.get(&id).ok_or("unresolved runtime relocation")?,
                Target::Data(id) => *data.get(&id).ok_or("unresolved data relocation")?,
                Target::StackOverflow => options.stack_overflow,
            };
            patch(
                &mut bytes,
                fixup.offset,
                target as i64 + i64::from(fixup.addend),
                fixup.byte,
                if fixup.byte.is_some() { 1 } else { 3 },
            )?;
        }
        image.routines.push(Routine {
            id: r.id.0,
            name: source.name.clone(),
            signature: source.signature.0,
            address: base,
            size: bytes.len() as u32,
            arguments: source
                .frame
                .parameters
                .iter()
                .map(|p| {
                    let Mir65816AbiHome::StackArgument {
                        offset,
                        size,
                        alignment,
                    } = p.incoming
                    else {
                        unreachable!()
                    };
                    Argument {
                        offset: offset.get(),
                        size: size.get(),
                        alignment: alignment.get(),
                    }
                })
                .collect(),
            outgoing_bytes: source.frame.incoming_extent.get(),
            result_bytes: match source.result_home {
                None => 0,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A8ZeroExtended)) => 1,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)) => 2,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended)) => 3,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16)) => 4,
                _ => return Err("invalid native export result home".into()),
            },
            fixed_frame: r.frame.extent,
            spill_bytes: r.frame.spill_bytes,
            local_stack_peak: r.frame.peak_below_entry,
        });
        image.segments.push(Segment {
            address: base,
            bytes,
            writable: false,
            executable: true,
        });
    }
    for item in &program.data {
        let base = data[&item.id];
        let (kind, id) = match item.id {
            Mir65816DataId::Global(id) => ("global", id.0),
            Mir65816DataId::Static(id) => ("static", id.0),
            Mir65816DataId::ArrayBacking(id) => ("array_backing", id.0),
        };
        image.data.push(DataSymbol {
            kind: kind.into(),
            id,
            name: item.name.clone(),
            address: base,
            size: item.size.get(),
            alignment: item.alignment.get(),
        });
        if item.placement != Mir65816DataPlacement::Allocate {
            if !item.bytes.is_empty() || !item.relocations.is_empty() {
                return Err("external/alias storage cannot own initialized bytes".into());
            }
            continue;
        }
        if item.bytes.len() as u64 + u64::from(item.zero_fill.get()) != u64::from(item.size.get()) {
            return Err("data initialization extent mismatch".into());
        }
        let mut bytes = item.bytes.clone();
        for relocation in &item.relocations {
            let target = match relocation.target {
                Mir65816RelocationTarget::Data(NirStorageId::Global(id)) => *data
                    .get(&Mir65816DataId::Global(id))
                    .ok_or("unresolved global relocation")?,
                Mir65816RelocationTarget::ArrayBacking(id) => *data
                    .get(&Mir65816DataId::ArrayBacking(id))
                    .ok_or("unresolved backing relocation")?,
                Mir65816RelocationTarget::Code(id) => {
                    *routines.get(&id).ok_or("unresolved code relocation")?
                }
                Mir65816RelocationTarget::Absolute(a) => address(a)?,
                Mir65816RelocationTarget::ImageEnd => image_end,
                _ => return Err("relocation refers to invocation storage".into()),
            };
            patch(
                &mut bytes,
                relocation.offset.get() as usize,
                i64::from(target)
                    .checked_add(relocation.addend)
                    .ok_or("relocation arithmetic overflow")?,
                relocation.byte_index,
                relocation.width.get(),
            )?;
        }
        if !bytes.is_empty() {
            image.segments.push(Segment {
                address: base,
                bytes,
                writable: item.mutable,
                executable: false,
            });
        }
        if !item.zero_fill.is_zero() {
            image.zero_fill.push(ZeroFill {
                address: base + item.bytes.len() as u32,
                size: item.zero_fill.get(),
                writable: item.mutable,
            });
        }
    }
    image.verify()?;
    Ok(image)
}

fn patch(
    bytes: &mut [u8],
    offset: usize,
    value: i64,
    selector: Option<u8>,
    width: u32,
) -> Result<(), String> {
    if !(0..i64::from(LIMIT)).contains(&value) || !(1..=4).contains(&width) {
        return Err("relocation value exceeds the 24-bit address space".into());
    }
    let value = value as u32;
    let value = if let Some(byte) = selector {
        if byte > 2 || width != 1 {
            return Err("invalid relocation byte selector".into());
        }
        (value >> (byte * 8)) & 0xff
    } else {
        if width < 4 && value >= (1 << (width * 8)) {
            return Err("relocation does not fit its destination".into());
        }
        value
    };
    let output = bytes
        .get_mut(
            offset
                ..offset
                    .checked_add(width as usize)
                    .ok_or("relocation offset overflow")?,
        )
        .ok_or("relocation exceeds initialized storage")?;
    output.copy_from_slice(&value.to_le_bytes()[..width as usize]);
    Ok(())
}
