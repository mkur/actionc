use std::collections::{BTreeMap, BTreeSet};

use super::analysis::cfg::NirCfg;
use super::analysis::dominance::NirDominance;
use super::analysis::use_def::{NirDefSite, NirUseDef};
use super::facts::{
    NirType, NirTypeKind, NirValue, SymbolId, TempId, value_is_oversized_literal, value_width,
};
use super::ir::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirDiagnostic {
    pub routine: Option<String>,
    pub block: Option<String>,
    pub message: String,
}

impl NirDiagnostic {
    fn program(message: impl Into<String>) -> Self {
        Self {
            routine: None,
            block: None,
            message: message.into(),
        }
    }

    fn routine(routine: &str, message: impl Into<String>) -> Self {
        Self {
            routine: Some(routine.to_string()),
            block: None,
            message: message.into(),
        }
    }

    fn block(routine: &str, block: &str, message: impl Into<String>) -> Self {
        Self {
            routine: Some(routine.to_string()),
            block: Some(block.to_string()),
            message: message.into(),
        }
    }
}

#[derive(Default)]
struct NirVerifier {
    diagnostics: Vec<NirDiagnostic>,
    static_ids: BTreeSet<SymbolId>,
}

struct NirTempFacts<'a> {
    temps: BTreeMap<TempId, &'a NirTemp>,
    dominance: NirDominance,
    use_def: NirUseDef,
}

impl NirVerifier {
    fn program(&mut self, program: &NirProgram) {
        let mut globals = BTreeSet::new();
        let mut global_ids = BTreeSet::new();
        for global in &program.globals {
            if !global_ids.insert(global.id) {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "duplicate global id `{}`",
                    global.id.0
                )));
            }
            if global.name.is_empty() {
                self.diagnostics
                    .push(NirDiagnostic::program("global name must not be empty"));
            } else if !globals.insert(global.name.as_str()) {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "duplicate global `{}`",
                    global.name
                )));
            }
            if matches!(global.backing, super::ir::NirGlobalBacking::Absolute(_))
                && global.ty.is_none()
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "absolute-backed global `{}` is missing type facts",
                    global.name
                )));
            }
            if matches!(global.backing, super::ir::NirGlobalBacking::Absolute(_))
                && global.storage_size == 0
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "absolute-backed global `{}` has zero storage size",
                    global.name
                )));
            }
            if let Some(init) = &global.init {
                self.global_init(global, init);
            }
        }

        let mut statics = BTreeSet::new();
        for static_data in &program.statics {
            if static_data.name.is_empty() {
                self.diagnostics
                    .push(NirDiagnostic::program("static data name must not be empty"));
            } else if !statics.insert(static_data.name.as_str()) {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "duplicate static data `{}`",
                    static_data.name
                )));
            }
            if !self.static_ids.insert(static_data.id) {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "duplicate static data id `{}`",
                    static_data.id.0
                )));
            }
            if static_data.alignment == 0 || !static_data.alignment.is_power_of_two() {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "static data `{}` alignment must be a nonzero power of two",
                    static_data.name
                )));
            }
            if static_data.section.is_empty() {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "static data `{}` section must not be empty",
                    static_data.name
                )));
            }
            if !static_data.mutable
                && static_data.section != "rodata"
                && static_data.display.as_bytes() != static_data.bytes
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "static data `{}` display does not match authoritative bytes",
                    static_data.name
                )));
            }
            self.type_shape_static(&static_data.ty, &static_data.name);
        }

        let mut routines = BTreeSet::new();
        for routine in &program.routines {
            if routine.name.is_empty() {
                self.diagnostics
                    .push(NirDiagnostic::routine("", "routine name must not be empty"));
            } else if !routines.insert(routine.name.as_str()) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate routine `{}`", routine.name),
                ));
            }
            self.routine(routine);
        }
    }

    fn routine(&mut self, routine: &NirRoutine) {
        let mut params = BTreeSet::new();
        let mut param_ids = BTreeSet::new();
        for param in &routine.params {
            if !param_ids.insert(param.id) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate param id `{}`", param.id.0),
                ));
            }
            if param.name.is_empty() {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    "param name must not be empty",
                ));
            } else if !params.insert(param.name.as_str()) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate param `{}`", param.name),
                ));
            }
            self.type_shape_static(&param.ty, &format!("param `{}`", param.name));
        }

        let mut locals = BTreeSet::new();
        let mut local_ids = BTreeSet::new();
        for local in &routine.locals {
            if !local_ids.insert(local.id) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate local id `{}`", local.id.0),
                ));
            }
            if local.name.is_empty() {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    "local name must not be empty",
                ));
            } else if !locals.insert(local.name.as_str()) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate local `{}`", local.name),
                ));
            }
            if local.kind.is_empty() {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("local `{}` kind must not be empty", local.name),
                ));
            }
            if let Some(init) = &local.init {
                self.storage_init(&routine.name, &local.name, init);
            }
            self.type_shape_static(&local.ty, &format!("local `{}`", local.name));
        }
        for note in &routine.notes {
            if note.text.is_empty() {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    "routine note must not be empty",
                ));
            }
        }

        if routine.blocks.is_empty() {
            self.diagnostics.push(NirDiagnostic::routine(
                &routine.name,
                "routine has no blocks",
            ));
            return;
        }

        let cfg = NirCfg::from_routine(routine);
        let mut block_ids = BTreeSet::new();
        let mut block_labels = BTreeSet::new();
        for block in &routine.blocks {
            if !block_ids.insert(block.id) {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("duplicate block id `{}`", block.id.0),
                ));
            }
            if block.label.is_empty() {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    "block label must not be empty",
                ));
            } else if !block_labels.insert(block.label.as_str()) {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("duplicate block label `{}`", block.label),
                ));
            }
        }

        let mut temp_ids = BTreeSet::new();
        let mut temp_map = BTreeMap::new();
        for temp in &routine.temps {
            if !temp_ids.insert(temp.id) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate temp table entry `%t{}`", temp.id.0),
                ));
            }
            self.type_shape_static(&temp.ty, &format!("temp `%t{}`", temp.id.0));
            if !cfg.block_ids().contains(&temp.def.block) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "temp `%t{}` references missing defining block id `{}`",
                        temp.id.0, temp.def.block.0
                    ),
                ));
            }
            temp_map.entry(temp.id).or_insert(temp);
        }

        let temp_facts = NirTempFacts {
            temps: temp_map,
            dominance: NirDominance::from_cfg(&cfg),
            use_def: NirUseDef::from_routine(routine),
        };

        for block in &routine.blocks {
            let mut defined_temps = BTreeSet::new();
            for (op_index, op) in block.ops.iter().enumerate() {
                self.op(
                    routine,
                    block,
                    op,
                    op_index,
                    &mut defined_temps,
                    &temp_facts,
                );
            }
            match &block.terminator {
                NirTerminator::Open => self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "block has no terminator",
                )),
                NirTerminator::Goto(label) => {
                    self.require_target(routine, block, &cfg, label);
                }
                NirTerminator::Branch {
                    condition,
                    then_label,
                    else_label,
                } => {
                    self.value_type(routine, block, condition, "branch condition");
                    self.branch_condition_type(routine, block, condition);
                    self.value_temp_use(
                        routine,
                        block,
                        condition,
                        block.ops.len(),
                        &temp_facts,
                        "branch condition",
                    );
                    self.require_target(routine, block, &cfg, then_label);
                    self.require_target(routine, block, &cfg, else_label);
                }
                NirTerminator::Return(Some(value)) => {
                    self.value_type(routine, block, value, "return value");
                    self.value_temp_use(
                        routine,
                        block,
                        value,
                        block.ops.len(),
                        &temp_facts,
                        "return value",
                    );
                }
                NirTerminator::Unknown(note) => self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("unknown terminator must be resolved before NIR verification: {note}"),
                )),
                NirTerminator::Fallthrough | NirTerminator::Return(None) | NirTerminator::Exit => {}
            }
        }
    }

    fn global_init(&mut self, global: &NirGlobal, init: &NirGlobalInit) {
        match init {
            NirGlobalInit::Bytes {
                bytes,
                zero_fill,
                section,
                ..
            } => {
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` init section must not be empty",
                        global.name
                    )));
                }
                if (bytes.len() as u16).saturating_add(*zero_fill) < global.storage_size {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` init payload is smaller than storage size",
                        global.name
                    )));
                }
            }
            NirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` descriptor init has unsupported size {}",
                        global.name, descriptor_size
                    )));
                }
                if backing.owner != global.id {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` descriptor backing owner does not match global id",
                        global.name
                    )));
                }
                if backing.bytes.is_empty() && backing.zero_fill == 0 {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` descriptor backing is empty",
                        global.name
                    )));
                }
                if backing.section.is_empty() || section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` descriptor sections must not be empty",
                        global.name
                    )));
                }
            }
            NirGlobalInit::ZeroFill { bytes, section, .. } => {
                if *bytes < global.storage_size {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` zero-fill is smaller than storage size",
                        global.name
                    )));
                }
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` zero-fill section must not be empty",
                        global.name
                    )));
                }
            }
            NirGlobalInit::ProgramEndWord { section, .. } => {
                if global.storage_size < 2 {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` program-end word init needs at least 2 bytes of storage",
                        global.name
                    )));
                }
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` program-end word section must not be empty",
                        global.name
                    )));
                }
            }
            NirGlobalInit::RoutineAddress {
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` routine-address init has unsupported size {}",
                        global.name, descriptor_size
                    )));
                }
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` routine-address section must not be empty",
                        global.name
                    )));
                }
            }
        }
    }

    fn storage_init(&mut self, routine: &str, name: &str, init: &NirStorageInit) {
        match init {
            NirStorageInit::Bytes { section, .. } | NirStorageInit::ZeroFill { section, .. } => {
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` init section must not be empty"),
                    ));
                }
            }
            NirStorageInit::Descriptor {
                backing,
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!(
                            "local `{name}` descriptor init has unsupported size {descriptor_size}"
                        ),
                    ));
                }
                if backing.bytes.is_empty() && backing.zero_fill == 0 {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` descriptor backing is empty"),
                    ));
                }
                if backing.section.is_empty() || section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` descriptor sections must not be empty"),
                    ));
                }
            }
        }
    }

    fn op(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        op: &NirOp,
        op_index: usize,
        defined_temps: &mut BTreeSet<TempId>,
        temp_facts: &NirTempFacts<'_>,
    ) {
        match op {
            NirOp::Set { address, value } if !is_runtime_helper_set(address, value) => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "legacy SET op must be lowered to an absolute Store",
                ));
            }
            NirOp::Set { .. } => {}
            NirOp::Assign { .. } => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "legacy Assign op must be lowered to Store",
                ));
            }
            NirOp::CompoundAssign { .. } => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "legacy CompoundAssign op must be lowered to Load/Binary/Store",
                ));
            }
            NirOp::Load { dest, ty, place } => {
                self.op_type(routine, block, ty, "load result");
                self.place_type(routine, block, place, "load place");
                self.reject_executable_symbol_place(routine, block, place, "load place");
                self.place_temp_uses(routine, block, place, op_index, temp_facts, "load place");
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
            }
            NirOp::AddrOf { dest, ty, place } => {
                self.op_type(routine, block, ty, "address result");
                self.place_type(routine, block, place, "address place");
                self.reject_executable_symbol_place(routine, block, place, "address place");
                self.place_temp_uses(routine, block, place, op_index, temp_facts, "address place");
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
            }
            NirOp::Store { place, src, ty } => {
                self.op_type(routine, block, ty, "store type");
                self.place_type(routine, block, place, "store place");
                self.reject_executable_symbol_place(routine, block, place, "store place");
                self.place_temp_uses(routine, block, place, op_index, temp_facts, "store place");
                self.value_type(routine, block, src, "store source");
                self.value_temp_use(routine, block, src, op_index, temp_facts, "store source");
                self.match_value_widths(routine, block, Some(ty), src, "store");
            }
            NirOp::Unary { dest, ty, src, .. } => {
                self.op_type(routine, block, ty, "unary result");
                self.value_type(routine, block, src, "unary source");
                self.value_temp_use(routine, block, src, op_index, temp_facts, "unary source");
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
            }
            NirOp::Cast {
                dest,
                src,
                from,
                to,
            } => {
                self.op_type(routine, block, from, "cast source type");
                self.op_type(routine, block, to, "cast result");
                self.value_type(routine, block, src, "cast source");
                self.value_temp_use(routine, block, src, op_index, temp_facts, "cast source");
                self.temp_def_matches_table(routine, block, *dest, to, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
            }
            NirOp::Binary {
                dest,
                ty,
                left,
                right,
                ..
            } => {
                self.op_type(routine, block, ty, "binary result");
                self.value_type(routine, block, left, "binary left operand");
                self.value_temp_use(
                    routine,
                    block,
                    left,
                    op_index,
                    temp_facts,
                    "binary left operand",
                );
                self.value_type(routine, block, right, "binary right operand");
                self.value_temp_use(
                    routine,
                    block,
                    right,
                    op_index,
                    temp_facts,
                    "binary right operand",
                );
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
            }
            NirOp::Compare {
                dest,
                ty,
                left,
                right,
                ..
            } => {
                self.op_type(routine, block, ty, "compare result");
                self.value_type(routine, block, left, "compare left operand");
                self.value_temp_use(
                    routine,
                    block,
                    left,
                    op_index,
                    temp_facts,
                    "compare left operand",
                );
                self.value_type(routine, block, right, "compare right operand");
                self.value_temp_use(
                    routine,
                    block,
                    right,
                    op_index,
                    temp_facts,
                    "compare right operand",
                );
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
            }
            NirOp::Call {
                callee,
                args,
                result,
                signature,
                ..
            } => {
                self.callee_type(routine, block, callee, op_index, temp_facts);
                for arg in args {
                    self.value_type(routine, block, arg, "call argument");
                    self.value_temp_use(routine, block, arg, op_index, temp_facts, "call argument");
                }
                if let Some(result) = result {
                    self.op_type(routine, block, &result.ty, "call result");
                    self.temp_def_matches_table(routine, block, result.dest, &result.ty, op_index);
                    if !defined_temps.insert(result.dest) {
                        self.diagnostics.push(NirDiagnostic::block(
                            &routine.name,
                            &block.label,
                            format!("duplicate temp definition `%t{}`", result.dest.0),
                        ));
                    }
                }
                if let Some(signature) = signature {
                    self.call_signature(routine, block, args, result.as_ref(), signature);
                } else if matches!(callee, NirCallee::Indirect { .. }) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "indirect call has no callable signature",
                    ));
                }
            }
            NirOp::Define { .. } | NirOp::Declare { .. } | NirOp::Note { .. } => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "metadata op must not appear in executable NIR block",
                ));
            }
            NirOp::MachineBlock { items, effects } => {
                if items.is_empty() {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "machine block must carry at least one machine item",
                    ));
                }
                self.machine_effects(routine, block, effects);
            }
            NirOp::Unsupported { .. } => {}
        }
    }

    fn place_temp_uses(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        use_index: usize,
        temp_facts: &NirTempFacts<'_>,
        label: &str,
    ) {
        match &place.kind {
            NirPlaceKind::Deref { addr } => {
                self.value_temp_use(routine, block, addr, use_index, temp_facts, label);
            }
            NirPlaceKind::Index {
                base_addr, index, ..
            } => {
                self.value_temp_use(routine, block, base_addr, use_index, temp_facts, label);
                self.value_temp_use(routine, block, index, use_index, temp_facts, label);
            }
            NirPlaceKind::Field { base, .. } => {
                self.place_temp_uses(routine, block, base, use_index, temp_facts, label);
            }
            NirPlaceKind::Symbol(_)
            | NirPlaceKind::Param { .. }
            | NirPlaceKind::Local { .. }
            | NirPlaceKind::Global { .. }
            | NirPlaceKind::Absolute(_)
            | NirPlaceKind::UnresolvedName(_) => {}
        }
    }

    fn callee_type(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        callee: &NirCallee,
        op_index: usize,
        temp_facts: &NirTempFacts<'_>,
    ) {
        match callee {
            NirCallee::Indirect { target, ty } => {
                self.type_shape(routine, block, ty, "indirect callee type");
                self.value_type(routine, block, target, "indirect callee");
                self.value_temp_use(
                    routine,
                    block,
                    target,
                    op_index,
                    temp_facts,
                    "indirect callee",
                );
                if !matches!(ty.kind, NirTypeKind::Callable { .. }) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "indirect callee must have callable type",
                    ));
                }
            }
            NirCallee::User(_) | NirCallee::Builtin(_) | NirCallee::Runtime { .. } => {}
        }
    }

    fn call_signature(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        args: &[NirValue],
        result: Option<&NirCallResult>,
        signature: &NirCallableSignature,
    ) {
        if signature.abi.is_empty() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "call signature ABI must not be empty",
            ));
        }
        if signature.kind.is_empty() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "call signature kind must not be empty",
            ));
        }
        if signature.variadic.is_none() && args.len() > signature.params.len() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "call arity mismatch: signature expects at most {}, got {}",
                    signature.params.len(),
                    args.len()
                ),
            ));
        }
        if signature.variadic.is_some() && args.len() < signature.params.len() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "call arity mismatch: signature expects at least {}, got {}",
                    signature.params.len(),
                    args.len()
                ),
            ));
        }
        if let Some(variadic) = &signature.variadic {
            self.type_shape(routine, block, variadic, "call variadic param");
        }
        for (index, arg) in args.iter().enumerate() {
            let Some(expected) = signature.params.get(index).or(signature.variadic.as_ref()) else {
                continue;
            };
            self.type_shape(routine, block, expected, &format!("call param {index}"));
            self.match_value_widths(routine, block, Some(expected), arg, "call argument");
        }
        match (result, &signature.result) {
            (Some(result), Some(expected)) => {
                self.type_shape(routine, block, expected, "call signature result");
                if &result.ty != expected {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "call result type does not match callable signature",
                    ));
                }
            }
            (None, Some(_)) => self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "call drops result required by callable signature",
            )),
            (Some(_), None) => self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "call materializes result for procedure signature",
            )),
            (None, None) => {}
        }
    }

    fn machine_effects(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        effects: &NirMachineEffects,
    ) {
        self.memory_access(
            routine,
            block,
            &effects.memory.reads,
            "machine read effects",
        );
        self.memory_access(
            routine,
            block,
            &effects.memory.writes,
            "machine write effects",
        );
        if !effects.opaque {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "machine blocks must be opaque scheduling barriers",
            ));
        }
    }

    fn memory_access(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        access: &NirMemoryAccess,
        label: &str,
    ) {
        if let NirMemoryAccess::Known { regions } = access
            && *regions == 0
        {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} cannot use a zero-region Known effect"),
            ));
        }
    }

    fn reject_executable_symbol_place(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        label: &str,
    ) {
        if place_has_symbol_identity(place) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} uses string storage identity"),
            ));
        }
    }

    fn place_type(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        label: &str,
    ) {
        let Some(ty) = place.ty.as_ref() else {
            self.missing_type(routine, block, label);
            return;
        };
        self.type_shape(routine, block, ty, label);
    }

    fn type_shape(&mut self, routine: &NirRoutine, block: &NirBlock, ty: &NirType, label: &str) {
        if ty.kind.width() != ty.width {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} NIR type width mismatch: kind {:?} has {:?}, legacy width is {:?}",
                    ty.kind,
                    ty.kind.width(),
                    ty.width
                ),
            ));
        }
        if ty.kind.is_pointer() != ty.pointer {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} NIR type pointer mismatch: kind {:?} has {}, legacy pointer is {}",
                    ty.kind,
                    ty.kind.is_pointer(),
                    ty.pointer
                ),
            ));
        }
    }

    fn type_shape_static(&mut self, ty: &NirType, label: &str) {
        if ty.kind.width() != ty.width {
            self.diagnostics.push(NirDiagnostic::program(format!(
                "static data `{label}` NIR type width mismatch: kind {:?} has {:?}, legacy width is {:?}",
                ty.kind,
                ty.kind.width(),
                ty.width
            )));
        }
        if ty.kind.is_pointer() != ty.pointer {
            self.diagnostics.push(NirDiagnostic::program(format!(
                "static data `{label}` NIR type pointer mismatch: kind {:?} has {}, legacy pointer is {}",
                ty.kind,
                ty.kind.is_pointer(),
                ty.pointer
            )));
        }
    }

    fn missing_type(&mut self, routine: &NirRoutine, block: &NirBlock, label: &str) {
        self.diagnostics.push(NirDiagnostic::block(
            &routine.name,
            &block.label,
            format!("{label} has no NIR type"),
        ));
    }

    fn value_type(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        value: &NirValue,
        label: &str,
    ) {
        match value {
            NirValue::ConstU8(_) | NirValue::ConstU16(_) => {}
            NirValue::StaticAddr { id, ty, .. } => {
                if !self.static_ids.contains(id) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} references missing static data id `{}`", id.0),
                    ));
                }
                self.type_shape(routine, block, ty, label)
            }
            NirValue::Temp { ty, .. } => self.type_shape(routine, block, ty, label),
            NirValue::Param(_) | NirValue::GlobalAddr(_) => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} has no NIR type"),
                ));
            }
        }
    }

    fn branch_condition_type(&mut self, routine: &NirRoutine, block: &NirBlock, value: &NirValue) {
        let valid = match value {
            NirValue::ConstU8(value) => *value <= 1,
            NirValue::Temp { ty, .. } => matches!(ty.kind, NirTypeKind::Bool),
            NirValue::ConstU16(_)
            | NirValue::StaticAddr { .. }
            | NirValue::Param(_)
            | NirValue::GlobalAddr(_) => false,
        };
        if !valid {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "branch condition must be a Bool/condition value",
            ));
        }
    }

    fn value_temp_use(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        value: &NirValue,
        use_index: usize,
        temp_facts: &NirTempFacts<'_>,
        label: &str,
    ) {
        if let Some(id) = value.temp() {
            self.require_temp_available(routine, block, id, use_index, temp_facts, label);
            if let Some(temp) = temp_facts.temps.get(&id) {
                let value_type = match value {
                    NirValue::Temp { ty, .. } => Some(ty),
                    NirValue::ConstU8(_)
                    | NirValue::ConstU16(_)
                    | NirValue::StaticAddr { .. }
                    | NirValue::Param(_)
                    | NirValue::GlobalAddr(_) => None,
                };
                if let Some(value_type) = value_type
                    && value_type != &temp.ty
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} temp `%t{}` type does not match temp table", id.0),
                    ));
                }
            }
        }
    }

    fn require_temp_available(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        id: TempId,
        use_index: usize,
        temp_facts: &NirTempFacts<'_>,
        label: &str,
    ) {
        let Some(temp) = temp_facts.temps.get(&id) else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} uses undefined temp `%t{}`", id.0),
            ));
            return;
        };

        let use_site_index = (use_index < block.ops.len()).then_some(use_index);
        debug_assert!(
            temp_facts.use_def.has_use_at(id, block.id, use_site_index),
            "shared NIR use-def facts must include every verified temp use"
        );

        let definition = temp_facts
            .use_def
            .unique_definition(id)
            .unwrap_or(NirDefSite {
                block: temp.def.block,
                op_index: temp.def.op_index,
            });
        let available = if definition.block == block.id {
            definition.op_index < use_index
        } else {
            temp_facts.dominance.dominates(definition.block, block.id)
        };

        if !available {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} uses temp `%t{}` before its definition", id.0),
            ));
        }
    }

    fn temp_def_matches_table(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        id: TempId,
        ty: &NirType,
        op_index: usize,
    ) {
        let Some(temp) = routine.temps.iter().find(|temp| temp.id == id) else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("temp definition `%t{}` is missing from temp table", id.0),
            ));
            return;
        };
        if temp.def.block != block.id || temp.def.op_index != op_index {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("temp definition `%t{}` has stale temp table location", id.0),
            ));
        }
        if &temp.ty != ty {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "temp definition `%t{}` type does not match temp table",
                    id.0
                ),
            ));
        }
    }

    fn op_type(&mut self, routine: &NirRoutine, block: &NirBlock, ty: &NirType, label: &str) {
        if ty.summary.is_empty() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} type summary must not be empty"),
            ));
        }
        if matches!(ty.kind, NirTypeKind::Error) || ty.summary.eq_ignore_ascii_case("error") {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} must not have Error type"),
            ));
        }
        self.type_shape(routine, block, ty, label);
    }

    fn match_value_widths(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        target: Option<&NirType>,
        value: &NirValue,
        label: &str,
    ) {
        let Some(target) = target else {
            return;
        };
        let (Some(target_width), Some(value_width)) = (target.width, value_width(value)) else {
            return;
        };
        if target_width != value_width && value_is_oversized_literal(value, target_width) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} width mismatch: target {} is {} byte(s), value is {} byte(s)",
                    target.summary, target_width, value_width
                ),
            ));
        }
    }

    fn require_target(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        cfg: &NirCfg,
        target: &str,
    ) {
        let Some(target_id) = cfg.resolve_label(target) else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("branch target `{target}` does not exist"),
            ));
            return;
        };
        if !cfg.block_ids().contains(&target_id) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "branch target `{target}` resolved to missing block id `{}`",
                    target_id.0
                ),
            ));
        }
    }
}

fn place_has_symbol_identity(place: &NirPlace) -> bool {
    match &place.kind {
        NirPlaceKind::Symbol(_) => true,
        NirPlaceKind::Field { base, .. } => place_has_symbol_identity(base),
        NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_)
        | NirPlaceKind::UnresolvedName(_)
        | NirPlaceKind::Deref { .. }
        | NirPlaceKind::Index { .. } => false,
    }
}

fn is_runtime_helper_set(address: &NirOperand, value: &NirOperand) -> bool {
    let NirOperandKind::Literal {
        value: Some(address),
        ..
    } = address.kind
    else {
        return false;
    };
    if !matches!(address, 0x04E4 | 0x04E6 | 0x04E8 | 0x04EA | 0x04EC | 0x04EE) {
        return false;
    }
    matches!(
        value.kind,
        NirOperandKind::Symbol(_) | NirOperandKind::AddressOfSymbol(_)
    )
}

pub(super) fn verify_program(program: &NirProgram) -> Result<(), Vec<NirDiagnostic>> {
    let mut verifier = NirVerifier::default();
    verifier.program(program);
    if verifier.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(verifier.diagnostics)
    }
}
