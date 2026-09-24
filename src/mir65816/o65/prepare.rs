use super::super::{
    relocation::{self, Owner, Target},
    *,
};
use super::profile::*;
use std::collections::{BTreeMap, BTreeSet};

/// Validated section contents and typed fixups. No load addresses are assigned.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub(crate) text: Vec<u8>,
    pub(crate) data: Vec<u8>,
    pub(crate) bss: u32,
    pub(crate) profile: Profile,
}
impl Artifact {
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    pub fn section_sizes(&self) -> [u32; 3] {
        [self.text.len() as u32, self.data.len() as u32, self.bss]
    }
}
fn contract(r: &Mir65816Routine, stack_peak: u16, irq_effect: u8) -> Result<Contract, String> {
    let args = r
        .frame
        .parameters
        .iter()
        .map(|p| match p.incoming {
            Mir65816AbiHome::StackArgument {
                offset,
                size,
                alignment,
            } => Ok(Argument {
                offset: offset.get(),
                size: size.get(),
                alignment: alignment.get(),
            }),
            _ => Err("unsupported native argument home".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let result = match r.result_home {
        None => 0,
        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A8ZeroExtended)) => 1,
        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)) => 2,
        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended)) => 3,
        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16)) => 4,
        _ => return Err("unsupported result home".into()),
    };
    let c = Contract {
        abi: abi::generated::ABI_NAME.into(),
        signature: r.signature.0,
        arguments: args,
        result,
        incoming: r.frame.incoming_extent.get(),
        stack_peak,
        irq_effect,
        kind: 0,
        domains: 3,
    };
    c.verify()?;
    Ok(c)
}
fn align(value: u32, alignment: u32) -> Result<u32, String> {
    if ![1, 2, 4].contains(&alignment) {
        return Err("o65 storage alignment above four or invalid".into());
    }
    abi::align_up(value, alignment)
        .filter(|v| *v < LIMIT)
        .ok_or("section alignment overflow".into())
}
fn extent(base: u32, size: u32) -> Result<u32, String> {
    base.checked_add(size)
        .filter(|v| *v <= LIMIT)
        .ok_or("o65 section exceeds 24 bits".into())
}

pub fn prepare(program: &Mir65816Program, options: &Options) -> Result<Artifact, String> {
    if options.profile != ID && options.profile != ID_V2 {
        return Err("unsupported experimental o65 profile".into());
    }
    if u32::from(options.nmi_extra_stack) + 26 > 65535 {
        return Err("invalid platform stack contract".into());
    }
    let machine = emit::materialize(program)?;
    let program = &machine.prepared;
    let fixups = relocation::collect(program, &machine)?;
    let arithmetic_fault = fixups.iter().any(|f| f.target == Target::ArithmeticFault);
    if arithmetic_fault && options.profile == ID {
        return Err("arithmetic fault requires experimental o65 profile v2".into());
    }
    let mut bindings = BTreeMap::new();
    let mut names = BTreeSet::new();
    for b in &options.imports {
        if !valid_name(&b.name)
            || b.name.starts_with("__a816_o65_")
            || b.name == ENTRY
            || b.name == OVERFLOW
            || b.name == ARITHMETIC_FAULT
            || !b.checks_stack
        {
            return Err("invalid o65 binding name or unchecked import".into());
        }
        if bindings.insert(RuntimeSymbolId(b.symbol), b).is_some() || !names.insert(&b.name) {
            return Err("duplicate o65 binding".into());
        }
    }
    let mut imports = vec![Import {
        name: OVERFLOW.into(),
        contract: Contract::overflow(),
    }];
    if arithmetic_fault {
        imports.push(Import {
            name: ARITHMETIC_FAULT.into(),
            contract: Contract::arithmetic_fault(),
        });
    }
    let mut targets = BTreeMap::<Owner, (Reference, u32)>::new();
    let mut runtime = BTreeMap::new();
    let mut used = BTreeSet::new();
    for r in &program.routines {
        if !r.entry.external {
            continue;
        }
        if r.entry.placement != crate::nir::NirRoutinePlacement::Relocatable {
            return Err("assembly imports require relocatable interface declarations".into());
        }
        let symbol = r
            .entry
            .external_symbol
            .ok_or("missing interface identity")?;
        let b = bindings
            .get(&symbol)
            .ok_or("unresolved o65 interface name")?;
        used.insert(symbol);
        let effect = match b.irq_effect {
            image::IrqEffect::Preserve => 0,
            image::IrqEffect::SaveDisable => 1,
            image::IrqEffect::Restore => 2,
        };
        let index = imports.len() as u32;
        let mut c = contract(r, b.stack_peak, effect)?;
        c.domains = b.domains;
        c.verify()?;
        imports.push(Import {
            name: b.name.clone(),
            contract: c,
        });
        runtime.insert(symbol, index);
        targets.insert(Owner::Routine(r.id), (Reference::Import(index), 0));
    }
    if used.len() != bindings.len() {
        return Err("o65 binding has no declared interface".into());
    }
    for b in &program.runtime_bindings {
        if matches!(b.target, Some(crate::nir::NirRuntimeTarget::Absolute(_))) {
            return Err("fixed runtime binding unsupported in o65".into());
        }
    }
    let mut artifact = Artifact {
        text: vec![],
        data: vec![],
        bss: 0,
        profile: Profile {
            version: if arithmetic_fault { 2 } else { 1 },
            nmi_extra_stack: options.nmi_extra_stack,
            entry: 0,
            routines: vec![],
            objects: vec![],
            imports,
            relocations: vec![],
        },
    };
    let mut entries = vec![];
    for r in &machine.routines {
        let source = program
            .routines
            .iter()
            .find(|s| s.id == r.id)
            .ok_or("missing routine identity")?;
        let size = u32::try_from(r.code.bytes.len()).map_err(|_| "routine overflow")?;
        if size == 0 || size > 65535 {
            return Err("native routine exceeds one code bank".into());
        }
        let mut base = artifact.text.len() as u32;
        if (base & 65535) + size > 65535 {
            base = extent(base | 65535, 1)?;
        }
        let end = extent(base, size)?;
        artifact.text.resize(end as usize, 0);
        artifact.text[base as usize..end as usize]
            .copy_from_slice(&relocation::routine_bytes(r, base)?);
        targets.insert(
            Owner::Routine(r.id),
            (Reference::Section(Section::Text), base),
        );
        if source.entry.program {
            entries.push(base);
        }
        artifact.profile.routines.push(Routine {
            id: r.id.0,
            name: source.name.clone(),
            offset: base,
            size,
            contract: contract(source, 0, 0)?,
            frame: r.frame.extent,
            spill: r.frame.spill_bytes,
            local_peak: u32::from(r.frame.peak_below_entry),
        });
    }
    if entries.len() != 1 {
        return Err("native image requires one emitted program entry".into());
    }
    artifact.profile.entry = entries[0];
    let mut locations = BTreeMap::<Mir65816DataId, Location>::new();
    for d in &program.data {
        if let Mir65816DataPlacement::Alias { .. } = d.placement {
            continue;
        }
        let location = match d.placement {
            Mir65816DataPlacement::Absolute(a) => Location {
                section: None,
                offset: u32::try_from(a.value)
                    .ok()
                    .filter(|v| *v < LIMIT)
                    .ok_or("absolute address exceeds 24 bits")?,
            },
            Mir65816DataPlacement::Allocate => {
                if d.bytes.len() as u64 + u64::from(d.zero_fill.get()) != u64::from(d.size.get()) {
                    return Err("data initialization extent mismatch".into());
                }
                let section = if !d.mutable {
                    Section::Text
                } else if d.bytes.is_empty() && d.relocations.is_empty() {
                    Section::Bss
                } else {
                    Section::Data
                };
                let old = artifact.section_sizes()[section.index()];
                let base = align(old, d.alignment.get())?;
                let end = extent(base, d.size.get())?;
                match section {
                    Section::Bss => artifact.bss = end,
                    _ => {
                        let bytes = if section == Section::Text {
                            &mut artifact.text
                        } else {
                            &mut artifact.data
                        };
                        bytes.resize(end as usize, 0);
                        bytes[base as usize..base as usize + d.bytes.len()]
                            .copy_from_slice(&d.bytes);
                    }
                }
                Location {
                    section: Some(section),
                    offset: base,
                }
            }
            _ => unreachable!(),
        };
        extent(location.offset, d.size.get())?;
        if locations.insert(d.id, location).is_some() {
            return Err("duplicate data identity".into());
        }
    }
    for _ in 0..program.data.len() {
        for d in &program.data {
            if locations.contains_key(&d.id) {
                continue;
            }
            if let Mir65816DataPlacement::Alias { target, offset } = d.placement {
                if let Some(mut loc) = locations.get(&target).copied() {
                    let owner = program
                        .data
                        .iter()
                        .find(|o| o.id == target)
                        .ok_or("missing alias owner")?;
                    if offset
                        .get()
                        .checked_add(d.size.get())
                        .is_none_or(|v| v > owner.size.get())
                    {
                        return Err("alias exceeds its owner".into());
                    }
                    loc.offset = extent(loc.offset, offset.get())?;
                    locations.insert(d.id, loc);
                }
            }
        }
    }
    if locations.len() != program.data.len() {
        return Err("unresolved or cyclic data alias".into());
    }
    for d in &program.data {
        let location = locations[&d.id];
        let (kind, id) = match d.id {
            Mir65816DataId::Global(id) => (0, id.0),
            Mir65816DataId::Static(id) => (1, id.0),
            Mir65816DataId::ArrayBacking(id) => (2, id.0),
        };
        artifact.profile.objects.push(Object {
            kind,
            id,
            name: d.name.clone(),
            location,
            size: d.size.get(),
            alignment: d.alignment.get(),
            mutable: d.mutable,
            alias: matches!(d.placement, Mir65816DataPlacement::Alias { .. }),
        });
    }
    for f in fixups {
        let (site_section, site_base) = match f.owner {
            Owner::Routine(_) => match targets[&f.owner] {
                (Reference::Section(s), offset) => (s, offset),
                _ => return Err("patch in external routine".into()),
            },
            Owner::Data(id) => {
                let l = locations[&id];
                (l.section.ok_or("patch in external data")?, l.offset)
            }
        };
        let (target, value) = match f.target {
            Target::Code(id, offset) => {
                let (target, value) = *targets
                    .get(&Owner::Routine(id))
                    .ok_or("unresolved code relocation")?;
                (Some(target), i64::from(value) + i64::from(offset))
            }
            Target::Runtime(id) => (
                Some(Reference::Import(
                    *runtime
                        .get(&id)
                        .ok_or("runtime binding lacks declared o65 interface")?,
                )),
                0,
            ),
            Target::StackOverflow => (Some(Reference::Import(0)), 0),
            Target::ArithmeticFault => (Some(Reference::Import(1)), 0),
            Target::Data(id) => {
                let l = *locations.get(&id).ok_or("unresolved data relocation")?;
                (l.section.map(Reference::Section), i64::from(l.offset))
            }
            Target::Absolute(a) => (None, i64::from(a)),
            Target::ImageEnd => return Err("ImageEnd unsupported in experimental o65".into()),
        };
        let value = value
            .checked_add(f.addend)
            .filter(|v| (0..i64::from(LIMIT)).contains(v))
            .ok_or("o65 relocation value exceeds 24 bits")? as u32;
        let offset = extent(site_base, f.offset)?;
        if let Some(target) = target {
            match target {
                Reference::Import(_) => {
                    if value != 0 {
                        return Err("nonzero import addend unsupported in o65".into());
                    }
                }
                Reference::Section(s) => {
                    if value > artifact.section_sizes()[s.index()] {
                        return Err("o65 target offset outside section".into());
                    }
                }
            }
            let encoding = match (f.width, f.selector) {
                (1, Some(0)) => Encoding::Low,
                (1, Some(1)) => Encoding::High,
                (1, Some(2)) => Encoding::Bank,
                (3 | 4, None) => Encoding::Long,
                _ => return Err("checked narrow address relocation unsupported in o65".into()),
            };
            artifact.profile.relocations.push(Relocation {
                section: site_section,
                offset,
                encoding,
                target,
                value,
                zero_extend: f.width == 4,
            });
        }
        let bytes = match site_section {
            Section::Text => &mut artifact.text,
            Section::Data => &mut artifact.data,
            _ => return Err("relocation in BSS".into()),
        };
        relocation::patch(
            bytes,
            offset as usize,
            i64::from(value),
            f.selector,
            f.width,
        )?;
    }
    artifact
        .profile
        .relocations
        .sort_by_key(|r| (r.section as u8, r.offset));
    Ok(artifact)
}
