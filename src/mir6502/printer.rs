use std::fmt::Write as _;

use super::ir::{
    MirAddr, MirAddressConsumer, MirArgHome, MirBinaryOp, MirCallTarget, MirCarryIn, MirCarryOut,
    MirCompareOp, MirCond, MirCondDest, MirDef, MirEdge, MirFlagTest, MirGlobalBacking,
    MirGlobalInit, MirMem, MirMemoryEffect, MirOp, MirPointerPair, MirProgram, MirReg,
    MirResultHome, MirRuntimeHelper, MirStorageInit, MirTerminator, MirUnaryOp, MirValue, MirWidth,
};

pub(super) fn format_program(program: &MirProgram) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "mir6502 program");
    for global in &program.globals {
        let _ = write!(
            out,
            "global g{} {}: {} {}{}",
            global.id.0,
            global.name,
            global
                .width
                .map(width_name)
                .unwrap_or_else(|| global.kind.as_str()),
            global_backing_summary(&global.backing),
            global_size_suffix(global.width, global.storage_size),
        );
        if let Some(init) = &global.init {
            let _ = write!(out, "{}", global_init_summary(init));
        }
        let _ = writeln!(out);
    }
    for static_data in &program.statics {
        let bytes = static_data
            .bytes
            .iter()
            .map(|byte| format!("${byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            out,
            "static s{} {}: {} bytes [{}] section={} align={} mutable={} display={:?}",
            static_data.id.0,
            static_data.name,
            static_data.ty,
            bytes,
            static_data.section,
            static_data.alignment,
            static_data.mutable,
            static_data.display
        );
    }
    for machine in &program.machine_blocks {
        let _ = writeln!(
            out,
            "machine m{} items=[{}]",
            machine.id.0,
            machine
                .items
                .iter()
                .map(machine_item_summary)
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    for routine in &program.routines {
        let _ = writeln!(out);
        let _ = writeln!(out, "routine r{} {}", routine.id.0, routine.name);
        for local in &routine.frame.locals {
            if let Some(init) = &local.init {
                let _ = writeln!(out, "  local l{}{}", local.id.0, storage_init_summary(init));
            }
        }
        for allocation in &routine.frame.zero_page_allocations {
            let _ = writeln!(
                out,
                "  zp{} -> ${:02X} size={}",
                allocation.slot.0, allocation.start.0, allocation.size
            );
        }
        for block in &routine.blocks {
            let params = block
                .params
                .iter()
                .map(|param| format!("v{}:{}", param.dest.0, width_name(param.width)))
                .collect::<Vec<_>>()
                .join(", ");
            if params.is_empty() {
                let _ = writeln!(out, "b{} {}:", block.id.0, block.label);
            } else {
                let _ = writeln!(out, "b{} {}({params}):", block.id.0, block.label);
            }
            for op in &block.ops {
                let _ = writeln!(out, "  {}", op_summary(op));
            }
            let _ = writeln!(out, "  {}", terminator_summary(&block.terminator));
        }
    }
    out
}

fn global_init_summary(init: &MirGlobalInit) -> String {
    match init {
        MirGlobalInit::Bytes {
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
        MirGlobalInit::Descriptor {
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
        MirGlobalInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => format!(
            " init zero_fill={} section={} mutable={}",
            bytes, section, mutable
        ),
        MirGlobalInit::ProgramEndWord { mutable, section } => format!(
            " init program_end_word section={} mutable={}",
            section, mutable
        ),
        MirGlobalInit::RoutineAddress {
            routine,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => format!(
            " init routine_address r{} size={} size_word={} section={} mutable={}",
            routine.0,
            descriptor_size,
            size_word
                .map(|value| format!("${value:04X}"))
                .unwrap_or_else(|| "-".to_string()),
            section,
            mutable
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

fn storage_init_summary(init: &MirStorageInit) -> String {
    match init {
        MirStorageInit::Bytes {
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
        MirStorageInit::Descriptor {
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
        MirStorageInit::RoutineAddress {
            routine,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => format!(
            " init routine_address r{} size={} size_word={} section={} mutable={}",
            routine.0,
            descriptor_size,
            size_word
                .map(|value| format!("${value:04X}"))
                .unwrap_or_else(|| "-".to_string()),
            section,
            mutable
        ),
        MirStorageInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => format!(
            " init zero_fill={} section={} mutable={}",
            bytes, section, mutable
        ),
    }
}

fn machine_item_summary(item: &super::ir::MirMachineItem) -> String {
    match item {
        super::ir::MirMachineItem::Byte(value) => format!("${value:02X}"),
        super::ir::MirMachineItem::Word(value) => format!("${value:04X}"),
        super::ir::MirMachineItem::StringLiteral(value) => format!("\"{value}\""),
        super::ir::MirMachineItem::CharLiteral(value) => format!("'{value}'"),
        super::ir::MirMachineItem::Name(name) => name.clone(),
        super::ir::MirMachineItem::AddressExpr { text, .. } => text.clone(),
        super::ir::MirMachineItem::AddressByte { high, name } => {
            let selector = if *high { ">" } else { "<" };
            format!("{selector}{name}")
        }
    }
}

fn global_backing_summary(backing: &MirGlobalBacking) -> String {
    match backing {
        MirGlobalBacking::Ordinary { offset } => format!("storage global+{offset}"),
        MirGlobalBacking::Absolute(address) => format!("absolute ${address:04X}"),
        MirGlobalBacking::Alias { target, offset } => {
            if *offset == 0 {
                format!("alias g{}", target.0)
            } else {
                format!("alias g{}+{offset}", target.0)
            }
        }
    }
}

fn global_size_suffix(width: Option<MirWidth>, storage_size: u16) -> String {
    if width.map(width_bytes).unwrap_or(0) == storage_size {
        String::new()
    } else {
        format!(" size={storage_size}")
    }
}

fn width_name(width: MirWidth) -> &'static str {
    match width {
        MirWidth::Byte => "byte",
        MirWidth::Word => "word",
    }
}

fn width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn op_summary(op: &MirOp) -> String {
    match op {
        MirOp::LoadImm { dst, value, width } => {
            format!("{} ={} #{value}", def_summary(dst), width_suffix(*width))
        }
        MirOp::Load { dst, src, width } => {
            format!(
                "{} ={} load {}",
                def_summary(dst),
                width_suffix(*width),
                addr_summary(src)
            )
        }
        MirOp::Store { dst, src, width } => {
            format!(
                "store{} {}, {}",
                width_suffix(*width),
                addr_summary(dst),
                value_summary(src)
            )
        }
        MirOp::MaterializeAddress { consumer, value } => {
            format!(
                "materialize {} <- {}",
                address_consumer_summary(consumer),
                value_summary(value)
            )
        }
        MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            scale,
        } => format!(
            "materialize_indexed {} <- {} + {}*{}",
            address_consumer_summary(consumer),
            value_summary(base),
            value_summary(index),
            scale
        ),
        MirOp::AdvanceAddress {
            consumer,
            index,
            scale,
        } => format!(
            "advance {} += {}*{}",
            address_consumer_summary(consumer),
            value_summary(index),
            scale
        ),
        MirOp::LoadIndirect {
            consumer,
            dst,
            offset,
        } => format!(
            "{} =.b load_indirect {}+{}",
            def_summary(dst),
            address_consumer_summary(consumer),
            offset
        ),
        MirOp::StoreIndirect {
            consumer,
            src,
            offset,
        } => format!(
            "store_indirect {}+{} {}",
            address_consumer_summary(consumer),
            offset,
            value_summary(src)
        ),
        MirOp::IndirectByteCompound {
            op,
            target,
            source,
            offset,
        } => format!(
            "indirect_byte_compound {} {} {}+{}",
            address_consumer_summary(target),
            binary_summary(*op),
            address_consumer_summary(source),
            offset
        ),
        MirOp::Move { dst, src, width } => {
            format!(
                "{} ={} {}",
                def_summary(dst),
                width_suffix(*width),
                value_summary(src)
            )
        }
        MirOp::LeaAddr { dst, target, width } => {
            format!(
                "{} ={} lea {}",
                def_summary(dst),
                width_suffix(*width),
                mem_summary(target)
            )
        }
        MirOp::Extend {
            dst,
            src,
            from_width,
            to_width,
            signed,
        } => format!(
            "{} ={} extend{} {} from{}",
            def_summary(dst),
            width_suffix(*to_width),
            if *signed { ".signed" } else { "" },
            value_summary(src),
            width_suffix(*from_width)
        ),
        MirOp::Truncate {
            dst,
            src,
            from_width,
            to_width,
        } => format!(
            "{} ={} truncate {} from{}",
            def_summary(dst),
            width_suffix(*to_width),
            value_summary(src),
            width_suffix(*from_width)
        ),
        MirOp::Unary {
            op,
            dst,
            src,
            width,
        } => format!(
            "{} ={} {} {}",
            def_summary(dst),
            width_suffix(*width),
            unary_summary(*op),
            value_summary(src)
        ),
        MirOp::Binary {
            op,
            dst,
            left,
            right,
            width,
            carry_in,
            carry_out,
        } => format!(
            "{} ={} {} {} {} carry_in={} carry_out={}",
            def_summary(dst),
            width_suffix(*width),
            value_summary(left),
            binary_summary(*op),
            value_summary(right),
            carry_in_summary(*carry_in),
            carry_out_summary(*carry_out)
        ),
        MirOp::UpdateMem { op, mem, width } => {
            format!(
                "{}{} {}",
                update_summary(*op),
                width_suffix(*width),
                mem_summary(mem)
            )
        }
        MirOp::UpdateIndexedMem { op, base } => {
            format!("{}.b {},x", update_summary(*op), mem_summary(base))
        }
        MirOp::AddByteToWordMem { mem, value } => {
            format!(
                "add_byte_to_word {} {}",
                mem_summary(mem),
                value_summary(value)
            )
        }
        MirOp::SubByteFromWordMem { mem, value } => {
            format!(
                "sub_byte_from_word {} {}",
                mem_summary(mem),
                value_summary(value)
            )
        }
        MirOp::OffsetPointerByIndirectByte {
            op,
            dst,
            source,
            offset,
        } => format!(
            "offset_pointer_by_indirect_byte.w {} {} {} +{}",
            mem_summary(dst),
            binary_summary(*op),
            address_consumer_summary(source),
            offset
        ),
        MirOp::CopyIndirectWord {
            source,
            destination,
            source_offset,
            destination_offset,
        } => format!(
            "copy_indirect_word {}+{} <- {}+{}",
            address_consumer_summary(destination),
            destination_offset,
            address_consumer_summary(source),
            source_offset
        ),
        MirOp::Compare {
            dst,
            op,
            left,
            right,
            width,
            signed,
        } => format!(
            "{} = cmp{}{} {} {} {}",
            cond_dest_summary(dst),
            width_suffix(*width),
            if *signed { ".signed" } else { "" },
            value_summary(left),
            compare_summary(*op),
            value_summary(right)
        ),
        MirOp::CompareIndirectBytes {
            dst,
            op,
            left,
            right,
            offset,
            signed,
        } => format!(
            "{} = cmp_indirect.b{} {}+{} {} {}+{}",
            cond_dest_summary(dst),
            if *signed { ".signed" } else { "" },
            address_consumer_summary(left),
            offset,
            compare_summary(*op),
            address_consumer_summary(right),
            offset
        ),
        MirOp::Call {
            target,
            abi,
            args,
            result,
            effects,
        } => format!(
            "call {} args=[{}] result={} clobbers={} preserves={} effects={}",
            call_target_summary(target),
            args.iter()
                .map(call_arg_summary)
                .collect::<Vec<_>>()
                .join(", "),
            result
                .as_ref()
                .map(call_result_summary)
                .unwrap_or_else(|| "-".to_string()),
            register_set_summary(&abi.clobbers),
            register_set_summary(&abi.preserves),
            effects_summary(effects)
        ),
        MirOp::RuntimeHelper {
            helper,
            args,
            result,
            effects,
        } => format!(
            "helper {} args=[{}] result={} effects={}",
            helper_summary(helper),
            args.iter()
                .map(arg_home_summary)
                .collect::<Vec<_>>()
                .join(", "),
            result
                .as_ref()
                .map(result_home_summary)
                .unwrap_or_else(|| "-".to_string()),
            effects_summary(effects)
        ),
        MirOp::Barrier { effects } => format!("barrier effects={}", effects_summary(effects)),
        MirOp::MachineBlock { id, effects } => {
            format!("machine m{} effects={}", id.0, effects_summary(effects))
        }
    }
}

fn def_summary(def: &MirDef) -> String {
    match def {
        MirDef::VTemp(id) => format!("v{}", id.0),
        MirDef::VTempByte { id, byte } => format!("v{}.b{}", id.0, byte),
        MirDef::Reg(reg) => reg_summary(*reg).to_string(),
    }
}

fn value_summary(value: &MirValue) -> String {
    match value {
        MirValue::ConstU8(value) => format!("#${value:02X}"),
        MirValue::ConstU16(value) => format!("#${value:04X}"),
        MirValue::Def(def) => def_summary(def),
        MirValue::Word { lo, hi } => format!("word({}, {})", value_summary(lo), value_summary(hi)),
        MirValue::StaticAddr(id) => format!("static_addr s{}", id.0),
        MirValue::GlobalAddr(id) => format!("global_addr g{}", id.0),
        MirValue::RoutineAddr(id) => format!("routine_addr r{}", id.0),
        MirValue::RoutineAddrByte { id, byte } => {
            let suffix = if *byte == 0 { "lo" } else { "hi" };
            format!("routine_addr_{suffix} r{}", id.0)
        }
        MirValue::StorageAddrByte { mem, byte } => {
            let suffix = if *byte == 0 { "lo" } else { "hi" };
            format!("storage_addr_{suffix} {}", mem_summary(mem))
        }
        MirValue::PointerCell(mem) => format!("*{}", mem_summary(mem)),
    }
}

fn address_consumer_summary(consumer: &MirAddressConsumer) -> String {
    let pointer = match consumer.pointer_pair() {
        MirPointerPair::Fixed { lo } => format!("zp${:02X}", lo.0),
        MirPointerPair::Virtual(slot) => format!("vzp{}", slot.0),
    };
    if consumer.uses_scaled_y() {
        format!("({pointer}),scaled_y")
    } else {
        format!("({pointer}),y")
    }
}

fn addr_summary(addr: &MirAddr) -> String {
    match addr {
        MirAddr::Direct(mem) => mem_summary(mem),
        MirAddr::Label(label) => format!("label {}", label.0),
        MirAddr::ZeroPageIndexedX { base } => format!("zp{}[x]", base.0),
        MirAddr::AbsoluteIndexedX { base } => format!("{}[x]", mem_summary(base)),
        MirAddr::AbsoluteIndexedY { base } => format!("{}[y]", mem_summary(base)),
        MirAddr::IndirectIndexedY { zp } => format!("(zp{}),y", zp.0),
        MirAddr::FixedIndirectIndexedY { zp } => format!("(fixed_zp ${:02X}),y", zp.0),
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset,
        } => format!(
            "computed {}[{};{}]+{}",
            value_summary(base),
            value_summary(index),
            elem_size,
            offset
        ),
        MirAddr::PointerCell { ptr, offset } => format!("*{}+{}", mem_summary(ptr), offset),
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset,
        } => format!(
            "*{}[{};{}]+{}",
            mem_summary(ptr),
            value_summary(index),
            elem_size,
            offset
        ),
        MirAddr::Deref { ptr, offset } => format!("*{}+{}", value_summary(ptr), offset),
    }
}

fn mem_summary(mem: &MirMem) -> String {
    match mem {
        MirMem::Absolute(address) => format!("${address:04X}"),
        MirMem::Static { id, offset } => format!("static s{}+{}", id.0, offset),
        MirMem::Global { id, offset } => format!("global g{}+{}", id.0, offset),
        MirMem::Local { id, offset } => format!("local l{}+{}", id.0, offset),
        MirMem::Param { id, offset } => format!("param p{}+{}", id.0, offset),
        MirMem::Spill { id, offset } => format!("spill sp{}+{}", id.0, offset),
        MirMem::ZeroPage(slot) => format!("zp{}", slot.0),
        MirMem::FixedZeroPage(slot) => format!("fixed_zp ${:02X}", slot.0),
    }
}

fn width_suffix(width: MirWidth) -> &'static str {
    match width {
        MirWidth::Byte => ".b",
        MirWidth::Word => ".w",
    }
}

fn reg_summary(reg: MirReg) -> &'static str {
    match reg {
        MirReg::A => "a",
        MirReg::X => "x",
        MirReg::Y => "y",
    }
}

fn unary_summary(op: MirUnaryOp) -> &'static str {
    match op {
        MirUnaryOp::Neg => "neg",
        MirUnaryOp::BitNot => "bitnot",
    }
}

fn update_summary(op: super::ir::MirUpdateOp) -> &'static str {
    match op {
        super::ir::MirUpdateOp::Inc => "inc",
        super::ir::MirUpdateOp::Dec => "dec",
    }
}

fn binary_summary(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "add",
        MirBinaryOp::Sub => "sub",
        MirBinaryOp::Mul => "mul",
        MirBinaryOp::Div => "div",
        MirBinaryOp::Mod => "mod",
        MirBinaryOp::Lsh => "lsh",
        MirBinaryOp::Rsh => "rsh",
        MirBinaryOp::And => "and",
        MirBinaryOp::Or => "or",
        MirBinaryOp::Xor => "xor",
    }
}

fn compare_summary(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "eq",
        MirCompareOp::Ne => "ne",
        MirCompareOp::Lt => "lt",
        MirCompareOp::Le => "le",
        MirCompareOp::Gt => "gt",
        MirCompareOp::Ge => "ge",
    }
}

fn cond_dest_summary(dst: &MirCondDest) -> String {
    match dst {
        MirCondDest::Temp(id) => format!("v{}", id.0),
        MirCondDest::Flags => "flags".to_string(),
    }
}

fn carry_in_summary(carry: Option<MirCarryIn>) -> &'static str {
    match carry {
        Some(MirCarryIn::Clear) => "clear",
        Some(MirCarryIn::Set) => "set",
        Some(MirCarryIn::FromPrevious) => "previous",
        None => "-",
    }
}

fn carry_out_summary(carry: MirCarryOut) -> &'static str {
    match carry {
        MirCarryOut::Ignore => "ignore",
        MirCarryOut::Produce => "produce",
    }
}

fn call_target_summary(target: &MirCallTarget) -> String {
    match target {
        MirCallTarget::Routine(id) => format!("r{}", id.0),
        MirCallTarget::Indirect { target, width } => {
            format!("indirect {}{}", value_summary(target), width_suffix(*width))
        }
        MirCallTarget::Builtin { name, address } => address
            .map(|address| format!("{name}@${address:04X}"))
            .unwrap_or_else(|| name.clone()),
        MirCallTarget::Runtime { name, address } => address
            .map(|address| format!("{name}@${address:04X}"))
            .unwrap_or_else(|| name.clone()),
    }
}

fn call_arg_summary(arg: &super::ir::MirCallArg) -> String {
    format!(
        "{}{} -> {}",
        value_summary(&arg.value),
        width_suffix(arg.width),
        arg_home_summary(&arg.home)
    )
}

fn call_result_summary(result: &super::ir::MirCallResult) -> String {
    format!(
        "{}{} <- {}",
        def_summary(&result.dst),
        width_suffix(result.width),
        result_home_summary(&result.home)
    )
}

fn arg_home_summary(home: &MirArgHome) -> String {
    match home {
        MirArgHome::Reg(reg) => reg_summary(*reg).to_string(),
        MirArgHome::RegisterPair { lo, hi } => {
            format!("{}:{}", reg_summary(*lo), reg_summary(*hi))
        }
        MirArgHome::BytePair { lo, hi } => {
            format!("{}:{}", arg_home_summary(lo), arg_home_summary(hi))
        }
        MirArgHome::ZeroPage(slot) => format!("zp{}", slot.0),
        MirArgHome::FixedZeroPage(slot) => format!("fixed_zp ${:02X}", slot.0),
        MirArgHome::Absolute(address) => format!("${address:04X}"),
        MirArgHome::StackFrame { base, offset } => format!("stack ${base:04X}+{offset}"),
    }
}

fn result_home_summary(home: &MirResultHome) -> String {
    match home {
        MirResultHome::Reg(reg) => reg_summary(*reg).to_string(),
        MirResultHome::RegisterPair { lo, hi } => {
            format!("{}:{}", reg_summary(*lo), reg_summary(*hi))
        }
        MirResultHome::ZeroPage(slot) => format!("zp{}", slot.0),
        MirResultHome::FixedZeroPage(slot) => format!("fixed_zp ${:02X}", slot.0),
        MirResultHome::Absolute(address) => format!("${address:04X}"),
        MirResultHome::ReturnSlot { offset } => format!("return+{offset}"),
    }
}

fn helper_summary(helper: &MirRuntimeHelper) -> &'static str {
    match helper {
        MirRuntimeHelper::Mul => "mul",
        MirRuntimeHelper::Div => "div",
        MirRuntimeHelper::Mod => "mod",
        MirRuntimeHelper::Lsh => "lsh",
        MirRuntimeHelper::Rsh => "rsh",
        MirRuntimeHelper::SArgs => "sargs",
    }
}

fn effects_summary(effects: &super::ir::MirEffects) -> String {
    let mut parts = Vec::new();
    if effects.opaque {
        parts.push("opaque".to_string());
    }
    if effects.may_call_os {
        parts.push("os".to_string());
    }
    if effects.stack_depth_delta.is_none() {
        parts.push("stack=?".to_string());
    } else if let Some(delta) = effects.stack_depth_delta {
        parts.push(format!("stack={delta}"));
    }
    parts.push(format!(
        "reads={}",
        memory_effect_summary(&effects.memory_reads)
    ));
    parts.push(format!(
        "writes={}",
        memory_effect_summary(&effects.memory_writes)
    ));
    let clobbers = register_set_summary(&effects.clobbers);
    if clobbers != "-" {
        parts.push(format!("clobbers={clobbers}"));
    }
    let preserves = register_set_summary(&effects.preserves);
    if preserves != "-" {
        parts.push(format!("preserves={preserves}"));
    }
    parts.join(",")
}

fn memory_effect_summary(effect: &MirMemoryEffect) -> String {
    match effect {
        MirMemoryEffect::None => "none".to_string(),
        MirMemoryEffect::Regions(regions) => regions
            .iter()
            .map(|region| {
                let kind = match region.kind {
                    super::ir::MirMemoryRegionKind::Local(id) => format!("local{}", id.0),
                    super::ir::MirMemoryRegionKind::Param(id) => format!("param{}", id.0),
                    super::ir::MirMemoryRegionKind::Global(id) => format!("global{}", id.0),
                    super::ir::MirMemoryRegionKind::Static(id) => format!("static{}", id.0),
                    super::ir::MirMemoryRegionKind::AbsoluteRange => "absolute".to_string(),
                    super::ir::MirMemoryRegionKind::ZeroPage => "zeropage".to_string(),
                    super::ir::MirMemoryRegionKind::Stack => "stack".to_string(),
                };
                format!("{kind}+{}:{}", region.offset, region.size)
            })
            .collect::<Vec<_>>()
            .join("|"),
        MirMemoryEffect::Unknown => "unknown".to_string(),
        MirMemoryEffect::All => "all".to_string(),
    }
}

fn register_set_summary(registers: &super::ir::MirRegisterSet) -> String {
    let mut names = Vec::new();
    if registers.a {
        names.push("a");
    }
    if registers.x {
        names.push("x");
    }
    if registers.y {
        names.push("y");
    }
    if registers.flags {
        names.push("flags");
    }
    if registers.sp {
        names.push("sp");
    }
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join("|")
    }
}

fn terminator_summary(terminator: &MirTerminator) -> String {
    match terminator {
        MirTerminator::Jump(edge) => format!("jump {}", edge_summary(edge)),
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => format!(
            "branch {} ? {} : {}",
            cond_summary(cond),
            edge_summary(then_edge),
            edge_summary(else_edge)
        ),
        MirTerminator::Return => "return".to_string(),
        MirTerminator::Exit => "exit".to_string(),
        MirTerminator::Unreachable => "unreachable".to_string(),
    }
}

fn edge_summary(edge: &MirEdge) -> String {
    if edge.args.is_empty() {
        return format!("b{}", edge.target.0);
    }
    format!(
        "b{}({})",
        edge.target.0,
        edge.args
            .iter()
            .map(|arg| format!("{}:{}", value_summary(&arg.value), width_name(arg.width)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn cond_summary(cond: &MirCond) -> String {
    match cond {
        MirCond::Deferred => "<deferred>".to_string(),
        MirCond::BoolValue(value) => format!("bool {}", value_summary(value)),
        MirCond::FlagTest(test) => format!("flag {}", flag_test_summary(test)),
        MirCond::AnyFlagTest(tests) => format!(
            "flag {}|{}",
            flag_test_summary(&tests[0]),
            flag_test_summary(&tests[1])
        ),
        MirCond::FusedCompare {
            producer,
            flag_test,
        } => format!(
            "fused b{}:{} {}",
            producer.block.0,
            producer.op_index,
            flag_test_summary(flag_test)
        ),
    }
}

fn flag_test_summary(test: &MirFlagTest) -> &'static str {
    match test {
        MirFlagTest::ZSet => "z_set",
        MirFlagTest::ZClear => "z_clear",
        MirFlagTest::CSet => "c_set",
        MirFlagTest::CClear => "c_clear",
        MirFlagTest::NSet => "n_set",
        MirFlagTest::NClear => "n_clear",
        MirFlagTest::VSet => "v_set",
        MirFlagTest::VClear => "v_clear",
    }
}
