//! Freestanding, explicitly placed native images. The platform establishes the
//! ABI execution domain and calls `entry`; this image contains no reset/IRQ stub.
use super::relocation::patch;
use super::{
    emit::{self, Target},
    *,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: u32 = 0x1000000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrqEffect {
    #[default]
    Preserve,
    SaveDisable,
    Restore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyImport {
    pub symbol: u32,
    pub signature: u32,
    pub abi: String,
    #[serde(deserialize_with = "json_address::deserialize")]
    pub address: u32,
    pub size: u32,
    /// Maximum reservation below this callee's entry S, excluding its caller's
    /// JSL return address. The assembly implementation must perform its checks.
    pub stack_peak: u16,
    pub checks_stack: bool,
    #[serde(default)]
    pub irq_effect: IrqEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkOptions {
    #[serde(deserialize_with = "json_address::deserialize")]
    pub code_origin: u32,
    #[serde(deserialize_with = "json_address::deserialize")]
    pub data_origin: u32,
    #[serde(default, deserialize_with = "json_address::optional")]
    pub read_only_origin: Option<u32>,
    #[serde(default, deserialize_with = "json_address::optional")]
    pub zero_fill_origin: Option<u32>,
    /// Raw nonreturning __a816_stack_overflow_v1 adapter, supplied by the platform.
    #[serde(deserialize_with = "json_address::deserialize")]
    pub stack_overflow: u32,
    #[serde(
        default,
        deserialize_with = "json_address::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub arithmetic_fault: Option<u32>,
    pub nmi_extra_stack: u16,
    pub imports: Vec<AssemblyImport>,
}

/// Accept readable layout addresses without changing numeric serialization.
/// The linker still checks the target's 24-bit address bounds.
mod json_address {
    use serde::{Deserialize, Deserializer, de::Error};

    #[derive(Deserialize)]
    #[serde(
        untagged,
        expecting = "an unsigned integer or a hex string prefixed with 0x, 0X or $"
    )]
    enum Address {
        Number(u32),
        Hex(String),
    }

    impl Address {
        fn value<E: Error>(self) -> Result<u32, E> {
            match self {
                Self::Number(value) => Ok(value),
                Self::Hex(text) => {
                    let digits = text
                        .strip_prefix("0x")
                        .or_else(|| text.strip_prefix("0X"))
                        .or_else(|| text.strip_prefix('$'))
                        .filter(|digits| {
                            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                        .ok_or_else(|| {
                            E::custom("hex address must have a 0x, 0X or $ prefix followed by hexadecimal digits")
                        })?;
                    u32::from_str_radix(digits, 16)
                        .map_err(|_| E::custom("hex address exceeds 32 bits"))
                }
            }
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
        Address::deserialize(deserializer)?.value()
    }

    pub fn optional<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
        Option::<Address>::deserialize(deserializer)?
            .map(Address::value)
            .transpose()
    }
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
    pub body_displacement: u32,
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
    pub objects: Vec<FrameObject>,
    pub temporaries: Vec<Temporary>,
    pub calls: Vec<CallCost>,
    /// Unknown for recursion and indirect calls; no whole-program analysis.
    pub whole_task_stack_bound: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameObject {
    pub id: u32,
    pub owner_kind: String,
    pub owner_id: u32,
    pub displacement: u32,
    pub size: u32,
    pub alignment: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Temporary {
    pub id: u32,
    pub home: TemporaryHome,
    pub size: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TemporaryHome {
    Stack { displacement: u16 },
    DirectPage { offset: u16 },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallCost {
    pub outgoing: u32,
    pub transfer_peak: u32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arithmetic_fault: Option<u32>,
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
            || !matches!(
                (self.version, self.arithmetic_fault),
                (3, None) | (4, Some(_))
            )
            || self.target != "wdc-65816-native"
            || self.abi != abi::generated::ABI_NAME
        {
            return Err(
                "unsupported native image identity; recompile for image version 3 or 4".into(),
            );
        }
        if self.stack_overflow >= LIMIT
            || self
                .arithmetic_fault
                .is_some_and(|a| a >= LIMIT || a == self.stack_overflow)
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
        if self
            .arithmetic_fault
            .is_some_and(|a| extents.iter().any(|&(start, end)| a >= start && a < end))
        {
            return Err("platform arithmetic-fault adapter overlaps image/import storage".into());
        }
        let mut routine_ids = BTreeSet::new();
        for r in &self.routines {
            if !routine_ids.insert(r.id)
                || r.fixed_frame > 254
                || r.fixed_frame % 2 != 0
                || r.spill_bytes > r.fixed_frame
                || !self.segments.iter().any(|s| {
                    s.executable && s.address == r.address && s.bytes.len() == r.size as usize
                })
                || r.whole_task_stack_bound.is_some()
            {
                return Err("invalid routine/frame map".into());
            }
            for a in &r.arguments {
                if Some(a.body_displacement) != (u32::from(r.fixed_frame) + 4).checked_add(a.offset)
                    || a.size == 0
                    || a.body_displacement
                        .checked_add(a.size)
                        .is_none_or(|e| e > 256)
                {
                    return Err("invalid incoming displacement map".into());
                }
            }
            let mut temp_ids = std::collections::BTreeSet::new();
            let mut dp_width = None;
            for temp in &r.temporaries {
                if !temp_ids.insert(temp.id) || !(1..=4).contains(&temp.size) {
                    return Err("invalid temporary identity or width".into());
                }
                if let TemporaryHome::DirectPage { offset } = temp.home {
                    let pointer = temp.size == 3
                        && [
                            abi::generated::DP_POINTER0_OFFSET as u16,
                            abi::generated::DP_POINTER1_OFFSET as u16,
                            abi::generated::DP_POINTER2_OFFSET as u16,
                        ]
                        .contains(&offset);
                    let scalar = temp.size == 2 && emit::scalar::word_offset(offset);
                    if (!pointer && !scalar)
                        || !r.calls.is_empty()
                        || dp_width.is_some_and(|w| w != temp.size)
                    {
                        return Err("invalid direct-page temporary map".into());
                    }
                    dp_width = Some(temp.size);
                }
            }
            for (offset, size) in r.objects.iter().map(|o| (o.displacement, o.size)).chain(
                r.temporaries.iter().filter_map(|t| match t.home {
                    TemporaryHome::Stack { displacement } => {
                        Some((u32::from(displacement), u32::from(t.size)))
                    }
                    TemporaryHome::DirectPage { .. } => None,
                }),
            ) {
                if offset == 0
                    || size == 0
                    || offset
                        .checked_add(size)
                        .is_none_or(|e| e > u32::from(r.fixed_frame) + 1)
                {
                    return Err("frame map object exceeds its allocation".into());
                }
            }
            let call_peak = r
                .calls
                .iter()
                .try_fold(0u32, |peak, c| {
                    c.outgoing
                        .checked_add(c.transfer_peak)
                        .map(|value| peak.max(value))
                })
                .ok_or("stack cost overflow")?;
            let peak = u32::from(r.fixed_frame)
                .checked_add(call_peak)
                .ok_or("stack cost overflow")?;
            if peak != u32::from(r.local_stack_peak)
                || r.calls.iter().any(|c| {
                    c.outgoing == 0
                        || c.outgoing % 2 != 1
                        || c.outgoing > 255
                        || ![3, 6].contains(&c.transfer_peak)
                })
            {
                return Err("invalid local stack cost map".into());
            }
        }
        if !self.routines.iter().any(|r| r.address == self.entry) {
            return Err("entry does not name a routine export".into());
        }
        Ok(())
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let envelope: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !matches!(
            envelope.get("version").and_then(|v| v.as_u64()),
            Some(3 | 4)
        ) {
            return Err(
                "unsupported native image version; recompile for image version 3 or 4".into(),
            );
        }
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
    if arithmetic::prepare(program)? != machine.prepared {
        return Err("machine code belongs to a different prepared program".into());
    }
    let program = &machine.prepared;
    let arithmetic_fault = if machine.routines.iter().any(|r| {
        r.code
            .fixups
            .iter()
            .any(|f| f.target == Target::ArithmeticFault)
    }) {
        Some(
            options
                .arithmetic_fault
                .ok_or("arithmetic requires __a816_arithmetic_fault_v1 in the image layout")?,
        )
    } else {
        None
    };
    super::relocation::collect(program, machine)?;
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
        || options.read_only_origin.is_some_and(|a| a >= LIMIT)
        || options.zero_fill_origin.is_some_and(|a| a >= LIMIT)
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
            let valid_effect = match import.irq_effect {
                IrqEffect::Preserve => true,
                IrqEffect::SaveDisable => {
                    r.frame.parameters.is_empty()
                        && r.result_home
                            == Some(Mir65816AbiHome::NativeResult(
                                abi::ResultLocation::A8ZeroExtended,
                            ))
                }
                IrqEffect::Restore => {
                    r.result_home.is_none()
                        && r.frame.parameters.len() == 1
                        && matches!(r.frame.parameters[0].incoming,Mir65816AbiHome::StackArgument{size,..} if size==ByteSize::ONE)
                }
            };
            if !valid_effect {
                return Err("IRQ-state import has an incompatible signature".into());
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
    let mut read_only_cursor = options.read_only_origin.unwrap_or(0);
    let mut zero_fill_cursor = options.zero_fill_origin.unwrap_or(0);
    let mut allocated_end = 0;
    for item in &program.data {
        let location = match item.placement {
            Mir65816DataPlacement::Allocate => {
                let cursor = if !item.mutable && options.read_only_origin.is_some() {
                    &mut read_only_cursor
                } else if item.bytes.is_empty() && options.zero_fill_origin.is_some() {
                    &mut zero_fill_cursor
                } else {
                    &mut data_cursor
                };
                *cursor = abi::align_up(*cursor, item.alignment.get())
                    .ok_or("data alignment overflow")?;
                let result = *cursor;
                *cursor = end(result, item.size.get())?;
                allocated_end = allocated_end.max(*cursor);
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
    let image_end = code_cursor.max(allocated_end);
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
        version: if arithmetic_fault.is_some() { 4 } else { 3 },
        target: "wdc-65816-native".into(),
        abi: abi::generated::ABI_NAME.into(),
        entry: routines[&entries[0].id],
        stack_overflow: options.stack_overflow,
        arithmetic_fault,
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
        let mut bytes = super::relocation::routine_bytes(r, base)?;
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
                Target::ArithmeticFault => {
                    arithmetic_fault.ok_or("missing arithmetic fault adapter")?
                }
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
                        body_displacement: u32::from(r.frame.extent) + 4 + offset.get(),
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
            objects: source
                .frame
                .objects
                .iter()
                .map(|o| {
                    let (kind, id) = match o.owner {
                        Mir65816FrameObjectOwner::Local(id) => ("local", id.0),
                        Mir65816FrameObjectOwner::Param(id) => ("parameter", id.0),
                    };
                    FrameObject {
                        id: o.id.0,
                        owner_kind: kind.into(),
                        owner_id: id,
                        displacement: o.stack_offset.get(),
                        size: o.size.get(),
                        alignment: o.alignment.get(),
                    }
                })
                .collect(),
            temporaries: r
                .frame
                .temps
                .iter()
                .map(|(id, slot)| Temporary {
                    id: id.0,
                    home: match slot {
                        emit::Location::Stack(slot) => TemporaryHome::Stack {
                            displacement: slot.offset,
                        },
                        emit::Location::DirectPage(slot) => TemporaryHome::DirectPage {
                            offset: slot.offset,
                        },
                    },
                    size: slot.slot().width,
                })
                .collect(),
            calls: source
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|op| {
                    if let Mir65816Op::Call { plan, .. } = op {
                        Some(CallCost {
                            outgoing: plan.outgoing_bytes.get(),
                            transfer_peak: plan.native.unwrap().transfer.peak_bytes().get(),
                        })
                    } else {
                        None
                    }
                })
                .collect(),
            whole_task_stack_bound: None,
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
