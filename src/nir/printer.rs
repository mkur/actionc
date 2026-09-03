use std::fmt::Write as _;

use super::facts::{BlockId, NirStorageId, NirValue, TempId};
use super::ir::*;

#[derive(Default)]
pub(super) struct NirPrinter {
    out: String,
}

impl NirPrinter {
    pub(super) fn program(&mut self, program: &NirProgram) {
        self.line("nir program");
        if program.target_layout.target != crate::target::TargetId::Atari6502 {
            self.line(format!(
                "target {} cpu={:?} endian={:?} address_bits={} data_pointer={} code_pointer={} abi={:?}",
                program.target_layout.target,
                program.target_layout.cpu,
                program.target_layout.endian,
                program.target_layout.address_bits,
                program.target_layout.data_pointer.size_bytes,
                program.target_layout.code_pointer.size_bytes,
                program.target_layout.abi,
            ));
        }
        for global in &program.globals {
            let backing = match global.backing {
                super::ir::NirGlobalBacking::Ordinary => String::new(),
                super::ir::NirGlobalBacking::Absolute(address) => {
                    format!(" absolute ${address:04X}")
                }
                super::ir::NirGlobalBacking::Alias { ref target, offset } => {
                    if offset == crate::target::ByteOffset::ZERO {
                        format!(" alias g{}", target.0)
                    } else {
                        format!(" alias g{}+{offset}", target.0)
                    }
                }
            };
            self.line(format!(
                "global {}: {}{}{}",
                global.name,
                global.kind,
                backing,
                global_init_suffix(global.init.as_ref())
            ));
        }
        for static_data in &program.statics {
            let bytes = static_data
                .image
                .bytes
                .iter()
                .map(|byte| format!("${byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            self.line(format!(
                "static {}:{} bytes=[{}]{} = {:?}",
                static_data.name,
                static_data.ty.summary,
                bytes,
                fragments_summary(&static_data.image),
                static_data.display
            ));
        }
        for routine in &program.routines {
            self.routine(routine);
        }
    }

    fn routine(&mut self, routine: &NirRoutine) {
        self.line("");
        let params = if routine.params.is_empty() {
            "-".to_string()
        } else {
            routine
                .params
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let locals = if routine.locals.is_empty() {
            "-".to_string()
        } else {
            routine
                .locals
                .iter()
                .map(|local| local.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.line(format!(
            "routine {} params=[{}] locals=[{}]",
            routine.name, params, locals
        ));
        for local in &routine.locals {
            let backing = match local.backing {
                super::ir::NirLocalBacking::Ordinary => String::new(),
                super::ir::NirLocalBacking::Absolute(address) => {
                    format!(" absolute ${address:04X}")
                }
                super::ir::NirLocalBacking::Alias {
                    ref target_name,
                    offset,
                    ..
                } => {
                    if offset == crate::target::ByteOffset::ZERO {
                        format!(" alias {target_name}")
                    } else {
                        format!(" alias {target_name}+{offset}")
                    }
                }
                super::ir::NirLocalBacking::GlobalAlias {
                    ref target_name,
                    offset,
                    ..
                } => {
                    if offset == crate::target::ByteOffset::ZERO {
                        format!(" global-alias {target_name}")
                    } else {
                        format!(" global-alias {target_name}+{offset}")
                    }
                }
            };
            self.line(format!(
                "  local {}: {}{}{}",
                local.name,
                local.kind,
                backing,
                storage_init_suffix(local.init.as_ref())
            ));
        }
        for note in &routine.notes {
            if note.text.starts_with("return-width ") {
                continue;
            }
            self.line(format!("  note {}", note.text));
        }
        let block_labels = routine
            .blocks
            .iter()
            .map(|block| (block.id, block.label.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        for block in &routine.blocks {
            let params = block
                .params
                .iter()
                .map(|param| format!("{}:{}", temp_summary(param.dest), param.ty.summary))
                .collect::<Vec<_>>()
                .join(", ");
            if params.is_empty() {
                self.line(format!("{}:", block.label));
            } else {
                self.line(format!("{}({params}):", block.label));
            }
            for op in &block.ops {
                self.line(format!("  {}", op_summary(op)));
            }
            self.line(format!(
                "  {}",
                terminator_summary(&block.terminator, &block_labels)
            ));
        }
    }

    pub(super) fn finish(self) -> String {
        self.out
    }

    fn line(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.out, "{}", line.as_ref());
    }
}

fn global_init_suffix(init: Option<&NirGlobalInit>) -> String {
    let Some(init) = init else {
        return String::new();
    };
    match init {
        NirGlobalInit::Bytes {
            image,
            zero_fill,
            mutable,
            section,
        } => format!(
            " init bytes=[{}]{} zero_fill={} section={} mutable={}",
            bytes_summary(&image.bytes),
            fragments_summary(image),
            zero_fill,
            section,
            mutable
        ),
        NirGlobalInit::Descriptor {
            backing,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => format!(
            " init descriptor size={} backing=g{} bytes=[{}]{} zero_fill={} backing_section={} size_word={} section={} mutable={}",
            descriptor_size,
            backing.owner.0,
            bytes_summary(&backing.image.bytes),
            fragments_summary(&backing.image),
            backing.zero_fill,
            backing.section,
            size_word
                .map(|value| format!("${value:04X}"))
                .unwrap_or_else(|| "-".to_string()),
            section,
            mutable
        ),
        NirGlobalInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => format!(
            " init zero_fill={} section={} mutable={}",
            bytes, section, mutable
        ),
        NirGlobalInit::LinkValue {
            value,
            width,
            mutable,
            section,
        } => format!(
            " init link_value={value:?} width={width} section={section} mutable={mutable}"
        ),
        NirGlobalInit::RoutineAddress {
            routine,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => format!(
            " init routine_address {} size={} size_word={} section={} mutable={}",
            format_args!("r{routine}"),
            descriptor_size,
            size_word
                .map(|value| format!("${value:04X}"))
                .unwrap_or_else(|| "-".to_string()),
            section,
            mutable
        ),
    }
}

fn storage_init_suffix(init: Option<&NirStorageInit>) -> String {
    let Some(init) = init else {
        return String::new();
    };
    match init {
        NirStorageInit::Bytes {
            image,
            zero_fill,
            mutable,
            section,
        } => format!(
            " init bytes=[{}]{} zero_fill={} section={} mutable={}",
            bytes_summary(&image.bytes),
            fragments_summary(image),
            zero_fill,
            section,
            mutable
        ),
        NirStorageInit::Descriptor {
            backing,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => format!(
            " init descriptor size={} backing=local bytes=[{}]{} zero_fill={} backing_section={} size_word={} section={} mutable={}",
            descriptor_size,
            bytes_summary(&backing.image.bytes),
            fragments_summary(&backing.image),
            backing.zero_fill,
            backing.section,
            size_word
                .map(|value| format!("${value:04X}"))
                .unwrap_or_else(|| "-".to_string()),
            section,
            mutable
        ),
        NirStorageInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => format!(
            " init zero_fill={} section={} mutable={}",
            bytes, section, mutable
        ),
    }
}

fn bytes_summary(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("${byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fragments_summary(image: &NirDataImage) -> String {
    if image.fragments.is_empty() {
        return String::new();
    }
    let fragments = image
        .fragments
        .iter()
        .map(|fragment| match fragment {
            NirDataFragment::Integer {
                offset,
                width,
                value,
            } => format!("{offset}:int{}=${value:X}", width.get() * 8),
            NirDataFragment::Address {
                offset,
                encoding,
                target,
                addend,
                ..
            } => {
                let encoding = match encoding {
                    NirDataAddressEncoding::Pointer {
                        address_space,
                        width,
                    } => format!("ptr{}@as{}", width.get() * 8, address_space.0),
                    NirDataAddressEncoding::TargetByte { target, byte_index } => {
                        format!("{}-byte{byte_index}", target.as_str())
                    }
                };
                let target = match target {
                    NirDataAddressTarget::Storage(NirStorageId::Global(id)) => {
                        format!("g{}", id.0)
                    }
                    NirDataAddressTarget::Storage(NirStorageId::Local(id)) => {
                        format!("l{}", id.0)
                    }
                    NirDataAddressTarget::Storage(NirStorageId::Param(id)) => {
                        format!("p{}", id.0)
                    }
                    NirDataAddressTarget::Routine(id) => format!("r{id}"),
                    NirDataAddressTarget::Absolute(address) => format!("${address:04X}"),
                };
                let addend = match addend {
                    0 => String::new(),
                    value if *value > 0 => format!("+{value}"),
                    value => value.to_string(),
                };
                format!("{offset}:{encoding}({target}{addend})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(" fragments=[{fragments}]")
}

fn op_summary(op: &NirOp) -> String {
    match op {
        NirOp::RuntimeHelperOverride { slot, target } => format!(
            "runtime_helper_override ${slot:04X} {}",
            match target {
                NirRuntimeHelperTarget::Absolute(address) => format!("${address:04X}"),
                NirRuntimeHelperTarget::Routine(id) => format!("r{id}"),
            }
        ),
        NirOp::Load { dest, ty, place } => {
            format!(
                "{}:{} = load {}",
                temp_summary(*dest),
                ty.summary,
                place_summary(place)
            )
        }
        NirOp::VolatileLoad { dest, ty, place } => {
            format!(
                "{}:{} = load volatile {}",
                temp_summary(*dest),
                ty.summary,
                place_summary(place)
            )
        }
        NirOp::AddrOf { dest, ty, place } => {
            format!(
                "{}:{} = addr {}",
                temp_summary(*dest),
                ty.summary,
                place_summary(place)
            )
        }
        NirOp::Store { place, src, .. } => {
            format!("store {} = {}", place_summary(place), value_summary(src))
        }
        NirOp::VolatileStore { place, src, .. } => {
            format!(
                "store volatile {} = {}",
                place_summary(place),
                value_summary(src)
            )
        }
        NirOp::CopyBytes {
            destination,
            source,
            size,
            destination_volatile,
            source_volatile,
        } => format!(
            "copy_bytes {} = {} size={}{}{}",
            place_summary(destination),
            place_summary(source),
            size,
            source_volatile.then_some(" source-volatile").unwrap_or(""),
            destination_volatile
                .then_some(" destination-volatile")
                .unwrap_or("")
        ),
        NirOp::Unary { dest, ty, op, src } => {
            format!(
                "{}:{} = {} {}",
                temp_summary(*dest),
                ty.summary,
                unary_op_summary(*op),
                value_summary(src)
            )
        }
        NirOp::Cast {
            dest,
            src,
            from,
            to,
            ..
        } => {
            format!(
                "{}:{} = cast {} -> {} {}",
                temp_summary(*dest),
                to.summary,
                from.summary,
                to.summary,
                value_summary(src)
            )
        }
        NirOp::PointerOffset {
            dest,
            ty,
            base,
            offset,
            subtract,
        } => format!(
            "{}:{} = ptr_offset {} {} {}",
            temp_summary(*dest),
            ty.summary,
            value_summary(base),
            if *subtract { "-" } else { "+" },
            value_summary(offset)
        ),
        NirOp::Binary {
            dest,
            ty,
            op,
            left,
            right,
        } => format!(
            "{}:{} = {} {} {}",
            temp_summary(*dest),
            ty.summary,
            value_summary(left),
            binary_op_summary(*op),
            value_summary(right)
        ),
        NirOp::Compare {
            dest,
            ty,
            operand_ty,
            op,
            left,
            right,
        } => format!(
            "{}:{} = cmp {} {} {} operand_ty={}",
            temp_summary(*dest),
            ty.summary,
            value_summary(left),
            compare_op_summary(*op),
            value_summary(right),
            operand_ty.summary
        ),
        NirOp::Real(real) => real_op_summary(real),
        NirOp::Call {
            callee,
            args,
            result,
            signature: _,
            effects,
        } => {
            let callee = callee_summary(callee);
            let args = args
                .iter()
                .map(value_summary)
                .collect::<Vec<_>>()
                .join(", ");
            let call = result
                .as_ref()
                .map(|result| {
                    format!(
                        "{}:{} = call {callee}({args})",
                        temp_summary(result.dest),
                        result.ty.summary
                    )
                })
                .unwrap_or_else(|| format!("call {callee}({args})"));
            format!("{call}{}", call_effects_suffix(effects))
        }
        NirOp::MachineBlock { items, effects } => {
            format!(
                "machine items={} effects={}",
                items.len(),
                machine_effects_summary(effects)
            )
        }
        NirOp::InlineAsm { code, effects } => {
            format!(
                "inline-asm bytes={} relocations={} effects={}",
                code.bytes.len(),
                code.relocations.len(),
                machine_effects_summary(effects)
            )
        }
        NirOp::Unsupported { note } => format!("unsupported {note}"),
    }
}

fn real_op_summary(op: &NirRealOp) -> String {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => format!(
            "real.copy {} = {}",
            place_summary(destination),
            real_source_summary(source)
        ),
        NirRealOp::Unary {
            operation,
            destination,
            operand,
        } => format!(
            "real.{} {} = {}",
            unary_op_summary(*operation).to_ascii_lowercase(),
            place_summary(destination),
            real_source_summary(operand)
        ),
        NirRealOp::Binary {
            operation,
            destination,
            left,
            right,
        } => format!(
            "real.{} {} = {}, {}",
            binary_op_summary(*operation).to_ascii_lowercase(),
            place_summary(destination),
            real_source_summary(left),
            real_source_summary(right)
        ),
        NirRealOp::Compare {
            predicate,
            result,
            result_type: _,
            left,
            right,
        } => format!(
            "{}:condition = real.cmp.{} {}, {}",
            temp_summary(*result),
            compare_op_summary(*predicate).to_ascii_lowercase(),
            real_source_summary(left),
            real_source_summary(right)
        ),
        NirRealOp::IntegerToReal {
            destination,
            source,
            source_type,
        } => format!(
            "real.convert {} = {}:{}",
            place_summary(destination),
            value_summary(source),
            source_type.summary
        ),
        NirRealOp::RealToInteger {
            result,
            result_type,
            source,
        } => format!(
            "{}:{} = real.convert {}",
            temp_summary(*result),
            result_type.summary,
            place_summary(source)
        ),
    }
}

fn real_source_summary(source: &NirRealSource) -> String {
    match source {
        NirRealSource::Place(place) => place_summary(place),
        NirRealSource::Static { name, .. } => format!("&{name}"),
    }
}

fn call_effects_suffix(effects: &NirCallEffects) -> String {
    if !effects.opaque
        && !effects.may_call_os
        && matches!(effects.memory.reads, NirMemoryAccess::None)
        && matches!(effects.memory.writes, NirMemoryAccess::None)
    {
        return String::new();
    }
    format!(
        " effects=reads:{} writes:{}{}{}",
        memory_access_summary(&effects.memory.reads),
        memory_access_summary(&effects.memory.writes),
        if effects.may_call_os { " os" } else { "" },
        if effects.opaque { " opaque" } else { "" },
    )
}

fn machine_effects_summary(effects: &NirMachineEffects) -> String {
    if effects.opaque || effects.may_call_os {
        "opaque".to_string()
    } else if matches!(effects.memory.reads, NirMemoryAccess::None)
        && matches!(effects.memory.writes, NirMemoryAccess::None)
    {
        "none".to_string()
    } else {
        format!(
            "reads:{} writes:{}",
            memory_access_summary(&effects.memory.reads),
            memory_access_summary(&effects.memory.writes)
        )
    }
}

fn memory_access_summary(access: &NirMemoryAccess) -> String {
    match access {
        NirMemoryAccess::None => "none".to_string(),
        NirMemoryAccess::Unknown => "unknown".to_string(),
        NirMemoryAccess::All => "all".to_string(),
        NirMemoryAccess::Regions(regions) => regions
            .iter()
            .map(memory_region_summary)
            .collect::<Vec<_>>()
            .join("|"),
    }
}

fn memory_region_summary(region: &NirMemoryRegion) -> String {
    let kind = match region.kind {
        NirMemoryRegionKind::Storage(NirStorageId::Local(id)) => format!("local{}", id.0),
        NirMemoryRegionKind::Storage(NirStorageId::Param(id)) => format!("param{}", id.0),
        NirMemoryRegionKind::Storage(NirStorageId::Global(id)) => format!("global{}", id.0),
        NirMemoryRegionKind::Static(id) => format!("static{}", id.0),
        NirMemoryRegionKind::AbsoluteRange(space)
            if space == crate::target::TargetLayout::DATA_ADDRESS_SPACE =>
        {
            "absolute".to_string()
        }
        NirMemoryRegionKind::AbsoluteRange(space) => format!("absolute-as{}", space.0),
        NirMemoryRegionKind::ZeroPage => "zeropage".to_string(),
    };
    format!("{kind}+{}:{}", region.offset, region.size)
}

fn callee_summary(callee: &NirCallee) -> String {
    match callee {
        NirCallee::User { name, .. } | NirCallee::Builtin(name) => name.clone(),
        NirCallee::Indirect { target, .. } => format!("indirect({})", value_summary(target)),
        NirCallee::Runtime { name, address } => address
            .map(|address| format!("{name}@${address:04X}"))
            .unwrap_or_else(|| name.clone()),
    }
}

fn unary_op_summary(op: NirUnaryOp) -> &'static str {
    match op {
        NirUnaryOp::Plus => "Plus",
        NirUnaryOp::Neg => "Neg",
    }
}

fn binary_op_summary(op: NirBinaryOp) -> &'static str {
    match op {
        NirBinaryOp::Add => "Add",
        NirBinaryOp::Sub => "Sub",
        NirBinaryOp::Mul => "Mul",
        NirBinaryOp::Div => "Div",
        NirBinaryOp::Mod => "Mod",
        NirBinaryOp::Lsh => "Lsh",
        NirBinaryOp::Rsh => "Rsh",
        NirBinaryOp::And => "And",
        NirBinaryOp::Or => "Or",
        NirBinaryOp::Xor => "Xor",
    }
}

fn compare_op_summary(op: NirCompareOp) -> &'static str {
    match op {
        NirCompareOp::Eq => "Eq",
        NirCompareOp::Ne => "Ne",
        NirCompareOp::Lt => "Lt",
        NirCompareOp::Le => "Le",
        NirCompareOp::Gt => "Gt",
        NirCompareOp::Ge => "Ge",
    }
}

fn terminator_summary(
    terminator: &NirTerminator,
    labels: &std::collections::BTreeMap<BlockId, &str>,
) -> String {
    match terminator {
        NirTerminator::Open => "open".to_string(),
        NirTerminator::Fallthrough => "fallthrough".to_string(),
        NirTerminator::Goto(edge) => format!("goto {}", edge_summary(edge, labels)),
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => format!(
            "branch {} ? {} : {}",
            value_summary(condition),
            edge_summary(then_edge, labels),
            edge_summary(else_edge, labels)
        ),
        NirTerminator::Return(value) => value
            .as_ref()
            .map(|value| format!("return {}", value_summary(value)))
            .unwrap_or_else(|| "return".to_string()),
        NirTerminator::Exit => "exit".to_string(),
    }
}

fn edge_summary(edge: &NirEdge, labels: &std::collections::BTreeMap<BlockId, &str>) -> String {
    let target = labels
        .get(&edge.target)
        .copied()
        .map(str::to_string)
        .unwrap_or_else(|| format!("bbid{}", edge.target.0));
    if edge.args.is_empty() {
        target
    } else {
        let args = edge
            .args
            .iter()
            .map(value_summary)
            .collect::<Vec<_>>()
            .join(", ");
        format!("{target}({args})")
    }
}

fn value_summary(value: &NirValue) -> String {
    match value {
        NirValue::ConstU8(value) => value.to_string(),
        NirValue::ConstU16(value) => format!("${value:X}"),
        NirValue::Null { .. } => "$0".to_string(),
        NirValue::AddressConst { address, .. } => format!("${address:X}"),
        NirValue::StaticAddr { name, .. } => format!("&{name}"),
        NirValue::Temp { id, .. } => temp_summary(*id),
        NirValue::Param(id) => format!("param{}", id.0),
        NirValue::GlobalAddr(id) => format!("global_addr{}", id.0),
        NirValue::RoutineAddr { id, name, .. } => format!("routine_addr{id}({name})"),
    }
}

fn temp_summary(temp: TempId) -> String {
    format!("%t{}", temp.0)
}

fn place_summary(place: &NirPlace) -> String {
    match &place.kind {
        NirPlaceKind::Param { name, .. }
        | NirPlaceKind::Local { name, .. }
        | NirPlaceKind::Global { name, .. } => name.clone(),
        NirPlaceKind::Absolute(address) => format!("@${address:04X}"),
        NirPlaceKind::Deref { addr } => format!("*{}", value_summary(addr)),
        NirPlaceKind::Index {
            base_addr,
            index,
            elem_size,
            ..
        } => format!(
            "{}[{};{}]",
            value_summary(base_addr),
            value_summary(index),
            elem_size
        ),
        NirPlaceKind::Field { base, offset, .. } => {
            format!("{}.+{}", place_summary(base), offset)
        }
    }
}
