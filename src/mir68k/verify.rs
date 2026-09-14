//! MIR contract checks are separate from the deliberately smaller executable
//! subset. Canary lowering may still describe arithmetic requiring later helpers.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn verify_contract(program: &Mir68kProgram) -> Result<(), Vec<Mir68kDiagnostic>> {
    let mut errors = Vec::new();
    let mut report = |routine: Option<&str>, message: String| {
        errors.push(Mir68kDiagnostic {
            routine: routine.map(str::to_owned),
            block: None,
            message,
        })
    };
    if program.target_layout.target != TargetId::Motorola68000
        || program.endian != Endian::Big
        || program.data_pointer_width.get() != 4
        || program.code_pointer_width.get() != 4
    {
        report(None, "inconsistent MC68000 target layout".into());
    }
    let data: BTreeMap<_, _> = program.data.iter().map(|d| (d.id, d)).collect();
    let routines: BTreeMap<_, _> = program.routines.iter().map(|r| (r.id, r)).collect();
    if data.len() != program.data.len() || routines.len() != program.routines.len() {
        report(None, "duplicate data or routine identity".into());
    }
    let entries: Vec<_> = program
        .routines
        .iter()
        .filter(|r| r.entry.program)
        .map(|r| r.id)
        .collect();
    if entries.len() > 1 || program.entry != entries.first().copied() {
        report(None, "inconsistent program entry identity".into());
    }
    for item in &program.data {
        if !item.alignment.get().is_power_of_two() || item.alignment.get() > 2 {
            report(None, format!("{}: invalid data alignment", item.name));
        }
        let extent = (item.bytes.len() as u64) + u64::from(item.zero_fill.get());
        if extent > u64::from(item.size.get())
            || matches!(item.placement, Mir68kDataPlacement::Allocate)
                && extent != u64::from(item.size.get())
        {
            report(
                None,
                format!(
                    "{}: initialization extent differs from allocation",
                    item.name
                ),
            );
        }
        if let Mir68kDataPlacement::Alias { target, offset } = item.placement {
            if data.get(&target).is_none_or(|d| {
                offset
                    .get()
                    .checked_add(item.size.get())
                    .is_none_or(|end| end > d.size.get())
            }) {
                report(
                    None,
                    format!("{}: alias target missing or too small", item.name),
                );
            }
            let mut visited = BTreeSet::new();
            let mut next = item.id;
            while let Some(d) = data.get(&next) {
                if !visited.insert(next) {
                    report(None, format!("{}: alias cycle", item.name));
                    break;
                }
                if let Mir68kDataPlacement::Alias { target, .. } = d.placement {
                    next = target;
                } else {
                    break;
                }
            }
        }
        for reloc in &item.relocations {
            if reloc
                .offset
                .get()
                .checked_add(reloc.width.get())
                .is_none_or(|end| end as usize > item.bytes.len())
                || !matches!(reloc.width.get(), 1 | 2 | 4)
                || reloc
                    .byte_index
                    .is_some_and(|b| b >= 4 || reloc.width.get() != 1)
            {
                report(
                    None,
                    format!("{}: invalid relocation encoding or extent", item.name),
                );
            }
            let found = match reloc.target {
                Mir68kRelocationTarget::Data(NirStorageId::Global(id)) => {
                    data.contains_key(&Mir68kDataId::Global(id))
                }
                Mir68kRelocationTarget::Data(_) => false, // no ambient frame for static data
                Mir68kRelocationTarget::ArrayBacking(id) => {
                    data.contains_key(&Mir68kDataId::ArrayBacking(id))
                }
                Mir68kRelocationTarget::Code(id) => routines.contains_key(&id),
                _ => true,
            };
            if !found {
                report(None, format!("{}: missing relocation target", item.name));
            }
        }
    }
    for routine in &program.routines {
        if let Err(error) = super::lower::verify_routine_plan(routine) {
            report(Some(&routine.name), error);
        }
        let blocks: BTreeMap<_, _> = routine.blocks.iter().map(|b| (b.id, b)).collect();
        let temps: BTreeMap<_, _> = routine.temps.iter().map(|(id, ty)| (*id, ty)).collect();
        let params: BTreeMap<_, _> = routine.params.iter().map(|(id, ty)| (*id, ty)).collect();
        if blocks.len() != routine.blocks.len()
            || temps.len() != routine.temps.len()
            || params.len() != routine.params.len()
        {
            report(
                Some(&routine.name),
                "duplicate block/temp/parameter identity".into(),
            );
        }
        let mut definitions = BTreeSet::new();
        let mut values = Vec::new();
        let mut addresses = Vec::new();
        for block in &routine.blocks {
            for (id, ty) in &block.params {
                if temps.get(id).copied() != Some(ty) || !definitions.insert(*id) {
                    report(
                        Some(&routine.name),
                        "invalid block parameter type or definition".into(),
                    );
                }
            }
            for (index, op) in block.ops.iter().enumerate() {
                if let Some((id, width)) = op_result(op) {
                    if temps.get(&id).and_then(|t| t.width) != Some(width)
                        || !definitions.insert(id)
                    {
                        report(
                            Some(&routine.name),
                            format!("conflicting definition/type for {id:?}"),
                        );
                    }
                }
                match op {
                    Mir68kOp::Fault(_) => {
                        if index + 1 != block.ops.len()
                            || !matches!(block.terminator, Mir68kTerminator::Exit)
                        {
                            report(
                                Some(&routine.name),
                                "native fault must end an Exit block".into(),
                            );
                        }
                    }
                    Mir68kOp::Load { address, .. } | Mir68kOp::AddressOf { address, .. } => {
                        addresses.push(address)
                    }
                    Mir68kOp::Store { address, value, .. } => {
                        addresses.push(address);
                        values.push(value);
                    }
                    Mir68kOp::Copy {
                        destination,
                        source,
                        ..
                    } => {
                        addresses.extend([destination, source]);
                    }
                    Mir68kOp::Unary { value, .. } | Mir68kOp::Cast { value, .. } => {
                        values.push(value)
                    }
                    Mir68kOp::PointerOffset { base, offset, .. } => values.extend([base, offset]),
                    Mir68kOp::Binary { left, right, .. }
                    | Mir68kOp::Compare { left, right, .. } => values.extend([left, right]),
                    Mir68kOp::Call {
                        target,
                        signature,
                        args,
                        plan,
                        ..
                    } => {
                        values.extend(args);
                        match target {
                            Mir68kCallTarget::Indirect(value, _) => values.push(value),
                            Mir68kCallTarget::Direct(id) => {
                                if routines
                                    .get(id)
                                    .is_none_or(|r| Some(r.signature.id) != *signature)
                                {
                                    report(
                                        Some(&routine.name),
                                        "missing direct callee or incompatible signature".into(),
                                    );
                                }
                            }
                            _ => {}
                        }
                        if args.len() != plan.arguments.len() {
                            report(
                                Some(&routine.name),
                                "call argument homes do not match values".into(),
                            );
                        }
                        for (value, home) in args.iter().zip(&plan.arguments) {
                            if let Mir68kAbiHome::StackArgument { size, .. } = home {
                                if value_width(value, routine) != Some(*size) {
                                    report(
                                        Some(&routine.name),
                                        "call argument width differs from ABI home".into(),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            let edges: Vec<&Mir68kEdge> = match &block.terminator {
                Mir68kTerminator::Goto(edge) => vec![edge],
                Mir68kTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => {
                    values.push(condition);
                    vec![then_edge, else_edge]
                }
                Mir68kTerminator::Return { value, .. } => {
                    if let Some(value) = value {
                        values.push(value);
                    }
                    if value.as_ref().and_then(|v| value_width(v, routine))
                        != routine.signature.result.as_ref().and_then(|t| t.width)
                    {
                        report(
                            Some(&routine.name),
                            "return width differs from routine result".into(),
                        );
                    }
                    Vec::new()
                }
                _ => Vec::new(),
            };
            for edge in edges {
                values.extend(&edge.args);
                if blocks.get(&edge.target).is_none_or(|b| {
                    b.params.len() != edge.args.len()
                        || b.params
                            .iter()
                            .zip(&edge.args)
                            .any(|((_, ty), v)| ty.width != value_width(v, routine))
                }) {
                    report(
                        Some(&routine.name),
                        "missing edge target or mismatched block arguments".into(),
                    );
                }
            }
        }
        for address in addresses {
            if let Some(index) = &address.index {
                values.push(&index.value);
            }
            let found = match &address.base {
                Mir68kAddressBase::Static(NirStorageId::Global(id)) => {
                    data.contains_key(&Mir68kDataId::Global(*id))
                }
                Mir68kAddressBase::Static(NirStorageId::Local(id)) => {
                    data.contains_key(&Mir68kDataId::Local(routine.id, *id))
                }
                Mir68kAddressBase::Static(NirStorageId::Param(_)) => false,
                Mir68kAddressBase::AutomaticFrame(id) => {
                    routine.frame.objects.iter().any(|o| o.id == *id)
                }
                Mir68kAddressBase::Parameter(id) => params.contains_key(id),
                Mir68kAddressBase::External(Mir68kExternalAddress::Global(id)) => {
                    data.contains_key(&Mir68kDataId::Global(*id))
                }
                Mir68kAddressBase::Indirect(value) => {
                    values.push(value);
                    true
                }
                _ => true,
            };
            if !found {
                report(Some(&routine.name), "missing storage/address home".into());
            }
        }
        for value in values {
            let valid = match value {
                Mir68kValue::Temp(id, width) => {
                    temps.get(id).and_then(|t| t.width) == Some(*width) && definitions.contains(id)
                }
                Mir68kValue::Param(id) => params.contains_key(id),
                Mir68kValue::StaticAddress(id, _) => data.contains_key(&Mir68kDataId::Static(*id)),
                Mir68kValue::GlobalAddress(id, _) => data.contains_key(&Mir68kDataId::Global(*id)),
                Mir68kValue::RoutineAddress(id, _) => routines.contains_key(id),
                _ => true,
            };
            if !valid {
                report(
                    Some(&routine.name),
                    format!("missing or mistyped value {value:?}"),
                );
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn value_width(value: &Mir68kValue, routine: &Mir68kRoutine) -> Option<ByteSize> {
    Some(match value {
        Mir68kValue::U8(_) => ByteSize::ONE,
        Mir68kValue::U16(_) => ByteSize::new(2),
        Mir68kValue::U32(_) => ByteSize::new(4),
        Mir68kValue::Null(w)
        | Mir68kValue::Address(_, w)
        | Mir68kValue::StaticAddress(_, w)
        | Mir68kValue::Temp(_, w)
        | Mir68kValue::GlobalAddress(_, w)
        | Mir68kValue::RoutineAddress(_, w) => *w,
        Mir68kValue::Param(id) => routine.params.iter().find(|(p, _)| p == id)?.1.width?,
    })
}

pub fn op_result(op: &Mir68kOp) -> Option<(TempId, ByteSize)> {
    match op {
        Mir68kOp::Load { dest, width, .. }
        | Mir68kOp::AddressOf { dest, width, .. }
        | Mir68kOp::Unary { dest, width, .. }
        | Mir68kOp::Binary { dest, width, .. }
        | Mir68kOp::PointerOffset { dest, width, .. } => Some((*dest, *width)),
        Mir68kOp::Cast { dest, to, .. } => Some((*dest, *to)),
        Mir68kOp::Compare { dest, .. } => Some((*dest, ByteSize::ONE)),
        Mir68kOp::Call { result, .. } => *result,
        _ => None,
    }
}
