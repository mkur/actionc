use std::fmt::Write as _;

use super::facts::{NirStorageId, NirValue, TempId};
use super::ir::*;

#[derive(Default)]
pub(super) struct NirPrinter {
    out: String,
}

impl NirPrinter {
    pub(super) fn program(&mut self, program: &NirProgram) {
        self.line("nir program");
        for global in &program.globals {
            let backing = match global.backing {
                super::ir::NirGlobalBacking::Ordinary => String::new(),
                super::ir::NirGlobalBacking::Absolute(address) => {
                    format!(" absolute ${address:04X}")
                }
                super::ir::NirGlobalBacking::Alias { ref target, offset } => {
                    if offset == 0 {
                        format!(" alias {target}")
                    } else {
                        format!(" alias {target}+{offset}")
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
                .bytes
                .iter()
                .map(|byte| format!("${byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            self.line(format!(
                "static {}:{} bytes=[{}] = {:?}",
                static_data.name, static_data.ty.summary, bytes, static_data.display
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
                    if offset == 0 {
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
                    if offset == 0 {
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
        for block in &routine.blocks {
            self.line(format!("{}:", block.label));
            for op in &block.ops {
                self.line(format!("  {}", op_summary(op)));
            }
            self.line(format!("  {}", terminator_summary(&block.terminator)));
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
            bytes,
            zero_fill,
            mutable,
            section,
        } => format!(
            " init bytes=[{}] zero_fill={} section={} mutable={}",
            bytes_summary(bytes),
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
            " init descriptor size={} backing=g{} bytes=[{}] zero_fill={} backing_section={} size_word={} section={} mutable={}",
            descriptor_size,
            backing.owner.0,
            bytes_summary(&backing.bytes),
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
        NirGlobalInit::ProgramEndWord { mutable, section } => format!(
            " init program_end_word section={} mutable={}",
            section, mutable
        ),
        NirGlobalInit::RoutineAddress {
            name,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => format!(
            " init routine_address {} size={} size_word={} section={} mutable={}",
            name,
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
            bytes,
            zero_fill,
            mutable,
            section,
        } => format!(
            " init bytes=[{}] zero_fill={} section={} mutable={}",
            bytes_summary(bytes),
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
            " init descriptor size={} backing=local bytes=[{}] zero_fill={} backing_section={} size_word={} section={} mutable={}",
            descriptor_size,
            bytes_summary(&backing.bytes),
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

fn op_summary(op: &NirOp) -> String {
    match op {
        NirOp::Define { name, value } => format!("define {name} = {value}"),
        NirOp::Set { address, value } => {
            format!(
                "set {} = {}",
                operand_summary(address),
                operand_summary(value)
            )
        }
        NirOp::Declare { name, kind } => format!("declare {name}: {kind}"),
        NirOp::Assign { target, value } => {
            format!("{} = {}", place_summary(target), operand_summary(value))
        }
        NirOp::CompoundAssign { target, op, value } => {
            format!("{} {op}= {}", place_summary(target), operand_summary(value))
        }
        NirOp::Load { dest, ty, place } => {
            format!(
                "{}:{} = load {}",
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
            op,
            left,
            right,
        } => format!(
            "{}:{} = cmp {} {} {}",
            temp_summary(*dest),
            ty.summary,
            value_summary(left),
            compare_op_summary(*op),
            value_summary(right)
        ),
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
        NirOp::Unsupported { note } => format!("unsupported {note}"),
        NirOp::Note { text } => format!("note {text}"),
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
        NirMemoryRegionKind::AbsoluteRange => "absolute".to_string(),
        NirMemoryRegionKind::ZeroPage => "zeropage".to_string(),
    };
    format!("{kind}+{}:{}", region.offset, region.size)
}

fn callee_summary(callee: &NirCallee) -> String {
    match callee {
        NirCallee::User(name) | NirCallee::Builtin(name) => name.clone(),
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

fn terminator_summary(terminator: &NirTerminator) -> String {
    match terminator {
        NirTerminator::Open => "open".to_string(),
        NirTerminator::Fallthrough => "fallthrough".to_string(),
        NirTerminator::Goto(label) => format!("goto {label}"),
        NirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => format!(
            "branch {} ? {then_label} : {else_label}",
            value_summary(condition)
        ),
        NirTerminator::Return(value) => value
            .as_ref()
            .map(|value| format!("return {}", value_summary(value)))
            .unwrap_or_else(|| "return".to_string()),
        NirTerminator::Exit => "exit".to_string(),
        NirTerminator::Unknown(note) => format!("unknown {note}"),
    }
}
fn operand_summary(operand: &NirOperand) -> String {
    match &operand.kind {
        NirOperandKind::Missing => "<missing>".to_string(),
        NirOperandKind::Raw(raw) => raw.clone(),
        NirOperandKind::UnresolvedName(name) => format!("unresolved({name})"),
        NirOperandKind::CurrentLocation => "*".to_string(),
        NirOperandKind::Literal { text, .. } => text.clone(),
        NirOperandKind::Temp(temp) => temp_summary(*temp),
        NirOperandKind::Symbol(symbol) => symbol.clone(),
        NirOperandKind::Place(place) => place_summary(place),
        NirOperandKind::AddressOf(place) => format!("&{}", place_summary(place)),
        NirOperandKind::AddressOfSymbol(symbol) => format!("&{symbol}"),
        NirOperandKind::Expr(expr) => expr.clone(),
        NirOperandKind::Call(call) => call.clone(),
    }
}

fn value_summary(value: &NirValue) -> String {
    match value {
        NirValue::ConstU8(value) => value.to_string(),
        NirValue::ConstU16(value) => format!("${value:X}"),
        NirValue::StaticAddr { name, .. } => format!("&{name}"),
        NirValue::Temp { id, .. } => temp_summary(*id),
        NirValue::Param(id) => format!("param{}", id.0),
        NirValue::GlobalAddr(id) => format!("global_addr{}", id.0),
    }
}

fn temp_summary(temp: TempId) -> String {
    format!("%t{}", temp.0)
}

fn place_summary(place: &NirPlace) -> String {
    match &place.kind {
        NirPlaceKind::Symbol(symbol) => symbol.clone(),
        NirPlaceKind::Param { name, .. }
        | NirPlaceKind::Local { name, .. }
        | NirPlaceKind::Global { name, .. } => name.clone(),
        NirPlaceKind::Absolute(address) => format!("@${address:04X}"),
        NirPlaceKind::UnresolvedName(name) => format!("unresolved({name})"),
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
