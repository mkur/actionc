use std::collections::{BTreeMap, BTreeSet};

use super::analysis::cfg::NirCfg;
use super::analysis::dominance::NirDominance;
use super::analysis::use_def::{NirDefSite, NirUseDef};
use super::facts::{
    NirStorageId, NirType, NirTypeKind, NirValue, RoutineId, RuntimeSymbolId, SignatureId,
    SymbolId, TempId, runtime_symbol_id, value_is_oversized_literal, value_width,
};
use super::ir::*;
use crate::target::{AddressValue, ByteSize, TargetLayout};

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
    static_sizes: BTreeMap<SymbolId, ByteSize>,
    static_types: BTreeMap<SymbolId, NirType>,
    global_sizes: BTreeMap<SymbolId, ByteSize>,
    global_types: BTreeMap<SymbolId, NirType>,
    global_ids: BTreeSet<SymbolId>,
    signatures: BTreeMap<SignatureId, NirCallableSignature>,
    runtime_symbols: BTreeMap<RuntimeSymbolId, String>,
    routine_signatures: BTreeMap<RoutineId, NirCallableSignature>,
    target_layout: TargetLayout,
}

struct NirTempFacts<'a> {
    temps: BTreeMap<TempId, &'a NirTemp>,
    dominance: NirDominance,
    use_def: NirUseDef,
}

impl NirVerifier {
    fn program(&mut self, program: &NirProgram) {
        self.target_layout = program.target_layout;
        if program.target_layout
            != crate::target::TargetLayout::for_target(program.target_layout.target)
        {
            self.diagnostics.push(NirDiagnostic::program(format!(
                "target layout does not match registered target `{}`",
                program.target_layout.target
            )));
        }
        if program.target_layout.address_bits == 0
            || program.target_layout.link_address_bits == 0
            || program.target_layout.link_address_bits > program.target_layout.address_bits
        {
            self.diagnostics.push(NirDiagnostic::program(
                "target layout has invalid architectural or link address width",
            ));
        }
        for (kind, pointer) in [
            ("data", program.target_layout.data_pointer),
            ("code", program.target_layout.code_pointer),
        ] {
            if pointer.size_bytes.is_zero() || pointer.alignment_bytes.is_zero() {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "target layout has invalid {kind}-pointer size or alignment"
                )));
            }
        }
        for routine in &program.routines {
            if self
                .routine_signatures
                .insert(routine.id, routine.signature.clone())
                .is_some()
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "duplicate routine id `{}`",
                    routine.id.0
                )));
            }
        }
        for binding in &program.runtime_bindings {
            if binding.name.is_empty() {
                self.diagnostics
                    .push(NirDiagnostic::program("runtime symbol name must not be empty"));
            }
            if runtime_symbol_id(&binding.name) != binding.symbol {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "runtime symbol `{}` has a mismatched stable id",
                    binding.name
                )));
            }
            if let Some(previous) = self
                .runtime_symbols
                .insert(binding.symbol, binding.name.clone())
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "duplicate or colliding runtime symbol id {} for `{previous}` and `{}`",
                    binding.symbol.0, binding.name
                )));
            }
            match binding.target {
                Some(NirRuntimeTarget::Absolute(address))
                    if address.address_space != self.target_layout.code_pointer.address_space
                        || !self.address_fits_target(address) =>
                {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "runtime binding `{}` is outside the selected target code address space",
                        binding.name
                    )));
                }
                Some(NirRuntimeTarget::Routine(id))
                    if !self.routine_signatures.contains_key(&id) =>
                {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "runtime binding `{}` references missing routine id {id}",
                        binding.name
                    )));
                }
                _ => {}
            }
        }
        self.global_ids = program.globals.iter().map(|global| global.id).collect();
        let mut globals = BTreeSet::new();
        let mut global_ids = BTreeSet::new();
        for global in &program.globals {
            self.global_sizes.insert(global.id, global.storage_size);
            if let Some(ty) = &global.ty {
                self.global_types.insert(global.id, ty.clone());
            }
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
                && global.storage_size.is_zero()
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "absolute-backed global `{}` has zero storage size",
                    global.name
                )));
            }
            if let super::ir::NirGlobalBacking::Absolute(address) = global.backing
                && (address.address_space != self.target_layout.data_pointer.address_space
                    || !self.address_extent_fits_target(address, global.storage_size))
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "absolute-backed global `{}` is outside the selected target address range",
                    global.name
                )));
            }
            if let Some(array) = &global.array
                && let Some(initializer) = array.address_initializer
                && (initializer.address_space != self.target_layout.data_pointer.address_space
                    || !self.address_fits_target(initializer))
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "array `{}` address initializer is outside the selected target data address space",
                    global.name
                )));
            }
            if let Some(array) = &global.array
                && let Some(initializer) = array.address_initializer
                && !array.pointer_backed
            {
                match global.backing {
                    super::ir::NirGlobalBacking::Absolute(address)
                        if address == initializer => {}
                    super::ir::NirGlobalBacking::Absolute(address) => {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "direct fixed array `{}` has backing ${address:04X} but address initializer ${initializer:04X}",
                            global.name
                        )));
                    }
                    _ => {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "direct fixed array `{}` must have absolute backing ${initializer:04X}",
                            global.name
                        )));
                    }
                }
            }
            if let super::ir::NirGlobalBacking::Alias { target, .. } = global.backing {
                if !self.global_ids.contains(&target) {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` aliases missing global id {}",
                        global.name, target.0
                    )));
                }
                if target == global.id {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` cannot alias itself",
                        global.name
                    )));
                }
            }
            if let Some(init) = &global.init {
                self.global_init(global, init);
            }
        }

        let mut statics = BTreeSet::new();
        for static_data in &program.statics {
            self.static_sizes.insert(
                static_data.id,
                ByteSize::try_from(static_data.image.bytes.len()).unwrap_or(ByteSize::new(u32::MAX)),
            );
            self.static_types
                .insert(static_data.id, static_data.ty.clone());
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
            if static_data.alignment.is_zero() || !static_data.alignment.is_power_of_two() {
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
                && static_data.display.as_bytes() != static_data.image.bytes
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "static data `{}` display does not match authoritative bytes",
                    static_data.name
                )));
            }
            self.data_image(
                &format!("static data `{}`", static_data.name),
                &static_data.image,
                None,
            );
            self.type_shape_static(&static_data.ty, &static_data.name);
            if matches!(static_data.ty.kind, NirTypeKind::Real)
                && (static_data.image.bytes.len() != 6
                    || static_data.mutable
                    || static_data.section != "rodata")
            {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "REAL static data `{}` must be an immutable six-byte rodata object",
                    static_data.name
                )));
            }
        }

        let mut routines = BTreeSet::new();
        let program_entry_count = program
            .routines
            .iter()
            .filter(|routine| routine.entry.program)
            .count();
        if program_entry_count > 1 {
            self.diagnostics.push(NirDiagnostic::program(
                "NIR contains more than one program-entry routine",
            ));
        }
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
        let expected_activation = match self.target_layout.routine_activation {
            crate::target::RoutineActivationModel::ClassicStatic => {
                NirActivationModel::ClassicStatic
            }
            crate::target::RoutineActivationModel::NativeReentrant => {
                NirActivationModel::NativeReentrant
            }
        };
        if routine.activation != expected_activation {
            self.diagnostics.push(NirDiagnostic::routine(
                &routine.name,
                "routine activation does not match the selected target ABI",
            ));
        }
        let activation_duration = match routine.activation {
            NirActivationModel::ClassicStatic => NirStorageDuration::RoutineStatic,
            NirActivationModel::NativeReentrant => NirStorageDuration::Automatic,
        };
        if routine.signature.convention != routine.convention {
            self.diagnostics.push(NirDiagnostic::routine(
                &routine.name,
                "routine signature convention does not match its entry convention",
            ));
        }
        self.intern_signature(
            &routine.name,
            None,
            &routine.signature,
            "routine signature",
        );
        if routine.signature.params.len() != routine.params.len() {
            self.diagnostics.push(NirDiagnostic::routine(
                &routine.name,
                "routine parameter table does not match its callable signature",
            ));
        }
        for (param, signature_param) in routine.params.iter().zip(&routine.signature.params) {
            if param.ty != *signature_param {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "parameter `{}` type does not match the routine signature",
                        param.name
                    ),
                ));
            }
        }
        for (index, param) in routine.signature.params.iter().enumerate() {
            self.type_shape_static(param, &format!("routine signature param {index}"));
        }
        if let Some(variadic) = &routine.signature.variadic {
            self.type_shape_static(variadic, "routine signature variadic param");
        }
        if let Some(result) = &routine.signature.result {
            self.type_shape_static(result, "routine signature result");
        }
        match routine.entry.placement {
            NirRoutinePlacement::Absolute(address)
                if address.address_space != self.target_layout.code_pointer.address_space
                    || !self.address_fits_target(address) =>
            {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    "absolute routine entry is outside the selected target code address space",
                ));
            }
            NirRoutinePlacement::Relocatable
            | NirRoutinePlacement::CurrentLocation
            | NirRoutinePlacement::Absolute(_) => {}
        }
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
            if param.duration != activation_duration {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "parameter `{}` duration does not match routine activation",
                        param.name
                    ),
                ));
            }
            self.object_layout(
                &routine.name,
                &format!("parameter `{}`", param.name),
                param.layout,
                param.ty.width,
            );
            self.type_shape_static(&param.ty, &format!("param `{}`", param.name));
        }

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
            }
            if local.kind.is_empty() {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!("local `{}` kind must not be empty", local.name),
                ));
            }
            let expected_duration = match local.backing {
                NirLocalBacking::Ordinary => Some(activation_duration),
                NirLocalBacking::Absolute(_) | NirLocalBacking::GlobalAlias { .. } => {
                    Some(NirStorageDuration::External)
                }
                NirLocalBacking::Alias { .. } => None,
            };
            if expected_duration.is_some_and(|duration| local.duration != duration) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "local `{}` duration does not match its activation and backing",
                        local.name
                    ),
                ));
            }
            self.object_layout(
                &routine.name,
                &format!("local `{}`", local.name),
                local.layout,
                match local.storage {
                    // Array locals carry the element type. Their storage
                    // object may instead be a target-sized descriptor.
                    NirStorageClass::Array => None,
                    NirStorageClass::Scalar | NirStorageClass::Record | NirStorageClass::Type => {
                        local.ty.width
                    }
                },
            );
            if let NirLocalBacking::Absolute(address) = local.backing
                && (address.address_space != self.target_layout.data_pointer.address_space
                    || !self.address_extent_fits_target(address, local.layout.size))
            {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "absolute-backed local `{}` is outside the selected target address range",
                        local.name
                    ),
                ));
            }
            if matches!(local.purpose, NirLocalPurpose::RealTemporary)
                && (!matches!(local.ty.kind, NirTypeKind::Real)
                    || local.ty.width != Some(ByteSize::new(6))
                    || !matches!(local.storage, NirStorageClass::Scalar)
                    || !matches!(local.backing, NirLocalBacking::Ordinary)
                    || local.init.is_some())
            {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "REAL temporary local `{}` must be ordinary, uninitialized, scalar six-byte REAL storage",
                        local.name
                    ),
                ));
            }
            self.type_shape_static(&local.ty, &format!("local `{}`", local.name));
        }
        for local in &routine.locals {
            if let NirLocalBacking::Alias { target, .. } = local.backing {
                match routine
                    .locals
                    .iter()
                    .find(|candidate| candidate.id == target)
                {
                    Some(target) if target.duration != local.duration => {
                        self.diagnostics.push(NirDiagnostic::routine(
                            &routine.name,
                            format!(
                                "local alias `{}` does not inherit target duration",
                                local.name
                            ),
                        ));
                    }
                    Some(_) => {}
                    None => {}
                }
            }
        }
        for local in &routine.locals {
            if let Some(init) = &local.init {
                self.storage_init(&routine.name, local, init, &param_ids, &local_ids);
            }
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
            if matches!(temp.ty.kind, NirTypeKind::Real) {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "temp `%t{}` cannot carry address-based REAL data",
                        temp.id.0
                    ),
                ));
            }
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
        for temp in &routine.temps {
            let definitions = temp_facts.use_def.definitions(temp.id);
            if definitions.len() != 1 {
                self.diagnostics.push(NirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "temp `%t{}` must have exactly one definition, found {}",
                        temp.id.0,
                        definitions.len()
                    ),
                ));
            }
        }

        for block in &routine.blocks {
            let mut defined_temps = BTreeSet::new();
            if !block.params.is_empty() && cfg.predecessors(block.id).is_empty() {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "block parameters require at least one predecessor edge",
                ));
            }
            for (param_index, param) in block.params.iter().enumerate() {
                self.op_type(routine, block, &param.ty, "block parameter");
                self.block_param_def_matches_table(
                    routine,
                    block,
                    param.dest,
                    &param.ty,
                    param_index,
                );
                if !defined_temps.insert(param.dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate block parameter definition `%t{}`", param.dest.0),
                    ));
                }
            }
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
                NirTerminator::Goto(edge) => {
                    self.require_edge(routine, block, edge, &temp_facts);
                }
                NirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
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
                    self.require_edge(routine, block, then_edge, &temp_facts);
                    self.require_edge(routine, block, else_edge, &temp_facts);
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
                NirTerminator::Fallthrough | NirTerminator::Return(None) | NirTerminator::Exit => {}
            }
        }
    }

    fn object_layout(
        &mut self,
        routine: &str,
        owner: &str,
        layout: NirObjectLayout,
        type_width: Option<ByteSize>,
    ) {
        if layout.size.is_zero() {
            self.diagnostics.push(NirDiagnostic::routine(
                routine,
                format!("{owner} layout size must be nonzero"),
            ));
        }
        if layout.alignment.is_zero() || !layout.alignment.is_power_of_two() {
            self.diagnostics.push(NirDiagnostic::routine(
                routine,
                format!("{owner} alignment must be a nonzero power of two"),
            ));
        }
        if type_width.is_some_and(|width| layout.size < width) {
            self.diagnostics.push(NirDiagnostic::routine(
                routine,
                format!("{owner} layout is smaller than its value type"),
            ));
        }
    }

    fn global_init(&mut self, global: &NirGlobal, init: &NirGlobalInit) {
        match init {
            NirGlobalInit::Bytes {
                image,
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
                self.data_image(&format!("global `{}`", global.name), image, None);
                if self.data_extent(&format!("global `{}`", global.name), image, *zero_fill)
                    != Some(usize::from(global.storage_size))
                {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` init payload does not match storage size {}",
                        global.name, global.storage_size
                    )));
                }
            }
            NirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                section,
                ..
            } => {
                if usize::from(*descriptor_size) != usize::from(global.storage_size) {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` descriptor size {} does not match storage size {}",
                        global.name, descriptor_size, global.storage_size
                    )));
                }
                if *descriptor_size
                    != self.target_layout.data_pointer.size_bytes.saturating_add(
                        if size_word.is_some() {
                            ByteSize::new(2)
                        } else {
                            ByteSize::ZERO
                        },
                    )
                {
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
                self.data_image(
                    &format!("global `{}` descriptor backing", global.name),
                    &backing.image,
                    None,
                );
                self.data_extent(
                    &format!("global `{}` descriptor backing", global.name),
                    &backing.image,
                    backing.zero_fill,
                );
                if backing.image.bytes.is_empty() && backing.zero_fill.is_zero() {
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
            NirGlobalInit::LinkValue {
                value: NirLinkValue::ImageEndAddress,
                width,
                section,
                ..
            } => {
                if width.is_zero() || *width > global.storage_size {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` link value width does not fit its storage",
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
                routine,
                descriptor_size,
                section,
                ..
            } => {
                if !self.routine_signatures.contains_key(routine) {
                    self.diagnostics.push(NirDiagnostic::program(format!(
                        "global `{}` routine-address init references missing routine id {}",
                        global.name, routine
                    )));
                }
                let pointer_size = self.target_layout.code_pointer.size_bytes;
                if *descriptor_size != pointer_size
                    && *descriptor_size != pointer_size.saturating_add(ByteSize::new(2))
                {
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

    fn storage_init(
        &mut self,
        routine: &str,
        local: &NirLocal,
        init: &NirStorageInit,
        param_ids: &BTreeSet<super::facts::ParamId>,
        local_ids: &BTreeSet<super::facts::LocalId>,
    ) {
        let name = &local.name;
        match init {
            NirStorageInit::Bytes {
                image,
                zero_fill,
                section,
                ..
            } => {
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` init section must not be empty"),
                    ));
                }
                self.data_image(
                    &format!("local `{name}` in `{routine}`"),
                    image,
                    Some((param_ids, local_ids)),
                );
                let extent = self.data_extent(
                    &format!("local `{name}` in `{routine}`"),
                    image,
                    *zero_fill,
                );
                if extent.is_some_and(|extent| extent > usize::from(local.layout.size)) {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` initializer exceeds its object layout"),
                    ));
                }
            }
            NirStorageInit::ZeroFill { bytes, section, .. } => {
                if section.is_empty() {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` init section must not be empty"),
                    ));
                }
                if *bytes > local.layout.size {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` zero-fill exceeds its object layout"),
                    ));
                }
            }
            NirStorageInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                section,
                ..
            } => {
                if *descriptor_size
                    != self.target_layout.data_pointer.size_bytes.saturating_add(
                        if size_word.is_some() {
                            ByteSize::new(2)
                        } else {
                            ByteSize::ZERO
                        },
                    )
                {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!(
                            "local `{name}` descriptor init has unsupported size {descriptor_size}"
                        ),
                    ));
                }
                if local.layout.size != *descriptor_size
                    || local.layout.alignment != self.target_layout.data_pointer.alignment_bytes
                {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!(
                            "local `{name}` descriptor layout does not match the target pointer layout"
                        ),
                    ));
                }
                self.object_layout(
                    routine,
                    &format!("local `{name}` descriptor backing"),
                    backing.layout,
                    None,
                );
                self.data_image(
                    &format!("local `{name}` descriptor backing in `{routine}`"),
                    &backing.image,
                    Some((param_ids, local_ids)),
                );
                let extent = self.data_extent(
                    &format!("local `{name}` descriptor backing in `{routine}`"),
                    &backing.image,
                    backing.zero_fill,
                );
                if extent != Some(usize::from(backing.layout.size)) {
                    self.diagnostics.push(NirDiagnostic::routine(
                        routine,
                        format!("local `{name}` descriptor backing layout has the wrong size"),
                    ));
                }
                if backing.image.bytes.is_empty() && backing.zero_fill.is_zero() {
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

    fn data_image(
        &mut self,
        owner: &str,
        image: &NirDataImage,
        routine_storage: Option<(
            &BTreeSet<super::facts::ParamId>,
            &BTreeSet<super::facts::LocalId>,
        )>,
    ) {
        let mut occupied = vec![false; image.bytes.len()];
        for fragment in &image.fragments {
            let (offset, width) = match fragment {
                NirDataFragment::Integer { offset, width, .. } => (*offset, *width),
                NirDataFragment::Address {
                    offset, encoding, ..
                } => (*offset, encoding.width()),
            };
            let start = usize::from(offset);
            let end = start.saturating_add(usize::from(width));
            if end > image.bytes.len() {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "{owner} data fragment at {offset} with width {width} exceeds {} initialized bytes",
                    image.bytes.len()
                )));
                continue;
            }
            if occupied[start..end].iter().any(|occupied| *occupied) {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "{owner} has overlapping data fragment at {offset}"
                )));
            }
            if image.bytes[start..end].iter().any(|byte| *byte != 0) {
                self.diagnostics.push(NirDiagnostic::program(format!(
                    "{owner} data fragment at {offset} placeholder bytes must be zero"
                )));
            }
            occupied[start..end].fill(true);
            let NirDataFragment::Address {
                encoding,
                target,
                addend,
                ..
            } = fragment
            else {
                if let NirDataFragment::Integer { width, value, .. } = fragment {
                    let bits = width.get().saturating_mul(8);
                    if width.is_zero()
                        || width.get() > 8
                        || (bits < 64 && *value >= (1u64 << bits))
                    {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} integer data fragment value does not fit width {width}"
                        )));
                    }
                }
                continue;
            };

            let expected_space = match target {
                NirDataAddressTarget::Routine(_) => self.target_layout.code_pointer.address_space,
                NirDataAddressTarget::Storage(_) => self.target_layout.data_pointer.address_space,
                NirDataAddressTarget::Absolute(address) => address.address_space,
            };
            match encoding {
                NirDataAddressEncoding::Pointer {
                    address_space,
                    width,
                } => {
                    let expected_width = if *address_space
                        == self.target_layout.data_pointer.address_space
                    {
                        Some(self.target_layout.data_pointer.size_bytes)
                    } else if *address_space == self.target_layout.code_pointer.address_space {
                        Some(self.target_layout.code_pointer.size_bytes)
                    } else {
                        None
                    };
                    if *address_space != expected_space || expected_width != Some(*width) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} address fragment does not match its target address space or pointer width"
                        )));
                    }
                }
                NirDataAddressEncoding::TargetByte {
                    target,
                    byte_index,
                } => {
                    if *target != self.target_layout.target {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} contains an address-byte selector for target `{}` under `{}`",
                            target, self.target_layout.target
                        )));
                    }
                    if u16::from(*byte_index) * 8 >= u16::from(self.target_layout.address_bits) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} address-byte selector is outside the target address width"
                        )));
                    }
                }
            }

            match target {
                NirDataAddressTarget::Storage(NirStorageId::Global(id)) => {
                    if !self.global_ids.contains(&id) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} address fragment references unknown global id {}",
                            id.0
                        )));
                    }
                }
                NirDataAddressTarget::Storage(NirStorageId::Param(id)) => {
                    if !routine_storage.is_some_and(|(params, _)| params.contains(&id)) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} address fragment references a parameter outside its owning routine"
                        )));
                    }
                }
                NirDataAddressTarget::Storage(NirStorageId::Local(id)) => {
                    if !routine_storage.is_some_and(|(_, locals)| locals.contains(&id)) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} address fragment references a local outside its owning routine"
                        )));
                    }
                }
                NirDataAddressTarget::Routine(id) => {
                    if !self.routine_signatures.contains_key(id) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} address fragment references unknown routine id {id}"
                        )));
                    }
                }
                NirDataAddressTarget::Absolute(address) => {
                    let value = address.checked_add_signed(*addend);
                    if value.is_none_or(|value| !self.address_fits_target(value)) {
                        self.diagnostics.push(NirDiagnostic::program(format!(
                            "{owner} absolute address fragment result is outside the selected target address range"
                        )));
                    }
                }
            }
        }
    }

    fn data_extent(
        &mut self,
        owner: &str,
        image: &NirDataImage,
        zero_fill: ByteSize,
    ) -> Option<usize> {
        let extent = image.bytes.len().checked_add(usize::from(zero_fill));
        if extent.is_none_or(|extent| ByteSize::try_from(extent).is_err()) {
            self.diagnostics.push(NirDiagnostic::program(format!(
                "{owner} initialized extent exceeds the NIR storage range"
            )));
            return None;
        }
        extent
    }

    fn address_fits_target(&self, address: AddressValue) -> bool {
        let known_space = address.address_space == self.target_layout.data_pointer.address_space
            || address.address_space == self.target_layout.code_pointer.address_space;
        let max = match self.target_layout.address_bits {
            64 => u64::MAX,
            bits => (1u64 << bits) - 1,
        };
        known_space && address.value <= max
    }

    fn address_extent_fits_target(&self, address: AddressValue, size: ByteSize) -> bool {
        if !self.address_fits_target(address) {
            return false;
        }
        if size.is_zero() {
            return true;
        }
        address
            .checked_add_signed(i64::from(size.get() - 1))
            .is_some_and(|end| self.address_fits_target(end))
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
            NirOp::Load { dest, ty, place } | NirOp::VolatileLoad { dest, ty, place } => {
                self.op_type(routine, block, ty, "load result");
                self.place_type(routine, block, place, "load place");
                self.reject_real_type(routine, block, ty, "ordinary load result");
                self.reject_real_place(routine, block, place, "ordinary load place");
                self.reject_record_type(routine, block, ty, "ordinary load result");
                self.reject_record_place(routine, block, place, "ordinary load place");
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
                if !ty.kind.is_pointer() {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "address-of result must have data-pointer type",
                    ));
                }
                self.place_type(routine, block, place, "address place");
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
            NirOp::Store { place, src, ty } | NirOp::VolatileStore { place, src, ty } => {
                self.op_type(routine, block, ty, "store type");
                self.place_type(routine, block, place, "store place");
                self.reject_real_type(routine, block, ty, "ordinary store type");
                self.reject_real_place(routine, block, place, "ordinary store place");
                self.reject_record_type(routine, block, ty, "ordinary store type");
                self.reject_record_place(routine, block, place, "ordinary store place");
                self.place_temp_uses(routine, block, place, op_index, temp_facts, "store place");
                self.value_type(routine, block, src, "store source");
                self.value_temp_use(routine, block, src, op_index, temp_facts, "store source");
                self.match_value_widths(routine, block, Some(ty), src, "store");
                if self.target_layout.target != crate::target::TargetId::Atari6502
                    && (ty.kind.is_address() || value_has_address_type(src))
                    && !value_matches_type(src, ty)
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "native pointer stores require an explicit, type-preserving address value",
                    ));
                }
            }
            NirOp::CopyBytes {
                destination,
                source,
                size,
                ..
            } => {
                self.place_type(routine, block, destination, "copy destination");
                self.place_type(routine, block, source, "copy source");
                self.place_temp_uses(
                    routine,
                    block,
                    destination,
                    op_index,
                    temp_facts,
                    "copy destination",
                );
                self.place_temp_uses(routine, block, source, op_index, temp_facts, "copy source");
                if size.is_zero() {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "copy_bytes extent must be non-zero",
                    ));
                }
                self.record_copy_place(routine, block, destination, *size, "copy destination");
                self.record_copy_place(routine, block, source, *size, "copy source");
            }
            NirOp::Unary { dest, ty, op, src } => {
                self.op_type(routine, block, ty, "unary result");
                self.value_type(routine, block, src, "unary source");
                self.reject_real_type(routine, block, ty, "ordinary unary result");
                self.reject_real_value(routine, block, src, "ordinary unary source");
                self.value_temp_use(routine, block, src, op_index, temp_facts, "unary source");
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
                if *op == NirUnaryOp::Neg && ty.kind != NirTypeKind::I16 {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "integer negation must produce cartridge-compatible INT",
                    ));
                }
                if ty.kind.is_address() || value_has_address_type(src) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "generic integer unary operation cannot carry a pointer value",
                    ));
                }
            }
            NirOp::Cast {
                dest,
                src,
                from,
                to,
                kind,
            } => {
                self.op_type(routine, block, from, "cast source type");
                self.op_type(routine, block, to, "cast result");
                self.reject_real_type(routine, block, from, "ordinary cast source type");
                self.reject_real_type(routine, block, to, "ordinary cast result type");
                self.value_type(routine, block, src, "cast source");
                self.reject_real_value(routine, block, src, "ordinary cast source");
                self.value_temp_use(routine, block, src, op_index, temp_facts, "cast source");
                self.temp_def_matches_table(routine, block, *dest, to, op_index);
                if !defined_temps.insert(*dest) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", dest.0),
                    ));
                }
                let expected_kind = match (from.kind.is_address(), to.kind.is_address()) {
                    (true, true) => NirCastKind::Pointer,
                    (false, true) => NirCastKind::IntegerToPointer,
                    (true, false) => NirCastKind::PointerToInteger,
                    (false, false) => NirCastKind::Integer,
                };
                if *kind != expected_kind {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "cast kind does not match its source and result types",
                    ));
                }
                if self.target_layout.target != crate::target::TargetId::Atari6502
                    && matches!(
                        kind,
                        NirCastKind::IntegerToPointer | NirCastKind::PointerToInteger
                    )
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "dynamic pointer/integer conversion requires a target-sized ADDRESS type",
                    ));
                }
            }
            NirOp::PointerOffset {
                dest,
                ty,
                base,
                offset,
                ..
            } => {
                self.op_type(routine, block, ty, "pointer offset result");
                if !ty.kind.is_pointer() {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "pointer offset result must have data-pointer type",
                    ));
                }
                self.value_type(routine, block, base, "pointer offset base");
                self.value_type(routine, block, offset, "pointer offset displacement");
                self.value_temp_use(
                    routine,
                    block,
                    base,
                    op_index,
                    temp_facts,
                    "pointer offset base",
                );
                self.value_temp_use(
                    routine,
                    block,
                    offset,
                    op_index,
                    temp_facts,
                    "pointer offset displacement",
                );
                if !value_matches_type(base, ty) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "pointer offset base type does not match its result type",
                    ));
                }
                if value_has_address_type(offset)
                    || value_width(offset).is_none_or(|width| width > ByteSize::new(2))
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "pointer offset displacement must be an Action! integer",
                    ));
                }
                self.temp_def_matches_table(routine, block, *dest, ty, op_index);
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
                op,
                left,
                right,
            } => {
                self.op_type(routine, block, ty, "binary result");
                self.reject_real_type(routine, block, ty, "ordinary binary result");
                self.value_type(routine, block, left, "binary left operand");
                self.reject_real_value(routine, block, left, "ordinary binary left operand");
                self.value_temp_use(
                    routine,
                    block,
                    left,
                    op_index,
                    temp_facts,
                    "binary left operand",
                );
                self.value_type(routine, block, right, "binary right operand");
                self.reject_real_value(routine, block, right, "ordinary binary right operand");
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
                if *op == NirBinaryOp::Mul && ty.kind != NirTypeKind::I16 {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "integer multiplication must produce cartridge-compatible INT",
                    ));
                }
                if self.target_layout.target != crate::target::TargetId::Atari6502
                    && (ty.kind.is_address()
                        || value_has_address_type(left)
                        || value_has_address_type(right))
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "generic integer binary operation cannot carry pointer values",
                    ));
                }
                if matches!(op, NirBinaryOp::Add | NirBinaryOp::Sub)
                    && ty.kind == NirTypeKind::U8
                    && constant_binary_value(*op, left, right)
                        .is_some_and(|value| value > u16::from(u8::MAX))
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "overflowing constant BYTE addition or subtraction must produce INT",
                    ));
                }
            }
            NirOp::Compare {
                dest,
                ty,
                operand_ty,
                left,
                right,
                ..
            } => {
                self.op_type(routine, block, ty, "compare result");
                self.op_type(routine, block, operand_ty, "compare operand");
                self.reject_real_type(routine, block, operand_ty, "ordinary compare operand");
                self.value_type(routine, block, left, "compare left operand");
                self.reject_real_value(routine, block, left, "ordinary compare left operand");
                self.value_temp_use(
                    routine,
                    block,
                    left,
                    op_index,
                    temp_facts,
                    "compare left operand",
                );
                self.value_type(routine, block, right, "compare right operand");
                self.reject_real_value(routine, block, right, "ordinary compare right operand");
                self.value_temp_use(
                    routine,
                    block,
                    right,
                    op_index,
                    temp_facts,
                    "compare right operand",
                );
                self.match_operand_widths(routine, block, left, right, "compare operands");
                self.match_operand_type_width(
                    routine,
                    block,
                    operand_ty,
                    left,
                    "compare left operand",
                );
                self.match_operand_type_width(
                    routine,
                    block,
                    operand_ty,
                    right,
                    "compare right operand",
                );
                self.match_operand_machine_type(
                    routine,
                    block,
                    operand_ty,
                    left,
                    "compare left operand",
                );
                self.match_operand_machine_type(
                    routine,
                    block,
                    operand_ty,
                    right,
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
            NirOp::Real(real) => {
                self.real_op(routine, block, real, op_index, defined_temps, temp_facts);
            }
            NirOp::Call {
                callee,
                args,
                result,
                signature,
                effects,
            } => {
                self.callee_type(routine, block, callee, op_index, temp_facts);
                for arg in args {
                    self.value_type(routine, block, arg, "call argument");
                    self.reject_real_value(routine, block, arg, "call argument");
                    self.value_temp_use(routine, block, arg, op_index, temp_facts, "call argument");
                }
                if let Some(result) = result {
                    self.op_type(routine, block, &result.ty, "call result");
                    self.reject_real_type(routine, block, &result.ty, "call result");
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
                    self.call_signature(routine, block, callee, args, result.as_ref(), signature);
                    if let NirCallee::Indirect { ty, .. } = callee
                        && let NirTypeKind::Callable {
                            signature: callee_signature,
                            convention: callee_convention,
                            ..
                        } = ty.kind
                        && (callee_signature != signature.id
                            || callee_convention != signature.convention)
                    {
                        self.diagnostics.push(NirDiagnostic::block(
                            &routine.name,
                            &block.label,
                            "indirect callee signature or convention does not match the call signature",
                        ));
                    }
                } else {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "call has no callable signature or convention",
                    ));
                }
                self.call_effects(routine, block, effects);
            }
            NirOp::ForeignCode { code, effects } => {
                if code.target != self.target_layout.target {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!(
                            "{} payload code for target `{}` cannot be used with selected target `{}` at source span {}..{}",
                            match code.kind {
                                NirForeignCodeKind::LegacyMachineBlock => "machine block",
                                NirForeignCodeKind::InlineAssembly => "inline assembly",
                            },
                            code.target,
                            self.target_layout.target,
                            code.span.start,
                            code.span.end,
                        ),
                    ));
                }
                match &code.payload {
                    NirForeignCodePayload::Structured(items) => {
                        if items.is_empty() {
                            self.diagnostics.push(NirDiagnostic::block(
                                &routine.name,
                                &block.label,
                                "machine block must carry at least one machine item",
                            ));
                        }
                        for item in items {
                            if let NirMachineItem::Relocation {
                                encoding,
                                target,
                                addend,
                                required_address_bits,
                                ..
                            } = item
                            {
                                self.foreign_relocation_shape(
                                    routine,
                                    block,
                                    code,
                                    *encoding,
                                    *required_address_bits,
                                );
                                if !(-65535..=65535).contains(addend) {
                                    self.diagnostics.push(NirDiagnostic::block(
                                        &routine.name,
                                        &block.label,
                                        format!(
                                            "machine relocation addend {addend} is outside the supported 16-bit address range"
                                        ),
                                    ));
                                }
                                self.resolved_symbol_target(
                                    routine,
                                    block,
                                    *target,
                                    "machine block",
                                );
                            }
                        }
                        self.machine_effects(routine, block, effects);
                    }
                    NirForeignCodePayload::Bytes { .. } => {
                        self.foreign_bytes(routine, block, code);
                        self.memory_access(
                            routine,
                            block,
                            &effects.memory.reads,
                            "inline assembler read effects",
                        );
                        self.memory_access(
                            routine,
                            block,
                            &effects.memory.writes,
                            "inline assembler write effects",
                        );
                    }
                }
            }
            NirOp::Unsupported { .. } => {}
        }
    }

    fn real_op(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        op: &NirRealOp,
        op_index: usize,
        defined_temps: &mut BTreeSet<TempId>,
        temp_facts: &NirTempFacts<'_>,
    ) {
        match op {
            NirRealOp::Copy {
                destination,
                source,
            } => {
                self.real_place(
                    routine,
                    block,
                    destination,
                    op_index,
                    temp_facts,
                    "REAL copy destination",
                );
                self.real_source(
                    routine,
                    block,
                    source,
                    op_index,
                    temp_facts,
                    "REAL copy source",
                );
            }
            NirRealOp::Unary {
                operation: _,
                destination,
                operand,
            } => {
                self.real_place(
                    routine,
                    block,
                    destination,
                    op_index,
                    temp_facts,
                    "REAL unary destination",
                );
                self.real_source(
                    routine,
                    block,
                    operand,
                    op_index,
                    temp_facts,
                    "REAL unary operand",
                );
            }
            NirRealOp::Binary {
                operation,
                destination,
                left,
                right,
            } => {
                if !matches!(
                    operation,
                    NirBinaryOp::Add | NirBinaryOp::Sub | NirBinaryOp::Mul | NirBinaryOp::Div
                ) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "REAL binary operation must be Add, Sub, Mul, or Div",
                    ));
                }
                self.real_place(
                    routine,
                    block,
                    destination,
                    op_index,
                    temp_facts,
                    "REAL binary destination",
                );
                self.real_source(
                    routine,
                    block,
                    left,
                    op_index,
                    temp_facts,
                    "REAL binary left operand",
                );
                self.real_source(
                    routine,
                    block,
                    right,
                    op_index,
                    temp_facts,
                    "REAL binary right operand",
                );
            }
            NirRealOp::Compare {
                predicate: _,
                result,
                result_type,
                left,
                right,
            } => {
                self.real_source(
                    routine,
                    block,
                    left,
                    op_index,
                    temp_facts,
                    "REAL compare left operand",
                );
                self.real_source(
                    routine,
                    block,
                    right,
                    op_index,
                    temp_facts,
                    "REAL compare right operand",
                );
                let ty = super::facts::condition_type();
                if result_type != &ty {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "REAL comparison result must have Bool/condition type",
                    ));
                }
                self.temp_def_matches_table(routine, block, *result, result_type, op_index);
                if !defined_temps.insert(*result) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", result.0),
                    ));
                }
            }
            NirRealOp::IntegerToReal {
                destination,
                source,
                source_type,
            } => {
                self.real_place(
                    routine,
                    block,
                    destination,
                    op_index,
                    temp_facts,
                    "REAL conversion destination",
                );
                self.op_type(routine, block, source_type, "REAL conversion source type");
                if !matches!(
                    source_type.kind,
                    NirTypeKind::U8 | NirTypeKind::I8 | NirTypeKind::U16 | NirTypeKind::I16
                ) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "integer-to-REAL conversion source must be an integer",
                    ));
                }
                self.value_type(routine, block, source, "REAL conversion source");
                self.reject_real_value(routine, block, source, "REAL conversion source");
                self.value_temp_use(
                    routine,
                    block,
                    source,
                    op_index,
                    temp_facts,
                    "REAL conversion source",
                );
                self.match_value_widths(
                    routine,
                    block,
                    Some(source_type),
                    source,
                    "REAL conversion source",
                );
            }
            NirRealOp::RealToInteger {
                result,
                result_type,
                source,
            } => {
                self.real_place(
                    routine,
                    block,
                    source,
                    op_index,
                    temp_facts,
                    "REAL conversion source",
                );
                self.op_type(routine, block, result_type, "REAL conversion result type");
                if !matches!(
                    result_type.kind,
                    NirTypeKind::U8 | NirTypeKind::I8 | NirTypeKind::U16 | NirTypeKind::I16
                ) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "REAL-to-integer conversion result must be an integer",
                    ));
                }
                self.temp_def_matches_table(routine, block, *result, result_type, op_index);
                if !defined_temps.insert(*result) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate temp definition `%t{}`", result.0),
                    ));
                }
            }
        }
    }

    fn real_place(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        op_index: usize,
        temp_facts: &NirTempFacts<'_>,
        label: &str,
    ) {
        self.place_type(routine, block, place, label);
        self.place_temp_uses(routine, block, place, op_index, temp_facts, label);
        if !place
            .ty
            .as_ref()
            .is_some_and(|ty| {
                matches!(ty.kind, NirTypeKind::Real) && ty.width == Some(ByteSize::new(6))
            })
        {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} must be a six-byte REAL place"),
            ));
            return;
        }
        match &place.kind {
            NirPlaceKind::Param { .. } => self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} cannot use deferred by-value REAL parameter storage"),
            )),
            NirPlaceKind::Local { id, .. }
                if !routine.locals.iter().any(|local| {
                    local.id == *id
                        && matches!(local.ty.kind, NirTypeKind::Real)
                        && local.ty.width == Some(ByteSize::new(6))
                }) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references missing or non-REAL local id {}", id.0),
                ));
            }
            NirPlaceKind::Global { id, .. }
                if self.global_sizes.get(id) != Some(&ByteSize::new(6))
                    || !self
                        .global_types
                        .get(id)
                        .is_some_and(|ty| matches!(ty.kind, NirTypeKind::Real)) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references missing or non-REAL global id {}", id.0),
                ));
            }
            NirPlaceKind::Index {
                elem_ty, elem_size, ..
            } if !matches!(elem_ty.kind, NirTypeKind::Real)
                || elem_ty.width != Some(ByteSize::new(6))
                || *elem_size != ByteSize::new(6) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} must index six-byte REAL elements"),
                ));
            }
            NirPlaceKind::Field { ty, .. }
                if !matches!(ty.kind, NirTypeKind::Real)
                    || ty.width != Some(ByteSize::new(6)) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} field metadata must describe six-byte REAL storage"),
                ));
            }
            NirPlaceKind::Local { .. }
            | NirPlaceKind::Global { .. }
            | NirPlaceKind::Absolute(_)
            | NirPlaceKind::Deref { .. }
            | NirPlaceKind::Index { .. }
            | NirPlaceKind::Field { .. } => {}
        }
    }

    fn real_source(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        source: &NirRealSource,
        op_index: usize,
        temp_facts: &NirTempFacts<'_>,
        label: &str,
    ) {
        match source {
            NirRealSource::Place(place) => {
                self.real_place(routine, block, place, op_index, temp_facts, label)
            }
            NirRealSource::Static { id, name } => {
                let valid = self.static_sizes.get(id) == Some(&ByteSize::new(6))
                    && self
                        .static_types
                        .get(id)
                        .is_some_and(|ty| matches!(ty.kind, NirTypeKind::Real));
                if !valid {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} `{name}` does not name six-byte REAL static data"),
                    ));
                }
            }
        }
    }

    fn reject_real_type(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        ty: &NirType,
        label: &str,
    ) {
        if matches!(ty.kind, NirTypeKind::Real) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} cannot use REAL in the byte/word scalar lane"),
            ));
        }
    }

    fn reject_real_place(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        label: &str,
    ) {
        if let Some(ty) = &place.ty {
            self.reject_real_type(routine, block, ty, label);
        }
    }

    fn reject_real_value(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        value: &NirValue,
        label: &str,
    ) {
        if let NirValue::StaticAddr { ty, .. } | NirValue::Temp { ty, .. } = value {
            self.reject_real_type(routine, block, ty, label);
        }
    }

    fn reject_record_type(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        ty: &NirType,
        label: &str,
    ) {
        if matches!(ty.kind, NirTypeKind::Record { .. }) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} cannot use a record in the byte/word scalar lane"),
            ));
        }
    }

    fn reject_record_place(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        label: &str,
    ) {
        if let Some(ty) = &place.ty {
            self.reject_record_type(routine, block, ty, label);
        }
    }

    fn record_copy_place(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        place: &NirPlace,
        size: ByteSize,
        label: &str,
    ) {
        let Some(ty) = &place.ty else {
            return;
        };
        if !matches!(ty.kind, NirTypeKind::Record { .. }) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} must have record storage type"),
            ));
            return;
        }
        if ty.width != Some(size) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} width {:?} does not match copy_bytes extent {size}",
                    ty.width
                ),
            ));
        }
    }

    fn foreign_bytes(&mut self, routine: &NirRoutine, block: &NirBlock, code: &NirForeignCode) {
        let NirForeignCodePayload::Bytes { bytes, relocations } = &code.payload else {
            return;
        };
        if bytes.is_empty() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "inline assembler must carry at least one byte",
            ));
            return;
        }
        let mut occupied = BTreeSet::new();
        for relocation in relocations {
            let width = usize::try_from(relocation.encoding.width())
                .expect("foreign relocation width fits usize");
            let start = usize::from(relocation.offset);
            if start.saturating_add(width) > bytes.len() {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!(
                        "inline assembler relocation at {} exceeds {}-byte payload",
                        relocation.offset,
                        bytes.len()
                    ),
                ));
                continue;
            }
            self.foreign_relocation_shape(
                routine,
                block,
                code,
                relocation.encoding,
                relocation.required_address_bits,
            );
            for byte in start..start + width {
                if !occupied.insert(byte) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("overlapping inline assembler relocation at byte {byte}"),
                    ));
                }
            }
            if !(-65535..=65535).contains(&relocation.addend) {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!(
                        "inline assembler relocation addend {} is outside the supported 16-bit address range",
                        relocation.addend
                    ),
                ));
            }
            if let NirForeignCodeTarget::Absolute(address) = relocation.target {
                let value = address.checked_add_signed(i64::from(relocation.addend));
                if value.is_none_or(|value| !self.address_fits_target(value)) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "inline assembler absolute relocation result is outside the selected target address range",
                    ));
                }
            }
            match relocation.target {
                NirForeignCodeTarget::Storage(NirStorageId::Param(id))
                    if !routine.params.iter().any(|param| param.id == id) =>
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("inline assembler references unknown param id {}", id.0),
                    ));
                }
                NirForeignCodeTarget::Storage(NirStorageId::Local(id))
                    if !routine.locals.iter().any(|local| local.id == id) =>
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("inline assembler references unknown local id {}", id.0),
                    ));
                }
                NirForeignCodeTarget::Storage(NirStorageId::Global(id))
                    if !self.global_sizes.contains_key(&id) =>
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("inline assembler references unknown global id {}", id.0),
                    ));
                }
                NirForeignCodeTarget::Routine(id)
                    if !self.routine_signatures.contains_key(&id) =>
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("inline assembler references unknown routine id {id}"),
                    ));
                }
                NirForeignCodeTarget::InlineOffset(offset)
                    if usize::from(offset) > bytes.len() =>
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("inline assembler target offset {offset} is outside its payload"),
                    ));
                }
                _ => {}
            }
        }
    }

    fn foreign_relocation_shape(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        code: &NirForeignCode,
        encoding: crate::foreign::ForeignRelocationEncoding,
        required_address_bits: Option<u8>,
    ) {
        if let crate::foreign::ForeignRelocationEncoding::TargetByte {
            target,
            byte_index,
        } = encoding
        {
            if target != code.target {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "foreign-code byte-selector relocation target does not match its payload target",
                ));
            }
            let address_bytes = self.target_layout.address_bits.div_ceil(8);
            if byte_index >= address_bytes {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "foreign-code byte-selector relocation is outside the target address width",
                ));
            }
        }
        if required_address_bits == Some(0)
            || required_address_bits.is_some_and(|bits| bits > self.target_layout.address_bits)
        {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "foreign-code relocation has an invalid address-width requirement",
            ));
        }
    }

    fn resolved_symbol_target(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        target: NirForeignCodeTarget,
        label: &str,
    ) {
        match target {
            NirForeignCodeTarget::Storage(NirStorageId::Param(id))
                if !routine.params.iter().any(|param| param.id == id) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown param id {}", id.0),
                ));
            }
            NirForeignCodeTarget::Storage(NirStorageId::Local(id))
                if !routine.locals.iter().any(|local| local.id == id) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown local id {}", id.0),
                ));
            }
            NirForeignCodeTarget::Storage(NirStorageId::Global(id))
                if !self.global_sizes.contains_key(&id) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown global id {}", id.0),
                ));
            }
            NirForeignCodeTarget::Routine(id) if !self.routine_signatures.contains_key(&id) => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown routine id {id}"),
                ));
            }
            NirForeignCodeTarget::InlineOffset(_) if label == "machine block" => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "machine block relocation cannot target inline-assembler storage",
                ));
            }
            _ => {}
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
            NirPlaceKind::Param { .. }
            | NirPlaceKind::Local { .. }
            | NirPlaceKind::Global { .. }
            | NirPlaceKind::Absolute(_) => {}
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
            NirCallee::User { id, .. } => {
                if !self.routine_signatures.contains_key(id) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("direct call references missing routine id {id}"),
                    ));
                }
            }
            NirCallee::Runtime { symbol, name } => {
                if self.runtime_symbols.get(symbol) != Some(name) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        "runtime call references a missing or mismatched runtime symbol",
                    ));
                }
            }
            NirCallee::Builtin(_) => {}
        }
    }

    fn intern_signature(
        &mut self,
        routine: &str,
        block: Option<&str>,
        signature: &NirCallableSignature,
        label: &str,
    ) {
        if let Some(existing) = self.signatures.get(&signature.id) {
            if existing != signature {
                let message = format!(
                    "signature id {} is reused for different callable facts",
                    signature.id.0
                );
                self.diagnostics.push(match block {
                    Some(block) => NirDiagnostic::block(routine, block, message),
                    None => NirDiagnostic::routine(routine, message),
                });
            }
        } else {
            self.signatures.insert(signature.id, signature.clone());
        }
        if signature.kind.is_empty() {
            let message = format!("{label} kind must not be empty");
            self.diagnostics.push(match block {
                Some(block) => NirDiagnostic::block(routine, block, message),
                None => NirDiagnostic::routine(routine, message),
            });
        }
    }

    fn call_signature(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        callee: &NirCallee,
        args: &[NirValue],
        result: Option<&NirCallResult>,
        signature: &NirCallableSignature,
    ) {
        self.intern_signature(
            &routine.name,
            Some(&block.label),
            signature,
            "call signature",
        );
        let expected_convention = match callee {
            NirCallee::User { id, .. } => self
                .routine_signatures
                .get(id)
                .map(|callee| callee.convention),
            NirCallee::Indirect { ty, .. } => match ty.kind {
                NirTypeKind::Callable { convention, .. } => Some(convention),
                _ => None,
            },
            NirCallee::Builtin(_) | NirCallee::Runtime { .. } => {
                Some(NirCallConvention::Runtime)
            }
        };
        if expected_convention.is_some_and(|expected| expected != signature.convention) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "call convention does not match its callee",
            ));
        }
        if let NirCallee::User { id, .. } = callee
            && let Some(callee_signature) = self.routine_signatures.get(id)
            && callee_signature != signature
        {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                "direct call signature does not match its callee",
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

    fn call_effects(&mut self, routine: &NirRoutine, block: &NirBlock, effects: &NirCallEffects) {
        self.memory_access(routine, block, &effects.memory.reads, "call read effects");
        self.memory_access(routine, block, &effects.memory.writes, "call write effects");
    }

    fn memory_access(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        access: &NirMemoryAccess,
        label: &str,
    ) {
        let NirMemoryAccess::Regions(regions) = access else {
            return;
        };
        if regions.is_empty() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} cannot use an empty region collection"),
            ));
        }
        for region in regions {
            self.memory_region(routine, block, region, label);
        }
    }

    fn memory_region(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        region: &NirMemoryRegion,
        label: &str,
    ) {
        if region.size.is_zero() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} contains a zero-size region"),
            ));
            return;
        }
        let Some(end) = region
            .offset
            .checked_add_size(region.size.saturating_sub(ByteSize::ONE))
        else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} region exceeds the 16-bit address space"),
            ));
            return;
        };
        let available = match region.kind {
            NirMemoryRegionKind::Storage(NirStorageId::Local(id)) => routine
                .locals
                .iter()
                .find(|local| local.id == id)
                .map(|local| local.ty.width),
            NirMemoryRegionKind::Storage(NirStorageId::Param(id)) => routine
                .params
                .iter()
                .find(|param| param.id == id)
                .map(|param| param.ty.width),
            NirMemoryRegionKind::Storage(NirStorageId::Global(id)) => {
                Some(self.global_sizes.get(&id).copied())
            }
            NirMemoryRegionKind::Static(id) => Some(self.static_sizes.get(&id).copied()),
            NirMemoryRegionKind::AbsoluteRange(space) => {
                if !self.address_fits_target(AddressValue::new(space, u64::from(end))) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} absolute region exceeds the selected target address space"),
                    ));
                }
                return;
            }
        };
        let Some(Some(available)) = available else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("{label} references missing storage identity"),
            ));
            return;
        };
        if u32::from(region.offset) + u32::from(region.size) > u32::from(available) {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} region {}+{} exceeds storage size {available}",
                    region.offset, region.size
                ),
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
        match place.kind {
            NirPlaceKind::Param { id, .. }
                if !routine.params.iter().any(|param| param.id == id) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown param id {}", id.0),
                ));
            }
            NirPlaceKind::Local { id, .. }
                if !routine.locals.iter().any(|local| local.id == id) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown local id {}", id.0),
                ));
            }
            NirPlaceKind::Global { id, .. } if !self.global_sizes.contains_key(&id) => {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("{label} references unknown global id {}", id.0),
                ));
            }
            NirPlaceKind::Absolute(address)
                if address.address_space != self.target_layout.data_pointer.address_space
                    || ty
                        .width
                        .is_none_or(|size| !self.address_extent_fits_target(address, size)) =>
            {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!(
                        "{label} is outside the selected target address range"
                    ),
                ));
            }
            _ => {}
        }
    }

    fn type_shape(&mut self, routine: &NirRoutine, block: &NirBlock, ty: &NirType, label: &str) {
        if ty.kind.width(self.target_layout) != ty.width {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} NIR type width mismatch: kind {:?} has {:?}, legacy width is {:?}",
                    ty.kind,
                    ty.kind.width(self.target_layout),
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
        if ty.kind.width(self.target_layout) != ty.width {
            self.diagnostics.push(NirDiagnostic::program(format!(
                "static data `{label}` NIR type width mismatch: kind {:?} has {:?}, legacy width is {:?}",
                ty.kind,
                ty.kind.width(self.target_layout),
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
            NirValue::Null { ty } => {
                self.type_shape(routine, block, ty, label);
                if !ty.kind.is_address() {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} null value must have pointer or callable type"),
                    ));
                }
            }
            NirValue::AddressConst { address, ty } => {
                self.type_shape(routine, block, ty, label);
                if pointer_type_address_space(ty) != Some(address.address_space)
                    || !self.address_fits_target(*address)
                {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} address constant does not fit its typed address space"),
                    ));
                }
            }
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
            NirValue::RoutineAddr { id, ty, .. } => {
                self.type_shape(routine, block, ty, label);
                if !matches!(
                    ty.kind,
                    NirTypeKind::Callable { address_space, .. }
                        if address_space == self.target_layout.code_pointer.address_space
                ) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} routine address must have code-pointer type"),
                    ));
                }
                if !self.routine_signatures.contains_key(id) {
                    self.diagnostics.push(NirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("{label} references missing routine id `{id}`"),
                    ));
                }
            }
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
            | NirValue::Null { .. }
            | NirValue::AddressConst { .. }
            | NirValue::StaticAddr { .. }
            | NirValue::Param(_)
            | NirValue::GlobalAddr(_)
            | NirValue::RoutineAddr { .. } => false,
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
                    | NirValue::Null { .. }
                    | NirValue::AddressConst { .. }
                    | NirValue::StaticAddr { .. }
                    | NirValue::Param(_)
                    | NirValue::GlobalAddr(_)
                    | NirValue::RoutineAddr { .. } => None,
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
            definition
                .op_index
                .is_none_or(|definition| definition < use_index)
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
        if temp.def.block != block.id || temp.def.op_index != Some(op_index) {
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

    fn block_param_def_matches_table(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        id: TempId,
        ty: &NirType,
        _param_index: usize,
    ) {
        let Some(temp) = routine.temps.iter().find(|temp| temp.id == id) else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("block parameter `%t{}` is missing from temp table", id.0),
            ));
            return;
        };
        if temp.def.block != block.id || temp.def.op_index.is_some() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("block parameter `%t{}` has stale temp table location", id.0),
            ));
        }
        if &temp.ty != ty {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "block parameter `%t{}` type does not match temp table",
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

    fn match_operand_widths(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        left: &NirValue,
        right: &NirValue,
        label: &str,
    ) {
        let (Some(left_width), Some(right_width)) = (value_width(left), value_width(right)) else {
            return;
        };
        if left_width != right_width {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} width mismatch: left is {left_width} byte(s), right is {right_width} byte(s)"
                ),
            ));
        }
    }

    fn match_operand_type_width(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        operand_ty: &NirType,
        value: &NirValue,
        label: &str,
    ) {
        let (Some(expected_width), Some(actual_width)) = (operand_ty.width, value_width(value))
        else {
            return;
        };
        if expected_width != actual_width {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} width mismatch: operand type {} is {expected_width} byte(s), value is {actual_width} byte(s)",
                    operand_ty.summary
                ),
            ));
        }
    }

    fn match_operand_machine_type(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        operand_ty: &NirType,
        value: &NirValue,
        label: &str,
    ) {
        let value_ty = match value {
            NirValue::Temp { ty, .. }
            | NirValue::StaticAddr { ty, .. }
            | NirValue::Null { ty }
            | NirValue::AddressConst { ty, .. }
            | NirValue::RoutineAddr { ty, .. } => ty,
            NirValue::ConstU8(_)
            | NirValue::ConstU16(_)
            | NirValue::Param(_)
            | NirValue::GlobalAddr(_) => return,
        };
        let Some(expected) = compare_machine_type(operand_ty) else {
            return;
        };
        let Some(actual) = compare_machine_type(value_ty) else {
            return;
        };
        if expected != actual {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "{label} type mismatch: operand type {} does not match value type {}",
                    operand_ty.summary, value_ty.summary
                ),
            ));
        }
    }

    fn require_edge(
        &mut self,
        routine: &NirRoutine,
        block: &NirBlock,
        edge: &NirEdge,
        temp_facts: &NirTempFacts<'_>,
    ) {
        let Some(target) = routine
            .blocks
            .iter()
            .find(|target| target.id == edge.target)
        else {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("edge target block id `{}` does not exist", edge.target.0),
            ));
            return;
        };
        if edge.args.len() != target.params.len() {
            self.diagnostics.push(NirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "edge to block id `{}` supplies {} argument(s), expected {}",
                    edge.target.0,
                    edge.args.len(),
                    target.params.len()
                ),
            ));
        }
        for (index, (arg, param)) in edge.args.iter().zip(&target.params).enumerate() {
            self.value_type(routine, block, arg, "edge argument");
            self.value_temp_use(
                routine,
                block,
                arg,
                block.ops.len(),
                temp_facts,
                "edge argument",
            );
            if !value_matches_type(arg, &param.ty) {
                self.diagnostics.push(NirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!(
                        "edge argument {index} to block id `{}` does not match parameter type {}",
                        edge.target.0, param.ty.summary
                    ),
                ));
            }
        }
    }
}

fn constant_binary_value(op: NirBinaryOp, left: &NirValue, right: &NirValue) -> Option<u16> {
    let value = |value: &NirValue| match value {
        NirValue::ConstU8(value) => Some(u16::from(*value)),
        NirValue::ConstU16(value) => Some(*value),
        _ => None,
    };
    let left = value(left)?;
    let right = value(right)?;
    match op {
        NirBinaryOp::Add => Some(left.wrapping_add(right)),
        NirBinaryOp::Sub => Some(left.wrapping_sub(right)),
        _ => None,
    }
}

fn value_matches_type(value: &NirValue, expected: &NirType) -> bool {
    match value {
        NirValue::ConstU8(_) => expected.width == Some(ByteSize::ONE),
        NirValue::ConstU16(_) => expected.width == Some(ByteSize::new(2)),
        NirValue::Null { ty }
        | NirValue::AddressConst { ty, .. }
        | NirValue::RoutineAddr { ty, .. }
        | NirValue::StaticAddr { ty, .. }
        | NirValue::Temp { ty, .. } => ty == expected,
        NirValue::Param(_) | NirValue::GlobalAddr(_) => false,
    }
}

fn value_has_address_type(value: &NirValue) -> bool {
    match value {
        NirValue::Null { .. }
        | NirValue::AddressConst { .. }
        | NirValue::RoutineAddr { .. }
        | NirValue::StaticAddr {
            ty: NirType {
                kind: NirTypeKind::Pointer { .. } | NirTypeKind::Callable { .. },
                ..
            },
            ..
        }
        | NirValue::Temp {
            ty: NirType {
                kind: NirTypeKind::Pointer { .. } | NirTypeKind::Callable { .. },
                ..
            },
            ..
        } => true,
        _ => false,
    }
}

fn pointer_type_address_space(ty: &NirType) -> Option<crate::target::AddressSpaceId> {
    match ty.kind {
        NirTypeKind::Pointer { address_space, .. }
        | NirTypeKind::Callable { address_space, .. } => Some(address_space),
        _ => None,
    }
}

fn compare_machine_type(ty: &NirType) -> Option<(u16, bool)> {
    let signed = matches!(ty.kind, NirTypeKind::I8 | NirTypeKind::I16);
    match ty.kind {
        NirTypeKind::Bool
        | NirTypeKind::U8
        | NirTypeKind::I8
        | NirTypeKind::U16
        | NirTypeKind::I16
        | NirTypeKind::Pointer { .. }
        | NirTypeKind::Callable { .. } => ty
            .width
            .and_then(|width| u16::try_from(width).ok())
            .map(|width| (width, signed)),
        NirTypeKind::Void | NirTypeKind::Real | NirTypeKind::Record { .. } | NirTypeKind::Error => {
            None
        }
    }
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
